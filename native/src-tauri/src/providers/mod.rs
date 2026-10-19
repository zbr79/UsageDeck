//! Shared provider framework.
//!
//! Ports `lib/provider-cache.ts` (TTL, one fetch in flight per key, persisted last-good value),
//! `lib/usage-cache.ts` (`usage-cache.json`), `lib/tracked-accounts.ts`, `lib/seen-accounts.ts`
//! and `lib/account-key.ts`.
//!
//! The rule every provider inherits: only a reply that deserialised into its typed struct is
//! written to the cache. A failed or invalid reply never overwrites the last good value; the
//! caller gets that cached value back, marked `stale`.

pub mod claude;
pub mod codex;
pub mod cursor;
pub mod grok;
pub mod opencode;
pub mod util;

use crate::store::{read_json, write_json_atomic};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

// ---- errors ---------------------------------------------------------------------------------

/// Why a provider fetch produced no fresh data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    #[error("missing-credentials")]
    MissingCredentials,
    #[error("auth-rejected")]
    AuthRejected,
    #[error("rate-limited")]
    RateLimited,
    #[error("network")]
    Network,
    #[error("http {0}")]
    Http(u16),
    #[error("schema-changed")]
    SchemaChanged,
    #[error("locked")]
    Locked,
}

impl ProviderError {
    /// The provider rejected the sign-in or we found only an expired one.
    pub fn is_auth_expired(self) -> bool {
        matches!(self, ProviderError::AuthRejected)
    }
}

/// A failed fetch. `fallback` is shown only when there is no cached value either (never persisted).
/// Boxed so the `Err` side of a fetch stays small.
#[derive(Clone, Debug)]
pub struct FetchError<T> {
    pub error: ProviderError,
    pub fallback: Option<Box<T>>,
}

impl<T> From<ProviderError> for FetchError<T> {
    fn from(error: ProviderError) -> Self {
        Self {
            error,
            fallback: None,
        }
    }
}

/// Deserialise a provider reply into its typed struct; anything else is `SchemaChanged`.
pub fn parse_reply<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProviderError> {
    serde_json::from_slice(bytes).map_err(|e| {
        log::warn!("provider reply did not match its type: {e}");
        ProviderError::SchemaChanged
    })
}

/// Send a request and deserialise a 2xx JSON reply. 401/403 -> AuthRejected, 429 -> RateLimited,
/// other non-2xx -> Http(code), transport errors -> Network, wrong shape -> SchemaChanged.
/// Providers whose service uses other codes can map the reply themselves.
pub async fn fetch_json<T: DeserializeOwned>(
    req: reqwest::RequestBuilder,
) -> Result<T, ProviderError> {
    let resp = req.send().await.map_err(|e| {
        log::warn!("provider request failed: {e}");
        ProviderError::Network
    })?;
    let status = resp.status().as_u16();
    match status {
        401 | 403 => return Err(ProviderError::AuthRejected),
        429 => return Err(ProviderError::RateLimited),
        s if !(200..300).contains(&s) => return Err(ProviderError::Http(s)),
        _ => {}
    }
    let bytes = resp.bytes().await.map_err(|_| ProviderError::Network)?;
    parse_reply(&bytes)
}

// ---- JSON shapes (lib/types.ts) -------------------------------------------------------------

/// Result of one cached provider lookup (`ProviderResult` in lib/provider-cache.ts).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderResult<T> {
    pub usage: Option<T>,
    /// true when `usage` is last-known data from disk, not a live fetch
    pub stale: bool,
    pub cached_at: Option<i64>,
    pub auth_expired: bool,
    /// why the live fetch failed (not part of the JSON contract)
    #[serde(skip)]
    pub error: Option<ProviderError>,
}

/// One signed-in account of Cursor, Grok Bot, Codex or Claude (`TrackedAccount` in lib/types.ts).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedAccount<T> {
    pub id: String,
    pub label: Option<String>,
    pub active: bool,
    pub usage: Option<T>,
    pub stale: bool,
    pub cached_at: Option<i64>,
    pub auth_expired: bool,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---- usage-cache.json (lib/usage-cache.ts) --------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Cached<T> {
    pub data: T,
    pub saved_at: i64,
}

/// Last-known usage per provider section and account key: `{ section: { key: { savedAt, data } } }`.
pub struct UsageCache {
    file: PathBuf,
    state: AsyncMutex<Option<Map<String, Value>>>,
}

