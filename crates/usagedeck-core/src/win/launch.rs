//! Launch or focus a provider's desktop app (classic `.exe` or packaged Windows MSIX / Store app),
//! falling back to its HTTPS web dashboard, plus Task Manager for the System strip slot.
//!
//! All local executable paths are checked with `is_safe_local_path` (rejecting UNC paths and
//! `..` traversals), MSIX AppUserModelIDs are strictly character-validated, and web fallbacks
//! only accept hardcoded `https://` URLs.

use crate::providers::cursor_signin::{
    exe_candidates as cursor_exe_candidates, is_safe_local_path, is_safe_local_path_str,
    UninstallEntry,
};
use crate::view::ProviderKind;
use crate::win::uninstall::{uninstall_entries, wide};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows::core::{w, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow,
    GetWindowLongW, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL,
    WS_EX_TOOLWINDOW,
};

const APPMODEL_PACKAGES_KEY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

/// Result of launching or focusing a provider app (or web dashboard fallback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchOutcome {
    Focused(&'static str),
    Launched(&'static str),
    OpenedWeb(&'static str),
}

/// Official HTTPS web dashboard fallback when a provider's desktop app is not installed locally.
pub fn web_dashboard_url(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "https://claude.ai/settings/usage",
        ProviderKind::Cursor => "https://cursor.com/dashboard",
        ProviderKind::Codex => "https://chatgpt.com/codex",
        ProviderKind::Grok => "https://grok.com",
        ProviderKind::OpenCode => "https://opencode.ai",
        ProviderKind::Antigravity => "https://antigravity.google",
    }
}

/// Lowercase executable filenames used to find an already-running desktop window for each provider.
pub fn app_exe_names(provider: ProviderKind) -> &'static [&'static str] {
    match provider {
        ProviderKind::Claude => &["claude.exe"],
        ProviderKind::Cursor => &["cursor.exe"],
        // OpenAI unified the Windows Codex desktop app (`OpenAI.Codex_*`) into `ChatGPT.exe`.
        ProviderKind::Codex => &["codex.exe", "chatgpt.exe"],
        ProviderKind::Grok => &["grok bot.exe", "grokbot.exe", "grok.exe"],
        ProviderKind::OpenCode => &["opencode.exe", "opencode-desktop.exe"],
        ProviderKind::Antigravity => &["antigravity.exe", "antigravity ide.exe"],
    }
}

/// MSIX / Microsoft Store package name prefixes (in priority order) for packaged desktop apps.
pub fn msix_package_prefixes(provider: ProviderKind) -> &'static [&'static str] {
    match provider {
        ProviderKind::Codex => &[
            "OpenAI.Codex_",
            "OpenAI.ChatGPT-Desktop_",
            "OpenAI.ChatGPT_",
        ],
        ProviderKind::Claude => &["Claude_"],
        _ => &[],
    }
}

/// Derive a Windows MSIX Package Family Name (`<Name>_<PublisherId>`) from a Package Full Name
/// (`<Name>_<Version>_<Arch>_<ResourceId>_<PublisherId>`).
pub fn package_family_name_from_full_name(full_name: &str) -> Option<String> {
    let trimmed = full_name.trim();
    let (name, rest) = trimmed.split_once('_')?;
    let publisher_id = rest.rsplit('_').next()?;
    if name.is_empty() || publisher_id.is_empty() || !rest.contains('_') {
        return None;
    }
    Some(format!("{name}_{publisher_id}"))
}

/// Strictly validate a Windows AppUserModelID (`<PackageFamilyName>!<AppId>`) before passing it
/// to `shell:AppsFolder\...`.
pub fn is_valid_aumid(aumid: &str) -> bool {
    let Some((family, app_id)) = aumid.split_once('!') else {
        return false;
    };
    !family.is_empty()
        && !app_id.is_empty()
        && !app_id.contains('!')
        && aumid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '!'))
}

