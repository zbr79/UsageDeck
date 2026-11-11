//! API-equivalent Claude Code spend from the local transcripts, `~/.claude/projects/**/*.jsonl`.
//!
//! Read-only and incremental: a byte offset per file lives in `claude-logs-index.json`; each scan
//! reads only bytes appended since, and only up to the last complete line. A file that shrinks or
//! is replaced (new creation time) is read again from the start; the seen-set drops what was
//! already counted. Only assistant lines that carry `usage` are parsed, into structs that hold
//! the usage fields only: no prompt or reply content is kept, stored or logged.
//!
//! Replies are deduplicated by `message.id` + `requestId` (Claude Code writes one line per content
//! block, each with the same usage, and resumed sessions copy earlier lines) with a seen-set of
//! 64-bit hashes covering 8 days. Token counts go into one-minute buckets per model and price
//! tier in `spend-buckets.json` (8 days); dollars are computed from them at read time. The tier is
//! the one thing priced per reply: it depends on that reply's prompt size (Haiku 5.5 over 100K).
//!
//! Port of the read-only probe the plan's numbers came from (scratch `spend.mjs`).

use crate::history::{self, HISTORY_FILE, HOUR_MS, MINUTE_MS};
use crate::pricing::{cost, price_of, tier_of, Tier, Tokens};
use crate::store::write_json_atomic;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const INDEX_FILE: &str = "claude-logs-index.json";
pub const BUCKETS_FILE: &str = "spend-buckets.json";
/// Backfill depth, seen-set and bucket retention.
const KEEP_MS: i64 = 8 * history::DAY_MS;
/// 2: bucket rows carry the price tier. Files of another version are dropped and the transcripts
/// read again.
const FORMAT_VERSION: u32 = 2;
/// A model id longer than this is not a model id: the reply is skipped.
const MAX_MODEL_LEN: usize = 100;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// `%USERPROFILE%\.claude\projects`.
pub fn default_root() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".claude").join("projects")
}

// ---- transcript line shapes (usage fields only) ------------------------------------------------

#[derive(Deserialize)]
struct LogLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<LogMessage>,
}

#[derive(Deserialize)]
struct LogMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<LogUsage>,
}

#[derive(Deserialize)]
struct LogUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_creation: Option<CacheCreation>,
}

#[derive(Deserialize)]
struct CacheCreation {
    ephemeral_5m_input_tokens: Option<u64>,
    ephemeral_1h_input_tokens: Option<u64>,
}

/// One assistant reply's usage.
#[derive(Clone, Debug, PartialEq)]
struct Reply {
    key: u64,
    t: i64,
    model: String,
    tokens: Tokens,
}

/// FNV-1a 64: stable across runs and Rust versions (the seen-set is persisted).
fn fnv1a(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            h ^= 0xff;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        for b in part.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// The usage of an assistant line, or None for any other line.
fn parse_reply(line: &str) -> Option<Reply> {
    // cheap filter first: most lines are user / tool lines
    if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
        return None;
    }
    let l: LogLine = serde_json::from_str(line).ok()?;
    if l.kind.as_deref() != Some("assistant") {
        return None;
    }
    let m = l.message?;
    let u = m.usage?;
    let t = chrono::DateTime::parse_from_rfc3339(l.timestamp.as_deref()?)
        .ok()?
        .timestamp_millis();
    let model = m.model.unwrap_or_default();
    if model.len() > MAX_MODEL_LEN {
        return None;
    }
    let (cache_write_5m, cache_write_1h) = match u.cache_creation {
        Some(c) => (
            c.ephemeral_5m_input_tokens.unwrap_or(0),
            c.ephemeral_1h_input_tokens.unwrap_or(0),
        ),
        // No split recorded: count all cache writes at the cheaper 5-minute rate (an
        // under-estimate when they were 1-hour writes). The probe counted them as 1-hour.
        None => (u.cache_creation_input_tokens.unwrap_or(0), 0),
    };
    Some(Reply {
        key: fnv1a(&[
            m.id.as_deref().unwrap_or(""),
            l.request_id.as_deref().unwrap_or(""),
        ]),
        t,
        model,
        tokens: Tokens {
            input: u.input_tokens.unwrap_or(0),
            // already includes thinking tokens
            output: u.output_tokens.unwrap_or(0),
            cache_read: u.cache_read_input_tokens.unwrap_or(0),
            cache_write_5m,
            cache_write_1h,
        },
    })
}