impl UsageCache {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            state: AsyncMutex::new(None),
        }
    }

    fn read_disk(&self) -> Map<String, Value> {
        match read_json(&self.file) {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        }
    }

    /// The cached value, or None when absent, null, or no longer of type `T`.
    pub async fn read<T: DeserializeOwned>(&self, section: &str, key: &str) -> Option<Cached<T>> {
        let mut state = self.state.lock().await;
        if state.is_none() {
            *state = Some(self.read_disk());
        }
        let entry = state.as_ref()?.get(section)?.get(key)?;
        let data = entry.get("data")?;
        if data.is_null() {
            return None;
        }
        let data = serde_json::from_value(data.clone()).ok()?;
        let saved_at = entry.get("savedAt").and_then(Value::as_i64).unwrap_or(0);
        Some(Cached { data, saved_at })
    }

    /// Store a good value and write the file at once, merged with what is on disk (newest entry
    /// wins) so other sections are never clobbered. Best-effort: a write error is only logged.
    pub async fn write<T: Serialize>(&self, section: &str, key: &str, data: &T) {
        let Ok(data) = serde_json::to_value(data) else {
            return;
        };
        let mut state = self.state.lock().await;
        if state.is_none() {
            *state = Some(self.read_disk());
        }
        let mine = state.as_mut().expect("loaded above");
        let entry = serde_json::json!({ "savedAt": now_ms(), "data": data });
        if !mine.get(section).is_some_and(Value::is_object) {
            mine.insert(section.to_string(), Value::Object(Map::new()));
        }
        if let Some(Value::Object(s)) = mine.get_mut(section) {
            s.insert(key.to_string(), entry);
        }
        let mut merged = self.read_disk();
        for (section, entries) in mine.iter() {
            let Some(entries) = entries.as_object() else {
                continue;
            };
            if !merged.get(section).is_some_and(Value::is_object) {
                merged.insert(section.clone(), Value::Object(Map::new()));
            }
            let Some(Value::Object(target)) = merged.get_mut(section) else {
                continue;
            };
            for (k, entry) in entries {
                let ours = entry.get("savedAt").and_then(Value::as_i64).unwrap_or(0);
                let theirs = target
                    .get(k)
                    .and_then(|t| t.get("savedAt"))
                    .and_then(Value::as_i64);
                if theirs.is_none_or(|t| t <= ours) {
                    target.insert(k.clone(), entry.clone());
                }
            }
        }
        if let Err(e) = write_json_atomic(&self.file, &merged) {
            log::warn!("usage cache write failed: {e}");
        }
    }
}

// ---- cached provider (lib/provider-cache.ts) ------------------------------------------------

struct Memo<T> {
    at: Instant,
    ok: bool,
    result: ProviderResult<T>,
}

/// The caching policy every provider shares:
///  - successful results are reused for `ttl`, failures for `failure_ttl`;
///  - concurrent callers for one key share a single fetch (the second waits, then reuses it);
///  - on failure, the last good value from disk comes back marked stale.
///
/// Hold one per provider, e.g. `static CACHE: LazyLock<CachedProvider<CursorUsage>>`.
pub struct CachedProvider<T> {
    section: &'static str,
    ttl: Duration,
    failure_ttl: Duration,
    memo: Mutex<HashMap<String, Memo<T>>>,
    gates: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl<T> CachedProvider<T>
where
    T: Clone + Serialize + DeserializeOwned + Send,
{
    pub fn new(section: &'static str, ttl: Duration, failure_ttl: Option<Duration>) -> Self {
        Self {
            section,
            ttl,
            failure_ttl: failure_ttl.unwrap_or(ttl),
            memo: Mutex::new(HashMap::new()),
            gates: Mutex::new(HashMap::new()),
        }
    }

    pub fn section(&self) -> &'static str {
        self.section
    }

    fn fresh(&self, key: &str) -> Option<ProviderResult<T>> {
        let memo = self.memo.lock().unwrap();
        let hit = memo.get(key)?;
        let ttl = if hit.ok { self.ttl } else { self.failure_ttl };
        (hit.at.elapsed() < ttl).then(|| hit.result.clone())
    }

