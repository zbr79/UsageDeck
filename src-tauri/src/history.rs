//! Usage readings over time (`usage-history.jsonl` in the app data folder) and the pace maths on
//! top of them: speed in %/h, even pace, and the projected time of hitting the limit.
//!
//! Generic per provider; only Claude writes readings for now. A reading holds the time, the
//! provider, the hashed account id (`providers::log_id`, never an email) and each window's
//! percentage and reset time. Kept 14 days; readings older than 24 h are thinned to one per 5 min.

use crate::store::write_file_atomic;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

pub const HISTORY_FILE: &str = "usage-history.jsonl";

pub const MINUTE_MS: i64 = 60_000;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
pub const DAY_MS: i64 = 24 * HOUR_MS;
const KEEP_MS: i64 = 14 * DAY_MS;
const FULL_DETAIL_MS: i64 = DAY_MS;
const THIN_SLOT_MS: i64 = 5 * MINUTE_MS;
/// How often an append also rewrites the file thinned.
const COMPACT_EVERY_MS: i64 = HOUR_MS;

pub const FIVE_HOUR: &str = "fiveHour";
pub const WEEKLY: &str = "weekly";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowReading {
    pub pct: f64,
    pub resets_at: Option<String>,
}

/// One line of `usage-history.jsonl`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    /// ms epoch
    pub t: i64,
    /// provider id, e.g. "claude"
    pub p: String,
    /// hashed account id (`providers::log_id`)
    pub a: String,
    /// window name ("fiveHour", "weekly") -> reading
    pub w: BTreeMap<String, WindowReading>,
}

/// Last compaction time of this process (ms epoch); also serialises writers.
static WRITE_LOCK: Mutex<Option<i64>> = Mutex::new(None);

/// Append one reading; once an hour also rewrite the file thinned and pruned.
pub fn append(file: &Path, reading: &Reading, now: i64) {
    let mut last_compact = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let line = match serde_json::to_string(reading) {
        Ok(s) => s,
        Err(_) => return,
    };
    let written = (|| -> std::io::Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)?;
        f.write_all(format!("{line}\n").as_bytes())
    })();
    if let Err(e) = written {
        log::warn!("usage history: append failed: {e}");
        return;
    }
    if last_compact.is_none_or(|t| now - t >= COMPACT_EVERY_MS) {
        *last_compact = Some(now);
        let kept = thin(load_all(file), now);
        let mut out = String::new();
        for r in &kept {
            if let Ok(s) = serde_json::to_string(r) {
                out.push_str(&s);
                out.push('\n');
            }
        }
        if let Err(e) = write_file_atomic(file, out.as_bytes()) {
            log::warn!("usage history: compaction failed: {e}");
        }
    }
}

/// Every parseable reading in the file, oldest first.
pub fn load_all(file: &Path) -> Vec<Reading> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let mut out: Vec<Reading> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    out.sort_by_key(|r| r.t);
    out
}

/// Drop readings older than 14 days; older than 24 h keep the first reading per 5-minute slot
/// per provider and account.
pub fn thin(mut readings: Vec<Reading>, now: i64) -> Vec<Reading> {
    readings.sort_by_key(|r| r.t);
    let mut slots: HashSet<(String, String, i64)> = HashSet::new();
    readings
        .into_iter()
        .filter(|r| r.t >= now - KEEP_MS)
        .filter(|r| {
            r.t >= now - FULL_DETAIL_MS
                || slots.insert((r.p.clone(), r.a.clone(), r.t.div_euclid(THIN_SLOT_MS)))
        })
        .collect()
}

// ---- pace maths -------------------------------------------------------------------------------

/// One window's reading at one time.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub t: i64,
    pub pct: f64,
    pub resets_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPace {
    pub pct: f64,
    pub pct_per_hour: Option<f64>,
    pub even_pace_pct: Option<f64>,
    /// ms epoch
    pub eta_to_limit: Option<i64>,
    pub resets_at: Option<String>,
    /// "measuring" | "lasts" | "limit"
    pub state: &'static str,
}

fn parse_ms(iso: &Option<String>) -> Option<i64> {
    iso.as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
}

