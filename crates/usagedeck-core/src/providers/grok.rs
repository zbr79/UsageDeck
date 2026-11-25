//! Grok usage per stored account (official xAI Grok CLI, Grok Bot, or Cursor fallback).
//! Port of lib/grokbot.ts and app/api/grok/route.ts, extended with xAI Grok CLI support.
//!
//! - Official xAI Grok CLI: reads `~/.grok/auth.json` (or `GROK_HOME/auth.json` / `GROK_AUTH`),
//!   skipped when `XAI_API_KEY` is set, and queries `cli-chat-proxy.grok.com/v1/billing?format=credits`.
//! - Grok Bot: reads `%APPDATA%\Grok Bot\sand-secrets.json`, sealed with its safeStorage key.
//!   If `sand-secrets.json` exists but fails to decrypt, we fail closed and do NOT fall back to
//!   Cursor's `state.vscdb`.
//! - Cursor fallback: only when neither Grok CLI nor Grok Bot has a sign-in (and Grok Bot is not
//!   unreadable). All credentials are strictly read-only.

use super::util::{number_or_null, string_or_null, to_iso};
use super::{
    collect_accounts, CachedProvider, FetchError, LiveAccount, ProviderError, TrackedAccount,
};
use crate::win::os_crypt::{app_key, unseal, AppKey};
use crate::Core;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

const SECTION: &str = "grokbot";
const PROVIDER: &str = "grok";
const XAI_BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
const XAI_CLI_USER_AGENT: &str = "grok-cli/1.0.77";
const XAI_AUTH_KEYS: [&str; 2] = [
    "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828",
    "https://accounts.x.ai/sign-in",
];

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
    /// `"grok-cli"`, `"grok-bot"`, or `"cursor"`: whose sign-in was used
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

#[derive(Clone, PartialEq)]
pub struct GrokAccount {
    pub id: String,
    pub token: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub source: &'static str,
    pub active: bool,
}

impl std::fmt::Debug for GrokAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrokAccount")
            .field("id", &self.id)
            .field("token", &"<redacted>")
            .field("email", &self.email)
            .field("name", &self.name)
            .field("source", &self.source)
            .field("active", &self.active)
            .finish()
    }
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

/// Extract a numeric value that may be either a bare JSON number or `{ "val": number }`.
fn metric_number(value: Option<&Value>) -> Option<f64> {
    let v = value?;
    number_or_null(v).or_else(|| v.get("val").and_then(number_or_null))
}

