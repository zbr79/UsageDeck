//! Codex usage per signed-in account. Port of lib/codex.ts and app/api/codex/route.ts.
//!
//! `~/.codex/auth.json` belongs to the Codex CLI and is only ever read here. Refreshing would
//! rotate the single-use refresh token and log the CLI out.

use super::util::{number_or_null, string_or_null, to_iso};
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
/// The banked rate-limit resets with their expiry (private, like wham/usage; read only).
const RESETS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const USER_AGENT: &str = "codex-cli/0.44.0";
const SECTION: &str = "codex";

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
    /// when each banked reset in hand expires, soonest first (from wham/rate-limit-reset-credits)
    pub reset_expiries: Vec<String>,
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
        reset_expiries: Vec::new(),
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

// ---- banked resets ---------------------------------------------------------------------------

/// What the resets list said: its count, and the expiry of each reset still in hand.
#[derive(Debug, Default, PartialEq)]
struct ResetList {
    available: Option<f64>,
    expiries: Vec<String>,
}

/// A credit that was spent or expired names it in a `*_at` field or its `status`.
fn spent(credit: &serde_json::Map<String, Value>) -> bool {
    let used = ["redeemed_at", "consumed_at", "used_at"]
        .iter()
        .any(|k| credit.get(*k).is_some_and(|v| !v.is_null()));
    let status = credit
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    used || matches!(
        status.as_deref(),
        Some("redeemed" | "consumed" | "used" | "expired")
    )
}

fn parse_reset_list(reply: &Value, now: &str) -> ResetList {
    let mut expiries: Vec<String> = reply["credits"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_object)
                .filter(|c| !spent(c))
                .filter_map(|c| to_iso(c.get("expires_at").unwrap_or(&Value::Null)))
                .filter(|at| at.as_str() > now)
                .collect()
        })
        .unwrap_or_default();
    expiries.sort();
    ResetList {
        available: number_or_null(&reply["available_count"]),
        expiries,
    }
}

/// Names only, sorted: the reply's top-level keys and the first credit's keys, for diagnostics.
fn shape(reply: &Value) -> String {
    let keys = |v: &Value| {
        v.as_object().map_or_else(
            || "not an object".to_string(),
            |o| {
                let mut keys: Vec<&str> = o.keys().map(String::as_str).collect();
                keys.sort_unstable();
                keys.join(", ")
            },
        )
    };
    let first = reply["credits"].get(0).map_or("none".to_string(), keys);
    format!("keys: {}; first credit keys: {first}", keys(reply))
}

/// When and what the last resets fetch saw, for the diagnostics report.
static LAST_RESETS: std::sync::Mutex<Option<(i64, String)>> = std::sync::Mutex::new(None);

pub fn last_resets_report() -> Option<(i64, String)> {
    LAST_RESETS.lock().ok().and_then(|r| r.clone())
}

fn note_resets(usage_count: Option<f64>, line: String) {
    let usage = usage_count.map_or("absent".to_string(), |n| n.to_string());
    if let Ok(mut last) = LAST_RESETS.lock() {
        *last = Some((
            chrono::Utc::now().timestamp_millis(),
            format!("Usage reply available_count: {usage}\nResets list: {line}"),
        ));
    }
}

/// GET the resets list. Best effort: the card keeps wham/usage's count when this fails.
async fn fetch_resets(
    http: &reqwest::Client,
    account: &CodexAccount,
    usage_count: Option<f64>,
) -> Option<ResetList> {
    let mut req = http
        .get(RESETS_URL)
        .bearer_auth(&account.token)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .timeout(Duration::from_secs(10));
    if let Some(id) = &account.account_id {
        req = req.header("chatgpt-account-id", id);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(_) => {
            note_resets(usage_count, "network error".into());
            return None;
        }
    };
    let status = resp.status().as_u16();
    let reply = resp
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let Some(reply) = reply.filter(|_| (200..300).contains(&status)) else {
        note_resets(usage_count, format!("HTTP {status}, no JSON reply used"));
        return None;
    };
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let list = parse_reset_list(&reply, &now);
    note_resets(
        usage_count,
        format!(
            "HTTP {status}; available_count {}; {} in hand with an expiry; {}",
            list.available
                .map_or("absent".to_string(), |n| n.to_string()),
            list.expiries.len(),
            shape(&reply)
        ),
    );
    Some(list)
}

/// The usage with the resets list's expiries, and its count when wham/usage had none.
fn with_resets(mut usage: CodexUsage, list: Option<ResetList>) -> CodexUsage {
    if let Some(list) = list {
        usage.reset_credits = usage.reset_credits.or(list.available);
        usage.reset_expiries = list.expiries;
    }
    usage
}

