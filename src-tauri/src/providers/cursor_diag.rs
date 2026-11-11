//! What the last attempt to read Cursor's usage did, step by step, so a failure can be read off
//! the screen: the Settings → Cursor "Diagnostics" rows, the copied report and the card's tooltip
//! are all made from this record.
//!
//! It never holds a token: only a hash prefix, the expiry, and replies with token-like text
//! redacted (`cursor_auth::redact`).
//!
//! No Windows or app dependencies, so the unit tests also run on Linux.

use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use std::fmt::Display;

/// How the read of `state.vscdb` went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DbHow {
    /// no file: Cursor is not installed for this Windows user
    #[default]
    Missing,
    /// opened read-only in place
    Opened,
    /// the file was busy, so a private copy (with its -wal and -shm) was read and deleted
    Copied,
    /// busy, and the copy could not be read either
    Locked,
    /// SQLite could not read it for another reason
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbRead {
    pub how: DbHow,
    /// the SQLite error, when one occurred (also set for `Copied`: why the copy was needed)
    pub detail: Option<String>,
    pub read_at_ms: i64,
    /// this read failed and the sign-in of an earlier read (at `read_at_ms`) was used
    pub reused_earlier: bool,
}

/// The stored sign-in, described without the token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfo {
    pub present: bool,
    /// "jwt", "opaque" (not a JWT), "safe-storage-v1" (encrypted) or the unknown scheme's name
    pub form: Option<String>,
    /// for an encrypted token: "ok", or why it could not be decrypted
    pub decrypt: Option<String>,
    /// first 8 hex characters of the token's SHA-256
    pub id: Option<String>,
    pub expires_at_ms: Option<i64>,
}

impl TokenInfo {
    pub fn expired(&self, now_ms: i64) -> Option<bool> {
        self.expires_at_ms.map(|at| at <= now_ms)
    }
}

/// One HTTP request of the attempt.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpAttempt {
    pub method: &'static str,
    /// host and path, never a query or a header
    pub endpoint: String,
    /// None: no reply (see `detail`)
    pub status: Option<u16>,
    /// the transport error, or for a non-2xx reply its Connect error or the start of its body
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorAttempt {
    pub at_ms: i64,
    pub db: DbRead,
    pub token: TokenInfo,
    pub requests: Vec<HttpAttempt>,
    /// "api2" (bearer token) or "cursor.com" (session cookie): whose reply gave the usage
    pub auth_path: Option<&'static str>,
    /// "ok", or what the reply lacked
    pub parse: Option<String>,
    /// "ok", "not-signed-in", or the error kind shown on the card
    pub outcome: String,
    /// failed attempts in a row, this one included
    pub failures: u32,
    pub last_ok_ms: Option<i64>,
}

fn at<Tz: TimeZone>(tz: &Tz, ms: i64) -> Option<DateTime<Tz>> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|t| t.with_timezone(tz))
}

fn clock<Tz: TimeZone>(tz: &Tz, ms: i64) -> String
where
    Tz::Offset: Display,
{
    at(tz, ms).map_or_else(|| "?".into(), |t| t.format("%H:%M").to_string())
}

fn day<Tz: TimeZone>(tz: &Tz, ms: i64) -> String
where
    Tz::Offset: Display,
{
    at(tz, ms).map_or_else(|| "?".into(), |t| t.format("%b %-d").to_string())
}

fn stamp<Tz: TimeZone>(tz: &Tz, ms: i64) -> String
where
    Tz::Offset: Display,
{
    at(tz, ms).map_or_else(
        || "?".into(),
        |t| t.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
    )
}

impl CursorAttempt {
    fn token_note<Tz: TimeZone>(&self, tz: &Tz) -> String
    where
        Tz::Offset: Display,
    {
        match (self.token.expires_at_ms, self.token.expired(self.at_ms)) {
            (Some(exp), Some(true)) => format!("token expired {}", day(tz, exp)),
            (Some(exp), _) => format!("token valid until {}", day(tz, exp)),
            (None, _) => "token has no expiry".into(),
        }
    }

    /// The request that decided the outcome: the last one that failed.
    fn failed_request(&self) -> Option<&HttpAttempt> {
        self.requests
            .iter()
            .rev()
            .find(|r| !r.status.is_some_and(|s| (200..300).contains(&s)))
    }