fn enum_reg_subkeys(root: HKEY, path: &str) -> Vec<String> {
    let mut key = HKEY::default();
    let w_path = wide(path);
    // SAFETY: `w_path` is NUL-terminated; `key` is closed before returning.
    if unsafe { RegOpenKeyExW(root, PCWSTR(w_path.as_ptr()), None, KEY_READ, &mut key) }
        != ERROR_SUCCESS
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut name = [0u16; 512];
    for index in 0.. {
        let mut len = name.len() as u32;
        // SAFETY: `name` has space for `len` UTF-16 code units.
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    // SAFETY: `key` was opened above.
    let _ = unsafe { RegCloseKey(key) };
    out
}

/// Find the installed MSIX AppUserModelID (e.g. `OpenAI.Codex_2p2nqsd0c76g0!App`) for any
/// package matching `prefixes` in priority order.
fn find_installed_msix_aumid(prefixes: &[&str]) -> Option<String> {
    if prefixes.is_empty() {
        return None;
    }
    let packages = enum_reg_subkeys(HKEY_CURRENT_USER, APPMODEL_PACKAGES_KEY);
    for prefix in prefixes {
        let prefix_lower = prefix.to_ascii_lowercase();
        let mut matches: Vec<&String> = packages
            .iter()
            .filter(|pkg| pkg.to_ascii_lowercase().starts_with(&prefix_lower))
            .collect();
        // Prefer lexicographically newest version when multiple revisions are staged.
        matches.sort();
        matches.reverse();
        for pkg in matches {
            let apps_path = format!(r"{APPMODEL_PACKAGES_KEY}\{pkg}\Applications");
            for app_id in enum_reg_subkeys(HKEY_CURRENT_USER, &apps_path) {
                if is_valid_aumid(&app_id) {
                    return Some(app_id);
                }
            }
            if let Some(family) = package_family_name_from_full_name(pkg) {
                let fallback = format!("{family}!App");
                if is_valid_aumid(&fallback) {
                    return Some(fallback);
                }
            }
        }
    }
    None
}

fn launch_msix_aumid(aumid: &str) -> Result<(), String> {
    if !is_valid_aumid(aumid) {
        return Err("Invalid packaged app identifier".into());
    }
    let arg = wide(&format!(r"shell:AppsFolder\{aumid}"));
    // SAFETY: `arg` is NUL-terminated UTF-16 and strictly validated to safe AUMID characters.
    let res = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("explorer.exe"),
            PCWSTR(arg.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (res.0 as isize) <= 32 {
        return Err(format!("Could not launch packaged app {aumid}"));
    }
    Ok(())
}

fn path_file_name_lower(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn matches_any_exe(path: &str, allowed_lower: &[&str]) -> bool {
    let name = path_file_name_lower(path);
    allowed_lower
        .iter()
        .any(|cand| name.eq_ignore_ascii_case(cand))
}

fn strip_display_icon_index(icon: &str) -> &str {
    let icon = icon.trim();
    let path = match icon.rsplit_once(',') {
        Some((path, index)) if index.trim().parse::<i32>().is_ok() => path,
        _ => icon,
    };
    path.trim().trim_matches('"')
}

fn uninstall_entry_exe(
    entry: &UninstallEntry,
    name_matches: impl Fn(&str) -> bool,
    publisher_matches: impl Fn(Option<&str>) -> bool,
    default_exes: &[&str],
    allowed_exes: &[&str],
) -> Vec<PathBuf> {
    let Some(name) = entry
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Vec::new();
    };
    if !name_matches(name) || !publisher_matches(entry.publisher.as_deref().map(str::trim)) {
        return Vec::new();
    }

    let mut out = Vec::new();
    if let Some(loc) = entry
        .install_location
        .as_deref()
        .map(|l| l.trim().trim_matches('"'))
        .filter(|l| is_safe_local_path_str(l))
    {
        for exe in default_exes {
            out.push(PathBuf::from(loc).join(exe));
        }
    }
    if let Some(icon) = entry.display_icon.as_deref() {
        let icon_path = strip_display_icon_index(icon);
        if is_safe_local_path_str(icon_path) && matches_any_exe(icon_path, allowed_exes) {
            out.push(PathBuf::from(icon_path));
        }
    }
    out
}

/// Candidate local `.exe` install paths for a provider in priority order (default per-user
/// installs, App Execution Aliases in `WindowsApps`, per-machine installs, then verified Windows
/// Uninstall registry entries).
pub fn provider_exe_candidates(
    provider: ProviderKind,
    local_app_data: Option<&Path>,
    program_files: Option<&Path>,
    entries: &[UninstallEntry],
) -> Vec<PathBuf> {
    if provider == ProviderKind::Cursor {
        return cursor_exe_candidates(local_app_data, program_files, entries);
    }

    let mut out = Vec::new();
    let mut push_safe = |p: PathBuf| {
        if is_safe_local_path(&p) && !out.contains(&p) {
            out.push(p);
        }
    };

    match provider {
        ProviderKind::Claude => {
            if let Some(local) = local_app_data {
                let squirrel_root = local.join("AnthropicClaude");
                push_safe(squirrel_root.join("claude.exe"));
                // Also inspect newest Squirrel `app-*` folder if the root shim isn't present.
                if let Ok(rd) = std::fs::read_dir(&squirrel_root) {
                    let mut app_dirs: Vec<PathBuf> = rd
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.is_dir()
                                && p.file_name()
                                    .and_then(|n| n.to_str())
                                    .is_some_and(|n| n.starts_with("app-"))
                        })
                        .collect();
                    app_dirs.sort();
                    app_dirs.reverse();
                    for dir in app_dirs {
                        push_safe(dir.join("claude.exe"));
                    }
                }
                push_safe(local.join("Programs").join("Claude").join("Claude.exe"));
                push_safe(local.join("Programs").join("claude").join("Claude.exe"));
                push_safe(
                    local
                        .join("Microsoft")
                        .join("WindowsApps")
                        .join("Claude.exe"),
                );
            }
            if let Some(pf) = program_files {
                push_safe(pf.join("Claude").join("Claude.exe"));
            }
            for e in entries {
                for p in uninstall_entry_exe(
                    e,
                    |n| {
                        let l = n.to_ascii_lowercase();
                        l == "claude" || l.starts_with("claude ") || l.starts_with("claude (")
                    },
                    |pub_opt| pub_opt.is_some_and(|p| p.to_ascii_lowercase().contains("anthropic")),
                    &["claude.exe", "Claude.exe"],
                    app_exe_names(ProviderKind::Claude),
                ) {
                    push_safe(p);
                }
            }
        }
        ProviderKind::Codex => {
            if let Some(local) = local_app_data {
                push_safe(local.join("Programs").join("Codex").join("Codex.exe"));
                push_safe(
                    local
                        .join("Programs")
                        .join("OpenAI")
                        .join("Codex")
                        .join("Codex.exe"),
                );
                push_safe(local.join("Codex").join("Codex.exe"));
                push_safe(
                    local
                        .join("Microsoft")
                        .join("WindowsApps")
                        .join("Codex.exe"),
                );
                push_safe(
                    local
                        .join("Microsoft")
                        .join("WindowsApps")
                        .join("ChatGPT.exe"),
                );
            }
            if let Some(pf) = program_files {
                push_safe(pf.join("Codex").join("Codex.exe"));
                push_safe(pf.join("OpenAI").join("Codex").join("Codex.exe"));
            }
            for e in entries {
                for p in uninstall_entry_exe(
                    e,
                    |n| {
                        let l = n.to_ascii_lowercase();
                        l == "codex"
                            || l.starts_with("codex ")
                            || l.starts_with("codex (")
                            || l.starts_with("openai codex")
                            || l == "chatgpt"
                            || l.starts_with("chatgpt ")
                    },
                    |pub_opt| pub_opt.is_some_and(|p| p.to_ascii_lowercase().contains("openai")),
                    &["Codex.exe", "ChatGPT.exe"],
                    app_exe_names(ProviderKind::Codex),
                ) {
                    push_safe(p);
                }
            }
        }
        ProviderKind::Grok => {
            if let Some(local) = local_app_data {
                push_safe(local.join("Programs").join("Grok Bot").join("Grok Bot.exe"));
                push_safe(local.join("Grok Bot").join("Grok Bot.exe"));
                push_safe(local.join("Programs").join("Grok").join("Grok.exe"));
            }
            if let Some(pf) = program_files {
                push_safe(pf.join("Grok Bot").join("Grok Bot.exe"));
                push_safe(pf.join("Grok").join("Grok.exe"));
            }
            for e in entries {
                for p in uninstall_entry_exe(
                    e,
                    |n| {
                        let l = n.to_ascii_lowercase();
                        l == "grok" || l == "grok bot" || l.starts_with("grok bot ")
                    },
                    |_| true,
                    &["Grok Bot.exe", "Grok.exe"],
                    app_exe_names(ProviderKind::Grok),
                ) {
                    push_safe(p);
                }
            }
        }
        ProviderKind::OpenCode => {
            if let Some(local) = local_app_data {
                push_safe(local.join("Programs").join("OpenCode").join("OpenCode.exe"));
                push_safe(local.join("Programs").join("opencode").join("OpenCode.exe"));
                push_safe(local.join("OpenCode").join("OpenCode.exe"));
            }
            if let Some(pf) = program_files {
                push_safe(pf.join("OpenCode").join("OpenCode.exe"));
            }
            for e in entries {
                for p in uninstall_entry_exe(
                    e,
                    |n| {
                        let l = n.to_ascii_lowercase();
                        l == "opencode" || l.starts_with("opencode ") || l.starts_with("opencode (")
                    },
                    |_| true,
                    &["OpenCode.exe", "opencode-desktop.exe"],
                    app_exe_names(ProviderKind::OpenCode),
                ) {
                    push_safe(p);
                }
            }
        }
        ProviderKind::Antigravity => {
            if let Some(local) = local_app_data {
                for dir in ["Antigravity", "Antigravity IDE", "Antigravity 2"] {
                    push_safe(local.join("Programs").join(dir).join("Antigravity.exe"));
                    push_safe(local.join(dir).join("Antigravity.exe"));
                }
            }
            if let Some(pf) = program_files {
                for dir in ["Antigravity", "Antigravity IDE", "Antigravity 2"] {
                    push_safe(pf.join(dir).join("Antigravity.exe"));
                }
            }
            for e in entries {
                for p in uninstall_entry_exe(
                    e,
                    |n| {
                        let l = n.to_ascii_lowercase();
                        l == "antigravity"
                            || l.starts_with("antigravity ")
                            || l.starts_with("antigravity (")
                    },
                    |pub_opt| {
                        pub_opt.is_none_or(|p| {
                            let l = p.to_ascii_lowercase();
                            l.is_empty() || l.contains("google")
                        })
                    },
                    &["Antigravity.exe"],
                    app_exe_names(ProviderKind::Antigravity),
                ) {
                    push_safe(p);
                }
            }
        }
        ProviderKind::Cursor => unreachable!("handled above"),
    }

    out
}

