//! Full-screen watcher: hides the widget while a full-screen app (game, video, presentation,
//! browser F11) is in front on the widget's monitor, and shows it again afterwards.
//!
//! Every second, while the `hideInFullscreen` setting is on, one reading is taken with cheap
//! Win32 calls. A reading is "full screen" when either
//! - the shell reports a Direct3D full-screen app or presentation mode
//!   (`SHQueryUserNotificationState`), or
//! - the foreground window is on the widget's monitor and its window rect covers the whole
//!   monitor rect (not just the work area, so a maximized window next to a visible taskbar does
//!   not count).
//!
//! Our own windows, the desktop / taskbar and a null foreground never count. The state changes
//! only after two identical readings in a row, so window transitions do not flicker the widget.
//! The widget is shown again only if this watcher hid it, and without taking focus.

use crate::{Core, WIDGET};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Shell::SHQueryUserNotificationState;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
    ShowWindow, GWL_STYLE, SW_SHOWNOACTIVATE, WS_CAPTION, WS_MAXIMIZE, WS_MINIMIZE,
};

const POLL: Duration = Duration::from_secs(1);
/// Identical readings in a row needed before the state changes (about 2 s at one per second).
const STABLE_READINGS: u8 = 2;

/// `QUERY_USER_NOTIFICATION_STATE` values (shellapi.h).
const QUNS_NOT_PRESENT: i32 = 1;
const QUNS_BUSY: i32 = 2;
const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;
const QUNS_PRESENTATION_MODE: i32 = 4;
const QUNS_ACCEPTS_NOTIFICATIONS: i32 = 5;
const QUNS_QUIET_TIME: i32 = 6;
const QUNS_APP: i32 = 7;

/// Desktop and taskbar window classes; they can cover a monitor but are never "an app".
const SHELL_CLASSES: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

/// A screen rectangle in physical pixels, right / bottom exclusive (same as a Win32 RECT).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

/// True when `win` covers all of `mon` (an empty monitor rect never counts).
pub fn covers_monitor(win: Rect, mon: Rect) -> bool {
    mon.right > mon.left
        && mon.bottom > mon.top
        && win.left <= mon.left
        && win.top <= mon.top
        && win.right >= mon.right
        && win.bottom >= mon.bottom
}

/// Full-screen test for one window given its rect, its monitor's rect and its `GWL_STYLE`.
/// A minimized window never counts. A maximized window that still has a caption is an ordinary
/// maximized app, not full screen; this keeps maximized windows from counting when the taskbar
/// auto-hides and the work area is the whole monitor (provisional guard, see the build doc).
pub fn looks_full_screen(win: Rect, mon: Rect, style: u32) -> bool {
    if style & WS_MINIMIZE.0 != 0 {
        return false;
    }
    if style & WS_MAXIMIZE.0 != 0 && style & WS_CAPTION.0 == WS_CAPTION.0 {
        return false;
    }
    covers_monitor(win, mon)
}

/// Desktop / taskbar classes that never count as a full-screen app.
pub fn is_ignored_class(class: &str) -> bool {
    SHELL_CLASSES.contains(&class)
}

/// Shell notification states that mean "full screen" by themselves. `QUNS_BUSY` is left out:
/// it does not say which monitor the app is on, and a full-screen window on the widget's monitor
/// is already caught by the rect test.
pub fn quns_is_full_screen(quns: i32) -> bool {
    quns == QUNS_RUNNING_D3D_FULL_SCREEN || quns == QUNS_PRESENTATION_MODE
}

pub fn quns_name(quns: i32) -> &'static str {
    match quns {
        QUNS_NOT_PRESENT => "QUNS_NOT_PRESENT",
        QUNS_BUSY => "QUNS_BUSY",
        QUNS_RUNNING_D3D_FULL_SCREEN => "QUNS_RUNNING_D3D_FULL_SCREEN",
        QUNS_PRESENTATION_MODE => "QUNS_PRESENTATION_MODE",
        QUNS_ACCEPTS_NOTIFICATIONS => "QUNS_ACCEPTS_NOTIFICATIONS",
        QUNS_QUIET_TIME => "QUNS_QUIET_TIME",
        QUNS_APP => "QUNS_APP",
        _ => "unknown",
    }
}

/// Changes state only after `STABLE_READINGS` identical readings that differ from it.
#[derive(Debug, Default)]
pub struct Debounce {
    state: bool,
    pending: Option<bool>,
    count: u8,
}

