//! Windows installation, update, uninstallation, registry, autostart, process control,
//! and Windows Credential Manager cleanup.

use crate::decide::{
    self, DesktopShortcutPolicy, InstallPlan, UninstallPlan, Version, APP_DATA_FOLDER,
    MAIN_BINARY_NAME, PRODUCT_NAME, UNINSTALLER_NAME,
};
use crate::shortcut;
use std::ffi::c_void;
use std::io::{self, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Environment::GetCommandLineW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_Programs, FOLDERID_RoamingAppData,
    SHGetKnownFolderPath, KF_FLAG_DEFAULT,
};

/// Embedded `usagedeck.exe` payload produced by `build.rs` (empty in debug builds without
/// `USAGEDECK_EXE_PATH`).
static EMBEDDED_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/usagedeck.bin"));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\UsageDeck";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const RUN_VALUE: &str = PRODUCT_NAME;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Metadata read from an existing per-user UsageDeck installation in the registry.
#[derive(Debug, Clone)]
pub struct InstalledInfo {
    pub version: Option<String>,
    pub install_location: Option<String>,
    pub main_binary_name: Option<String>,
}

/// Result returned when an installation or update completes.
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub install_dir: PathBuf,
    pub exe_path: PathBuf,
    pub warning: Option<String>,
}

/// Raw Win32 command line for the current process.
pub fn command_line() -> String {
    // SAFETY: `GetCommandLineW` returns a NUL-terminated string owned by the process.
    unsafe { GetCommandLineW().to_string() }.unwrap_or_default()
}

/// Parsed crate version (`CARGO_PKG_VERSION`).
pub fn version() -> Version {
    Version::parse(VERSION).expect("CARGO_PKG_VERSION must be valid semver x.y.z")
}

/// Query `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\UsageDeck`.
pub fn installed() -> Option<InstalledInfo> {
    let version = reg_read_string(UNINSTALL_KEY, "DisplayVersion");
    let install_location = reg_read_string(UNINSTALL_KEY, "InstallLocation");
    let main_binary_name = reg_read_string(UNINSTALL_KEY, "MainBinaryName");
    if version.is_none() && install_location.is_none() && main_binary_name.is_none() {
        return None;
    }
    Some(InstalledInfo {
        version,
        install_location,
        main_binary_name,
    })
}

/// Resolve target install directory using `/D=`, existing registry entry, or `%LOCALAPPDATA%\UsageDeck`.
pub fn resolved_install_dir(cli_dir: Option<&Path>) -> PathBuf {
    let existing = installed();
    let local = local_app_data_dir().unwrap_or_default();
    decide::target_install_dir(
        cli_dir,
        existing
            .as_ref()
            .and_then(|i| i.install_location.as_deref()),
        &local,
    )
}

/// Resolve the currently installed `usagedeck.exe` path.
pub fn installed_exe() -> PathBuf {
    let existing = installed();
    let local = local_app_data_dir().unwrap_or_default();
    decide::installed_exe(
        existing
            .as_ref()
            .and_then(|i| i.install_location.as_deref()),
        existing
            .as_ref()
            .and_then(|i| i.main_binary_name.as_deref()),
        &local,
    )
}

pub fn desktop_shortcut_path() -> Option<PathBuf> {
    known_folder(&FOLDERID_Desktop).map(|d| d.join(format!("{PRODUCT_NAME}.lnk")))
}

pub fn start_menu_shortcut_path() -> Option<PathBuf> {
    known_folder(&FOLDERID_Programs).map(|d| d.join(format!("{PRODUCT_NAME}.lnk")))
}

fn legacy_start_menu_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_Programs).map(|d| d.join(PRODUCT_NAME))
}

fn local_app_data_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_LocalAppData)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
}

fn roaming_app_data_dir() -> Option<PathBuf> {
    known_folder(&FOLDERID_RoamingAppData)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
}

fn settings_file() -> Option<PathBuf> {
    roaming_app_data_dir().map(|d| d.join(APP_DATA_FOLDER).join("settings.json"))
}

/// Current `startAtLogin` preference (defaults to `true` on a fresh machine).
pub fn start_at_login() -> bool {
    let text = settings_file().and_then(|f| std::fs::read_to_string(f).ok());
    decide::start_at_login(text.as_deref())
}

/// Launch the installed `usagedeck.exe` detached with its install directory as CWD.
pub fn launch_app(exe: &Path) -> io::Result<()> {
    let mut cmd = Command::new(exe);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn()?;
    Ok(())
}

