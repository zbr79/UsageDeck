//! Claude usage per signed-in account (Claude desktop token, then Claude Code token).
//! Port of lib/claude.ts and app/api/claude/route.ts.
//!
//! Credentials belong to Claude Code / Claude desktop. This module only ever reads them:
//! refreshing would rotate the refresh token out from under the owning app and log it out.
//!
//! Fixed against the old code: the identity (email) comes from the same app as the token. A
//! Claude desktop token is filed under Claude Code's email only when both apps are signed into
//! the same account (desktop `lastKnownAccountUuid` == `~/.claude.json` `oauthAccount.accountUuid`).

use super::{
    collect_accounts, CachedProvider, FetchError, LiveAccount, ProviderError, TrackedAccount,
};
use crate::win::os_crypt::{app_key, unseal};
use crate::Core;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const SECTION: &str = "claude";
const PROVIDER: &str = "claude";

/// This endpoint rate-limits hard: a good value is reused for 60 s, a failure for 120 s.
static CACHE: LazyLock<CachedProvider<ClaudeUsage>> = LazyLock::new(|| {
    CachedProvider::new(
        SECTION,
        Duration::from_secs(60),
        Some(Duration::from_secs(120)),
    )
});

// ---- JSON shapes (lib/types.ts) -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeWindow {
    pub percent: Option<f64>,
    pub resets_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeUsage {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub signed_in: bool,
    pub subscribed: bool,
    pub subscription_type: Option<String>,
    pub plan_name: Option<String>,
    pub five_hour: ClaudeWindow,
    pub weekly: ClaudeWindow,
    pub weekly_opus: Option<ClaudeWindow>,
    pub weekly_sonnet: Option<ClaudeWindow>,
    pub reset_credits: Option<i64>,
    pub billing_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub org_uuid: Option<String>,
}

// ---- the usage reply ------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum ResetValue {
    Seconds(f64),
    Text(String),
}

#[derive(Clone, Debug, Deserialize)]
struct WindowReply {
    utilization: Option<f64>,
    percent: Option<f64>,
    resets_at: Option<ResetValue>,
    #[serde(rename = "resetsAt")]
    resets_at_camel: Option<ResetValue>,
}

