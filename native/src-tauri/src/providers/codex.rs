//! Codex usage per signed-in account. Port of lib/codex.ts and app/api/codex/route.ts.
//!
//! `~/.codex/auth.json` belongs to the Codex CLI and is only ever read here. Refreshing would
//! rotate the single-use refresh token and log the CLI out.
//!
//! Also holds the small value helpers of lib/time.ts that the Cursor port shares.

use super::{
    collect_accounts, parse_reply, CachedProvider, FetchError, LiveAccount, ProviderError,
};
use crate::Core;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const USER_AGENT: &str = "codex-cli/0.44.0";
const SECTION: &str = "codex";

// ---- lib/time.ts ----------------------------------------------------------------------------

/// Seconds or milliseconds since the epoch, a numeric string, or a date string, as an ISO time
/// in the `Date.toISOString()` format. Anything else is None.
pub(crate) fn to_iso(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => {
            let v = n.as_f64().filter(|v| v.is_finite())?;
            let ms = if v < 1e12 { v * 1000.0 } else { v };
            let d = chrono::DateTime::from_timestamp_millis(ms.trunc() as i64)?;
            Some(d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        }
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                return None;
            }
            if let Ok(n) = t.parse::<f64>() {
                if t.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    return to_iso(&serde_json::json!(n));
                }
            }
            let d = chrono::DateTime::parse_from_rfc3339(t).ok()?;
            Some(d.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        }
        _ => None,
    }
}

pub(crate) fn number_or_null(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite())
}

pub(crate) fn string_or_null(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

// ---- JSON shapes ----------------------------------------------------------------------------

/// `CodexUsage` in lib/types.ts. The weekly fields are new; `default` lets older cache entries load.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodexUsage {
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub used_percent: Option<f64>,
    pub limit_window_seconds: Option<f64>,
    pub reset_at: Option<String>,
    /// `rate_limit.secondary_window`: the weekly limit (ignored by the old parser)
    pub weekly_used_percent: Option<f64>,
    pub weekly_limit_window_seconds: Option<f64>,
    pub weekly_reset_at: Option<String>,
    pub credits_balance: Option<f64>,
    pub reset_credits: Option<f64>,
    pub limit_reached: bool,
}

/// The wham/usage reply. `rate_limit` must be an object; every field inside stays lenient as in
/// the old parser.
#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    email: Value,
    #[serde(default)]
    plan_type: Value,
    rate_limit: RawRateLimit,
    #[serde(default)]
    credits: Value,
    #[serde(default)]
    rate_limit_reset_credits: Value,
}

#[derive(Deserialize)]
struct RawRateLimit {
    #[serde(default)]
    limit_reached: Value,
    #[serde(default)]
    primary_window: Value,
    #[serde(default)]
    secondary_window: Value,
}

/// Port of `parseCodexUsage`, plus the weekly (secondary) window.
pub fn parse_codex_usage(bytes: &[u8]) -> Result<CodexUsage, ProviderError> {
    let raw: RawUsage = parse_reply(bytes)?;
    let rl = &raw.rate_limit;
    let p = &rl.primary_window;
    let s = &rl.secondary_window;
    let balance = &raw.credits["balance"];
    let credits_balance = match balance {
        Value::Number(_) => number_or_null(balance),
        Value::String(t) => t.trim().parse::<f64>().ok().filter(|v| v.is_finite()),
        _ => None,
    };
    Ok(CodexUsage {
        email: string_or_null(&raw.email),
        plan_type: string_or_null(&raw.plan_type),
        used_percent: number_or_null(&p["used_percent"]),
        limit_window_seconds: number_or_null(&p["limit_window_seconds"]),
        reset_at: to_iso(&p["reset_at"]),
        weekly_used_percent: number_or_null(&s["used_percent"]),
        weekly_limit_window_seconds: number_or_null(&s["limit_window_seconds"]),
        weekly_reset_at: to_iso(&s["reset_at"]),
        credits_balance,
        reset_credits: number_or_null(&raw.rate_limit_reset_credits["available_count"]),
        limit_reached: rl.limit_reached == Value::Bool(true),
    })
}

