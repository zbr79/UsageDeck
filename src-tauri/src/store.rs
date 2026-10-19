//! Local storage under the Tauri `app_data_dir`.
//!
//! Ports `lib/fs-atomic.ts` (atomic writes, per-file locks), `lib/settings-schema.ts` +
//! `lib/settings.ts` (defaults and patch rules) and the account list of `lib/opencode.ts`.
//! OpenCode keys no longer live in `accounts.json`: that file holds emails only, and the keys sit
//! in Windows Credential Manager (service `UsageDeck`, user = lower-case email).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

pub const SETTINGS_FILE: &str = "settings.json";
pub const ACCOUNTS_FILE: &str = "accounts.json";
pub const USAGE_CACHE_FILE: &str = "usage-cache.json";
pub const SEEN_ACCOUNTS_FILE: &str = "seen-accounts.json";
pub const WINDOW_POS_FILE: &str = "window-pos.json";
pub const KEYRING_SERVICE: &str = "UsageDeck";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("file error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("credential store error: {0}")]
    Vault(String),
}

// ---- atomic files (lib/fs-atomic.ts) -------------------------------------------------------

/// Write via temp file + rename so a crash mid-write never leaves a truncated file. The rename
/// is retried briefly because Windows can hold the target open (antivirus, indexer) for a moment.
pub fn write_file_atomic(file: &Path, data: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = file.with_file_name(format!(
        "{name}.{}.{nanos:08x}{:x}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    {
        use std::io::Write;
        let mut out = std::fs::File::create(&tmp)?;
        let written = out.write_all(data).and_then(|()| out.sync_all());
        if let Err(err) = written {
            drop(out);
            let _ = std::fs::remove_file(&tmp);
            return Err(err);
        }
    }
    let mut attempt: u64 = 0;
    loop {
        match std::fs::rename(&tmp, file) {
            Ok(()) => return Ok(()),
            Err(err) => {
                // EPERM / EBUSY / EACCES equivalents: access denied (5), sharing violation (32),
                // lock violation (33).
                let busy = err.kind() == io::ErrorKind::PermissionDenied
                    || matches!(err.raw_os_error(), Some(5) | Some(32) | Some(33));
                if attempt >= 5 || !busy {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(25 * (attempt + 1)));
                attempt += 1;
            }
        }
    }
}

pub fn write_json_atomic<T: Serialize + ?Sized>(file: &Path, value: &T) -> Result<(), StoreError> {
    let text = serde_json::to_string_pretty(value)?;
    write_file_atomic(file, text.as_bytes())?;
    Ok(())
}

/// Parsed JSON of one of this app's own files, or None when it is missing or corrupt. A corrupt
/// file is first moved aside to `<name>.corrupt-<unix-seconds>`, so the defaults written next
/// never overwrite the damaged original.
pub fn read_json(file: &Path) -> Option<Value> {
    let bytes = std::fs::read(file).ok()?;
    match parse_json_bytes(&bytes) {
        Some(v) => Some(v),
        None => {
            set_aside_corrupt(file);
            None
        }
    }
}

/// Parsed JSON, or None when missing or corrupt; never touches the file (for other apps' files).
pub fn read_json_untouched(file: &Path) -> Option<Value> {
    parse_json_bytes(&std::fs::read(file).ok()?)
}

fn parse_json_bytes(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

fn set_aside_corrupt(file: &Path) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let aside = file.with_file_name(format!("{name}.corrupt-{secs}"));
    match std::fs::rename(file, &aside) {
        Ok(()) => log::warn!("{name} was not valid JSON; moved it to {name}.corrupt-{secs}"),
        Err(e) => log::warn!("{name} was not valid JSON and could not be moved aside: {e}"),
    }
}

// ---- key vault ------------------------------------------------------------------------------

/// Where OpenCode API keys live. Production: Windows Credential Manager.
pub trait KeyVault: Send + Sync {
    fn get(&self, user: &str) -> Option<String>;
    fn set(&self, user: &str, key: &str) -> Result<(), StoreError>;
    fn delete(&self, user: &str) -> Result<(), StoreError>;
}

/// Windows Credential Manager through the `keyring` crate.
pub struct CredentialVault;

impl KeyVault for CredentialVault {
    fn get(&self, user: &str) -> Option<String> {
        keyring::Entry::new(KEYRING_SERVICE, user)
            .ok()?
            .get_password()
            .ok()
    }
    fn set(&self, user: &str, key: &str) -> Result<(), StoreError> {
        keyring::Entry::new(KEYRING_SERVICE, user)
            .and_then(|e| e.set_password(key))
            .map_err(|e| StoreError::Vault(e.to_string()))
    }
    fn delete(&self, user: &str) -> Result<(), StoreError> {
        match keyring::Entry::new(KEYRING_SERVICE, user).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(StoreError::Vault(e.to_string())),
        }
    }
}