/// The `oauth/usage` reply. A window of the wrong type, or a reply holding neither the 5-hour
/// nor the weekly window, is `SchemaChanged` (never an all-null success).
#[derive(Clone, Debug, Deserialize)]
pub struct UsageReply {
    five_hour: Option<WindowReply>,
    seven_day: Option<WindowReply>,
    seven_day_opus: Option<WindowReply>,
    seven_day_sonnet: Option<WindowReply>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

pub fn parse_usage_reply(bytes: &[u8]) -> Result<UsageReply, ProviderError> {
    let value: Value = super::parse_reply(bytes)?;
    let has_window = value
        .as_object()
        .is_some_and(|o| o.contains_key("five_hour") || o.contains_key("seven_day"));
    if !has_window {
        log::warn!("claude usage reply holds no usage window");
        return Err(ProviderError::SchemaChanged);
    }
    serde_json::from_value(value).map_err(|e| {
        log::warn!("claude usage reply did not match its type: {e}");
        ProviderError::SchemaChanged
    })
}

fn iso(d: DateTime<Utc>) -> String {
    d.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn window_of(w: Option<&WindowReply>) -> ClaudeWindow {
    let Some(w) = w else {
        return ClaudeWindow {
            percent: None,
            resets_at: None,
        };
    };
    let resets_at = match w.resets_at.as_ref().or(w.resets_at_camel.as_ref()) {
        Some(ResetValue::Seconds(s)) => {
            DateTime::from_timestamp_millis((s * 1000.0) as i64).map(iso)
        }
        Some(ResetValue::Text(t)) => DateTime::parse_from_rfc3339(t.trim())
            .ok()
            .map(|d| iso(d.with_timezone(&Utc))),
        None => None,
    };
    ClaudeWindow {
        percent: w.utilization.or(w.percent),
        resets_at,
    }
}

fn reset_credits_of(rest: &Map<String, Value>) -> Option<i64> {
    for key in [
        "cedar_ember",
        "juniper_tide",
        "iguana_necktie",
        "amber_gauge",
    ] {
        let Some(v) = rest.get(key).and_then(Value::as_object) else {
            continue;
        };
        if v.get("eligible") == Some(&Value::Bool(false)) {
            continue;
        }
        if let Some(grants) = v.get("grants").and_then(Value::as_array) {
            return Some(grants.len() as i64);
        }
        if let Some(available) = v.get("available").and_then(Value::as_bool) {
            return Some(available as i64);
        }
    }
    None
}

// ---- local sign-ins -------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeToken {
    pub token: String,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub expires_at: Option<i64>,
    pub expired: bool,
}

fn is_expired(expires_at: Option<i64>) -> bool {
    expires_at.is_some_and(|t| t <= Utc::now().timestamp_millis())
}

fn str_of(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

/// Pick the best token: live Claude Code > any live > expired ones (an expired token still tells
/// us the plan, just not the usage).
pub fn pick_token(entries: &Value) -> Option<ClaudeToken> {
    let mut candidates: Vec<(u8, ClaudeToken)> = Vec::new();
    for (name, entry) in entries.as_object()? {
        let Some(token) = entry
            .get("token")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let expires_at = entry
            .get("expiresAt")
            .and_then(Value::as_f64)
            .map(|t| t as i64);
        let expired = is_expired(expires_at);
        let rank = if expired { 2 } else { 0 } + if name.contains("claude_code") { 0 } else { 1 };
        candidates.push((
            rank,
            ClaudeToken {
                token: token.to_string(),
                subscription_type: str_of(entry.get("subscriptionType")),
                rate_limit_tier: str_of(entry.get("rateLimitTier")),
                expires_at,
                expired,
            },
        ));
    }
    candidates.sort_by_key(|(rank, _)| *rank); // stable, like Array.sort
    candidates.into_iter().next().map(|(_, t)| t)
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Claude desktop config dirs: the MSIX package's virtualised Roaming dir, then %APPDATA%\Claude.
fn desktop_config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        if let Ok(entries) = std::fs::read_dir(PathBuf::from(local).join("Packages")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if entry.file_type().is_ok_and(|t| t.is_dir()) && name.starts_with("claude_") {
                    dirs.push(
                        entry
                            .path()
                            .join("LocalCache")
                            .join("Roaming")
                            .join("Claude"),
                    );
                }
            }
        }
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(roaming).join("Claude"));
    }
    dirs
}

/// Claude desktop's token and the account it was signed into (`lastKnownAccountUuid`).
#[derive(Clone, Debug, PartialEq)]
pub struct DesktopToken {
    pub token: ClaudeToken,
    pub account_uuid: Option<String>,
}

fn read_desktop_token() -> Option<DesktopToken> {
    for dir in desktop_config_dirs() {
        let Some(config) = read_json(&dir.join("config.json")) else {
            continue;
        };
        let sealed = config
            .get("oauth:tokenCacheV2")
            .or_else(|| config.get("oauth:tokenCache"));
        if !sealed.is_some_and(Value::is_string) {
            continue;
        }
        let Some(key) = app_key(&dir.join("Local State")) else {
            continue;
        };
        let Some(raw) = unseal(&key, sealed) else {
            continue;
        };
        let Ok(entries) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if let Some(token) = pick_token(&entries) {
            return Some(DesktopToken {
                token,
                account_uuid: str_of(config.get("lastKnownAccountUuid")),
            });
        }
    }
    None
}

fn read_cli_token() -> Option<ClaudeToken> {
    let creds = read_json(&home().join(".claude").join(".credentials.json"))?;
    let oauth = creds.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?.to_string();
    let expires_at = oauth
        .get("expiresAt")
        .and_then(Value::as_f64)
        .map(|t| t as i64);
    Some(ClaudeToken {
        token,
        subscription_type: str_of(oauth.get("subscriptionType")),
        rate_limit_tier: str_of(oauth.get("rateLimitTier")),
        expires_at,
        expired: is_expired(expires_at),
    })
}