    /// One line saying why the card has no fresh usage, for its tooltip. None when it has.
    pub fn reason<Tz: TimeZone>(&self, tz: &Tz) -> Option<String>
    where
        Tz::Offset: Display,
    {
        let when = clock(tz, self.at_ms);
        let line = match self.outcome.as_str() {
            "ok" => return None,
            "not-signed-in" => match self.db.how {
                DbHow::Missing => "Cursor's sign-in file was not found on this PC".to_string(),
                _ => "Cursor is signed out (its sign-in file holds no token)".to_string(),
            },
            "locked" => format!("Cursor's sign-in file was locked at {when}; trying again"),
            "sign-in-unreadable" => {
                let why = self
                    .token
                    .decrypt
                    .as_deref()
                    .filter(|d| *d != "ok")
                    .or(self.db.detail.as_deref())
                    .unwrap_or("unknown format");
                format!("Cursor's sign-in could not be read at {when}: {why}")
            }
            kind => {
                let request = self.failed_request();
                let host = request
                    .map(|r| r.endpoint.split('/').next().unwrap_or("Cursor"))
                    .unwrap_or("Cursor");
                match (kind, request.and_then(|r| r.status)) {
                    ("auth-rejected", Some(status))
                        if self.token.expired(self.at_ms) == Some(true) =>
                    {
                        format!(
                            "{host} replied {status} at {when}; {}. Open Cursor to renew it",
                            self.token_note(tz)
                        )
                    }
                    (_, Some(status)) => {
                        let code = request
                            .and_then(|r| r.detail.as_deref())
                            .and_then(|d| d.split(':').next())
                            .filter(|c| !c.is_empty() && c.len() <= 24 && !c.contains(' '))
                            .map(|c| format!(" {c}"))
                            .unwrap_or_default();
                        format!(
                            "{host} replied {status}{code} at {when}; {}",
                            self.token_note(tz)
                        )
                    }
                    ("schema-changed", None) => format!(
                        "Cursor's reply at {when} had no usage in it ({})",
                        self.parse.as_deref().unwrap_or("unknown shape")
                    ),
                    (_, None) => {
                        let why = request
                            .and_then(|r| r.detail.as_deref())
                            .unwrap_or("no reply");
                        format!("{host} could not be reached at {when} ({why})")
                    }
                }
            }
        };
        Some(line)
    }

