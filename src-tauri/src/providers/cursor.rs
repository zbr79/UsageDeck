//! Cursor usage for the signed-in account. Port of lib/cursor.ts and app/api/cursor/route.ts.
//!
//! The sign-in is read from Cursor's `state.vscdb`, opened READ-ONLY. When the file is busy, a
//! private copy (with its -wal, -shm and -journal) is read and deleted, so a running Cursor never
//! hides its own sign-in. Cursor 3.27 and later store the token sealed with Electron safeStorage
//! (`cursor-safe-storage:v1:`); it is opened with the key in Cursor's `Local State`, the same way
//! Claude desktop's and Grok Bot's sign-ins are.
//!
//! The usage comes from the DashboardService with the bearer token. When that refuses a token that
//! has not expired, or answers without the usage, `https://cursor.com/api/usage-summary` is asked
//! with the session cookie made from the same token.
//!
//! When the usage shows API usage, the cycle's usage events
//! (`cursor.com/api/dashboard/get-filtered-usage-events`, same cookie) say what it cost and which
//! models it went to. They are read at most every five minutes and never fail the usage.
//!
//! UsageDeck never refreshes Cursor's token and never writes Cursor's files: only Cursor does.
//! After a failed fetch every poll re-reads the DB, so the token Cursor renews is used at once;
//! `open_cursor` starts Cursor for the widget's "Open Cursor" button. What each attempt did is
//! kept in `cursor_diag::CursorAttempt` for the diagnostics, without the token.

use super::cursor_auth::{
    claims, classify, decode_value, excerpt, session_cookie, token_id, try_web_after, StoredToken,
    WebReason,
};
use super::cursor_diag::{CursorAttempt, DbHow, DbRead, HttpAttempt, TokenInfo};
use super::cursor_signin::{exe_candidates, reuse_state};
pub use super::cursor_usage::CursorUsage;
use super::cursor_usage::{
    api_spend, cycle_of, events_complete, events_page, reply_detail, usage_from_dashboard,
    usage_from_summary, ApiSpend, Cycle, UsageEvent,
};
use super::{collect_accounts, CachedProvider, FetchError, LiveAccount, ProviderError};
use crate::win::os_crypt;
use crate::Core;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rusqlite::{ErrorCode, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const API_HOST: &str = "api2.cursor.sh";
const API_PATH: &str = "aiserver.v1.DashboardService";
const WEB_SUMMARY: &str = "cursor.com/api/usage-summary";
const WEB_EVENTS: &str = "cursor.com/api/dashboard/get-filtered-usage-events";
/// Events asked for per page; Cursor may send fewer, so `totalUsageEventsCount` decides the end.
const EVENTS_PAGE: usize = 500;
const EVENTS_MAX_PAGES: u32 = 20;
/// The events are read again at most this often: a month of them can take several pages.
const EVENTS_TTL: Duration = Duration::from_secs(300);
const SECTION: &str = "cursor";
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
const STATE_TTL: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---- state.vscdb ----------------------------------------------------------------------------

/// What `state.vscdb` holds about the sign-in, as stored (the token may be sealed).
#[derive(Clone, Debug, Default, PartialEq)]
struct RawState {
    token: Option<String>,
    account_name: Option<String>,
}

/// `%APPDATA%\Cursor`.
fn cursor_dir() -> PathBuf {
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join("AppData").join("Roaming")
        });
    roaming.join("Cursor")
}

/// Where the sign-in and the key that seals it live.
#[derive(Clone, Debug)]
struct Paths {
    db: PathBuf,
    local_state: PathBuf,
}