/// The Claude Code account (`~/.claude.json` `oauthAccount`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClaudeAccount {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub billing_type: Option<String>,
    pub organization_type: Option<String>,
    pub seat_tier: Option<String>,
    pub org_uuid: Option<String>,
    pub account_uuid: Option<String>,
}

pub fn account_of(claude_json: &Value) -> ClaudeAccount {
    let o = claude_json.get("oauthAccount");
    let get = |k: &str| str_of(o.and_then(|o| o.get(k)));
    ClaudeAccount {
        email: get("emailAddress"),
        display_name: get("displayName"),
        billing_type: get("billingType"),
        organization_type: get("organizationType"),
        seat_tier: get("seatTier"),
        org_uuid: get("organizationUuid"),
        account_uuid: get("accountUuid"),
    }
}

fn read_cli_account() -> ClaudeAccount {
    read_json(&home().join(".claude.json"))
        .map(|v| account_of(&v))
        .unwrap_or_default()
}

pub fn plan_name_of(
    subscription_type: Option<&str>,
    organization_type: Option<&str>,
    rate_limit_tier: Option<&str>,
) -> Option<String> {
    let raw = [subscription_type, organization_type, rate_limit_tier]
        .into_iter()
        .flatten()
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if raw.is_empty() {
        return None;
    }
    let named = [
        ("20x", "Max 20X"),
        ("5x", "Max 5X"),
        ("max", "Max"),
        ("pro", "Pro"),
        ("free", "Free"),
        ("team", "Team"),
        ("enterprise", "Enterprise"),
    ];
    for (needle, name) in named {
        if raw.contains(needle) {
            return Some(name.to_string());
        }
    }
    subscription_type.or(organization_type).map(str::to_string)
}

pub fn build_usage(
    account: &ClaudeAccount,
    token: Option<&ClaudeToken>,
    reply: Option<&UsageReply>,
) -> ClaudeUsage {
    let subscription_type = token.and_then(|t| t.subscription_type.clone());
    let rate_limit_tier = token.and_then(|t| t.rate_limit_tier.clone());
    ClaudeUsage {
        email: account.email.clone(),
        display_name: account.display_name.clone(),
        signed_in: token.is_some(),
        subscribed: subscription_type.is_some()
            || account.billing_type.as_deref().is_some_and(|b| b != "none")
            || account.seat_tier.is_some(),
        plan_name: plan_name_of(
            subscription_type.as_deref(),
            account.organization_type.as_deref(),
            rate_limit_tier.as_deref(),
        ),
        subscription_type,
        five_hour: window_of(reply.and_then(|r| r.five_hour.as_ref())),
        weekly: window_of(reply.and_then(|r| r.seven_day.as_ref())),
        weekly_opus: reply
            .and_then(|r| r.seven_day_opus.as_ref())
            .map(|w| window_of(Some(w))),
        weekly_sonnet: reply
            .and_then(|r| r.seven_day_sonnet.as_ref())
            .map(|w| window_of(Some(w))),
        reset_credits: reply.and_then(|r| reset_credits_of(&r.rest)),
        billing_type: account.billing_type.clone(),
        rate_limit_tier,
        org_uuid: account.org_uuid.clone(),
    }
}

/// The account Claude is signed into now. Claude keeps only that one token.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveClaude {
    pub id: String,
    pub label: Option<String>,
    /// The identity of the app whose token is used (empty when it is unknown).
    pub account: ClaudeAccount,
    pub token: Option<ClaudeToken>,
}

