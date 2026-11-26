//! Google Antigravity usage limits (supports Antigravity Desktop IDE, Antigravity 2.0,
//! and Antigravity CLI / ACP).
//!
//! Strictly read-only — UsageDeck never writes, rotates, or refreshes third-party OAuth tokens.
//! Live quota is discovered and fetched via:
//! 1. Running local Antigravity language server (`language_server*.exe` / `agy.exe`) loopback
//!    Connect-RPC (`RetrieveUserQuotaSummary`, falling back to `GetUserStatus` /
//!    `GetCommandModelConfigs`), which stays authenticated even when on-disk OAuth access tokens
//!    have expired.
//! 2. Antigravity Desktop IDE SQLite `state.vscdb` (`%APPDATA%\*Antigravity*\User\globalStorage\state.vscdb`):
//!    reads `antigravityUnifiedStateSync.oauthToken`, legacy `jetskiStateSync.agentManagerInitState`,
//!    `antigravityAuthStatus`, and `antigravityUnifiedStateSync.userStatus`.
//! 3. Windows Credential Manager (`gemini:antigravity` and `*antigravity*` generic credentials),
//!    reading raw credential bytes (avoiding UTF-16LE mis-decoding of UTF-8 `go-keyring-*` blobs).
//! 4. Local OAuth token files under `%USERPROFILE%\.gemini\` (`jetski-standalone-oauth-token`,
//!    `antigravity-cli/antigravity-oauth-token`, `antigravity-acp/acp_token.json`, etc.).
//! 5. Cloud `POST /v1internal:retrieveUserQuotaSummary` on `daily-cloudcode-pa.googleapis.com`
//!    (merging missing pools from `cloudcode-pa.googleapis.com`, falling back to
//!    `v1internal:fetchAvailableModels`), and `agy -p /quota` / `agy -p /usage` CLI fallback.

use super::cursor_auth::decode_value;
use super::{
    collect_accounts, CachedProvider, FetchError, LiveAccount, ProviderError, TrackedAccount,
};
use crate::Core;
use base64::engine::general_purpose::{
    STANDARD as B64, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD,
};
use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows::Win32::Security::Credentials::{CredEnumerateW, CredFree, CREDENTIALW};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

const DAILY_QUOTA_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
const PROD_QUOTA_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
const DAILY_MODELS_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels";
const PROD_MODELS_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels";
const DEFAULT_PROJECT_ID: &str = "aicode-consumers";
const USER_AGENT: &str = "antigravity/hub/2.34.0 (aidev_client; os_type=win32; arch=amd64)";
const LOCAL_RPC_SERVICE: &str = "/exa.language_server_pb.LanguageServerService";
const LOCAL_RPC_BODY: &str =
    r#"{"metadata":{"ideName":"antigravity","extensionName":"antigravity","locale":"en"}}"#;
const SECTION: &str = "antigravity";
const PROVIDER: &str = "antigravity";
const DB_BUSY: Duration = Duration::from_millis(150);

static CACHE: LazyLock<CachedProvider<AntigravityUsage>> = LazyLock::new(|| {
    CachedProvider::new(
        SECTION,
        Duration::from_secs(60),
        Some(Duration::from_secs(60)),
    )
});

/// Actionable Antigravity setup state surfaced directly in the taskbar pill, hover card, and
/// Settings so users never have to guess why Antigravity 2.34+ isn't showing quota.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AntigravitySetupIssue {
    /// Antigravity Desktop 2.34+ hardened its local RPC (stdin token) and/or cached desktop token
    /// expired, and Google's official `agy` CLI is not installed yet.
    NeedsAgyCli,
    /// Google's official `agy` CLI is installed, but needs a one-time sign-in.
    NeedsAgySignIn,
}

/// Secret-free diagnostic snapshot of the most recent Antigravity discovery & fetch pass.
static LAST_DIAG: LazyLock<Mutex<Option<(i64, String)>>> = LazyLock::new(|| Mutex::new(None));
static LAST_SETUP_ISSUE: LazyLock<Mutex<Option<AntigravitySetupIssue>>> =
    LazyLock::new(|| Mutex::new(None));

/// Most recent secret-free Antigravity discovery/fetch report `(timestamp_ms, text)`.
pub fn last_diag_report() -> Option<(i64, String)> {
    LAST_DIAG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Most recent actionable setup issue for Antigravity (cleared as soon as live quota succeeds).
pub fn setup_issue() -> Option<AntigravitySetupIssue> {
    *LAST_SETUP_ISSUE.lock().unwrap_or_else(|e| e.into_inner())
}

fn set_setup_issue(issue: Option<AntigravitySetupIssue>) {
    *LAST_SETUP_ISSUE.lock().unwrap_or_else(|e| e.into_inner()) = issue;
}

/// Invalidate Antigravity's in-memory TTL cache so the next poll re-checks immediately (used
/// after the user installs or signs in to `agy`).
pub fn clear_cache() {
    CACHE.invalidate(None);
}

fn save_diag_report(lines: &[String]) {
    let now_ms = Utc::now().timestamp_millis();
    let body = lines.join("\n");
    *LAST_DIAG.lock().unwrap_or_else(|e| e.into_inner()) = Some((now_ms, body));
}

/// Open a visible Windows PowerShell window that installs Google's official Antigravity CLI
/// (`agy`) via `https://antigravity.google/cli/install.ps1` and launches `agy` for sign-in.
pub fn launch_agy_install_terminal() -> Result<(), String> {
    let script = concat!(
        "$ErrorActionPreference = 'Continue'; ",
        "Write-Host '=== UsageDeck: Installing Google Antigravity CLI (agy) ===' -ForegroundColor Cyan; ",
        "Write-Host ''; ",
        "irm https://antigravity.google/cli/install.ps1 | iex; ",
        "$candidates = @(",
        "  (Join-Path $env:LOCALAPPDATA 'agy\\bin\\agy.exe'), ",
        "  (Join-Path $env:LOCALAPPDATA 'Antigravity\\agy.exe'), ",
        "  (Join-Path $env:LOCALAPPDATA 'Antigravity\\bin\\agy.exe')",
        "); ",
        "$agy = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1; ",
        "if (-not $agy) { $cmd = Get-Command agy -ErrorAction SilentlyContinue; if ($cmd) { $agy = $cmd.Source } }; ",
        "if ($agy) { ",
        "  Write-Host ''; ",
        "  Write-Host 'Antigravity CLI installed! Launching agy to verify sign-in (close once signed in)...' -ForegroundColor Green; ",
        "  & $agy ",
        "} else { ",
        "  Write-Host ''; ",
        "  Write-Host 'Install finished. Open a new terminal and run `agy` if prompted to sign in.' -ForegroundColor Yellow ",
        "}"
    );
    std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoExit",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to launch PowerShell: {e}"))
}

/// Open a visible Windows PowerShell window running the installed `agy` CLI so the user can
/// complete a one-time Google sign-in.
pub fn launch_agy_signin_terminal() -> Result<(), String> {
    let bin = find_agy_binary().ok_or_else(|| "agy CLI not found".to_string())?;
    let escaped = bin.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Write-Host '=== UsageDeck: Sign in to Google Antigravity CLI (agy) ===' -ForegroundColor Cyan; & '{escaped}'"
    );
    std::process::Command::new("powershell.exe")
        .args(["-NoLogo", "-NoExit", "-Command", &script])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to launch PowerShell: {e}"))
}

/// One named Antigravity quota pool (e.g. `"Gemini"`, `"Claude & GPT"`, `"Gemini 4 Argon"`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AntigravityQuotaGroup {
    pub name: String,
    pub five_hour_percent: Option<f64>,
    pub five_hour_resets_at: Option<String>,
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at: Option<String>,
}

/// Antigravity 5-hour and weekly quota usage windows, plus per-model-pool breakdown.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AntigravityUsage {
    pub email: Option<String>,
    pub five_hour_percent: Option<f64>,
    pub five_hour_resets_at: Option<String>,
    pub weekly_percent: Option<f64>,
    pub weekly_resets_at: Option<String>,
    #[serde(default)]
    pub groups: Vec<AntigravityQuotaGroup>,
}

/// Read-only OAuth credential discovered for Antigravity.
#[derive(Clone, PartialEq)]
pub struct AntigravityCredential {
    pub access_token: String,
    pub email: Option<String>,
    pub project_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl AntigravityCredential {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|exp| exp <= now)
    }
}

impl std::fmt::Debug for AntigravityCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AntigravityCredential")
            .field("access_token", &"<redacted>")
            .field("email", &self.email)
            .field("project_id", &self.project_id)
            .field("has_expiry", &self.expires_at.is_some())
            .finish()
    }
}

/// Running local Antigravity `language_server` or `agy` Connect-RPC server discovered in memory.
#[derive(Clone, PartialEq, Eq)]
pub struct LocalAntigravityServer {
    pub pid: u32,
    pub csrf_token: Option<String>,
    pub extension_server_port: Option<u16>,
    pub extension_server_csrf_token: Option<String>,
    pub listen_ports: Vec<u16>,
    pub is_cli: bool,
}

impl std::fmt::Debug for LocalAntigravityServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAntigravityServer")
            .field("pid", &self.pid)
            .field(
                "csrf_token",
                &self.csrf_token.as_ref().map(|_| "<redacted>"),
            )
            .field("extension_server_port", &self.extension_server_port)
            .field(
                "extension_server_csrf_token",
                &self
                    .extension_server_csrf_token
                    .as_ref()
                    .map(|_| "<redacted>"),
            )
            .field("listen_ports", &self.listen_ports)
            .field("is_cli", &self.is_cli)
            .finish()
    }
}

// ---- local language_server / agy discovery (read-only, pure Win32) --------------------------

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(
        process_handle: HANDLE,
        process_information_class: u32,
        process_information: *mut std::ffi::c_void,
        process_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

fn read_process_command_line(handle: HANDLE) -> Option<String> {
    let mut needed: u32 = 0;
    // SAFETY: size query against an open process handle with `PROCESS_QUERY_LIMITED_INFORMATION`.
    let _ = unsafe {
        NtQueryInformationProcess(
            handle,
            PROCESS_COMMAND_LINE_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    let cap = (needed as usize).clamp(512, 128 * 1024);
    // Align buffer for `UnicodeString` (8-byte aligned on x64).
    let mut storage = vec![0u64; cap.div_ceil(8)];
    let mut written: u32 = 0;
    // SAFETY: `storage` is properly aligned and sized to `storage.len() * 8` bytes.
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            PROCESS_COMMAND_LINE_INFORMATION,
            storage.as_mut_ptr().cast(),
            (storage.len() * 8) as u32,
            &mut written,
        )
    };
    if status < 0 {
        return None;
    }
    // SAFETY: on NTSTATUS >= 0, the leading struct is a valid `UnicodeString` pointing into `storage`.
    let us = unsafe { &*(storage.as_ptr().cast::<UnicodeString>()) };
    if us.buffer.is_null() || us.length == 0 {
        return None;
    }
    let units = (us.length / 2) as usize;
    let base = storage.as_ptr() as usize;
    let end = base + storage.len() * 8;
    let ptr = us.buffer as usize;
    if ptr < base || ptr + units * 2 > end {
        return None;
    }
    // SAFETY: verified `us.buffer..us.buffer+units` lies inside `storage`.
    let slice = unsafe { std::slice::from_raw_parts(us.buffer, units) };
    Some(String::from_utf16_lossy(slice))
}

fn read_process_image_path(handle: HANDLE) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units; `handle` is valid.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok()
    };
    if !ok || len == 0 {
        return None;
    }
    Some(
        OsString::from_wide(&buf[..len as usize])
            .to_string_lossy()
            .into_owned(),
    )
}

