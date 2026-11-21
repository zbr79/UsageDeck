//! Grok Bot usage per stored account. Port of lib/grokbot.ts and app/api/grok/route.ts.
//!
//! Grok Bot keeps every sign-in in `sand-secrets.json`, sealed with its safeStorage key. When it
//! has none, Cursor's current sign-in (same backend) is used as a single account. Both are only
//! ever read.

use super::util::{string_or_null, to_iso};
use super::{
    collect_accounts, CachedProvider, FetchError, LiveAccount, ProviderError, TrackedAccount,
};
use crate::win::os_crypt::{app_key, unseal, AppKey};
use crate::Core;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

const SECTION: &str = "grokbot";
const PROVIDER: &str = "grok";

static CACHE: LazyLock<CachedProvider<GrokUsage>> =
    LazyLock::new(|| CachedProvider::new(SECTION, Duration::from_secs(30), None));

/// `GrokUsage` in lib/types.ts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokUsage {
    pub email: Option<String>,
    pub name: Option<String>,
    pub percent_used: Option<f64>,
    pub reset_at: Option<String>,
    pub plan_label: Option<String>,
    pub cursor_plan_name: Option<String>,
    /// "grok-bot" or "cursor": whose sign-in was used
    pub source: String,
}

/// The `GetSandUsageStatus` reply. A wrong type, or no `usagePercent` (the old check), is
/// `SchemaChanged`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandReply {
    usage_percent: Option<f64>,
    next_reset_timestamp_utc: Option<Value>,
    grok_plan_label: Option<Value>,
    cursor_plan_name: Option<Value>,
}

pub fn parse_sand_reply(bytes: &[u8]) -> Result<SandReply, ProviderError> {
    let reply: SandReply = super::parse_reply(bytes)?;
    if reply.usage_percent.is_none_or(|p| !p.is_finite()) {
        log::warn!("grok usage reply holds no usagePercent");
        return Err(ProviderError::SchemaChanged);
    }
    Ok(reply)
}

#[derive(Clone, Debug, PartialEq)]
pub struct GrokAccount {
    pub id: String,
    pub token: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub source: &'static str,
    pub active: bool,
}

pub fn build_usage(reply: &SandReply, account: &GrokAccount) -> GrokUsage {
    GrokUsage {
        email: account.email.clone(),
        name: account.name.clone(),
        percent_used: reply.usage_percent.filter(|p| p.is_finite()),
        reset_at: reply.next_reset_timestamp_utc.as_ref().and_then(to_iso),
        plan_label: reply.grok_plan_label.as_ref().and_then(string_or_null),
        cursor_plan_name: reply.cursor_plan_name.as_ref().and_then(string_or_null),
        source: account.source.to_string(),
    }
}

fn appdata() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
                .join("AppData")
                .join("Roaming")
        })
}

/// Every account stored by Grok Bot, not only the one selected in the app.
pub fn grok_bot_accounts(secrets: &Value, key: &AppKey) -> Vec<GrokAccount> {
    let Some(parsed) = secrets
        .get("cursor-accounts")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    else {
        return Vec::new();
    };
    let Some(entries) = parsed.get("accounts").and_then(Value::as_object) else {
        return Vec::new();
    };
    let active_key = parsed.get("active").and_then(Value::as_str).unwrap_or("");
    let mut out: Vec<GrokAccount> = Vec::new();
    for (storage_id, entry) in entries {
        let Some(token) = unseal(key, entry.get("cursor-access-token")) else {
            continue;
        };
        let profile = unseal(key, entry.get("cursor-account-profile"))
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        let email = profile
            .as_ref()
            .and_then(|p| p.get("email"))
            .and_then(string_or_null);
        let name = profile
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(string_or_null);
        let id = email.as_deref().unwrap_or(storage_id).trim().to_lowercase();
        if id.is_empty() {
            continue;
        }
        let active = storage_id == active_key
            || email
                .as_deref()
                .is_some_and(|e| e.to_lowercase() == active_key.to_lowercase());
        let account = GrokAccount {
            id,
            token,
            email,
            name,
            source: "grok-bot",
            active,
        };
        match out.iter_mut().find(|a| a.id == account.id) {
            Some(prev) if account.active && !prev.active => *prev = account,
            Some(_) => {}
            None => out.push(account),
        }
    }
    if out.len() == 1 {
        out[0].active = true;
    }
    out
}