/// Parse the xAI Grok CLI billing reply (`GET /v1/billing?format=credits`) into `GrokUsage`.
pub fn parse_xai_billing_reply(
    bytes: &[u8],
    account: &GrokAccount,
) -> Result<GrokUsage, ProviderError> {
    let reply: Value = super::parse_reply(bytes)?;
    if !reply.is_object() {
        return Err(ProviderError::SchemaChanged);
    }
    let cfg = reply
        .get("config")
        .filter(|v| v.is_object())
        .unwrap_or(&reply);

    let current_period = cfg
        .get("currentPeriod")
        .or_else(|| cfg.get("current_period"));
    let reset_at = [
        current_period.and_then(|p| p.get("end")),
        current_period.and_then(|p| p.get("endTime")),
        current_period.and_then(|p| p.get("end_time")),
        cfg.get("billingPeriodEnd"),
        cfg.get("periodEnd"),
        cfg.get("period_end"),
        cfg.get("resetAt"),
        cfg.get("reset_at"),
    ]
    .into_iter()
    .flatten()
    .find_map(to_iso);

    let weekly_period = current_period
        .and_then(|p| p.get("type").or_else(|| p.get("periodType")))
        .or_else(|| cfg.get("periodType"))
        .or_else(|| cfg.get("period_type"))
        .and_then(Value::as_str)
        .is_some_and(|t| t == "USAGE_PERIOD_TYPE_WEEKLY");

    let percent_used = [
        cfg.get("creditUsagePercent"),
        cfg.get("usedPercent"),
        cfg.get("used_percent"),
    ]
    .into_iter()
    .flatten()
    .find_map(number_or_null)
    .map(|p| p.clamp(0.0, 100.0))
    .or_else(|| {
        [cfg.get("remainingPercent"), cfg.get("remaining_percent")]
            .into_iter()
            .flatten()
            .find_map(number_or_null)
            .map(|rem| (100.0 - rem).clamp(0.0, 100.0))
    })
    .or_else(|| {
        let total = metric_number(
            cfg.get("totalCredits")
                .or_else(|| cfg.get("total_credits"))
                .or_else(|| cfg.get("monthlyLimit"))
                .or_else(|| cfg.get("monthly_limit"))
                .or_else(|| cfg.get("limit")),
        )
        .filter(|t| *t > 0.0)?;
        if let Some(used) = metric_number(
            cfg.get("usedCredits")
                .or_else(|| cfg.get("used_credits"))
                .or_else(|| cfg.get("used")),
        ) {
            return Some((used / total * 100.0).clamp(0.0, 100.0));
        }
        let remaining = metric_number(
            cfg.get("remainingCredits")
                .or_else(|| cfg.get("remaining_credits"))
                .or_else(|| cfg.get("remaining")),
        )?;
        Some(((total - remaining) / total * 100.0).clamp(0.0, 100.0))
    })
    .or_else(|| {
        // Protobuf JSON omits scalar 0 fields right after a weekly reset.
        (weekly_period && reset_at.is_some()).then_some(0.0)
    });

    let Some(percent_used) = percent_used.filter(|p| p.is_finite()) else {
        log::warn!("grok cli billing reply holds no usable credit percentage");
        return Err(ProviderError::SchemaChanged);
    };

    let plan_label = [
        reply.get("subscriptionTierDisplay"),
        reply.get("subscription_tier_display"),
        reply.get("subscriptionTier"),
        reply.get("subscription_tier"),
        cfg.get("planName"),
        cfg.get("plan_name"),
        cfg.get("tier"),
    ]
    .into_iter()
    .flatten()
    .find_map(string_or_null)
    .or_else(|| Some("Grok CLI".to_string()));

    Ok(GrokUsage {
        email: account.email.clone(),
        name: account.name.clone(),
        percent_used: Some(percent_used),
        reset_at,
        plan_label,
        cursor_plan_name: None,
        source: account.source.to_string(),
    })
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

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

fn jwt_email(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let json: Value = serde_json::from_slice(&bytes).ok()?;
    json.get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Parse an xAI Grok CLI `auth.json` payload for a subscription session token.
pub fn parse_grok_cli_auth(json: &Value) -> Option<GrokAccount> {
    let obj = json.as_object()?;
    for key in XAI_AUTH_KEYS {
        let Some(entry) = obj.get(key) else {
            continue;
        };
        let token = entry
            .as_str()
            .or_else(|| entry.get("key").and_then(Value::as_str))
            .or_else(|| entry.get("access_token").and_then(Value::as_str))
            .or_else(|| entry.get("accessToken").and_then(Value::as_str))
            .or_else(|| entry.get("token").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(token) = token else {
            continue;
        };
        let email = entry
            .get("email")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| jwt_email(token));
        let id = email.as_deref().unwrap_or("grok-cli").trim().to_lowercase();
        return Some(GrokAccount {
            id,
            token: token.to_string(),
            email,
            name: None,
            source: "grok-cli",
            active: true,
        });
    }
    None
}

fn read_grok_cli_account_with(
    xai_api_key: Option<&str>,
    grok_auth: Option<&str>,
    grok_home: Option<&Path>,
    home: Option<&Path>,
) -> Option<GrokAccount> {
    // When XAI_API_KEY is set, the user is on pay-as-you-go API billing, not CLI subscription.
    if xai_api_key.is_some_and(|k| !k.trim().is_empty()) {
        return None;
    }
    if let Some(raw) = grok_auth.map(str::trim).filter(|s| !s.is_empty()) {
        if raw.starts_with('{') {
            if let Ok(json) = serde_json::from_str::<Value>(raw) {
                if let Some(acc) = parse_grok_cli_auth(&json) {
                    return Some(acc);
                }
            }
        } else {
            let candidate = PathBuf::from(raw);
            let path = if candidate.is_dir() {
                candidate.join("auth.json")
            } else {
                candidate
            };
            if let Some(acc) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|v| parse_grok_cli_auth(&v))
            {
                return Some(acc);
            }
        }
    }
    let mut candidates = Vec::new();
    if let Some(dir) = grok_home {
        candidates.push(dir.join("auth.json"));
    }
    if let Some(h) = home {
        candidates.push(h.join(".grok").join("auth.json"));
    }
    for path in candidates {
        if let Some(acc) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| parse_grok_cli_auth(&v))
        {
            return Some(acc);
        }
    }
    None
}