/// Execute an install or update plan, invoking `on_progress(fraction_0_to_1, status_label)`.
pub fn install_with_progress(
    plan: &InstallPlan,
    mut on_progress: impl FnMut(f32, &str),
) -> Result<InstallOutcome, String> {
    if EMBEDDED_EXE.is_empty() {
        return Err(
            "This installer build has no embedded usagedeck.exe (built without USAGEDECK_EXE_PATH)."
                .into(),
        );
    }

    let install_dir = resolved_install_dir(plan.install_dir.as_deref());
    let exe_path = install_dir.join(MAIN_BINARY_NAME);
    let uninstaller_path = install_dir.join(UNINSTALLER_NAME);

    on_progress(0.12, "Stopping running UsageDeck…");
    stop_running_app();

    on_progress(0.30, "Preparing installation folder…");
    std::fs::create_dir_all(&install_dir)
        .map_err(|e| format!("Could not create {}: {e}", install_dir.display()))?;

    on_progress(
        0.50,
        if plan.update {
            "Updating UsageDeck…"
        } else {
            "Installing UsageDeck…"
        },
    );
    write_bytes_with_retry(&exe_path, EMBEDDED_EXE).map_err(|e| {
        format!(
            "Could not write {}: {e}. Close UsageDeck if it is still open and try again.",
            exe_path.display()
        )
    })?;

    on_progress(0.68, "Installing uninstaller…");
    copy_self_as_uninstaller(&uninstaller_path).map_err(|e| {
        format!(
            "Could not copy uninstaller to {}: {e}",
            uninstaller_path.display()
        )
    })?;
    let legacy_nsis_uninstaller = install_dir.join("uninstall.exe");
    if legacy_nsis_uninstaller.exists() {
        let _ = std::fs::remove_file(&legacy_nsis_uninstaller);
    }

    on_progress(0.80, "Creating shortcuts…");
    let mut warnings = Vec::new();
    if plan.create_start_menu_shortcut {
        if let Some(lnk) = start_menu_shortcut_path() {
            if let Err(e) = shortcut::create_shortcut(
                &lnk,
                &exe_path,
                &install_dir,
                "UsageDeck — AI usage, always in view",
            ) {
                warnings.push(format!("Start Menu shortcut could not be created ({e})."));
            }
        }
    }

    if let Some(desktop_lnk) = desktop_shortcut_path() {
        match plan.desktop_shortcut {
            DesktopShortcutPolicy::Create => {
                if let Err(e) = shortcut::create_shortcut(
                    &desktop_lnk,
                    &exe_path,
                    &install_dir,
                    "UsageDeck — AI usage, always in view",
                ) {
                    warnings.push(format!("Desktop shortcut could not be created ({e})."));
                }
            }
            DesktopShortcutPolicy::UpdateIfExists => {
                if desktop_lnk.exists() {
                    let _ = shortcut::create_shortcut(
                        &desktop_lnk,
                        &exe_path,
                        &install_dir,
                        "UsageDeck — AI usage, always in view",
                    );
                }
            }
            DesktopShortcutPolicy::Remove => {
                let _ = std::fs::remove_file(&desktop_lnk);
            }
            DesktopShortcutPolicy::Never => {}
        }
    }

    on_progress(0.90, "Registering with Windows…");
    register_uninstall_key(&install_dir)
        .map_err(|e| format!("Could not write Windows Uninstall registry entry: {e}"))?;

    match plan.start_with_windows {
        Some(on) => {
            if let Err(e) = set_start_with_windows(on, &exe_path) {
                let state = if on { "on" } else { "off" };
                warnings.push(format!(
                    "Start with Windows could not be turned {state} ({e})."
                ));
            }
        }
        None => {
            // Preserve existing preference, but refresh the Run key path if already enabled
            // (for example, when updating into a new `/D=` directory).
            if reg_read_string(RUN_KEY, RUN_VALUE).is_some() {
                let _ = reg_write_string(RUN_KEY, RUN_VALUE, &decide::run_entry(&exe_path));
            }
        }
    }

    on_progress(1.0, "Done");

    if plan.relaunch {
        if let Err(e) = launch_app(&exe_path) {
            warnings.push(format!("Installed, but could not launch UsageDeck ({e})."));
        }
    }

    let warning = if warnings.is_empty() {
        None
    } else {
        Some(warnings.join(" "))
    };

    Ok(InstallOutcome {
        install_dir,
        exe_path,
        warning,
    })
}

