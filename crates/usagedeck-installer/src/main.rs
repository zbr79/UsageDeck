//! UsageDeck v2 pure-Rust single-file Windows installer (`UsageDeck-Setup.exe`).
//!
//! Zero Tauri, zero WebView2, zero NSIS.
//! - Double-clicked with no arguments: opens the native Win32/Direct2D setup window.
//! - Started with NSIS-compatible flags (`/S`, `/P`, `/R`, `/UPDATE`, `/D=<folder>`, `/NS`,
//!   `/UNINSTALL`, `/DELETE-DATA` / `/PURGE`): executes headlessly with full backwards
//!   compatibility for v1.1.0–v1.5.15 updaters.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg_attr(not(windows), allow(dead_code))]
mod decide;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod shortcut;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn main() {
    use decide::LaunchMode;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };

    // SAFETY: enable PerMonitorV2 DPI awareness and initialize STA COM before creating any HWND.
    let com_initialized = unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok()
    };

    let cmd = install::command_line();
    let args = decide::command_line_args(&cmd);

    let exit_code = match decide::decide_launch(args) {
        LaunchMode::Gui => match ui::run() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("UsageDeck Setup UI failed: {e}");
                1
            }
        },
        LaunchMode::HeadlessInstall(plan) => {
            match install::install_with_progress(&plan, |_, _| {}) {
                Ok(_) => 0,
                Err(e) => {
                    if !plan.silent {
                        eprintln!("UsageDeck install failed: {e}");
                    }
                    1
                }
            }
        }
        LaunchMode::HeadlessUninstall(plan) => {
            match install::uninstall_with_progress(&plan, |_, _| {}) {
                Ok(()) => 0,
                Err(e) => {
                    if !plan.silent {
                        eprintln!("UsageDeck uninstall failed: {e}");
                    }
                    1
                }
            }
        }
    };

    if com_initialized {
        // SAFETY: balances successful `CoInitializeEx` above.
        unsafe {
            CoUninitialize();
        }
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("UsageDeck-Setup is a Windows installer binary; run `cargo test` to run unit tests.");
    std::process::exit(1);
}