/// Map one HTTP reply. 401 is a rejected sign-in; 403 only when it is the API's own JSON error
/// (a Cloudflare HTML page is `Http(403)`, not a sign-in problem).
pub fn classify_reply(status: u16, bytes: &[u8]) -> Result<CodexUsage, ProviderError> {
    match status {
        401 => Err(ProviderError::AuthRejected),
        403 if serde_json::from_slice::<Value>(bytes).is_ok_and(|v| v.is_object()) => {
            Err(ProviderError::AuthRejected)
        }
        429 => Err(ProviderError::RateLimited),
        s if !(200..300).contains(&s) => Err(ProviderError::Http(s)),
        _ => parse_codex_usage(bytes),
    }
}

// ---- auth.json ------------------------------------------------------------------------------

#[derive(Clone)]
struct CodexAccount {
    id: String,
    label: Option<String>,
    token: String,
    account_id: Option<String>,
}

fn auth_file() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".codex").join("auth.json")
}

fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Email claim from a JWT payload. The signature is not checked; this only names the account.
pub fn email_from_jwt(token: Option<&str>) -> Option<String> {
    let part = token?.split('.').nth(1)?;
    let json: Value = serde_json::from_slice(&base64url_decode(part)?).ok()?;
    let email = json.get("email")?.as_str()?.trim();
    (!email.is_empty()).then(|| email.to_string())
}

fn account_from_auth(auth: &Value) -> Option<CodexAccount> {
    let tokens = auth.get("tokens")?;
    let token = tokens.get("access_token")?.as_str().filter(|t| !t.is_empty())?;
    let email = email_from_jwt(tokens.get("id_token").and_then(Value::as_str));
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let label = email.or_else(|| account_id.clone());
    Some(CodexAccount {
        id: label.as_deref().unwrap_or("signed-in").to_lowercase(),
        label,
        token: token.to_string(),
        account_id,
    })
}

fn read_codex_account() -> Option<CodexAccount> {
    let bytes = std::fs::read(auth_file()).ok()?;
    account_from_auth(&serde_json::from_slice(&bytes).ok()?)
}

// ---- fetch ----------------------------------------------------------------------------------

static CACHE: LazyLock<CachedProvider<CodexUsage>> =
    LazyLock::new(|| CachedProvider::new(SECTION, Duration::from_secs(30), None));