/// Execute full uninstallation according to `plan`.
pub fn uninstall_with_progress(
    plan: &UninstallPlan,
    mut on_progress: impl FnMut(f32, &str),
) -> Result<(), String> {
    let install_dir = resolved_install_dir(plan.install_dir.as_deref());
    let exe_path = install_dir.join(MAIN_BINARY_NAME);
    let uninstaller_path = install_dir.join(UNINSTALLER_NAME);

    on_progress(0.15, "Stopping UsageDeck…");
    stop_running_app();

    on_progress(0.32, "Removing startup entries…");
    remove_autostart_keys();

    on_progress(0.48, "Removing shortcuts…");
    if let Some(lnk) = desktop_shortcut_path() {
        let _ = std::fs::remove_file(lnk);
    }
    if let Some(lnk) = start_menu_shortcut_path() {
        let _ = std::fs::remove_file(lnk);
    }
    if let Some(legacy_dir) = legacy_start_menu_dir() {
        let _ = std::fs::remove_file(legacy_dir.join(format!("{PRODUCT_NAME}.lnk")));
        let _ = std::fs::remove_dir(legacy_dir);
    }

    on_progress(0.64, "Removing application files…");
    remove_file_with_retry(&exe_path)
        .map_err(|e| format!("Could not remove {}: {e}", exe_path.display()))?;
    let _ = std::fs::remove_file(install_dir.join("uninstall.exe"));

    on_progress(0.78, "Removing Windows registry entry…");
    remove_uninstall_key();

    if plan.delete_data {
        on_progress(0.88, "Removing application data & saved keys…");
        if let Some(roaming) = roaming_app_data_dir() {
            let _ = std::fs::remove_dir_all(roaming.join(APP_DATA_FOLDER));
        }
        if let Some(local) = local_app_data_dir() {
            let _ = std::fs::remove_dir_all(local.join(APP_DATA_FOLDER));
        }
        delete_usagedeck_credentials();
    }

    on_progress(0.96, "Finishing cleanup…");
    cleanup_uninstaller_and_dir(&uninstaller_path, &install_dir);

    on_progress(1.0, "Uninstalled");
    Ok(())
}

/// Apply `startAtLogin` to `settings.json` and the HKCU `Run` / `StartupApproved` keys.
pub fn set_start_with_windows(on: bool, exe: &Path) -> io::Result<()> {
    if let Some(file) = settings_file() {
        let current = std::fs::read_to_string(&file).ok();
        if let Some(next) = decide::settings_with_start_at_login(current.as_deref(), on) {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            write_bytes_with_retry(&file, next.as_bytes())?;
        }
    }

    if on {
        reg_write_string(RUN_KEY, RUN_VALUE, &decide::run_entry(exe))?;
    } else {
        remove_autostart_keys();
    }
    Ok(())
}

fn remove_autostart_keys() {
    for subkey in [RUN_KEY, STARTUP_APPROVED_KEY] {
        let _ = reg_delete_value(subkey, RUN_VALUE);
    }
}

fn register_uninstall_key(install_dir: &Path) -> io::Result<()> {
    let install_loc = install_dir.display().to_string();
    let uninstall_str = decide::uninstall_command(install_dir);
    let quiet_uninstall_str = decide::quiet_uninstall_command(install_dir);
    let display_icon = decide::display_icon_value(install_dir);
    let estimated_kb = (EMBEDDED_EXE.len() as u32).div_ceil(1024);

    reg_write_string(UNINSTALL_KEY, "DisplayName", PRODUCT_NAME)?;
    reg_write_string(UNINSTALL_KEY, "DisplayVersion", VERSION)?;
    reg_write_string(UNINSTALL_KEY, "Publisher", PRODUCT_NAME)?;
    reg_write_string(UNINSTALL_KEY, "InstallLocation", &install_loc)?;
    reg_write_string(UNINSTALL_KEY, "MainBinaryName", MAIN_BINARY_NAME)?;
    reg_write_string(UNINSTALL_KEY, "UninstallString", &uninstall_str)?;
    reg_write_string(UNINSTALL_KEY, "QuietUninstallString", &quiet_uninstall_str)?;
    reg_write_string(UNINSTALL_KEY, "DisplayIcon", &display_icon)?;
    reg_write_dword(UNINSTALL_KEY, "EstimatedSize", estimated_kb)?;
    reg_write_dword(UNINSTALL_KEY, "NoModify", 1)?;
    reg_write_dword(UNINSTALL_KEY, "NoRepair", 1)?;
    Ok(())
}

fn remove_uninstall_key() {
    let subkey = wide(UNINSTALL_KEY);
    // SAFETY: NUL-terminated subkey under `HKEY_CURRENT_USER`.
    let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr())) };
}