/// Read complete lines of `path` from `start` up to `len`; returns the offset after the last
/// complete line (a partial last line is left for the next scan). Shared-read open: Claude Code
/// keeps appending to these files.
fn read_lines(path: &Path, start: u64, len: u64, mut on_line: impl FnMut(&str)) -> u64 {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        opts.share_mode(0x1 | 0x2 | 0x4);
    }
    let Ok(mut file) = opts.open(path) else {
        return start;
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return start;
    }
    let mut reader = BufReader::with_capacity(256 * 1024, file.take(len.saturating_sub(start)));
    let mut offset = start;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if buf.last() != Some(&b'\n') {
                    break; // partial last line
                }
                offset += n as u64;
                if let Ok(text) = std::str::from_utf8(&buf) {
                    on_line(text.trim_end());
                }
            }
        }
    }
    offset
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let Ok(kind) = e.file_type() else { continue };
        let path = e.path();
        if kind.is_dir() {
            walk(&path, out);
        } else if kind.is_file() && path.extension().is_some_and(|x| x == "jsonl") {
            out.push(path);
        }
    }
}

fn ms_of(t: std::io::Result<SystemTime>) -> i64 {
    t.ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

// ---- persisted state ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct FileMark {
    offset: u64,
    /// file length at the last scan: unchanged means nothing to read (even with a partial line)
    #[serde(default)]
    len: u64,
    /// creation time (ms epoch): a different value means the file was replaced
    created: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    /// path relative to the projects folder -> read position
    files: BTreeMap<String, FileMark>,
    /// seen-set: base64 of (u64 key LE, u32 minute LE) records
    seen: String,
}

/// (minute, model, tier, input, output, cache read, cache write 5m, cache write 1h, replies)
type BucketRow = (i64, String, Tier, u64, u64, u64, u64, u64, u64);

#[derive(Default, Serialize, Deserialize)]
struct BucketsFile {
    version: u32,
    rows: Vec<BucketRow>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Bucket {
    tokens: Tokens,
    replies: u64,
}

/// minute (ms epoch / 60 000) -> (model, price tier) -> bucket
type Buckets = BTreeMap<i64, BTreeMap<(String, Tier), Bucket>>;

#[derive(Default)]
struct ScanState {
    files: HashMap<String, FileMark>,
    /// reply key -> minute of the reply
    seen: HashMap<u64, i64>,
}

#[derive(Default)]
struct Shared {
    loaded: bool,
    buckets: Buckets,
    /// ms epoch of the last completed scan in this process
    scanned_at: Option<i64>,
}

/// What one scan did, for the log and `--pace`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStats {
    pub files: usize,
    pub files_read: usize,
    pub bytes_read: u64,
    pub new_replies: usize,
    pub millis: u64,
}

/// The spend engine: scanner state plus the aggregated buckets.
pub struct SpendEngine {
    dir: PathBuf,
    root: PathBuf,
    scan: Mutex<Option<ScanState>>,
    shared: Mutex<Shared>,
}

fn encode_seen(seen: &HashMap<u64, i64>) -> String {
    let mut bytes = Vec::with_capacity(seen.len() * 12);
    for (k, m) in seen {
        bytes.extend_from_slice(&k.to_le_bytes());
        bytes.extend_from_slice(&(*m as u32).to_le_bytes());
    }
    STANDARD.encode(bytes)
}