fn paths() -> Paths {
    if !super::codex::real_sign_ins() {
        let none = PathBuf::from("no-real-sign-in-in-tests");
        return Paths {
            db: none.clone(),
            local_state: none,
        };
    }
    let dir = cursor_dir();
    Paths {
        db: dir.join("User").join("globalStorage").join("state.vscdb"),
        local_state: dir.join("Local State"),
    }
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

fn query(db: &Path, flags: OpenFlags, busy: Duration) -> rusqlite::Result<RawState> {
    let conn = rusqlite::Connection::open_with_flags(db, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(busy)?;
    let get = |key: &str| -> rusqlite::Result<Option<String>> {
        let v: Option<rusqlite::types::Value> = conn
            .query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(match v {
            Some(rusqlite::types::Value::Text(s)) => Some(s),
            Some(rusqlite::types::Value::Blob(b)) => decode_value(&b),
            _ => None,
        })
    };
    let token = get("cursorAuth/accessToken")?;
    let mut account_name = get("cursorAuth/cachedEmail")?;
    if account_name.is_none() {
        account_name = get("cursorAuth/cachedScopedProfile")?
            .and_then(|p| serde_json::from_str::<Value>(&p).ok())
            .and_then(|p| {
                p.get("displayName")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
    }
    Ok(RawState {
        token,
        account_name,
    })
}

/// Read a private copy of the DB. The copy takes the -wal and -shm (a WAL database's newest
/// pages) and a -journal (so SQLite rolls a half-written transaction back in the copy), is opened
/// read-write because both recoveries write, and is deleted before this returns: it holds the
/// sign-in.
fn query_copy(db: &Path, busy: Duration) -> Result<RawState, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "usagedeck-cursor-state-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("no temporary folder: {e}"))?;
    let read = || -> Result<RawState, String> {
        let copy = dir.join("state.vscdb");
        std::fs::copy(db, &copy).map_err(|e| format!("copy failed: {e}"))?;
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut side = db.as_os_str().to_owned();
            side.push(suffix);
            let mut target = copy.as_os_str().to_owned();
            target.push(suffix);
            // a sidecar that is not there, or goes away mid-copy, is normal
            let _ = std::fs::copy(PathBuf::from(side), PathBuf::from(target));
        }
        query(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE, busy).map_err(|e| format!("copy: {e}"))
    };
    let result = read();
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        log::warn!("cursor: could not delete the temporary copy of state.vscdb: {e}");
    }
    result
}

/// Read the stored sign-in. None with `DbHow::Missing`: Cursor is not installed. None with
/// `Locked` or `Failed`: the file is there and could not be read, which says nothing about the
/// sign-in.
fn read_state(db: &Path, busy: Duration) -> (Option<RawState>, DbRead) {
    let read_at_ms = now_ms();
    let outcome = |how, detail| DbRead {
        how,
        detail,
        read_at_ms,
        reused_earlier: false,
    };
    if !db.exists() {
        return (None, outcome(DbHow::Missing, None));
    }
    let first = match query(db, OpenFlags::SQLITE_OPEN_READ_ONLY, busy) {
        Ok(state) => return (Some(state), outcome(DbHow::Opened, None)),
        Err(e) => e,
    };
    let why = excerpt(&first.to_string(), 120);
    match query_copy(db, busy) {
        Ok(state) => (Some(state), outcome(DbHow::Copied, Some(why))),
        Err(copy) => {
            let how = if is_busy(&first) {
                DbHow::Locked
            } else {
                DbHow::Failed
            };
            log::info!("cursor state DB unreadable: {why}; {copy}");
            let detail = format!("{why}; {}", excerpt(&copy, 120));
            (None, outcome(how, Some(detail)))
        }
    }
}

// ---- the sign-in ----------------------------------------------------------------------------

/// One read of Cursor's sign-in: how it went, and the token when there is a usable one.
#[derive(Clone, Debug)]
struct SignIn {
    db: DbRead,
    token: TokenInfo,
    /// the account's email or display name, as Cursor stored it
    label: Option<String>,
    /// Ok(None): signed out. Err: a sign-in may be there but could not be read.
    access: Result<Option<String>, ProviderError>,
}

/// Open a `cursor-safe-storage:v1:` value with the key in Cursor's `Local State`.
fn open_sealed(sealed_b64: &str, local_state: &Path) -> Result<String, String> {
    let sealed = B64
        .decode(sealed_b64.trim())
        .map_err(|_| "the stored value is not base64".to_string())?;
    let key = os_crypt::app_key_or_why(local_state).map_err(str::to_string)?;
    os_crypt::unseal_bytes(&key, &sealed)
}

fn sign_in(paths: &Paths, busy: Duration) -> SignIn {
    let (state, db) = read_state(&paths.db, busy);
    let mut token = TokenInfo::default();
    let Some(state) = state else {
        let access = match db.how {
            DbHow::Locked => Err(ProviderError::Locked),
            DbHow::Failed => Err(ProviderError::SignInUnreadable),
            _ => Ok(None),
        };
        return SignIn {
            db,
            token,
            label: None,
            access,
        };
    };
    let label = state
        .account_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let stored = state.token.unwrap_or_default();
    let access = if stored.trim().is_empty() {
        Ok(None)
    } else {
        token.present = true;
        let opened = match classify(&stored) {
            StoredToken::Plain(t) => Ok(t.to_string()),
            StoredToken::Sealed(sealed) => {
                token.form = Some("safe-storage-v1".into());
                let opened = open_sealed(sealed, &paths.local_state);
                token.decrypt = Some(opened.as_ref().map_or_else(Clone::clone, |_| "ok".into()));
                opened
            }
            StoredToken::UnknownScheme(scheme) => {
                token.decrypt = Some(format!("{scheme} is not a format this version knows"));
                token.form = Some(scheme);
                Err(String::new())
            }
        };
        match opened {
            Ok(access) if !access.trim().is_empty() => {
                let claims = claims(&access);
                let kind = if claims.is_some() { "jwt" } else { "opaque" };
                token.form.get_or_insert_with(|| kind.into());
                token.id = Some(token_id(&access));
                token.expires_at_ms = claims
                    .and_then(|c| c.exp)
                    .map(|secs| secs.saturating_mul(1000));
                Ok(Some(access))
            }
            Ok(_) => Ok(None),
            Err(_) => Err(ProviderError::SignInUnreadable),
        }
    };
    SignIn {
        db,
        token,
        label,
        access,
    }
}

/// Last usable read per DB path, and when.
static STATE: LazyLock<Mutex<HashMap<PathBuf, (Instant, SignIn)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The last fetch failed: read the DB again on the next poll instead of reusing `STATE`.
static FETCH_FAILED: AtomicBool = AtomicBool::new(false);

/// The hash of the token the last fetch used, to notice the moment Cursor renews it.
static LAST_TOKEN: Mutex<Option<String>> = Mutex::new(None);

fn current_sign_in(paths: &Paths, busy: Duration) -> SignIn {
    let failed = FETCH_FAILED.load(Ordering::Relaxed);
    if let Some((at, sign)) = STATE.lock().unwrap().get(&paths.db) {
        if reuse_state(at.elapsed(), STATE_TTL, failed) {
            return sign.clone();
        }
    }
    let fresh = sign_in(paths, busy);
    let mut state = STATE.lock().unwrap();
    match (&fresh.access, fresh.db.how) {
        (Ok(_), _) => {
            state.insert(paths.db.clone(), (Instant::now(), fresh.clone()));
            fresh
        }
        // The file could not be read at all: the sign-in of the last good read still stands, and
        // the record says so. A sign-in that was read but could not be opened is never replaced
        // by an older one.
        (Err(_), DbHow::Locked | DbHow::Failed) => match state.get(&paths.db) {
            Some((_, earlier)) if matches!(earlier.access, Ok(Some(_))) => {
                let mut earlier = earlier.clone();
                earlier.db = DbRead {
                    how: fresh.db.how,
                    detail: fresh.db.detail,
                    read_at_ms: earlier.db.read_at_ms,
                    reused_earlier: true,
                };
                earlier
            }
            _ => fresh,
        },
        (Err(_), _) => fresh,
    }
}

/// `current_sign_in` on the blocking pool: the SQLite read can wait up to `busy` on a locked DB
/// and must not stall the async runtime.
async fn current_sign_in_off_runtime(paths: &Paths, busy: Duration) -> SignIn {
    let paths = paths.clone();
    tokio::task::spawn_blocking(move || current_sign_in(&paths, busy))
        .await
        .unwrap_or_else(|e| {
            log::warn!("cursor state read did not finish: {e}");
            SignIn {
                db: DbRead {
                    how: DbHow::Locked,
                    detail: Some("the read did not finish".into()),
                    read_at_ms: now_ms(),
                    reused_earlier: false,
                },
                token: TokenInfo::default(),
                label: None,
                access: Err(ProviderError::Locked),
            }
        })
}

/// Cursor's current access token and account name, for Grok Bot's fallback. Blocking (SQLite).
pub(crate) fn bearer_token() -> Option<(String, Option<String>)> {
    let sign = sign_in(&paths(), Duration::ZERO);
    Some((sign.access.ok()??, sign.label))
}

// ---- diagnostics ----------------------------------------------------------------------------

static LAST_ATTEMPT: Mutex<Option<CursorAttempt>> = Mutex::new(None);

/// What the most recent attempt did (never the token).
pub fn last_attempt() -> Option<CursorAttempt> {
    LAST_ATTEMPT.lock().unwrap().clone()
}

/// What an attempt's requests added to the record.
#[derive(Default)]
struct Trace {
    requests: Vec<HttpAttempt>,
    auth_path: Option<&'static str>,
    parse: Option<String>,
}

/// Keep the attempt, counting failures in a row; a new kind of failure is logged once.
fn record(sign: &SignIn, trace: Trace, outcome: &str) {
    let mut last = LAST_ATTEMPT.lock().unwrap();
    let at_ms = now_ms();
    let ok = outcome == "ok";
    let failed = !ok && outcome != "not-signed-in";
    let mut attempt = CursorAttempt {
        at_ms,
        db: sign.db.clone(),
        token: sign.token.clone(),
        requests: trace.requests,
        auth_path: trace.auth_path,
        parse: trace.parse,
        outcome: outcome.to_string(),
        failures: 0,
        last_ok_ms: last.as_ref().and_then(|l| l.last_ok_ms),
    };
    if ok {
        attempt.last_ok_ms = Some(at_ms);
    } else if failed {
        attempt.failures = last.as_ref().map_or(0, |l| l.failures) + 1;
    }
    let reason = attempt.reason(&chrono::Utc);
    let changed = last
        .as_ref()
        .is_none_or(|l| l.outcome != attempt.outcome || l.auth_path != attempt.auth_path);
    if changed {
        match &reason {
            Some(reason) => log::warn!("cursor: {reason} (UTC)"),
            None => log::info!(
                "cursor: usage read through {}",
                attempt.auth_path.unwrap_or("?")
            ),
        }
    }
    *last = Some(attempt);
}

// ---- Open Cursor ----------------------------------------------------------------------------

/// Start Cursor so it renews its own sign-in (the widget's "Open Cursor" button). Cursor.exe is
/// looked for in its default per-user and per-machine folders, then in the uninstall entries.
pub fn open_cursor() -> Result<(), String> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let candidates = exe_candidates(
        env("LOCALAPPDATA").as_deref(),
        env("ProgramFiles").as_deref(),
        &crate::win::uninstall::uninstall_entries(),
    );
    let Some(exe) = candidates.into_iter().find(|p| p.is_file()) else {
        log::warn!("open cursor: Cursor.exe not found");
        return Err("Cursor is not installed".into());
    };
    std::process::Command::new(&exe).spawn().map_err(|e| {
        log::warn!("open cursor: could not start {}: {e}", exe.display());
        "Cursor could not be started".to_string()
    })?;
    log::info!("open cursor: started {}", exe.display());
    Ok(())
}

