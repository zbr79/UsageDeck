//! Cursor usage for the signed-in account. Port of lib/cursor.ts and app/api/cursor/route.ts.
//!
//! The sign-in is read from Cursor's `state.vscdb`, opened READ-ONLY. A busy or locked DB is
//! `ProviderError::Locked`: the last good sign-in (or, after a restart, the newest remembered
//! account and its cached usage) is served instead, so the card never disappears.
//!
//! UsageDeck never refreshes Cursor's token: only Cursor does, when it runs. While the API rejects
//! the token, every poll re-reads the DB so the token Cursor renews is used at once; `open_cursor`
//! starts Cursor for the widget's "Open Cursor" button.

use super::cursor_signin::{exe_candidates, reuse_state};
use super::util::{number_or_null, string_or_null, to_iso};
use super::{
    collect_accounts, parse_reply, CachedProvider, FetchError, LiveAccount, ProviderError,
};
use crate::Core;
use rusqlite::{ErrorCode, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const API: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService";
const SECTION: &str = "cursor";
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
const STATE_TTL: Duration = Duration::from_secs(60);

/// `CursorUsage` in lib/types.ts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorUsage {
    pub auto_percent_used: Option<f64>,
    pub api_percent_used: Option<f64>,
    pub total_percent_used: Option<f64>,
    pub billing_cycle_end: Option<String>,
    pub total_spend: Option<f64>,
    pub limit: Option<f64>,
    pub bonus_spend: Option<f64>,
    pub account_name: Option<String>,
    pub plan_name: Option<String>,
}

/// GetCurrentPeriodUsage reply: `planUsage` must be an object; its fields stay lenient.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUsage {
    plan_usage: serde_json::Map<String, Value>,
    #[serde(default)]
    billing_cycle_end: Value,
}

/// "20" followed by a non-word character or the end (the `\$?\s*20(?:\.00)?\b` of the old code).
fn mentions_twenty(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(1)).any(|i| {
        b[i] == b'2'
            && b[i + 1] == b'0'
            && b.get(i + 2)
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
    })
}

fn cursor_plan_name(value: &Value) -> Option<String> {
    let plan = string_or_null(value)?;
    let lower = plan.to_lowercase();
    Some(if lower.contains("pro") || mentions_twenty(&lower) {
        "Pro".into()
    } else {
        plan
    })
}

/// Port of `parseCursorUsage`. The plan reply is optional (a failed plan call is `None`).
pub fn parse_cursor_usage(
    usage: &[u8],
    plan: Option<&Value>,
    account_name: Option<String>,
) -> Result<CursorUsage, ProviderError> {
    let raw: RawUsage = parse_reply(usage)?;
    let pu = |k: &str| number_or_null(raw.plan_usage.get(k).unwrap_or(&Value::Null));
    Ok(CursorUsage {
        auto_percent_used: pu("autoPercentUsed"),
        api_percent_used: pu("apiPercentUsed"),
        total_percent_used: pu("totalPercentUsed"),
        billing_cycle_end: to_iso(&raw.billing_cycle_end),
        total_spend: pu("totalSpend"),
        limit: pu("limit"),
        bonus_spend: pu("bonusSpend"),
        account_name,
        plan_name: plan.and_then(|p| cursor_plan_name(&p["planInfo"]["planName"])),
    })
}

// ---- state.vscdb ----------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CursorState {
    pub(crate) token: Option<String>,
    pub(crate) account_name: Option<String>,
}

#[derive(Clone)]
struct CursorAccount {
    id: String,
    label: Option<String>,
    token: String,
}

pub(crate) fn state_db() -> PathBuf {
    if !super::codex::real_sign_ins() {
        return PathBuf::from("no-real-sign-in-in-tests");
    }
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join("AppData").join("Roaming")
        });
    roaming
        .join("Cursor")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// Read the sign-in in one read-only open. Ok(None): Cursor not installed or not signed in.
