//! UsageDeck native shell: windows, tray, plugins and the commands the frontend invokes.
//! Window behaviour is ported from desktop/main.js.

pub mod api;
pub mod cli;
pub mod providers;
pub mod store;
pub mod win;

use providers::{SeenAccounts, UsageCache};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use store::{CredentialVault, Store, SEEN_ACCOUNTS_FILE, USAGE_CACHE_FILE, WINDOW_POS_FILE};
use tauri::menu::{Menu, MenuItem};
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

/// Shared engine state, managed by Tauri and passed to every provider.
pub struct Core {
    pub store: Store,
    pub cache: UsageCache,
    pub seen: SeenAccounts,
    pub http: reqwest::Client,
}

impl Core {
    pub fn new(store: Store) -> Self {
        let cache = UsageCache::new(store.file(USAGE_CACHE_FILE));
        let seen = SeenAccounts::new(store.file(SEEN_ACCOUNTS_FILE));
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self {
            store,
            cache,
            seen,
            http,
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

#[tauri::command]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

#[tauri::command]
fn close_settings(app: AppHandle) {
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

/// Cover the work area of the widget's monitor (desktop/main.js:162-195), then show and focus.
fn show_settings(app: &AppHandle) {
    let Some(settings) = app.get_webview_window(SETTINGS) else {
        return;
    };
    let monitor = app
        .get_webview_window(WIDGET)
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let area = m.work_area();
        let _ = settings.set_position(area.position);
        let _ = settings.set_size(area.size);
    }
    let _ = settings.show();
    let _ = settings.set_focus();
}

/// Hide the overlay and reload its page while hidden, so the next open starts fresh: the panel
/// centred (a drag offset is page state), the first tab selected. Electron got this for free by
/// building a new window per open (desktop/main.js:165-195); here the hidden window is reused.
fn hide_settings(app: &AppHandle) {
    if let Some(settings) = app.get_webview_window(SETTINGS) {
        let _ = settings.hide();
        let _ = settings.reload();
    }
    let _ = app.emit_to(WIDGET, EVENT_SETTINGS_CHANGED, ());
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
    let toggle = MenuItem::with_id(app, "toggle", "Expand/Collapse", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &refresh, &settings, &quit])?;
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
#[cfg_attr(debug_assertions, allow(dead_code))]
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
    // `--import-old-data <dir>` and `--dump` run headless and exit before any window exists.
    if let Some(code) = cli::try_run() {
        std::process::exit(code);
    }
    tauri::Builder::default()
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
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            api,
            widget_resize,
            open_settings,
            close_settings,
            quit_app
        ])
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            app.manage(Core::new(Store::new(dir, Box::new(CredentialVault))));
            app.manage(PosSaver(AtomicU64::new(0)));

            // Start at login. Release builds only, and never from a cargo target folder, so dev
            // and test runs never write the Run key.
            #[cfg(not(debug_assertions))]
            if !running_from_cargo_target() {
                use tauri_plugin_autostart::ManagerExt;
                if let Err(e) = app.autolaunch().enable() {
                    log::warn!("could not enable autostart: {e}");
                }
            }

            if let Some(widget) = app.get_webview_window(WIDGET) {
                restore_position(&app.state::<Core>(), &widget);
                let handle = app.handle().clone();
                widget.on_window_event(move |event| {
                    if let WindowEvent::Moved(pos) = event {
                        schedule_position_save(handle.clone(), *pos);
                    }
                });
            }
            if let Some(settings) = app.get_webview_window(SETTINGS) {
                // Alt+F4 on the overlay hides it like the close button, so it can reopen.
                let handle = app.handle().clone();
                settings.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        hide_settings(&handle);
                    }
                });
            }
            build_tray(app)?;
            win::screen::start_sampler(app.handle().clone());
            if cli::wants_settings(std::env::args()) {
                show_settings(app.handle());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running UsageDeck");
}