/// Two reset times name the same window when within this of each other (the endpoint's reset
/// time can move by a few seconds between replies).
const SAME_RESET_MS: i64 = 5 * MINUTE_MS;
/// A drop of more than this between readings is a reset.
const DROP_PCT: f64 = 0.5;
const SLOPE_SPAN_MS: i64 = HOUR_MS;
const MIN_POINTS: usize = 3;
const MIN_SPAN_MS: i64 = 15 * MINUTE_MS;

/// The readings of the current window: the newest run of points with the same reset time and no
/// drop in percentage. `points` must be oldest first.
pub fn current_segment(points: &[Point]) -> &[Point] {
    let Some(last) = points.last() else {
        return points;
    };
    let reset = parse_ms(&last.resets_at);
    let mut start = points.len() - 1;
    while start > 0 {
        let prev = &points[start - 1];
        let next = &points[start];
        let same_reset = match (parse_ms(&prev.resets_at), reset) {
            (Some(a), Some(b)) => (a - b).abs() <= SAME_RESET_MS,
            (None, None) => true,
            _ => false,
        };
        if !same_reset || prev.pct > next.pct + DROP_PCT {
            break;
        }
        start -= 1;
    }
    &points[start..]
}

/// Least-squares slope of pct over time, in %/h.
fn slope_per_hour(points: &[Point]) -> f64 {
    let n = points.len() as f64;
    let t0 = points[0].t;
    let xs: Vec<f64> = points
        .iter()
        .map(|p| (p.t - t0) as f64 / HOUR_MS as f64)
        .collect();
    let mx = xs.iter().sum::<f64>() / n;
    let my = points.iter().map(|p| p.pct).sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (x, p) in xs.iter().zip(points) {
        sxy += (x - mx) * (p.pct - my);
        sxx += (x - mx) * (x - mx);
    }
    if sxx == 0.0 {
        0.0
    } else {
        sxy / sxx
    }
}

/// Pace of one window from its readings (oldest first) at `now`. None without readings, or when
/// the newest reading's reset time has already passed (its percentage no longer applies).
pub fn window_pace(points: &[Point], now: i64, window_ms: i64) -> Option<WindowPace> {
    let last = points.last()?;
    let reset = parse_ms(&last.resets_at);
    if reset.is_some_and(|r| r <= now) {
        return None;
    }
    let segment = current_segment(points);
    let recent: Vec<Point> = segment
        .iter()
        .filter(|p| p.t > now - SLOPE_SPAN_MS && p.t <= now)
        .cloned()
        .collect();
    let enough = recent.len() >= MIN_POINTS
        && recent.last().map_or(0, |p| p.t) - recent.first().map_or(0, |p| p.t) >= MIN_SPAN_MS;
    let pct_per_hour = enough.then(|| slope_per_hour(&recent));
    let even_pace_pct = reset.map(|r| {
        let elapsed = (now - (r - window_ms)) as f64 / window_ms as f64;
        elapsed.clamp(0.0, 1.0) * 100.0
    });
    let (state, eta_to_limit) = match pct_per_hour {
        None => ("measuring", None),
        Some(s) if s <= 0.0 => ("lasts", None),
        Some(s) => {
            let eta = now + (((100.0 - last.pct).max(0.0) / s) * HOUR_MS as f64) as i64;
            match reset {
                Some(r) if eta < r => ("limit", Some(eta)),
                _ => ("lasts", None),
            }
        }
    };
    Some(WindowPace {
        pct: last.pct,
        pct_per_hour,
        even_pace_pct,
        eta_to_limit,
        resets_at: last.resets_at.clone(),
        state,
    })
}

/// The points of one window from readings (one provider and account, oldest first).
pub fn points_of(readings: &[Reading], window: &str) -> Vec<Point> {
    readings
        .iter()
        .filter_map(|r| {
            r.w.get(window).map(|w| Point {
                t: r.t,
                pct: w.pct,
                resets_at: w.resets_at.clone(),
            })
        })
        .collect()
}

