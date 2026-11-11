//! Pure rules for Cursor's stored sign-in: how the value in `state.vscdb` is encoded, what its
//! JWT says, the cursor.com session cookie built from it, and the redaction applied to anything
//! shown in diagnostics.
//!
//! Cursor 3.27 began storing `cursorAuth/accessToken` sealed with Electron safeStorage, as
//! `cursor-safe-storage:v1:` + base64. Sending that text as a bearer token is a 401, so the stored
//! value is classified here before it is used.
//!
//! No Windows or app dependencies, so the unit tests also run on Linux.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const SAFE_STORAGE_V1: &str = "cursor-safe-storage:v1:";
const SAFE_STORAGE: &str = "cursor-safe-storage:";

/// How Cursor stored its access token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoredToken<'a> {
    /// The token itself (Cursor before 3.27).
    Plain(&'a str),
    /// Base64 of an Electron safeStorage value: decrypt it with Cursor's `Local State` key.
    Sealed(&'a str),
    /// A `cursor-safe-storage:` version this build does not know, e.g. "cursor-safe-storage:v2".
    UnknownScheme(String),
}

pub fn classify(raw: &str) -> StoredToken<'_> {
    let raw = raw.trim();
    if let Some(sealed) = raw.strip_prefix(SAFE_STORAGE_V1) {
        return StoredToken::Sealed(sealed);
    }
    match raw.strip_prefix(SAFE_STORAGE) {
        Some(rest) => {
            let version: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect();
            StoredToken::UnknownScheme(format!("{SAFE_STORAGE}{version}"))
        }
        None => StoredToken::Plain(raw),
    }
}

/// An `ItemTable` value as text. SQLite hands back TEXT or a BLOB; a BLOB of ASCII UTF-16LE is
/// also valid UTF-8 with NULs in it, so it is recognised first.
pub fn decode_value(bytes: &[u8]) -> Option<String> {
    let utf16 = bytes.len() >= 2
        && bytes.len().is_multiple_of(2)
        && bytes.chunks_exact(2).all(|c| c[0] != 0 && c[1] == 0);
    if utf16 {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from(c[0])).collect();
        return String::from_utf16(&units).ok();
    }
    String::from_utf8(bytes.to_vec()).ok()
}

/// The claims of a JWT this app reads. None when the token is not a JWT.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Claims {
    pub sub: Option<String>,
    /// seconds since the epoch
    pub exp: Option<i64>,
}

pub fn claims(token: &str) -> Option<Claims> {
    let mut parts = token.split('.');
    let (_header, payload) = (parts.next()?, parts.next()?);
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let json: Value = serde_json::from_slice(&bytes).ok()?;
    let exp = &json["exp"];
    Some(Claims {
        sub: json["sub"].as_str().map(str::to_string),
        exp: exp.as_i64().or_else(|| exp.as_f64().map(|f| f as i64)),
    })
}

/// Cursor's user id: the part of `sub` after the last `|` ("auth0|user_01ABC" gives "user_01ABC").
/// None when it holds anything a cookie value cannot.
pub fn user_id(sub: &str) -> Option<&str> {
    let id = sub.rsplit('|').next()?.trim();
    let safe = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    (!id.is_empty() && id.chars().all(safe)).then_some(id)
}

/// The `Cookie` header cursor.com accepts for this access token, as the trackers build it:
/// `WorkosCursorSessionToken=<user id>%3A%3A<access token>`. None when the token names no user.
pub fn session_cookie(token: &str) -> Option<String> {
    let sub = claims(token)?.sub?;
    let id = user_id(&sub)?;
    let safe = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    token
        .chars()
        .all(safe)
        .then(|| format!("WorkosCursorSessionToken={id}%3A%3A{token}"))
}

/// A stable stand-in for a token in diagnostics: the first 8 hex characters of its SHA-256.
pub fn token_id(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether the cursor.com fallback is worth a request after the DashboardService failed.
/// A 401 or 403 for a token that has expired is Cursor's to renew, so nothing else is tried.
pub fn try_web_after(status_or_shape: WebReason, exp: Option<i64>, now: i64) -> bool {
    match status_or_shape {
        WebReason::Rejected => exp.is_none_or(|e| e > now),
        WebReason::BadReply | WebReason::ServerError => true,
    }
}

/// Why the DashboardService gave no usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebReason {
    /// 401 or 403
    Rejected,
    /// 2xx without the usage fields
    BadReply,
    /// any other non-2xx except 429
    ServerError,
}

const REDACTED: &str = "[redacted]";
/// Runs this long of token characters are taken for a secret. Words, paths and ids are shorter.
const SECRET_RUN: usize = 24;

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '%' | '.')
}