    /// Cached result for `key` (use "default" for one-account providers), fetching when expired.
    pub async fn get<F, Fut>(&self, cache: &UsageCache, key: &str, fetch: F) -> ProviderResult<T>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = Result<T, FetchError<T>>>,
    {
        if let Some(hit) = self.fresh(key) {
            return hit;
        }
        let gate = {
            let mut gates = self.gates.lock().unwrap();
            gates.entry(key.to_string()).or_default().clone()
        };
        let _in_flight = gate.lock().await;
        if let Some(hit) = self.fresh(key) {
            return hit;
        }
        let (ok, result) = match fetch(key.to_string()).await {
            Ok(data) => {
                cache.write(self.section, key, &data).await;
                let result = ProviderResult {
                    usage: Some(data),
                    stale: false,
                    cached_at: None,
                    auth_expired: false,
                    error: None,
                };
                (true, result)
            }
            Err(FetchError { error, fallback }) => {
                log::info!("{} [{key}]: {error}", self.section);
                let auth_expired = error.is_auth_expired();
                let result = match cache.read::<T>(self.section, key).await {
                    Some(cached) => ProviderResult {
                        usage: Some(cached.data),
                        stale: true,
                        cached_at: Some(cached.saved_at),
                        auth_expired,
                        error: Some(error),
                    },
                    None => ProviderResult {
                        usage: fallback.map(|b| *b),
                        stale: false,
                        cached_at: None,
                        auth_expired,
                        error: Some(error),
                    },
                };
                (false, result)
            }
        };
        self.memo.lock().unwrap().insert(
            key.to_string(),
            Memo {
                at: Instant::now(),
                ok,
                result: result.clone(),
            },
        );
        result
    }

    /// Forget the memo for one key, or all keys, so the next `get` fetches again.
    pub fn invalidate(&self, key: Option<&str>) {
        let mut memo = self.memo.lock().unwrap();
        match key {
            Some(k) => {
                memo.remove(k);
            }
            None => memo.clear(),
        }
    }
}

// ---- seen-accounts.json (lib/seen-accounts.ts) ----------------------------------------------

/// An account identity this monitor has seen. Tokens are never stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenAccount {
    pub id: String,
    pub label: Option<String>,
}

const MAX_SEEN: usize = 50;
const MAX_ID: usize = 320;

fn normalize_id(id: &str) -> String {
    id.trim().to_lowercase()
}

fn clean_label(label: Option<&str>) -> Option<String> {
    let t = label?.trim();
    (!t.is_empty()).then(|| t.chars().take(MAX_ID).collect())
}

/// Keep the newest `MAX_SEEN` entries (the end of the list).
fn keep_newest(mut list: Vec<SeenAccount>) -> Vec<SeenAccount> {
    if list.len() > MAX_SEEN {
        list.drain(..list.len() - MAX_SEEN);
    }
    list
}

pub struct SeenAccounts {
    file: PathBuf,
    lock: AsyncMutex<()>,
}