fn process_exe_filename_lower(pid: u32) -> Option<String> {
    // SAFETY: `OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION` is read-only and closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()? };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 code units; `handle` is valid.
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok()
    };
    // SAFETY: `handle` was returned by `OpenProcess`.
    let _ = unsafe { CloseHandle(handle) };
    if !ok || len == 0 {
        return None;
    }
    let path = OsString::from_wide(&buf[..len as usize])
        .to_string_lossy()
        .into_owned();
    Some(path_file_name_lower(&path).to_ascii_lowercase())
}

fn is_window_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    // SAFETY: `cloaked` matches the 4-byte `DWMWA_CLOAKED` attribute.
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    hr.is_ok() && cloaked != 0
}

struct EnumTarget<'a> {
    exe_names: &'a [&'static str],
    self_pid: u32,
    found: Option<(HWND, u32)>,
}

unsafe extern "system" fn enum_matching_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam.0 as *mut EnumTarget<'_>);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    // Ignore tool windows and owned popups; we want a main top-level application window.
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
        return BOOL(1);
    }
    if GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.0.is_null()) {
        return BOOL(1);
    }
    if GetWindowTextLengthW(hwnd) == 0 || is_window_cloaked(hwnd) {
        return BOOL(1);
    }
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 || pid == state.self_pid {
        return BOOL(1);
    }
    if let Some(exe) = process_exe_filename_lower(pid) {
        if state
            .exe_names
            .iter()
            .any(|cand| exe.eq_ignore_ascii_case(cand))
        {
            state.found = Some((hwnd, pid));
            return BOOL(0);
        }
    }
    BOOL(1)
}