    /// The Cursor part of the copied report.
    pub fn report<Tz: TimeZone>(&self, tz: &Tz) -> String
    where
        Tz::Offset: Display,
    {
        let mut out = Vec::new();
        let last_ok = self
            .last_ok_ms
            .map_or_else(|| "none since UsageDeck started".into(), |ms| stamp(tz, ms));
        out.push(format!(
            "Last attempt: {} -> {}",
            stamp(tz, self.at_ms),
            self.outcome
        ));
        out.push(format!(
            "Failures in a row: {}; last success: {last_ok}",
            self.failures
        ));
        let how = match self.db.how {
            DbHow::Missing => "not found",
            DbHow::Opened => "found; opened read-only",
            DbHow::Copied => "found; busy, read from a private copy",
            DbHow::Locked => "found; locked",
            DbHow::Failed => "found; could not be read",
        };
        let mut db = format!("state.vscdb: {how}");
        if let Some(detail) = &self.db.detail {
            db += &format!(" ({detail})");
        }
        if self.db.reused_earlier {
            db += &format!(
                "; using the sign-in read at {}",
                clock(tz, self.db.read_at_ms)
            );
        }
        out.push(db);
        let mut token = match (&self.token.present, &self.token.form) {
            (false, _) => "Token: none stored".to_string(),
            (true, form) => format!("Token: stored as {}", form.as_deref().unwrap_or("?")),
        };
        if let Some(decrypt) = &self.token.decrypt {
            token += &format!("; decrypt: {decrypt}");
        }
        if let Some(id) = &self.token.id {
            token += &format!("; sha256 {id}; {}", self.token_note(tz));
            if let Some(exp) = self.token.expires_at_ms {
                token += &format!(" ({})", stamp(tz, exp));
            }
        }
        out.push(token);
        if self.requests.is_empty() {
            out.push("Requests: none sent".into());
        }
        for r in &self.requests {
            let status = r
                .status
                .map_or_else(|| "no reply".into(), |s| s.to_string());
            let detail = r
                .detail
                .as_deref()
                .map(|d| format!(" ({d})"))
                .unwrap_or_default();
            out.push(format!(
                "Request: {} {} -> {status}{detail}",
                r.method, r.endpoint
            ));
        }
        out.push(format!(
            "Usage from: {}; parse: {}",
            self.auth_path.unwrap_or("none"),
            self.parse.as_deref().unwrap_or("not reached")
        ));
        if let Some(reason) = self.reason(tz) {
            out.push(format!("Reason: {reason}"));
        }
        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-11 14:02:00 UTC
    const AT: i64 = 1_794_405_720_000;
    /// 2026-12-03 10:00:00 UTC
    const DEC_3: i64 = 1_796_292_000_000;

    fn attempt(outcome: &str) -> CursorAttempt {
        CursorAttempt {
            at_ms: AT,
            db: DbRead {
                how: DbHow::Opened,
                read_at_ms: AT,
                ..Default::default()
            },
            token: TokenInfo {
                present: true,
                form: Some("jwt".into()),
                decrypt: None,
                id: Some("1a2b3c4d".into()),
                expires_at_ms: Some(DEC_3),
            },
            outcome: outcome.into(),
            failures: 3,
            ..Default::default()
        }
    }

    fn request(status: Option<u16>, detail: Option<&str>) -> HttpAttempt {
        HttpAttempt {
            method: "POST",
            endpoint: "api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage".into(),
            status,
            detail: detail.map(Into::into),
        }
    }

    #[test]
    fn a_rejected_live_token_names_the_status_time_and_expiry() {
        let mut a = attempt("auth-rejected");
        a.requests = vec![request(Some(401), Some("unauthenticated: Not logged in"))];
        assert_eq!(
            a.reason(&Utc).as_deref(),
            Some("api2.cursor.sh replied 401 unauthenticated at 14:02; token valid until Dec 3")
        );
    }

    #[test]
    fn an_expired_token_says_to_open_cursor() {
        let mut a = attempt("auth-rejected");
        a.token.expires_at_ms = Some(AT - 86_400_000);
        a.requests = vec![request(Some(401), None)];
        assert_eq!(
            a.reason(&Utc).as_deref(),
            Some("api2.cursor.sh replied 401 at 14:02; token expired Nov 10. Open Cursor to renew it")
        );
    }

    #[test]
    fn every_other_failure_has_its_own_line() {
        assert_eq!(attempt("ok").reason(&Utc), None);

        let mut locked = attempt("locked");
        locked.db.how = DbHow::Locked;
        assert_eq!(
            locked.reason(&Utc).as_deref(),
            Some("Cursor's sign-in file was locked at 14:02; trying again")
        );

        let mut sealed = attempt("sign-in-unreadable");
        sealed.token.decrypt = Some("Cursor's Local State has no key".into());
        assert_eq!(
            sealed.reason(&Utc).as_deref(),
            Some("Cursor's sign-in could not be read at 14:02: Cursor's Local State has no key")
        );

        let mut offline = attempt("network");
        offline.requests = vec![request(None, Some("connection error"))];
        assert_eq!(
            offline.reason(&Utc).as_deref(),
            Some("api2.cursor.sh could not be reached at 14:02 (connection error)")
        );

        let mut shape = attempt("schema-changed");
        shape.requests = vec![request(Some(200), None)];
        shape.parse = Some("no planUsage; keys: billingCycleEnd".into());
        assert_eq!(
            shape.reason(&Utc).as_deref(),
            Some(
                "Cursor's reply at 14:02 had no usage in it (no planUsage; keys: billingCycleEnd)"
            )
        );

        let mut down = attempt("http");
        down.requests = vec![request(Some(503), Some("<html>Bad gateway</html>"))];
        assert_eq!(
            down.reason(&Utc).as_deref(),
            Some("api2.cursor.sh replied 503 at 14:02; token valid until Dec 3")
        );

        let mut none = attempt("not-signed-in");
        none.db.how = DbHow::Missing;
        assert_eq!(
            none.reason(&Utc).as_deref(),
            Some("Cursor's sign-in file was not found on this PC")
        );
    }

    #[test]
    fn the_report_lists_every_step_and_no_token() {
        let mut a = attempt("ok");
        a.failures = 0;
        a.last_ok_ms = Some(AT);
        a.db = DbRead {
            how: DbHow::Copied,
            detail: Some("database is locked".into()),
            read_at_ms: AT,
            reused_earlier: false,
        };
        a.token.form = Some("safe-storage-v1".into());
        a.token.decrypt = Some("ok".into());
        a.requests = vec![
            request(Some(401), Some("unauthenticated: Not logged in")),
            HttpAttempt {
                method: "GET",
                endpoint: "cursor.com/api/usage-summary".into(),
                status: Some(200),
                detail: None,
            },
        ];
        a.auth_path = Some("cursor.com");
        a.parse = Some("ok".into());
        assert_eq!(
            a.report(&Utc),
            "Last attempt: 2026-11-11 14:02:00 +00:00 -> ok\n\
             Failures in a row: 0; last success: 2026-11-11 14:02:00 +00:00\n\
             state.vscdb: found; busy, read from a private copy (database is locked)\n\
             Token: stored as safe-storage-v1; decrypt: ok; sha256 1a2b3c4d; token valid until Dec 3 (2026-12-03 10:00:00 +00:00)\n\
             Request: POST api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage -> 401 (unauthenticated: Not logged in)\n\
             Request: GET cursor.com/api/usage-summary -> 200\n\
             Usage from: cursor.com; parse: ok"
        );
    }

    #[test]
    fn serialises_for_the_settings_rows() {
        let v = serde_json::to_value(attempt("locked")).unwrap();
        assert_eq!(v["db"]["how"], "opened");
        assert_eq!(v["token"]["id"], "1a2b3c4d");
        assert_eq!(v["token"]["expiresAtMs"], DEC_3);
        assert_eq!(v["failures"], 3);
        assert_eq!(v["authPath"], serde_json::Value::Null);
    }
}
