//! Headless command-line modes, handled before any window exists:
//!
//! - `--import-old-data <dir>`: one-time copy of the old app's `data/` folder.
//! - `--dump`: poll every provider once and print a redacted JSON summary.
//! - `--pace [<data dir>]`: scan the Claude Code transcripts once and print the
//!   `/api/claude/pace` JSON (scan timings on stderr). The optional folder replaces the app data
//!   folder, so a diagnostic run need not touch the real index files.
//! - `--fullscreen-probe`: print one full-screen reading (notification state, foreground window
//!   class, whether it covers its monitor, verdict). Never prints a window title.
//!
//! No mode ever prints an API key or a full account label.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use usagedeck_core::store::{
    apply_settings_patch, read_json_untouched, write_json_atomic, Account, CredentialVault, Store,
    ACCOUNTS_FILE, SEEN_ACCOUNTS_FILE, USAGE_CACHE_FILE,
};
use usagedeck_core::{claude_logs, default_data_dir, providers, Core};

const ENC_PREFIX: &str = "enc:v1:";

/// Handle a headless mode if the process arguments ask for one. Returns the exit code, or `None`
/// to launch the normal taskbar UI.
pub fn try_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    try_run_args(&args)
}

/// Handle a headless mode for an explicit argument slice (excluding argv\[0\]).
pub fn try_run_args(args: &[String]) -> Option<i32> {
    let mode = args.first()?.as_str();
    if mode != "--import-old-data"
        && mode != "--dump"
        && mode != "--pace"
        && mode != "--fullscreen-probe"
    {
        return None;
    }
    attach_console();
    if mode == "--fullscreen-probe" {
        return Some(fullscreen_probe());
    }
    let custom_dir = (mode == "--pace")
        .then(|| args.get(1).map(PathBuf::from))
        .flatten();
    let Some(dir) = custom_dir.or_else(default_data_dir) else {
        eprintln!("APPDATA is not set");
        return Some(2);
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("could not start the runtime: {e}");
            return Some(2);
        }
    };
    let store = Store::new(dir, Box::new(CredentialVault));
    Some(match mode {
        "--import-old-data" => match args.get(1) {
            Some(src) => runtime.block_on(import_old_data(&store, Path::new(src))),
            None => {
                eprintln!("usage: usagedeck --import-old-data <old data dir>");
                2
            }
        },
        "--pace" => runtime.block_on(pace(Core::new(store))),
        _ => runtime.block_on(dump(Core::new(store))),
    })
}

/// True when the command line asks for the settings dialog: `usagedeck.exe --settings`.
pub fn wants_settings<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|a| a.as_ref() == "--settings")
}

/// True when the command line asks for preview demo mode: `usagedeck.exe --demo`.
pub fn wants_demo<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|a| a.as_ref() == "--demo")
}