/// Find and bring to the foreground an already-running visible top-level window matching any of
/// `exe_names` (case-insensitive executable filename).
pub fn focus_window_by_exe_names(exe_names: &[&'static str]) -> bool {
    let mut target = EnumTarget {
        exe_names,
        self_pid: std::process::id(),
        found: None,
    };
    // SAFETY: `target` stays alive for the synchronous `EnumWindows` traversal.
    let _ = unsafe {
        EnumWindows(
            Some(enum_matching_window),
            LPARAM(&mut target as *mut EnumTarget<'_> as isize),
        )
    };
    let Some((hwnd, pid)) = target.found else {
        return false;
    };

    // SAFETY: `hwnd` is a live top-level window discovered above.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        let _ = AllowSetForegroundWindow(pid);
        let _ = BringWindowToTop(hwnd);
        if !SetForegroundWindow(hwnd).as_bool() {
            // Brief thread-input attach fallback when Windows foreground lock blocks activation.
            let fg = GetForegroundWindow();
            let fg_tid = if !fg.0.is_null() {
                GetWindowThreadProcessId(fg, None)
            } else {
                0
            };
            let cur_tid = GetCurrentThreadId();
            let attached = fg_tid != 0
                && fg_tid != cur_tid
                && AttachThreadInput(cur_tid, fg_tid, true).as_bool();
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            if attached {
                let _ = AttachThreadInput(cur_tid, fg_tid, false);
            }
        }
    }
    true
}

fn open_https_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Refusing to open non-HTTPS URL".into());
    }
    let w_url = wide(url);
    // SAFETY: `w_url` is NUL-terminated UTF-16; `ShellExecuteW` opens it with the user's browser.
    let res = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(w_url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (res.0 as isize) <= 32 {
        return Err(format!("Could not open {url}"));
    }
    Ok(())
}