fn decode_seen(s: &str) -> HashMap<u64, i64> {
    let bytes = STANDARD.decode(s).unwrap_or_default();
    let (records, _) = bytes.as_chunks::<12>();
    records
        .iter()
        .map(|c| {
            let (key, minute) = c.split_at(8);
            let k = u64::from_le_bytes(key.try_into().unwrap_or_default());
            let m = u32::from_le_bytes(minute.try_into().unwrap_or_default());
            (k, i64::from(m))
        })
        .collect()
}

impl SpendEngine {
    pub fn new(dir: PathBuf, root: PathBuf) -> Self {
        Self {
            dir,
            root,
            scan: Mutex::new(None),
            shared: Mutex::new(Shared::default()),
        }
    }

    fn lock_shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !s.loaded {
            s.loaded = true;
            // buckets without their index would be counted again by the backfill
            if self.dir.join(INDEX_FILE).exists() {
                let file: Option<BucketsFile> = std::fs::read(self.dir.join(BUCKETS_FILE))
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok());
                if let Some(f) = file.filter(|f| f.version == FORMAT_VERSION) {
                    for (minute, model, tier, i, o, cr, w5, w1, n) in f.rows {
                        s.buckets.entry(minute).or_default().insert(
                            (model, tier),
                            Bucket {
                                tokens: Tokens {
                                    input: i,
                                    output: o,
                                    cache_read: cr,
                                    cache_write_5m: w5,
                                    cache_write_1h: w1,
                                },
                                replies: n,
                            },
                        );
                    }
                }
            }
        }
        s
    }

    fn load_index(&self) -> ScanState {
        let file: Option<IndexFile> = std::fs::read(self.dir.join(INDEX_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        match file.filter(|f| f.version == FORMAT_VERSION) {
            Some(f) => ScanState {
                files: f.files.into_iter().collect(),
                seen: decode_seen(&f.seen),
            },
            None => {
                // no usable index: start over, and drop buckets it would count again
                let mut s = self.lock_shared();
                s.buckets.clear();
                ScanState::default()
            }
        }
    }

    /// One incremental scan at `now` (ms epoch). Blocking: run it off the async runtime.
    pub fn scan_once(&self, now: i64) -> ScanStats {
        let started = Instant::now();
        let mut guard = self.scan.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(self.load_index());
        }
        let state = guard.as_mut().expect("loaded above");
        let mut stats = ScanStats::default();
        let mut paths = Vec::new();
        walk(&self.root, &mut paths);
        stats.files = paths.len();
        let mut present = HashSet::new();
        let mut dirty = !self.dir.join(INDEX_FILE).exists();
        let mut fresh: Vec<Reply> = Vec::new();
        let oldest = now - KEEP_MS;
        for path in paths {
            let Ok(rel) = path.strip_prefix(&self.root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            present.insert(rel.clone());
            let len = meta.len();
            let created = ms_of(meta.created());
            let start = match state.files.get(&rel) {
                // first sight of a file untouched for 8 days: nothing in it can count
                None if ms_of(meta.modified()) < oldest => {
                    dirty = true;
                    state.files.insert(
                        rel,
                        FileMark {
                            offset: len,
                            len,
                            created,
                        },
                    );
                    continue;
                }
                None => 0,
                // shrunk or replaced: read again from the start (the seen-set drops repeats)
                Some(m) if len < m.offset || m.created != created => 0,
                Some(m) if len == m.offset || len == m.len => continue,
                Some(m) => m.offset,
            };
            stats.files_read += 1;
            dirty = true;
            let offset = read_lines(&path, start, len, |line| {
                if let Some(r) = parse_reply(line) {
                    fresh.push(r);
                }
            });
            stats.bytes_read += offset - start;
            state.files.insert(
                rel,
                FileMark {
                    offset,
                    len,
                    created,
                },
            );
        }
        let known = state.files.len();
        state.files.retain(|k, _| present.contains(k));
        dirty |= state.files.len() != known;

        let mut added: Vec<Reply> = Vec::new();
        for r in fresh {
            if r.t < oldest || r.t > now + HOUR_MS {
                continue;
            }
            if state.seen.insert(r.key, r.t / MINUTE_MS).is_none() {
                added.push(r);
            }
        }
        let oldest_minute = oldest / MINUTE_MS;
        state.seen.retain(|_, m| *m >= oldest_minute);
        stats.new_replies = added.len();

        let rows = {
            let mut s = self.lock_shared();
            for r in &added {
                let b = s
                    .buckets
                    .entry(r.t / MINUTE_MS)
                    .or_default()
                    .entry((r.model.clone(), tier_of(&r.model, &r.tokens)))
                    .or_default();
                b.tokens.add(&r.tokens);
                b.replies += 1;
            }
            s.buckets = s.buckets.split_off(&oldest_minute);
            s.scanned_at = Some(now);
            s.buckets
                .iter()
                .flat_map(|(minute, models)| {
                    models.iter().map(move |((model, tier), b)| {
                        let t = b.tokens;
                        (
                            *minute,
                            model.clone(),
                            *tier,
                            t.input,
                            t.output,
                            t.cache_read,
                            t.cache_write_5m,
                            t.cache_write_1h,
                            b.replies,
                        )
                    })
                })
                .collect::<Vec<BucketRow>>()
        };
        if !dirty {
            stats.millis = started.elapsed().as_millis() as u64;
            return stats;
        }
        // The index goes first: a crash between the two writes loses a minute of counts rather
        // than counting it twice.
        let index = IndexFile {
            version: FORMAT_VERSION,
            files: state
                .files
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            seen: encode_seen(&state.seen),
        };
        if let Err(e) = write_json_atomic(&self.dir.join(INDEX_FILE), &index) {
            log::warn!("claude logs: index not saved: {e}");
        }
        let buckets = BucketsFile {
            version: FORMAT_VERSION,
            rows,
        };
        if let Err(e) = write_json_atomic(&self.dir.join(BUCKETS_FILE), &buckets) {
            log::warn!("claude logs: buckets not saved: {e}");
        }
        stats.millis = started.elapsed().as_millis() as u64;
        stats
    }

    /// Dollars spent from `since` (ms epoch) to `now`, priced models only.
    pub fn spend_between(&self, since: i64, now: i64) -> f64 {
        let s = self.lock_shared();
        sum_where(&s.buckets, |mid| mid > since && mid <= now + MINUTE_MS)
    }

    /// The `spend` object of `/api/claude/pace` at `now`.
    pub fn spend(&self, now: i64) -> Spend {
        let local = Local
            .timestamp_millis_opt(now)
            .single()
            .unwrap_or_else(Local::now);
        let midnight = local
            .with_time(chrono::NaiveTime::MIN)
            .single()
            .map_or(now - now.rem_euclid(history::DAY_MS), |d| {
                d.timestamp_millis()
            });
        let hour0 = now
            - i64::from(local.minute()) * MINUTE_MS
            - i64::from(local.second()) * 1000
            - i64::from(local.timestamp_subsec_millis());
        let s = self.lock_shared();
        summarize(&s.buckets, now, midnight, hour0, s.scanned_at)
    }
}

/// The midpoint of a minute bucket: a bucket counts in a window when its midpoint is inside, so
/// the partial minutes at both edges even out.
fn mid(minute: i64) -> i64 {
    minute * MINUTE_MS + MINUTE_MS / 2
}

fn sum_where(buckets: &Buckets, keep: impl Fn(i64) -> bool) -> f64 {
    let mut prices = HashMap::new();
    let mut usd = 0.0;
    for (minute, models) in buckets {
        if !keep(mid(*minute)) {
            continue;
        }
        for ((model, tier), b) in models {
            if let Some(p) = *prices
                .entry((model.as_str(), *tier))
                .or_insert_with(|| price_of(model, *tier))
            {
                usd += cost(&p, &b.tokens);
            }
        }
    }
    usd
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpend {
    pub model: String,
    pub usd24h: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    pub last_hour: f64,
    pub last5h: f64,
    pub today: f64,
    pub last7d: f64,
    pub rate60m: f64,
    pub rate15m: f64,
    pub by_model: Vec<ModelSpend>,
    pub hourly: Vec<f64>,
    /// Replies, last 7 days, on models missing from the price table; none of their tokens are in
    /// the dollar figures. Replies without tokens (`<synthetic>`) cost nothing and are left out.
    pub unpriced_replies: u64,
    /// Those models' ids, sorted.
    pub unpriced_models: Vec<String>,
    pub scanned_at: Option<i64>,
    pub backfilling: bool,
    /// diagnostics for `--pace` only; not part of the API body
    #[serde(skip)]
    pub last24h: f64,
}

impl Spend {
    /// The Claude spend part of the diagnostics report.
    pub fn report(&self) -> String {
        if self.backfilling {
            return "Logs: first read still running".into();
        }
        if self.unpriced_models.is_empty() {
            return "Not priced (last 7 days): none".into();
        }
        format!(
            "Not priced (last 7 days): {} ({} replies, not in the dollar figures)",
            self.unpriced_models.join(", "),
            self.unpriced_replies
        )
    }
}

fn summarize(
    buckets: &Buckets,
    now: i64,
    midnight: i64,
    hour0: i64,
    scanned_at: Option<i64>,
) -> Spend {
    let mut out = Spend {
        last_hour: 0.0,
        last5h: 0.0,
        today: 0.0,
        last7d: 0.0,
        rate60m: 0.0,
        rate15m: 0.0,
        by_model: Vec::new(),
        hourly: vec![0.0; 24],
        unpriced_replies: 0,
        unpriced_models: Vec::new(),
        scanned_at,
        backfilling: scanned_at.is_none(),
        last24h: 0.0,
    };
    let mut by_model: BTreeMap<&str, f64> = BTreeMap::new();
    let mut unpriced: BTreeSet<&str> = BTreeSet::new();
    let mut last15 = 0.0;
    let since = |hours: i64, m: i64| m > now - hours * HOUR_MS && m <= now + MINUTE_MS;
    for (minute, models) in buckets.range((now - 7 * history::DAY_MS) / MINUTE_MS - 1..) {
        let m = mid(*minute);
        if !since(7 * 24, m) {
            continue;
        }
        for ((model, tier), b) in models {
            let Some(price) = price_of(model, *tier) else {
                if !b.tokens.is_empty() {
                    out.unpriced_replies += b.replies;
                    unpriced.insert(model);
                }
                continue;
            };
            let usd = cost(&price, &b.tokens);
            out.last7d += usd;
            if since(24, m) {
                out.last24h += usd;
                *by_model.entry(model.as_str()).or_default() += usd;
            }
            if since(5, m) {
                out.last5h += usd;
            }
            if since(1, m) {
                out.last_hour += usd;
            }
            if m > now - 15 * MINUTE_MS && m <= now + MINUTE_MS {
                last15 += usd;
            }
            if m >= midnight {
                out.today += usd;
            }
            let start = minute * MINUTE_MS;
            let h = if start >= hour0 {
                0
            } else {
                (hour0 - start - 1) / HOUR_MS + 1
            };
            if (0..24).contains(&h) {
                out.hourly[h as usize] += usd;
            }
        }
    }
    out.rate60m = out.last_hour;
    out.rate15m = last15 * 4.0;
    out.by_model = by_model
        .into_iter()
        .map(|(model, usd24h)| ModelSpend {
            model: model.to_string(),
            usd24h,
        })
        .collect();
    out.by_model.sort_by(|a, b| b.usd24h.total_cmp(&a.usd24h));
    out.unpriced_models = unpriced.into_iter().map(str::to_string).collect();
    out
}

// ---- /api/claude/pace ---------------------------------------------------------------------------

/// The `/api/claude/pace` body. Windows come from the newest Claude account in the history (the
/// active account: only its fresh readings are recorded).
pub fn pace_body(engine: &SpendEngine, dir: &Path, now: i64) -> Value {
    let spend = engine.spend(now);
    let readings = history::load_all(&dir.join(HISTORY_FILE));
    let claude: Vec<history::Reading> = readings.into_iter().filter(|r| r.p == "claude").collect();
    let active = claude.last().map(|r| r.a.clone());
    let mine: Vec<history::Reading> = claude
        .into_iter()
        .filter(|r| Some(&r.a) == active.as_ref())
        .collect();
    let five = history::points_of(&mine, history::FIVE_HOUR);
    let weekly = history::points_of(&mine, history::WEEKLY);
    let usd_per_weekly_pct =
        history::usd_per_weekly_pct(&weekly, now, |t| engine.spend_between(t, now));
    json!({
        "spend": spend,
        "windows": {
            "fiveHour": history::window_pace(&five, now, 5 * HOUR_MS),
            "weekly": history::window_pace(&weekly, now, 7 * history::DAY_MS),
        },
        "calibration": { "usdPerWeeklyPct": usd_per_weekly_pct },
        "coverage": "claude-code-this-pc",
    })
}

/// Scan now, then every 60 s, on its own thread for the life of the app.
pub fn start_scanner(engine: std::sync::Arc<SpendEngine>) {
    let spawned = std::thread::Builder::new()
        .name("claude-logs".into())
        .spawn(move || loop {
            let stats = engine.scan_once(now_ms());
            if stats.millis > 2000 || stats.new_replies > 1000 {
                log::info!(
                    "claude logs: {} files, {} read, {} bytes, {} new replies in {} ms",
                    stats.files,
                    stats.files_read,
                    stats.bytes_read,
                    stats.new_replies,
                    stats.millis
                );
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        });
    if let Err(e) = spawned {
        log::warn!("claude logs: scanner thread not started: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/claude_logs/proj"
    );
    /// 2026-10-17T10:30:00Z, half an hour after the fixture's replies
    const NOW: i64 = 1_792_233_000_000;
    const MSG1: f64 = 0.024_04; // opus: 10 in, 100 out, 5000 read, 1000 5m + 2000 1h writes
    const MSG2: f64 = 0.012_04; // sonnet: 20 in, 200 out, 4000 writes without a split -> 5m
    const MSG4: f64 = 0.001_004; // opus: 1 in, 50 out

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "usagedeck-test-logs-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn copy_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("sub")).unwrap();
        for f in ["a.jsonl", "sub/b.jsonl"] {
            std::fs::copy(Path::new(FIXTURE).join(f), root.join(f)).unwrap();
        }
    }

    fn append(file: &Path, text: &str) {
        let mut f = std::fs::OpenOptions::new().append(true).open(file).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn reply_line(id: &str, minute: u32, output: u64) -> String {
        format!(
            "{{\"type\":\"assistant\",\"message\":{{\"id\":\"{id}\",\"model\":\"claude-opus-5-5\",\"content\":[],\"usage\":{{\"input_tokens\":0,\"output_tokens\":{output}}}}},\"requestId\":\"r-{id}\",\"timestamp\":\"2026-10-17T10:{minute:02}:00.000Z\"}}\n"
        )
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn parse_keeps_usage_only_and_splits_the_cache() {
        let text = std::fs::read_to_string(Path::new(FIXTURE).join("a.jsonl")).unwrap();
        let replies: Vec<Reply> = text.lines().filter_map(parse_reply).collect();
        assert_eq!(replies.len(), 4); // the user and summary lines are skipped
        let r = &replies[0];
        assert_eq!(r.model, "claude-opus-5-5");
        assert_eq!(
            (r.tokens.cache_write_5m, r.tokens.cache_write_1h),
            (1000, 2000)
        );
        // no `cache_creation` split: all writes at the 5-minute rate
        let s = &replies[2];
        assert_eq!(
            (s.tokens.cache_write_5m, s.tokens.cache_write_1h),
            (4000, 0)
        );
        assert_eq!(replies[0].key, replies[1].key);
        assert_ne!(replies[0].key, replies[2].key);
    }

    #[test]
    fn scanner_dedupes_grows_waits_for_lines_and_rereads_shrunk_files() {
        let (root, dir) = (temp("root"), temp("data"));
        copy_fixture(&root);
        let engine = SpendEngine::new(dir.clone(), root.clone());
        assert!(engine.spend(NOW).backfilling);

        // msg_1 three times over two files, msg_2, msg_3 (<synthetic>); msg_4 is still partial
        let s = engine.scan_once(NOW);
        assert_eq!((s.files, s.files_read, s.new_replies), (2, 2, 3));
        let spend = engine.spend(NOW);
        assert!(!spend.backfilling && spend.scanned_at == Some(NOW));
        assert!(close(spend.last_hour, MSG1 + MSG2), "{}", spend.last_hour);
        assert!(close(spend.last7d, MSG1 + MSG2));
        // <synthetic> has no tokens: nothing is missing from the dollars
        assert_eq!(spend.unpriced_replies, 0);
        assert!(spend.unpriced_models.is_empty());
        let models: Vec<&str> = spend.by_model.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(models, ["claude-opus-5-5", "claude-sonnet-5-5"]);
        assert_eq!(spend.hourly.len(), 24);
        assert!(close(spend.hourly.iter().sum::<f64>(), MSG1 + MSG2));

        // nothing new: nothing read
        let s = engine.scan_once(NOW);
        assert_eq!((s.files_read, s.new_replies), (0, 0));

        // the partial last line completes; a.jsonl grows by one reply
        append(
            &root.join("sub/b.jsonl"),
            "50}},\"requestId\":\"req_4\",\"uuid\":\"b-2\",\"timestamp\":\"2026-10-17T10:20:00.000Z\"}\n",
        );
        append(&root.join("a.jsonl"), &reply_line("msg_5", 25, 1000));
        let s = engine.scan_once(NOW);
        assert_eq!((s.files_read, s.new_replies), (2, 2));
        let total = MSG1 + MSG2 + MSG4 + 0.02;
        assert!(close(engine.spend(NOW).last_hour, total));

        // a.jsonl shrinks (rewritten): read from the start again, repeats dropped
        let first_two: String = std::fs::read_to_string(root.join("a.jsonl"))
            .unwrap()
            .lines()
            .take(2)
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(root.join("a.jsonl"), first_two).unwrap();
        let s = engine.scan_once(NOW);
        assert_eq!((s.files_read, s.new_replies), (1, 0));
        append(&root.join("a.jsonl"), &reply_line("msg_6", 26, 500));
        assert_eq!(engine.scan_once(NOW).new_replies, 1);
        let total = total + 0.01;
        assert!(close(engine.spend(NOW).last_hour, total));

        // a restart picks up the saved index, seen-set and buckets
        let again = SpendEngine::new(dir.clone(), root.clone());
        assert!(close(again.spend(NOW).last7d, total));
        let s = again.scan_once(NOW);
        assert_eq!((s.files_read, s.new_replies), (0, 0));
        assert!(close(again.spend(NOW).last7d, total));

        // the stored files hold aggregates only
        for f in [INDEX_FILE, BUCKETS_FILE] {
            let text = std::fs::read_to_string(dir.join(f)).unwrap();
            assert!(!text.contains("redacted") && !text.contains("msg_"), "{f}");
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_and_hours_follow_the_clock() {
        let mut b = Buckets::new();
        let add = |b: &mut Buckets, mins_ago: i64, out: u64| {
            b.entry((NOW - mins_ago * MINUTE_MS) / MINUTE_MS)
                .or_default()
                .entry(("claude-opus-5-5".into(), Tier::Standard))
                .or_default()
                .tokens
                .output += out;
        };
        add(&mut b, 5, 50_000); // $1
        add(&mut b, 30, 50_000); // $1
        add(&mut b, 120, 100_000); // $2
        add(&mut b, 26 * 60, 50_000); // $1
        add(&mut b, 9 * 24 * 60, 50_000); // outside 7 days
        let hour0 = NOW - 30 * MINUTE_MS; // 10:00
        let midnight = NOW - 10 * HOUR_MS - 30 * MINUTE_MS;
        let s = summarize(&b, NOW, midnight, hour0, Some(NOW));
        assert!(close(s.last_hour, 2.0) && close(s.rate60m, 2.0));
        assert!(close(s.rate15m, 4.0));
        assert!(close(s.last5h, 4.0) && close(s.today, 4.0));
        assert!(close(s.last24h, 4.0) && close(s.last7d, 5.0));
        // 5 and 30 min ago are in the current hour (10:00-10:30); 120 min ago is 08:30 -> index 2
        assert!(close(s.hourly[0], 2.0) && close(s.hourly[2], 2.0));
        assert!(close(s.by_model[0].usd24h, 4.0));
    }

    #[test]
    fn unpriced_models_are_named_and_kept_out_of_the_dollars() {
        let mut b = Buckets::new();
        let minute = (NOW - 10 * MINUTE_MS) / MINUTE_MS;
        let mut add = |model: &str, output: u64, replies: u64| {
            let bucket = b
                .entry(minute)
                .or_default()
                .entry((model.into(), Tier::Standard))
                .or_default();
            bucket.tokens.output += output;
            bucket.replies += replies;
        };
        add("claude-opus-5-5", 50_000, 1); // $1
        add("claude-mystery-9", 1_000, 2);
        add("claude-fable-9", 10, 1);
        add("<synthetic>", 0, 3);
        let s = summarize(&b, NOW, NOW - HOUR_MS, NOW - 30 * MINUTE_MS, Some(NOW));
        assert!(close(s.last7d, 1.0), "{}", s.last7d);
        assert_eq!(s.unpriced_replies, 3);
        assert_eq!(s.unpriced_models, ["claude-fable-9", "claude-mystery-9"]);
        let body = serde_json::to_value(&s).unwrap();
        assert_eq!(
            body["unpricedModels"],
            json!(["claude-fable-9", "claude-mystery-9"])
        );
        assert_eq!(
            s.report(),
            "Not priced (last 7 days): claude-fable-9, claude-mystery-9 (3 replies, not in the dollar figures)"
        );
        let priced = summarize(&Buckets::new(), NOW, NOW, NOW, Some(NOW));
        assert_eq!(priced.report(), "Not priced (last 7 days): none");
        let reading = summarize(&Buckets::new(), NOW, NOW, NOW, None);
        assert_eq!(reading.report(), "Logs: first read still running");
    }

    #[test]
    fn long_haiku_prompts_are_priced_per_reply() {
        let (root, dir) = (temp("haiku-root"), temp("haiku-data"));
        let line = |id: &str, input: u64| {
            format!(
                "{{\"type\":\"assistant\",\"message\":{{\"id\":\"{id}\",\"model\":\"claude-haiku-5-5\",\"usage\":{{\"input_tokens\":{input},\"output_tokens\":0}}}},\"requestId\":\"r-{id}\",\"timestamp\":\"2026-10-17T10:20:00.000Z\"}}\n"
            )
        };
        // same minute, same model: one prompt under 100K tokens, one over
        let text = line("h1", 50_000) + &line("h2", 150_000);
        std::fs::write(root.join("h.jsonl"), text).unwrap();
        let engine = SpendEngine::new(dir.clone(), root.clone());
        assert_eq!(engine.scan_once(NOW).new_replies, 2);
        // 50 000 x $0.10 + 150 000 x $0.50, per million; a merged bucket would give $0.02
        let spend = engine.spend(NOW);
        assert!(close(spend.last_hour, 0.005 + 0.075), "{}", spend.last_hour);
        assert_eq!(
            spend.by_model.len(),
            1,
            "one row per model, whatever the tier"
        );
        assert!(close(spend.by_model[0].usd24h, 0.08));
        // and after a restart, from the saved buckets
        let again = SpendEngine::new(dir.clone(), root.clone());
        assert!(close(again.spend(NOW).last_hour, 0.08));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
