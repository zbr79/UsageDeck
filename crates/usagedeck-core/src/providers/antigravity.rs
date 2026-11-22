//! Google Antigravity usage limits (port of `pingdotgg/t3code`'s `antigravityUsageLimits.ts`).
//!
//! Credentials belong to Antigravity / Gemini CLI and are strictly read-only here:
//! 1. Windows Credential Manager target `"gemini:antigravity"` (decoding Go keyring
//!    `go-keyring-base64:` / `go-keyring-encoded:` payloads when present).
//! 2. Fallback files under `%USERPROFILE%\.gemini\antigravity-acp\` (`antigravity-oauth-token`
//!    and `acp_token.json`).
//!
//! Usage is fetched via `POST /v1internal:retrieveUserQuotaSummary` on
//! `daily-cloudcode-pa.googleapis.com` (falling back to `cloudcode-pa.googleapis.com`).
//! Disabled buckets and untouched rolling-window reset timestamps (`remainingFraction == 1.0`
//! with no description and `resetTime ≈ now + window`) are ignored.

use super::{
    collect_accounts, CachedProvider, FetchError, LiveAccount, ProviderError, TrackedAccount,
};
use crate::Core;
use base64::engine::general_purpose::{STANDARD as B64, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

const DAILY_QUOTA_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
const PROD_QUOTA_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
const USER_AGENT: &str = "antigravity/hub/2.24.0 (aidev_client; os_type=win32; arch=x64)";
const SECTION: &str = "antigravity";
const PROVIDER: &str = "antigravity";

static CACHE: LazyLock<CachedProvider<AntigravityUsage>> = LazyLock::new(|| {
    CachedProvider::new(
        SECTION,
        Duration::from_secs(60),
        Some(Duration::from_secs(60)),
    )
});

/// Antigravity 5-hour and weekly quota usage windows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AntigravityUsage {
    pub email: Option<String>,
    pub five_hour_percent: Option<f64>,
    pub five_hour_resets_at: Option<String>,
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at: Option<String>,
}

/// Read-only OAuth credential discovered for Antigravity.
#[derive(Clone, PartialEq)]
pub struct AntigravityCredential {
    pub access_token: String,
    pub email: Option<String>,
}

impl std::fmt::Debug for AntigravityCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AntigravityCredential")
            .field("access_token", &"<redacted>")
            .field("email", &self.email)
            .finish()
    }
}

// ---- credential discovery (read-only) -------------------------------------------------------

