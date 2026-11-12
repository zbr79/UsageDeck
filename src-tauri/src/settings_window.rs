//! The settings window: a normal top-level window with a title bar and a taskbar button
//! (PowerToys `MainWindow.xaml`). It is built on demand, destroyed when closed, and its position
//! and size are saved to `settings-window.json` in the app data folder, then restored and clamped
//! to a visible monitor on the next open (PowerToys `WindowHelper`).
//!
//! Backdrop: solid colour, not Mica. Tauri's Mica effect needs a transparent window and webview,
//! and WebView2 drops ClearType on a transparent surface, so text would render worse.

use crate::store::PanelTheme;
use crate::{Core, EVENT_SETTINGS_CHANGED, SETTINGS, WIDGET};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::window::Color;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Theme, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

/// Saved position and size, in the app data folder next to `window-pos.json`.
pub const STATE_FILE: &str = "settings-window.json";
/// Default and minimum inner size, logical pixels.
const DEFAULT_SIZE: (f64, f64) = (960.0, 680.0);
const MIN_SIZE: (f64, f64) = (640.0, 480.0);
/// The repository named on the About page.
const REPO_URL: &str = "https://github.com/zbr79/UsageDeck";

/// Bumped on every move / resize; only the last one within the debounce window is saved.
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
/// True while a settings window is being built. Building runs off the main thread and takes a
/// moment, so a second open request in that gap would otherwise build a second window.
static BUILDING: AtomicBool = AtomicBool::new(false);

/// A window or work-area rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    fn overlap(&self, other: &Rect) -> i64 {
        let w = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let h = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        if w <= 0 || h <= 0 {
            0
        } else {
            i64::from(w) * i64::from(h)
        }
    }
}

/// Fit a saved window onto the work area it overlaps most: shrink it to fit, then move it fully
/// inside. `None` when it overlaps no work area (its monitor is gone) or the size is empty.
pub fn clamp_to_work_areas(saved: Rect, areas: &[Rect]) -> Option<Rect> {
    if saved.width <= 0 || saved.height <= 0 {
        return None;
    }
    let area = areas
        .iter()
        .filter(|a| saved.overlap(a) > 0)
        .max_by_key(|a| saved.overlap(a))?;
    let width = saved.width.min(area.width);
    let height = saved.height.min(area.height);
    Some(Rect {
        x: saved.x.clamp(area.x, area.x + area.width - width),
        y: saved.y.clamp(area.y, area.y + area.height - height),
        width,
        height,
    })
}

/// A `width` x `height` window centred in a work area, shrunk to fit it.
pub fn centred_in(area: Rect, width: i32, height: i32) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Read `{ x, y, width, height, maximized }`; `None` for a missing or malformed file.
pub fn parse_state(value: &Value) -> Option<(Rect, bool)> {
    let int = |key: &str| value[key].as_i64().and_then(|n| i32::try_from(n).ok());
    let rect = Rect {
        x: int("x")?,
        y: int("y")?,
        width: int("width")?,
        height: int("height")?,
    };
    Some((rect, value["maximized"].as_bool().unwrap_or(false)))
}

/// What to save for the window's current state. A minimised window saves nothing; a maximised
/// one keeps the last normal rectangle, so un-maximising after a restore lands somewhere sane.
pub fn next_state(
    previous: Option<Rect>,
    current: Rect,
    maximized: bool,
    minimized: bool,
) -> Option<(Rect, bool)> {
    if minimized || current.x <= -32000 || current.y <= -32000 {
        return None;
    }
    if maximized {
        return Some((previous.unwrap_or(current), true));
    }
    Some((current, false))
}

fn state_json(rect: Rect, maximized: bool) -> Value {
    json!({
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height,
        "maximized": maximized,
    })
}

fn outer_rect(window: &WebviewWindow) -> Option<Rect> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(Rect {
        x: pos.x,
        y: pos.y,
        width: i32::try_from(size.width).ok()?,
        height: i32::try_from(size.height).ok()?,
    })
}

fn work_area(monitor: &tauri::Monitor) -> Rect {
    let a = monitor.work_area();
    Rect {
        x: a.position.x,
        y: a.position.y,
        width: i32::try_from(a.size.width).unwrap_or(i32::MAX),
        height: i32::try_from(a.size.height).unwrap_or(i32::MAX),
    }
}

/// Save the window's position and size now (close, or the end of a move / resize).
fn save_now(app: &AppHandle) {
    let Some(window) = app.get_webview_window(SETTINGS) else {
        return;
    };
    let Some(current) = outer_rect(&window) else {
        return;
    };
    let core = app.state::<Core>();
    let previous = core
        .store
        .read_json(STATE_FILE)
        .as_ref()
        .and_then(parse_state)
        .map(|(rect, _)| rect);
    let maximized = window.is_maximized().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    let Some((rect, maximized)) = next_state(previous, current, maximized, minimized) else {
        return;
    };
    if let Err(e) = core
        .store
        .write_json(STATE_FILE, &state_json(rect, maximized))
    {
        log::warn!("could not save the settings window position: {e}");
    }
}