impl Debounce {
    pub fn state(&self) -> bool {
        self.state
    }

    /// Feed one reading; returns the new state when it changes.
    pub fn update(&mut self, reading: bool) -> Option<bool> {
        if reading == self.state {
            self.pending = None;
            self.count = 0;
            return None;
        }
        if self.pending == Some(reading) {
            self.count += 1;
        } else {
            self.pending = Some(reading);
            self.count = 1;
        }
        if self.count >= STABLE_READINGS {
            self.state = reading;
            self.pending = None;
            self.count = 0;
            return Some(reading);
        }
        None
    }

    /// Forget everything and go back to "not full screen" (used while the setting is off).
    pub fn reset(&mut self) {
        *self = Debounce::default();
    }
}

/// One reading of the screen. Never holds a window title.
#[derive(Debug, Clone)]
pub struct Reading {
    /// `SHQueryUserNotificationState` value, None when the call failed
    pub quns: Option<i32>,
    /// foreground window class; empty for a null foreground
    pub class: String,
    /// null foreground, one of our own windows, or the desktop / taskbar
    pub ignored: bool,
    /// the foreground is on the reference monitor (the widget's, or its own without a widget)
    pub same_monitor: bool,
    /// the foreground window covers its monitor (and is not minimized / captioned-maximized)
    pub covers: bool,
    pub full_screen: bool,
}

fn monitor_rect(hwnd: HWND) -> Option<(isize, Rect)> {
    // SAFETY: plain Win32 queries; MONITORINFO is sized and owned by this frame.
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if mon.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return None;
        }
        Some((mon.0 as isize, info.rcMonitor.into()))
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: the buffer outlives the call and its length is passed along.
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Take one reading. `widget` is the widget's HWND; without one (the CLI probe) the foreground
/// window is compared with its own monitor.
pub fn read(widget: Option<HWND>) -> Reading {
    // SAFETY: SHQueryUserNotificationState only writes its out value.
    let quns = unsafe { SHQueryUserNotificationState() }.ok().map(|q| q.0);
    // SAFETY: no arguments, returns a possibly null HWND.
    let fg = unsafe { GetForegroundWindow() };
    let mut reading = Reading {
        quns,
        class: String::new(),
        ignored: true,
        same_monitor: false,
        covers: false,
        full_screen: quns.is_some_and(quns_is_full_screen),
    };
    if fg.is_invalid() {
        return reading;
    }
    reading.class = class_name(fg);
    let mut pid = 0u32;
    // SAFETY: `pid` is owned by this frame.
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    if pid == std::process::id() || is_ignored_class(&reading.class) {
        return reading;
    }
    reading.ignored = false;
    let Some((fg_mon, mon_rect)) = monitor_rect(fg) else {
        return reading;
    };
    reading.same_monitor = match widget {
        Some(w) => monitor_rect(w).is_some_and(|(m, _)| m == fg_mon),
        None => true,
    };
    let mut rect = RECT::default();
    // SAFETY: `rect` is owned by this frame.
    if unsafe { GetWindowRect(fg, &mut rect) }.is_err() {
        return reading;
    }
    // SAFETY: plain query of the window style bits.
    let style = unsafe { GetWindowLongW(fg, GWL_STYLE) } as u32;
    reading.covers = looks_full_screen(rect.into(), mon_rect, style);
    reading.full_screen |= reading.same_monitor && reading.covers;
    reading
}

fn hide_in_fullscreen(app: &AppHandle) -> bool {
    app.try_state::<Core>()
        .is_none_or(|core| core.store.get_settings().hide_in_fullscreen)
}

/// Show the widget again without activating it, then put it back on top.
fn reshow(widget: &tauri::WebviewWindow) {
    match widget.hwnd() {
        // SAFETY: the HWND belongs to our live widget window; SW_SHOWNOACTIVATE keeps focus
        // with the user's app.
        Ok(hwnd) => unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        },
        Err(e) => {
            log::warn!("full screen: no widget handle, showing normally: {e}");
            let _ = widget.show();
        }
    }
    let _ = widget.set_always_on_top(true);
}

/// State kept by the watcher thread between ticks.
#[derive(Default)]
struct Watch {
    debounce: Debounce,
    /// the widget is hidden because of full screen; only then is it shown again
    hidden_by_us: bool,
}

