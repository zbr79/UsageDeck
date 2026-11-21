//! OpenCode Go usage per stored API key. Port of lib/opencode.ts (getUsage, parseOpenCodeUsage,
//! diffText, getLastChange) and app/api/usage/route.ts. Keys come from the `Store`
//! (Windows Credential Manager); the persisted last-good value lives in `UsageCache`.

use super::{parse_reply, CachedProvider, FetchError, ProviderError};
use crate::Core;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
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
    let mut map = STATE.lock().unwrap();
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
/// Accounts load one after another; the old route ran them in parallel.
pub async fn accounts(core: &Core) -> Vec<Value> {
    let accounts = core.store.get_accounts().await;
    let disabled = core.store.get_settings().disabled_accounts;
    let mut rows = Vec::with_capacity(accounts.len());
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
        let enabled = !disabled.contains(&email.to_lowercase());
        rows.push(row(&email, enabled, result));
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