fn copy_self_as_uninstaller(dest: &Path) -> io::Result<()> {
    let self_exe = std::env::current_exe()?;
    if same_path(&self_exe, dest) {
        return Ok(());
    }
    let bytes = std::fs::read(&self_exe)?;
    write_bytes_with_retry(dest, &bytes)
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
    }
}

fn cleanup_uninstaller_and_dir(uninstaller_path: &Path, install_dir: &Path) {
    let running_from_install_dir = std::env::current_exe().ok().is_some_and(|self_exe| {
        same_path(&self_exe, uninstaller_path)
            || self_exe
                .parent()
                .is_some_and(|parent| same_path(parent, install_dir))
    });

    if running_from_install_dir {
        if let Ok(self_exe) = std::env::current_exe() {
            // Windows cannot delete a running executable; spawn a brief detached cmd.exe
            // from %TEMP% (so it does not hold a handle on `install_dir`) to delete the
            // uninstaller binary and remove the directory if now empty. Use `raw_arg` so
            // Rust does not escape inner quotes as `\"` (which cmd.exe does not unescape).
            let script = format!(
                "\"ping 127.0.0.1 -n 2 > NUL & del /F /Q \"{}\" & rmdir \"{}\"\"",
                self_exe.display(),
                install_dir.display()
            );
            let _ = Command::new("cmd.exe")
                .arg("/C")
                .raw_arg(&script)
                .current_dir(std::env::temp_dir())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
    } else {
        let _ = remove_file_with_retry(uninstaller_path);
        let _ = std::fs::remove_dir(install_dir);
    }
}

/// Stop any running `usagedeck.exe` (or `usagedeck-taskbar.exe`) instances and wait for them to exit.
pub fn stop_running_app() {
    let mut snapshot_ok = false;
    for _pass in 0..3 {
        match terminate_matching_processes(&["usagedeck.exe", "usagedeck-taskbar.exe"]) {
            Ok(0) => {
                snapshot_ok = true;
                break;
            }
            Ok(_) => {
                snapshot_ok = true;
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(_) => break,
        }
    }

    if !snapshot_ok {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", MAIN_BINARY_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn terminate_matching_processes(names: &[&str]) -> windows::core::Result<usize> {
    let self_pid = std::process::id();
    let mut terminated = 0usize;

    // SAFETY: Win32 Toolhelp32 process enumeration and termination on matching child processes.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32ProcessID != self_pid {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                    if names.iter().any(|n| exe_name.eq_ignore_ascii_case(n)) {
                        if let Ok(proc_handle) = OpenProcess(
                            PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                            false,
                            entry.th32ProcessID,
                        ) {
                            let _ = TerminateProcess(proc_handle, 0);
                            let _ = WaitForSingleObject(proc_handle, 5_000);
                            let _ = CloseHandle(proc_handle);
                            terminated += 1;
                        }
                    }
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }

    Ok(terminated)
}

fn write_bytes_with_retry(target: &Path, data: &[u8]) -> io::Result<()> {
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "payload".into());
    let tmp = target.with_file_name(format!("{file_name}.{}.tmp", std::process::id()));

    {
        let mut file = std::fs::File::create(&tmp)?;
        if let Err(err) = file.write_all(data).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err(err);
        }
    }

    for attempt in 0..15u64 {
        match std::fs::rename(&tmp, target) {
            Ok(()) => return Ok(()),
            Err(err) => {
                // On Windows, `rename` fails if the target exists and is momentarily locked by AV
                // or a just-exited process; try removing the old target before retrying.
                let _ = std::fs::remove_file(target);
                if attempt == 14 || !is_transient_lock_error(&err) {
                    // One final attempt after remove_file before giving up.
                    if std::fs::rename(&tmp, target).is_ok() {
                        return Ok(());
                    }
                    let _ = std::fs::remove_file(&tmp);
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(120 + 40 * attempt));
            }
        }
    }

    let _ = std::fs::remove_file(&tmp);
    Err(io::Error::other("timed out replacing target file"))
}