fn read_grok_bot_accounts() -> Vec<GrokAccount> {
    let dir = appdata().join("Grok Bot");
    let Some(secrets) = std::fs::read_to_string(dir.join("sand-secrets.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        return Vec::new();
    };
    let Some(key) = app_key(&dir.join("Local State")) else {
        return Vec::new();
    };
    grok_bot_accounts(&secrets, &key)
}

/// Grok Bot's saved sign-ins; when it has none, Cursor's current sign-in as a single account.
/// Blocking (files and SQLite): call through `tokio::task::spawn_blocking`, as `accounts` does.
fn read_accounts() -> Vec<GrokAccount> {
    let own = read_grok_bot_accounts();
    if !own.is_empty() {
        return own;
    }
    let Some((token, name)) = super::cursor::bearer_token() else {
        return Vec::new();
    };
    let label = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    vec![GrokAccount {
        id: label
            .clone()
            .unwrap_or_else(|| "cursor".into())
            .to_lowercase(),
        token,
        email: label,
        name: None,
        source: "cursor",
        active: true,
    }]
}

/// POST GetSandUsageStatus with Cursor's request. 401/403 -> AuthRejected, 429 -> RateLimited.
pub async fn fetch_reply(
    http: &reqwest::Client,
    token: &str,
) -> Result<(SandReply, Vec<u8>), ProviderError> {
    let resp = super::cursor::dashboard_post(http, token, "GetSandUsageStatus")
        .send()
        .await
        .map_err(|e| {
            log::warn!("grok usage request failed: {}", e.without_url());
            ProviderError::Network
        })?;
    super::note_retry_after(&resp);
    match resp.status().as_u16() {
        401 | 403 => return Err(ProviderError::AuthRejected),
        429 => return Err(ProviderError::RateLimited),
        s if !(200..300).contains(&s) => return Err(ProviderError::Http(s)),
        _ => {}
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|_| ProviderError::Network)?
        .to_vec();
    Ok((parse_sand_reply(&bytes)?, bytes))
}

async fn fetch(
    http: reqwest::Client,
    account: GrokAccount,
) -> Result<GrokUsage, FetchError<GrokUsage>> {
    let (reply, _) = fetch_reply(&http, &account.token).await?;
    Ok(build_usage(&reply, &account))
}

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<GrokUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let accounts = tokio::task::spawn_blocking(read_accounts)
        .await
        .unwrap_or_default();
    let live: Vec<LiveAccount<'_, GrokUsage>> = accounts
        .into_iter()
        .map(|account| {
            let id = account.id.clone();
            let label = account.email.clone().or_else(|| account.name.clone());
            let active = account.active;
            let http = core.http.clone();
            LiveAccount {
                id: id.clone(),
                label,
                active,
                load: Box::pin(async move {
                    CACHE
                        .get(&core.cache, &id, move |_| fetch(http, account))
                        .await
                }),
                label_from: None,
            }
        })
        .collect();
    let rows: Vec<TrackedAccount<GrokUsage>> =
        collect_accounts(&core.seen, &core.cache, PROVIDER, SECTION, live).await;
    rows.into_iter()
        .filter_map(|r| serde_json::to_value(r).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::UsageCache;
    use crate::win::os_crypt::seal;
    use serde_json::json;

    const OK: &[u8] = include_bytes!("../../tests/fixtures/grok/ok.json");
    const INVALID: &[u8] = include_bytes!("../../tests/fixtures/grok/invalid.json");

    fn account() -> GrokAccount {
        GrokAccount {
            id: "user@example.com".into(),
            token: "REDACTED".into(),
            email: Some("user@example.com".into()),
            name: None,
            source: "grok-bot",
            active: true,
        }
    }

    #[test]
    fn ok_fixture_parses_to_expected_values() {
        let usage = build_usage(&parse_sand_reply(OK).expect("ok.json parses"), &account());
        assert_eq!(usage.percent_used, Some(0.957435));
        assert_eq!(usage.reset_at.as_deref(), Some("2026-10-25T22:15:02.859Z"));
        assert_eq!(usage.plan_label.as_deref(), Some("Grok Bot Plan"));
        assert_eq!(usage.cursor_plan_name.as_deref(), Some("Pro Student"));
        let v = serde_json::to_value(&usage).unwrap();
        assert_eq!(v["source"], json!("grok-bot"));
        assert!(v.get("percentUsed").is_some() && v.get("cursorPlanName").is_some());
    }

    #[test]
    fn to_iso_matches_lib_time() {
        assert_eq!(
            to_iso(&json!(1_760_000_000)).as_deref(),
            Some("2025-10-09T08:53:20.000Z")
        );
        assert_eq!(
            to_iso(&json!("2025-10-09T08:53:20Z")).as_deref(),
            Some("2025-10-09T08:53:20.000Z")
        );
        assert_eq!(to_iso(&json!("")), None);
        assert_eq!(to_iso(&json!(true)), None);
    }

    #[tokio::test]
    async fn invalid_fixture_is_schema_changed_and_keeps_last_good() {
        assert_eq!(
            parse_sand_reply(INVALID).err(),
            Some(ProviderError::SchemaChanged)
        );
        // the old check: a reply without usagePercent is a failure, not an all-null success
        assert_eq!(
            parse_sand_reply(b"{}").err(),
            Some(ProviderError::SchemaChanged)
        );

        let dir = std::env::temp_dir().join(format!("usagedeck-grok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let provider: CachedProvider<GrokUsage> =
            CachedProvider::new(SECTION, Duration::from_secs(30), None);
        let parse = |bytes: &'static [u8]| {
            move |_| async move {
                let reply = parse_sand_reply(bytes).map_err(FetchError::from)?;
                Ok(build_usage(&reply, &account()))
            }
        };
        let good = provider.get(&cache, "user@example.com", parse(OK)).await;
        assert!(!good.stale && good.usage.is_some());
        provider.invalidate(None);
        let bad = provider
            .get(&cache, "user@example.com", parse(INVALID))
            .await;
        assert_eq!(bad.error, Some(ProviderError::SchemaChanged));
        assert!(bad.stale);
        assert_eq!(bad.usage, good.usage);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grok_bot_accounts_unseal_and_pick_active() {
        let key: AppKey = [9; 32];
        let profile = |email: &str| seal(&key, &json!({ "email": email, "name": "N" }).to_string());
        let inner = json!({
            "active": "s2",
            "accounts": {
                "s1": { "cursor-access-token": seal(&key, "t1"), "cursor-account-profile": profile("A@example.com") },
                "s2": { "cursor-access-token": seal(&key, "t2"), "cursor-account-profile": profile("b@example.com") },
                "s3": { "cursor-access-token": "not sealed" },
            }
        });
        let secrets = json!({ "cursor-accounts": inner.to_string() });
        let accounts = grok_bot_accounts(&secrets, &key);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].id, "a@example.com");
        assert!(!accounts[0].active);
        assert!(accounts[1].active && accounts[1].token == "t2");
    }

    /// Live check against the user's own sign-in. Prints only the parsed summary. Set
    /// USAGEDECK_FIXTURE_OUT to a scratch path to save the raw reply for redaction.
    #[tokio::test]
    #[ignore]
    async fn live_grok() {
        let accounts = tokio::task::spawn_blocking(read_accounts).await.unwrap();
        println!(
            "accounts: {} (source {:?})",
            accounts.len(),
            accounts.first().map(|a| a.source)
        );
        let account = accounts
            .iter()
            .find(|a| a.active)
            .or(accounts.first())
            .expect("a sign-in")
            .clone();
        let (reply, raw) = fetch_reply(&reqwest::Client::new(), &account.token)
            .await
            .expect("live reply");
        if let Some(out) = std::env::var_os("USAGEDECK_FIXTURE_OUT") {
            std::fs::write(out, raw).unwrap();
        }
        let u = build_usage(&reply, &account);
        println!(
            "plan={:?} cursorPlan={:?} used={:?}% resets {:?}",
            u.plan_label, u.cursor_plan_name, u.percent_used, u.reset_at
        );
    }
}
