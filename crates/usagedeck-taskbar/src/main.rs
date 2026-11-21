//! UsageDeck v2.0.0 — 100% pure-Rust Windows taskbar strip, anchored flyout,
//! native Settings dialog, background auto-updater, and headless CLI.
//!
//! Zero WebView2, zero Node/React, single-process native Win32 + Direct2D/DirectWrite.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod cli;
mod clipboard;
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
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

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

    // Single-instance guard so launching twice doesn't stack two strips on the taskbar.
    // SAFETY: named local mutex held for the lifetime of the process.
    let mutex_handle = unsafe {
        match CreateMutexW(None, true, w!("Local\\UsageDeckTaskbarSingleton")) {
            Ok(h) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
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
        on_status_change: Some(Arc::new(|status| {
            let text = format_update_status(status);
            settings_dialog::set_update_status_text(&text);
        })),
        ..Default::default()
    });

    let shared = SharedSnapshot::new(demo);
    if let Err(e) = window::run_ui(shared, updater_handle.clone(), demo, open_settings_on_start) {
        log::error!("UsageDeck taskbar UI exited with error: {e}");
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

fn format_update_status(status: &updater::UpdateStatus) -> String {
    match &status.phase {
        updater::Phase::Idle => format!("Version {}", status.current_version),
        updater::Phase::Checking => "Checking for updates...".to_string(),
        updater::Phase::Downloading { version } => format!("Downloading v{version}..."),
        updater::Phase::Ready { version, .. } => {
            format!("Update v{version} ready — will install automatically")
        }
        updater::Phase::UpToDate => format!("Up to date (v{})", status.current_version),
        updater::Phase::Failed { message } => format!("Update check failed: {message}"),
    }
}