/// Choose the token as the old code did (live desktop > live CLI > expired desktop > expired
/// CLI), then take the identity from the same app as that token.
pub fn resolve_live(
    cli_account: ClaudeAccount,
    desktop: Option<DesktopToken>,
    cli: Option<ClaudeToken>,
) -> Option<LiveClaude> {
    let desktop_live = desktop.as_ref().is_some_and(|d| !d.token.expired);
    let cli_live = cli.as_ref().is_some_and(|t| !t.expired);
    let use_desktop = desktop_live || (desktop.is_some() && !cli_live);
    if use_desktop {
        let d = desktop.expect("checked above");
        let same_account = matches!(
            (&d.account_uuid, &cli_account.account_uuid),
            (Some(a), Some(b)) if a.trim().eq_ignore_ascii_case(b.trim())
        );
        if !same_account {
            // Claude desktop is signed into an account we hold no email for: never borrow
            // Claude Code's identity for it.
            let id = match &d.account_uuid {
                Some(uuid) if !uuid.trim().is_empty() => {
                    format!("desktop-{}", uuid.trim().to_lowercase())
                }
                _ => "claude-desktop".to_string(),
            };
            return Some(LiveClaude {
                id,
                label: Some("Claude desktop".to_string()),
                account: ClaudeAccount::default(),
                token: Some(d.token),
            });
        }
        return Some(live_with(cli_account, Some(d.token)));
    }
    if cli.is_none() && cli_account.email.is_none() {
        return None;
    }
    Some(live_with(cli_account, cli))
}

fn live_with(account: ClaudeAccount, token: Option<ClaudeToken>) -> LiveClaude {
    let trimmed = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let label = trimmed(&account.email).or_else(|| trimmed(&account.display_name));
    let id = label
        .clone()
        .unwrap_or_else(|| "signed-in".to_string())
        .to_lowercase();
    LiveClaude {
        id,
        label,
        account,
        token,
    }
}

fn read_live_claude() -> Option<LiveClaude> {
    let account = read_cli_account();
    let desktop = read_desktop_token();
    let cli = if desktop.as_ref().is_some_and(|d| !d.token.expired) {
        None
    } else {
        read_cli_token()
    };
    resolve_live(account, desktop, cli)
}

// ---- fetch ----------------------------------------------------------------------------------

/// GET the usage reply. 401/403 -> AuthRejected, 429 -> RateLimited.
pub async fn fetch_reply(
    http: &reqwest::Client,
    token: &str,
) -> Result<(UsageReply, Vec<u8>), ProviderError> {
    let resp = http
        .get(USAGE_URL)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("anthropic-version", "2023-06-01")
        .header("User-Agent", "claude-cli/2.1.293")
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            log::warn!("claude usage request failed: {}", e.without_url());
            ProviderError::Network
        })?;
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
    Ok((parse_usage_reply(&bytes)?, bytes))
}