/// Save ~400 ms after the last move or resize.
fn schedule_save(app: AppHandle) {
    let generation = SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if SAVE_GENERATION.load(Ordering::SeqCst) == generation {
            save_now(&app);
        }
    });
}

/// Restore the saved rectangle if it still overlaps a monitor, else centre the window on the
/// widget's monitor (the first open).
fn place(app: &AppHandle, window: &WebviewWindow) {
    let areas: Vec<Rect> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(work_area)
        .collect();
    let saved = app
        .state::<Core>()
        .store
        .read_json(STATE_FILE)
        .as_ref()
        .and_then(parse_state)
        .and_then(|(rect, max)| clamp_to_work_areas(rect, &areas).map(|r| (r, max)));
    let (Ok(outer), Ok(inner)) = (window.outer_size(), window.inner_size()) else {
        return;
    };
    let frame_w = outer.width.saturating_sub(inner.width);
    let frame_h = outer.height.saturating_sub(inner.height);
    if let Some((rect, maximized)) = saved {
        let _ = window.set_size(PhysicalSize::new(
            (rect.width as u32).saturating_sub(frame_w),
            (rect.height as u32).saturating_sub(frame_h),
        ));
        let _ = window.set_position(PhysicalPosition::new(rect.x, rect.y));
        if maximized {
            let _ = window.maximize();
        }
        return;
    }
    let monitor = app
        .get_webview_window(WIDGET)
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let scale = monitor.scale_factor();
    let width = (DEFAULT_SIZE.0 * scale).round() as i32 + frame_w as i32;
    let height = (DEFAULT_SIZE.1 * scale).round() as i32 + frame_h as i32;
    let rect = centred_in(work_area(&monitor), width, height);
    let _ = window.set_position(PhysicalPosition::new(rect.x, rect.y));
    let _ = window.set_size(PhysicalSize::new(
        (rect.width as u32).saturating_sub(frame_w),
        (rect.height as u32).saturating_sub(frame_h),
    ));
}

fn create(app: &AppHandle, page: Option<&str>) -> tauri::Result<WebviewWindow> {
    let panel_theme = app.state::<Core>().store.get_settings().panel_theme;
    // "Use system setting": no forced theme, so the caption follows Windows live. The first
    // background colour comes from the widget window, which never forces a theme.
    let forced = match panel_theme {
        PanelTheme::Light => Some(Theme::Light),
        PanelTheme::Dark => Some(Theme::Dark),
        PanelTheme::System => None,
    };
    let dark = match forced {
        Some(theme) => theme == Theme::Dark,
        None => app
            .get_webview_window(WIDGET)
            .and_then(|w| w.theme().ok())
            .is_none_or(|theme| theme == Theme::Dark),
    };
    // The page paints the same colour; this avoids a white flash before it loads.
    let background = if dark {
        Color(0x18, 0x18, 0x1b, 0xff)
    } else {
        Color(0xff, 0xff, 0xff, 0xff)
    };
    let mut builder =
        WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App(settings_url(page).into()))
            .title("UsageDeck Settings")
            .inner_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1)
            .min_inner_size(MIN_SIZE.0, MIN_SIZE.1)
            .resizable(true)
            .maximizable(true)
            .decorations(true)
            .always_on_top(false)
            .skip_taskbar(false)
            .transparent(false)
            .theme(forced)
            .background_color(background)
            .visible(false);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone())?;
    }
    builder.build()
}

fn build(app: &AppHandle, page: Option<&'static str>) {
    if let Some(window) = app.get_webview_window(SETTINGS) {
        let _ = window.set_focus();
        return;
    }
    let window = match create(app, page) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("could not open settings: {e}");
            return;
        }
    };
    place(app, &window);
    // The title bar's X and Alt+F4 close (destroy) the window; the widget then reloads settings.
    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => schedule_save(handle.clone()),
        WindowEvent::CloseRequested { .. } => {
            log::info!("settings close requested by the window (title bar X or Alt+F4)");
            save_now(&handle);
            let _ = handle.emit_to(WIDGET, EVENT_SETTINGS_CHANGED, ());
        }
        WindowEvent::Destroyed => log::info!("settings window destroyed"),
        _ => {}
    });
    let _ = window.show();
    let _ = window.set_focus();
}

/// Open the settings window, or focus it when it is already open.
pub fn show_settings(app: &AppHandle) {
    show_settings_at(app, None);
}

/// The settings page URL; `page` opens that page first (`settings.html?page=about`).
fn settings_url(page: Option<&str>) -> String {
    match page {
        Some(page) => format!("settings.html?page={page}"),
        None => "settings.html".to_string(),
    }
}

