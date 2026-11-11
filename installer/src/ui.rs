//! The installer window (ui/ folder) and the commands it calls.

use crate::decide::{self, Action};
use crate::system;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

/// Set while the NSIS installer runs, so Alt+F4 cannot orphan it mid-install.
static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Overview {
    version: &'static str,
    action: Action,
    installed_version: Option<String>,
    desktop_shortcut: bool,
    start_with_windows: bool,
}

/// The options start as the current state of an installed copy (shortcut present, setting on),
/// so reinstalling never re-adds what the user removed; a fresh install starts with both on.
#[tauri::command]
fn overview() -> Overview {
    let installed = system::installed();
    let installed_version = installed.and_then(|i| i.version);
    let action = decide::action(installed_version.as_deref(), system::version());
    let desktop_shortcut =
        action == Action::Install || system::desktop_shortcut().is_some_and(|lnk| lnk.exists());
    Overview {
        version: system::VERSION,
        action,
        installed_version,
        desktop_shortcut,
        start_with_windows: system::start_at_login(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Options {
    desktop_shortcut: bool,
    start_with_windows: bool,
}

#[derive(Serialize)]
struct Failure {
    message: String,
    detail: String,
}

/// Runs the NSIS installer silently (`/S`), then applies the options. Silent mode skips the
/// template's reinstall page, so an installed copy is overwritten in place with its settings
/// and keys, and a running UsageDeck is closed by the Restart Manager (utils.nsh
/// CheckIfAppIsRunning). Returns a note when the app installed but an option could not be set.
#[tauri::command]
async fn install(options: Options) -> Result<Option<String>, Failure> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err(Failure {
            message: "The installer is already running.".into(),
            detail: String::new(),
        });
    }
    let result = tauri::async_runtime::spawn_blocking(move || install_blocking(options))
        .await
        .map_err(|e| Failure {
            message: "Something went wrong while installing.".into(),
            detail: e.to_string(),
        });
    INSTALLING.store(false, Ordering::SeqCst);
    result?
}

fn install_blocking(options: Options) -> Result<Option<String>, Failure> {
    let shortcut = system::desktop_shortcut();
    let had_shortcut = shortcut.as_ref().is_some_and(|lnk| lnk.exists());
    let code = system::run_nsis("/S").map_err(|e| Failure {
        message: "The installer could not start.".into(),
        detail: format!(
            "Could not run the UsageDeck {} installer: {e}",
            system::VERSION
        ),
    })?;
    if code != 0 {
        let message = match code {
            1 => "The installation was cancelled.",
            _ => "UsageDeck could not be installed. Close UsageDeck if it is open, then try again.",
        };
        return Err(Failure {
            message: message.into(),
            detail: format!("The NSIS installer exited with code {code}."),
        });
    }
    // Silent installs always create the desktop shortcut (installer.nsi, Section Install); `/NS`
    // would drop the Start menu entry as well, so an unwanted shortcut is removed afterwards.
    if !options.desktop_shortcut && !had_shortcut {
        if let Some(lnk) = shortcut {
            let _ = std::fs::remove_file(lnk);
        }
    }
    let exe = system::installed_exe();
    Ok(system::set_start_with_windows(options.start_with_windows, &exe)
        .err()
        .map(|e| {
            let wanted = if options.start_with_windows { "on" } else { "off" };
            format!("Start with Windows could not be turned {wanted} ({e}). Change it in UsageDeck's settings.")
        }))
}

#[tauri::command]
fn launch(app: AppHandle) -> Result<(), String> {
    let exe = system::installed_exe();
    let mut command = std::process::Command::new(&exe);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    command
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", exe.display()))?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn close(app: AppHandle) {
    app.exit(0);
}

/// The page calls this once it has painted, so the window never shows blank.
#[tauri::command]
fn ready(window: WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            overview, install, launch, close, ready
        ])
        .on_window_event(|_, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if INSTALLING.load(Ordering::SeqCst) {
                    api.prevent_close();
                }
            }
        })
        .setup(|app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("UsageDeck Setup")
                .inner_size(420.0, 320.0)
                .resizable(false)
                .maximizable(false)
                .minimizable(false)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .center()
                .visible(false)
                // WebView2's profile goes to %TEMP%, not a %LOCALAPPDATA% folder left behind.
                .data_directory(std::env::temp_dir().join("UsageDeck-Setup-WebView2"))
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("installer window failed");
}
