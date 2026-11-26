//! UsageDeck v2.0.0 — 100% pure-Rust Windows taskbar strip, anchored flyout,
//! native Settings dialog, background auto-updater, and headless CLI.
//!
//! Zero WebView2, zero Node/React, single-process native Win32 + Direct2D/DirectWrite.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod cli;
mod clipboard;
mod context_menu;
mod icons;
mod layout;
mod logging;
mod poller;
mod render;
mod settings_dialog;
mod taskbar;
mod tooltip;
mod updater;
mod window;

use poller::SharedSnapshot;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use usagedeck_core::store::{CredentialVault, Store};
use usagedeck_core::{crash, default_data_dir, default_log_dir};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

fn show_fatal_dialog(msg: &str) {
    let text: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: NUL-terminated UTF-16 message box shown only when GUI startup fails fatally.
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("UsageDeck"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn main() {
    crash::install(default_log_dir());
    let _ = logging::init();

    // Headless CLI modes (`--import-old-data`, `--dump`, `--pace`, `--fullscreen-probe`).
    if let Some(code) = cli::try_run() {
        std::process::exit(code);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let demo = cli::wants_demo(&args);
    let open_settings_on_start = cli::wants_settings(&args);
    let explicit_updated = cli::wants_updated(&args);
    let just_updated = detect_just_updated(
        default_data_dir().as_deref(),
        env!("CARGO_PKG_VERSION"),
        explicit_updated,
    );
    log::info!(
        "UsageDeck v{} starting (demo={demo}, settings={open_settings_on_start}, updated={just_updated})",
        env!("CARGO_PKG_VERSION")
    );

    // Single-instance guard so launching twice doesn't stack two strips on the taskbar.
    // SAFETY: named local mutex held for the lifetime of the process.
    let mutex_handle = unsafe {
        match CreateMutexW(None, true, w!("Local\\UsageDeckTaskbarSingleton")) {
            Ok(h) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    log::info!("another UsageDeck instance is already running; exiting");
                    let _ = CloseHandle(h);
                    return;
                }
                Some(h)
            }
            Err(_) => None,
        }
    };

    // Sync Windows autostart registry entry with saved setting (skipped in dev/cargo builds).
    if let Some(dir) = default_data_dir() {
        let store = Store::new(dir, Box::new(CredentialVault));
        autostart::set_start_at_login(store.get_settings().start_at_login);
    }

    // SAFETY: enable PerMonitorV2 DPI awareness and STA COM before creating any HWND.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    // Start the pure-Rust background auto-updater; pauses silent restart while Settings is open.
    let pause_flag = Arc::new(AtomicBool::new(false));
    let updater_handle = updater::start_updater_with_options(updater::UpdaterOptions {
        pause_auto_install: pause_flag,
        on_update_starting: Some(Arc::new(|version| {
            window::post_toast_from_any_thread(format!("Updating to v{version}…"));
        })),
        on_status_change: Some(Arc::new(|status| {
            let text = format_update_status(status);
            settings_dialog::set_update_status_text(&text);
            let ready_ver = match &status.phase {
                updater::Phase::Ready { version, .. } => Some(version.as_str()),
                _ => None,
            };
            settings_dialog::set_update_ready(ready_ver);
        })),
        ..Default::default()
    });

    let shared = SharedSnapshot::new(demo);
    if let Err(e) = window::run_ui(
        shared,
        updater_handle.clone(),
        demo,
        open_settings_on_start,
        just_updated,
    ) {
        log::error!("UsageDeck taskbar UI exited with error: {e}");
        show_fatal_dialog(&format!(
            "UsageDeck could not start its taskbar UI:\n\n{e}\n\nCheck %LOCALAPPDATA%\\com.usagedeck.app\\logs\\usagedeck.log for details."
        ));
    }

    updater_handle.stop();

    // SAFETY: balance CoInitializeEx and release single-instance mutex.
    unsafe {
        CoUninitialize();
        if let Some(h) = mutex_handle {
            let _ = CloseHandle(h);
        }
    }
}

const LAST_VERSION_FILE: &str = "last-version.txt";

/// Return `true` when UsageDeck just launched after an update (explicit `--updated` flag,
/// version change compared to `last-version.txt`, or upgrading from an older release that already
/// wrote `settings.json` before `last-version.txt` existed), and persist `current_version`.
fn detect_just_updated(
    data_dir: Option<&std::path::Path>,
    current_version: &str,
    explicit_flag: bool,
) -> bool {
    let Some(dir) = data_dir else {
        return explicit_flag;
    };
    let ver_path = dir.join(LAST_VERSION_FILE);
    let prev = std::fs::read_to_string(&ver_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let updated = if explicit_flag {
        true
    } else if let Some(ref prev_ver) = prev {
        prev_ver != current_version
    } else {
        dir.join("settings.json").exists()
    };

    if prev.as_deref() != Some(current_version) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&ver_path, current_version);
    }
    updated
}

fn format_update_status(status: &updater::UpdateStatus) -> String {
    match &status.phase {
        updater::Phase::Idle => format!("Version {}", status.current_version),
        updater::Phase::Checking => "Checking for updates…".to_string(),
        updater::Phase::Downloading { version } => format!("Downloading v{version}…"),
        updater::Phase::Ready { version, .. } => {
            format!("Update v{version} ready — click Install or close Settings")
        }
        updater::Phase::UpToDate => format!("Up to date (v{})", status.current_version),
        updater::Phase::Failed { message } => format!("Update check failed: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_update_from_flag_version_change_or_legacy_settings() {
        let tmp = std::env::temp_dir().join(format!("usagedeck-ver-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Fresh install: no settings.json, no last-version.txt -> false, records version.
        assert!(!detect_just_updated(Some(&tmp), "2.5.0", false));
        assert_eq!(
            std::fs::read_to_string(tmp.join(LAST_VERSION_FILE)).unwrap(),
            "2.5.0"
        );
        // Normal restart on same version -> false.
        assert!(!detect_just_updated(Some(&tmp), "2.5.0", false));
        // Explicit --updated flag -> true.
        assert!(detect_just_updated(Some(&tmp), "2.5.0", true));
        // Version bump -> true, then false on next restart.
        assert!(detect_just_updated(Some(&tmp), "2.6.0", false));
        assert!(!detect_just_updated(Some(&tmp), "2.6.0", false));

        // Legacy upgrade (settings.json exists, last-version.txt missing) -> true.
        let _ = std::fs::remove_file(tmp.join(LAST_VERSION_FILE));
        std::fs::write(tmp.join("settings.json"), "{}").unwrap();
        assert!(detect_just_updated(Some(&tmp), "2.5.0", false));
        assert!(!detect_just_updated(Some(&tmp), "2.5.0", false));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