/// Release builds use the GUI subsystem: without an inherited stdout (a pipe or a file), attach
/// to the parent console so CLI output is visible when run from cmd or PowerShell.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{
            AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
        };
        let has_stdout =
            GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| !h.is_invalid() && !h.0.is_null());
        if !has_stdout {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

// ---- --fullscreen-probe -----------------------------------------------------------------------

/// Print one full-screen reading for the current foreground window and exit. There is no widget
/// in this mode, so the foreground window is compared with its own monitor. Never prints a
/// window title.
pub fn fullscreen_probe() -> i32 {
    use usagedeck_core::win::fullscreen::{quns_name, read};
    // Same DPI mode as the app, so window and monitor rects are both in physical pixels.
    // SAFETY: called once, before any window exists in this process.
    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let r = read(None);
    match r.quns {
        Some(q) => println!("quns: {} ({q})", quns_name(q)),
        None => println!("quns: unavailable"),
    }
    println!("foreground class: {:?}", r.class);
    println!("ignored: {}", r.ignored);
    println!("covers its monitor: {}", r.covers);
    println!(
        "verdict: {}",
        if r.full_screen {
            "full screen"
        } else {
            "not full screen"
        }
    );
    0
}

// ---- --pace -----------------------------------------------------------------------------------

async fn pace(core: Core) -> i32 {
    let engine = core.spend.clone();
    let now = claude_logs::now_ms();
    let Ok(stats) = tokio::task::spawn_blocking(move || engine.scan_once(now)).await else {
        eprintln!("scan failed");
        return 1;
    };
    let engine_pace = core.spend.clone();
    let dir = core.store.dir().to_path_buf();
    let pace_now = claude_logs::now_ms();
    let Ok(body) =
        tokio::task::spawn_blocking(move || claude_logs::pace_body(&engine_pace, &dir, pace_now))
            .await
    else {
        eprintln!("pace failed");
        return 1;
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    let diag = json!({ "scan": stats, "last24h": core.spend.spend(now).last24h });
    eprintln!("{diag}");
    0
}

// ---- enc:v1 keys (lib/key-crypto.ts) ----------------------------------------------------------

fn aes_key(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

fn b64url(part: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).ok()
}

/// `decryptKey` of `lib/key-crypto.ts`: plaintext passes through; `enc:v1:<iv>:<tag>:<data>`
/// (base64url parts) is AES-256-GCM with key = SHA-256(secret).
pub fn decrypt_key(stored: &str, secret: &str) -> Result<String, String> {
    let Some(rest) = stored.strip_prefix(ENC_PREFIX) else {
        return Ok(stored.to_string());
    };
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 3 {
        return Err("stored API key is not valid encrypted data".into());
    }
    let (Some(iv), Some(tag), Some(data)) = (b64url(parts[0]), b64url(parts[1]), b64url(parts[2]))
    else {
        return Err("stored API key is not valid encrypted data".into());
    };
    if iv.len() != 12 || tag.len() != 16 || data.is_empty() {
        return Err("stored API key is not valid encrypted data".into());
    }
    let cipher = Aes256Gcm::new(&aes_key(secret).into());
    let mut sealed = data;
    sealed.extend_from_slice(&tag);
    let plain = cipher
        .decrypt(Nonce::from_slice(&iv), sealed.as_slice())
        .map_err(|_| "decryption failed (wrong secret or damaged data)".to_string())?;
    String::from_utf8(plain).map_err(|_| "decrypted key is not UTF-8".to_string())
}

// ---- --import-old-data ------------------------------------------------------------------------

fn read_old(dir: &Path, name: &str) -> Option<Value> {
    // The old app's folder is read-only: a corrupt file there is skipped, never moved.
    read_json_untouched(&dir.join(name))
}

/// `{ provider: [ { id: string, label?: string|null } ] }`
fn seen_compatible(v: &Value) -> bool {
    v.as_object().is_some_and(|m| {
        m.values().all(|rows| {
            rows.as_array().is_some_and(|rows| {
                rows.iter()
                    .all(|r| r.get("id").is_some_and(Value::is_string))
            })
        })
    })
}

/// `{ section: { key: { savedAt: number, data: any } } }`
fn cache_compatible(v: &Value) -> bool {
    v.as_object().is_some_and(|m| {
        m.values().all(|section| {
            section.as_object().is_some_and(|entries| {
                entries.values().all(|e| {
                    e.get("savedAt").is_some_and(Value::is_number) && e.get("data").is_some()
                })
            })
        })
    })
}

/// Import legacy `settings.json`, `seen-accounts.json`, `usage-cache.json`, and encrypted
/// `accounts.json` + `.api-secret` from `src` into `store`.
pub async fn import_old_data(store: &Store, src: &Path) -> i32 {
    if !src.is_dir() {
        eprintln!("not a folder: {}", src.display());
        return 2;
    }
    println!(
        "Importing old UsageDeck data into {}",
        store.dir().display()
    );

    match read_old(src, "settings.json") {
        Some(old) => match store
            .update_settings(|cur| apply_settings_patch(&cur, &old))
            .await
        {
            Ok(_) => println!("settings.json: imported"),
            Err(e) => println!("settings.json: FAILED ({e})"),
        },
        None => println!("settings.json: not found or unreadable, skipped"),
    }

    for (name, compatible) in [
        (SEEN_ACCOUNTS_FILE, seen_compatible as fn(&Value) -> bool),
        (USAGE_CACHE_FILE, cache_compatible),
    ] {
        match read_old(src, name) {
            Some(v) if compatible(&v) => match write_json_atomic(&store.file(name), &v) {
                Ok(()) => println!("{name}: imported"),
                Err(e) => println!("{name}: FAILED ({e})"),
            },
            Some(_) => println!("{name}: shape not compatible, skipped"),
            None => println!("{name}: not found or unreadable, skipped"),
        }
    }

    let rows: Vec<(String, String)> = match read_old(src, ACCOUNTS_FILE) {
        Some(Value::Array(rows)) => rows
            .iter()
            .filter_map(|r| {
                Some((
                    r.get("email")?.as_str()?.to_string(),
                    r.get("key")?.as_str()?.to_string(),
                ))
            })
            .collect(),
        _ => Vec::new(),
    };
    let secret = std::fs::read_to_string(src.join(".api-secret"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let mut imported: Vec<Account> = Vec::new();
    let mut failed = 0usize;
    for (email, stored) in &rows {
        if stored.starts_with(ENC_PREFIX) && secret.is_empty() {
            failed += 1;
            continue;
        }
        match decrypt_key(stored, &secret) {
            Ok(key) if !key.is_empty() => imported.push(Account {
                email: email.clone(),
                key,
            }),
            _ => failed += 1,
        }
    }
    let count = imported.len();
    if count > 0 {
        let result = store
            .update_accounts(move |mut accounts| {
                for a in imported {
                    let lower = a.email.to_lowercase();
                    match accounts
                        .iter_mut()
                        .find(|x| x.email.to_lowercase() == lower)
                    {
                        Some(existing) => existing.key = a.key,
                        None => accounts.push(a),
                    }
                }
                accounts
            })
            .await;
        if let Err(e) = result {
            println!("accounts.json: FAILED to store keys ({e})");
            return 1;
        }
    }
    println!(
        "accounts.json: {} found, {count} keys moved to Credential Manager, {failed} could not be decrypted",
        rows.len()
    );
    0
}

// ---- --dump -----------------------------------------------------------------------------------

fn mask(label: &str) -> String {
    let head: String = label.chars().take(2).collect();
    format!("{head}…")
}

fn reset_for(obj: &Map<String, Value>, percent_key: &str) -> Value {
    // weeklyUsedPercent -> weeklyResetAt; usedPercent / percentUsed -> resetAt / resetsAt;
    // Cursor's percentages reset with the billing cycle.
    let prefix = percent_key
        .split_once("UsedPercent")
        .map(|(p, _)| p)
        .filter(|p| !p.is_empty());
    let mut candidates: Vec<String> = Vec::new();
    if let Some(p) = prefix {
        candidates.push(format!("{p}ResetAt"));
        candidates.push(format!("{p}ResetsAt"));
    } else {
        candidates.extend(["resetAt", "resetsAt", "billingCycleEnd"].map(String::from));
    }
    candidates
        .iter()
        .find_map(|k| obj.get(k).filter(|v| !v.is_null()).cloned())
        .unwrap_or(Value::Null)
}

/// Every usage window inside a provider's usage object: nested `{ percent, resetsAt }` objects
/// and flat `*Percent*` numbers with their matching reset field.
pub fn windows_of(usage: &Value, path: &str, out: &mut Map<String, Value>) {
    let Some(obj) = usage.as_object() else {
        return;
    };
    if obj.contains_key("percent") {
        out.insert(
            if path.is_empty() {
                "window".into()
            } else {
                path.to_string()
            },
            json!({
                "percent": obj["percent"],
                "reset": obj.get("resetsAt").cloned().unwrap_or(Value::Null),
            }),
        );
        return;
    }
    for (k, v) in obj {
        let name = if path.is_empty() {
            k.clone()
        } else {
            format!("{path}.{k}")
        };
        if v.is_object() {
            windows_of(v, &name, out);
        } else if k.to_lowercase().contains("percent") && (v.is_number() || v.is_null()) {
            out.insert(name, json!({ "percent": v, "reset": reset_for(obj, k) }));
        }
    }
}

fn plan_of(usage: &Value) -> Value {
    usage
        .as_object()
        .and_then(|o| {
            ["planName", "planLabel", "planType", "plan"]
                .iter()
                .find_map(|k| o.get(*k).filter(|v| v.is_string()).cloned())
        })
        .unwrap_or(Value::Null)
}

/// One redacted account row: masked label, plan, windows and status flags.
pub fn redact_account(row: &Value) -> Value {
    let label = ["label", "email", "id"]
        .iter()
        .find_map(|k| row.get(*k).and_then(Value::as_str))
        .map(mask);
    let usage = row.get("usage").cloned().unwrap_or(Value::Null);
    let mut windows = Map::new();
    windows_of(&usage, "", &mut windows);
    json!({
        "label": label,
        "plan": plan_of(&usage),
        "windows": windows,
        "stale": row.get("stale").cloned().unwrap_or(Value::Null),
        "authExpired": row.get("authExpired").cloned().unwrap_or(Value::Null),
        "error": row.get("error").cloned().unwrap_or(Value::Null),
    })
}

/// Poll all 5 providers directly through `usagedeck_core::providers` and return the redacted
/// route-keyed summary matching v1's `--dump` contract.
pub async fn dump_providers(core: &Core) -> Value {
    let opencode = providers::opencode::accounts(core).await;
    let cursor_body = providers::cursor::response(core).await;
    let cursor = cursor_body["accounts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let grok = providers::grok::accounts(core).await;
    let codex = providers::codex::accounts(core).await;
    let claude = providers::claude::accounts(core).await;

    let mut out = Map::new();
    for (route, rows) in [
        ("/api/usage", opencode),
        ("/api/cursor", cursor),
        ("/api/grok", grok),
        ("/api/codex", codex),
        ("/api/claude", claude),
    ] {
        let accounts: Vec<Value> = rows.iter().map(redact_account).collect();
        out.insert(
            route.to_string(),
            json!({ "status": 200, "accounts": accounts }),
        );
    }
    Value::Object(out)
}

async fn dump(core: Core) -> i32 {
    let out = dump_providers(&core).await;
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use usagedeck_core::store::MemoryVault;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-cli-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn settings_and_demo_flags_are_found_anywhere_in_argv() {
        let exe = r"C:\Users\me\AppData\Local\UsageDeck\usagedeck.exe";
        assert!(wants_settings([exe, "--settings"]));
        assert!(wants_settings(vec![
            exe.to_string(),
            "--foo".into(),
            "--settings".into(),
        ]));
        assert!(!wants_settings([exe]));
        assert!(!wants_settings([
            exe,
            "--setting",
            "settings",
            "--settings=1",
        ]));

        assert!(wants_demo([exe, "--demo"]));
        assert!(!wants_demo([exe, "--dump"]));
    }

    /// Deterministic `encryptKey` of `lib/key-crypto.ts`, for tests only.
    fn encrypt(plain: &str, secret: &str) -> String {
        let cipher = Aes256Gcm::new(&aes_key(secret).into());
        let iv = [0x11u8; 12];
        let sealed = cipher
            .encrypt(Nonce::from_slice(&iv), plain.as_bytes())
            .unwrap();
        let (data, tag) = sealed.split_at(sealed.len() - 16);
        format!(
            "{ENC_PREFIX}{}:{}:{}",
            URL_SAFE_NO_PAD.encode(iv),
            URL_SAFE_NO_PAD.encode(tag),
            URL_SAFE_NO_PAD.encode(data)
        )
    }

    #[test]
    fn enc_v1_round_trip() {
        let stored = encrypt("sk-test-0123456789", "s3cret");
        assert!(stored.starts_with(ENC_PREFIX));
        assert_eq!(
            decrypt_key(&stored, "s3cret").unwrap(),
            "sk-test-0123456789"
        );
        assert!(decrypt_key(&stored, "wrong").is_err());
        assert!(decrypt_key("enc:v1:abc", "s3cret").is_err());
        assert!(decrypt_key("enc:v1:a:b:c:d", "s3cret").is_err());
        assert_eq!(decrypt_key("sk-plain", "s3cret").unwrap(), "sk-plain");
    }

    #[test]
    fn redaction_masks_label_and_keeps_windows() {
        let row = json!({
            "label": "someone@example.com",
            "stale": false,
            "authExpired": false,
            "error": null,
            "usage": {
                "planType": "plus",
                "usedPercent": 12.5,
                "resetAt": "r1",
                "weeklyUsedPercent": 40,
                "weeklyResetAt": "r2",
            }
        });
        let r = redact_account(&row);
        assert_eq!(r["label"], "so…");
        assert_eq!(r["plan"], "plus");
        assert_eq!(r["windows"]["usedPercent"]["reset"], "r1");
        assert_eq!(r["windows"]["weeklyUsedPercent"]["reset"], "r2");
        assert!(!r.to_string().contains("example.com"));
        assert_eq!(r["error"], Value::Null);
    }

    #[test]
    fn dump_shows_the_error_kind() {
        let row = json!({
            "id": "a@b.com",
            "stale": true,
            "authExpired": false,
            "error": "rate-limited",
            "usage": null,
        });
        assert_eq!(redact_account(&row)["error"], "rate-limited");
    }

    #[test]
    fn compatibility_checks_validate_old_shapes() {
        assert!(seen_compatible(&json!({
            "claude": [{ "id": "a@b.com", "label": "Alice" }]
        })));
        assert!(!seen_compatible(&json!({ "claude": [{}] })));

        assert!(cache_compatible(&json!({
            "claude": { "a@b.com": { "savedAt": 12345, "data": {} } }
        })));
        assert!(!cache_compatible(&json!({
            "claude": { "a@b.com": { "savedAt": "bad", "data": {} } }
        })));
    }

    #[tokio::test]
    async fn import_old_data_migrates_settings_cache_and_encrypted_accounts() {
        let src = temp_dir("old-src");
        let dst = temp_dir("new-dst");

        std::fs::write(
            src.join("settings.json"),
            r#"{ "showProviderNames": true, "claudeHourlyTarget": 25 }"#,
        )
        .unwrap();
        std::fs::write(
            src.join(SEEN_ACCOUNTS_FILE),
            r#"{ "claude": [{ "id": "u@example.com", "label": "User" }] }"#,
        )
        .unwrap();
        std::fs::write(
            src.join(USAGE_CACHE_FILE),
            r#"{ "claude": { "u@example.com": { "savedAt": 1000, "data": { "ok": true } } } }"#,
        )
        .unwrap();
        std::fs::write(src.join(".api-secret"), "my-old-secret\n").unwrap();
        let enc_key = encrypt("oc_sk_migrated_key_9999", "my-old-secret");
        std::fs::write(
            src.join(ACCOUNTS_FILE),
            serde_json::to_string(&json!([
                { "email": "alice@example.com", "key": enc_key },
                { "email": "bad@example.com", "key": "enc:v1:invalid" }
            ]))
            .unwrap(),
        )
        .unwrap();

        let store = Store::new(dst.clone(), Box::new(MemoryVault::default()));
        let code = import_old_data(&store, &src).await;
        assert_eq!(code, 0);

        let settings = store.get_settings();
        assert!(settings.show_provider_names);
        assert_eq!(settings.claude_hourly_target, Some(25));

        let accounts = store.get_accounts().await;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].email, "alice@example.com");
        assert_eq!(accounts[0].key, "oc_sk_migrated_key_9999");
        assert!(dst.join(SEEN_ACCOUNTS_FILE).is_file());
        assert!(dst.join(USAGE_CACHE_FILE).is_file());

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
    }
}