/// Decode a Go keyring string (`go-keyring-base64:...` / `go-keyring-encoded:...`) into its
/// underlying UTF-8 payload, or return plain text trimmed of whitespace and trailing NULs.
pub fn decode_keyring_payload(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if trimmed.is_empty() {
        return None;
    }
    let encoded = trimmed
        .strip_prefix("go-keyring-base64:")
        .or_else(|| trimmed.strip_prefix("go-keyring-encoded:"));
    if let Some(b64) = encoded {
        let clean = b64.trim();
        let bytes = B64
            .decode(clean)
            .or_else(|_| STANDARD_NO_PAD.decode(clean.trim_end_matches('=')))
            .ok()?;
        let decoded = String::from_utf8(bytes).ok()?;
        let cleaned = decoded.trim_matches(|c: char| c.is_whitespace() || c == '\0');
        return (!cleaned.is_empty()).then(|| cleaned.to_string());
    }
    Some(trimmed.to_string())
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

/// Parse an Antigravity OAuth JSON payload (optionally wrapped in a `go-keyring-*:` prefix).
pub fn parse_credential_json(raw: &str) -> Option<AntigravityCredential> {
    let decoded = decode_keyring_payload(raw)?;
    let json: Value = serde_json::from_str(&decoded).ok()?;
    let token_obj = json.get("token");
    let access_token = token_obj
        .and_then(|t| t.get("access_token").or_else(|| t.get("accessToken")))
        .or_else(|| json.get("access_token"))
        .or_else(|| json.get("accessToken"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();

    let email = [
        json.get("email"),
        json.pointer("/account/email"),
        json.pointer("/user/email"),
        token_obj.and_then(|t| t.get("email")),
    ]
    .into_iter()
    .flatten()
    .find_map(|v| {
        v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
    .or_else(|| {
        token_obj
            .and_then(|t| t.get("id_token").or_else(|| t.get("idToken")))
            .or_else(|| json.get("id_token"))
            .or_else(|| json.get("idToken"))
            .and_then(Value::as_str)
            .and_then(jwt_email)
    });

    Some(AntigravityCredential {
        access_token,
        email,
    })
}

fn read_keyring_credential() -> Option<AntigravityCredential> {
    // Go's zalando/go-keyring stores Windows generic credentials under TargetName "gemini:antigravity".
    keyring::Entry::new_with_target("gemini:antigravity", "gemini", "antigravity")
        .ok()
        .and_then(|e| e.get_password().ok())
        .and_then(|s| parse_credential_json(&s))
        .or_else(|| {
            keyring::Entry::new("gemini", "antigravity")
                .ok()
                .and_then(|e| e.get_password().ok())
                .and_then(|s| parse_credential_json(&s))
        })
}

fn read_file_credential_in(home: &Path) -> Option<AntigravityCredential> {
    let dir = home.join(".gemini").join("antigravity-acp");
    for filename in ["antigravity-oauth-token", "acp_token.json"] {
        let path = dir.join(filename);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(cred) = parse_credential_json(&text) {
                return Some(cred);
            }
        }
    }
    None
}

fn read_credential() -> Option<AntigravityCredential> {
    if !super::codex::real_sign_ins() {
        return None;
    }
    if let Some(cred) = read_keyring_credential() {
        return Some(cred);
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())?;
    read_file_credential_in(&home)
}

// ---- quota summary parsing ------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowKind {
    FiveHour,
    Weekly,
}

impl WindowKind {
    fn duration(self) -> ChronoDuration {
        match self {
            Self::FiveHour => ChronoDuration::hours(5),
            Self::Weekly => ChronoDuration::days(7),
        }
    }

    fn parse(window: Option<&str>, bucket_id: Option<&str>) -> Option<Self> {
        if let Some(w) = window.map(str::trim) {
            if w.eq_ignore_ascii_case("5h")
                || w.eq_ignore_ascii_case("five_hour")
                || w.eq_ignore_ascii_case("5_hour")
            {
                return Some(Self::FiveHour);
            }
            if w.eq_ignore_ascii_case("weekly")
                || w.eq_ignore_ascii_case("7d")
                || w.eq_ignore_ascii_case("seven_day")
            {
                return Some(Self::Weekly);
            }
        }
        let id = bucket_id?.trim().to_ascii_lowercase();
        if id.ends_with("-5h") || id.ends_with("_5h") {
            Some(Self::FiveHour)
        } else if id.ends_with("-weekly") || id.ends_with("_weekly") {
            Some(Self::Weekly)
        } else {
            None
        }
    }
}

struct ParsedBucket {
    kind: WindowKind,
    percent_used: f64,
    resets_at: Option<String>,
}

fn parse_bucket(bucket: &Value, now: DateTime<Utc>) -> Option<ParsedBucket> {
    let obj = bucket.as_object()?;
    if obj.get("disabled") == Some(&Value::Bool(true)) {
        return None;
    }
    let kind = WindowKind::parse(
        obj.get("window").and_then(Value::as_str),
        obj.get("bucketId")
            .or_else(|| obj.get("bucket_id"))
            .and_then(Value::as_str),
    )?;
    let remaining_fraction = obj
        .get("remainingFraction")
        .or_else(|| obj.get("remaining_fraction"))
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())?
        .clamp(0.0, 1.0);

    let percent_used = ((1.0 - remaining_fraction) * 100.0).clamp(0.0, 100.0);
    let has_description = obj
        .get("description")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());

    let raw_reset = obj
        .get("resetTime")
        .or_else(|| obj.get("reset_time"))
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
        .map(|d| d.with_timezone(&Utc));

    // For an untouched bucket (remainingFraction == 1.0, no description), Google returns a rolling
    // resetTime ≈ now + window. Drop that synthetic rolling timestamp so idle bars stay clean.
    let resets_at = raw_reset.and_then(|reset_dt| {
        let untouched = (remaining_fraction - 1.0).abs() < 1e-9;
        if untouched && !has_description {
            let expected = now + kind.duration();
            if (reset_dt - expected).num_seconds().abs() <= 60 {
                return None;
            }
        }
        Some(reset_dt.to_rfc3339_opts(SecondsFormat::Millis, true))
    });

    Some(ParsedBucket {
        kind,
        percent_used,
        resets_at,
    })
}

fn merge_window(
    slot_pct: &mut Option<f64>,
    slot_reset: &mut Option<String>,
    candidate: ParsedBucket,
) {
    let take = match *slot_pct {
        None => true,
        Some(prev) if candidate.percent_used > prev + 1e-9 => true,
        Some(prev) if (candidate.percent_used - prev).abs() <= 1e-9 => {
            slot_reset.is_none() && candidate.resets_at.is_some()
        }
        Some(_) => false,
    };
    if take {
        *slot_pct = Some(candidate.percent_used);
        *slot_reset = candidate.resets_at;
    }
}

/// Parse a `retrieveUserQuotaSummary` JSON reply into `AntigravityUsage`.
pub fn parse_quota_summary(
    bytes: &[u8],
    email: Option<String>,
    now: DateTime<Utc>,
) -> Result<AntigravityUsage, ProviderError> {
    let reply: Value = super::parse_reply(bytes)?;
    let Some(root) = reply.as_object() else {
        return Err(ProviderError::SchemaChanged);
    };

    let mut usage = AntigravityUsage {
        email,
        ..AntigravityUsage::default()
    };

    if let Some(groups) = root.get("groups").and_then(Value::as_array) {
        for group in groups {
            if let Some(buckets) = group.get("buckets").and_then(Value::as_array) {
                for bucket in buckets {
                    if let Some(parsed) = parse_bucket(bucket, now) {
                        match parsed.kind {
                            WindowKind::FiveHour => merge_window(
                                &mut usage.five_hour_percent,
                                &mut usage.five_hour_resets_at,
                                parsed,
                            ),
                            WindowKind::Weekly => merge_window(
                                &mut usage.weekly_percent,
                                &mut usage.weekly_resets_at,
                                parsed,
                            ),
                        }
                    }
                }
            }
        }
    }

    if let Some(buckets) = root.get("buckets").and_then(Value::as_array) {
        for bucket in buckets {
            if let Some(parsed) = parse_bucket(bucket, now) {
                match parsed.kind {
                    WindowKind::FiveHour => merge_window(
                        &mut usage.five_hour_percent,
                        &mut usage.five_hour_resets_at,
                        parsed,
                    ),
                    WindowKind::Weekly => merge_window(
                        &mut usage.weekly_percent,
                        &mut usage.weekly_resets_at,
                        parsed,
                    ),
                }
            }
        }
    }

    if usage.five_hour_percent.is_none() && usage.weekly_percent.is_none() {
        log::warn!("antigravity quota summary reply held no active 5h/weekly buckets");
        return Err(ProviderError::SchemaChanged);
    }
    Ok(usage)
}

// ---- fetch ----------------------------------------------------------------------------------

async fn post_quota_summary(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
    email: Option<String>,
) -> Result<AntigravityUsage, ProviderError> {
    let resp = http
        .post(url)
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .body("{}")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            log::warn!("antigravity quota request failed: {}", e.without_url());
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
    parse_quota_summary(&bytes, email, Utc::now())
}

async fn fetch(
    http: reqwest::Client,
    cred: AntigravityCredential,
) -> Result<AntigravityUsage, FetchError<AntigravityUsage>> {
    match post_quota_summary(
        &http,
        DAILY_QUOTA_URL,
        &cred.access_token,
        cred.email.clone(),
    )
    .await
    {
        Ok(usage) => Ok(usage),
        Err(err @ (ProviderError::AuthRejected | ProviderError::RateLimited)) => Err(err.into()),
        Err(_) => post_quota_summary(&http, PROD_QUOTA_URL, &cred.access_token, cred.email)
            .await
            .map_err(FetchError::from),
    }
}

/// Rows for Antigravity (`TrackedAccount<AntigravityUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let cred = tokio::task::spawn_blocking(read_credential)
        .await
        .ok()
        .flatten();
    let live: Vec<LiveAccount<'_, AntigravityUsage>> = cred
        .into_iter()
        .map(|cred| {
            let label = cred.email.clone().or_else(|| Some("Antigravity".into()));
            let id = cred
                .email
                .as_deref()
                .unwrap_or("antigravity")
                .trim()
                .to_lowercase();
            let http = core.http.clone();
            LiveAccount {
                id: id.clone(),
                label,
                active: true,
                load: Box::pin(async move {
                    CACHE
                        .get(&core.cache, &id, move |_| fetch(http, cred))
                        .await
                }),
                label_from: None,
            }
        })
        .collect();
    let rows: Vec<TrackedAccount<AntigravityUsage>> =
        collect_accounts(&core.seen, &core.cache, PROVIDER, SECTION, live).await;
    rows.into_iter()
        .filter_map(|r| serde_json::to_value(r).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::UsageCache;
    use serde_json::json;

    fn utc(iso: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(iso)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn credential_debug_redacts_access_token() {
        let cred = AntigravityCredential {
            access_token: "ya29.super-secret-oauth-token".into(),
            email: Some("dev@example.com".into()),
        };
        let dbg = format!("{cred:?}");
        assert!(!dbg.contains("ya29.super-secret-oauth-token"));
        assert!(dbg.contains("<redacted>"));
        assert!(dbg.contains("dev@example.com"));
    }

    #[test]
    fn decode_keyring_and_parse_credentials() {
        let raw_json = json!({
            "email": "user@example.com",
            "token": { "access_token": "ya29.abc123" }
        })
        .to_string();

        // go-keyring-base64: prefix
        let b64 = format!("go-keyring-base64:{}", B64.encode(&raw_json));
        let cred = parse_credential_json(&b64).expect("go-keyring-base64 parses");
        assert_eq!(cred.access_token, "ya29.abc123");
        assert_eq!(cred.email.as_deref(), Some("user@example.com"));

        // go-keyring-encoded: prefix
        let enc = format!("go-keyring-encoded:{}", B64.encode(&raw_json));
        let cred2 = parse_credential_json(&enc).expect("go-keyring-encoded parses");
        assert_eq!(cred2.access_token, "ya29.abc123");

        // Flat acp_token.json shape with email extracted from JWT id_token
        let payload_b64 = URL_SAFE_NO_PAD.encode(r#"{"email":"jwt@example.com"}"#);
        let jwt = format!("eyJhbGciOiJSUzI1NiJ9.{payload_b64}.sig");
        let flat = json!({ "access_token": "ya29.flat", "id_token": jwt }).to_string();
        let cred3 = parse_credential_json(&flat).expect("flat acp_token parses");
        assert_eq!(cred3.access_token, "ya29.flat");
        assert_eq!(cred3.email.as_deref(), Some("jwt@example.com"));

        // Invalid or empty
        assert_eq!(
            parse_credential_json("go-keyring-base64:!!!not-b64!!!"),
            None
        );
        assert_eq!(parse_credential_json("{}"), None);
    }

    #[test]
    fn fallback_file_discovery_reads_antigravity_acp_dir() {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-antigravity-files-{}",
            std::process::id()
        ));
        let acp = dir.join(".gemini").join("antigravity-acp");
        std::fs::create_dir_all(&acp).unwrap();
        let payload =
            json!({ "token": { "access_token": "ya29.from-file" }, "email": "f@example.com" });
        std::fs::write(acp.join("acp_token.json"), payload.to_string()).unwrap();

        let cred = read_file_credential_in(&dir).expect("reads acp_token.json");
        assert_eq!(cred.access_token, "ya29.from-file");
        assert_eq!(cred.email.as_deref(), Some("f@example.com"));

        // antigravity-oauth-token takes precedence when present
        let primary = json!({ "access_token": "ya29.primary-file" });
        std::fs::write(acp.join("antigravity-oauth-token"), primary.to_string()).unwrap();
        let cred2 = read_file_credential_in(&dir).expect("reads antigravity-oauth-token");
        assert_eq!(cred2.access_token, "ya29.primary-file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_quota_summary_handles_groups_top_level_disabled_and_untouched_rolling_resets() {
        let now = utc("2026-10-21T10:16:04Z");
        let payload = json!({
            "groups": [
                {
                    "displayName": "Gemini Models",
                    "buckets": [
                        {
                            "bucketId": "gemini-5h",
                            "window": "5h",
                            "remainingFraction": 0.85,
                            "resetTime": "2026-10-21T13:29:53Z",
                            "description": "You have used some of your 5-hour limit"
                        },
                        {
                            "bucketId": "gemini-weekly",
                            "window": "weekly",
                            "remainingFraction": 0.60,
                            "resetTime": "2026-10-25T10:00:00Z"
                        },
                        {
                            "bucketId": "disabled-5h",
                            "window": "5h",
                            "remainingFraction": 0.0,
                            "disabled": true
                        }
                    ]
                },
                {
                    "displayName": "Claude and GPT models",
                    "buckets": [
                        {
                            "bucketId": "3p-5h",
                            "window": "5h",
                            "remainingFraction": 1.0,
                            "resetTime": "2026-10-21T15:16:05Z"
                        },
                        {
                            "bucketId": "3p-weekly",
                            "window": "weekly",
                            "remainingFraction": 1.0,
                            "resetTime": "2026-10-28T10:16:05Z"
                        }
                    ]
                }
            ]
        });

        let usage = parse_quota_summary(
            &serde_json::to_vec(&payload).unwrap(),
            Some("user@example.com".into()),
            now,
        )
        .expect("summary parses");
        assert_eq!(usage.email.as_deref(), Some("user@example.com"));
        assert!((usage.five_hour_percent.unwrap() - 15.0).abs() < 1e-6);
        assert_eq!(
            usage.five_hour_resets_at.as_deref(),
            Some("2026-10-21T13:29:53.000Z")
        );
        assert!((usage.weekly_percent.unwrap() - 40.0).abs() < 1e-6);
        assert_eq!(
            usage.weekly_resets_at.as_deref(),
            Some("2026-10-25T10:00:00.000Z")
        );

        // Untouched rolling-window buckets (top-level `buckets[]`) keep 0% used and drop resetTime.
        let untouched = json!({
            "buckets": [
                {
                    "bucketId": "3p-5h",
                    "window": "5h",
                    "remainingFraction": 1.0,
                    "resetTime": "2026-10-21T15:16:05Z"
                },
                {
                    "bucketId": "3p-weekly",
                    "window": "weekly",
                    "remainingFraction": 1.0,
                    "resetTime": "2026-10-28T10:16:05Z"
                }
            ]
        });
        let idle = parse_quota_summary(&serde_json::to_vec(&untouched).unwrap(), None, now)
            .expect("untouched summary parses");
        assert_eq!(idle.five_hour_percent, Some(0.0));
        assert_eq!(idle.five_hour_resets_at, None);
        assert_eq!(idle.weekly_percent, Some(0.0));
        assert_eq!(idle.weekly_resets_at, None);

        // All disabled or empty -> SchemaChanged
        let all_disabled = json!({
            "buckets": [
                { "bucketId": "gemini-5h", "window": "5h", "remainingFraction": 0.5, "disabled": true }
            ]
        });
        assert_eq!(
            parse_quota_summary(&serde_json::to_vec(&all_disabled).unwrap(), None, now),
            Err(ProviderError::SchemaChanged)
        );
        assert_eq!(
            parse_quota_summary(b"{}", None, now),
            Err(ProviderError::SchemaChanged)
        );
    }

    #[tokio::test]
    async fn invalid_reply_keeps_last_good_cached_usage() {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-antigravity-cache-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let provider: CachedProvider<AntigravityUsage> =
            CachedProvider::new(SECTION, Duration::from_secs(60), None);
        let now = utc("2026-10-21T10:16:04Z");
        let ok = serde_json::to_vec(&json!({
            "buckets": [{ "window": "5h", "remainingFraction": 0.75, "resetTime": "2026-10-21T14:00:00Z" }]
        }))
        .unwrap();

        let good = provider
            .get(&cache, "user@example.com", |_| {
                let ok = ok.clone();
                async move {
                    parse_quota_summary(&ok, Some("user@example.com".into()), now)
                        .map_err(FetchError::from)
                }
            })
            .await;
        assert!(!good.stale && good.usage.is_some());

        provider.invalidate(None);
        let bad = provider
            .get(&cache, "user@example.com", |_| async move {
                parse_quota_summary(b"{}", None, now).map_err(FetchError::from)
            })
            .await;
        assert_eq!(bad.error, Some(ProviderError::SchemaChanged));
        assert!(bad.stale);
        assert_eq!(bad.usage, good.usage);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