/// Focus a running provider desktop window, launch the installed desktop `.exe` or packaged MSIX
/// app if found, or fall back to opening the provider's HTTPS web dashboard in the default browser.
pub fn open_provider_app_or_web(provider: ProviderKind) -> Result<LaunchOutcome, String> {
    let label = provider.display_name();
    if focus_window_by_exe_names(app_exe_names(provider)) {
        log::info!("launch: focused running {label} window");
        return Ok(LaunchOutcome::Focused(label));
    }

    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let candidates = provider_exe_candidates(
        provider,
        env("LOCALAPPDATA").as_deref(),
        env("ProgramFiles").as_deref(),
        &uninstall_entries(),
    );
    if let Some(exe) = candidates
        .into_iter()
        .find(|p| is_safe_local_path(p) && p.is_file())
    {
        match std::process::Command::new(&exe).spawn() {
            Ok(_) => {
                log::info!("launch: started {label} desktop app");
                return Ok(LaunchOutcome::Launched(label));
            }
            Err(e) => {
                log::warn!(
                    "launch: failed to start {label} desktop app ({:?}), trying packaged app / web",
                    e.kind()
                );
            }
        }
    }

    if let Some(aumid) = find_installed_msix_aumid(msix_package_prefixes(provider)) {
        match launch_msix_aumid(&aumid) {
            Ok(()) => {
                log::info!("launch: activated packaged {label} app ({aumid})");
                return Ok(LaunchOutcome::Launched(label));
            }
            Err(e) => {
                log::warn!("launch: MSIX activation for {label} failed ({e}), falling back to web");
            }
        }
    }

    let url = web_dashboard_url(provider);
    open_https_url(url)?;
    log::info!("launch: opened {label} web dashboard ({url})");
    Ok(LaunchOutcome::OpenedWeb(url))
}

