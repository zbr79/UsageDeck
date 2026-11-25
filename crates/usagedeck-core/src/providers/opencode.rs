//! OpenCode Go usage per stored API key. Port of lib/opencode.ts (getUsage, parseOpenCodeUsage,
//! diffText, getLastChange) and app/api/usage/route.ts. Keys come from the `Store`
//! (Windows Credential Manager); the persisted last-good value lives in `UsageCache`.

use super::{parse_reply, CachedProvider, FetchError, ProviderError};
use crate::store::{Account, OPENCODE_KEY_PREFIXES};
use crate::Core;
use rusqlite::OpenFlags;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";
const SECTION: &str = "opencode";

/// `UsageWindow` in lib/types.ts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub status: String,
    pub percent: f64,
    pub resets_at: String,
}

/// `OpenCodeUsage` in lib/types.ts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenCodeUsage {
    pub rolling: UsageWindow,
    pub weekly: UsageWindow,
    pub monthly: UsageWindow,
}

/// A window of the reply: `percent` must be a number; status and resetsAt stay lenient.
#[derive(Deserialize)]
struct RawWindow {
    #[serde(default)]
    status: Value,
    percent: f64,
    #[serde(default, rename = "resetsAt")]
    resets_at: Value,
}

#[derive(Deserialize)]
struct RawUsage {
    rolling: RawWindow,
    weekly: RawWindow,
    monthly: RawWindow,
}

#[derive(Deserialize)]
struct RawReply {
    usage: RawUsage,
}

fn window(w: RawWindow) -> UsageWindow {
    UsageWindow {
        status: w.status.as_str().unwrap_or("ok").to_string(),
        percent: w.percent,
        resets_at: w.resets_at.as_str().unwrap_or("").to_string(),
    }
}

/// Port of `parseOpenCodeUsage`: all three windows or `SchemaChanged`.
pub fn parse_opencode_usage(bytes: &[u8]) -> Result<OpenCodeUsage, ProviderError> {
    let raw: RawReply = parse_reply(bytes)?;
    Ok(OpenCodeUsage {
        rolling: window(raw.usage.rolling),
        weekly: window(raw.usage.weekly),
        monthly: window(raw.usage.monthly),
    })
}