async fn fetch(
    http: reqwest::Client,
    live: LiveClaude,
) -> Result<ClaudeUsage, FetchError<ClaudeUsage>> {
    // Without live numbers, still show the account/plan we know locally; that partial snapshot
    // is never persisted as "last known usage".
    let fallback = build_usage(&live.account, live.token.as_ref(), None);
    let fail = |error| FetchError {
        error,
        fallback: Some(Box::new(fallback.clone())),
    };
    let Some(token) = live.token.as_ref() else {
        return Err(fail(ProviderError::MissingCredentials));
    };
    if token.expired {
        return Err(fail(ProviderError::AuthRejected));
    }
    let (reply, _) = fetch_reply(&http, &token.token).await.map_err(fail)?;
    Ok(build_usage(&live.account, Some(token), Some(&reply)))
}

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<ClaudeUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let live = tokio::task::spawn_blocking(read_live_claude)
        .await
        .ok()
        .flatten();
    let rows: Vec<LiveAccount<'_, ClaudeUsage>> = live
        .into_iter()
        .map(|live| {
            let id = live.id.clone();
            let label = live.label.clone();
            let http = core.http.clone();
            LiveAccount {
                id: id.clone(),
                label,
                active: true,
                load: Box::pin(async move {
                    CACHE
                        .get(&core.cache, &id, move |_| fetch(http, live))
                        .await
                }),
                label_from: None,
            }
        })
        .collect();
    let tracked: Vec<TrackedAccount<ClaudeUsage>> =
        collect_accounts(&core.seen, &core.cache, PROVIDER, SECTION, rows).await;
    tracked
        .into_iter()
        .filter_map(|row| serde_json::to_value(row).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::UsageCache;
    use serde_json::json;

    const OK: &[u8] = include_bytes!("../../tests/fixtures/claude/ok.json");
    const INVALID: &[u8] = include_bytes!("../../tests/fixtures/claude/invalid.json");

    fn token(name: &str) -> ClaudeToken {
        ClaudeToken {
            token: name.to_string(),
            subscription_type: Some("max".into()),
            rate_limit_tier: Some("default_claude_max_20x".into()),
            expires_at: None,
            expired: false,
        }
    }

    fn cli_account(uuid: &str) -> ClaudeAccount {
        account_of(&json!({ "oauthAccount": {
            "emailAddress": "cli@example.com", "displayName": "CLI", "billingType": "stripe_subscription",
            "organizationType": "claude_max", "organizationUuid": "org-1", "accountUuid": uuid } }))
    }

    #[test]
    fn ok_fixture_parses_to_expected_values() {
        let reply = parse_usage_reply(OK).expect("ok.json parses");
        let usage = build_usage(&cli_account("u1"), Some(&token("t")), Some(&reply));
        assert_eq!(usage.email.as_deref(), Some("cli@example.com"));
        assert_eq!(usage.plan_name.as_deref(), Some("Max 20X"));
        assert!(usage.signed_in && usage.subscribed);
        assert_eq!(usage.five_hour.percent, Some(68.0));
        assert_eq!(
            usage.five_hour.resets_at.as_deref(),
            Some("2026-10-19T00:49:59.594Z")
        );
        assert_eq!(usage.weekly.percent, Some(47.0));
        assert_eq!(
            usage.weekly.resets_at.as_deref(),
            Some("2026-10-21T04:59:59.594Z")
        );
        // JSON field names are the old camelCase ones
        let v = serde_json::to_value(&usage).unwrap();
        for k in [
            "fiveHour",
            "weekly",
            "weeklyOpus",
            "weeklySonnet",
            "resetCredits",
            "planName",
            "orgUuid",
        ] {
            assert!(v.get(k).is_some(), "{k}");
        }
        assert_eq!(usage.weekly_opus, None);
        assert_eq!(usage.reset_credits, None);
        assert_eq!(v["fiveHour"]["resetsAt"], json!("2026-10-19T00:49:59.594Z"));
    }

    #[test]
    fn window_and_plan_helpers_match_the_old_code() {
        let w: WindowReply =
            serde_json::from_value(json!({ "percent": 5, "resetsAt": 1_760_000_000 })).unwrap();
        assert_eq!(
            window_of(Some(&w)),
            ClaudeWindow {
                percent: Some(5.0),
                resets_at: Some("2025-10-09T08:53:20.000Z".into())
            }
        );
        assert_eq!(
            plan_name_of(Some("pro"), None, None).as_deref(),
            Some("Pro")
        );
        assert_eq!(
            plan_name_of(None, Some("odd_org"), None).as_deref(),
            Some("odd_org")
        );
        assert_eq!(plan_name_of(None, None, None), None);
        let rest =
            json!({ "cedar_ember": { "eligible": false }, "juniper_tide": { "grants": [1, 2] } });
        assert_eq!(reset_credits_of(rest.as_object().unwrap()), Some(2));
    }

    #[test]
    fn pick_token_prefers_live_claude_code() {
        let entries = json!({
            "other": { "token": "a", "expiresAt": 9_999_999_999_999i64 },
            "x:claude_code": { "token": "b", "expiresAt": 9_999_999_999_999i64 },
            "old:claude_code": { "token": "c", "expiresAt": 1 },
        });
        assert_eq!(pick_token(&entries).unwrap().token, "b");
        assert!(
            pick_token(&json!({ "only": { "token": "c", "expiresAt": 1 } }))
                .unwrap()
                .expired
        );
    }

    #[tokio::test]
    async fn invalid_fixture_is_schema_changed_and_keeps_last_good() {
        assert_eq!(
            parse_usage_reply(INVALID).err(),
            Some(ProviderError::SchemaChanged)
        );
        assert_eq!(
            parse_usage_reply(b"{}").err(),
            Some(ProviderError::SchemaChanged)
        );

        let dir = std::env::temp_dir().join(format!("usagedeck-claude-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let provider: CachedProvider<ClaudeUsage> =
            CachedProvider::new(SECTION, Duration::from_secs(60), None);
        let account = cli_account("u1");
        let parse = |bytes: &'static [u8]| {
            let account = account.clone();
            move |_| async move {
                let reply = parse_usage_reply(bytes).map_err(FetchError::from)?;
                Ok(build_usage(&account, Some(&token("t")), Some(&reply)))
            }
        };
        let good = provider.get(&cache, "cli@example.com", parse(OK)).await;
        assert!(!good.stale);
        provider.invalidate(None);
        let bad = provider
            .get(&cache, "cli@example.com", parse(INVALID))
            .await;
        assert_eq!(bad.error, Some(ProviderError::SchemaChanged));
        assert!(bad.stale);
        assert_eq!(bad.usage, good.usage);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn desktop_token_is_not_filed_under_a_different_cli_email() {
        let desktop = DesktopToken {
            token: token("desktop"),
            account_uuid: Some("UUID-DESKTOP".into()),
        };
        let live = resolve_live(
            cli_account("uuid-cli"),
            Some(desktop.clone()),
            Some(token("cli")),
        )
        .unwrap();
        assert_eq!(live.token.as_ref().unwrap().token, "desktop");
        assert_eq!(live.id, "desktop-uuid-desktop");
        assert_eq!(live.label.as_deref(), Some("Claude desktop"));
        assert_eq!(live.account.email, None);
        let usage = build_usage(&live.account, live.token.as_ref(), None);
        assert_eq!(usage.email, None);
        assert_eq!(usage.org_uuid, None);

        // desktop without a known account: still not the CLI email
        let unknown = DesktopToken {
            account_uuid: None,
            ..desktop.clone()
        };
        assert_eq!(
            resolve_live(cli_account("uuid-cli"), Some(unknown), None)
                .unwrap()
                .id,
            "claude-desktop"
        );

        // same account in both apps: the CLI email is right
        let same = resolve_live(cli_account("uuid-desktop"), Some(desktop.clone()), None).unwrap();
        assert_eq!(same.id, "cli@example.com");
        assert_eq!(same.account.email.as_deref(), Some("cli@example.com"));

        // an expired desktop token with a live CLI token: the CLI token and its own email
        let expired = DesktopToken {
            token: ClaudeToken {
                expired: true,
                ..token("desktop")
            },
            ..desktop
        };
        let cli = resolve_live(cli_account("uuid-cli"), Some(expired), Some(token("cli"))).unwrap();
        assert_eq!(cli.token.unwrap().token, "cli");
        assert_eq!(cli.id, "cli@example.com");

        assert_eq!(resolve_live(ClaudeAccount::default(), None, None), None);
    }

    /// Live check against the user's own sign-in. Prints only the parsed summary. Set
    /// USAGEDECK_FIXTURE_OUT to a scratch path to save the raw reply for redaction.
    #[tokio::test]
    #[ignore]
    async fn live_claude() {
        let live = read_live_claude().expect("a local Claude sign-in");
        let token = live.token.clone().expect("a token");
        println!(
            "identity: {} (desktop/cli by id), token expired: {}",
            if live.account.email.is_some() {
                "email"
            } else {
                "none"
            },
            token.expired
        );
        let (reply, raw) = fetch_reply(&reqwest::Client::new(), &token.token)
            .await
            .expect("live reply");
        if let Some(out) = std::env::var_os("USAGEDECK_FIXTURE_OUT") {
            std::fs::write(out, raw).unwrap();
        }
        let u = build_usage(&live.account, Some(&token), Some(&reply));
        println!(
            "plan={:?} 5h={:?}% resets {:?} | weekly={:?}% resets {:?} | opus={:?} sonnet={:?} credits={:?}",
            u.plan_name, u.five_hour.percent, u.five_hour.resets_at, u.weekly.percent, u.weekly.resets_at,
            u.weekly_opus.map(|w| w.percent), u.weekly_sonnet.map(|w| w.percent), u.reset_credits
        );
    }
}