fn read_grok_cli_account() -> Option<GrokAccount> {
    if !super::codex::real_sign_ins() {
        return None;
    }
    let xai_api_key = std::env::var("XAI_API_KEY").ok();
    let grok_auth = std::env::var("GROK_AUTH").ok();
    let grok_home = std::env::var_os("GROK_HOME").map(PathBuf::from);
    let home = home_dir();
    read_grok_cli_account_with(
        xai_api_key.as_deref(),
        grok_auth.as_deref(),
        grok_home.as_deref(),
        home.as_deref(),
    )
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

#[derive(Debug, PartialEq)]
enum GrokBotRead {
    /// `sand-secrets.json` does not exist on disk.
    Missing,
    /// `sand-secrets.json` was read cleanly (empty when signed out).
    Accounts(Vec<GrokAccount>),
    /// `sand-secrets.json` exists but could not be parsed or decrypted (fail closed).
    Unreadable,
}

fn read_grok_bot_in(dir: &Path) -> GrokBotRead {
    let secrets_path = dir.join("sand-secrets.json");
    if !secrets_path.exists() {
        return GrokBotRead::Missing;
    }
    let Some(secrets) = std::fs::read_to_string(&secrets_path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        log::warn!("grok-bot: sand-secrets.json unreadable; not falling back to Cursor");
        return GrokBotRead::Unreadable;
    };
    let has_stored_accounts = secrets
        .get("cursor-accounts")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .is_some_and(|v| {
            v.get("accounts")
                .and_then(Value::as_object)
                .is_some_and(|m| !m.is_empty())
        });
    if !has_stored_accounts {
        return GrokBotRead::Accounts(Vec::new());
    }
    let Some(key) = app_key(&dir.join("Local State")) else {
        log::warn!("grok-bot: Local State key unreadable; not falling back to Cursor");
        return GrokBotRead::Unreadable;
    };
    let accounts = grok_bot_accounts(&secrets, &key);
    if accounts.is_empty() {
        log::warn!("grok-bot: stored accounts failed to decrypt; not falling back to Cursor");
        return GrokBotRead::Unreadable;
    }
    GrokBotRead::Accounts(accounts)
}

fn resolve_accounts(
    grok_cli: Option<GrokAccount>,
    grok_bot: GrokBotRead,
    cursor_fallback: impl FnOnce() -> Option<(String, Option<String>)>,
) -> Vec<GrokAccount> {
    let mut out = Vec::new();
    if let Some(cli) = grok_cli {
        out.push(cli);
    }
    match grok_bot {
        GrokBotRead::Accounts(bot) => {
            for acc in bot {
                if !out.iter().any(|existing| existing.id == acc.id) {
                    out.push(acc);
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
        GrokBotRead::Unreadable => {
            // Fail closed: do NOT silently fall back to Cursor's state.vscdb when Grok Bot's
            // own sand-secrets.json exists but failed to decrypt.
            return out;
        }
        GrokBotRead::Missing => {
            if !out.is_empty() {
                return out;
            }
        }
    }
    let Some((token, name)) = cursor_fallback() else {
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

/// Official xAI Grok CLI and Grok Bot sign-ins; when neither is present (and Grok Bot is not
/// unreadable), Cursor's current sign-in as a single account.
/// Blocking (files and SQLite): call through `tokio::task::spawn_blocking`, as `accounts` does.
fn read_accounts() -> Vec<GrokAccount> {
    let cli = read_grok_cli_account();
    let bot = if super::codex::real_sign_ins() {
        read_grok_bot_in(&appdata().join("Grok Bot"))
    } else {
        GrokBotRead::Missing
    };
    resolve_accounts(cli, bot, super::cursor::bearer_token)
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

/// GET official xAI Grok CLI billing credits endpoint.
async fn fetch_xai_billing(
    http: &reqwest::Client,
    account: &GrokAccount,
) -> Result<GrokUsage, ProviderError> {
    let resp = http
        .get(XAI_BILLING_URL)
        .bearer_auth(&account.token)
        .header("x-xai-token-auth", "xai-grok-cli")
        .header(reqwest::header::USER_AGENT, XAI_CLI_USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            log::warn!("grok cli billing request failed: {}", e.without_url());
            ProviderError::Network
        })?;
    super::note_retry_after(&resp);
    match resp.status().as_u16() {
        401 | 403 => return Err(ProviderError::AuthRejected),
        429 => return Err(ProviderError::RateLimited),
        s if !(200..300).contains(&s) => return Err(ProviderError::Http(s)),
        _ => {}
    }
    let bytes = resp.bytes().await.map_err(|_| ProviderError::Network)?;
    parse_xai_billing_reply(&bytes, account)
}

async fn fetch(
    http: reqwest::Client,
    account: GrokAccount,
) -> Result<GrokUsage, FetchError<GrokUsage>> {
    if account.source == "grok-cli" {
        return fetch_xai_billing(&http, &account)
            .await
            .map_err(FetchError::from);
    }
    let (reply, _) = fetch_reply(&http, &account.token).await?;
    Ok(build_usage(&reply, &account))
}

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<GrokUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let accounts = tokio::task::spawn_blocking(read_accounts)
        .await
        .unwrap_or_default();
    let found_any = !accounts.is_empty();
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
    if !found_any {
        super::record_provider_status(
            PROVIDER,
            "not found: no ~/.grok/auth.json or %APPDATA%\\Grok Bot\\sand-secrets.json sign-in",
            false,
        );
    } else {
        let (ok, summary) = super::summarize_tracked_rows(&rows);
        super::record_provider_status(PROVIDER, summary, ok);
    }
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

    fn cli_account() -> GrokAccount {
        GrokAccount {
            id: "cli@example.com".into(),
            token: "secret-xai-token".into(),
            email: Some("cli@example.com".into()),
            name: None,
            source: "grok-cli",
            active: true,
        }
    }

    #[test]
    fn grok_account_debug_redacts_token() {
        let dbg = format!("{:?}", cli_account());
        assert!(!dbg.contains("secret-xai-token"));
        assert!(dbg.contains("<redacted>"));
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

    #[test]
    fn grok_bot_fails_closed_without_falling_back_to_cursor() {
        let dir =
            std::env::temp_dir().join(format!("usagedeck-grok-failclosed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // sand-secrets.json exists with an account, but Local State is missing -> Unreadable
        let inner = json!({
            "active": "s1",
            "accounts": { "s1": { "cursor-access-token": "sealed-blob" } }
        });
        std::fs::write(
            dir.join("sand-secrets.json"),
            json!({ "cursor-accounts": inner.to_string() }).to_string(),
        )
        .unwrap();
        let read = read_grok_bot_in(&dir);
        assert_eq!(read, GrokBotRead::Unreadable);

        let mut cursor_called = false;
        let resolved = resolve_accounts(None, read, || {
            cursor_called = true;
            Some(("cursor-token".into(), Some("c@example.com".into())))
        });
        assert!(!cursor_called, "must not fall back to Cursor state.vscdb");
        assert!(resolved.is_empty());

        // When sand-secrets.json is absent, Cursor fallback is still used.
        let resolved_missing = resolve_accounts(None, GrokBotRead::Missing, || {
            cursor_called = true;
            Some(("cursor-token".into(), Some("c@example.com".into())))
        });
        assert!(cursor_called);
        assert_eq!(resolved_missing.len(), 1);
        assert_eq!(resolved_missing[0].source, "cursor");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_grok_cli_auth_supports_both_xai_keys_and_skips_when_api_key_set() {
        let primary = json!({
            "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828": {
                "key": "xai-session-1",
                "email": "dev@example.com"
            }
        });
        let acc = parse_grok_cli_auth(&primary).expect("primary key parses");
        assert_eq!(acc.token, "xai-session-1");
        assert_eq!(acc.id, "dev@example.com");
        assert_eq!(acc.source, "grok-cli");

        let secondary = json!({
            "https://accounts.x.ai/sign-in": {
                "access_token": "xai-session-2"
            }
        });
        let acc2 = parse_grok_cli_auth(&secondary).expect("secondary key parses");
        assert_eq!(acc2.token, "xai-session-2");
        assert_eq!(acc2.id, "grok-cli");

        // XAI_API_KEY set -> skip Grok CLI auth entirely
        let raw_auth = primary.to_string();
        assert_eq!(
            read_grok_cli_account_with(Some("xai-paid-key"), Some(&raw_auth), None, None),
            None
        );
        assert_eq!(
            read_grok_cli_account_with(None, Some(&raw_auth), None, None)
                .unwrap()
                .token,
            "xai-session-1"
        );
    }

    #[test]
    fn parse_xai_billing_reply_handles_credits_zero_reset_and_invalid_payloads() {
        let weekly = json!({
            "subscriptionTierDisplay": "SuperGrok",
            "config": {
                "creditUsagePercent": 42.5,
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "end": "2026-10-05T01:53:09Z"
                }
            }
        });
        let usage = parse_xai_billing_reply(&serde_json::to_vec(&weekly).unwrap(), &cli_account())
            .expect("weekly credits reply parses");
        assert_eq!(usage.percent_used, Some(42.5));
        assert_eq!(usage.reset_at.as_deref(), Some("2026-10-05T01:53:09.000Z"));
        assert_eq!(usage.plan_label.as_deref(), Some("SuperGrok"));
        assert_eq!(usage.source, "grok-cli");

        // Protobuf JSON omits creditUsagePercent right after weekly reset (0% used).
        let fresh_reset = json!({
            "config": {
                "currentPeriod": {
                    "type": "USAGE_PERIOD_TYPE_WEEKLY",
                    "end": "2026-10-12T00:00:00Z"
                }
            }
        });
        let zero =
            parse_xai_billing_reply(&serde_json::to_vec(&fresh_reset).unwrap(), &cli_account())
                .expect("fresh weekly reset parses as 0%");
        assert_eq!(zero.percent_used, Some(0.0));
        assert_eq!(zero.reset_at.as_deref(), Some("2026-10-12T00:00:00.000Z"));

        // Ratio fallback: remainingCredits / totalCredits
        let ratio = json!({
            "config": {
                "remainingCredits": 75.0,
                "totalCredits": 100.0,
                "billingPeriodEnd": "2026-11-01T00:00:00Z"
            }
        });
        let used_ratio =
            parse_xai_billing_reply(&serde_json::to_vec(&ratio).unwrap(), &cli_account())
                .expect("ratio credits reply parses");
        assert_eq!(used_ratio.percent_used, Some(25.0));

        // Nested { val } used / monthlyLimit fallback
        let monthly = json!({
            "config": {
                "used": { "val": 30 },
                "monthlyLimit": { "val": 120 }
            }
        });
        let used_monthly =
            parse_xai_billing_reply(&serde_json::to_vec(&monthly).unwrap(), &cli_account())
                .expect("monthly val reply parses");
        assert_eq!(used_monthly.percent_used, Some(25.0));

        // Empty or invalid payload -> SchemaChanged
        assert_eq!(
            parse_xai_billing_reply(b"{}", &cli_account()),
            Err(ProviderError::SchemaChanged)
        );
        assert_eq!(
            parse_xai_billing_reply(b"not-json", &cli_account()),
            Err(ProviderError::SchemaChanged)
        );
    }

    /// Live check against the user's own sign-in. Prints only the parsed summary. Set
    /// USAGEDECK_FIXTURE_OUT to a scratch path to save the raw reply for redaction.
    #[tokio::test]
    #[ignore]
    async fn live_grok() {
        crate::providers::codex::REAL_SIGN_INS.store(true, std::sync::atomic::Ordering::Relaxed);
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
        if account.source == "grok-cli" {
            let u = fetch_xai_billing(&reqwest::Client::new(), &account)
                .await
                .expect("live grok cli reply");
            println!(
                "plan={:?} used={:?}% resets {:?}",
                u.plan_label, u.percent_used, u.reset_at
            );
            return;
        }
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