async fn fetch_usage(http: &reqwest::Client, account: &CodexAccount) -> Result<CodexUsage, ProviderError> {
    let mut req = http
        .get(USAGE_URL)
        .bearer_auth(&account.token)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .timeout(Duration::from_secs(10));
    if let Some(id) = &account.account_id {
        req = req.header("chatgpt-account-id", id);
    }
    let resp = req.send().await.map_err(|e| {
        log::warn!("codex request failed: {}", e.without_url());
        ProviderError::Network
    })?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.map_err(|_| ProviderError::Network)?;
    classify_reply(status, &bytes)
}

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<CodexUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let live = match read_codex_account() {
        Some(account) => {
            let key = account.id.clone();
            let label_from: fn(&CodexUsage) -> Option<String> = |u| u.email.clone();
            vec![LiveAccount {
                id: account.id.clone(),
                label: account.label.clone(),
                active: true,
                load: Box::pin(async move {
                    CACHE
                        .get(&core.cache, &key, |_| async move {
                            fetch_usage(&core.http, &account).await.map_err(FetchError::from)
                        })
                        .await
                }),
                label_from: Some(label_from),
            }]
        }
        None => Vec::new(),
    };
    collect_accounts(&core.seen, &core.cache, SECTION, SECTION, live)
        .await
        .into_iter()
        .filter_map(|row| serde_json::to_value(row).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::UsageCache;

    const OK: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/codex/ok.json"));
    const INVALID: &[u8] =
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/codex/invalid.json"));

    #[test]
    fn codex_ok_fixture_parses_with_weekly_window() {
        let u = classify_reply(200, OK).unwrap();
        assert_eq!(u.email.as_deref(), Some("user@example.com"));
        assert_eq!(u.plan_type.as_deref(), Some("plus"));
        assert_eq!(u.used_percent, Some(37.0));
        assert_eq!(u.limit_window_seconds, Some(18000.0));
        assert_eq!(u.reset_at.as_deref(), Some("2026-10-14T17:46:40.000Z"));
        assert_eq!(u.weekly_used_percent, Some(81.0));
        assert_eq!(u.weekly_limit_window_seconds, Some(604800.0));
        assert_eq!(u.weekly_reset_at.as_deref(), Some("2026-10-18T17:46:40.000Z"));
        assert_eq!(u.credits_balance, Some(12.5));
        assert_eq!(u.reset_credits, Some(2.0));
        assert!(!u.limit_reached);
        // JSON keys stay camelCase as in lib/types.ts
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["weeklyUsedPercent"], 81.0);
        assert_eq!(v["planType"], "plus");
    }

    #[test]
    fn codex_status_mapping() {
        assert_eq!(classify_reply(401, b"{}"), Err(ProviderError::AuthRejected));
        assert_eq!(classify_reply(403, br#"{"detail":"nope"}"#), Err(ProviderError::AuthRejected));
        let cloudflare = b"<!DOCTYPE html><html><title>Just a moment...</title></html>";
        assert_eq!(classify_reply(403, cloudflare), Err(ProviderError::Http(403)));
        assert_eq!(classify_reply(429, b""), Err(ProviderError::RateLimited));
        assert_eq!(classify_reply(500, b""), Err(ProviderError::Http(500)));
    }

    #[tokio::test]
    async fn codex_invalid_fixture_is_schema_changed_and_keeps_last_good() {
        assert_eq!(classify_reply(200, INVALID), Err(ProviderError::SchemaChanged));
        let dir = std::env::temp_dir().join(format!("usagedeck-codex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let p: CachedProvider<CodexUsage> = CachedProvider::new(SECTION, Duration::from_secs(60), None);
        let good = p.get(&cache, "k", |_| async { classify_reply(200, OK).map_err(FetchError::from) }).await;
        p.invalidate(None);
        let bad = p.get(&cache, "k", |_| async { classify_reply(200, INVALID).map_err(FetchError::from) }).await;
        assert_eq!(bad.usage, good.usage);
        assert!(bad.stale);
        assert_eq!(bad.error, Some(ProviderError::SchemaChanged));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_account_from_auth_json() {
        // header.payload.signature with payload {"email":"user@example.com"}
        let payload = "eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20ifQ";
        let auth = serde_json::json!({ "tokens": {
            "access_token": "t", "id_token": format!("h.{payload}.s"), "account_id": " acc-1 " } });
        let a = account_from_auth(&auth).unwrap();
        assert_eq!(a.id, "user@example.com");
        assert_eq!(a.account_id.as_deref(), Some("acc-1"));
        assert!(account_from_auth(&serde_json::json!({ "tokens": {} })).is_none());
    }

    /// Live: reads ~/.codex/auth.json (read-only) and prints only the parsed summary.
    #[tokio::test]
    #[ignore]
    async fn codex_live() {
        let Some(account) = read_codex_account() else {
            println!("codex live: no sign-in");
            return;
        };
        match fetch_usage(&reqwest::Client::new(), &account).await {
            Ok(u) => println!(
                "codex live: plan={:?} used={:?}% reset={:?} weekly={:?}% weeklyReset={:?}",
                u.plan_type, u.used_percent, u.reset_at, u.weekly_used_percent, u.weekly_reset_at
            ),
            Err(e) => println!("codex live: {e}"),
        }
    }
}