/// Err(Locked): Cursor holds the DB right now; that says nothing about the sign-in.
pub fn read_state(db: &Path, busy: Duration) -> Result<Option<CursorState>, ProviderError> {
    if !db.exists() {
        return Ok(None);
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let read = || -> rusqlite::Result<CursorState> {
        let conn = rusqlite::Connection::open_with_flags(db, flags)?;
        conn.busy_timeout(busy)?;
        let get = |key: &str| -> rusqlite::Result<Option<String>> {
            let v: Option<rusqlite::types::Value> = conn
                .query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |r| {
                    r.get(0)
                })
                .optional()?;
            Ok(match v {
                Some(rusqlite::types::Value::Text(s)) => Some(s),
                Some(rusqlite::types::Value::Blob(b)) => String::from_utf8(b).ok(),
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
        Ok(CursorState {
            token,
            account_name,
        })
    };
    match read() {
        Ok(state) => Ok(Some(state)),
        Err(e) if is_busy(&e) => Err(ProviderError::Locked),
        Err(e) => {
            log::info!("cursor state DB unreadable: {e}");
            Ok(None)
        }
    }
}

/// When a DB path was last read, and what it held.
type StateCache = HashMap<PathBuf, (Instant, Option<CursorState>)>;

/// Last good read per DB path. A locked read never replaces it.
static STATE: LazyLock<Mutex<StateCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The access token the API last answered 401 for, until a fetch with a token works again.
static REJECTED: Mutex<Option<String>> = Mutex::new(None);

/// `current_state` on the blocking pool: the SQLite read can wait up to `busy` on a locked DB and
/// must not stall the async runtime.
async fn current_state_off_runtime(
    db: &Path,
    busy: Duration,
) -> Result<Option<CursorState>, ProviderError> {
    let db = db.to_path_buf();
    tokio::task::spawn_blocking(move || current_state(&db, busy))
        .await
        .unwrap_or_else(|e| {
            log::warn!("cursor state read did not finish: {e}");
            Err(ProviderError::Locked)
        })
}

fn current_state(db: &Path, busy: Duration) -> Result<Option<CursorState>, ProviderError> {
    if let Some((at, state)) = STATE.lock().unwrap().get(db) {
        let cached = state.as_ref().and_then(|s| s.token.as_deref());
        if reuse_state(
            at.elapsed(),
            STATE_TTL,
            cached,
            REJECTED.lock().unwrap().as_deref(),
        ) {
            return Ok(state.clone());
        }
    }
    match read_state(db, busy) {
        Ok(state) => {
            STATE
                .lock()
                .unwrap()
                .insert(db.to_path_buf(), (Instant::now(), state.clone()));
            Ok(state)
        }
        Err(e) => {
            // serve the last good sign-in, however old
            match STATE.lock().unwrap().get(db) {
                Some((_, state)) => Ok(state.clone()),
                None => Err(e),
            }
        }
    }
}

fn account_of(state: Option<CursorState>) -> Option<CursorAccount> {
    let state = state?;
    let token = state.token.filter(|t| !t.is_empty())?;
    let label = state
        .account_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Some(CursorAccount {
        id: label.as_deref().unwrap_or("signed-in").to_lowercase(),
        label,
        token,
    })
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
    http.post(format!("{API}/{method}"))
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("Connect-Protocol-Version", "1")
        .body("{}")
        .timeout(Duration::from_secs(10))
}

/// A failing reply: its status and the `exp` claim of the token it was sent with.
type Failure = (u16, Option<i64>);

/// The last failure per method, so a reply that keeps failing the same way is logged once.
static FAILING: LazyLock<Mutex<HashMap<&'static str, Failure>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One DashboardService call: the reply body, or the error its status maps to. A failing status
/// is logged with the endpoint (never the token).
async fn call(
    http: &reqwest::Client,
    token: &str,
    method: &'static str,
) -> Result<Vec<u8>, ProviderError> {
    let resp = dashboard_post(http, token, method)
        .send()
        .await
        .map_err(|e| {
            log::warn!("cursor {method}: request failed: {}", e.without_url());
            ProviderError::Network
        })?;
    super::note_retry_after(&resp);
    let status = resp.status().as_u16();
    let body = resp.bytes().await.map_err(|_| ProviderError::Network)?;
    if (200..300).contains(&status) {
        FAILING.lock().unwrap().remove(method);
        return Ok(body.to_vec());
    }
    let exp = super::codex::jwt_payload(token).and_then(|p| p["exp"].as_i64());
    if FAILING.lock().unwrap().insert(method, (status, exp)) != Some((status, exp)) {
        let expiry = if status == 401 {
            token_expiry(exp, chrono::Utc::now().timestamp())
        } else {
            String::new()
        };
        log::warn!(
            "cursor {API}/{method}: HTTP {status}{}{expiry}",
            connect_error(&body)
        );
    }
    Err(match status {
        401 => ProviderError::AuthRejected,
        429 => ProviderError::RateLimited,
        s => ProviderError::Http(s),
    })
}

/// The Connect error in a failed reply, e.g. ` (unauthenticated: ...)`.
fn connect_error(body: &[u8]) -> String {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return String::new();
    };
    let code = v["code"].as_str().unwrap_or("?");
    let message: String = v["message"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(160)
        .collect();
    format!(" ({code}: {message})")
}

/// Whether the access token Cursor stored had expired, from its `exp` claim: an expired token says
/// Cursor has not written a renewed one yet; a valid one says the server refused the request.
fn token_expiry(exp: Option<i64>, now: i64) -> String {
    match exp.and_then(|e| chrono::DateTime::from_timestamp(e, 0).map(|at| (e, at))) {
        Some((e, at)) if e <= now => format!("; the stored access token expired at {at}"),
        Some((_, at)) => format!("; the stored access token is valid until {at}"),
        None => "; the stored access token has no expiry".into(),
    }
}

async fn fetch_usage(
    http: &reqwest::Client,
    account: &CursorAccount,
) -> Result<CursorUsage, ProviderError> {
    let token = account.token.as_str();
    let (usage, plan) = tokio::join!(
        call(http, token, "GetCurrentPeriodUsage"),
        call(http, token, "GetPlanInfo")
    );
    let plan = plan
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    parse_cursor_usage(&usage?, plan.as_ref(), account.label.clone())
}

/// Rows for the `accounts` array of the old route's JSON body (`TrackedAccount<CursorUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    accounts_from(core, &state_db(), BUSY_TIMEOUT).await
}

async fn accounts_from(core: &Core, db: &Path, busy: Duration) -> Vec<Value> {
    let live = match current_state_off_runtime(db, busy).await {
        Ok(state) => match account_of(state) {
            Some(account) => {
                let key = account.id.clone();
                // Cursor renewed the rejected token: drop the cached 401 so this poll uses it.
                let mut rejected = REJECTED.lock().unwrap();
                if rejected.as_deref().is_some_and(|t| t != account.token) {
                    *rejected = None;
                    CACHE.invalidate(Some(&key));
                }
                drop(rejected);
                vec![LiveAccount {
                    id: account.id.clone(),
                    label: account.label.clone(),
                    active: true,
                    load: Box::pin(async move {
                        CACHE
                            .get(&core.cache, &key, |_| async move {
                                let result = fetch_usage(&core.http, &account).await;
                                match &result {
                                    Err(ProviderError::AuthRejected) => {
                                        *REJECTED.lock().unwrap() = Some(account.token.clone())
                                    }
                                    Ok(_) => *REJECTED.lock().unwrap() = None,
                                    Err(_) => {}
                                }
                                result.map_err(FetchError::from)
                            })
                            .await
                    }),
                    label_from: None,
                }]
            }
            None => Vec::new(),
        },
        // Locked with nothing read yet this run: keep the newest remembered account on the card,
        // with its cached usage.
        Err(error) => match core.seen.remembered(SECTION).pop() {
            Some(seen) => {
                let key = seen.id.clone();
                vec![LiveAccount {
                    id: seen.id,
                    label: seen.label,
                    active: true,
                    load: Box::pin(async move {
                        CACHE
                            .get(&core.cache, &key, |_| async move { Err(error.into()) })
                            .await
                    }),
                    label_from: None,
                }]
            }
            None => Vec::new(),
        },
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

    #[test]
    fn cursor_ok_fixture_parses() {
        let (usage, plan) = split(OK);
        let u = parse_cursor_usage(&usage, Some(&plan), Some("user@example.com".into())).unwrap();
        assert_eq!(u.auto_percent_used, Some(100.0));
        assert!((u.api_percent_used.unwrap() - 18.977777777777778).abs() < 1e-9);
        assert!((u.total_percent_used.unwrap() - 59.711111111111116).abs() < 1e-9);
        assert_eq!(u.total_spend, Some(5374.0));
        assert_eq!(u.limit, Some(2000.0));
        assert_eq!(u.bonus_spend, Some(3374.0));
        assert_eq!(
            u.billing_cycle_end.as_deref(),
            Some("2026-11-11T22:16:13.000Z")
        );
        assert_eq!(u.plan_name.as_deref(), Some("Pro"));
        assert_eq!(u.account_name.as_deref(), Some("user@example.com"));
        assert_eq!(
            cursor_plan_name(&Value::from("$20/mo")).as_deref(),
            Some("Pro")
        );
        assert_eq!(
            cursor_plan_name(&Value::from("Business")).as_deref(),
            Some("Business")
        );
        assert_eq!(
            cursor_plan_name(&Value::from("plan 200")).as_deref(),
            Some("plan 200")
        );
    }

    #[tokio::test]
    async fn cursor_invalid_fixture_is_schema_changed_and_keeps_last_good() {
        let (bad, _) = split(INVALID);
        assert_eq!(
            parse_cursor_usage(&bad, None, None),
            Err(ProviderError::SchemaChanged)
        );
        let dir = temp_dir("invalid");
        let cache = UsageCache::new(dir.join("usage-cache.json"));
        let p: CachedProvider<CursorUsage> =
            CachedProvider::new(SECTION, Duration::from_secs(60), None);
        let (usage, plan) = split(OK);
        let good = p
            .get(&cache, "k", |_| async {
                parse_cursor_usage(&usage, Some(&plan), None).map_err(FetchError::from)
            })
            .await;
        p.invalidate(None);
        let stale = p
            .get(&cache, "k", |_| async {
                parse_cursor_usage(&bad, None, None).map_err(FetchError::from)
            })
            .await;
        assert_eq!(stale.usage, good.usage);
        assert!(stale.stale);
        assert_eq!(stale.error, Some(ProviderError::SchemaChanged));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn make_db(path: &Path) {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
             INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', 'tok');
             INSERT INTO ItemTable VALUES ('cursorAuth/cachedEmail', 'User@Example.com');",
        )
        .unwrap();
    }

    fn lock(path: &Path) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        c
    }

    #[tokio::test]
    async fn cursor_locked_db_is_locked_and_keeps_the_card() {
        let dir = temp_dir("locked");
        let db = dir.join("state.vscdb");
        make_db(&db);
        let short = Duration::from_millis(50);

        // a readable DB gives the sign-in
        let state = read_state(&db, short).unwrap().unwrap();
        assert_eq!(state.token.as_deref(), Some("tok"));
        // missing DB: no account (not an error)
        assert_eq!(read_state(&dir.join("none.vscdb"), short), Ok(None));

        // an exclusive lock makes the read Locked, never "no account"
        let holder = lock(&db);
        assert_eq!(read_state(&db, short), Err(ProviderError::Locked));

        // fresh run, locked DB, nothing in memory: the remembered account keeps its cached usage
        let core = Core::new(Store::new(dir.clone(), Box::new(MemoryVault::default())));
        let cached = CursorUsage {
            total_percent_used: Some(42.0),
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
        let rows = accounts_from(&core, &db, short).await;
        assert_eq!(rows.len(), 1, "card must not disappear");
        assert_eq!(rows[0]["id"], "user@example.com");
        assert_eq!(rows[0]["active"], true);
        assert_eq!(rows[0]["stale"], true);
        assert_eq!(rows[0]["usage"]["totalPercentUsed"], 42.0);
        drop(holder);

        // a good read is remembered; a later locked read serves it instead of "no account"
        STATE.lock().unwrap().remove(&db);
        assert!(current_state(&db, short).unwrap().is_some());
        if let Some(entry) = STATE.lock().unwrap().get_mut(&db) {
            entry.0 = Instant::now() - STATE_TTL * 2;
        }
        let holder = lock(&db);
        let served = current_state(&db, short).unwrap();
        assert_eq!(account_of(served).unwrap().id, "user@example.com");
        drop(holder);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Live: reads Cursor's state.vscdb (read-only) and prints only the parsed summary.
    #[tokio::test]
    #[ignore]
    async fn cursor_live() {
        crate::providers::codex::REAL_SIGN_INS.store(true, std::sync::atomic::Ordering::Relaxed);
        let account = match current_state(&state_db(), BUSY_TIMEOUT).map(account_of) {
            Ok(Some(a)) => a,
            other => {
                println!("cursor live: no sign-in ({:?})", other.err());
                return;
            }
        };
        match fetch_usage(&reqwest::Client::new(), &account).await {
            Ok(u) => println!(
                "cursor live: plan={:?} total={:?}% auto={:?}% api={:?}% cycleEnd={:?}",
                u.plan_name,
                u.total_percent_used,
                u.auto_percent_used,
                u.api_percent_used,
                u.billing_cycle_end
            ),
            Err(e) => println!("cursor live: {e}"),
        }
    }
}