impl SeenAccounts {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            lock: AsyncMutex::new(()),
        }
    }

    fn roster(&self) -> Vec<(String, Vec<SeenAccount>)> {
        let Some(Value::Object(map)) = read_json(&self.file) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (provider, rows) in map {
            let Value::Array(rows) = rows else { continue };
            if provider.is_empty() || provider.chars().count() > 32 {
                continue;
            }
            let mut seen = HashSet::new();
            let mut list = Vec::new();
            for row in rows {
                let id = row
                    .get("id")
                    .and_then(Value::as_str)
                    .map(normalize_id)
                    .unwrap_or_default();
                if id.is_empty() || id.chars().count() > MAX_ID || !seen.insert(id.clone()) {
                    continue;
                }
                let label = clean_label(row.get("label").and_then(Value::as_str));
                list.push(SeenAccount { id, label });
            }
            out.push((provider, keep_newest(list)));
        }
        out
    }

    pub fn remembered(&self, provider: &str) -> Vec<SeenAccount> {
        self.roster()
            .into_iter()
            .find(|(p, _)| p == provider)
            .map(|(_, list)| list)
            .unwrap_or_default()
    }

    /// Add identities. Labels fill in when learned; an existing label is kept if the new one is empty.
    pub async fn remember(&self, provider: &str, accounts: &[SeenAccount]) {
        let incoming: Vec<SeenAccount> = accounts
            .iter()
            .map(|a| SeenAccount {
                id: normalize_id(&a.id),
                label: clean_label(a.label.as_deref()),
            })
            .filter(|a| !a.id.is_empty() && a.id.chars().count() <= MAX_ID)
            .collect();
        if incoming.is_empty() {
            return;
        }
        let _guard = self.lock.lock().await;
        let mut roster = self.roster();
        let idx = match roster.iter().position(|(p, _)| p == provider) {
            Some(i) => i,
            None => {
                roster.push((provider.to_string(), Vec::new()));
                roster.len() - 1
            }
        };
        let list = &mut roster[idx].1;
        let mut changed = false;
        for account in incoming {
            match list.iter_mut().find(|a| a.id == account.id) {
                Some(prev) => {
                    let label = account.label.or_else(|| prev.label.clone());
                    if prev.label != label {
                        prev.label = label;
                        changed = true;
                    }
                }
                None => {
                    list.push(account);
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
        // The old code kept the oldest 50 (`slice(0, 50)`); keep the newest instead.
        *list = keep_newest(std::mem::take(list));
        let out: Map<String, Value> = roster
            .into_iter()
            .map(|(p, list)| {
                (
                    p,
                    serde_json::to_value(list).unwrap_or(Value::Array(Vec::new())),
                )
            })
            .collect();
        if let Err(e) = write_json_atomic(&self.file, &out) {
            log::warn!("seen-accounts write failed: {e}");
        }
    }
}

// ---- account keys (lib/account-key.ts) ------------------------------------------------------

/// Settings key for one account, prefixed so it cannot collide with an OpenCode email.
pub fn account_key(provider: &str, id: &str) -> String {
    format!("{provider}:{}", id.trim().to_lowercase())
}

pub fn is_account_hidden(disabled: &[String], provider: &str, id: &str) -> bool {
    let key = account_key(provider, id);
    disabled.contains(&key)
}

// ---- tracked accounts (lib/tracked-accounts.ts) ---------------------------------------------

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A sign-in the provider's app holds right now.
pub struct LiveAccount<'a, T> {
    pub id: String,
    pub label: Option<String>,
    pub active: bool,
    /// usually `Box::pin(CACHE.get(&core.cache, &id, fetch))`
    pub load: BoxFuture<'a, ProviderResult<T>>,
    /// Email or name learned from the usage payload, once a fetch succeeds.
    pub label_from: Option<fn(&T) -> Option<String>>,
}

/// Live sign-ins plus every account seen before. Accounts the app is no longer signed into keep
/// their last cached reading and are not fetched again. Live loads run one after another.
pub async fn collect_accounts<'a, T>(
    seen: &SeenAccounts,
    cache: &UsageCache,
    provider: &str,
    section: &str,
    live: Vec<LiveAccount<'a, T>>,
) -> Vec<TrackedAccount<T>>
where
    T: Clone + DeserializeOwned,
{
    let mut usable: Vec<LiveAccount<'a, T>> = live
        .into_iter()
        .map(|mut a| {
            a.id = normalize_id(&a.id);
            a
        })
        .filter(|a| !a.id.is_empty())
        .collect();
    let remember: Vec<SeenAccount> = usable
        .iter()
        .map(|a| SeenAccount {
            id: a.id.clone(),
            label: a.label.clone(),
        })
        .collect();
    seen.remember(provider, &remember).await;
    let mut ids = seen.remembered(provider);
    for a in &usable {
        if !ids.iter().any(|s| s.id == a.id) {
            ids.push(SeenAccount {
                id: a.id.clone(),
                label: a.label.clone(),
            });
        }
    }
    let live_ids: HashSet<String> = usable.iter().map(|a| a.id.clone()).collect();
    let single = ids.len() == 1;

    let mut rows = Vec::with_capacity(ids.len());
    for entry in &ids {
        let Some(pos) = usable.iter().position(|a| a.id == entry.id) else {
            let cached = cache.read::<T>(section, &entry.id).await;
            rows.push(TrackedAccount {
                id: entry.id.clone(),
                label: entry.label.clone(),
                active: false,
                stale: cached.is_some(),
                cached_at: cached.as_ref().map(|c| c.saved_at),
                usage: cached.map(|c| c.data),
                auth_expired: false,
            });
            continue;
        };
        let current = usable.swap_remove(pos);
        let result = current.load.await;
        let mut usage = result.usage;
        let mut stale = result.stale;
        let mut cached_at = result.cached_at;
        // Older builds cached the single active account under "default".
        if usage.is_none() && single {
            if let Some(legacy) = cache.read::<T>(section, "default").await {
                usage = Some(legacy.data);
                stale = true;
                cached_at = Some(legacy.saved_at);
            }
        }
        let learned = current
            .label_from
            .and_then(|f| usage.as_ref().and_then(f))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let label = learned.or(current.label).or_else(|| entry.label.clone());
        rows.push(TrackedAccount {
            id: entry.id.clone(),
            label,
            active: current.active,
            usage,
            stale,
            cached_at,
            auth_expired: result.auth_expired,
        });
    }

    let learned: Vec<SeenAccount> = rows
        .iter()
        .filter(|r| r.label.is_some() && live_ids.contains(&r.id))
        .map(|r| SeenAccount {
            id: r.id.clone(),
            label: r.label.clone(),
        })
        .collect();
    if !learned.is_empty() {
        seen.remember(provider, &learned).await;
    }

    rows.sort_by(|a, b| {
        b.active.cmp(&a.active).then_with(|| {
            let la = a.label.as_deref().unwrap_or(&a.id).to_lowercase();
            let lb = b.label.as_deref().unwrap_or(&b.id).to_lowercase();
            la.cmp(&lb)
        })
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Usage {
        percent_used: f64,
    }

    #[tokio::test]
    async fn cache_keeps_last_good_on_failure_and_invalid_reply() {
        let dir = temp_dir("cache");
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let provider: CachedProvider<Usage> =
            CachedProvider::new("test", Duration::from_secs(60), None);

        let good = provider
            .get(&cache, "a@x.com", |_| async {
                parse_reply::<Usage>(br#"{"percentUsed": 42.0}"#).map_err(FetchError::from)
            })
            .await;
        assert_eq!(good.usage, Some(Usage { percent_used: 42.0 }));
        assert!(!good.stale);

        // within the TTL the memo answers without fetching
        let memo = provider
            .get(&cache, "a@x.com", |_| async { panic!("must not fetch") })
            .await;
        assert_eq!(memo, good);

        // invalid reply: does not deserialise, so it is a failure and the cache keeps 42
        provider.invalidate(Some("a@x.com"));
        let invalid = provider
            .get(&cache, "a@x.com", |_| async {
                parse_reply::<Usage>(br#"{"percentUsed": null}"#).map_err(FetchError::from)
            })
            .await;
        assert_eq!(invalid.usage, Some(Usage { percent_used: 42.0 }));
        assert!(invalid.stale);
        assert!(invalid.cached_at.is_some());
        assert_eq!(invalid.error, Some(ProviderError::SchemaChanged));

        // failed fetch (auth rejected) after a restart: last good value comes from disk
        let cache2 = UsageCache::new(dir.join("usage-cache.json"));
        let provider2: CachedProvider<Usage> =
            CachedProvider::new("test", Duration::from_secs(60), None);
        let failed = provider2
            .get(&cache2, "a@x.com", |_| async {
                Err(ProviderError::AuthRejected.into())
            })
            .await;
        assert_eq!(failed.usage, Some(Usage { percent_used: 42.0 }));
        assert!(failed.stale && failed.auth_expired);

        // nothing cached: the fallback is shown, not persisted
        let fresh = provider2
            .get(&cache2, "b@x.com", |_| async {
                Err(FetchError {
                    error: ProviderError::Network,
                    fallback: Some(Box::new(Usage { percent_used: 0.0 })),
                })
            })
            .await;
        assert_eq!(fresh.usage, Some(Usage { percent_used: 0.0 }));
        assert!(!fresh.stale);
        assert!(cache2.read::<Usage>("test", "b@x.com").await.is_none());

        let disk = read_json(&dir.join("usage-cache.json")).unwrap();
        assert_eq!(
            disk["test"]["a@x.com"]["data"],
            json!({ "percentUsed": 42.0 })
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn seen_accounts_keep_newest_50() {
        let dir = temp_dir("seen");
        let seen = SeenAccounts::new(dir.join("seen-accounts.json"));
        for i in 0..60 {
            seen.remember(
                "cursor",
                &[SeenAccount {
                    id: format!("User{i}@X.com"),
                    label: None,
                }],
            )
            .await;
        }
        let list = seen.remembered("cursor");
        assert_eq!(list.len(), 50);
        assert_eq!(list.first().unwrap().id, "user10@x.com");
        assert_eq!(list.last().unwrap().id, "user59@x.com");

        // a label fills in; an empty label never erases it
        seen.remember(
            "cursor",
            &[SeenAccount {
                id: "user59@x.com".into(),
                label: Some(" Name ".into()),
            }],
        )
        .await;
        seen.remember(
            "cursor",
            &[SeenAccount {
                id: "user59@x.com".into(),
                label: Some("  ".into()),
            }],
        )
        .await;
        assert_eq!(
            seen.remembered("cursor").last().unwrap().label.as_deref(),
            Some("Name")
        );
        assert!(seen.remembered("grok").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn account_keys() {
        assert_eq!(account_key("codex", "  A@B.com "), "codex:a@b.com");
        assert!(is_account_hidden(&["claude:x".to_string()], "claude", "X"));
    }
}