fn tick(app: &AppHandle, watch: &mut Watch) {
    let Some(widget) = app.get_webview_window(WIDGET) else {
        return;
    };
    if !hide_in_fullscreen(app) {
        watch.debounce.reset();
        if std::mem::take(&mut watch.hidden_by_us) {
            log::info!("full screen: setting off, showing the widget again");
            reshow(&widget);
        }
        return;
    }
    let reading = read(widget.hwnd().ok());
    match watch.debounce.update(reading.full_screen) {
        Some(true) => {
            log::info!(
                "full screen entered ({}, foreground class {:?})",
                reading.quns.map_or("QUNS unavailable", quns_name),
                reading.class
            );
            // Only a visible widget is hidden, so a widget hidden for another reason stays so.
            if widget.is_visible().unwrap_or(false) {
                let _ = widget.hide();
                watch.hidden_by_us = true;
            }
        }
        Some(false) => {
            log::info!("full screen left (foreground class {:?})", reading.class);
            if std::mem::take(&mut watch.hidden_by_us) && !widget.is_visible().unwrap_or(true) {
                reshow(&widget);
            }
        }
        None => {}
    }
}

pub fn start_watcher(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("fullscreen-watch".into())
        .spawn(move || {
            let mut watch = Watch::default();
            loop {
                tick(&app, &mut watch);
                std::thread::sleep(POLL);
            }
        });
    if let Err(e) = spawned {
        log::warn!("could not start the full-screen watcher: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: Rect = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn covers_monitor_needs_the_full_bounds() {
        assert!(covers_monitor(MON, MON));
        // borderless windows sometimes overshoot by a few pixels
        assert!(covers_monitor(r(-1, -1, 1921, 1081), MON));
        // maximized next to a visible taskbar: work area only
        assert!(!covers_monitor(r(-8, -8, 1928, 1040), MON));
        assert!(!covers_monitor(r(0, 0, 1919, 1080), MON));
        // a second monitor to the right
        let right = r(1920, 0, 4480, 1440);
        assert!(covers_monitor(right, right));
        assert!(!covers_monitor(MON, right));
        // an empty monitor rect never counts
        assert!(!covers_monitor(MON, r(0, 0, 0, 0)));
    }

    #[test]
    fn maximized_with_caption_and_minimized_never_count() {
        let full = r(-8, -8, 1928, 1088);
        assert!(looks_full_screen(MON, MON, 0));
        assert!(!looks_full_screen(full, MON, WS_MAXIMIZE.0 | WS_CAPTION.0));
        assert!(looks_full_screen(full, MON, WS_MAXIMIZE.0));
        assert!(!looks_full_screen(MON, MON, WS_MINIMIZE.0));
    }

    #[test]
    fn shell_classes_are_ignored() {
        for c in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ] {
            assert!(is_ignored_class(c), "{c}");
        }
        assert!(!is_ignored_class("Chrome_WidgetWin_1"));
        assert!(!is_ignored_class("progman"));
        assert!(!is_ignored_class(""));
    }

    #[test]
    fn quns_states_that_count() {
        assert!(quns_is_full_screen(QUNS_RUNNING_D3D_FULL_SCREEN));
        assert!(quns_is_full_screen(QUNS_PRESENTATION_MODE));
        for q in [
            QUNS_NOT_PRESENT,
            QUNS_BUSY,
            QUNS_ACCEPTS_NOTIFICATIONS,
            QUNS_QUIET_TIME,
            QUNS_APP,
            0,
        ] {
            assert!(!quns_is_full_screen(q), "{q}");
        }
        assert_eq!(quns_name(3), "QUNS_RUNNING_D3D_FULL_SCREEN");
        assert_eq!(quns_name(99), "unknown");
    }

    #[test]
    fn debounce_needs_two_identical_readings() {
        let mut d = Debounce::default();
        assert!(!d.state());
        assert_eq!(d.update(false), None);
        assert_eq!(d.update(true), None);
        assert_eq!(d.update(true), Some(true));
        assert!(d.state());
        assert_eq!(d.update(true), None);
        // a single dip does not leave, and resets the count
        assert_eq!(d.update(false), None);
        assert_eq!(d.update(true), None);
        assert_eq!(d.update(false), None);
        assert_eq!(d.update(false), Some(false));
        assert!(!d.state());
        // reset goes back to "not full screen"
        d.update(true);
        d.update(true);
        assert!(d.state());
        d.reset();
        assert!(!d.state());
        assert_eq!(d.update(true), None);
    }
}
