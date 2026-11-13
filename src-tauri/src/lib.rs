//! UsageDeck native shell: windows, tray, plugins and the commands the frontend invokes.
//! Window behaviour is ported from desktop/main.js.

pub mod api;
pub mod claude_logs;
pub mod cli;
pub mod crash;
pub mod diagnostics;
pub mod history;
pub mod pricing;
pub mod providers;
pub mod settings_window;
pub mod store;
pub mod updater;
pub mod win;

use providers::{SeenAccounts, UsageCache};
use serde_json::Value;
use settings_window::{hide_settings, show_settings};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use store::{CredentialVault, Store, SEEN_ACCOUNTS_FILE, USAGE_CACHE_FILE, WINDOW_POS_FILE};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow, WindowEvent,
};

pub const WIDGET: &str = "widget";
pub const SETTINGS: &str = "settings";
/// Widget width in logical pixels (desktop/main.js WIDTH).
const WIDTH: f64 = 238.0;

pub const EVENT_TOGGLE_EXPAND: &str = "widget-toggle-expand";
pub const EVENT_REFRESH: &str = "widget-refresh";
pub const EVENT_SETTINGS_CHANGED: &str = "widget-settings-changed";
/// Tells an open settings window to show General, where the update check's result appears.
pub const EVENT_SHOW_UPDATES: &str = "settings-show-updates";

/// Shared engine state, managed by Tauri and passed to every provider.
pub struct Core {
    pub store: Store,
    pub cache: UsageCache,
    pub seen: SeenAccounts,
    pub http: reqwest::Client,
    /// Claude Code spend from the local transcripts (`/api/claude/pace`).
    pub spend: std::sync::Arc<claude_logs::SpendEngine>,
}

impl Core {
    pub fn new(store: Store) -> Self {
        let cache = UsageCache::new(store.file(USAGE_CACHE_FILE));
        let seen = SeenAccounts::new(store.file(SEEN_ACCOUNTS_FILE));
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        let spend = std::sync::Arc::new(claude_logs::SpendEngine::new(
            store.dir().to_path_buf(),
            claude_logs::default_root(),
        ));
        Self {
            store,
            cache,
            seen,
            http,
            spend,
        }
    }
}

/// Bumped on every widget move; only the last move within the debounce window is saved.
struct PosSaver(AtomicU64);

// ---- commands -------------------------------------------------------------------------------

#[tauri::command]
async fn api(
    app: AppHandle,
    core: tauri::State<'_, Core>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<api::ApiResponse, String> {
    let response = api::dispatch(&core, &method, &path, body).await;
    if let Some(on) = response.start_at_login {
        set_start_at_login(&app, on);
    }
    if response.settings_changed {
        let _ = app.emit_to(WIDGET, EVENT_SETTINGS_CHANGED, ());
    }
    Ok(response)
}

/// Width stays 238; height is clamped to 40 .. work area height - 8 (desktop/main.js:223-229).
#[tauri::command]
fn widget_resize(app: AppHandle, height: f64) {
    if !height.is_finite() {
        return;
    }
    let Some(widget) = app.get_webview_window(WIDGET) else {
        return;
    };
    let max_h = widget
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| m.work_area().size.height as f64 / m.scale_factor() - 8.0)
        .unwrap_or(f64::MAX);
    let h = height.round().min(max_h).max(40.0);
    let _ = widget.set_size(LogicalSize::new(WIDTH, h));
}

/// Copy an OpenCode account's full key to the clipboard from Rust, so the key never enters the
/// webview (replaces the old `GET /api/key` route).
#[tauri::command]
async fn copy_key(
    app: AppHandle,
    core: tauri::State<'_, Core>,
    email: String,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let key = api::account_key(&core, &email)
        .await
        .ok_or_else(|| "unknown account".to_string())?;
    app.clipboard().write_text(key).map_err(|e| {
        log::warn!("could not copy key: {e}");
        "could not copy the key".to_string()
    })
}