/// Focus Windows Task Manager if open, or launch `taskmgr.exe`.
pub fn open_task_manager() -> Result<LaunchOutcome, String> {
    if focus_window_by_exe_names(&["taskmgr.exe"]) {
        log::info!("launch: focused running Task Manager");
        return Ok(LaunchOutcome::Focused("Task Manager"));
    }
    // SAFETY: hardcoded system executable name; `ShellExecuteW` handles UAC cleanly.
    let res = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("taskmgr.exe"),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (res.0 as isize) <= 32 {
        return Err("Could not open Task Manager".into());
    }
    log::info!("launch: started Task Manager");
    Ok(LaunchOutcome::Launched("Task Manager"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        name: &str,
        publisher: Option<&str>,
        location: Option<&str>,
        icon: Option<&str>,
    ) -> UninstallEntry {
        UninstallEntry {
            display_name: Some(name.into()),
            publisher: publisher.map(Into::into),
            install_location: location.map(Into::into),
            display_icon: icon.map(Into::into),
        }
    }

    #[test]
    fn every_provider_has_https_dashboard_and_exe_names() {
        for kind in [
            ProviderKind::Claude,
            ProviderKind::Cursor,
            ProviderKind::Codex,
            ProviderKind::Grok,
            ProviderKind::OpenCode,
            ProviderKind::Antigravity,
        ] {
            assert!(web_dashboard_url(kind).starts_with("https://"));
            assert!(!app_exe_names(kind).is_empty());
        }
        // Codex desktop app on Windows runs as ChatGPT.exe (OpenAI.Codex MSIX package).
        assert!(app_exe_names(ProviderKind::Codex).contains(&"chatgpt.exe"));
        assert!(msix_package_prefixes(ProviderKind::Codex).contains(&"OpenAI.Codex_"));
        assert!(msix_package_prefixes(ProviderKind::Claude).contains(&"Claude_"));
    }

    #[test]
    fn msix_package_family_and_aumid_validation() {
        assert_eq!(
            package_family_name_from_full_name("OpenAI.Codex_26.1007.2314.0_x64__2p2nqsd0c76g0")
                .as_deref(),
            Some("OpenAI.Codex_2p2nqsd0c76g0")
        );
        assert_eq!(
            package_family_name_from_full_name("Claude_1.7196.0.0_x64__pzs8sxrjxfjjc").as_deref(),
            Some("Claude_pzs8sxrjxfjjc")
        );
        assert_eq!(package_family_name_from_full_name("NotAPackage"), None);

        assert!(is_valid_aumid("OpenAI.Codex_2p2nqsd0c76g0!App"));
        assert!(is_valid_aumid("Claude_pzs8sxrjxfjjc!Claude"));
        assert!(!is_valid_aumid("OpenAI.Codex_2p2nqsd0c76g0"));
        assert!(!is_valid_aumid("bad id!App & calc"));
        assert!(!is_valid_aumid(r"..\evil!App"));
    }

    #[test]
    fn provider_exe_candidates_resolve_defaults_and_reject_unc_paths() {
        let local = Path::new(r"C:\Users\me\AppData\Local");
        let pf = Path::new(r"C:\Program Files");
        let entries = vec![
            entry("Claude", Some("Anthropic"), Some(r"D:\Apps\Claude"), None),
            entry(
                "Antigravity IDE",
                Some("Google LLC"),
                Some(r"D:\Apps\Antigravity"),
                None,
            ),
            entry(
                "Antigravity",
                Some("Google LLC"),
                Some(r"\\evil\share\Antigravity"),
                None,
            ),
        ];

        let claude = provider_exe_candidates(ProviderKind::Claude, Some(local), Some(pf), &entries);
        assert!(claude.contains(&local.join("AnthropicClaude").join("claude.exe")));
        assert!(claude.contains(
            &local
                .join("Microsoft")
                .join("WindowsApps")
                .join("Claude.exe")
        ));
        assert!(claude.contains(&PathBuf::from(r"D:\Apps\Claude").join("claude.exe")));

        let codex = provider_exe_candidates(ProviderKind::Codex, Some(local), Some(pf), &entries);
        assert!(codex.contains(
            &local
                .join("Microsoft")
                .join("WindowsApps")
                .join("ChatGPT.exe")
        ));

        let ag =
            provider_exe_candidates(ProviderKind::Antigravity, Some(local), Some(pf), &entries);
        assert!(ag.contains(
            &local
                .join("Programs")
                .join("Antigravity")
                .join("Antigravity.exe")
        ));
        assert!(ag.contains(&PathBuf::from(r"D:\Apps\Antigravity").join("Antigravity.exe")));
        assert!(!ag
            .iter()
            .any(|p| p.to_string_lossy().contains(r"\\evil\share")));
    }

    #[test]
    fn open_https_url_rejects_non_https_schemes() {
        assert!(open_https_url("http://example.com").is_err());
        assert!(open_https_url("file:///C:/Windows/win.ini").is_err());
        assert!(open_https_url("cmd.exe").is_err());
    }
}