fn remove_file_with_retry(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for attempt in 0..12u64 {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) if attempt < 11 && is_transient_lock_error(&err) => {
                std::thread::sleep(Duration::from_millis(120 + 40 * attempt));
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn is_transient_lock_error(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::PermissionDenied || matches!(err.raw_os_error(), Some(5 | 32 | 33))
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: on success, `SHGetKnownFolderPath` allocates a NUL-terminated path freed with
    // `CoTaskMemFree`.
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0 as *const c_void));
        path.map(PathBuf::from)
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn reg_read_string(subkey: &str, name: &str) -> Option<String> {
    let subkey = wide(subkey);
    let name = wide(name);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut bytes = 0u32;

    // SAFETY: size query on NUL-terminated UTF-16 registry paths.
    let sized = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if sized != ERROR_SUCCESS || bytes == 0 {
        return None;
    }

    let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
    // SAFETY: `buf` is sized to hold `bytes` bytes as queried above.
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if read != ERROR_SUCCESS {
        return None;
    }

    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len])).filter(|s| !s.trim().is_empty())
}

fn reg_write_string(subkey: &str, name: &str, value: &str) -> io::Result<()> {
    let subkey_w = wide(subkey);
    let name_w = wide(name);
    let val_w = wide(value);
    let mut key = HKEY::default();

    // SAFETY: NUL-terminated UTF-16 strings; `key` is closed before returning.
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }

        let bytes = std::slice::from_raw_parts(val_w.as_ptr().cast::<u8>(), val_w.len() * 2);
        let write_status = RegSetValueExW(key, PCWSTR(name_w.as_ptr()), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(key);

        if write_status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(write_status.0 as i32));
        }
    }
    Ok(())
}

fn reg_write_dword(subkey: &str, name: &str, value: u32) -> io::Result<()> {
    let subkey_w = wide(subkey);
    let name_w = wide(name);
    let mut key = HKEY::default();

    // SAFETY: NUL-terminated UTF-16 strings; `key` is closed before returning.
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }

        let bytes = value.to_le_bytes();
        let write_status =
            RegSetValueExW(key, PCWSTR(name_w.as_ptr()), None, REG_DWORD, Some(&bytes));
        let _ = RegCloseKey(key);

        if write_status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(write_status.0 as i32));
        }
    }
    Ok(())
}

fn reg_delete_value(subkey: &str, name: &str) -> io::Result<()> {
    let subkey_w = wide(subkey);
    let name_w = wide(name);
    let mut key = HKEY::default();

    // SAFETY: NUL-terminated UTF-16 strings; `key` is closed before returning.
    unsafe {
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_w.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut key,
        );
        if status != ERROR_SUCCESS {
            return Ok(());
        }
        let _ = RegDeleteValueW(key, PCWSTR(name_w.as_ptr()));
        let _ = RegCloseKey(key);
    }
    Ok(())
}

// ---- Windows Credential Manager cleanup (pure Win32 advapi32 ABI, zero PowerShell) -------------

const CRED_TYPE_GENERIC: u32 = 1;

#[repr(C)]
#[allow(dead_code)]
struct CredentialW {
    flags: u32,
    cred_type: u32,
    target_name: *mut u16,
    comment: *mut u16,
    last_written: [u32; 2],
    credential_blob_size: u32,
    credential_blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut c_void,
    target_alias: *mut u16,
    user_name: *mut u16,
}

#[link(name = "advapi32")]
extern "system" {
    fn CredEnumerateW(
        filter: *const u16,
        flags: u32,
        count: *mut u32,
        credentials: *mut *mut *mut CredentialW,
    ) -> i32;
    fn CredDeleteW(target_name: *const u16, cred_type: u32, flags: u32) -> i32;
    fn CredFree(buffer: *mut c_void);
}

/// Delete all Windows Credential Manager generic entries belonging to UsageDeck (`*.UsageDeck`).
fn delete_usagedeck_credentials() {
    let mut count: u32 = 0;
    let mut creds: *mut *mut CredentialW = std::ptr::null_mut();

    // SAFETY: `CredEnumerateW` returns a heap buffer of `count` pointers owned until `CredFree`.
    unsafe {
        if CredEnumerateW(std::ptr::null(), 0, &mut count, &mut creds) == 0 || creds.is_null() {
            return;
        }

        let mut targets = Vec::new();
        for i in 0..count as usize {
            let entry = *creds.add(i);
            if entry.is_null()
                || (*entry).cred_type != CRED_TYPE_GENERIC
                || (*entry).target_name.is_null()
            {
                continue;
            }
            let target = read_wide_ptr((*entry).target_name);
            if decide::is_usagedeck_credential_target(&target) {
                targets.push(target);
            }
        }
        CredFree(creds.cast());

        for target in targets {
            let target_w = wide(&target);
            let _ = CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0);
        }
    }
}

unsafe fn read_wide_ptr(ptr: *const u16) -> String {
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    String::from_utf16_lossy(slice)
}