/// The Cursor card's "Open Cursor" button: start Cursor so it renews its own sign-in.
#[tauri::command]
fn open_cursor() -> Result<(), String> {
    providers::cursor::open_cursor()
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

/// `reason` names the page's close path (Escape, Ctrl+W) and the event behind
/// it; it is logged so an unexpected close can be traced (src/ui/close-reason.ts).
#[tauri::command]
fn close_settings(app: AppHandle, reason: String) {
    log::info!("settings closed by the page: {reason}");
    hide_settings(&app);
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

// ---- windows --------------------------------------------------------------------------------

fn show_widget(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(WIDGET) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Register or remove the Run key entry. Release builds only, and never from a cargo target
/// folder, so dev and test runs never write the Run key.
fn set_start_at_login(app: &AppHandle, on: bool) {
    if cfg!(debug_assertions) || running_from_cargo_target() {
        log::info!("start at login left unchanged (dev build), wanted {on}");
        return;
    }
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    let result = match (on, launcher.is_enabled().unwrap_or(!on)) {
        (true, _) => launcher.enable(),
        (false, true) => launcher.disable(),
        (false, false) => Ok(()),
    };
    match result {
        Ok(()) => log::info!("start at login set to {on}"),
        Err(e) => log::warn!("could not set start at login to {on}: {e}"),
    }
}

/// Open the app's log folder in Explorer.
fn open_log_folder(app: &AppHandle) {
    let dir = match app.path().app_log_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::warn!("no log folder: {e}");
            return;
        }
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Err(e) = std::process::Command::new("explorer.exe").arg(&dir).spawn() {
        log::warn!("could not open the log folder: {e}");
    }
}

/// Restore the saved widget position if it is still on a visible monitor (desktop/main.js:71-87).
fn restore_position(core: &Core, widget: &WebviewWindow) {
    let Some(saved) = core.store.read_json(WINDOW_POS_FILE) else {
        return;
    };
    let (Some(x), Some(y)) = (saved["x"].as_i64(), saved["y"].as_i64()) else {
        return;
    };
    let width = widget
        .outer_size()
        .map(|s| s.width as i64)
        .unwrap_or(WIDTH as i64);
    let visible = widget
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .any(|m| {
            let a = m.work_area();
            let margin = (40.0 * m.scale_factor()).round() as i64;
            let (ax, ay) = (a.position.x as i64, a.position.y as i64);
            let (aw, ah) = (a.size.width as i64, a.size.height as i64);
            x + margin < ax + aw
                && x + width - margin > ax
                && y + margin < ay + ah
                && y + margin > ay
        });
    if visible {
        let _ = widget.set_position(PhysicalPosition::new(x as i32, y as i32));
    }
}

/// Save the widget position ~300 ms after the last move (desktop/main.js:89-98).
fn schedule_position_save(app: AppHandle, pos: PhysicalPosition<i32>) {
    // Windows reports minimised windows at -32000.
    if pos.x <= -32000 || pos.y <= -32000 {
        return;
    }
    let generation = app.state::<PosSaver>().0.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        if app.state::<PosSaver>().0.load(Ordering::SeqCst) != generation {
            return;
        }
        let core = app.state::<Core>();
        if let Err(e) = core.store.write_json(
            WINDOW_POS_FILE,
            &serde_json::json!({ "x": pos.x, "y": pos.y }),
        ) {
            log::warn!("could not save widget position: {e}");
        }
    });
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let version_label = format!("UsageDeck v{}", app.package_info().version);
    let version = MenuItem::with_id(app, "version", version_label, false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let toggle = MenuItem::with_id(app, "toggle", "Expand/Collapse", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Check for updates", true, None::<&str>)?;
    let logs = MenuItem::with_id(app, "logs", "Open log folder", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &version, &separator, &toggle, &refresh, &settings, &update, &logs, &quit,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("UsageDeck")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => {
                let _ = app.emit_to(WIDGET, EVENT_TOGGLE_EXPAND, ());
            }
            "refresh" => {
                let _ = app.emit_to(WIDGET, EVENT_REFRESH, ());
            }
            "settings" => show_settings(app),
            // The result shows in Settings → General → UsageDeck. A new settings window opens
            // there; an open one is told to go there.
            "update" => {
                updater::check_now(app);
                let _ = app.emit_to(SETTINGS, EVENT_SHOW_UPDATES, ());
                settings_window::show_settings_at(app, Some("updates"));
            }
            "logs" => open_log_folder(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_widget(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// True when the exe sits inside a cargo target folder: any `target` component, or an ancestor
/// holding cargo's `CACHEDIR.TAG` / `.rustc_info.json` (covers a custom CARGO_TARGET_DIR).
fn running_from_cargo_target() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return true;
    };
    exe.ancestors().skip(1).any(|dir| {
        dir.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("target"))
            || dir.join("CACHEDIR.TAG").is_file()
            || dir.join(".rustc_info.json").is_file()
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // first: a panic anywhere after this leaves a crash file in the log folder
    crash::install(crash::default_log_dir());
    // `--import-old-data <dir>` and `--dump` run headless and exit before any window exists.
    if let Some(code) = cli::try_run() {
        std::process::exit(code);
    }
    let context = tauri::generate_context!();
    let version = context.package_info().version.to_string();
    tauri::Builder::default()
        // Managed before the config's widget window exists: its first `get_update_status` and
        // `get_diagnostics` cannot arrive ahead of this state.
        .manage(updater::Updater::new(version))
        // must be first: a second launch focuses the running widget instead, or opens the
        // settings window when it was started with `--settings`
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if cli::wants_settings(&args) {
                show_settings(app);
            } else {
                show_widget(app);
            }
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                // ~1 MB per file; the active file plus 4 archives = 5 files
                .max_file_size(1_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(4))
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            api,
            copy_key,
            open_cursor,
            widget_resize,
            open_settings,
            close_settings,
            settings_window::open_about_link,
            quit_app,
            updater::get_update_status,
            updater::check_for_updates,
            updater::install_update,
            diagnostics::get_diagnostics,
            diagnostics::copy_diagnostics
        ])
        .setup(|app| {
            if let Ok(dir) = app.path().app_log_dir() {
                crash::set_log_dir(dir);
            }
            let dir = app.path().app_data_dir()?;
            app.manage(Core::new(Store::new(dir, Box::new(CredentialVault))));
            claude_logs::start_scanner(app.state::<Core>().spend.clone());
            app.manage(PosSaver(AtomicU64::new(0)));

            // Start at login follows the setting (default on).
            let start_at_login = app.state::<Core>().store.get_settings().start_at_login;
            set_start_at_login(app.handle(), start_at_login);

            if let Some(widget) = app.get_webview_window(WIDGET) {
                restore_position(&app.state::<Core>(), &widget);
                let handle = app.handle().clone();
                widget.on_window_event(move |event| {
                    if let WindowEvent::Moved(pos) = event {
                        schedule_position_save(handle.clone(), *pos);
                    }
                });
            }
            build_tray(app)?;
            // Same rule as the Run key: a dev or cargo-target build never updates itself in
            // the background ("Check for updates" still works).
            let dev_build = cfg!(debug_assertions) || running_from_cargo_target();
            if dev_build {
                log::info!("background update checks off (dev build)");
            } else {
                updater::start(app.handle().clone());
            }
            updater::watch_auto_install(app.handle().clone(), dev_build);
            win::screen::start_sampler(app.handle().clone());
            win::fullscreen::start_watcher(app.handle().clone());
            if cli::wants_settings(std::env::args()) {
                show_settings(app.handle());
            }
            Ok(())
        })
        .run(context)
        .expect("error while running UsageDeck");
}