/// Dollars per weekly percentage point over the current weekly window: spend between the first
/// reading of this window and now, divided by the points gained since that reading. None below
/// 2 points gained. `spend_since(t)` gives the dollars spent from `t` to now.
pub fn usd_per_weekly_pct(
    weekly: &[Point],
    now: i64,
    spend_since: impl Fn(i64) -> f64,
) -> Option<f64> {
    let last = weekly.last()?;
    let reset = parse_ms(&last.resets_at)?;
    if reset <= now {
        return None;
    }
    let start = reset - 7 * DAY_MS;
    let segment = current_segment(weekly);
    let first = segment.iter().find(|p| p.t >= start)?;
    let gained = last.pct - first.pct;
    (gained >= 2.0).then(|| spend_since(first.t) / gained)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn iso(ms: i64) -> Option<String> {
        DateTime::from_timestamp_millis(ms).map(|d| d.to_rfc3339())
    }

    fn pt(mins_ago: i64, pct: f64, reset: i64) -> Point {
        Point {
            t: NOW - mins_ago * MINUTE_MS,
            pct,
            resets_at: iso(reset),
        }
    }

    fn reading(t: i64, a: &str) -> Reading {
        let mut w = BTreeMap::new();
        w.insert(
            FIVE_HOUR.to_string(),
            WindowReading {
                pct: 1.0,
                resets_at: None,
            },
        );
        Reading {
            t,
            p: "claude".into(),
            a: a.into(),
            w,
        }
    }

    /// A reading every 10 min over the last hour, climbing `per_hour` to `pct_now`.
    fn climb(pct_now: f64, per_hour: f64, reset: i64) -> Vec<Point> {
        (0..=6)
            .rev()
            .map(|i| pt(i * 10, pct_now - per_hour * i as f64 / 6.0, reset))
            .collect()
    }

    #[test]
    fn steady_climb_projects_the_limit_before_reset() {
        let reset = NOW + 4 * HOUR_MS;
        let w = window_pace(&climb(80.0, 10.0, reset), NOW, 5 * HOUR_MS).unwrap();
        assert!((w.pct_per_hour.unwrap() - 10.0).abs() < 1e-9);
        // 20 % left at 10 %/h = 2 h, before the reset in 4 h
        assert_eq!(
            (w.state, w.eta_to_limit),
            ("limit", Some(NOW + 2 * HOUR_MS))
        );
        // the 5 h window started 1 h ago: 20 % of it has passed
        assert!((w.even_pace_pct.unwrap() - 20.0).abs() < 1e-9);
        assert_eq!(w.pct, 80.0);
    }

    #[test]
    fn slow_climb_lasts_until_reset() {
        let reset = NOW + HOUR_MS;
        let w = window_pace(&climb(50.0, 10.0, reset), NOW, 5 * HOUR_MS).unwrap();
        assert_eq!((w.state, w.eta_to_limit), ("lasts", None));
    }

    #[test]
    fn flat_or_falling_speed_lasts() {
        let reset = NOW + 3 * HOUR_MS;
        let flat = window_pace(&climb(30.0, 0.0, reset), NOW, 5 * HOUR_MS).unwrap();
        assert_eq!(flat.pct_per_hour, Some(0.0));
        assert_eq!((flat.state, flat.eta_to_limit), ("lasts", None));
        // a small downward wobble (under the reset threshold) gives a negative slope
        let pts = vec![pt(40, 30.4, reset), pt(20, 30.2, reset), pt(0, 30.0, reset)];
        let falling = window_pace(&pts, NOW, 5 * HOUR_MS).unwrap();
        assert!(falling.pct_per_hour.unwrap() < 0.0);
        assert_eq!(falling.state, "lasts");
    }

    #[test]
    fn too_few_readings_or_too_short_is_measuring() {
        let reset = NOW + 3 * HOUR_MS;
        let two = vec![pt(30, 10.0, reset), pt(0, 20.0, reset)];
        let w = window_pace(&two, NOW, 5 * HOUR_MS).unwrap();
        assert_eq!(
            (w.state, w.pct_per_hour, w.eta_to_limit),
            ("measuring", None, None)
        );
        let short = vec![pt(10, 10.0, reset), pt(5, 15.0, reset), pt(0, 20.0, reset)];
        assert_eq!(
            window_pace(&short, NOW, 5 * HOUR_MS).unwrap().state,
            "measuring"
        );
        // readings older than 60 min do not count
        let old = vec![pt(90, 1.0, reset), pt(70, 2.0, reset), pt(0, 3.0, reset)];
        assert_eq!(
            window_pace(&old, NOW, 5 * HOUR_MS).unwrap().state,
            "measuring"
        );
        assert_eq!(window_pace(&[], NOW, 5 * HOUR_MS), None);
    }

    #[test]
    fn a_reset_inside_the_hour_starts_a_new_segment() {
        let old_reset = NOW - 25 * MINUTE_MS;
        let new_reset = NOW + 5 * HOUR_MS - 25 * MINUTE_MS;
        let pts = vec![
            pt(55, 90.0, old_reset),
            pt(45, 95.0, old_reset),
            pt(35, 99.0, old_reset),
            pt(20, 1.0, new_reset),
            pt(10, 2.0, new_reset),
            pt(0, 3.0, new_reset),
        ];
        assert_eq!(current_segment(&pts).len(), 3);
        let w = window_pace(&pts, NOW, 5 * HOUR_MS).unwrap();
        // only the 3 readings after the reset: 1 -> 3 over 20 min = 6 %/h, not the old climb
        assert!((w.pct_per_hour.unwrap() - 6.0).abs() < 1e-9);
        assert_eq!(w.state, "lasts");
        // a drop with an unchanged reset time also counts as a reset
        let same = vec![pt(30, 80.0, NOW + HOUR_MS), pt(20, 2.0, NOW + HOUR_MS)];
        assert_eq!(current_segment(&same).len(), 1);
    }

    #[test]
    fn a_passed_reset_means_no_window() {
        let pts = vec![pt(30, 10.0, NOW - MINUTE_MS)];
        assert_eq!(window_pace(&pts, NOW, 5 * HOUR_MS), None);
    }

    #[test]
    fn usd_per_weekly_pct_needs_two_points() {
        let reset = NOW + 3 * DAY_MS;
        let spend = |t: i64| (NOW - t) as f64 / HOUR_MS as f64 * 100.0; // $100/h
        let gained = vec![
            pt(600, 40.0, reset),
            pt(300, 41.0, reset),
            pt(0, 45.0, reset),
        ];
        // 10 h at $100/h over 5 points
        let usd = usd_per_weekly_pct(&gained, NOW, spend).unwrap();
        assert!((usd - 200.0).abs() < 1e-9);
        let little = vec![pt(600, 40.0, reset), pt(0, 41.5, reset)];
        assert_eq!(usd_per_weekly_pct(&little, NOW, spend), None);
        // the first reading of an earlier week is not used
        let mut across = vec![pt(900, 90.0, NOW - 4 * DAY_MS)];
        across.extend(gained);
        assert!((usd_per_weekly_pct(&across, NOW, spend).unwrap() - 200.0).abs() < 1e-9);
        assert_eq!(usd_per_weekly_pct(&[], NOW, spend), None);
    }

    #[test]
    fn thinning_keeps_recent_detail_and_one_per_five_minutes_after_a_day() {
        let mut rs = Vec::new();
        // every minute for the last 2 days, two accounts
        for m in 0..(2 * 24 * 60) {
            rs.push(reading(NOW - m * MINUTE_MS, "aaaa"));
            rs.push(reading(NOW - m * MINUTE_MS, "bbbb"));
        }
        rs.push(reading(NOW - 15 * DAY_MS, "aaaa"));
        let kept = thin(rs, NOW);
        let recent = kept.iter().filter(|r| r.t >= NOW - DAY_MS).count();
        let older = kept.iter().filter(|r| r.t < NOW - DAY_MS).count();
        assert_eq!(recent, 2 * (24 * 60 + 1));
        // 24 h of one-per-minute readings -> about 288 five-minute slots per account
        assert!((2 * 287..=2 * 289).contains(&older), "{older}");
        assert!(kept.iter().all(|r| r.t >= NOW - 14 * DAY_MS));
        assert!(kept.windows(2).all(|w| w[0].t <= w[1].t));
    }

    #[test]
    fn append_and_load_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("usagedeck-test-history-{}", std::process::id()));
        let file = dir.join(HISTORY_FILE);
        let _ = std::fs::remove_file(&file);
        append(&file, &reading(NOW - MINUTE_MS, "aaaa"), NOW);
        append(&file, &reading(NOW, "aaaa"), NOW);
        let all = load_all(&file);
        assert_eq!(all.len(), 2);
        assert_eq!(points_of(&all, FIVE_HOUR).len(), 2);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains('@'));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