/// Open the settings window on `page` when it is not open yet, or focus it when it is (the
/// caller tells an open window where to go).
pub fn show_settings_at(app: &AppHandle, page: Option<&'static str>) {
    if let Some(window) = app.get_webview_window(SETTINGS) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    // Building a window waits on the event loop: never do it on the main thread (tray, menu and
    // single-instance callbacks and sync commands run there), or it deadlocks on Windows.
    if BUILDING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        build(&app, page);
        BUILDING.store(false, Ordering::SeqCst);
    });
}

/// Close the settings window (page close paths: Esc, Ctrl+W) and tell the widget to reload.
pub fn hide_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS) {
        save_now(app);
        if let Err(e) = window.destroy() {
            log::warn!("could not close settings: {e}");
        }
    }
    let _ = app.emit_to(WIDGET, EVENT_SETTINGS_CHANGED, ());
}

/// About page links: "logs" opens the log folder, "repo" the repository in the default browser.
/// Only these two fixed targets exist; anything else is ignored.
#[tauri::command]
pub fn open_about_link(app: AppHandle, link: String) {
    match link.as_str() {
        "logs" => crate::open_log_folder(&app),
        "repo" => {
            if let Err(e) = std::process::Command::new("explorer.exe")
                .arg(REPO_URL)
                .spawn()
            {
                log::warn!("could not open the repository link: {e}");
            }
        }
        other => log::warn!("unknown about link {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIMARY: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    const RIGHT: Rect = Rect {
        x: 1920,
        y: 0,
        width: 2560,
        height: 1400,
    };

    fn r(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_fully_visible_window_is_kept() {
        let saved = r(100, 100, 960, 680);
        assert_eq!(clamp_to_work_areas(saved, &[PRIMARY, RIGHT]), Some(saved));
    }

    #[test]
    fn a_window_hanging_off_the_edge_is_moved_inside() {
        assert_eq!(
            clamp_to_work_areas(r(1500, 800, 960, 680), &[PRIMARY]),
            Some(r(960, 360, 960, 680))
        );
        assert_eq!(
            clamp_to_work_areas(r(-200, -50, 960, 680), &[PRIMARY]),
            Some(r(0, 0, 960, 680))
        );
    }

    #[test]
    fn a_window_goes_to_the_monitor_it_overlaps_most() {
        // mostly on the right monitor: clamped into it, not into the primary one
        assert_eq!(
            clamp_to_work_areas(r(1800, 100, 960, 680), &[PRIMARY, RIGHT]),
            Some(r(1920, 100, 960, 680))
        );
    }

    #[test]
    fn a_window_larger_than_the_work_area_shrinks_to_fit() {
        assert_eq!(
            clamp_to_work_areas(r(10, 10, 3000, 2000), &[PRIMARY]),
            Some(PRIMARY)
        );
    }

    #[test]
    fn a_window_on_a_missing_monitor_is_not_restored() {
        assert_eq!(
            clamp_to_work_areas(r(5000, 100, 960, 680), &[PRIMARY]),
            None
        );
        assert_eq!(clamp_to_work_areas(r(10, 10, 0, 680), &[PRIMARY]), None);
        assert_eq!(clamp_to_work_areas(r(10, 10, 960, 680), &[]), None);
    }

    #[test]
    fn centred_in_the_work_area() {
        assert_eq!(centred_in(PRIMARY, 960, 680), r(480, 180, 960, 680));
        assert_eq!(centred_in(r(0, 0, 800, 600), 960, 680), r(0, 0, 800, 600));
    }

    #[test]
    fn parses_saved_state_and_rejects_bad_files() {
        let v = json!({ "x": -10, "y": 20, "width": 900, "height": 700, "maximized": true });
        assert_eq!(parse_state(&v), Some((r(-10, 20, 900, 700), true)));
        let v = json!({ "x": 1, "y": 2, "width": 3, "height": 4 });
        assert_eq!(parse_state(&v), Some((r(1, 2, 3, 4), false)));
        assert_eq!(parse_state(&json!({ "x": 1, "y": 2 })), None);
        assert_eq!(
            parse_state(&json!({ "x": "1", "y": 2, "width": 3, "height": 4 })),
            None
        );
        assert_eq!(parse_state(&Value::Null), None);
    }

    #[test]
    fn next_state_skips_minimised_and_keeps_the_normal_rect_when_maximised() {
        let normal = r(100, 100, 960, 680);
        let max = r(-8, -8, 1936, 1056);
        assert_eq!(
            next_state(None, normal, false, false),
            Some((normal, false))
        );
        assert_eq!(
            next_state(Some(normal), max, true, false),
            Some((normal, true))
        );
        assert_eq!(next_state(None, max, true, false), Some((max, true)));
        assert_eq!(next_state(Some(normal), normal, false, true), None);
        assert_eq!(
            next_state(Some(normal), r(-32000, -32000, 160, 28), false, false),
            None
        );
    }
}