// ---- auth.json ------------------------------------------------------------------------------

#[derive(Clone)]
struct CodexAccount {
    id: String,
    label: Option<String>,
    token: String,
    account_id: Option<String>,
}

/// Unit tests never read this PC's real sign-ins; only the `#[ignore]` live tests switch it on.
#[cfg(test)]
pub(crate) static REAL_SIGN_INS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// True when the real sign-in files may be read (always outside tests).
pub(crate) fn real_sign_ins() -> bool {
    #[cfg(test)]
    return REAL_SIGN_INS.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    true
}

fn auth_file() -> PathBuf {
    if !real_sign_ins() {
        return PathBuf::from("no-real-sign-in-in-tests");
    }
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

/// A JWT's payload. The signature is not checked: the claims only name or describe the token.
pub(crate) fn jwt_payload(token: &str) -> Option<Value> {
    let part = token.split('.').nth(1)?;
    serde_json::from_slice(&base64url_decode(part)?).ok()
}

/// Email claim from a JWT payload.
pub fn email_from_jwt(token: Option<&str>) -> Option<String> {
    let json = jwt_payload(token?)?;
    let email = json.get("email")?.as_str()?.trim();
    (!email.is_empty()).then(|| email.to_string())
}

fn account_from_auth(auth: &Value) -> Option<CodexAccount> {
    let tokens = auth.get("tokens")?;
    let token = tokens
        .get("access_token")?
        .as_str()
        .filter(|t| !t.is_empty())?;
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

async fn fetch_usage(
    http: &reqwest::Client,
    account: &CodexAccount,
) -> Result<CodexUsage, ProviderError> {
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
    super::note_retry_after(&resp);
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
                            let usage = fetch_usage(&core.http, &account)
                                .await
                                .map_err(FetchError::from)?;
                            let list =
                                fetch_resets(&core.http, &account, usage.reset_credits).await;
                            Ok(with_resets(usage, list))
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

    const OK: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/codex/ok.json"
    ));
    const INVALID: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/codex/invalid.json"
    ));

    #[test]
    fn the_resets_list_keeps_unspent_unexpired_credits_soonest_first() {
        let reply = serde_json::json!({
            "available_count": 2,
            "credits": [
                { "granted_at": 1790000000, "expires_at": "2026-12-20T00:00:00Z" },
                { "expires_at": 1796000000, "status": "available" },
                { "expires_at": "2026-12-01T00:00:00Z", "redeemed_at": "2026-11-01T00:00:00Z" },
                { "expires_at": "2026-11-01T00:00:00Z" },
                { "granted_at": 1790000000 }
            ]
        });
        let list = parse_reset_list(&reply, "2026-11-12T00:00:00.000Z");
        assert_eq!(list.available, Some(2.0));
        assert_eq!(
            list.expiries,
            ["2026-11-30T00:53:20.000Z", "2026-12-20T00:00:00.000Z"]
        );
        assert_eq!(
            shape(&reply),
            "keys: available_count, credits; first credit keys: expires_at, granted_at"
        );
        let usage = with_resets(CodexUsage::default(), Some(list));
        assert_eq!(usage.reset_credits, Some(2.0));
        assert_eq!(usage.reset_expiries.len(), 2);
    }

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
        assert_eq!(
            u.weekly_reset_at.as_deref(),
            Some("2026-10-18T17:46:40.000Z")
        );
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
        assert_eq!(
            classify_reply(403, br#"{"detail":"nope"}"#),
            Err(ProviderError::AuthRejected)
        );
        let cloudflare = b"<!DOCTYPE html><html><title>Just a moment...</title></html>";
        assert_eq!(
            classify_reply(403, cloudflare),
            Err(ProviderError::Http(403))
        );
        assert_eq!(classify_reply(429, b""), Err(ProviderError::RateLimited));
        assert_eq!(classify_reply(500, b""), Err(ProviderError::Http(500)));
    }

    #[tokio::test]
    async fn codex_invalid_fixture_is_schema_changed_and_keeps_last_good() {
        assert_eq!(
            classify_reply(200, INVALID),
            Err(ProviderError::SchemaChanged)
        );
        let dir = std::env::temp_dir().join(format!("usagedeck-codex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let p: CachedProvider<CodexUsage> =
            CachedProvider::new(SECTION, Duration::from_secs(60), None);
        let good = p
            .get(&cache, "k", |_| async {
                classify_reply(200, OK).map_err(FetchError::from)
            })
            .await;
        p.invalidate(None);
        let bad = p
            .get(&cache, "k", |_| async {
                classify_reply(200, INVALID).map_err(FetchError::from)
            })
            .await;
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
        REAL_SIGN_INS.store(true, std::sync::atomic::Ordering::Relaxed);
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