fn split_cmdline_tokens(cmdline: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in cmdline.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn flag_name_matches(tok: &str, canonical_name: &str) -> bool {
    let stripped = tok.trim_start_matches('-');
    if stripped.len() == tok.len() || tok.len() - stripped.len() > 2 {
        return false;
    }
    stripped.eq_ignore_ascii_case(canonical_name)
        || stripped
            .replace('-', "_")
            .eq_ignore_ascii_case(canonical_name)
}

fn extract_flag_value(tokens: &[String], flag: &str) -> Option<String> {
    let canonical = flag.trim_start_matches('-');
    for (i, tok) in tokens.iter().enumerate() {
        if let Some((lhs, rhs)) = tok.split_once('=') {
            if flag_name_matches(lhs, canonical) {
                let v = rhs.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        if flag_name_matches(tok, canonical) {
            if let Some(next) = tokens.get(i + 1) {
                let v = next.trim();
                if !v.is_empty() && !v.starts_with('-') {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Extract only flag names (no values) from a command line for secret-free diagnostics.
pub fn extract_flag_names(cmdline: &str) -> Vec<String> {
    let mut names = Vec::new();
    for tok in split_cmdline_tokens(cmdline) {
        if !tok.starts_with('-') {
            continue;
        }
        let name = tok.split_once('=').map_or(tok.as_str(), |(k, _)| k);
        if name.len() >= 2
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_string());
        }
    }
    names
}

/// Parsed command-line metadata for a running Antigravity `language_server` or `agy` process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedServerArgs {
    pub csrf_token: Option<String>,
    pub extension_server_port: Option<u16>,
    pub extension_server_csrf_token: Option<String>,
    pub is_cli: bool,
}

/// Classify a running process command line / image path as an Antigravity local RPC server
/// (ignoring unrelated Codeium/Windsurf language servers).
pub fn parse_language_server_cmdline(
    exe_name: &str,
    cmdline: &str,
    exe_path: Option<&str>,
) -> Option<ParsedServerArgs> {
    let exe_lower = exe_name.trim().to_ascii_lowercase();
    let tokens = split_cmdline_tokens(cmdline);
    let csrf_token = extract_flag_value(&tokens, "--csrf_token");
    let extension_server_port = extract_flag_value(&tokens, "--extension_server_port")
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|&p| p > 0);
    let extension_server_csrf_token = extract_flag_value(&tokens, "--extension_server_csrf_token");

    if exe_lower == "agy.exe" || exe_lower == "agy" {
        return Some(ParsedServerArgs {
            csrf_token,
            extension_server_port,
            extension_server_csrf_token,
            is_cli: true,
        });
    }

    if !exe_lower.starts_with("language_server") {
        return None;
    }

    let app_data_dir = extract_flag_value(&tokens, "--app_data_dir")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let path_lower = exe_path.unwrap_or_default().to_ascii_lowercase();
    let cmd_lower = cmdline.to_ascii_lowercase();

    let is_antigravity = app_data_dir.contains("antigravity")
        || (app_data_dir.is_empty()
            && (path_lower.contains("antigravity") || cmd_lower.contains("antigravity")));
    if !is_antigravity {
        return None;
    }

    // Desktop language_server requires a CSRF token to answer Connect-RPC requests.
    // Antigravity 2.34+ removed `--csrf_token` from argv (`--read_host_bridge_token_from_stdin`),
    // so external processes cannot call its local RPC directly and must use the cloud or `agy` CLI path.
    if csrf_token.is_none() && extension_server_csrf_token.is_none() {
        return None;
    }

    Some(ParsedServerArgs {
        csrf_token,
        extension_server_port,
        extension_server_csrf_token,
        is_cli: false,
    })
}

/// Produce a secret-free diagnostic reason when a `language_server` candidate is skipped.
pub fn classify_skipped_language_server(cmdline: Option<&str>, exe_path: Option<&str>) -> String {
    let Some(cmdline_str) = cmdline else {
        return "cmdline unreadable".to_string();
    };
    let cmd_lower = cmdline_str.to_ascii_lowercase();
    let path_lower = exe_path.unwrap_or_default().to_ascii_lowercase();
    if !cmd_lower.contains("antigravity") && !path_lower.contains("antigravity") {
        return "non-antigravity".to_string();
    }
    let flags = extract_flag_names(cmdline_str);
    if flags.iter().any(|f| {
        f.to_ascii_lowercase()
            .contains("read_host_bridge_token_from_stdin")
            || f.to_ascii_lowercase().contains("csrf_token_stdin")
    }) {
        return "v2.34+ hardened local RPC (stdin token; use agy CLI)".to_string();
    }
    if flags.is_empty() {
        "missing --csrf_token".to_string()
    } else {
        format!("missing --csrf_token; flags: {}", flags.join(", "))
    }
}

fn listening_loopback_ports_for_pid(pid: u32) -> Vec<u16> {
    let mut ports = Vec::new();
    let mut push_port = |port: u16| {
        if port > 0 && !ports.contains(&port) {
            ports.push(port);
        }
    };

    // IPv4 listeners (`127.0.0.1` or `0.0.0.0`).
    let mut size: u32 = 0;
    // SAFETY: size query for `GetExtendedTcpTable`.
    let res = unsafe {
        GetExtendedTcpTable(
            None,
            &mut size,
            false,
            u32::from(AF_INET.0),
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if (res == ERROR_INSUFFICIENT_BUFFER.0 || res == NO_ERROR.0) && size > 0 {
        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` is sized to `size` bytes as requested.
        let ok = unsafe {
            GetExtendedTcpTable(
                Some(buf.as_mut_ptr().cast()),
                &mut size,
                false,
                u32::from(AF_INET.0),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if ok == NO_ERROR.0 && buf.len() >= std::mem::size_of::<MIB_TCPTABLE_OWNER_PID>() {
            // SAFETY: header starts with `dwNumEntries` followed by `MIB_TCPROW_OWNER_PID` entries.
            let table = unsafe { &*(buf.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>()) };
            let count = table.dwNumEntries as usize;
            let row_start = std::mem::offset_of!(MIB_TCPTABLE_OWNER_PID, table);
            let max_rows =
                (buf.len().saturating_sub(row_start)) / std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
            let n = count.min(max_rows);
            // SAFETY: bounded by `max_rows` inside `buf`.
            let rows = unsafe {
                std::slice::from_raw_parts(
                    buf.as_ptr().add(row_start).cast::<MIB_TCPROW_OWNER_PID>(),
                    n,
                )
            };
            for row in rows {
                if row.dwOwningPid == pid {
                    let ip = row.dwLocalAddr.to_ne_bytes();
                    if ip == [127, 0, 0, 1] || ip == [0, 0, 0, 0] {
                        let port = u16::from_be(row.dwLocalPort as u16);
                        push_port(port);
                    }
                }
            }
        }
    }

    // IPv6 listeners (`::1` or `::`).
    let mut size6: u32 = 0;
    // SAFETY: size query for IPv6 `GetExtendedTcpTable`.
    let res6 = unsafe {
        GetExtendedTcpTable(
            None,
            &mut size6,
            false,
            u32::from(AF_INET6.0),
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if (res6 == ERROR_INSUFFICIENT_BUFFER.0 || res6 == NO_ERROR.0) && size6 > 0 {
        let mut buf = vec![0u8; size6 as usize];
        // SAFETY: `buf` is sized to `size6` bytes as requested.
        let ok = unsafe {
            GetExtendedTcpTable(
                Some(buf.as_mut_ptr().cast()),
                &mut size6,
                false,
                u32::from(AF_INET6.0),
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if ok == NO_ERROR.0 && buf.len() >= std::mem::size_of::<MIB_TCP6TABLE_OWNER_PID>() {
            // SAFETY: header starts with `dwNumEntries` followed by `MIB_TCP6ROW_OWNER_PID` entries.
            let table = unsafe { &*(buf.as_ptr().cast::<MIB_TCP6TABLE_OWNER_PID>()) };
            let count = table.dwNumEntries as usize;
            let row_start = std::mem::offset_of!(MIB_TCP6TABLE_OWNER_PID, table);
            let max_rows = (buf.len().saturating_sub(row_start))
                / std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>();
            let n = count.min(max_rows);
            // SAFETY: bounded by `max_rows` inside `buf`.
            let rows = unsafe {
                std::slice::from_raw_parts(
                    buf.as_ptr().add(row_start).cast::<MIB_TCP6ROW_OWNER_PID>(),
                    n,
                )
            };
            const LOOPBACK_V6: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
            const UNSPEC_V6: [u8; 16] = [0; 16];
            for row in rows {
                if row.dwOwningPid == pid
                    && (row.ucLocalAddr == LOOPBACK_V6 || row.ucLocalAddr == UNSPEC_V6)
                {
                    let port = u16::from_be(row.dwLocalPort as u16);
                    push_port(port);
                }
            }
        }
    }

    ports
}

fn discover_local_servers_with_diag() -> (Vec<LocalAntigravityServer>, String, bool) {
    if !super::codex::real_sign_ins() {
        return (
            Vec::new(),
            "Local servers: skipped (real sign-ins disabled)".to_string(),
            false,
        );
    }
    // SAFETY: read-only ToolHelp process snapshot, closed before returning.
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return (
            Vec::new(),
            "Local servers: CreateToolhelp32Snapshot failed".to_string(),
            false,
        );
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut candidates: Vec<(u32, String)> = Vec::new();
    // SAFETY: `entry.dwSize` is initialized above.
    if unsafe { Process32FirstW(snap, &mut entry).is_ok() } {
        loop {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            let lower = exe.to_ascii_lowercase();
            if lower.starts_with("language_server") || lower == "agy.exe" {
                candidates.push((entry.th32ProcessID, exe));
            }
            // SAFETY: continues enumerating the open snapshot.
            if unsafe { Process32NextW(snap, &mut entry).is_err() } {
                break;
            }
        }
    }
    // SAFETY: `snap` was returned by `CreateToolhelp32Snapshot`.
    let _ = unsafe { CloseHandle(snap) };

    let total_candidates = candidates.len();
    let mut servers = Vec::new();
    let mut skips: Vec<String> = Vec::new();
    let mut saw_v234_server = false;
    for (pid, exe) in candidates {
        // SAFETY: read-only process handle, closed below.
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            skips.push(format!("{exe}: OpenProcess denied"));
            continue;
        };
        let exe_path = read_process_image_path(handle);
        let cmdline = read_process_command_line(handle);
        // SAFETY: `handle` was opened above.
        let _ = unsafe { CloseHandle(handle) };

        let cmdline_str = cmdline.as_deref().unwrap_or("");
        let Some(parsed) = parse_language_server_cmdline(&exe, cmdline_str, exe_path.as_deref())
        else {
            let reason = classify_skipped_language_server(cmdline.as_deref(), exe_path.as_deref());
            if reason.starts_with("v2.34+") {
                saw_v234_server = true;
            }
            skips.push(format!("{exe}: skipped ({reason})"));
            continue;
        };
        let listen_ports = listening_loopback_ports_for_pid(pid);
        if listen_ports.is_empty() && parsed.extension_server_port.is_none() {
            skips.push(format!("{exe}: skipped (0 listening loopback ports)"));
            continue;
        }
        servers.push(LocalAntigravityServer {
            pid,
            csrf_token: parsed.csrf_token,
            extension_server_port: parsed.extension_server_port,
            extension_server_csrf_token: parsed.extension_server_csrf_token,
            listen_ports,
            is_cli: parsed.is_cli,
        });
    }
    // Prefer desktop IDE language_server over CLI server when both are running.
    servers.sort_by_key(|s| s.is_cli);

    let desktop_n = servers.iter().filter(|s| !s.is_cli).count();
    let cli_n = servers.iter().filter(|s| s.is_cli).count();
    let total_ports: usize = servers
        .iter()
        .map(|s| s.listen_ports.len() + usize::from(s.extension_server_port.is_some()))
        .sum();
    let mut diag = format!(
        "Local servers: {desktop_n} desktop, {cli_n} cli (ports: {total_ports}; candidates seen: {total_candidates})"
    );
    if !skips.is_empty() {
        diag.push_str(&format!(" [{}]", skips.join("; ")));
    }
    (servers, diag, saw_v234_server)
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

/// Decode raw Windows Credential Manager blob bytes safely, preferring UTF-8 (which
/// `zalando/go-keyring` writes) before falling back to UTF-16LE.
///
/// Rust's `keyring::Entry::get_password()` on Windows assumes any even-length blob is UTF-16LE,
/// which garbles 50% of UTF-8 `go-keyring-base64:` payloads into CJK characters.
pub fn decode_credential_blob(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    // Trim trailing NUL padding bytes for UTF-8 check.
    let mut utf8_end = bytes.len();
    while utf8_end > 0 && bytes[utf8_end - 1] == 0 {
        utf8_end -= 1;
    }
    let utf8_slice = &bytes[..utf8_end];
    if !utf8_slice.is_empty() && !utf8_slice.contains(&0) {
        if let Ok(text) = std::str::from_utf8(utf8_slice) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if bytes.len().is_multiple_of(2) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .collect();
        let decoded = String::from_utf16_lossy(&units);
        let trimmed = decoded.trim_matches(|c: char| c.is_whitespace() || c == '\0');
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
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

fn is_google_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

/// Extract all distinct Google OAuth2 `ya29.*` access tokens from raw bytes.
fn find_all_ya29_tokens(bytes: &[u8], out: &mut Vec<String>) {
    const PREFIX: &[u8] = b"ya29.";
    let mut pos = 0;
    while pos + PREFIX.len() < bytes.len() {
        let Some(rel) = bytes[pos..].windows(PREFIX.len()).position(|w| w == PREFIX) else {
            break;
        };
        let start = pos + rel;
        let mut end = start + PREFIX.len();
        while end < bytes.len() && is_google_token_char(bytes[end]) {
            end += 1;
        }
        if end - start >= 24 {
            if let Ok(tok) = std::str::from_utf8(&bytes[start..end]) {
                if !out.iter().any(|existing| existing == tok) {
                    out.push(tok.to_string());
                }
            }
        }
        pos = end;
    }
}

fn find_ya29_token(bytes: &[u8]) -> Option<String> {
    let mut found = Vec::new();
    find_all_ya29_tokens(bytes, &mut found);
    found.into_iter().next()
}

fn decode_any_base64(input: &str) -> Option<Vec<u8>> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.len() < 8 {
        return None;
    }
    B64.decode(&clean)
        .or_else(|_| STANDARD_NO_PAD.decode(clean.trim_end_matches('=')))
        .or_else(|_| URL_SAFE.decode(&clean))
        .or_else(|_| URL_SAFE_NO_PAD.decode(clean.trim_end_matches('=')))
        .ok()
}

fn read_protobuf_varint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut val = 0u64;
    let mut shift = 0u32;
    while *pos < bytes.len() && shift < 64 {
        let b = bytes[*pos];
        *pos += 1;
        val |= u64::from(b & 0x7f) << shift;
        if (b & 0x80) == 0 {
            return Some(val);
        }
        shift += 7;
    }
    None
}

/// Walk protobuf wire-format length-delimited fields (`wire_type == 2`) cleanly so a varint length
/// byte that happens to be an ASCII alphanumeric char is never concatenated onto a base64 field.
fn collect_protobuf_tokens(bytes: &[u8], depth: u8, out: &mut Vec<String>) {
    find_all_ya29_tokens(bytes, out);
    if depth == 0 || bytes.len() < 2 {
        return;
    }
    let mut pos = 0usize;
    let mut parsed_any = false;
    while pos < bytes.len() {
        let Some(tag) = read_protobuf_varint(bytes, &mut pos) else {
            break;
        };
        let field_num = tag >> 3;
        let wire_type = tag & 0x7;
        if field_num == 0 {
            break;
        }
        match wire_type {
            0 => {
                if read_protobuf_varint(bytes, &mut pos).is_none() {
                    break;
                }
                parsed_any = true;
            }
            1 => {
                if pos + 8 > bytes.len() {
                    break;
                }
                pos += 8;
                parsed_any = true;
            }
            2 => {
                let Some(len) = read_protobuf_varint(bytes, &mut pos) else {
                    break;
                };
                let Ok(len) = usize::try_from(len) else {
                    break;
                };
                if pos.checked_add(len).is_none_or(|end| end > bytes.len()) {
                    break;
                }
                let slice = &bytes[pos..pos + len];
                pos += len;
                parsed_any = true;

                find_all_ya29_tokens(slice, out);
                // Recursively inspect nested protobuf messages.
                collect_protobuf_tokens(slice, depth - 1, out);
                // If this length-delimited field is an ASCII base64 string, decode and recurse!
                if slice.len() >= 24 {
                    if let Ok(s) = std::str::from_utf8(slice) {
                        if let Some(decoded) = decode_any_base64(s) {
                            collect_protobuf_tokens(&decoded, depth - 1, out);
                        }
                    }
                }
            }
            5 => {
                if pos + 4 > bytes.len() {
                    break;
                }
                pos += 4;
                parsed_any = true;
            }
            _ => break,
        }
    }

    // Fallback ASCII base64 run scan for non-standard/partial buffers when wire parsing stopped early.
    if !parsed_any || out.is_empty() {
        let is_b64_byte =
            |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_');
        let mut i = 0;
        while i < bytes.len() {
            if !is_b64_byte(bytes[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && is_b64_byte(bytes[i]) {
                i += 1;
            }
            if i - start >= 32 {
                if let Ok(run) = std::str::from_utf8(&bytes[start..i]) {
                    if let Some(inner) = decode_any_base64(run) {
                        find_all_ya29_tokens(&inner, out);
                    }
                }
            }
        }
    }
}

/// Decode Antigravity IDE's `antigravityUnifiedStateSync.oauthToken` or legacy
/// `jetskiStateSync.agentManagerInitState` value (base64-encoded protobuf wrapping nested
/// protobuf/base64 payloads containing the `ya29.*` access token).
pub fn extract_oauth_token_from_unified_sync(raw: &str) -> Option<String> {
    extract_all_oauth_tokens_from_blob(raw).into_iter().next()
}

fn extract_all_oauth_tokens_from_blob(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    find_all_ya29_tokens(trimmed.as_bytes(), &mut out);
    if let Some(outer) = decode_any_base64(trimmed) {
        collect_protobuf_tokens(&outer, 4, &mut out);
    } else {
        collect_protobuf_tokens(trimmed.as_bytes(), 3, &mut out);
    }
    out
}

fn extract_email_from_bytes(bytes: &[u8]) -> Option<String> {
    let is_email_char =
        |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+' | b'@');
    let mut i = 0;
    while i < bytes.len() {
        if !is_email_char(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_email_char(bytes[i]) {
            i += 1;
        }
        if let Ok(cand) = std::str::from_utf8(&bytes[start..i]) {
            let trimmed = cand.trim_matches('.');
            if let Some((local, domain)) = trimmed.split_once('@') {
                if !local.is_empty()
                    && !domain.contains('@')
                    && domain.contains('.')
                    && domain.len() >= 4
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
                {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

fn parse_auth_status_json(raw: &str) -> (Option<String>, Option<String>) {
    let Ok(json) = serde_json::from_str::<Value>(raw.trim()) else {
        return (
            find_ya29_token(raw.as_bytes()),
            extract_email_from_bytes(raw.as_bytes()),
        );
    };
    let token = [
        json.get("apiKey"),
        json.get("accessToken"),
        json.get("access_token"),
        json.pointer("/token/access_token"),
        json.pointer("/token/accessToken"),
    ]
    .into_iter()
    .flatten()
    .find_map(|v| {
        v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    let email = [
        json.get("email"),
        json.pointer("/user/email"),
        json.pointer("/account/email"),
        json.get("name")
            .filter(|v| v.as_str().is_some_and(|s| s.contains('@'))),
    ]
    .into_iter()
    .flatten()
    .find_map(|v| {
        v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    (token, email)
}

fn parse_expiry_value(v: &Value) -> Option<DateTime<Utc>> {
    if let Some(s) = v.as_str() {
        let trimmed = s.trim();
        if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
            return Some(dt.with_timezone(&Utc));
        }
        if let Ok(n) = trimmed.parse::<i64>() {
            return if n > 10_000_000_000 {
                DateTime::from_timestamp_millis(n)
            } else if n > 0 {
                DateTime::from_timestamp(n, 0)
            } else {
                None
            };
        }
    }
    if let Some(n) = v.as_i64() {
        return if n > 10_000_000_000 {
            DateTime::from_timestamp_millis(n)
        } else if n > 0 {
            DateTime::from_timestamp(n, 0)
        } else {
            None
        };
    }
    None
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
        .or_else(|| json.get("apiKey"))
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

    let project_id = [
        json.get("project_id"),
        json.get("projectId"),
        json.get("project"),
        json.get("cloudaicompanionProject"),
    ]
    .into_iter()
    .flatten()
    .find_map(|v| {
        v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });

    let expires_at = [
        token_obj.and_then(|t| t.get("expiry")),
        token_obj.and_then(|t| t.get("expires_at")),
        token_obj.and_then(|t| t.get("expiresAt")),
        token_obj.and_then(|t| t.get("expiry_date")),
        token_obj.and_then(|t| t.get("expiryDate")),
        json.get("expiry"),
        json.get("expires_at"),
        json.get("expiresAt"),
        json.get("expiry_date"),
        json.get("expiryDate"),
    ]
    .into_iter()
    .flatten()
    .find_map(parse_expiry_value);

    Some(AntigravityCredential {
        access_token,
        email,
        project_id,
        expires_at,
    })
}

fn parse_credential_flexible(raw: &str) -> Option<AntigravityCredential> {
    if let Some(cred) = parse_credential_json(raw) {
        return Some(cred);
    }
    let decoded = decode_keyring_payload(raw)?;
    let tok = extract_oauth_token_from_unified_sync(&decoded)?;
    let email = extract_email_from_bytes(decoded.as_bytes());
    Some(AntigravityCredential {
        access_token: tok,
        email,
        project_id: None,
        expires_at: None,
    })
}

struct StateDbDiag {
    creds: Vec<AntigravityCredential>,
    detail: String,
}

fn query_ide_state_db_with_diag(db: &Path, flags: OpenFlags) -> rusqlite::Result<StateDbDiag> {
    let conn = rusqlite::Connection::open_with_flags(db, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(DB_BUSY)?;
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

    let auth_status = get("antigravityAuthStatus")?;
    let unified_oauth = get("antigravityUnifiedStateSync.oauthToken")?;
    let legacy_init = get("jetskiStateSync.agentManagerInitState")?;
    let unified_user = get("antigravityUnifiedStateSync.userStatus")?;

    let (status_tok, status_email) = auth_status
        .as_deref()
        .map(parse_auth_status_json)
        .unwrap_or((None, None));

    let email = status_email
        .or_else(|| {
            unified_user.as_deref().and_then(|raw| {
                extract_email_from_bytes(raw.as_bytes())
                    .or_else(|| decode_any_base64(raw).and_then(|b| extract_email_from_bytes(&b)))
            })
        })
        .or_else(|| {
            legacy_init.as_deref().and_then(|raw| {
                extract_email_from_bytes(raw.as_bytes())
                    .or_else(|| decode_any_base64(raw).and_then(|b| extract_email_from_bytes(&b)))
            })
        });

    let mut tokens: Vec<String> = Vec::new();
    let mut push_tok = |tok: String| {
        if !tok.is_empty() && !tokens.iter().any(|t| t == &tok) {
            tokens.push(tok);
        }
    };

    if let Some(raw) = unified_oauth.as_deref() {
        for t in extract_all_oauth_tokens_from_blob(raw) {
            push_tok(t);
        }
    }
    if let Some(raw) = legacy_init.as_deref() {
        for t in extract_all_oauth_tokens_from_blob(raw) {
            push_tok(t);
        }
    }
    if let Some(tok) = status_tok {
        push_tok(tok);
    }

    let yn = |opt: &Option<String>| if opt.is_some() { "yes" } else { "no" };
    let detail = format!(
        "tokens={} (authStatus={}, unifiedOAuth={}, legacyInit={})",
        tokens.len(),
        yn(&auth_status),
        yn(&unified_oauth),
        yn(&legacy_init)
    );
    let creds = tokens
        .into_iter()
        .map(|access_token| AntigravityCredential {
            access_token,
            email: email.clone(),
            project_id: None,
            expires_at: None,
        })
        .collect();
    Ok(StateDbDiag { creds, detail })
}

fn read_ide_state_db_copy_with_diag(db: &Path) -> StateDbDiag {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "usagedeck-antigravity-db-{}-{nanos}",
        std::process::id()
    ));
    if std::fs::create_dir_all(&dir).is_err() {
        return StateDbDiag {
            creds: Vec::new(),
            detail: "temp copy dir failed".into(),
        };
    }
    let res = (|| -> StateDbDiag {
        let copy = dir.join("state.vscdb");
        if std::fs::copy(db, &copy).is_err() {
            return StateDbDiag {
                creds: Vec::new(),
                detail: "copy failed".into(),
            };
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut side = db.as_os_str().to_owned();
            side.push(suffix);
            let mut target = copy.as_os_str().to_owned();
            target.push(suffix);
            let _ = std::fs::copy(PathBuf::from(side), PathBuf::from(target));
        }
        match query_ide_state_db_with_diag(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE) {
            Ok(d) => d,
            Err(e) => StateDbDiag {
                creds: Vec::new(),
                detail: format!("sqlite copy err: {e}"),
            },
        }
    })();
    let _ = std::fs::remove_dir_all(&dir);
    res
}

fn read_ide_state_db_all_with_diag(db: &Path) -> Option<StateDbDiag> {
    if !db.is_file() {
        return None;
    }
    Some(
        match query_ide_state_db_with_diag(db, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(d) => d,
            Err(_) => read_ide_state_db_copy_with_diag(db),
        },
    )
}

/// Read all Antigravity Desktop IDE sign-in tokens from a `state.vscdb` SQLite database
/// (read-only first, falling back to a private temporary copy if the running IDE holds a lock).
pub fn read_ide_state_db_all(db: &Path) -> Vec<AntigravityCredential> {
    read_ide_state_db_all_with_diag(db)
        .map(|d| d.creds)
        .unwrap_or_default()
}

/// Read the primary Antigravity Desktop IDE sign-in from a `state.vscdb` SQLite database.
pub fn read_ide_state_db(db: &Path) -> Option<AntigravityCredential> {
    read_ide_state_db_all(db).into_iter().next()
}

fn ide_state_db_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_unique = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };

    let scan_base = |base: &Path, push: &mut dyn FnMut(PathBuf)| {
        for app_dir in [
            "Antigravity",
            "Antigravity IDE",
            "Antigravity 2",
            "Antigravity - Insiders",
        ] {
            push(
                base.join(app_dir)
                    .join("User")
                    .join("globalStorage")
                    .join("state.vscdb"),
            );
        }
        // Also scan any `*antigravity*` subfolder under AppData / .config.
        if let Ok(rd) = std::fs::read_dir(base) {
            for entry in rd.flatten() {
                let name = entry.file_name();
                let lower = name.to_string_lossy().to_ascii_lowercase();
                if lower.contains("antigravity") {
                    push(
                        entry
                            .path()
                            .join("User")
                            .join("globalStorage")
                            .join("state.vscdb"),
                    );
                }
            }
        }
    };

    for env_key in ["APPDATA", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(env_key)
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
        {
            scan_base(&base, &mut push_unique);
        }
    }
    if let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        scan_base(&home.join(".config"), &mut push_unique);
    }
    out
}

fn read_keyring_entry_bytes(target: Option<&str>, service: &str, user: &str) -> Option<Vec<u8>> {
    let entry = match target {
        Some(t) => keyring::Entry::new_with_target(t, service, user).ok()?,
        None => keyring::Entry::new(service, user).ok()?,
    };
    entry
        .get_secret()
        .ok()
        .or_else(|| entry.get_password().ok().map(String::into_bytes))
}

/// Enumerate read-only Windows Generic Credentials matching `*antigravity*` (or `gemini:antigravity`)
/// using raw `CredentialBlob` bytes to avoid UTF-16LE mis-decoding of UTF-8 `go-keyring-*` entries.
fn read_keyring_credentials_with_diag() -> (Vec<AntigravityCredential>, usize) {
    let mut found = Vec::new();
    let mut matched_entries = 0usize;
    let mut push_cred = |cred: AntigravityCredential| {
        if !found
            .iter()
            .any(|c: &AntigravityCredential| c.access_token == cred.access_token)
        {
            found.push(cred);
        }
    };

    // 1. Direct lookup of Go's `zalando/go-keyring` default target `"gemini:antigravity"`.
    for (target, service, user) in [
        (Some("gemini:antigravity"), "gemini", "antigravity"),
        (None, "gemini", "antigravity"),
        (Some("antigravity:oauth"), "antigravity", "oauth"),
    ] {
        if let Some(bytes) = read_keyring_entry_bytes(target, service, user) {
            matched_entries += 1;
            if let Some(text) = decode_credential_blob(&bytes) {
                if let Some(cred) = parse_credential_flexible(&text) {
                    push_cred(cred);
                }
            }
        }
    }

    // 2. Wildcard enumeration (`*antigravity*`) so per-account or CLI-specific keyring targets work.
    let filter: Vec<u16> = "*antigravity*"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut count: u32 = 0;
    let mut creds_ptr: *mut *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `filter` is NUL-terminated UTF-16; `flags = None` permits wildcard TargetName filter;
    // `creds_ptr` is freed via `CredFree` below.
    let ok = unsafe {
        CredEnumerateW(PCWSTR(filter.as_ptr()), None, &mut count, &mut creds_ptr).is_ok()
    };
    if ok && !creds_ptr.is_null() {
        matched_entries += count as usize;
        // SAFETY: `CredEnumerateW` guarantees `count` valid `*mut CREDENTIALW` pointers.
        let slice = unsafe { std::slice::from_raw_parts(creds_ptr, count as usize) };
        for &ptr in slice {
            if ptr.is_null() {
                continue;
            }
            // SAFETY: non-null entry from `CredEnumerateW`.
            let cred = unsafe { &*ptr };
            if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
                continue;
            }
            // SAFETY: `CredentialBlob` has `CredentialBlobSize` bytes.
            let blob = unsafe {
                std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize)
            };
            if let Some(text) = decode_credential_blob(blob) {
                if let Some(parsed) = parse_credential_flexible(&text) {
                    push_cred(parsed);
                }
            }
        }
        // SAFETY: `creds_ptr` was allocated by `CredEnumerateW`.
        unsafe { CredFree(creds_ptr.cast()) };
    }

    (found, matched_entries)
}

fn read_file_credentials_in_with_count(home: &Path) -> (Vec<AntigravityCredential>, usize) {
    let mut found = Vec::new();
    let mut files_existed = 0usize;
    let mut try_file = |path: PathBuf| {
        if path.is_file() {
            files_existed += 1;
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Some(cred) = parse_credential_flexible(&text) {
                    found.push(cred);
                }
            }
        }
    };

    let gemini_dir = home.join(".gemini");
    // Standalone Antigravity 2.0 / Jetski token at root of ~/.gemini as well as subdirs.
    for filename in ["jetski-standalone-oauth-token", "antigravity-oauth-token"] {
        try_file(gemini_dir.join(filename));
    }
    // Official Antigravity CLI (`antigravity-cli`), ACP (`antigravity-acp`), and base folders.
    for subdir in ["antigravity", "antigravity-cli", "antigravity-acp"] {
        let dir = gemini_dir.join(subdir);
        for filename in [
            "jetski-standalone-oauth-token",
            "antigravity-oauth-token",
            "acp_token.json",
            "oauth_creds.json",
            "credentials.json",
        ] {
            try_file(dir.join(filename));
        }
    }
    // Also check ~/.antigravity if present.
    let ag_home = home.join(".antigravity");
    for filename in [
        "antigravity-oauth-token",
        "oauth_creds.json",
        "credentials.json",
    ] {
        try_file(ag_home.join(filename));
    }
    (found, files_existed)
}

#[cfg(test)]
fn read_file_credential_in(home: &Path) -> Option<AntigravityCredential> {
    read_file_credentials_in_with_count(home)
        .0
        .into_iter()
        .next()
}

/// Discover all available Antigravity credentials on this machine in priority order, returning
/// both the deduplicated credentials and secret-free diagnostic lines.
fn read_credentials_with_diag() -> (Vec<AntigravityCredential>, Vec<String>) {
    if !super::codex::real_sign_ins() {
        return (
            Vec::new(),
            vec!["Credentials: skipped (real sign-ins disabled)".to_string()],
        );
    }
    let mut creds = Vec::new();
    let mut push_unique = |cand: AntigravityCredential| {
        if !creds
            .iter()
            .any(|c: &AntigravityCredential| c.access_token == cand.access_token)
        {
            creds.push(cand);
        }
    };

    let db_candidates = ide_state_db_candidates();
    let mut db_notes = Vec::new();
    for db in &db_candidates {
        if let Some(d) = read_ide_state_db_all_with_diag(db) {
            let app_folder = db
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("Antigravity");
            db_notes.push(format!("{app_folder}: {}", d.detail));
            for cred in d.creds {
                push_unique(cred);
            }
        }
    }
    let db_line = if db_notes.is_empty() {
        format!(
            "state.vscdb: 0 existing of {} paths checked",
            db_candidates.len()
        )
    } else {
        format!(
            "state.vscdb: {} existing [{}]",
            db_notes.len(),
            db_notes.join("; ")
        )
    };

    let now = Utc::now();
    let (kr_creds, kr_entries) = read_keyring_credentials_with_diag();
    let kr_count = kr_creds.len();
    let kr_expired = kr_creds.iter().filter(|c| c.is_expired(now)).count();
    for cred in kr_creds {
        push_unique(cred);
    }
    let kr_line = if kr_expired > 0 {
        format!(
            "Credential Manager: {kr_count} token(s) decoded ({kr_expired} expired; entries matched: {kr_entries})"
        )
    } else {
        format!("Credential Manager: {kr_count} token(s) decoded (entries matched: {kr_entries})")
    };

    let (file_count, file_expired, files_existed) = if let Some(home) =
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    {
        let (fc, existed) = read_file_credentials_in_with_count(&home);
        let n = fc.len();
        let exp = fc.iter().filter(|c| c.is_expired(now)).count();
        for cred in fc {
            push_unique(cred);
        }
        (n, exp, existed)
    } else {
        (0, 0, 0)
    };
    let file_line = if file_expired > 0 {
        format!(
            "Local files (~/.gemini, ~/.antigravity): {file_count} token(s) decoded ({file_expired} expired; files existed: {files_existed})"
        )
    } else {
        format!(
            "Local files (~/.gemini, ~/.antigravity): {file_count} token(s) decoded (files existed: {files_existed})"
        )
    };

    (creds, vec![db_line, kr_line, file_line])
}

// ---- quota summary & local RPC parsing ------------------------------------------------------

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
            let wl = w.to_ascii_lowercase();
            if matches!(
                wl.as_str(),
                "5h" | "five_hour" | "five-hour" | "five hour" | "5_hour" | "5-hour"
            ) || wl.contains("five_hour")
                || wl.contains("5_hour")
                || wl.contains("5-hour")
            {
                return Some(Self::FiveHour);
            }
            if matches!(
                wl.as_str(),
                "weekly" | "week" | "7d" | "seven_day" | "seven-day" | "seven day"
            ) || wl.contains("week")
                || wl.contains("7d")
            {
                return Some(Self::Weekly);
            }
        }
        let id = bucket_id?.trim().to_ascii_lowercase();
        if id.ends_with("-5h")
            || id.ends_with("_5h")
            || id.contains("five_hour")
            || id.contains("five-hour")
        {
            Some(Self::FiveHour)
        } else if id.ends_with("-weekly")
            || id.ends_with("_weekly")
            || id.contains("weekly")
            || id.contains("7d")
        {
            Some(Self::Weekly)
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct ParsedBucket {
    kind: WindowKind,
    percent_used: f64,
    resets_at: Option<String>,
}

fn short_group_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("argon") {
        "Gemini 4 Argon".to_string()
    } else if lower == "gemini models" || lower == "gemini" {
        "Gemini".to_string()
    } else if lower.contains("claude") && lower.contains("gpt") {
        "Claude & GPT".to_string()
    } else if trimmed.is_empty() {
        "Quota".to_string()
    } else {
        trimmed.to_string()
    }
}

fn model_pool_name(label_or_id: &str) -> Option<&'static str> {
    let lower = label_or_id.to_ascii_lowercase();
    if lower.contains("argon") {
        Some("Gemini 4 Argon")
    } else if lower.contains("gemini") {
        Some("Gemini")
    } else if lower.contains("claude")
        || lower.contains("sonnet")
        || lower.contains("opus")
        || lower.contains("gpt")
    {
        Some("Claude & GPT")
    } else {
        None
    }
}

fn sanitize_reset_time(
    raw_reset: Option<DateTime<Utc>>,
    remaining_fraction: f64,
    has_description: bool,
    kind: WindowKind,
    now: DateTime<Utc>,
) -> Option<String> {
    raw_reset.and_then(|reset_dt| {
        let untouched = (remaining_fraction - 1.0).abs() < 1e-9;
        if untouched && !has_description {
            let expected = now + kind.duration();
            if (reset_dt - expected).num_seconds().abs() <= 60 {
                return None;
            }
        }
        Some(reset_dt.to_rfc3339_opts(SecondsFormat::Millis, true))
    })
}

fn parse_bucket(bucket: &Value, now: DateTime<Utc>) -> Option<ParsedBucket> {
    let obj = bucket.as_object()?;
    if obj.get("disabled") == Some(&Value::Bool(true)) {
        return None;
    }
    let kind = WindowKind::parse(
        obj.get("window")
            .or_else(|| obj.get("cadence"))
            .and_then(Value::as_str),
        obj.get("bucketId")
            .or_else(|| obj.get("bucket_id"))
            .or_else(|| obj.get("displayName"))
            .and_then(Value::as_str),
    )?;
    let raw_reset = obj
        .get("resetTime")
        .or_else(|| obj.get("reset_time"))
        .or_else(|| {
            obj.get("remaining")
                .and_then(|r| r.get("resetTime").or_else(|| r.get("reset_time")))
        })
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
        .map(|d| d.with_timezone(&Utc));

    let remaining_fraction = match obj
        .get("remainingFraction")
        .or_else(|| obj.get("remaining_fraction"))
        .or_else(|| {
            obj.get("remaining").and_then(|r| {
                r.get("remainingFraction")
                    .or_else(|| r.get("remaining_fraction"))
            })
        })
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
    {
        Some(f) => f.clamp(0.0, 1.0),
        // Protobuf3 JSON omits zero-valued float fields: an active bucket with a resetTime
        // and omitted remainingFraction means 0% remaining (100% used).
        None if raw_reset.is_some() => 0.0,
        None => return None,
    };

    let percent_used = ((1.0 - remaining_fraction) * 100.0).clamp(0.0, 100.0);
    let has_description = obj
        .get("description")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());

    // For an untouched bucket (remainingFraction == 1.0, no description), Google returns a rolling
    // resetTime ≈ now + window. Drop that synthetic rolling timestamp so idle bars stay clean.
    let resets_at = sanitize_reset_time(raw_reset, remaining_fraction, has_description, kind, now);

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

fn merge_usage(primary: &mut AntigravityUsage, secondary: AntigravityUsage) {
    if primary.email.is_none() {
        primary.email = secondary.email;
    }
    if let Some(pct) = secondary.five_hour_percent {
        merge_window(
            &mut primary.five_hour_percent,
            &mut primary.five_hour_resets_at,
            ParsedBucket {
                kind: WindowKind::FiveHour,
                percent_used: pct,
                resets_at: secondary.five_hour_resets_at,
            },
        );
    }
    if let Some(pct) = secondary.weekly_percent {
        merge_window(
            &mut primary.weekly_percent,
            &mut primary.weekly_resets_at,
            ParsedBucket {
                kind: WindowKind::Weekly,
                percent_used: pct,
                resets_at: secondary.weekly_resets_at,
            },
        );
    }
    for sg in secondary.groups {
        if let Some(existing) = primary
            .groups
            .iter_mut()
            .find(|g| g.name.eq_ignore_ascii_case(&sg.name))
        {
            if let Some(pct) = sg.five_hour_percent {
                merge_window(
                    &mut existing.five_hour_percent,
                    &mut existing.five_hour_resets_at,
                    ParsedBucket {
                        kind: WindowKind::FiveHour,
                        percent_used: pct,
                        resets_at: sg.five_hour_resets_at,
                    },
                );
            }
            if let Some(pct) = sg.weekly_percent {
                merge_window(
                    &mut existing.weekly_percent,
                    &mut existing.weekly_resets_at,
                    ParsedBucket {
                        kind: WindowKind::Weekly,
                        percent_used: pct,
                        resets_at: sg.weekly_resets_at,
                    },
                );
            }
        } else {
            primary.groups.push(sg);
        }
    }
}

fn has_primary_gemini_group(usage: &AntigravityUsage) -> bool {
    usage.groups.iter().any(|g| {
        let lower = g.name.to_ascii_lowercase();
        lower.contains("gemini") && !lower.contains("argon")
    })
}

fn max_usage_percent(usage: &AntigravityUsage) -> f64 {
    let mut peak = usage
        .five_hour_percent
        .unwrap_or(0.0)
        .max(usage.weekly_percent.unwrap_or(0.0));
    for g in &usage.groups {
        if let Some(p) = g.five_hour_percent {
            peak = peak.max(p);
        }
        if let Some(p) = g.weekly_percent {
            peak = peak.max(p);
        }
    }
    peak
}

fn format_usage_groups_diag(usage: &AntigravityUsage) -> String {
    let fmt_pct = |v: Option<f64>| match v {
        Some(p) => format!("{:.0}%", p.round()),
        None => "-".to_string(),
    };
    if usage.groups.is_empty() {
        return format!(
            "Groups: summary (5h {}, weekly {})",
            fmt_pct(usage.five_hour_percent),
            fmt_pct(usage.weekly_percent),
        );
    }
    let parts: Vec<String> = usage
        .groups
        .iter()
        .map(|g| {
            format!(
                "{} (5h {}, weekly {})",
                g.name,
                fmt_pct(g.five_hour_percent),
                fmt_pct(g.weekly_percent),
            )
        })
        .collect();
    format!("Groups: {}", parts.join(" | "))
}

/// Parse a `retrieveUserQuotaSummary` JSON reply into `AntigravityUsage`.
pub fn parse_quota_summary(
    bytes: &[u8],
    email: Option<String>,
    now: DateTime<Utc>,
) -> Result<AntigravityUsage, ProviderError> {
    let reply: Value = super::parse_reply(bytes)?;
    parse_quota_summary_value(&reply, email, now)
}

fn parse_quota_summary_value(
    reply: &Value,
    email: Option<String>,
    now: DateTime<Utc>,
) -> Result<AntigravityUsage, ProviderError> {
    let Some(root) = reply.as_object() else {
        return Err(ProviderError::SchemaChanged);
    };

    let mut usage = AntigravityUsage {
        email,
        ..AntigravityUsage::default()
    };

    if let Some(groups) = root.get("groups").and_then(Value::as_array) {
        for group in groups {
            let name = group
                .get("displayName")
                .or_else(|| group.get("display_name"))
                .or_else(|| group.get("name"))
                .and_then(Value::as_str)
                .map(short_group_name)
                .unwrap_or_else(|| "Quota".to_string());
            let mut g = AntigravityQuotaGroup {
                name,
                ..AntigravityQuotaGroup::default()
            };
            if let Some(buckets) = group.get("buckets").and_then(Value::as_array) {
                for bucket in buckets {
                    if let Some(parsed) = parse_bucket(bucket, now) {
                        match parsed.kind {
                            WindowKind::FiveHour => {
                                merge_window(
                                    &mut g.five_hour_percent,
                                    &mut g.five_hour_resets_at,
                                    parsed.clone(),
                                );
                                merge_window(
                                    &mut usage.five_hour_percent,
                                    &mut usage.five_hour_resets_at,
                                    parsed,
                                );
                            }
                            WindowKind::Weekly => {
                                merge_window(
                                    &mut g.weekly_percent,
                                    &mut g.weekly_resets_at,
                                    parsed.clone(),
                                );
                                merge_window(
                                    &mut usage.weekly_percent,
                                    &mut usage.weekly_resets_at,
                                    parsed,
                                );
                            }
                        }
                    }
                }
            }
            if g.five_hour_percent.is_some() || g.weekly_percent.is_some() {
                usage.groups.push(g);
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

fn push_model_quota_entry(
    usage: &mut AntigravityUsage,
    pool_name: &str,
    remaining_fraction: f64,
    raw_reset: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) {
    let percent_used = ((1.0 - remaining_fraction.clamp(0.0, 1.0)) * 100.0).clamp(0.0, 100.0);
    let resets_at = sanitize_reset_time(
        raw_reset,
        remaining_fraction,
        false,
        WindowKind::FiveHour,
        now,
    );
    let candidate = ParsedBucket {
        kind: WindowKind::FiveHour,
        percent_used,
        resets_at,
    };

    let grp = match usage.groups.iter_mut().find(|g| g.name == pool_name) {
        Some(existing) => existing,
        None => {
            usage.groups.push(AntigravityQuotaGroup {
                name: pool_name.to_string(),
                ..AntigravityQuotaGroup::default()
            });
            usage.groups.last_mut().expect("pushed above")
        }
    };
    merge_window(
        &mut grp.five_hour_percent,
        &mut grp.five_hour_resets_at,
        candidate.clone(),
    );
    merge_window(
        &mut usage.five_hour_percent,
        &mut usage.five_hour_resets_at,
        candidate,
    );
}

/// Parse local Antigravity `GetUserStatus` or `GetCommandModelConfigs` Connect-RPC replies.
pub fn parse_local_user_status(
    bytes: &[u8],
    email_hint: Option<String>,
    now: DateTime<Utc>,
) -> Result<AntigravityUsage, ProviderError> {
    let reply: Value = super::parse_reply(bytes)?;
    let user_status = reply.get("userStatus").unwrap_or(&reply);
    let email = user_status
        .get("email")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(email_hint);

    let configs = user_status
        .pointer("/cascadeModelConfigData/clientModelConfigs")
        .or_else(|| reply.get("clientModelConfigs"))
        .and_then(Value::as_array)
        .ok_or(ProviderError::SchemaChanged)?;

    let mut usage = AntigravityUsage {
        email,
        ..AntigravityUsage::default()
    };

    for cfg in configs {
        let Some(quota_info) = cfg.get("quotaInfo") else {
            continue;
        };
        let label = cfg
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| cfg.pointer("/modelOrAlias/model").and_then(Value::as_str))
            .unwrap_or_default();
        let Some(pool) = model_pool_name(label) else {
            continue;
        };

        let raw_reset = quota_info
            .get("resetTime")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
            .map(|d| d.with_timezone(&Utc));
        let remaining = match quota_info
            .get("remainingFraction")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
        {
            Some(f) => f.clamp(0.0, 1.0),
            // Proto3 omits 0.0 when exhausted; require resetTime so ineligible models are skipped.
            None if raw_reset.is_some() => 0.0,
            None => continue,
        };
        push_model_quota_entry(&mut usage, pool, remaining, raw_reset, now);
    }

    if usage.five_hour_percent.is_none() {
        return Err(ProviderError::SchemaChanged);
    }
    Ok(usage)
}

/// Parse cloud `POST /v1internal:fetchAvailableModels` reply as a fallback when
/// `retrieveUserQuotaSummary` is unavailable for a token.
pub fn parse_fetch_available_models(
    bytes: &[u8],
    email: Option<String>,
    now: DateTime<Utc>,
) -> Result<AntigravityUsage, ProviderError> {
    let reply: Value = super::parse_reply(bytes)?;
    let models = reply
        .get("models")
        .and_then(Value::as_object)
        .ok_or(ProviderError::SchemaChanged)?;

    let mut usage = AntigravityUsage {
        email,
        ..AntigravityUsage::default()
    };

    for (model_id, entry) in models {
        let Some(quota_info) = entry.get("quotaInfo") else {
            continue;
        };
        let label = entry
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(model_id);
        let Some(pool) = model_pool_name(label).or_else(|| model_pool_name(model_id)) else {
            continue;
        };
        let raw_reset = quota_info
            .get("resetTime")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
            .map(|d| d.with_timezone(&Utc));
        let Some(remaining) = quota_info
            .get("remainingFraction")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
        else {
            // Only treat omitted remainingFraction as 0% when a resetTime is explicitly present
            // (free-tier accounts omit both on inaccessible 3P models).
            if raw_reset.is_some() {
                push_model_quota_entry(&mut usage, pool, 0.0, raw_reset, now);
            }
            continue;
        };
        push_model_quota_entry(&mut usage, pool, remaining, raw_reset, now);
    }

    if usage.five_hour_percent.is_none() {
        return Err(ProviderError::SchemaChanged);
    }
    Ok(usage)
}

// ---- local RPC fetch ------------------------------------------------------------------------

fn build_local_rpc_client() -> Option<reqwest::Client> {
    // Strictly used for `127.0.0.1` loopback calls to Antigravity's local self-signed Connect-RPC
    // language server. Outbound internet traffic always uses `core.http` (hardened TLS 1.2+).
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .ok()
}

async fn call_local_rpc(
    client: &reqwest::Client,
    scheme: &str,
    port: u16,
    method: &str,
    csrf_token: Option<&str>,
) -> Result<Vec<u8>, String> {
    let url = format!("{scheme}://127.0.0.1:{port}{LOCAL_RPC_SERVICE}/{method}");
    let mut req = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("Connect-Protocol-Version", "1")
        .body(LOCAL_RPC_BODY);
    if let Some(tok) = csrf_token.filter(|t| !t.is_empty()) {
        req = req.header("X-Codeium-Csrf-Token", tok);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("connect err ({})", e.without_url()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|_| "body read err".to_string())
}

async fn fetch_via_local_server(
    client: &reqwest::Client,
    server: &LocalAntigravityServer,
    email_hint: Option<String>,
    trace: &mut Vec<String>,
) -> Option<AntigravityUsage> {
    // Build ordered `(scheme, port, csrf_token)` candidates: HTTPS connect ports first, then
    // HTTP fallback on `extension_server_port`.
    let mut endpoints: Vec<(&str, u16, Option<&str>)> = Vec::new();
    let mut seen_ports: HashSet<u16> = HashSet::new();

    for &port in &server.listen_ports {
        if server.extension_server_port == Some(port) {
            continue;
        }
        if seen_ports.insert(port) {
            endpoints.push(("https", port, server.csrf_token.as_deref()));
        }
    }
    if let Some(ext_port) = server.extension_server_port {
        let http_tok = server
            .extension_server_csrf_token
            .as_deref()
            .or(server.csrf_token.as_deref());
        endpoints.push(("http", ext_port, http_tok));
        endpoints.push(("https", ext_port, server.csrf_token.as_deref()));
    }
    // Also try plain HTTP on listening loopback ports if HTTPS handshake failed (covers both
    // CLI servers and multi-port desktop language servers without `--extension_server_port`).
    for &port in &server.listen_ports {
        if server.extension_server_port == Some(port) {
            continue;
        }
        endpoints.push(("http", port, server.csrf_token.as_deref()));
    }

    let now = Utc::now();
    for (scheme, port, csrf) in endpoints {
        // 1. Prefer RetrieveUserQuotaSummary (includes both 5-hour and weekly pools!).
        match call_local_rpc(client, scheme, port, "RetrieveUserQuotaSummary", csrf).await {
            Ok(bytes) => match parse_quota_summary(&bytes, email_hint.clone(), now) {
                Ok(mut usage) => {
                    if usage.email.is_none() {
                        // Best-effort email lookup from GetUserStatus on the same working port.
                        if let Ok(ub) =
                            call_local_rpc(client, scheme, port, "GetUserStatus", csrf).await
                        {
                            if let Ok(u) = parse_local_user_status(&ub, email_hint.clone(), now) {
                                usage.email = u.email;
                            }
                        }
                    }
                    trace.push(format!(
                        "local {scheme}:{port} RetrieveUserQuotaSummary -> ok"
                    ));
                    log::info!(
                        "antigravity: read live quota via local server RetrieveUserQuotaSummary"
                    );
                    return Some(usage);
                }
                Err(e) => trace.push(format!(
                    "local {scheme}:{port} RetrieveUserQuotaSummary -> HTTP 200 ({e})"
                )),
            },
            Err(e) => trace.push(format!(
                "local {scheme}:{port} RetrieveUserQuotaSummary -> {e}"
            )),
        }
        // 2. Fallback to GetUserStatus (real-time 5-hour per-pool quota + email).
        match call_local_rpc(client, scheme, port, "GetUserStatus", csrf).await {
            Ok(bytes) => match parse_local_user_status(&bytes, email_hint.clone(), now) {
                Ok(usage) => {
                    trace.push(format!("local {scheme}:{port} GetUserStatus -> ok"));
                    log::info!("antigravity: read live quota via local server GetUserStatus");
                    return Some(usage);
                }
                Err(e) => trace.push(format!(
                    "local {scheme}:{port} GetUserStatus -> HTTP 200 ({e})"
                )),
            },
            Err(e) => trace.push(format!("local {scheme}:{port} GetUserStatus -> {e}")),
        }
        // 3. Fallback to GetCommandModelConfigs.
        match call_local_rpc(client, scheme, port, "GetCommandModelConfigs", csrf).await {
            Ok(bytes) => match parse_local_user_status(&bytes, email_hint.clone(), now) {
                Ok(usage) => {
                    trace.push(format!(
                        "local {scheme}:{port} GetCommandModelConfigs -> ok"
                    ));
                    log::info!(
                        "antigravity: read live quota via local server GetCommandModelConfigs"
                    );
                    return Some(usage);
                }
                Err(e) => trace.push(format!(
                    "local {scheme}:{port} GetCommandModelConfigs -> HTTP 200 ({e})"
                )),
            },
            Err(e) => trace.push(format!(
                "local {scheme}:{port} GetCommandModelConfigs -> {e}"
            )),
        }
    }
    None
}

// ---- cloud fetch ----------------------------------------------------------------------------

struct CloudError {
    err: ProviderError,
    detail: String,
}

impl CloudError {
    fn from_status(status: u16, expired_hint: bool) -> Self {
        match status {
            401 => Self {
                err: ProviderError::AuthRejected,
                detail: if expired_hint {
                    "HTTP 401 (expired token)".into()
                } else {
                    "HTTP 401".into()
                },
            },
            403 => Self {
                err: ProviderError::AuthRejected,
                detail: "HTTP 403".into(),
            },
            429 => Self {
                err: ProviderError::RateLimited,
                detail: "HTTP 429".into(),
            },
            s => Self {
                err: ProviderError::Http(s),
                detail: format!("HTTP {s}"),
            },
        }
    }

    fn from_provider(err: ProviderError) -> Self {
        let detail = err.to_string();
        Self { err, detail }
    }
}

async fn post_quota_summary(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
    email: Option<String>,
    project_id: Option<&str>,
    expired_hint: bool,
) -> Result<AntigravityUsage, CloudError> {
    let explicit_project = project_id.map(str::trim).filter(|s| !s.is_empty());
    let proj = explicit_project.unwrap_or(DEFAULT_PROJECT_ID);
    let body = serde_json::json!({ "project": proj }).to_string();

    let resp = http
        .post(url)
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .body(body)
        .timeout(Duration::from_secs(7))
        .send()
        .await
        .map_err(|e| {
            log::warn!("antigravity quota request failed: {}", e.without_url());
            CloudError::from_provider(ProviderError::Network)
        })?;
    super::note_retry_after(&resp);
    let status = resp.status().as_u16();
    // If a workspace/enterprise account rejects the default consumer project field with 400 or
    // 403, retry once read-only with `{}`.
    if status == 400 || (status == 403 && explicit_project.is_none()) {
        let retry = http
            .post(url)
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .body("{}")
            .timeout(Duration::from_secs(7))
            .send()
            .await
            .map_err(|_| CloudError::from_provider(ProviderError::Network))?;
        super::note_retry_after(&retry);
        let retry_status = retry.status().as_u16();
        if !(200..300).contains(&retry_status) {
            return Err(CloudError::from_status(retry_status, expired_hint));
        }
        let bytes = retry
            .bytes()
            .await
            .map_err(|_| CloudError::from_provider(ProviderError::Network))?;
        return parse_quota_summary(&bytes, email, Utc::now()).map_err(CloudError::from_provider);
    }
    if !(200..300).contains(&status) {
        return Err(CloudError::from_status(status, expired_hint));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|_| CloudError::from_provider(ProviderError::Network))?;
    parse_quota_summary(&bytes, email, Utc::now()).map_err(CloudError::from_provider)
}

async fn post_available_models(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
    email: Option<String>,
    project_id: Option<&str>,
    expired_hint: bool,
) -> Result<AntigravityUsage, CloudError> {
    let proj = project_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_PROJECT_ID);
    let body = serde_json::json!({ "project": proj }).to_string();
    let resp = http
        .post(url)
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .body(body)
        .timeout(Duration::from_secs(7))
        .send()
        .await
        .map_err(|_| CloudError::from_provider(ProviderError::Network))?;
    super::note_retry_after(&resp);
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(CloudError::from_status(status, expired_hint));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|_| CloudError::from_provider(ProviderError::Network))?;
    parse_fetch_available_models(&bytes, email, Utc::now()).map_err(CloudError::from_provider)
}

async fn fetch_cloud_for_cred(
    http: &reqwest::Client,
    cred: &AntigravityCredential,
    email: Option<String>,
    expired_hint: bool,
) -> Result<(AntigravityUsage, &'static str), CloudError> {
    // Prefer production (`cloudcode-pa.googleapis.com`) first: `daily-cloudcode-pa.googleapis.com`
    // is Google's separate daily/staging quota backend and can return a nearly-empty `0%`/`1%`
    // bucket while production Antigravity Desktop / `agy` holds the user's real usage.
    match post_quota_summary(
        http,
        PROD_QUOTA_URL,
        &cred.access_token,
        email.clone(),
        cred.project_id.as_deref(),
        expired_hint,
    )
    .await
    {
        Ok(mut prod) => {
            if !has_primary_gemini_group(&prod) {
                if let Ok(daily) = post_quota_summary(
                    http,
                    DAILY_QUOTA_URL,
                    &cred.access_token,
                    email,
                    cred.project_id.as_deref(),
                    expired_hint,
                )
                .await
                {
                    merge_usage(&mut prod, daily);
                    return Ok((prod, "prod+daily"));
                }
            } else if max_usage_percent(&prod) < 1.5 {
                // If prod looks untouched (< 1.5%), check daily once in case this account runs on
                // a daily/canary Antigravity build whose real usage lives on `daily-cloudcode-pa`.
                if let Ok(daily) = post_quota_summary(
                    http,
                    DAILY_QUOTA_URL,
                    &cred.access_token,
                    email,
                    cred.project_id.as_deref(),
                    expired_hint,
                )
                .await
                {
                    if max_usage_percent(&daily) >= 2.0 {
                        return Ok((daily, "daily"));
                    }
                }
            }
            Ok((prod, "prod"))
        }
        Err(e)
            if matches!(
                e.err,
                ProviderError::RateLimited | ProviderError::AuthRejected
            ) =>
        {
            Err(e)
        }
        Err(prod_err) => {
            match post_quota_summary(
                http,
                DAILY_QUOTA_URL,
                &cred.access_token,
                email.clone(),
                cred.project_id.as_deref(),
                expired_hint,
            )
            .await
            {
                Ok(daily) => Ok((daily, "daily")),
                Err(e)
                    if matches!(
                        e.err,
                        ProviderError::RateLimited | ProviderError::AuthRejected
                    ) =>
                {
                    Err(e)
                }
                Err(_) => {
                    // Final cloud fallback: per-model quotas via fetchAvailableModels (prod first).
                    for (url, tag) in [
                        (PROD_MODELS_URL, "prod-models"),
                        (DAILY_MODELS_URL, "daily-models"),
                    ] {
                        if let Ok(models) = post_available_models(
                            http,
                            url,
                            &cred.access_token,
                            email.clone(),
                            cred.project_id.as_deref(),
                            expired_hint,
                        )
                        .await
                        {
                            return Ok((models, tag));
                        }
                    }
                    Err(prod_err)
                }
            }
        }
    }
}

// ---- agy CLI fallback -----------------------------------------------------------------------

/// Strip ANSI terminal escape sequences (`ESC [ ... letter`) so `agy` colored tables parse cleanly.
pub fn strip_ansi_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for ch in chars.by_ref() {
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn push_agy_bucket(usage: &mut AntigravityUsage, group_name: Option<String>, cand: ParsedBucket) {
    let kind = cand.kind;
    if let Some(group_name) = group_name {
        let grp = match usage.groups.iter_mut().find(|g| g.name == group_name) {
            Some(existing) => existing,
            None => {
                usage.groups.push(AntigravityQuotaGroup {
                    name: group_name,
                    ..AntigravityQuotaGroup::default()
                });
                usage.groups.last_mut().expect("pushed above")
            }
        };
        match kind {
            WindowKind::FiveHour => merge_window(
                &mut grp.five_hour_percent,
                &mut grp.five_hour_resets_at,
                cand.clone(),
            ),
            WindowKind::Weekly => merge_window(
                &mut grp.weekly_percent,
                &mut grp.weekly_resets_at,
                cand.clone(),
            ),
        }
    }
    match kind {
        WindowKind::FiveHour => merge_window(
            &mut usage.five_hour_percent,
            &mut usage.five_hour_resets_at,
            cand,
        ),
        WindowKind::Weekly => {
            merge_window(&mut usage.weekly_percent, &mut usage.weekly_resets_at, cand)
        }
    }
}

fn parse_window_from_label(lower: &str) -> Option<WindowKind> {
    if lower.contains("five hour")
        || lower.contains("five-hour")
        || lower.contains("5 hour")
        || lower.contains("5-hour")
        || lower.contains(" 5h")
        || lower.starts_with("5h")
    {
        Some(WindowKind::FiveHour)
    } else if lower.contains("weekly") || lower.contains("7-day") || lower.contains("7 day") {
        Some(WindowKind::Weekly)
    } else {
        None
    }
}

/// Parse `agy --output-format json -p /usage` stdout into `AntigravityUsage`.
///
/// Supports both the structured `command.data.groups[]` envelope returned by `agy` 1.1.11+ and
/// string `response` envelopes containing TSV/table output.
pub fn parse_agy_json_output(
    output: &str,
    email: Option<String>,
    now: DateTime<Utc>,
) -> Option<AntigravityUsage> {
    let clean = strip_ansi_escapes(output);
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed: Value = serde_json::from_str(trimmed).ok().or_else(|| {
        trimmed
            .lines()
            .rev()
            .map(str::trim)
            .filter(|l| l.starts_with('{') && l.ends_with('}'))
            .find_map(|l| serde_json::from_str::<Value>(l).ok())
    })?;

    for candidate in [
        parsed.pointer("/command/data"),
        parsed.pointer("/data"),
        parsed.pointer("/result"),
        Some(&parsed),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(usage) = parse_quota_summary_value(candidate, email.clone(), now) {
            return Some(usage);
        }
    }

    for text_field in [
        parsed.pointer("/response"),
        parsed.pointer("/command/output"),
        parsed.pointer("/output"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(s) = text_field.as_str() {
            if let Some(mut usage) = parse_agy_usage_output(s) {
                if usage.email.is_none() {
                    usage.email = email;
                }
                return Some(usage);
            }
        }
    }

    None
}

/// Parse `agy -p /quota` or `agy -p /usage` stdout into `AntigravityUsage`.
///
/// Handles TSV (`Group\tLimit\t80%\t2026-...Z`), `|`-delimited tables, and line layouts (with or
/// without ANSI color codes) containing `Five Hour` / `5h` and `Weekly` remaining or used
/// percentages along with optional RFC3339 reset timestamps, preserving per-group splits.
pub fn parse_agy_usage_output(output: &str) -> Option<AntigravityUsage> {
    let clean = strip_ansi_escapes(output);
    let mut usage = AntigravityUsage::default();
    let mut found_any = false;

    for raw_line in clean.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        // Standard non-interactive `agy -p /usage` TSV format:
        // `Gemini Models\tWeekly Limit Remaining\t80%\t2026-11-24T23:28:29Z`
        // `Gemini 4 Argon\tGemini 4 Argon Weekly Limit\t100%\t2026-11-28T05:16:42Z`
        // Column 3 is always Remaining % unless explicitly marked "used" / "consumed".
        if line.contains('\t') {
            let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
            if cols.len() >= 3 {
                let label_lower = cols[1].to_ascii_lowercase();
                if let Some(kind) = parse_window_from_label(&label_lower) {
                    if let Some(pct) =
                        extract_percent_token(cols[2]).or_else(|| extract_percent_token(line))
                    {
                        let pct_lower = cols[2].to_ascii_lowercase();
                        let is_used = label_lower.contains("used")
                            || label_lower.contains("consumed")
                            || pct_lower.contains("used");
                        let used_pct = if is_used {
                            pct.clamp(0.0, 100.0)
                        } else {
                            (100.0 - pct).clamp(0.0, 100.0)
                        };
                        let resets_at = cols
                            .get(3)
                            .and_then(|c| extract_iso_timestamp(c))
                            .or_else(|| extract_iso_timestamp(line));
                        let group_name = (!cols[0].is_empty()).then(|| short_group_name(cols[0]));
                        push_agy_bucket(
                            &mut usage,
                            group_name,
                            ParsedBucket {
                                kind,
                                percent_used: used_pct,
                                resets_at,
                            },
                        );
                        found_any = true;
                        continue;
                    }
                }
            }
        }

        let lower = line.to_ascii_lowercase();
        let Some(kind) = parse_window_from_label(&lower) else {
            continue;
        };
        let Some(pct) = extract_percent_token(line) else {
            continue;
        };
        let is_remaining = lower.contains("remaining") || lower.contains("left");
        let used_pct = if is_remaining {
            (100.0 - pct).clamp(0.0, 100.0)
        } else {
            pct.clamp(0.0, 100.0)
        };
        let resets_at = extract_iso_timestamp(line);
        let group_name = if line.contains('|') {
            line.split('|')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(short_group_name)
        } else {
            model_pool_name(line).map(str::to_string)
        };
        push_agy_bucket(
            &mut usage,
            group_name,
            ParsedBucket {
                kind,
                percent_used: used_pct,
                resets_at,
            },
        );
        found_any = true;
    }

    found_any.then_some(usage)
}

fn extract_percent_token(line: &str) -> Option<f64> {
    let bytes = line.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        if b != b'%' {
            continue;
        }
        let mut start = idx;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        if start < idx {
            if let Ok(val) = line[start..idx].parse::<f64>() {
                if val.is_finite() && (0.0..=100.0).contains(&val) {
                    return Some(val);
                }
            }
        }
    }
    None
}

fn extract_iso_timestamp(line: &str) -> Option<String> {
    for tok in
        line.split(|c: char| c.is_whitespace() || c == '|' || c == ',' || c == '(' || c == ')')
    {
        let cleaned = tok.trim_matches(|c: char| c == '[' || c == ']' || c == '"' || c == '\'');
        if cleaned.len() >= 20 && cleaned.contains('T') {
            if let Ok(dt) = DateTime::parse_from_rfc3339(cleaned) {
                return Some(
                    dt.with_timezone(&Utc)
                        .to_rfc3339_opts(SecondsFormat::Millis, true),
                );
            }
        }
    }
    None
}

/// Expand Windows `%VAR%` placeholders inside `PATH` entries (handles `REG_SZ` user `Path`
/// entries like `%LOCALAPPDATA%\agy\bin` that Windows does not pre-expand for GUI apps).
pub fn expand_windows_env_vars(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push('%');
            out.push_str(after);
            return out;
        };
        let var_name = &after[..end];
        if !var_name.is_empty()
            && var_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '(' | ')' | ' '))
        {
            if let Ok(val) =
                std::env::var(var_name).or_else(|_| std::env::var(var_name.to_ascii_uppercase()))
            {
                out.push_str(&val);
            } else {
                out.push('%');
                out.push_str(var_name);
                out.push('%');
            }
        } else {
            out.push('%');
            out.push_str(var_name);
            out.push('%');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn find_agy_binary() -> Option<PathBuf> {
    if let Some(override_path) = std::env::var_os("ANTIGRAVITY_CLI_PATH") {
        let p = PathBuf::from(expand_windows_env_vars(&override_path.to_string_lossy()));
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(local_app) = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        for rel in [
            "agy/bin/agy.exe",
            "agy/bin/agy.cmd",
            "agy/agy.exe",
            "Antigravity/agy.exe",
            "Antigravity/bin/agy.exe",
            "Antigravity/bin/agy.cmd",
            "Programs/Antigravity/resources/bin/agy.exe",
            "Programs/Antigravity/bin/agy.exe",
            "Programs/Antigravity/bin/agy.cmd",
            "Programs/Antigravity IDE/resources/bin/agy.exe",
            "Programs/Antigravity IDE/bin/agy.exe",
            "Programs/Antigravity IDE/bin/agy.cmd",
            "Programs/agy/agy.exe",
            "Programs/agy/bin/agy.exe",
        ] {
            let candidate = local_app.join(rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        for rel in [
            ".local/bin/agy.exe",
            ".local/bin/agy.cmd",
            ".local/bin/agy",
            ".gemini/antigravity/bin/agy.exe",
            ".gemini/antigravity/bin/agy.cmd",
            ".gemini/antigravity/bin/agy",
            ".antigravity/bin/agy.exe",
            ".antigravity/bin/agy.cmd",
        ] {
            let candidate = home.join(rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(app_data) = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        for rel in ["npm/agy.cmd", "npm/agy.exe"] {
            let candidate = app_data.join(rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(pf) = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        for rel in [
            "Antigravity/bin/agy.exe",
            "Antigravity/bin/agy.cmd",
            "Antigravity/resources/bin/agy.exe",
            "Antigravity IDE/bin/agy.exe",
        ] {
            let candidate = pf.join(rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for raw_dir in std::env::split_paths(&path_var) {
            let dir = PathBuf::from(expand_windows_env_vars(&raw_dir.to_string_lossy()));
            for name in ["agy.exe", "agy.cmd", "agy"] {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

enum AgyAttempt {
    Ok(AntigravityUsage, &'static str),
    TimedOut,
    Failed(String),
}

async fn run_agy_command(
    bin: PathBuf,
    args: &'static [&'static str],
    json_mode: bool,
) -> AgyAttempt {
    let waited = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || {
            let mut cmd = std::process::Command::new(&bin);
            cmd.args(args)
                .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
                .current_dir(std::env::temp_dir())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            cmd.output()
        }),
    )
    .await;

    let output = match waited {
        Ok(Ok(Ok(out))) => out,
        Ok(Ok(Err(_))) => return AgyAttempt::Failed("spawn error".into()),
        Ok(Err(_)) => return AgyAttempt::Failed("task error".into()),
        Err(_) => return AgyAttempt::TimedOut,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let now = Utc::now();
    if json_mode {
        if let Some(usage) = parse_agy_json_output(&stdout, None, now) {
            return AgyAttempt::Ok(usage, "json");
        }
    }
    if let Some(usage) = parse_agy_usage_output(&stdout) {
        return AgyAttempt::Ok(usage, "text");
    }
    let combined = format!("{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    if json_mode {
        if let Some(usage) = parse_agy_json_output(&combined, None, now) {
            return AgyAttempt::Ok(usage, "json");
        }
    }
    if let Some(usage) = parse_agy_usage_output(&combined) {
        return AgyAttempt::Ok(usage, "text");
    }

    match output.status.code() {
        Some(0) => AgyAttempt::Failed("unparseable output".into()),
        Some(code) => AgyAttempt::Failed(format!("exit {code}")),
        None => AgyAttempt::Failed("terminated".into()),
    }
}

async fn fetch_via_agy_cli(
    bin: Option<PathBuf>,
    email: Option<String>,
) -> Result<(AntigravityUsage, &'static str), String> {
    let Some(bin) = bin else {
        return Err("not installed".into());
    };

    // Prefer structured JSON mode (`-p /usage` last, auto-update disabled).
    match run_agy_command(
        bin.clone(),
        &[
            "--output-format",
            "json",
            "--print-timeout",
            "25s",
            "-p",
            "/usage",
        ],
        true,
    )
    .await
    {
        AgyAttempt::Ok(mut usage, mode) => {
            if usage.email.is_none() {
                usage.email = email;
            }
            return Ok((usage, mode));
        }
        AgyAttempt::TimedOut => return Err("timeout".into()),
        AgyAttempt::Failed(_) => {}
    }

    // Fallback to plain-text TSV mode for older `agy` versions without `--output-format json`.
    match run_agy_command(bin, &["--print-timeout", "25s", "-p", "/usage"], false).await {
        AgyAttempt::Ok(mut usage, mode) => {
            if usage.email.is_none() {
                usage.email = email;
            }
            Ok((usage, mode))
        }
        AgyAttempt::TimedOut => Err("timeout".into()),
        AgyAttempt::Failed(reason) => {
            log::warn!("antigravity: agy CLI fallback failed ({reason})");
            Err(reason)
        }
    }
}

// ---- top-level fetch orchestration ----------------------------------------------------------

async fn fetch(
    http: reqwest::Client,
    creds: Vec<AntigravityCredential>,
    local_servers: Vec<LocalAntigravityServer>,
    agy_bin: Option<PathBuf>,
    saw_v234_server: bool,
    mut diag_lines: Vec<String>,
) -> Result<AntigravityUsage, FetchError<AntigravityUsage>> {
    let fallback_email = creds.iter().find_map(|c| c.email.clone());
    let has_agy_bin = agy_bin.is_some();
    let mut local_partial: Option<AntigravityUsage> = None;
    let mut trace: Vec<String> = Vec::new();
    let now = Utc::now();

    let finish = |lines: &mut Vec<String>,
                  trace: &[String],
                  outcome: &str,
                  usage: Option<&AntigravityUsage>,
                  issue: Option<AntigravitySetupIssue>| {
        if !trace.is_empty() {
            lines.push(format!("Attempts: {}", trace.join(" | ")));
        }
        if let Some(u) = usage {
            lines.push(format_usage_groups_diag(u));
        }
        lines.push(format!("Outcome: {outcome}"));
        save_diag_report(lines);
        set_setup_issue(issue);
    };

    // 1. Prefer live local language_server / agy Connect-RPC when running — always holds a fresh
    //    in-memory session even after on-disk `ya29.*` access tokens expire.
    if !local_servers.is_empty() {
        if let Some(local_client) = build_local_rpc_client() {
            for srv in &local_servers {
                if let Some(usage) =
                    fetch_via_local_server(&local_client, srv, fallback_email.clone(), &mut trace)
                        .await
                {
                    if usage.weekly_percent.is_some() {
                        finish(
                            &mut diag_lines,
                            &trace,
                            "ok (local server)",
                            Some(&usage),
                            None,
                        );
                        return Ok(usage);
                    }
                    // Local GetUserStatus succeeded (5-hour live pools) — keep it and try cloud/CLI
                    // once to enrich weekly limits.
                    local_partial = Some(usage);
                    break;
                }
            }
        }
    }

    // Partition discovered tokens into unexpired first, then known-expired as a last-resort
    // fallback (avoids burning round trips on expired tokens before trying `agy` CLI).
    let mut fresh_creds: Vec<(usize, &AntigravityCredential)> = Vec::new();
    let mut expired_creds: Vec<(usize, &AntigravityCredential)> = Vec::new();
    for (idx, cred) in creds.iter().enumerate() {
        if cred.is_expired(now) {
            expired_creds.push((idx + 1, cred));
        } else {
            fresh_creds.push((idx + 1, cred));
        }
    }

    let mut last_err = ProviderError::AuthRejected;

    // 2. Cloud API across unexpired read-only tokens (state.vscdb, Credential Manager, files).
    for (cred_num, cred) in fresh_creds {
        let email = cred
            .email
            .clone()
            .or_else(|| local_partial.as_ref().and_then(|u| u.email.clone()))
            .or_else(|| fallback_email.clone());
        match fetch_cloud_for_cred(&http, cred, email, false).await {
            Ok((mut cloud_usage, source)) => {
                trace.push(format!("cloud cred#{cred_num} ({source}) -> ok"));
                // Safety net: if a cloud reply looks nearly empty (< 2% across all pools) while
                // `agy` CLI is installed, cross-check `agy` once so a mismatched default project
                // or backend bucket can never snap live usage down to 0%/1%.
                let mut outcome_label = format!("ok (cloud:{source})");
                if max_usage_percent(&cloud_usage) < 2.0 && agy_bin.is_some() {
                    match fetch_via_agy_cli(agy_bin.clone(), fallback_email.clone()).await {
                        Ok((agy_usage, mode)) => {
                            if max_usage_percent(&agy_usage) > max_usage_percent(&cloud_usage) + 1.0
                            {
                                trace.push(format!(
                                    "agy CLI -> ok ({mode}; overrode near-empty cloud)"
                                ));
                                cloud_usage = agy_usage;
                                outcome_label = "ok (agy CLI)".to_string();
                            } else {
                                trace.push(format!("agy CLI -> ok ({mode}; confirmed cloud)"));
                            }
                        }
                        Err(reason) => {
                            trace.push(format!("agy CLI verify -> {reason}"));
                        }
                    }
                }
                if let Some(local) = local_partial {
                    let mut combined = local;
                    merge_usage(&mut combined, cloud_usage);
                    finish(
                        &mut diag_lines,
                        &trace,
                        &format!("ok (local + {outcome_label})"),
                        Some(&combined),
                        None,
                    );
                    return Ok(combined);
                }
                if cloud_usage.email.is_none() {
                    cloud_usage.email = fallback_email;
                }
                finish(
                    &mut diag_lines,
                    &trace,
                    &outcome_label,
                    Some(&cloud_usage),
                    None,
                );
                return Ok(cloud_usage);
            }
            Err(e) if e.err == ProviderError::RateLimited => {
                trace.push(format!("cloud cred#{cred_num} -> rate-limited"));
                if let Some(local) = local_partial {
                    finish(
                        &mut diag_lines,
                        &trace,
                        "ok (local 5h; cloud rate-limited)",
                        Some(&local),
                        None,
                    );
                    return Ok(local);
                }
                finish(&mut diag_lines, &trace, "failed (rate-limited)", None, None);
                return Err(ProviderError::RateLimited.into());
            }
            Err(e) => {
                trace.push(format!("cloud cred#{cred_num} -> {}", e.detail));
                last_err = e.err;
            }
        }
    }

    // 3. Official `agy` CLI (`--output-format json -p /usage`) — handles Antigravity 2.34+ where
    //    `language_server.exe` moved its CSRF token to stdin and cached desktop access tokens expire.
    let mut agy_failed_signin = false;
    match fetch_via_agy_cli(agy_bin, fallback_email.clone()).await {
        Ok((usage, mode)) => {
            trace.push(format!("agy CLI -> ok ({mode})"));
            if let Some(local) = local_partial {
                let mut combined = local;
                merge_usage(&mut combined, usage);
                finish(
                    &mut diag_lines,
                    &trace,
                    "ok (local + agy CLI)",
                    Some(&combined),
                    None,
                );
                return Ok(combined);
            }
            finish(&mut diag_lines, &trace, "ok (agy CLI)", Some(&usage), None);
            return Ok(usage);
        }
        Err(reason) => {
            if has_agy_bin && reason != "timeout" {
                agy_failed_signin = true;
            }
            trace.push(format!("agy CLI -> {reason}"));
        }
    }

    // 4. Last-resort cloud probe for tokens whose local expiry timestamp was in the past (covers
    //    clock skew and records exact HTTP 401/403 status in diagnostics).
    for (cred_num, cred) in expired_creds {
        let email = cred
            .email
            .clone()
            .or_else(|| local_partial.as_ref().and_then(|u| u.email.clone()))
            .or_else(|| fallback_email.clone());
        match fetch_cloud_for_cred(&http, cred, email, true).await {
            Ok((mut cloud_usage, source)) => {
                trace.push(format!("cloud cred#{cred_num} ({source}) -> ok"));
                if let Some(local) = local_partial {
                    let mut combined = local;
                    merge_usage(&mut combined, cloud_usage);
                    finish(
                        &mut diag_lines,
                        &trace,
                        &format!("ok (local + cloud:{source})"),
                        Some(&combined),
                        None,
                    );
                    return Ok(combined);
                }
                if cloud_usage.email.is_none() {
                    cloud_usage.email = fallback_email;
                }
                finish(
                    &mut diag_lines,
                    &trace,
                    &format!("ok (cloud:{source})"),
                    Some(&cloud_usage),
                    None,
                );
                return Ok(cloud_usage);
            }
            Err(e) if e.err == ProviderError::RateLimited => {
                trace.push(format!("cloud cred#{cred_num} -> rate-limited"));
                if let Some(local) = local_partial {
                    finish(
                        &mut diag_lines,
                        &trace,
                        "ok (local 5h; cloud rate-limited)",
                        Some(&local),
                        None,
                    );
                    return Ok(local);
                }
                finish(&mut diag_lines, &trace, "failed (rate-limited)", None, None);
                return Err(ProviderError::RateLimited.into());
            }
            Err(e) => {
                trace.push(format!("cloud cred#{cred_num} -> {}", e.detail));
                last_err = e.err;
            }
        }
    }

    // If local Connect-RPC already returned live 5-hour quota, use it rather than surfacing a
    // stale on-disk token's `AuthRejected`.
    if let Some(local) = local_partial {
        finish(
            &mut diag_lines,
            &trace,
            "ok (local 5h; cloud/CLI unavailable)",
            Some(&local),
            None,
        );
        return Ok(local);
    }

    let issue = if !has_agy_bin && (saw_v234_server || last_err == ProviderError::AuthRejected) {
        Some(AntigravitySetupIssue::NeedsAgyCli)
    } else if agy_failed_signin {
        Some(AntigravitySetupIssue::NeedsAgySignIn)
    } else {
        None
    };

    finish(
        &mut diag_lines,
        &trace,
        &format!("failed ({last_err})"),
        None,
        issue,
    );
    Err(last_err.into())
}

/// Rows for Antigravity (`TrackedAccount<AntigravityUsage>` each).
pub async fn accounts(core: &Core) -> Vec<Value> {
    let (creds, local_servers, agy_bin, saw_v234_server, mut diag_lines) =
        tokio::task::spawn_blocking(|| {
            let (local_servers, server_diag, saw_v234_server) = discover_local_servers_with_diag();
            let (creds, cred_diags) = read_credentials_with_diag();
            let agy_bin = if !super::codex::real_sign_ins() {
                None
            } else {
                find_agy_binary()
            };
            let mut diag_lines = Vec::with_capacity(2 + cred_diags.len());
            diag_lines.push(server_diag);
            diag_lines.extend(cred_diags);
            diag_lines.push(format!(
                "agy CLI: {}",
                if agy_bin.is_some() {
                    "found"
                } else {
                    "not found (checked %LOCALAPPDATA%\\agy\\bin, %LOCALAPPDATA%\\Antigravity, PATH)"
                }
            ));
            (creds, local_servers, agy_bin, saw_v234_server, diag_lines)
        })
        .await
        .unwrap_or((
            Vec::new(),
            Vec::new(),
            None,
            false,
            vec!["Discovery task panicked".to_string()],
        ));

    let detected =
        !creds.is_empty() || !local_servers.is_empty() || agy_bin.is_some() || saw_v234_server;
    let diag_for_fetch = diag_lines.clone();
    let live: Vec<LiveAccount<'_, AntigravityUsage>> = if detected {
        let email = creds.iter().find_map(|c| c.email.clone());
        let label = email.clone().or_else(|| Some("Antigravity".into()));
        let id = email
            .as_deref()
            .unwrap_or("antigravity")
            .trim()
            .to_lowercase();
        let http = core.http.clone();
        vec![LiveAccount {
            id: id.clone(),
            label,
            active: true,
            load: Box::pin(async move {
                CACHE
                    .get(&core.cache, &id, move |_| {
                        fetch(
                            http,
                            creds,
                            local_servers,
                            agy_bin,
                            saw_v234_server,
                            diag_for_fetch,
                        )
                    })
                    .await
            }),
            label_from: Some(|u| u.email.clone()),
        }]
    } else {
        Vec::new()
    };

    let rows: Vec<TrackedAccount<AntigravityUsage>> =
        collect_accounts(&core.seen, &core.cache, PROVIDER, SECTION, live).await;
    if !detected {
        let reason =
            "not found: no running language_server/agy, no state.vscdb/keyring/file tokens, no agy CLI";
        diag_lines.push(format!("Outcome: {reason}"));
        save_diag_report(&diag_lines);
        set_setup_issue(None);
        super::record_provider_status(PROVIDER, reason, false);
    } else {
        let (ok, summary) = super::summarize_tracked_rows(&rows);
        if last_diag_report().is_none() {
            diag_lines.push(format!("Outcome: {summary} (served from cache)"));
            save_diag_report(&diag_lines);
        }
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
    use serde_json::json;

    fn utc(iso: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(iso)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn encode_varint(mut v: usize, buf: &mut Vec<u8>) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            buf.push(b);
            if v == 0 {
                break;
            }
        }
    }

    fn encode_ld_field(field_num: u64, payload: &[u8], buf: &mut Vec<u8>) {
        encode_varint(((field_num << 3) | 2) as usize, buf);
        encode_varint(payload.len(), buf);
        buf.extend_from_slice(payload);
    }

    #[test]
    fn credential_and_server_debug_redact_secrets() {
        let cred = AntigravityCredential {
            access_token: "ya29.super-secret-oauth-token".into(),
            email: Some("dev@example.com".into()),
            project_id: None,
            expires_at: None,
        };
        let dbg = format!("{cred:?}");
        assert!(!dbg.contains("ya29.super-secret-oauth-token"));
        assert!(dbg.contains("<redacted>"));
        assert!(dbg.contains("dev@example.com"));

        let srv = LocalAntigravityServer {
            pid: 42,
            csrf_token: Some("secret-csrf-uuid".into()),
            extension_server_port: Some(19116),
            extension_server_csrf_token: Some("ext-secret-uuid".into()),
            listen_ports: vec![61278],
            is_cli: false,
        };
        let sdbg = format!("{srv:?}");
        assert!(!sdbg.contains("secret-csrf-uuid"));
        assert!(!sdbg.contains("ext-secret-uuid"));
        assert!(sdbg.contains("<redacted>"));
    }

    #[test]
    fn decode_credential_blob_handles_even_length_utf8_and_utf16le() {
        // Even-length UTF-8 go-keyring blob (previously mis-decoded as UTF-16LE by keyring::get_password).
        let raw_json = json!({
            "email": "user@example.com",
            "token": { "access_token": "ya29.even_length_token_1234567890" }
        })
        .to_string();
        let mut payload = format!("go-keyring-base64:{}", B64.encode(&raw_json));
        if payload.len() % 2 == 1 {
            payload.push(' ');
        }
        assert_eq!(payload.len() % 2, 0);
        let decoded = decode_credential_blob(payload.as_bytes()).expect("decodes even UTF-8");
        let cred = parse_credential_json(&decoded).expect("parses go-keyring JSON");
        assert_eq!(cred.access_token, "ya29.even_length_token_1234567890");

        // UTF-16LE credential blob (e.g. written by Windows Credential Manager UI / PowerShell).
        let utf16: Vec<u8> = raw_json.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let decoded16 = decode_credential_blob(&utf16).expect("decodes UTF-16LE");
        let cred16 = parse_credential_json(&decoded16).expect("parses UTF-16LE JSON");
        assert_eq!(cred16.access_token, "ya29.even_length_token_1234567890");
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
    fn protobuf_decoder_handles_ascii_alphanumeric_varint_lengths_and_nested_messages() {
        // Construct an inner base64 payload whose exact byte length is 80 (0x50 = ASCII 'P'!).
        // A naive ASCII-run scanner would prepend 'P' to the base64 run and corrupt it;
        // the real protobuf wire-format walker decodes it cleanly.
        let tok = "ya29.a0AfB_byC1234567890_EXACT_ASCII_LEN_TOKEN";
        let mut inner_proto = Vec::new();
        encode_ld_field(1, tok.as_bytes(), &mut inner_proto);
        encode_ld_field(2, b"1//refresh_tok", &mut inner_proto);
        let inner_b64 = B64.encode(&inner_proto);
        assert!(
            inner_b64.len() >= 48 && inner_b64.len() <= 122,
            "length {} must fall in ASCII alphanumeric varint range",
            inner_b64.len()
        );
        let mut sentinel_val = Vec::new();
        encode_ld_field(1, inner_b64.as_bytes(), &mut sentinel_val);
        let mut outer_proto = Vec::new();
        encode_ld_field(1, b"oauthTokenInfoSentinelKey", &mut outer_proto);
        encode_ld_field(2, &sentinel_val, &mut outer_proto);
        let outer_b64 = B64.encode(&outer_proto);

        assert_eq!(
            extract_oauth_token_from_unified_sync(&outer_b64).as_deref(),
            Some(tok)
        );
    }

    #[test]
    fn parse_language_server_cmdline_identifies_antigravity_and_ignores_windsurf() {
        let ag_cmd = r#"D:\Antigravity\resources\app\extensions\antigravity\bin\language_server_windows_x64.exe --enable_lsp --extension_server_port 19116 --csrf_token e8d42e20-02b4-4ec8-9156-6ce5d35d0f01 --random_port --app_data_dir antigravity"#;
        let parsed = parse_language_server_cmdline(
            "language_server_windows_x64.exe",
            ag_cmd,
            Some(r"D:\Antigravity\resources\app\extensions\antigravity\bin\language_server_windows_x64.exe"),
        )
        .expect("detects Antigravity language_server");
        assert!(!parsed.is_cli);
        assert_eq!(
            parsed.csrf_token.as_deref(),
            Some("e8d42e20-02b4-4ec8-9156-6ce5d35d0f01")
        );
        assert_eq!(parsed.extension_server_port, Some(19116));

        // Windsurf language server must be ignored.
        let ws_cmd = r#"C:\Windsurf\language_server_windows_x64.exe --csrf_token abc --app_data_dir windsurf"#;
        assert_eq!(
            parse_language_server_cmdline("language_server_windows_x64.exe", ws_cmd, None),
            None
        );

        // `agy.exe` CLI server requires no CSRF token.
        let cli = parse_language_server_cmdline("agy.exe", "agy.exe", None)
            .expect("detects agy CLI server");
        assert!(cli.is_cli);
        assert_eq!(cli.csrf_token, None);
    }

    #[test]
    fn parse_local_user_status_and_fetch_available_models_group_pools() {
        let now = utc("2026-11-24T10:00:00Z");
        let status_json = json!({
            "userStatus": {
                "email": "ide@example.com",
                "cascadeModelConfigData": {
                    "clientModelConfigs": [
                        {
                            "label": "Gemini 3.1 Pro (High)",
                            "quotaInfo": {
                                "remainingFraction": 0.80,
                                "resetTime": "2026-11-24T13:00:00Z"
                            }
                        },
                        {
                            "label": "Claude Sonnet 4.6 (Thinking)",
                            "quotaInfo": {
                                "remainingFraction": 0.45,
                                "resetTime": "2026-11-24T12:30:00Z"
                            }
                        },
                        {
                            "label": "Ineligible Free Model",
                            "quotaInfo": {}
                        }
                    ]
                }
            }
        });
        let usage = parse_local_user_status(&serde_json::to_vec(&status_json).unwrap(), None, now)
            .expect("GetUserStatus parses");
        assert_eq!(usage.email.as_deref(), Some("ide@example.com"));
        assert!((usage.five_hour_percent.unwrap() - 55.0).abs() < 1e-6);
        assert_eq!(usage.groups.len(), 2);
        assert_eq!(usage.groups[0].name, "Gemini");
        assert!((usage.groups[0].five_hour_percent.unwrap() - 20.0).abs() < 1e-6);
        assert_eq!(usage.groups[1].name, "Claude & GPT");
        assert!((usage.groups[1].five_hour_percent.unwrap() - 55.0).abs() < 1e-6);

        let models_json = json!({
            "models": {
                "gemini-3-pro": {
                    "displayName": "Gemini 3 Pro",
                    "quotaInfo": { "remainingFraction": 0.75, "resetTime": "2026-11-24T14:00:00Z" }
                },
                "claude-opus-4-6": {
                    "displayName": "Claude Opus 4.6",
                    "quotaInfo": {}
                }
            }
        });
        let m_usage = parse_fetch_available_models(
            &serde_json::to_vec(&models_json).unwrap(),
            Some("cloud@example.com".into()),
            now,
        )
        .expect("fetchAvailableModels parses");
        assert_eq!(m_usage.email.as_deref(), Some("cloud@example.com"));
        assert!((m_usage.five_hour_percent.unwrap() - 25.0).abs() < 1e-6);
        assert_eq!(m_usage.groups.len(), 1);
        assert_eq!(m_usage.groups[0].name, "Gemini");
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

    #[test]
    fn cli_dir_and_agy_usage_output_parse_with_ansi_escapes() {
        let dir =
            std::env::temp_dir().join(format!("usagedeck-antigravity-cli-{}", std::process::id()));
        let cli_dir = dir.join(".gemini").join("antigravity-cli");
        std::fs::create_dir_all(&cli_dir).unwrap();
        let token = json!({
            "access_token": "cli-tok-123",
            "email": "dev@example.com"
        });
        std::fs::write(cli_dir.join("antigravity-oauth-token"), token.to_string()).unwrap();
        let cred = read_file_credential_in(&dir).expect("reads antigravity-cli token");
        assert_eq!(cred.access_token, "cli-tok-123");
        assert_eq!(cred.email.as_deref(), Some("dev@example.com"));
        let _ = std::fs::remove_dir_all(&dir);

        let stdout = "\
Model Group          | Limit                      | Remaining | Resets
\x1b[32mGemini Models\x1b[0m        | Five Hour Limit Remaining  | \x1b[1m82%\x1b[0m       | 2026-11-23T05:00:00Z
Gemini Models        | Weekly Limit Remaining     | 65%       | 2026-11-28T00:00:00Z
Claude and GPT       | Five Hour Limit Remaining  | 40%       | 2026-11-23T04:30:00Z
";
        let parsed = parse_agy_usage_output(stdout).expect("parses agy /usage output");
        assert_eq!(parsed.five_hour_percent, Some(60.0));
        assert_eq!(
            parsed.five_hour_resets_at.as_deref(),
            Some("2026-11-23T04:30:00.000Z")
        );
        assert_eq!(parsed.weekly_percent, Some(35.0));
        assert_eq!(
            parsed.weekly_resets_at.as_deref(),
            Some("2026-11-28T00:00:00.000Z")
        );
        assert_eq!(parsed.groups.len(), 2);
        assert_eq!(parsed.groups[0].name, "Gemini");
        assert_eq!(parsed.groups[1].name, "Claude & GPT");
    }

    #[test]
    fn ide_state_db_returns_all_tokens_including_legacy_and_auth_status() {
        // Simulate Antigravity IDE's double-base64 protobuf value in antigravityUnifiedStateSync.oauthToken
        // PLUS a fallback token in antigravityAuthStatus and legacy jetskiStateSync.
        let tok = "ya29.a0AfB_byC1234567890_DESKTOP_IDE_TOKEN";
        let mut inner_proto = Vec::new();
        encode_ld_field(1, tok.as_bytes(), &mut inner_proto);
        encode_ld_field(2, b"1//refresh_tok_123", &mut inner_proto);
        let inner_b64 = B64.encode(&inner_proto);
        let mut outer_proto = Vec::new();
        encode_ld_field(1, b"oauthTokenInfoSentinelKey", &mut outer_proto);
        encode_ld_field(2, inner_b64.as_bytes(), &mut outer_proto);
        let outer_b64 = B64.encode(&outer_proto);
        assert_eq!(
            extract_oauth_token_from_unified_sync(&outer_b64).as_deref(),
            Some(tok)
        );

        let dir = std::env::temp_dir().join(format!(
            "usagedeck-antigravity-vscdb-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("state.vscdb");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                [
                    "antigravityAuthStatus",
                    r#"{"email":"ide-user@example.com","apiKey":"ya29.older_status_token_1234567890"}"#,
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                ["antigravityUnifiedStateSync.oauthToken", outer_b64.as_str()],
            )
            .unwrap();
        }
        let all = read_ide_state_db_all(&db);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].access_token, tok);
        assert_eq!(all[0].email.as_deref(), Some("ide-user@example.com"));
        assert_eq!(all[1].access_token, "ya29.older_status_token_1234567890");
        let _ = std::fs::remove_dir_all(&dir);

        // Nested `remaining.remainingFraction` + separate Gemini and Claude & GPT groups.
        let now = utc("2026-10-21T10:00:00Z");
        let nested = json!({
            "groups": [
                {
                    "displayName": "Gemini Models",
                    "buckets": [
                        {
                            "bucketId": "gemini-5h",
                            "window": "5h",
                            "remaining": { "remainingFraction": 0.90, "resetTime": "2026-10-21T14:00:00Z" }
                        }
                    ]
                },
                {
                    "displayName": "Claude and GPT models",
                    "buckets": [
                        {
                            "bucketId": "3p-5h",
                            "window": "5h",
                            "remaining": { "remainingFraction": 0.0, "resetTime": "2026-10-21T13:00:00Z" }
                        }
                    ]
                }
            ]
        });
        let usage = parse_quota_summary(&serde_json::to_vec(&nested).unwrap(), None, now)
            .expect("nested summary parses");
        assert_eq!(usage.five_hour_percent, Some(100.0));
        assert_eq!(usage.groups.len(), 2);
        assert_eq!(usage.groups[0].name, "Gemini");
        assert!((usage.groups[0].five_hour_percent.unwrap() - 10.0).abs() < 1e-6);
        assert_eq!(usage.groups[1].name, "Claude & GPT");
        assert!((usage.groups[1].five_hour_percent.unwrap() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn agy_json_and_tsv_outputs_parse_with_argon_and_remaining_inversion() {
        let now = utc("2026-11-24T10:00:00Z");

        // 1. Structured JSON envelope from `agy --output-format json -p /usage`.
        let json_out = json!({
            "status": "SUCCESS",
            "command": {
                "name": "usage",
                "data": {
                    "groups": [
                        {
                            "displayName": "Gemini Models",
                            "buckets": [
                                {
                                    "window": "weekly",
                                    "remaining_fraction": 0.80,
                                    "reset_time": "2026-11-28T23:28:29Z"
                                },
                                {
                                    "window": "5h",
                                    "remaining_fraction": 0.81,
                                    "reset_time": "2026-11-24T14:18:33Z"
                                }
                            ]
                        },
                        {
                            "displayName": "Claude and GPT models",
                            "buckets": [
                                {
                                    "window": "weekly",
                                    "remaining_fraction": 1.0,
                                    "reset_time": "2026-12-01T10:00:05Z"
                                }
                            ]
                        },
                        {
                            "displayName": "Gemini 4 Argon",
                            "buckets": [
                                {
                                    "window": "weekly",
                                    "remaining_fraction": 1.0,
                                    "reset_time": "2026-12-01T10:00:05Z"
                                }
                            ]
                        }
                    ]
                }
            }
        })
        .to_string();

        let parsed = parse_agy_json_output(&json_out, Some("dev@example.com".into()), now)
            .expect("parses structured agy JSON");
        assert_eq!(parsed.email.as_deref(), Some("dev@example.com"));
        assert!((parsed.five_hour_percent.unwrap() - 19.0).abs() < 1e-6);
        assert!((parsed.weekly_percent.unwrap() - 20.0).abs() < 1e-6);
        assert_eq!(parsed.groups.len(), 3);
        assert_eq!(parsed.groups[2].name, "Gemini 4 Argon");
        assert!((parsed.groups[2].weekly_percent.unwrap() - 0.0).abs() < 1e-6);

        // 2. Real TSV output from `agy -p /usage` (note Argon row omits "Remaining" word, yet
        //    column 3 is still remaining % and must invert to 0% used, not 100% used!).
        let tsv = "\
Gemini Models\tWeekly Limit Remaining\t80%\t2026-11-28T23:28:29Z
Gemini Models\tFive Hour Limit Remaining\t81%\t2026-11-24T14:18:33Z
Claude and GPT models\tWeekly Limit Remaining\t100%\t2026-12-01T05:16:42Z
Claude and GPT models\tFive Hour Limit Remaining\t100%\t2026-11-24T15:16:42Z
Gemini 4 Argon\tGemini 4 Argon Weekly Limit\t100%\t2026-12-01T05:16:42Z
";
        let from_tsv = parse_agy_usage_output(tsv).expect("parses agy TSV");
        assert!((from_tsv.five_hour_percent.unwrap() - 19.0).abs() < 1e-6);
        assert!((from_tsv.weekly_percent.unwrap() - 20.0).abs() < 1e-6);
        let argon = from_tsv
            .groups
            .iter()
            .find(|g| g.name == "Gemini 4 Argon")
            .expect("includes Argon group");
        assert!((argon.weekly_percent.unwrap() - 0.0).abs() < 1e-6);

        // 3. JSON envelope with plain TSV in `response`.
        let wrapped = json!({ "status": "SUCCESS", "response": tsv }).to_string();
        let from_wrapped =
            parse_agy_json_output(&wrapped, None, now).expect("parses wrapped response TSV");
        assert!((from_wrapped.weekly_percent.unwrap() - 20.0).abs() < 1e-6);
    }

    #[test]
    fn v234_stdin_auth_detection_flag_variants_and_secret_free_flag_names() {
        let v234_cmd = r#"C:\Users\me\AppData\Local\Programs\Antigravity\resources\bin\language_server.exe --standalone --override_ide_name antigravity --https_server_port 0 --app_data_dir antigravity --host_bridge_url=http://127.0.0.1:9999 --read_host_bridge_token_from_stdin"#;
        assert_eq!(
            parse_language_server_cmdline("language_server.exe", v234_cmd, None),
            None
        );
        let reason = classify_skipped_language_server(Some(v234_cmd), None);
        assert!(reason.contains("v2.34+"));
        assert!(reason.contains("agy CLI"));

        // Single-dash and hyphenated CSRF flags are accepted.
        let single_dash = r#"C:\Antigravity\language_server.exe -csrf-token 11111111-2222-3333-4444-555555555555 -app-data-dir antigravity"#;
        let parsed = parse_language_server_cmdline("language_server.exe", single_dash, None)
            .expect("accepts single-dash/hyphenated CSRF flags");
        assert_eq!(
            parsed.csrf_token.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );

        // Flag-name extractor must NEVER include flag values or secrets.
        let with_secret = r#"language_server.exe --csrf_token=super-secret-uuid-999 --app_data_dir antigravity --secret_arg my-secret-val"#;
        let names = extract_flag_names(with_secret);
        assert_eq!(
            names,
            vec!["--csrf_token", "--app_data_dir", "--secret_arg"]
        );
        assert!(!names.iter().any(|n| n.contains("super-secret")));
        assert!(!names.iter().any(|n| n.contains("my-secret-val")));
    }

    #[test]
    fn credential_expiry_and_windows_env_var_expansion() {
        let now = utc("2026-11-24T12:00:00Z");
        let expired_json = json!({
            "email": "user@example.com",
            "token": {
                "access_token": "ya29.old_token",
                "expiry": "2026-11-24T10:00:00Z"
            }
        })
        .to_string();
        let cred = parse_credential_json(&expired_json).expect("parses expired cred");
        assert!(cred.is_expired(now));

        let fresh_ms = (now + ChronoDuration::minutes(30)).timestamp_millis();
        let fresh_json = json!({
            "access_token": "ya29.fresh_token",
            "expiry_date": fresh_ms
        })
        .to_string();
        let cred_fresh = parse_credential_json(&fresh_json).expect("parses epoch-ms expiry");
        assert!(!cred_fresh.is_expired(now));

        // Env expansion preserves unknown vars and expands known ones.
        let path_val = std::env::var("PATH").unwrap_or_default();
        if !path_val.is_empty() {
            assert_eq!(
                expand_windows_env_vars("%PATH%/agy/bin"),
                format!("{path_val}/agy/bin")
            );
        }
        assert_eq!(
            expand_windows_env_vars("%USAGEDECK_NONEXISTENT_VAR_XYZ%\\agy\\bin"),
            "%USAGEDECK_NONEXISTENT_VAR_XYZ%\\agy\\bin"
        );
    }

    #[test]
    fn primary_gemini_and_group_diag_helpers() {
        let argon_only = AntigravityUsage {
            five_hour_percent: Some(1.0),
            weekly_percent: Some(0.0),
            groups: vec![AntigravityQuotaGroup {
                name: "Gemini 4 Argon".to_string(),
                five_hour_percent: Some(1.0),
                weekly_percent: Some(0.0),
                ..AntigravityQuotaGroup::default()
            }],
            ..AntigravityUsage::default()
        };
        assert!(!has_primary_gemini_group(&argon_only));
        assert!((max_usage_percent(&argon_only) - 1.0).abs() < 1e-6);

        let full = AntigravityUsage {
            five_hour_percent: Some(100.0),
            weekly_percent: Some(93.0),
            groups: vec![
                AntigravityQuotaGroup {
                    name: "Gemini".to_string(),
                    five_hour_percent: Some(38.0),
                    weekly_percent: Some(93.0),
                    ..AntigravityQuotaGroup::default()
                },
                AntigravityQuotaGroup {
                    name: "Claude & GPT".to_string(),
                    five_hour_percent: Some(100.0),
                    weekly_percent: Some(50.0),
                    ..AntigravityQuotaGroup::default()
                },
            ],
            ..AntigravityUsage::default()
        };
        assert!(has_primary_gemini_group(&full));
        assert!((max_usage_percent(&full) - 100.0).abs() < 1e-6);
        assert_eq!(
            format_usage_groups_diag(&full),
            "Groups: Gemini (5h 38%, weekly 93%) | Claude & GPT (5h 100%, weekly 50%)"
        );
    }
}