pub fn diff_text(prev: &OpenCodeUsage, cur: &OpenCodeUsage) -> String {
    let pairs = [
        ("rolling", &prev.rolling, &cur.rolling),
        ("weekly", &prev.weekly, &cur.weekly),
        ("monthly", &prev.monthly, &cur.monthly),
    ];
    pairs
        .iter()
        .filter(|(_, p, c)| p.percent != c.percent)
        .map(|(name, p, c)| format!("{name} {}% → {}%", p.percent, c.percent))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---- read-only auto-discovery ---------------------------------------------------------------

/// Whether a string looks like an OpenCode API key (`oc_sk_...` or `sk-...`).
pub fn is_valid_opencode_key(key: &str) -> bool {
    let trimmed = key.trim();
    OPENCODE_KEY_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
        && trimmed.len() > 8
}

fn extract_key_from_value(value: &Value) -> Option<String> {
    let candidate = value
        .as_str()
        .or_else(|| value.get("key").and_then(Value::as_str))
        .or_else(|| value.get("apiKey").and_then(Value::as_str))
        .or_else(|| value.get("api_key").and_then(Value::as_str))
        .or_else(|| value.get("token").and_then(Value::as_str))
        .map(str::trim)?;
    is_valid_opencode_key(candidate).then(|| candidate.to_string())
}

/// Read-only discovery from an OpenCode `auth.json` file (`opencode-go.key` or `opencode.key`).
pub fn discover_from_auth_json(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    for field in ["opencode-go", "opencode"] {
        if let Some(key) = json.get(field).and_then(extract_key_from_value) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Read-only discovery from OpenCode's SQLite database (`opencode.db`).
pub fn discover_from_sqlite(path: &Path) -> Vec<String> {
    if !path.is_file() {
        return Vec::new();
    }
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return Vec::new();
    };
    let _ = conn.busy_timeout(Duration::from_millis(500));
    let Ok(mut stmt) = conn.prepare(
        "SELECT value FROM credential \
         WHERE integration_id IN ('opencode','opencode-go') AND active IS NOT 0",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    for raw in rows.flatten() {
        let trimmed = raw.trim();
        let extracted = if trimmed.starts_with('{') {
            serde_json::from_str::<Value>(trimmed)
                .ok()
                .and_then(|v| extract_key_from_value(&v))
        } else {
            is_valid_opencode_key(trimmed).then(|| trimmed.to_string())
        };
        if let Some(key) = extracted {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Discover read-only local OpenCode keys from env, `auth.json`, and `opencode.db`.
/// Never writes or mutates `accounts.json` or any external OpenCode file.
fn discover_local_accounts() -> Vec<Account> {
    if !super::codex::real_sign_ins() {
        return Vec::new();
    }
    let mut found: Vec<Account> = Vec::new();

    if let Ok(env_key) = std::env::var("OPENCODE_API_KEY") {
        let trimmed = env_key.trim();
        if is_valid_opencode_key(trimmed) {
            found.push(Account {
                email: "opencode-env".to_string(),
                key: trimmed.to_string(),
            });
        }
    }

    let mut dirs = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        dirs.push(local.join("opencode"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
    {
        dirs.push(home.join(".local").join("share").join("opencode"));
    }

    for dir in &dirs {
        for key in discover_from_auth_json(&dir.join("auth.json")) {
            found.push(Account {
                email: "opencode-cli".to_string(),
                key,
            });
        }
    }

    for dir in &dirs {
        for key in discover_from_sqlite(&dir.join("opencode.db")) {
            found.push(Account {
                email: "opencode-db".to_string(),
                key,
            });
        }
    }

    found
}

/// Merge Credential Manager accounts with read-only discovered keys, deduplicating by key and
/// guaranteeing unique synthetic labels without ever mutating `accounts.json`.
pub fn merge_discovered_accounts(stored: Vec<Account>, discovered: Vec<Account>) -> Vec<Account> {
    let mut seen_keys: HashSet<String> = stored
        .iter()
        .map(|a| a.key.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    let mut seen_ids: HashSet<String> = stored.iter().map(|a| a.email.to_lowercase()).collect();
    let mut merged = stored;

    for mut acc in discovered {
        let trimmed_key = acc.key.trim().to_string();
        if !is_valid_opencode_key(&trimmed_key) || !seen_keys.insert(trimmed_key.clone()) {
            continue;
        }
        acc.key = trimmed_key;
        let base = acc.email.clone();
        let mut candidate = base.clone();
        let mut suffix = 2u32;
        while !seen_ids.insert(candidate.to_lowercase()) {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        acc.email = candidate;
        merged.push(acc);
    }

    merged
}

// ---- per-account state, keyed by email (as in lib/opencode.ts) ------------------------------

#[derive(Default)]
struct AccountState {
    /// hash of the key in use, so a changed key refetches (the key itself is not kept here)
    key_hash: Option<u64>,
    last_seen: Option<OpenCodeUsage>,
    /// `prev` stays stable across cache hits until the next live fetch
    prev_snapshot: Option<OpenCodeUsage>,
    last_change: Option<(i64, String)>,
    subscription: Option<bool>,
}

static STATE: LazyLock<Mutex<HashMap<String, AccountState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static CACHE: LazyLock<CachedProvider<OpenCodeUsage>> = LazyLock::new(|| {
    CachedProvider::new(
        SECTION,
        Duration::from_secs(10),
        Some(Duration::from_secs(15)),
    )
});

fn with_state<R>(email: &str, f: impl FnOnce(&mut AccountState) -> R) -> R {
    let mut map = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(map.entry(email.to_string()).or_default())
}

/// Map one HTTP reply and update the per-account state.
/// Fix: 401/403 is a rejected key (`AuthRejected`, shown as `authExpired`), not a bare failure.
/// An `EntitlementError` body means the key works but has no Go subscription.
fn handle_reply(email: &str, status: u16, bytes: &[u8]) -> Result<OpenCodeUsage, ProviderError> {
    if !(200..300).contains(&status) {
        let body: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
        let entitlement = body["error"]["type"] == "EntitlementError";
        with_state(email, |s| {
            s.subscription = if entitlement { Some(false) } else { None }
        });
        return Err(match status {
            _ if entitlement => ProviderError::Http(status),
            401 | 403 => ProviderError::AuthRejected,
            429 => ProviderError::RateLimited,
            s => ProviderError::Http(s),
        });
    }
    let data = match parse_opencode_usage(bytes) {
        Ok(d) => d,
        Err(e) => {
            with_state(email, |s| s.subscription = None);
            return Err(e);
        }
    };
    with_state(email, |s| {
        s.subscription = Some(true);
        let prev = s.last_seen.take();
        if let Some(p) = &prev {
            let detail = diff_text(p, &data);
            if !detail.is_empty() {
                s.last_change = Some((chrono::Utc::now().timestamp_millis(), detail));
            }
        }
        s.prev_snapshot = prev;
        s.last_seen = Some(data.clone());
    });
    Ok(data)
}

async fn fetch_usage(
    http: &reqwest::Client,
    email: &str,
    key: &str,
) -> Result<OpenCodeUsage, ProviderError> {
    let resp = http
        .get(USAGE_URL)
        .bearer_auth(key)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            log::warn!("opencode request failed: {}", e.without_url());
            ProviderError::Network
        })?;
    super::note_retry_after(&resp);
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.map_err(|_| ProviderError::Network)?;
    handle_reply(email, status, &bytes)
}

fn key_hash(key: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// One `OpenCodeAccountUsage` row (lib/types.ts), plus `authExpired` for a rejected key.
fn row(email: &str, enabled: bool, result: super::ProviderResult<OpenCodeUsage>) -> Value {
    let (prev, subscription, change) = with_state(email, |s| {
        (
            s.prev_snapshot.clone(),
            s.subscription,
            s.last_change.clone(),
        )
    });
    serde_json::json!({
        "email": email,
        "usage": result.usage,
        "prev": prev,
        "enabled": enabled,
        "error": result.error,
        "stale": result.stale,
        "cachedAt": result.cached_at,
        "subscriptionActive": subscription,
        "lastChangeAt": change.as_ref().map(|c| c.0),
        "lastChangeDetail": change.map(|c| c.1),
        "authExpired": result.auth_expired,
    })
}

/// Rows for the `accounts` array of the old route's JSON body (`OpenCodeAccountUsage` each).
/// Combines Windows Credential Manager accounts with read-only local discovery (`OPENCODE_API_KEY`,
/// `auth.json`, and read-only `opencode.db`), deduplicated by key.
pub async fn accounts(core: &Core) -> Vec<Value> {
    let stored = core.store.get_accounts().await;
    let discovered = tokio::task::spawn_blocking(discover_local_accounts)
        .await
        .unwrap_or_default();
    let accounts = merge_discovered_accounts(stored, discovered);
    if accounts.is_empty() {
        super::record_provider_status(
            SECTION,
            "not found: no saved accounts, OPENCODE_API_KEY, auth.json, or opencode.db",
            false,
        );
        return Vec::new();
    }
    let disabled = core.store.get_settings().disabled_accounts;
    let mut rows = Vec::with_capacity(accounts.len());
    let mut ok_count = 0usize;
    let mut err_reasons: Vec<String> = Vec::new();
    for acc in accounts {
        let email = acc.email.clone();
        let hash = key_hash(&acc.key);
        let changed = with_state(&email, |s| {
            let changed = s.key_hash != Some(hash);
            s.key_hash = Some(hash);
            changed
        });
        if changed {
            CACHE.invalidate(Some(&email));
        }
        let result = CACHE
            .get(&core.cache, &email, |email| async move {
                if acc.key.is_empty() {
                    // the email is listed but its key could not be read from Credential Manager
                    return Err(ProviderError::MissingCredentials.into());
                }
                fetch_usage(&core.http, &email, &acc.key)
                    .await
                    .map_err(FetchError::from)
            })
            .await;
        if result.error.is_none() && result.usage.is_some() {
            ok_count += 1;
        } else if let Some(err) = result.error {
            err_reasons.push(err.to_string());
        }
        let enabled = !disabled.contains(&email.to_lowercase());
        rows.push(row(&email, enabled, result));
    }
    if err_reasons.is_empty() {
        super::record_provider_status(SECTION, format!("ok ({ok_count} active)"), true);
    } else {
        super::record_provider_status(
            SECTION,
            format!("fetch failed: {} ({ok_count} ok)", err_reasons.join("; ")),
            false,
        );
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::UsageCache;

    const OK: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/opencode/ok.json"
    ));
    const INVALID: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/opencode/invalid.json"
    ));

    #[test]
    fn opencode_ok_fixture_parses() {
        let u = parse_opencode_usage(OK).unwrap();
        assert_eq!(u.rolling.percent, 12.5);
        assert_eq!(u.weekly.percent, 64.0);
        assert_eq!(u.monthly.status, "exhausted");
        assert_eq!(u.monthly.resets_at, "2026-11-01T00:00:00.000Z");
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["rolling"]["resetsAt"], "2026-10-18T15:00:00.000Z");
    }

    #[tokio::test]
    async fn opencode_invalid_fixture_is_schema_changed_and_keeps_last_good() {
        assert_eq!(
            parse_opencode_usage(INVALID),
            Err(ProviderError::SchemaChanged)
        );
        let dir = std::env::temp_dir().join(format!("usagedeck-opencode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let p: CachedProvider<OpenCodeUsage> =
            CachedProvider::new(SECTION, Duration::from_secs(60), None);
        let email = "invalid@example.com";
        let good = p
            .get(&cache, email, |e| async move {
                handle_reply(&e, 200, OK).map_err(FetchError::from)
            })
            .await;
        p.invalidate(None);
        let bad = p
            .get(&cache, email, |e| async move {
                handle_reply(&e, 200, INVALID).map_err(FetchError::from)
            })
            .await;
        assert_eq!(bad.usage, good.usage);
        assert!(bad.stale);
        assert_eq!(bad.error, Some(ProviderError::SchemaChanged));
        let r = row(email, true, bad);
        assert_eq!(r["error"], "schema-changed");
        assert_eq!(r["subscriptionActive"], Value::Null);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn opencode_rejected_key_marks_auth_expired() {
        let email = "rejected@example.com";
        assert_eq!(
            handle_reply(email, 401, b"{}"),
            Err(ProviderError::AuthRejected)
        );
        assert_eq!(
            handle_reply(email, 403, b"<html>"),
            Err(ProviderError::AuthRejected)
        );
        let result = super::super::ProviderResult {
            usage: None,
            stale: false,
            cached_at: None,
            auth_expired: ProviderError::AuthRejected.is_auth_expired(),
            error: Some(ProviderError::AuthRejected),
        };
        let r = row(email, true, result);
        assert_eq!(r["authExpired"], true);
        assert_eq!(r["error"], "auth-rejected");
        // no subscription: a working key, so not a rejected one
        let body = br#"{"error":{"type":"EntitlementError"}}"#;
        assert_eq!(
            handle_reply(email, 403, body),
            Err(ProviderError::Http(403))
        );
        assert_eq!(with_state(email, |s| s.subscription), Some(false));
    }

    #[test]
    fn opencode_prev_and_last_change() {
        let email = "change@example.com";
        handle_reply(email, 200, OK).unwrap();
        assert!(with_state(email, |s| s.prev_snapshot.is_none()
            && s.last_change.is_none()));
        let next = String::from_utf8(OK.to_vec())
            .unwrap()
            .replace("12.5", "20");
        handle_reply(email, 200, next.as_bytes()).unwrap();
        let (prev, change) =
            with_state(email, |s| (s.prev_snapshot.clone(), s.last_change.clone()));
        assert_eq!(prev.unwrap().rolling.percent, 12.5);
        assert_eq!(change.unwrap().1, "rolling 12.5% → 20%");
        assert_eq!(with_state(email, |s| s.subscription), Some(true));
    }

    #[test]
    fn opencode_discovers_auth_json_sqlite_and_deduplicates() {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-opencode-discover-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // auth.json discovery
        let auth = dir.join("auth.json");
        std::fs::write(
            &auth,
            serde_json::json!({
                "opencode-go": { "type": "api", "key": "oc_sk_from_go_1234" },
                "opencode": { "type": "api", "key": "sk-legacy_key_5678" },
                "other": { "key": "bad_prefix_9999" }
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            discover_from_auth_json(&auth),
            vec!["oc_sk_from_go_1234", "sk-legacy_key_5678"]
        );

        // read-only SQLite opencode.db discovery
        let db = dir.join("opencode.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE credential (integration_id TEXT, value TEXT, active INTEGER);
             INSERT INTO credential VALUES ('opencode-go', 'oc_sk_from_db_1111', 1);
             INSERT INTO credential VALUES ('opencode', '{\"key\":\"sk-from_db_json_2222\"}', NULL);
             INSERT INTO credential VALUES ('opencode-go', 'oc_sk_inactive_3333', 0);
             INSERT INTO credential VALUES ('other', 'oc_sk_other_4444', 1);",
        )
        .unwrap();
        drop(conn);
        assert_eq!(
            discover_from_sqlite(&db),
            vec!["oc_sk_from_db_1111", "sk-from_db_json_2222"]
        );

        // deduplication against Credential Manager accounts
        let stored = vec![Account {
            email: "me@example.com".into(),
            key: "oc_sk_from_go_1234".into(),
        }];
        let discovered = vec![
            Account {
                email: "opencode-cli".into(),
                key: "oc_sk_from_go_1234".into(),
            },
            Account {
                email: "opencode-cli".into(),
                key: "sk-legacy_key_5678".into(),
            },
            Account {
                email: "opencode-cli".into(),
                key: "oc_sk_from_db_1111".into(),
            },
            Account {
                email: "opencode-bad".into(),
                key: "not-a-key".into(),
            },
        ];
        let merged = merge_discovered_accounts(stored, discovered);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].email, "me@example.com");
        assert_eq!(merged[1].email, "opencode-cli");
        assert_eq!(merged[1].key, "sk-legacy_key_5678");
        assert_eq!(merged[2].email, "opencode-cli-2");
        assert_eq!(merged[2].key, "oc_sk_from_db_1111");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live: uses the keys stored in Credential Manager and prints only the parsed summary.
    #[tokio::test]
    #[ignore]
    async fn opencode_live() {
        // the installed app's data folder (read only here)
        let dir = std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap())
            .join("com.usagedeck.app");
        let store = crate::store::Store::new(dir, Box::new(crate::store::CredentialVault));
        let accounts = store.get_accounts().await;
        if accounts.is_empty() {
            println!("opencode live: no stored key");
        }
        for (i, acc) in accounts.iter().enumerate() {
            match fetch_usage(&reqwest::Client::new(), &acc.email, &acc.key).await {
                Ok(u) => println!(
                    "opencode live #{i}: rolling={}% weekly={}% monthly={}% weeklyReset={}",
                    u.rolling.percent, u.weekly.percent, u.monthly.percent, u.weekly.resets_at
                ),
                Err(e) => println!("opencode live #{i}: {e}"),
            }
        }
    }
}
