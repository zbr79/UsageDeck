//! Headless command-line modes, handled before any window exists:
//!
//! - `--import-old-data <dir>`: one-time copy of the old app's `data/` folder.
//! - `--dump`: run every provider route once and print a redacted JSON summary.
//!
//! Neither mode ever prints an API key or a full account label.

use crate::store::{
    apply_settings_patch, write_json_atomic, Account, CredentialVault, Store, ACCOUNTS_FILE,
    SEEN_ACCOUNTS_FILE, USAGE_CACHE_FILE,
};
use crate::Core;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The Tauri identifier; `app_data_dir()` is `%APPDATA%\<identifier>` on Windows.
const IDENTIFIER: &str = "com.usagedeck.app";
const ENC_PREFIX: &str = "enc:v1:";
const DUMP_ROUTES: [&str; 5] = [
    "/api/usage",
    "/api/cursor",
    "/api/grok",
    "/api/codex",
    "/api/claude",
];

/// Handle a headless mode if the arguments ask for one. Returns the exit code, or None to start
/// the normal app.
pub fn try_run() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first()?.as_str();
    if mode != "--import-old-data" && mode != "--dump" {
        return None;
    }
    attach_console();
    let Some(dir) = app_data_dir() else {
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
        _ => runtime.block_on(dump(Core::new(store))),
    })
}

/// True when the command line (the process's own, or the argv a second launch hands to the
/// running instance) asks for the settings window: `usagedeck.exe --settings`.
pub fn wants_settings<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|a| a.as_ref() == "--settings")
}

fn app_data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join(IDENTIFIER))
}

/// Release builds use the GUI subsystem: without an inherited stdout (a pipe or a file), attach
/// to the parent console so the summary is visible when run from cmd or PowerShell.
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

// ---- enc:v1 keys (lib/key-crypto.ts) ----------------------------------------------------------

fn aes_key(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

fn b64url(part: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).ok()
}

/// `decryptKey` of lib/key-crypto.ts: plaintext passes through; `enc:v1:<iv>:<tag>:<data>`
/// (base64url parts) is AES-256-GCM with key = SHA-256(secret).
pub fn decrypt_key(stored: &str, secret: &str) -> Result<String, String> {
    let Some(rest) = stored.strip_prefix(ENC_PREFIX) else {
        return Ok(stored.to_string());
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let (Some(iv), Some(tag), Some(data)) = (
        parts.first().and_then(|p| b64url(p)),
        parts.get(1).and_then(|p| b64url(p)),
        parts.get(2).and_then(|p| b64url(p)),
    ) else {
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
    crate::store::read_json(&dir.join(name))
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

async fn import_old_data(store: &Store, src: &Path) -> i32 {
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
    let Some(obj) = usage.as_object() else { return };
    if obj.contains_key("percent") {
        out.insert(
            if path.is_empty() { "window".into() } else { path.to_string() },
            json!({ "percent": obj["percent"], "reset": obj.get("resetsAt").cloned().unwrap_or(Value::Null) }),
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

/// One redacted account row: masked label, plan, windows and the status flags.
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

async fn dump(core: Core) -> i32 {
    let mut out = Map::new();
    for route in DUMP_ROUTES {
        let response = crate::api::dispatch(&core, "GET", route, None).await;
        let accounts: Vec<Value> = response.body["accounts"]
            .as_array()
            .map(|rows| rows.iter().map(redact_account).collect())
            .unwrap_or_default();
        out.insert(
            route.to_string(),
            json!({ "status": response.status, "accounts": accounts }),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&Value::Object(out)).unwrap_or_default()
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::OsRng;
    use aes_gcm::AeadCore;

    #[test]
    fn settings_flag_is_found_anywhere_in_argv() {
        let exe = r"C:\Users\me\AppData\Local\UsageDeck\usagedeck.exe";
        assert!(wants_settings([exe, "--settings"]));
        assert!(wants_settings(vec![
            exe.to_string(),
            "--foo".into(),
            "--settings".into()
        ]));
        assert!(!wants_settings([exe]));
        assert!(!wants_settings([
            exe,
            "--setting",
            "settings",
            "--settings=1"
        ]));
    }

    /// encryptKey of lib/key-crypto.ts, for the test only.
    fn encrypt(plain: &str, secret: &str) -> String {
        let cipher = Aes256Gcm::new(&aes_key(secret).into());
        let iv = Aes256Gcm::generate_nonce(&mut OsRng);
        let sealed = cipher.encrypt(&iv, plain.as_bytes()).unwrap();
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
        assert_eq!(decrypt_key("sk-plain", "s3cret").unwrap(), "sk-plain");
    }

    #[test]
    fn redaction_masks_label_and_keeps_windows() {
        let row = json!({
            "label": "someone@example.com",
            "stale": false,
            "authExpired": false,
            "error": null,
            "usage": { "planType": "plus", "usedPercent": 12.5, "resetAt": "r1",
                       "weeklyUsedPercent": 40, "weeklyResetAt": "r2" }
        });
        let r = redact_account(&row);
        assert_eq!(r["label"], "so…");
        assert_eq!(r["plan"], "plus");
        assert_eq!(r["windows"]["usedPercent"]["reset"], "r1");
        assert_eq!(r["windows"]["weeklyUsedPercent"]["reset"], "r2");
        assert!(!r.to_string().contains("example.com"));
    }
}