/// In-memory vault for tests.
#[derive(Default)]
pub struct MemoryVault(Mutex<HashMap<String, String>>);

impl KeyVault for MemoryVault {
    fn get(&self, user: &str) -> Option<String> {
        self.0.lock().unwrap().get(user).cloned()
    }
    fn set(&self, user: &str, key: &str) -> Result<(), StoreError> {
        self.0
            .lock()
            .unwrap()
            .insert(user.to_string(), key.to_string());
        Ok(())
    }
    fn delete(&self, user: &str) -> Result<(), StoreError> {
        self.0.lock().unwrap().remove(user);
        Ok(())
    }
}

// ---- store ----------------------------------------------------------------------------------

/// The data folder plus one async lock per file, so read-modify-write cycles are serialised
/// (the `withLock` of `lib/fs-atomic.ts`).
pub struct Store {
    dir: PathBuf,
    locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    vault: Box<dyn KeyVault>,
}

impl Store {
    pub fn new(dir: PathBuf, vault: Box<dyn KeyVault>) -> Self {
        Self {
            dir,
            locks: Mutex::new(HashMap::new()),
            vault,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Hold the returned guard across a read-modify-write of `name`.
    pub async fn lock(&self, name: &str) -> OwnedMutexGuard<()> {
        let m = {
            let mut map = self.locks.lock().unwrap();
            map.entry(name.to_string()).or_default().clone()
        };
        m.lock_owned().await
    }

    pub fn read_json(&self, name: &str) -> Option<Value> {
        read_json(&self.file(name))
    }

    pub fn write_json<T: Serialize + ?Sized>(
        &self,
        name: &str,
        value: &T,
    ) -> Result<(), StoreError> {
        write_json_atomic(&self.file(name), value)
    }

    // -- settings (lib/settings.ts) --

    pub fn get_settings(&self) -> Settings {
        normalize_settings(self.read_json(SETTINGS_FILE).as_ref())
    }

    /// Read-modify-write under the lock so concurrent saves cannot lose updates.
    pub async fn update_settings(
        &self,
        change: impl FnOnce(Settings) -> Settings,
    ) -> Result<Settings, StoreError> {
        let _guard = self.lock(SETTINGS_FILE).await;
        let next = change(self.get_settings());
        self.write_json(SETTINGS_FILE, &next)?;
        Ok(next)
    }

    /// Drop per-account settings when an OpenCode account is removed.
    pub async fn forget_account(&self, email: &str) -> Result<Settings, StoreError> {
        let lower = email.to_lowercase();
        self.update_settings(|mut s| {
            s.display_names.retain(|k, _| k.to_lowercase() != lower);
            s.disabled_accounts.retain(|e| *e != lower);
            s
        })
        .await
    }

    // -- OpenCode accounts (lib/opencode.ts getAccounts / updateAccounts) --

    fn stored_emails(&self) -> Vec<String> {
        match self.read_json(ACCOUNTS_FILE) {
            Some(Value::Array(rows)) => rows
                .iter()
                .filter_map(|r| r.get("email").and_then(Value::as_str).map(str::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Every listed account. When its key cannot be read from the vault the account is still
    /// kept, with an empty `key` (its row then reports `missing-credentials`), so a passing
    /// Credential Manager failure never drops the email from `accounts.json`.
    fn load_accounts(&self) -> Vec<Account> {
        self.stored_emails()
            .into_iter()
            .map(|email| {
                let key = self.vault.get(&email.to_lowercase()).unwrap_or_else(|| {
                    log::info!(
                        "opencode [{}]: no key in Credential Manager",
                        crate::providers::log_id(&email)
                    );
                    String::new()
                });
                Account { email, key }
            })
            .collect()
    }

    pub async fn get_accounts(&self) -> Vec<Account> {
        let _guard = self.lock(ACCOUNTS_FILE).await;
        self.load_accounts()
    }

    pub async fn update_accounts(
        &self,
        change: impl FnOnce(Vec<Account>) -> Vec<Account>,
    ) -> Result<Vec<Account>, StoreError> {
        let _guard = self.lock(ACCOUNTS_FILE).await;
        let before = self.load_accounts();
        let next = change(before.clone());
        // an empty key is one that could not be read: leave the vault entry as it is
        for account in next.iter().filter(|a| !a.key.is_empty()) {
            self.vault
                .set(&account.email.to_lowercase(), &account.key)?;
        }
        let kept: HashSet<String> = next.iter().map(|a| a.email.to_lowercase()).collect();
        for old in &before {
            let lower = old.email.to_lowercase();
            if !kept.contains(&lower) {
                self.vault.delete(&lower)?;
            }
        }
        let rows: Vec<Value> = next
            .iter()
            .map(|a| serde_json::json!({ "email": a.email }))
            .collect();
        self.write_json(ACCOUNTS_FILE, &rows)?;
        Ok(next)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub email: String,
    pub key: String,
}

/// "sk-…a1b2": enough to recognise a key, useless to anyone who sees it.
pub fn key_hint(key: &str) -> String {
    if key.encode_utf16().count() > 8 {
        let chars: Vec<char> = key.chars().collect();
        let head: String = chars.iter().take(3).collect();
        let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
        format!("{head}…{tail}")
    } else {
        "•••".to_string()
    }
}

// ---- settings (lib/settings-schema.ts) ------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelTheme {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetProvider {
    Opencode,
    Cursor,
    Grok,
    Codex,
    Claude,
}

impl WidgetProvider {
    fn parse(id: &str) -> Option<Self> {
        Some(match id {
            "opencode" => Self::Opencode,
            "cursor" => Self::Cursor,
            "grok" => Self::Grok,
            "codex" => Self::Codex,
            "claude" => Self::Claude,
            _ => return None,
        })
    }
}

pub const DEFAULT_PROVIDER_ORDER: [WidgetProvider; 5] = [
    WidgetProvider::Opencode,
    WidgetProvider::Cursor,
    WidgetProvider::Grok,
    WidgetProvider::Codex,
    WidgetProvider::Claude,
];

pub const MAX_NICKNAME_LENGTH: usize = 64;
const MAX_KEY_LENGTH: usize = 320;
const MAX_ENTRIES: usize = 500;
const FORBIDDEN_KEYS: [&str; 3] = ["__proto__", "constructor", "prototype"];

/// Same shape as `Settings` in `lib/types.ts`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub opencode_enabled: bool,
    pub cursor_enabled: bool,
    pub grok_enabled: bool,
    pub codex_enabled: bool,
    pub claude_enabled: bool,
    pub deepseek_peak_hour_warning: bool,
    pub show_provider_names: bool,
    pub theme_mode: ThemeMode,
    pub panel_theme: PanelTheme,
    /// nickname per OpenCode account email, or per provider id; values are always strings
    pub display_names: Map<String, Value>,
    /// lower-cased account keys hidden from the widget
    pub disabled_accounts: Vec<String>,
    pub provider_order: Vec<WidgetProvider>,
    /// register the app in the Windows Run key so it starts with Windows
    pub start_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            opencode_enabled: true,
            cursor_enabled: true,
            grok_enabled: true,
            codex_enabled: true,
            claude_enabled: true,
            deepseek_peak_hour_warning: true,
            show_provider_names: false,
            theme_mode: ThemeMode::Auto,
            panel_theme: PanelTheme::Dark,
            display_names: Map::new(),
            disabled_accounts: Vec::new(),
            provider_order: DEFAULT_PROVIDER_ORDER.to_vec(),
            start_at_login: true,
        }
    }
}

/// JS `string.length` (UTF-16 code units).
fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// JS `string.slice(0, n)` on UTF-16 units, never splitting a character.
fn js_slice(s: &str, n: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        used += c.len_utf16();
        if used > n {
            break;
        }
        out.push(c);
    }
    out
}

/// Keep a saved order, drop unknowns and duplicates, and append any provider that was missing.
pub fn normalize_provider_order(value: &Value) -> Vec<WidgetProvider> {
    let mut out: Vec<WidgetProvider> = Vec::new();
    if let Value::Array(items) = value {
        for item in items {
            if let Some(p) = item.as_str().and_then(WidgetProvider::parse) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    for p in DEFAULT_PROVIDER_ORDER {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Merge nickname changes: a non-empty name sets it, an empty one removes it. Merging (rather
/// than replacing the map) means a client holding a stale copy cannot wipe out other nicknames.
pub fn merge_display_names(
    current: &Map<String, Value>,
    changes: &Value,
) -> Option<Map<String, Value>> {
    let changes = changes.as_object()?;
    let mut out = current.clone();
    for (key, name) in changes {
        let Some(name) = name.as_str() else { continue };
        if key.is_empty() || js_len(key) > MAX_KEY_LENGTH || FORBIDDEN_KEYS.contains(&key.as_str())
        {
            continue;
        }
        let trimmed = js_slice(name.trim(), MAX_NICKNAME_LENGTH);
        if trimmed.is_empty() {
            out.shift_remove(key);
        } else {
            out.insert(key.clone(), Value::String(trimmed));
        }
    }
    Some(out.into_iter().take(MAX_ENTRIES).collect())
}

pub fn sanitize_disabled_accounts(value: &Value) -> Option<Vec<String>> {
    let items = value.as_array()?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let Some(s) = item.as_str() else { continue };
        if s.is_empty() || js_len(s) > MAX_KEY_LENGTH {
            continue;
        }
        let lower = s.to_lowercase();
        if seen.insert(lower.clone()) {
            out.push(lower);
        }
    }
    out.truncate(MAX_ENTRIES);
    Some(out)
}

/// Apply a partial update. Unknown keys and wrongly-typed values are ignored, so a client can
/// only ever change what it sent, and only to valid values.
pub fn apply_settings_patch(current: &Settings, patch: &Value) -> Settings {
    let Some(obj) = patch.as_object() else {
        return current.clone();
    };
    let mut next = current.clone();
    {
        let bools: [(&str, &mut bool); 8] = [
            ("opencodeEnabled", &mut next.opencode_enabled),
            ("cursorEnabled", &mut next.cursor_enabled),
            ("grokEnabled", &mut next.grok_enabled),
            ("codexEnabled", &mut next.codex_enabled),
            ("claudeEnabled", &mut next.claude_enabled),
            (
                "deepseekPeakHourWarning",
                &mut next.deepseek_peak_hour_warning,
            ),
            ("showProviderNames", &mut next.show_provider_names),
            ("startAtLogin", &mut next.start_at_login),
        ];
        for (key, slot) in bools {
            if let Some(Value::Bool(b)) = obj.get(key) {
                *slot = *b;
            }
        }
    }
    match obj.get("themeMode").and_then(Value::as_str) {
        // White was the old duplicate of Light: converge on the single light theme.
        Some("white") | Some("light") => next.theme_mode = ThemeMode::Light,
        Some("auto") => next.theme_mode = ThemeMode::Auto,
        Some("dark") => next.theme_mode = ThemeMode::Dark,
        _ => {}
    }
    match obj.get("panelTheme").and_then(Value::as_str) {
        Some("light") => next.panel_theme = PanelTheme::Light,
        Some("dark") => next.panel_theme = PanelTheme::Dark,
        _ => {}
    }
    if let Some(names) = obj
        .get("displayNames")
        .and_then(|v| merge_display_names(&current.display_names, v))
    {
        next.display_names = names;
    }
    if let Some(disabled) = obj
        .get("disabledAccounts")
        .and_then(sanitize_disabled_accounts)
    {
        next.disabled_accounts = disabled;
    }
    if let Some(order @ Value::Array(_)) = obj.get("providerOrder") {
        next.provider_order = normalize_provider_order(order);
    }
    next
}

/// Settings from whatever is on disk (missing or corrupt values become defaults).
pub fn normalize_settings(raw: Option<&Value>) -> Settings {
    match raw {
        Some(v) => apply_settings_patch(&Settings::default(), v),
        None => Settings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-test-{tag}-{}-{}",
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
    fn atomic_write_round_trip() {
        let dir = temp_dir("atomic");
        let file = dir.join("sub").join("x.json");
        let value = json!({ "a": 1, "b": ["x", "y"], "c": { "d": null } });
        write_json_atomic(&file, &value).unwrap();
        assert_eq!(read_json(&file), Some(value.clone()));
        // overwrite an existing file
        write_json_atomic(&file, &json!([1, 2])).unwrap();
        assert_eq!(read_json(&file), Some(json!([1, 2])));
        // no temp files left behind
        let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        assert_eq!(read_json(&dir.join("missing.json")), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_patch_ignores_unknown_and_invalid_fields() {
        let current = Settings::default();
        let patch = json!({
            "bogus": true,
            "cursorEnabled": "no",
            "themeMode": "purple",
            "panelTheme": 3,
            "providerOrder": "claude",
            "displayNames": ["x"],
            "disabledAccounts": { "a": 1 },
            "__proto__": { "polluted": true }
        });
        let next = apply_settings_patch(&current, &patch);
        assert_eq!(next, current);
        let out = serde_json::to_value(&next).unwrap();
        assert!(out.get("bogus").is_none());
        // a non-object patch changes nothing
        assert_eq!(apply_settings_patch(&current, &json!([1])), current);
    }

    #[test]
    fn settings_patch_applies_valid_fields() {
        let mut current = Settings::default();
        current
            .display_names
            .insert("keep@x.com".into(), Value::String("Keep".into()));
        current
            .display_names
            .insert("drop@x.com".into(), Value::String("Drop".into()));
        let long = "n".repeat(80);
        let patch = json!({
            "cursorEnabled": false,
            "showProviderNames": true,
            "themeMode": "white",
            "panelTheme": "light",
            "displayNames": { "drop@x.com": "  ", "new@x.com": format!("  {long} "), "__proto__": "x", "": "y", "n": 5 },
            "disabledAccounts": ["A@X.com", "a@x.com", "", 7, "b@x.com"],
            "providerOrder": ["claude", "bogus", "claude", "grok"]
        });
        let next = apply_settings_patch(&current, &patch);
        assert!(!next.cursor_enabled);
        assert!(next.show_provider_names);
        assert_eq!(next.theme_mode, ThemeMode::Light);
        assert_eq!(next.panel_theme, PanelTheme::Light);
        assert_eq!(next.display_names.get("keep@x.com"), Some(&json!("Keep")));
        assert!(next.display_names.get("drop@x.com").is_none());
        assert_eq!(
            next.display_names.get("new@x.com"),
            Some(&json!("n".repeat(64)))
        );
        assert_eq!(next.display_names.len(), 2);
        assert_eq!(next.disabled_accounts, vec!["a@x.com", "b@x.com"]);
        use WidgetProvider::*;
        assert_eq!(
            next.provider_order,
            vec![Claude, Grok, Opencode, Cursor, Codex]
        );
        let out = serde_json::to_value(&next).unwrap();
        assert_eq!(out["themeMode"], json!("light"));
        assert_eq!(out["providerOrder"][0], json!("claude"));
        assert_eq!(out["deepseekPeakHourWarning"], json!(true));
    }

    #[test]
    fn settings_patch_start_at_login() {
        let current = Settings::default();
        assert!(current.start_at_login);
        assert_eq!(
            serde_json::to_value(&current).unwrap()["startAtLogin"],
            json!(true)
        );
        let off = apply_settings_patch(&current, &json!({ "startAtLogin": false }));
        assert!(!off.start_at_login);
        // wrong types are ignored
        let same = apply_settings_patch(&off, &json!({ "startAtLogin": "true" }));
        assert!(!same.start_at_login);
        let on = apply_settings_patch(&off, &json!({ "startAtLogin": true }));
        assert!(on.start_at_login);
        // an old settings file without the field starts at login
        assert!(normalize_settings(Some(&json!({ "grokEnabled": false }))).start_at_login);
    }

    #[test]
    fn corrupt_settings_file_gives_defaults() {
        assert_eq!(
            normalize_settings(Some(&json!("garbage"))),
            Settings::default()
        );
        assert_eq!(normalize_settings(None), Settings::default());
    }

    #[test]
    fn corrupt_json_file_is_set_aside_not_overwritten() {
        let dir = temp_dir("corrupt");
        let file = dir.join(SETTINGS_FILE);
        std::fs::write(&file, b"{\"theme\": \"dark\", trunc").unwrap();
        assert_eq!(read_json(&file), None);
        assert!(!file.exists());
        let aside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("settings.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1);
        let secs = aside[0].trim_start_matches("settings.json.corrupt-");
        assert!(secs.parse::<u64>().unwrap() > 1_700_000_000);
        assert_eq!(
            std::fs::read(dir.join(&aside[0])).unwrap(),
            b"{\"theme\": \"dark\", trunc"
        );
        // defaults written afterwards do not touch the damaged copy
        let store = Store::new(dir.clone(), Box::new(MemoryVault::default()));
        assert_eq!(store.get_settings(), Settings::default());
        store.write_json(SETTINGS_FILE, &json!({})).unwrap();
        assert!(dir.join(&aside[0]).exists());
        // a valid file and someone else's file are never moved
        assert!(read_json(&file).is_some());
        let theirs = dir.join("theirs.json");
        std::fs::write(&theirs, b"not json").unwrap();
        assert_eq!(read_json_untouched(&theirs), None);
        assert!(theirs.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn unreadable_key_keeps_the_account() {
        let dir = temp_dir("vault");
        let vault = MemoryVault::default();
        vault.set("a@x.com", "sk-a").unwrap();
        // b@x.com is listed but its Credential Manager read fails (no entry)
        let store = Store::new(dir.clone(), Box::new(vault));
        store
            .write_json(
                ACCOUNTS_FILE,
                &json!([{ "email": "a@x.com" }, { "email": "B@x.com" }]),
            )
            .unwrap();
        let accounts = store.get_accounts().await;
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[1].email, "B@x.com");
        assert!(accounts[1].key.is_empty());

        // an unrelated update keeps the email on disk and writes no empty key
        store
            .update_accounts(|mut list| {
                list.push(Account {
                    email: "c@x.com".into(),
                    key: "sk-c".into(),
                });
                list
            })
            .await
            .unwrap();
        assert_eq!(
            store.read_json(ACCOUNTS_FILE).unwrap(),
            json!([{ "email": "a@x.com" }, { "email": "B@x.com" }, { "email": "c@x.com" }])
        );
        let after = store.get_accounts().await;
        assert!(after[1].key.is_empty());
        assert_eq!(after[2].key, "sk-c");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn key_hint_masks() {
        assert_eq!(key_hint("sk-abcdefghij1234"), "sk-…1234");
        assert_eq!(key_hint("sk-short"), "•••");
    }
}