/// `text` with everything that could be a token replaced: JWTs, and any long run of base64 or
/// URL-encoded characters. Applied to every server reply and error shown in diagnostics.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let dense = run.chars().filter(|c| *c != '.').count();
        let jwt = run.starts_with("eyJ") && run.len() >= 12;
        if jwt || dense >= SECRET_RUN && !looks_like_host(run) {
            out.push_str(REDACTED);
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in text.chars() {
        if token_char(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// A dotted name whose labels are all short ("api2.cursor.sh/aiserver.v1.DashboardService").
fn looks_like_host(run: &str) -> bool {
    run.contains('.')
        && run
            .split(['.', '/'])
            .all(|label| label.len() < SECRET_RUN && !label.contains(['=', '%', '+']))
}

/// The first `max` characters of `text`, redacted, on one line.
pub fn excerpt(text: &str, max: usize) -> String {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect();
    redact(line.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JWT with the given payload; the header and signature are filler.
    pub(crate) fn jwt(payload: &str) -> String {
        format!(
            "eyJhbGciOiJIUzI1NiJ9.{}.c2lnbmF0dXJlLXNpZ25hdHVyZS1zaWduYXR1cmU",
            URL_SAFE_NO_PAD.encode(payload)
        )
    }

    #[test]
    fn stored_values_are_classified_before_use() {
        let token = jwt(r#"{"sub":"auth0|user_01ABC","exp":1800000000}"#);
        assert_eq!(classify(&token), StoredToken::Plain(&token));
        assert_eq!(
            classify("cursor-safe-storage:v1:djEwAAAA"),
            StoredToken::Sealed("djEwAAAA")
        );
        assert_eq!(
            classify("cursor-safe-storage:v2:whatever"),
            StoredToken::UnknownScheme("cursor-safe-storage:v2".into())
        );
        assert_eq!(
            classify("  opaque-token\n"),
            StoredToken::Plain("opaque-token")
        );
    }

    #[test]
    fn blob_values_decode_as_utf8_or_ascii_utf16() {
        assert_eq!(decode_value(b"tok").as_deref(), Some("tok"));
        assert_eq!(decode_value(b"t\0o\0k\0").as_deref(), Some("tok"));
        assert_eq!(decode_value(&[0xff, 0xfe, 0xfd]), None);
    }

    #[test]
    fn jwt_claims_user_id_and_expiry() {
        let token = jwt(r#"{"sub":"auth0|user_01ABC","exp":1800000000,"type":"session"}"#);
        let c = claims(&token).unwrap();
        assert_eq!(c.sub.as_deref(), Some("auth0|user_01ABC"));
        assert_eq!(c.exp, Some(1_800_000_000));
        assert_eq!(user_id("auth0|user_01ABC"), Some("user_01ABC"));
        assert_eq!(user_id("google-oauth2|user_9"), Some("user_9"));
        assert_eq!(user_id("user_plain"), Some("user_plain"));
        assert_eq!(user_id("auth0|bad;id"), None);
        assert_eq!(user_id("auth0|"), None);
        assert_eq!(claims("not-a-jwt"), None);
        assert_eq!(claims("a.b.c"), None);
        assert_eq!(claims(&jwt("{}")), Some(Claims::default()));
    }

    #[test]
    fn session_cookie_is_user_id_then_encoded_colons_then_token() {
        let token = jwt(r#"{"sub":"auth0|user_01ABC","exp":1800000000}"#);
        assert_eq!(
            session_cookie(&token),
            Some(format!("WorkosCursorSessionToken=user_01ABC%3A%3A{token}"))
        );
        assert_eq!(session_cookie(&jwt(r#"{"exp":1}"#)), None);
        assert_eq!(session_cookie(&jwt(r#"{"sub":"auth0|a b"}"#)), None);
        assert_eq!(session_cookie("opaque"), None);
    }

    #[test]
    fn token_id_is_eight_hex_and_never_the_token() {
        let id = token_id("secret-token");
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id, token_id("secret-token"));
        assert_ne!(id, token_id("secret-token2"));
    }

    #[test]
    fn web_fallback_only_for_a_live_token_or_a_broken_reply() {
        let now = 1_000;
        assert!(try_web_after(WebReason::Rejected, Some(2_000), now));
        assert!(try_web_after(WebReason::Rejected, None, now));
        assert!(!try_web_after(WebReason::Rejected, Some(999), now));
        assert!(try_web_after(WebReason::BadReply, Some(999), now));
        assert!(try_web_after(WebReason::ServerError, None, now));
    }

    #[test]
    fn redaction_removes_tokens_and_keeps_the_message() {
        let token = jwt(r#"{"sub":"auth0|user_01ABC","exp":1800000000}"#);
        let body = format!(r#"{{"code":"unauthenticated","message":"bad token {token}"}}"#);
        let clean = redact(&body);
        assert!(!clean.contains(&token));
        assert!(!clean.contains("eyJ"));
        assert!(clean.contains(r#""code":"unauthenticated""#));
        assert!(clean.contains("bad token [redacted]"));

        let cookie = format!("WorkosCursorSessionToken=user_01ABC%3A%3A{token}");
        assert_eq!(redact(&cookie), "[redacted]");
        assert_eq!(
            redact("Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789"),
            "Authorization: Bearer [redacted]"
        );
        let sealed = "cursor-safe-storage:v1:djEwQUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=";
        assert_eq!(redact(sealed), "cursor-safe-storage:v1:[redacted]");

        let plain = "POST api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage 401";
        assert_eq!(redact(plain), plain);
        assert_eq!(
            redact("resets 2026-11-11T22:16:13.000Z"),
            "resets 2026-11-11T22:16:13.000Z"
        );
    }

    #[test]
    fn excerpts_are_short_single_lines() {
        let long = "x ".repeat(300);
        assert_eq!(excerpt(&long, 200).chars().count(), 199);
        assert_eq!(
            excerpt("<html>\n<body>Blocked</body>", 200),
            "<html> <body>Blocked</body>"
        );
    }
}