// ---- fetch ----------------------------------------------------------------------------------

static CACHE: LazyLock<CachedProvider<CursorUsage>> =
    LazyLock::new(|| CachedProvider::new(SECTION, Duration::from_secs(30), None));

/// A Connect unary call to Cursor's DashboardService, sent the way current trackers send it:
/// bearer token, JSON body, `Connect-Protocol-Version: 1`.
pub(crate) fn dashboard_post(
    http: &reqwest::Client,
    token: &str,
    method: &str,
) -> reqwest::RequestBuilder {
    http.post(format!("https://{API_HOST}/{API_PATH}/{method}"))
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("Connect-Protocol-Version", "1")
        .body("{}")
        .timeout(REQUEST_TIMEOUT)
}

/// The request behind Cursor's web dashboard, with the session cookie made from the access token.
fn summary_get(http: &reqwest::Client, cookie: &str) -> reqwest::RequestBuilder {
    http.get(format!("https://{WEB_SUMMARY}"))
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::COOKIE, cookie)
        .timeout(REQUEST_TIMEOUT)
}

/// One page of the usage events since the cycle started, the request behind Cursor's web Usage
/// page. Cursor checks the `Origin` of this POST.
fn events_post(
    http: &reqwest::Client,
    cookie: &str,
    start_ms: i64,
    page: u32,
) -> reqwest::RequestBuilder {
    let body = serde_json::json!({
        "startDate": start_ms.to_string(),
        "endDate": now_ms().to_string(),
        "page": page,
        "pageSize": EVENTS_PAGE,
    });
    http.post(format!("https://{WEB_EVENTS}"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ORIGIN, "https://cursor.com")
        .header(reqwest::header::COOKIE, cookie)
        .body(body.to_string())
        .timeout(REQUEST_TIMEOUT)
}

/// Send one request: the reply body or the error its status maps to, and what to record of it
/// (endpoint, status, and for a failure what the server said, redacted).
async fn send(
    request: reqwest::RequestBuilder,
    method: &'static str,
    endpoint: String,
) -> (Result<Vec<u8>, ProviderError>, HttpAttempt) {
    let mut attempt = HttpAttempt {
        method,
        endpoint,
        status: None,
        detail: None,
    };
    let resp = match request.send().await {
        Ok(resp) => resp,
        Err(e) => {
            let kind = if e.is_timeout() {
                "timed out"
            } else if e.is_connect() {
                "could not connect"
            } else {
                "request failed"
            };
            attempt.detail = Some(excerpt(&format!("{kind}: {}", e.without_url()), 200));
            return (Err(ProviderError::Network), attempt);
        }
    };
    super::note_retry_after(&resp);
    let status = resp.status().as_u16();
    attempt.status = Some(status);
    let body = match resp.bytes().await {
        Ok(body) => body,
        Err(e) => {
            attempt.status = None;
            attempt.detail = Some(excerpt(
                &format!(
                    "HTTP {status}, then the reply broke off: {}",
                    e.without_url()
                ),
                200,
            ));
            return (Err(ProviderError::Network), attempt);
        }
    };
    if (200..300).contains(&status) {
        return (Ok(body.to_vec()), attempt);
    }
    attempt.detail = reply_detail(&body);
    let error = match status {
        401 | 403 => ProviderError::AuthRejected,
        429 => ProviderError::RateLimited,
        s => ProviderError::Http(s),
    };
    (Err(error), attempt)
}

async fn call(
    http: &reqwest::Client,
    token: &str,
    method: &'static str,
) -> (Result<Vec<u8>, ProviderError>, HttpAttempt) {
    let endpoint = format!("{API_HOST}/{API_PATH}/{method}");
    send(dashboard_post(http, token, method), "POST", endpoint).await
}

/// Every usage event of this cycle, page by page; each page sent is added to the trace. Err when
/// they could not all be read: a partial sum would understate the spend.
async fn cycle_events(
    http: &reqwest::Client,
    cookie: &str,
    start_ms: i64,
    trace: &mut Trace,
) -> Result<Vec<UsageEvent>, ()> {
    let mut events = Vec::new();
    for page in 1..=EVENTS_MAX_PAGES {
        let (reply, mut attempt) = send(
            events_post(http, cookie, start_ms, page),
            "POST",
            WEB_EVENTS.into(),
        )
        .await;
        let Ok(body) = reply else {
            trace.requests.push(attempt);
            return Err(());
        };
        let read = match events_page(&body) {
            Ok(read) => read,
            Err(lacked) => {
                attempt.detail = Some(lacked);
                trace.requests.push(attempt);
                return Err(());
            }
        };
        let sent = read.events.len();
        events.extend(read.events);
        if events_complete(read.total, sent, events.len(), EVENTS_PAGE) {
            attempt.detail = Some(format!("{} events this cycle", events.len()));
            trace.requests.push(attempt);
            return Ok(events);
        }
        trace.requests.push(attempt);
    }
    if let Some(last) = trace.requests.last_mut() {
        last.detail = Some(format!("more than {EVENTS_MAX_PAGES} pages; not summed"));
    }
    Err(())
}

/// The last read of the events: for which token (its hash) and cycle start, when, and its spend.
type EventsRead = (String, i64, Instant, Option<ApiSpend>);
static EVENTS: Mutex<Option<EventsRead>> = Mutex::new(None);

/// The API bucket's spend this cycle and the models it went to, when the usage shows API usage.
/// Read at most every `EVENTS_TTL`; a failed read keeps the last spend of the same cycle. None
/// never fails the usage: the card then shows the percentages alone.
async fn api_spend_for(
    http: &reqwest::Client,
    token: &str,
    usage_body: &[u8],
    usage: &CursorUsage,
    trace: &mut Trace,
) -> Option<ApiSpend> {
    if !usage.other_models_percent_used.is_some_and(|p| p > 0.0) {
        return None;
    }
    let cycle: Cycle = cycle_of(usage_body)?;
    let who = token_id(token);
    let earlier = EVENTS
        .lock()
        .unwrap()
        .clone()
        .filter(|(id, start, _, _)| *id == who && *start == cycle.start_ms);
    if let Some((_, _, at, spend)) = &earlier {
        if at.elapsed() < EVENTS_TTL {
            return spend.clone();
        }
    }
    let cookie = session_cookie(token)?;
    let spend = match cycle_events(http, &cookie, cycle.start_ms, trace).await {
        Ok(events) => api_spend(&events, &cycle),
        Err(()) => earlier.and_then(|(_, _, _, spend)| spend),
    };
    *EVENTS.lock().unwrap() = Some((who, cycle.start_ms, Instant::now(), spend.clone()));
    spend
}

/// The usage for `token`: the DashboardService first, then cursor.com when that refused a token
/// that has not expired or answered without the usage. The error is the DashboardService's.
async fn fetch_usage(
    http: &reqwest::Client,
    token: &str,
    label: Option<String>,
) -> (Result<CursorUsage, ProviderError>, Trace) {
    let mut trace = Trace::default();
    let (usage, plan, hard_limit) = tokio::join!(
        call(http, token, "GetCurrentPeriodUsage"),
        call(http, token, "GetPlanInfo"),
        call(http, token, "GetHardLimit")
    );
    let json = |reply: Result<Vec<u8>, ProviderError>| {
        reply
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    };
    trace.requests.push(usage.1);
    trace.requests.push(plan.1);
    trace.requests.push(hard_limit.1);
    let (error, why) = match usage.0 {
        Ok(body) => {
            let (plan, hard_limit) = (json(plan.0), json(hard_limit.0));
            match usage_from_dashboard(&body, plan.as_ref(), hard_limit.as_ref(), label.clone()) {
                Ok(mut usage) => {
                    trace.auth_path = Some("api2");
                    trace.parse = Some("ok".into());
                    usage.api_spend = api_spend_for(http, token, &body, &usage, &mut trace).await;
                    return (Ok(usage), trace);
                }
                Err(lacked) => {
                    log::warn!("cursor GetCurrentPeriodUsage: {lacked}");
                    trace.parse = Some(lacked);
                    (ProviderError::SchemaChanged, WebReason::BadReply)
                }
            }
        }
        Err(ProviderError::AuthRejected) => (ProviderError::AuthRejected, WebReason::Rejected),
        Err(ProviderError::Http(status)) => (ProviderError::Http(status), WebReason::ServerError),
        Err(other) => return (Err(other), trace),
    };
    let exp = claims(token).and_then(|c| c.exp);
    if !try_web_after(why, exp, chrono::Utc::now().timestamp()) {
        return (Err(error), trace);
    }
    let Some(cookie) = session_cookie(token) else {
        return (Err(error), trace);
    };
    let (summary, attempt) = send(summary_get(http, &cookie), "GET", WEB_SUMMARY.into()).await;
    trace.requests.push(attempt);
    let Ok(body) = summary else {
        return (Err(error), trace);
    };
    match usage_from_summary(&body, label) {
        Ok(mut usage) => {
            trace.auth_path = Some("cursor.com");
            trace.parse = Some("ok".into());
            usage.api_spend = api_spend_for(http, token, &body, &usage, &mut trace).await;
            (Ok(usage), trace)
        }
        Err(lacked) => {
            log::warn!("cursor usage-summary: {lacked}");
            let first = trace
                .parse
                .take()
                .map(|p| format!("{p}; "))
                .unwrap_or_default();
            trace.parse = Some(format!("{first}cursor.com: {lacked}"));
            (Err(error), trace)
        }
    }
}

/// The `/api/cursor` body: the account rows (`TrackedAccount<CursorUsage>` each) and, when the
/// last attempt failed, the one-line reason for the card's tooltip.
pub async fn response(core: &Core) -> Value {
    let accounts = accounts_from(core, &paths(), BUSY_TIMEOUT).await;
    let attempt = last_attempt();
    serde_json::json!({
        "accounts": accounts,
        "attempt": attempt.map(|a| serde_json::json!({
            "reason": a.reason(&chrono::Local),
            "failures": a.failures,
        })),
    })
}

async fn accounts_from(core: &Core, paths: &Paths, busy: Duration) -> Vec<Value> {
    let sign = current_sign_in_off_runtime(paths, busy).await;
    let label = sign.label.clone();
    let id = label.as_deref().unwrap_or("signed-in").to_lowercase();
    let live = match sign.access.clone() {
        Ok(Some(token)) => {
            // Cursor wrote another token: use it on this poll, not after the failure memo.
            let mut last = LAST_TOKEN.lock().unwrap();
            if last.as_deref() != sign.token.id.as_deref() {
                if last.is_some() {
                    CACHE.invalidate(Some(&id));
                }
                last.clone_from(&sign.token.id);
            }
            drop(last);
            let key = id.clone();
            vec![LiveAccount {
                id,
                label: label.clone(),
                active: true,
                load: Box::pin(async move {
                    CACHE
                        .get(&core.cache, &key, |_| async move {
                            let (result, trace) = fetch_usage(&core.http, &token, label).await;
                            FETCH_FAILED.store(result.is_err(), Ordering::Relaxed);
                            let outcome = result.as_ref().map_or_else(|e| e.kind(), |_| "ok");
                            record(&sign, trace, outcome);
                            result.map_err(FetchError::from)
                        })
                        .await
                }),
                label_from: None,
            }]
        }
        Ok(None) => {
            record(&sign, Trace::default(), "not-signed-in");
            Vec::new()
        }
        // The sign-in could not be read. The card stays, under the account the DB names or the
        // newest remembered one, with its cached usage and this error.
        Err(error) => {
            let (id, label) = match (label, core.seen.remembered(SECTION).pop()) {
                (Some(label), _) => (id, Some(label)),
                (None, Some(seen)) => (seen.id, seen.label),
                (None, None) => (id, None),
            };
            let key = id.clone();
            vec![LiveAccount {
                id,
                label,
                active: true,
                load: Box::pin(async move {
                    CACHE
                        .get(&core.cache, &key, |_| async move {
                            FETCH_FAILED.store(true, Ordering::Relaxed);
                            record(&sign, Trace::default(), error.kind());
                            Err(error.into())
                        })
                        .await
                }),
                label_from: None,
            }]
        }
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
    use crate::providers::{SeenAccount, UsageCache};
    use crate::store::{MemoryVault, Store};

    const OK: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cursor/ok.json"
    ));
    const INVALID: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cursor/invalid.json"
    ));

    fn split(fixture: &[u8]) -> (Vec<u8>, Value) {
        let v: Value = serde_json::from_slice(fixture).unwrap();
        (serde_json::to_vec(&v["usage"]).unwrap(), v["plan"].clone())
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-cursor-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parse(usage: &[u8], plan: Option<&Value>) -> Result<CursorUsage, FetchError<CursorUsage>> {
        usage_from_dashboard(usage, plan, None, None)
            .map_err(|_| ProviderError::SchemaChanged.into())
    }

    #[tokio::test]
    async fn cursor_invalid_fixture_is_schema_changed_and_keeps_last_good() {
        let (bad, _) = split(INVALID);
        let dir = temp_dir("invalid");
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let p: CachedProvider<CursorUsage> =
            CachedProvider::new(SECTION, Duration::from_secs(60), None);
        let (usage, plan) = split(OK);
        let good = p
            .get(&cache, "k", |_| async { parse(&usage, Some(&plan)) })
            .await;
        p.invalidate(None);
        let stale = p.get(&cache, "k", |_| async { parse(&bad, None) }).await;
        assert_eq!(stale.usage, good.usage);
        assert!(stale.stale);
        assert!(
            !stale.auth_expired,
            "an unreadable reply is not a rejected sign-in"
        );
        assert_eq!(stale.error, Some(ProviderError::SchemaChanged));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn make_db(path: &Path, token: &str) {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
             INSERT INTO ItemTable VALUES ('cursorAuth/cachedEmail', 'User@Example.com');",
        )
        .unwrap();
        c.execute(
            "INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', ?1)",
            [token],
        )
        .unwrap();
    }

    fn lock(path: &Path) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        c
    }

    fn paths_in(dir: &Path) -> Paths {
        Paths {
            db: dir.join("state.vscdb"),
            local_state: dir.join("Local State"),
        }
    }

    const SHORT: Duration = Duration::from_millis(50);

    #[test]
    fn a_missing_db_is_not_signed_in_and_a_plain_token_is_used_as_stored() {
        let dir = temp_dir("plain");
        let paths = paths_in(&dir);
        let none = sign_in(&paths, SHORT);
        assert_eq!(none.db.how, DbHow::Missing);
        assert_eq!(none.access, Ok(None));

        make_db(&paths.db, "tok");
        let sign = sign_in(&paths, SHORT);
        assert_eq!(sign.db.how, DbHow::Opened);
        assert_eq!(sign.access, Ok(Some("tok".into())));
        assert_eq!(sign.label.as_deref(), Some("User@Example.com"));
        assert_eq!(sign.token.form.as_deref(), Some("opaque"));
        assert_eq!(sign.token.id, Some(token_id("tok")));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Cursor 3.27 and later: the stored value is sealed, and sending it as it is stored is a 401.
    #[test]
    fn a_sealed_token_is_opened_with_the_local_state_key() {
        let dir = temp_dir("sealed");
        let paths = paths_in(&dir);
        let key: os_crypt::AppKey = std::array::from_fn(|i| (i * 5 + 1) as u8);
        os_crypt::write_local_state(&paths.local_state, &key);
        let jwt =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhdXRoMHx1c2VyXzAxQUJDIiwiZXhwIjoxODAwMDAwMDAwfQ.c2ln";
        let stored = format!("cursor-safe-storage:v1:{}", os_crypt::seal(&key, jwt));
        make_db(&paths.db, &stored);

        let sign = sign_in(&paths, SHORT);
        assert_eq!(sign.access, Ok(Some(jwt.to_string())));
        assert_eq!(sign.token.form.as_deref(), Some("safe-storage-v1"));
        assert_eq!(sign.token.decrypt.as_deref(), Some("ok"));
        assert_eq!(sign.token.expires_at_ms, Some(1_800_000_000_000));
        assert_eq!(sign.token.id, Some(token_id(jwt)));

        // no key: the card says the sign-in could not be read, never "signed out"
        let other: os_crypt::AppKey = [9; 32];
        let sealed_elsewhere = format!("cursor-safe-storage:v1:{}", os_crypt::seal(&other, jwt));
        let dir2 = temp_dir("sealed-nokey");
        let paths2 = paths_in(&dir2);
        make_db(&paths2.db, &sealed_elsewhere);
        let unreadable = sign_in(&paths2, SHORT);
        assert_eq!(unreadable.access, Err(ProviderError::SignInUnreadable));
        assert_eq!(
            unreadable.token.decrypt.as_deref(),
            Some("Local State not found")
        );
        assert_eq!(unreadable.token.id, None);
        assert_eq!(unreadable.label.as_deref(), Some("User@Example.com"));

        // a key that does not match
        os_crypt::write_local_state(&paths2.local_state, &key);
        let wrong = sign_in(&paths2, SHORT);
        assert_eq!(wrong.access, Err(ProviderError::SignInUnreadable));
        assert!(wrong.token.decrypt.unwrap().starts_with("AES-GCM rejected"));

        // a scheme newer than this build
        let dir3 = temp_dir("sealed-v2");
        let paths3 = paths_in(&dir3);
        make_db(&paths3.db, "cursor-safe-storage:v2:AAAA");
        let unknown = sign_in(&paths3, SHORT);
        assert_eq!(unknown.access, Err(ProviderError::SignInUnreadable));
        assert_eq!(
            unknown.token.form.as_deref(),
            Some("cursor-safe-storage:v2")
        );
        for d in [dir, dir2, dir3] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[tokio::test]
    async fn a_locked_db_is_read_from_a_copy_and_an_unreadable_one_keeps_the_card() {
        let dir = temp_dir("locked");
        let paths = paths_in(&dir);
        make_db(&paths.db, "tok");

        // Cursor holds the file: the sign-in still comes out, through a private copy
        let holder = lock(&paths.db);
        let (state, read) = read_state(&paths.db, SHORT);
        assert_eq!(read.how, DbHow::Copied);
        assert!(read.detail.is_some());
        assert_eq!(state.unwrap().token.as_deref(), Some("tok"));
        drop(holder);
        let leftovers = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with(&format!("usagedeck-cursor-state-{}-", std::process::id()))
            })
            .count();
        assert_eq!(
            leftovers, 0,
            "the copy holds the sign-in and must be deleted"
        );

        // a file SQLite cannot read at all, nothing in memory: the remembered account keeps its
        // card and cached usage, with the error
        std::fs::write(
            &paths.db,
            b"this is not a database, it is long enough to be read as one",
        )
        .unwrap();
        let core = Core::new(Store::new(dir.clone(), Box::new(MemoryVault::default())));
        let cached = CursorUsage {
            cursor_models_percent_used: Some(42.0),
            ..Default::default()
        };
        core.cache.write(SECTION, "user@example.com", &cached).await;
        core.seen
            .remember(
                SECTION,
                &[SeenAccount {
                    id: "user@example.com".into(),
                    label: Some("User@Example.com".into()),
                }],
            )
            .await;
        STATE.lock().unwrap().remove(&paths.db);
        let rows = accounts_from(&core, &paths, SHORT).await;
        assert_eq!(rows.len(), 1, "card must not disappear");
        assert_eq!(rows[0]["id"], "user@example.com");
        assert_eq!(rows[0]["active"], true);
        assert_eq!(rows[0]["stale"], true);
        assert_eq!(rows[0]["authExpired"], false);
        assert_eq!(rows[0]["error"], "sign-in-unreadable");
        assert_eq!(rows[0]["usage"]["cursorModelsPercentUsed"], 42.0);
        let attempt = last_attempt().unwrap();
        assert_eq!(attempt.outcome, "sign-in-unreadable");
        assert_eq!(attempt.db.how, DbHow::Failed);
        assert!(attempt.requests.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_read_reuses_the_earlier_sign_in_and_says_so() {
        let dir = temp_dir("reuse");
        let paths = paths_in(&dir);
        make_db(&paths.db, "tok");
        FETCH_FAILED.store(false, Ordering::Relaxed);
        let first = current_sign_in(&paths, SHORT);
        assert!(!first.db.reused_earlier);
        if let Some(entry) = STATE.lock().unwrap().get_mut(&paths.db) {
            entry.0 = Instant::now() - STATE_TTL * 2;
        }
        std::fs::write(
            &paths.db,
            b"this is not a database, it is long enough to be read as one",
        )
        .unwrap();
        let served = current_sign_in(&paths, SHORT);
        assert_eq!(served.access, Ok(Some("tok".into())));
        assert!(served.db.reused_earlier);
        assert_eq!(served.db.how, DbHow::Failed);
        assert_eq!(served.db.read_at_ms, first.db.read_at_ms);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Live: reads Cursor's state.vscdb (read-only) and prints only the parsed summary.
    #[tokio::test]
    #[ignore]
    async fn cursor_live() {
        crate::providers::codex::REAL_SIGN_INS.store(true, std::sync::atomic::Ordering::Relaxed);
        let sign = sign_in(&paths(), BUSY_TIMEOUT);
        let Ok(Some(token)) = sign.access.clone() else {
            println!("cursor live: no sign-in ({:?}, {:?})", sign.db, sign.token);
            return;
        };
        let (result, trace) =
            fetch_usage(&reqwest::Client::new(), &token, sign.label.clone()).await;
        record(
            &sign,
            trace,
            result.as_ref().map_or_else(|e| e.kind(), |_| "ok"),
        );
        println!("{}", last_attempt().unwrap().report(&chrono::Local));
        match result {
            Ok(u) => println!(
                "cursor live: plan={:?} cursorModels={:?}% otherModels={:?}% resetsAt={:?} onDemand={:?}",
                u.plan_name,
                u.cursor_models_percent_used,
                u.other_models_percent_used,
                u.resets_at,
                u.on_demand
            ),
            Err(e) => println!("cursor live: {e}"),
        }
    }
}
