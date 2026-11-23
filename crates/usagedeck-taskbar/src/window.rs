//! Native Win32 windowing layer:
//! - TaskbarStripWindow (`WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST`)
//! - TooltipWindow (interactive Direct2D hover card anchored above taskbar pills, plus copy/status toasts)
//! - System Tray icon (`Shell_NotifyIconW`) with left-click Settings shortcut & `TaskbarCreated` recovery
//! - Wiring for native Settings window (hover -> panel -> Settings), icon-click OpenCode key copy,
//!   Cursor launch + burst-poll, hide-until-reset, and auto-updater.
//!
//! Re-entrancy invariant: Win32 calls that synchronously dispatch window messages or run nested
//! modal loops (`ShowWindow`, `SetWindowPos`, `DestroyWindow`, `open_settings_dialog_on_page`)
//! are ALWAYS invoked strictly outside any `STATE` `RefCell` borrow, and `try_with_state` uses
//! `try_borrow_mut()` so a re-entrant callback can never panic across an `extern "system"` FFI
//! boundary.

use crate::clipboard;
use crate::layout::{compute_strip_layout, exhausted_until_ms, PillStyle, StripLayout, StripMode};
use crate::poller::{PollerCmd, SharedSnapshot, WM_APP_SNAPSHOT};
use crate::render::{compute_update_flash_frame, Renderer};
use crate::settings_dialog;
use crate::taskbar::{query_taskbar, StableStripAnchor, TaskbarMetrics, TaskbarTheme};
use crate::tooltip::{
    pill_screen_rect_px, HoverCardAction, TooltipWindow, WM_APP_HOVER_CARD_CLICK,
    WM_APP_HOVER_CARD_LEAVE, WM_APP_HOVER_CARD_MOVE,
};
use crate::updater::UpdaterHandle;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usagedeck_core::default_data_dir;
use usagedeck_core::store::{CredentialVault, Store, ThemeMode};
use usagedeck_core::view::ProviderKind;
use usagedeck_core::win::fullscreen;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    GetWindow, KillTimer, LoadCursorW, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow,
    TrackPopupMenu, TranslateMessage, GW_HWNDPREV, HICON, HWND_TOPMOST, IDC_HAND, IDI_APPLICATION,
    LR_DEFAULTCOLOR, MF_SEPARATOR, MF_STRING, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NULL,
    WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WM_WINDOWPOSCHANGED, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_OBJECT_REORDER: u32 = 0x8004;
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

const WM_TRAY_ICON: u32 = WM_APP + 2;
const WM_APP_TOAST: u32 = WM_APP + 3;
pub const WM_APP_ENSURE_ZORDER: u32 = WM_APP + 4;
const WM_APP_QUIT: u32 = WM_APP + 9;
const TIMER_SYNC_ID: usize = 1;
const TIMER_SYNC_MS: u32 = 300;
const TIMER_ZORDER_ID: usize = 2;
const TIMER_ZORDER_MS: u32 = 60;
const TIMER_HOVER_CLOSE_ID: usize = 3;
const TIMER_HOVER_CLOSE_MS: u32 = 80;
const TIMER_ANIM_ID: usize = 4;
const TIMER_ANIM_MS: u32 = 16;
const TIMER_HOVER_OPEN_ID: usize = 5;
const HOVER_OPEN_DELAY_MS: u32 = 1000;
const HOVER_DISMISS_GRACE: Duration = Duration::from_millis(260);
const TOAST_DURATION: Duration = Duration::from_millis(2200);

const MENU_QUICK_SETTINGS: usize = 5001;
const MENU_QUICK_REFRESH: usize = 5002;
const MENU_QUICK_UPDATE: usize = 5003;
const MENU_QUICK_QUIT: usize = 5004;

const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/icon.ico");

/// Registered `TaskbarCreated` message ID stored in an atomic so `strip_wnd_proc` never
/// needs to borrow `STATE` on unrelated Win32 messages.
static TASKBAR_CREATED_MSG: AtomicU32 = AtomicU32::new(0);
static STRIP_HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static TRAY_HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static STRIP_VISIBLE_FLAG: AtomicBool = AtomicBool::new(false);
static RAISING_ZORDER: AtomicBool = AtomicBool::new(false);

/// Pending brief toast notification posted from background worker threads (e.g. "Key copied").
static PENDING_TOAST: Mutex<Option<(String, Option<RECT>)>> = Mutex::new(None);

struct ToastNotice {
    text: String,
    anchor: Option<RECT>,
    expires_at: Instant,
}

struct AppState {
    renderer: Renderer,
    tooltip: Option<TooltipWindow>,
    shared: Arc<SharedSnapshot>,
    poller_tx: Option<Sender<PollerCmd>>,
    updater: UpdaterHandle,
    strip_hwnd: HWND,
    tray_icon: HICON,
    metrics: Option<TaskbarMetrics>,
    strip_anchor: StableStripAnchor,
    strip_layout: Option<StripLayout>,
    strip_pos: (i32, i32, i32, i32),
    strip_hovered: bool,
    strip_hovered_pill: Option<usize>,
    strip_hovered_icon_pill: Option<usize>,
    strip_hovered_hide_pill: Option<usize>,
    strip_hovered_system: bool,
    strip_tracking_leave: bool,
    pending_hover_pill: Option<usize>,
    pending_hover_system: bool,
    active_hover_pill: Option<usize>,
    active_hover_system: bool,
    hover_card_hovered: bool,
    hover_dismiss_at: Option<Instant>,
    fullscreen_debounce: fullscreen::Debounce,
    strip_visible: bool,
    toast: Option<ToastNotice>,
    update_flash_pending: bool,
    update_flash_started_at: Option<Instant>,
    last_status_log: Option<String>,
}

thread_local! {
    static STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

fn try_with_state<R>(f: impl FnOnce(&mut AppState) -> R) -> Option<R> {
    STATE.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else {
            return None;
        };
        borrow.as_mut().map(f)
    })
}

fn effective_theme(mode: ThemeMode, taskbar_theme: TaskbarTheme) -> TaskbarTheme {
    match mode {
        ThemeMode::Auto => taskbar_theme,
        ThemeMode::Light => TaskbarTheme::Light,
        ThemeMode::Dark => TaskbarTheme::Dark,
    }
}

/// Load the embedded UsageDeck `.ico` into an `HICON`, falling back to `IDI_APPLICATION`.
fn load_app_icon() -> HICON {
    if let Some(icon) = load_ico_bytes(APP_ICON_BYTES) {
        return icon;
    }
    // SAFETY: standard system icon fallback.
    unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() }
}

fn load_ico_bytes(bytes: &[u8]) -> Option<HICON> {
    if bytes.len() < 6 {
        return None;
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 || bytes.len() < 6 + count * 16 {
        return None;
    }
    let mut best: Option<(usize, usize, i32)> = None;
    for i in 0..count {
        let off = 6 + i * 16;
        let w = if bytes[off] == 0 {
            256
        } else {
            i32::from(bytes[off])
        };
        let size = u32::from_le_bytes([
            bytes[off + 8],
            bytes[off + 9],
            bytes[off + 10],
            bytes[off + 11],
        ]) as usize;
        let data_off = u32::from_le_bytes([
            bytes[off + 12],
            bytes[off + 13],
            bytes[off + 14],
            bytes[off + 15],
        ]) as usize;
        if data_off
            .checked_add(size)
            .is_none_or(|end| end > bytes.len())
        {
            continue;
        }
        let score = (w - 32).abs();
        if best.is_none_or(|(_, _, bs)| score < bs) {
            best = Some((data_off, size, score));
        }
    }
    let (data_off, size, _) = best?;
    let slice = &bytes[data_off..data_off + size];
    // SAFETY: `slice` is a valid bounded ICO image entry.
    unsafe { CreateIconFromResourceEx(slice, true, 0x0003_0000, 32, 32, LR_DEFAULTCOLOR).ok() }
}

fn add_tray_icon(hwnd: HWND, icon: HICON) {
    let mut tip = [0u16; 128];
    for (dst, src) in tip.iter_mut().zip("UsageDeck".encode_utf16()) {
        *dst = src;
    }
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY_ICON,
        hIcon: icon,
        szTip: tip,
        ..Default::default()
    };
    // SAFETY: valid `NOTIFYICONDATAW` bound to our live message window.
    let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &nid).as_bool() };
    if ok {
        log::info!("system tray icon registered");
    } else {
        log::warn!("Shell_NotifyIconW(NIM_ADD) returned false");
    }
}

fn remove_tray_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    // SAFETY: removes the icon registered by `add_tray_icon`.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn post_toast(strip_hwnd_raw: isize, text: impl Into<String>, anchor: Option<RECT>) {
    if let Ok(mut slot) = PENDING_TOAST.lock() {
        *slot = Some((text.into(), anchor));
    }
    let hwnd = HWND(strip_hwnd_raw as *mut std::ffi::c_void);
    // SAFETY: `PostMessageW` is thread-safe for custom `WM_APP` messages.
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_TOAST, WPARAM(0), LPARAM(0));
    }
}

/// Post a brief taskbar toast anchored above the strip from any background thread.
pub fn post_toast_from_any_thread(text: impl Into<String>) {
    let strip_hwnd_raw = STRIP_HWND_RAW.load(Ordering::Acquire);
    if strip_hwnd_raw != 0 {
        post_toast(strip_hwnd_raw, text, None);
    }
}

/// Request a clean application shutdown from another window (e.g. Settings -> General -> Quit).
pub fn post_quit_app() {
    let strip_hwnd_raw = STRIP_HWND_RAW.load(Ordering::Acquire);
    if strip_hwnd_raw != 0 {
        let hwnd = HWND(strip_hwnd_raw as *mut std::ffi::c_void);
        // SAFETY: `PostMessageW` is thread-safe for custom `WM_APP` messages on our strip HWND.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

fn log_status_if_changed(state: &mut AppState, summary: String) {
    if state.last_status_log.as_deref() != Some(summary.as_str()) {
        log::info!("taskbar strip: {summary}");
        state.last_status_log = Some(summary);
    }
}

fn open_settings_ui() {
    open_settings_on_page_ui(None);
}

fn open_settings_on_page_ui(provider: Option<ProviderKind>) {
    let Some((strip_hwnd, poller_tx, updater_for_btn)) = try_with_state(|state| {
        clear_hover_card_state(state);
        state.updater.set_pause_auto_install(true);
        (
            state.strip_hwnd,
            state.poller_tx.clone(),
            state.updater.clone(),
        )
    }) else {
        return;
    };

    settings_dialog::open_settings_dialog_on_page(
        strip_hwnd,
        provider,
        Box::new(move || {
            if let Some(ref tx) = poller_tx {
                let _ = tx.send(PollerCmd::RefreshNow);
            }
        }),
        Some(Box::new(move |_hwnd| {
            if updater_for_btn.ready_version().is_some() {
                if let Err(e) = updater_for_btn.install_now() {
                    log::warn!("install_now failed: {e}");
                    let _ = updater_for_btn.check_now();
                }
            } else {
                let _ = updater_for_btn.check_now();
            }
        })),
    );
}

fn open_settings_on_widget_ui() {
    let Some((strip_hwnd, poller_tx, updater_for_btn)) = try_with_state(|state| {
        clear_hover_card_state(state);
        state.updater.set_pause_auto_install(true);
        (
            state.strip_hwnd,
            state.poller_tx.clone(),
            state.updater.clone(),
        )
    }) else {
        return;
    };

    settings_dialog::open_settings_dialog_on_widget(
        strip_hwnd,
        Box::new(move || {
            if let Some(ref tx) = poller_tx {
                let _ = tx.send(PollerCmd::RefreshNow);
            }
        }),
        Some(Box::new(move |_hwnd| {
            if updater_for_btn.ready_version().is_some() {
                if let Err(e) = updater_for_btn.install_now() {
                    log::warn!("install_now failed: {e}");
                    let _ = updater_for_btn.check_now();
                }
            } else {
                let _ = updater_for_btn.check_now();
            }
        })),
    );
}

fn open_cursor_and_watch_ui() {
    let strip_hwnd_raw = try_with_state(|s| s.strip_hwnd.0 as isize).unwrap_or(0);
    match usagedeck_core::providers::cursor::open_cursor() {
        Ok(()) => {
            try_with_state(|s| {
                if let Some(ref tx) = s.poller_tx {
                    let _ = tx.send(PollerCmd::WatchCursorSignIn);
                }
            });
        }
        Err(e) => {
            log::warn!("open_cursor failed: {e}");
            if strip_hwnd_raw != 0 {
                post_toast(strip_hwnd_raw, format!("Could not open Cursor: {e}"), None);
            }
        }
    }
}

fn hide_card_until_reset_ui(provider: ProviderKind, account_id: String, until_ms: u64) {
    let poller_tx = try_with_state(|state| state.poller_tx.clone()).flatten();
    let key = format!("{}:{}", provider.key_prefix(), account_id).to_lowercase();
    std::thread::spawn(move || {
        let Some(dir) = default_data_dir() else {
            return;
        };
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        let store = Store::new(dir, Box::new(CredentialVault));
        let _ = rt.block_on(store.update_settings(|mut s| {
            s.snoozed_until.insert(key, until_ms);
            s
        }));
        if let Some(tx) = poller_tx {
            let _ = tx.send(PollerCmd::RefreshNow);
        }
    });
}

fn copy_opencode_key(email: String, anchor: Option<RECT>) {
    let strip_hwnd_raw = try_with_state(|s| s.strip_hwnd.0 as isize).unwrap_or(0);
    std::thread::spawn(move || {
        let Some(dir) = default_data_dir() else {
            if strip_hwnd_raw != 0 {
                post_toast(strip_hwnd_raw, "Copy failed", anchor);
            }
            return;
        };
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            if strip_hwnd_raw != 0 {
                post_toast(strip_hwnd_raw, "Copy failed", anchor);
            }
            return;
        };
        let store = Store::new(dir, Box::new(CredentialVault));
        let accounts = rt.block_on(store.get_accounts());
        let lower = email.to_lowercase();
        let copied = accounts
            .into_iter()
            .find(|a| a.email.to_lowercase() == lower)
            .filter(|a| !a.key.is_empty())
            .is_some_and(|acc| match clipboard::copy_text(&acc.key) {
                Ok(()) => true,
                Err(e) => {
                    log::warn!("clipboard copy failed: {e}");
                    false
                }
            });
        if strip_hwnd_raw != 0 {
            let msg = if copied { "Key copied" } else { "Copy failed" };
            post_toast(strip_hwnd_raw, msg, anchor);
        }
    });
}

fn quit_app_ui() {
    let Some(strip_hwnd) = try_with_state(|state| {
        hide_tooltip(state);
        state.updater.stop();
        if let Some(ref tx) = state.poller_tx {
            let _ = tx.send(PollerCmd::Stop);
        }
        state.strip_hwnd
    }) else {
        return;
    };

    remove_tray_icon(strip_hwnd);
    // SAFETY: destroys our strip window outside any `STATE` borrow and exits the message loop cleanly.
    unsafe {
        let _ = DestroyWindow(strip_hwnd);
        PostQuitMessage(0);
    }
}

fn hide_tooltip(state: &mut AppState) {
    if let Some(ref mut tip) = state.tooltip {
        tip.hide();
    }
}

fn clear_hover_card_state(state: &mut AppState) {
    state.pending_hover_pill = None;
    state.pending_hover_system = false;
    state.active_hover_pill = None;
    state.active_hover_system = false;
    state.hover_card_hovered = false;
    state.hover_dismiss_at = None;
    hide_tooltip(state);
}

fn point_in_rect(pt: POINT, r: RECT, pad: i32) -> bool {
    pt.x >= r.left - pad && pt.x <= r.right + pad && pt.y >= r.top - pad && pt.y <= r.bottom + pad
}

/// True while the cursor is on the interactive hover card, on its anchored taskbar pill, or
/// crossing the vertical bridge gap between them.
fn cursor_in_hover_bridge(pt: POINT, card: RECT, pill: RECT) -> bool {
    if point_in_rect(pt, card, 4) || point_in_rect(pt, pill, 4) {
        return true;
    }
    let bridge_left = card.left.min(pill.left) - 8;
    let bridge_right = card.right.max(pill.right) + 8;
    let bridge_top = card.bottom - 2;
    let bridge_bottom = pill.top + 2;
    pt.x >= bridge_left && pt.x <= bridge_right && pt.y >= bridge_top && pt.y <= bridge_bottom
}

fn update_active_tooltip(state: &mut AppState) {
    let Some(metrics) = state.metrics else {
        clear_hover_card_state(state);
        return;
    };
    let (snapshot, _) = state.shared.get();
    let theme = effective_theme(snapshot.theme_mode, metrics.theme);

    // 1. Brief toast notice (e.g. "Key copied" / "Hidden until reset") takes precedence for ~2.2s.
    if let Some(ref toast) = state.toast {
        if Instant::now() < toast.expires_at {
            let (sx, sy, sw, sh) = state.strip_pos;
            let anchor = toast.anchor.unwrap_or(RECT {
                left: sx,
                top: sy,
                right: sx + sw.max(1),
                bottom: sy + sh.max(1),
            });
            if let Some(ref mut tip) = state.tooltip {
                tip.show(&toast.text, anchor, &metrics, theme);
            }
            return;
        }
        state.toast = None;
    }

    // 2. Taskbar strip hover: only show hover cards once activated (after 1s dwell or direct click).
    if !state.strip_visible {
        clear_hover_card_state(state);
        return;
    }
    let Some(layout) = state.strip_layout.as_ref() else {
        clear_hover_card_state(state);
        return;
    };
    let (sx, sy, _, _) = state.strip_pos;

    if let Some(pill_idx) = state.active_hover_pill {
        let (Some(item), Some(pill_rect)) =
            (snapshot.items.get(pill_idx), layout.pill_rect(pill_idx))
        else {
            clear_hover_card_state(state);
            return;
        };
        let anchor = pill_screen_rect_px(sx, sy, pill_rect, metrics.scale);
        let footer_hint: Option<String> = if !state.hover_card_hovered {
            if state.strip_hovered_hide_pill == Some(pill_idx) {
                Some("Click × to hide until reset".to_string())
            } else if state.strip_hovered_icon_pill == Some(pill_idx)
                && item.provider == ProviderKind::OpenCode
            {
                Some("Click icon to copy API key".to_string())
            } else if state.strip_hovered_icon_pill == Some(pill_idx)
                && item.provider == ProviderKind::Cursor
                && item.can_open_cursor
            {
                Some("Click icon to sign in".to_string())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(ref mut tip) = state.tooltip {
            tip.show_provider_card(
                item,
                state.renderer.icons(),
                anchor,
                &metrics,
                theme,
                footer_hint.as_deref(),
            );
        }
        return;
    }

    if state.active_hover_system {
        if let (Some(sys), Some(sys_rect)) = (snapshot.system.as_ref(), layout.system_rect()) {
            let anchor = pill_screen_rect_px(sx, sy, sys_rect, metrics.scale);
            if let Some(ref mut tip) = state.tooltip {
                tip.show_system_card(sys, anchor, &metrics, theme);
            }
            return;
        }
    }

    state.active_hover_pill = None;
    state.active_hover_system = false;
    state.hover_card_hovered = false;
    state.hover_dismiss_at = None;
    hide_tooltip(state);
}

enum AnimTimerOp {
    None,
    Start,
    Stop,
}

enum StripWindowOp {
    None,
    Hide(HWND),
    Show {
        strip_hwnd: HWND,
        need_show: bool,
        anim_timer: AnimTimerOp,
    },
}

fn sync_and_paint_strip_ui() {
    let op = try_with_state(|state| -> StripWindowOp {
        // Keep auto-updater pause flag synchronized with the native Settings window open state.
        state
            .updater
            .set_pause_auto_install(settings_dialog::is_settings_open());

        let Some(metrics) = query_taskbar() else {
            log_status_if_changed(state, "hidden (Shell_TrayWnd unavailable)".to_string());
            clear_hover_card_state(state);
            if state.strip_visible {
                state.strip_visible = false;
                return StripWindowOp::Hide(state.strip_hwnd);
            }
            return StripWindowOp::None;
        };
        TRAY_HWND_RAW.store(metrics.tray_hwnd.0 as isize, Ordering::Release);
        state.metrics = Some(metrics);

        let (snapshot, loading) = state.shared.get();
        let theme = effective_theme(snapshot.theme_mode, metrics.theme);
        let forced_contrast_backdrop = matches!(
            (snapshot.theme_mode, metrics.theme),
            (ThemeMode::Light, TaskbarTheme::Dark) | (ThemeMode::Dark, TaskbarTheme::Light)
        );

        // Hide during full-screen apps (debounced over 2 ticks, reusing usagedeck_core::win::fullscreen).
        if snapshot.hide_in_fullscreen {
            let reading = fullscreen::read(Some(metrics.tray_hwnd));
            let _ = state.fullscreen_debounce.update(reading.full_screen);
        } else {
            state.fullscreen_debounce.reset();
        }

        if metrics.hidden || state.fullscreen_debounce.state() {
            let reason = if metrics.hidden {
                "taskbar hidden/auto-hide"
            } else {
                "fullscreen app active"
            };
            log_status_if_changed(state, format!("hidden ({reason})"));
            clear_hover_card_state(state);
            if state.strip_visible {
                state.strip_visible = false;
                return StripWindowOp::Hide(state.strip_hwnd);
            }
            return StripWindowOp::None;
        }

        let (stable_right_px, stable_avail_dip) = state.strip_anchor.resolve(&metrics);
        let layout = compute_strip_layout(&snapshot, stable_avail_dip, loading);
        if matches!(layout.mode, StripMode::Hidden) {
            log_status_if_changed(
                state,
                format!(
                    "hidden (insufficient width: {:.0} DIP, dpi={}, tray={}..{}, notify={}..{})",
                    stable_avail_dip,
                    metrics.dpi,
                    metrics.tray_rect.left,
                    metrics.tray_rect.right,
                    metrics.notify_rect.left,
                    metrics.notify_rect.right,
                ),
            );
            state.strip_layout = Some(layout);
            clear_hover_card_state(state);
            if state.strip_visible {
                state.strip_visible = false;
                return StripWindowOp::Hide(state.strip_hwnd);
            }
            return StripWindowOp::None;
        }

        let width_px = (layout.width_dip * metrics.scale).ceil() as i32;
        let height_px = (layout.height_dip * metrics.scale).ceil() as i32;
        let (x, y) = metrics.strip_origin_for_right_px(stable_right_px, width_px, height_px);
        state.strip_pos = (x, y, width_px, height_px);

        let mode_label = match &layout.mode {
            StripMode::Pills {
                slots,
                overflow_badge,
                system_slot,
            } => {
                let style_tag = match slots.first().map(|s| s.style) {
                    Some(PillStyle::WideMeters) => "wide",
                    Some(PillStyle::CompactStacked) => "compact",
                    None => "pills",
                };
                format!(
                    "{style_tag}({}{}{})",
                    slots.len(),
                    overflow_badge
                        .map(|(_, n)| format!("+{n}"))
                        .unwrap_or_default(),
                    if system_slot.is_some() { "+sys" } else { "" },
                )
            }
            StripMode::CompactSummary { .. } => "compact-summary".to_string(),
            StripMode::Empty { label, .. } => format!("empty({label})"),
            StripMode::Hidden => "hidden".to_string(),
        };
        log_status_if_changed(
            state,
            format!(
                "visible mode={mode_label} theme={theme:?} pos=({x},{y}) size=({width_px}x{height_px}) dpi={} avail={:.0}dip",
                metrics.dpi, metrics.available_width_dip
            ),
        );

        // Start the one-shot first-card "Update complete" animation as soon as the first real card
        // is visible on the taskbar, and stop the 60fps animation timer the instant it completes.
        let mut anim_timer = AnimTimerOp::None;
        let has_first_card = matches!(
            &layout.mode,
            StripMode::Pills {
                slots,
                system_slot,
                ..
            } if !slots.is_empty() || system_slot.is_some()
        );
        if state.update_flash_pending && has_first_card {
            state.update_flash_pending = false;
            state.update_flash_started_at = Some(Instant::now());
            anim_timer = AnimTimerOp::Start;
        }
        let update_flash = if let Some(started) = state.update_flash_started_at {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            match compute_update_flash_frame(elapsed_ms) {
                Some(frame) => Some(frame),
                None => {
                    state.update_flash_started_at = None;
                    anim_timer = AnimTimerOp::Stop;
                    None
                }
            }
        } else {
            None
        };

        let effective_hovered_pill = state.strip_hovered_pill.or(state.active_hover_pill);
        let effective_hovered_system = state.strip_hovered_system || state.active_hover_system;
        if let Err(e) = state.renderer.render_strip(
            state.strip_hwnd,
            x,
            y,
            metrics.dpi,
            theme,
            &snapshot,
            &layout,
            state.strip_hovered || state.hover_card_hovered,
            effective_hovered_pill,
            state.strip_hovered_icon_pill,
            state.strip_hovered_hide_pill,
            effective_hovered_system,
            forced_contrast_backdrop,
            update_flash,
        ) {
            log::warn!("render_strip failed: {e}");
        }
        state.strip_layout = Some(layout);

        let need_show = !state.strip_visible;
        state.strip_visible = true;

        update_active_tooltip(state);

        StripWindowOp::Show {
            strip_hwnd: state.strip_hwnd,
            need_show,
            anim_timer,
        }
    })
    .unwrap_or(StripWindowOp::None);

    // Perform `ShowWindow` / `SetWindowPos` / timer changes strictly outside any `STATE` borrow
    // so synchronous Win32 messages never collide with `RefCell`.
    match op {
        StripWindowOp::None => {}
        StripWindowOp::Hide(strip_hwnd) => {
            STRIP_VISIBLE_FLAG.store(false, Ordering::Release);
            // SAFETY: hides our own strip window.
            unsafe {
                let _ = ShowWindow(strip_hwnd, SW_HIDE);
            }
        }
        StripWindowOp::Show {
            strip_hwnd,
            need_show,
            anim_timer,
        } => {
            STRIP_VISIBLE_FLAG.store(true, Ordering::Release);
            if need_show {
                // SAFETY: non-activating reveal on our own HWND.
                unsafe {
                    let _ = ShowWindow(strip_hwnd, SW_SHOWNOACTIVATE);
                }
            }
            match anim_timer {
                AnimTimerOp::None => {}
                AnimTimerOp::Start => {
                    // SAFETY: 60fps timer on our own HWND for the ~1.5s post-update flash.
                    unsafe {
                        SetTimer(Some(strip_hwnd), TIMER_ANIM_ID, TIMER_ANIM_MS, None);
                    }
                }
                AnimTimerOp::Stop => {
                    // SAFETY: stops the animation timer once the flash completes.
                    unsafe {
                        let _ = KillTimer(Some(strip_hwnd), TIMER_ANIM_ID);
                    }
                }
            }
            ensure_strip_above_tray_now();
        }
    }
}

/// Re-assert topmost Z-order above `Shell_TrayWnd` using atomics only so it is safe to invoke
/// from WinEvent hooks, Settings window activation handlers, and `WM_WINDOWPOSCHANGED` without
/// ever borrowing `STATE`.
pub fn ensure_strip_above_tray_now() {
    if !STRIP_VISIBLE_FLAG.load(Ordering::Acquire) {
        return;
    }
    let strip_raw = STRIP_HWND_RAW.load(Ordering::Acquire);
    let tray_raw = TRAY_HWND_RAW.load(Ordering::Acquire);
    if strip_raw == 0 || tray_raw == 0 {
        return;
    }
    if RAISING_ZORDER
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let strip_hwnd = HWND(strip_raw as *mut std::ffi::c_void);
    let tray_hwnd = HWND(tray_raw as *mut std::ffi::c_void);
    // SAFETY: non-activating topmost Z-order maintenance on our own HWND; guarded against
    // re-entrancy by `RAISING_ZORDER`.
    unsafe {
        if !is_above_tray(strip_hwnd, tray_hwnd) {
            let _ = SetWindowPos(
                strip_hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
    RAISING_ZORDER.store(false, Ordering::Release);
}

/// Queue an asynchronous Z-order check on the strip message loop (used after Settings caption
/// updates when Explorer re-raises `Shell_TrayWnd` one frame later).
pub fn post_ensure_zorder() {
    let strip_raw = STRIP_HWND_RAW.load(Ordering::Acquire);
    if strip_raw != 0 {
        let hwnd = HWND(strip_raw as *mut std::ffi::c_void);
        // SAFETY: `PostMessageW` is safe for custom `WM_APP` messages on our own strip HWND.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_ENSURE_ZORDER, WPARAM(0), LPARAM(0));
        }
    }
}

/// True if `strip` appears above `tray` in the Win32 Z-order chain.
unsafe fn is_above_tray(strip: HWND, tray: HWND) -> bool {
    let mut cur = tray;
    for _ in 0..128 {
        match GetWindow(cur, GW_HWNDPREV) {
            Ok(prev) if !prev.is_invalid() => {
                if prev == strip {
                    return true;
                }
                cur = prev;
            }
            _ => break,
        }
    }
    false
}

fn cursor_dip(lparam: LPARAM, scale: f32) -> (f32, f32) {
    let x = (lparam.0 & 0xffff) as i16 as f32 / scale;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as f32 / scale;
    (x, y)
}

fn track_mouse_leave(hwnd: HWND) {
    let mut tme = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    // SAFETY: standard Win32 mouse-leave tracking on our own HWND.
    unsafe {
        let _ = TrackMouseEvent(&mut tme);
    }
}

/// Exhausted card the user can hide until it resets: `(provider, account_id, title, until_ms)`.
type HideTarget = (ProviderKind, String, String, u64);

/// Compact remaining-time label like `"45m"`, `"3h 56m"`, `"2d 4h"`.
fn format_remaining(ms: u64) -> String {
    let mins = ms.div_ceil(60_000);
    let (d, h, m) = (mins / 1440, (mins % 1440) / 60, mins % 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{}m", m.max(1))
    }
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Resolve the hide-until-reset target for snapshot item `item_idx`, if it is exhausted with a
/// known future reset time.
fn hide_target_for(state: &AppState, item_idx: usize) -> Option<HideTarget> {
    let (snapshot, _) = state.shared.get();
    let item = snapshot.items.get(item_idx)?;
    let until = exhausted_until_ms(item, snapshot.updated_at_ms)?;
    Some((
        item.provider,
        item.account_id.clone(),
        item.title.clone(),
        until,
    ))
}

fn hide_target_with_toast(target: HideTarget, anchor: Option<RECT>) {
    let (provider, account_id, title, until_ms) = target;
    let remaining = format_remaining(until_ms.saturating_sub(now_epoch_ms()));
    hide_card_until_reset_ui(provider, account_id, until_ms);
    let strip_hwnd_raw = STRIP_HWND_RAW.load(Ordering::Acquire);
    if strip_hwnd_raw != 0 {
        post_toast(
            strip_hwnd_raw,
            format!("{title} hidden until reset ({remaining})"),
            anchor,
        );
    }
}

enum StripClickAction {
    None,
    OpenHoverCard,
    OpenSettings,
    CopyOpenCodeKey {
        email: String,
        anchor: Option<RECT>,
    },
    OpenCursor,
    HideUntilReset {
        target: HideTarget,
        anchor: Option<RECT>,
    },
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _time: u32,
) {
    if event == EVENT_SYSTEM_FOREGROUND {
        ensure_strip_above_tray_now();
        return;
    }
    let tray = TRAY_HWND_RAW.load(Ordering::Acquire);
    if tray != 0 && hwnd.0 as isize == tray {
        ensure_strip_above_tray_now();
    }
}

unsafe extern "system" fn strip_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let taskbar_created = TASKBAR_CREATED_MSG.load(Ordering::Relaxed);
    if taskbar_created != 0 && msg == taskbar_created {
        if let Some((strip_hwnd, tray_icon)) = try_with_state(|s| (s.strip_hwnd, s.tray_icon)) {
            add_tray_icon(strip_hwnd, tray_icon);
        }
        sync_and_paint_strip_ui();
        return LRESULT(0);
    }

    match msg {
        WM_TIMER if wparam.0 == TIMER_SYNC_ID || wparam.0 == TIMER_ANIM_ID => {
            sync_and_paint_strip_ui();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_ZORDER_ID => {
            ensure_strip_above_tray_now();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_HOVER_OPEN_ID => {
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
            let opened = try_with_state(|s| {
                if !s.strip_visible {
                    s.pending_hover_pill = None;
                    s.pending_hover_system = false;
                    return false;
                }
                if let Some(idx) = s.pending_hover_pill.take() {
                    if s.strip_hovered_pill == Some(idx) {
                        if let Some(ref mut tip) = s.tooltip {
                            let _ = tip.set_hovered_button(None);
                        }
                        s.active_hover_pill = Some(idx);
                        s.active_hover_system = false;
                        s.hover_card_hovered = false;
                        s.hover_dismiss_at = None;
                        return true;
                    }
                } else if s.pending_hover_system {
                    s.pending_hover_system = false;
                    if s.strip_hovered_system {
                        if let Some(ref mut tip) = s.tooltip {
                            let _ = tip.set_hovered_button(None);
                        }
                        s.active_hover_pill = None;
                        s.active_hover_system = true;
                        s.hover_card_hovered = false;
                        s.hover_dismiss_at = None;
                        return true;
                    }
                }
                false
            })
            .unwrap_or(false);
            if opened {
                let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                sync_and_paint_strip_ui();
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_HOVER_CLOSE_ID => {
            let mut pt = POINT::default();
            let have_cursor = GetCursorPos(&mut pt).is_ok();
            let (kill_timer, needs_repaint) = try_with_state(|s| {
                let has_active_hover = s.active_hover_pill.is_some() || s.active_hover_system;
                let strip_over_target = s.strip_hovered_pill.is_some() || s.strip_hovered_system;
                if !has_active_hover || s.hover_card_hovered || strip_over_target {
                    s.hover_dismiss_at = None;
                    return (true, false);
                }
                if have_cursor {
                    let target_rect = {
                        let scale = s.metrics.map_or(1.0, |m| m.scale);
                        let (sx, sy, _, _) = s.strip_pos;
                        if let Some(idx) = s.active_hover_pill {
                            s.strip_layout
                                .as_ref()
                                .and_then(|l| l.pill_rect(idx))
                                .map(|r| pill_screen_rect_px(sx, sy, r, scale))
                        } else if s.active_hover_system {
                            s.strip_layout
                                .as_ref()
                                .and_then(StripLayout::system_rect)
                                .map(|r| pill_screen_rect_px(sx, sy, r, scale))
                        } else {
                            None
                        }
                    };
                    let in_bridge = match (
                        s.tooltip
                            .as_ref()
                            .and_then(TooltipWindow::card_screen_rect_px),
                        target_rect,
                    ) {
                        (Some(card_rc), Some(pill_rc)) => {
                            cursor_in_hover_bridge(pt, card_rc, pill_rc)
                        }
                        (Some(card_rc), None) => point_in_rect(pt, card_rc, 4),
                        _ => false,
                    };
                    if in_bridge {
                        s.hover_dismiss_at = Some(Instant::now() + HOVER_DISMISS_GRACE);
                        return (false, false);
                    }
                }
                if s.hover_dismiss_at.is_none_or(|t| Instant::now() >= t) {
                    clear_hover_card_state(s);
                    (true, true)
                } else {
                    (false, false)
                }
            })
            .unwrap_or((true, false));
            if kill_timer {
                let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
            }
            if needs_repaint {
                sync_and_paint_strip_ui();
            }
            LRESULT(0)
        }
        WM_APP_ENSURE_ZORDER => {
            ensure_strip_above_tray_now();
            LRESULT(0)
        }
        WM_WINDOWPOSCHANGED => {
            if !RAISING_ZORDER.load(Ordering::Acquire) {
                ensure_strip_above_tray_now();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_APP_SNAPSHOT | WM_SETTINGCHANGE => {
            sync_and_paint_strip_ui();
            LRESULT(0)
        }
        WM_APP_TOAST => {
            let pending = PENDING_TOAST.lock().ok().and_then(|mut slot| slot.take());
            if let Some((text, anchor)) = pending {
                try_with_state(|s| {
                    s.toast = Some(ToastNotice {
                        text,
                        anchor,
                        expires_at: Instant::now() + TOAST_DURATION,
                    });
                    update_active_tooltip(s);
                });
            }
            LRESULT(0)
        }
        WM_APP_HOVER_CARD_MOVE => {
            let x_px = (lparam.0 & 0xffff) as i16 as f32;
            let y_px = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
            try_with_state(|s| {
                let was_hovered = s.hover_card_hovered;
                s.hover_card_hovered = true;
                s.hover_dismiss_at = None;
                let hit = s
                    .tooltip
                    .as_ref()
                    .and_then(|t| t.hit_test_button_px(x_px, y_px));
                let btn_changed = s
                    .tooltip
                    .as_mut()
                    .is_some_and(|t| t.set_hovered_button(hit));
                if !was_hovered || btn_changed {
                    update_active_tooltip(s);
                }
            });
            LRESULT(0)
        }
        WM_APP_HOVER_CARD_LEAVE => {
            let arm_timer = try_with_state(|s| {
                s.hover_card_hovered = false;
                let btn_changed = s
                    .tooltip
                    .as_mut()
                    .is_some_and(|t| t.set_hovered_button(None));
                if btn_changed {
                    update_active_tooltip(s);
                }
                let has_active = s.active_hover_pill.is_some() || s.active_hover_system;
                let over_strip = s.strip_hovered_pill.is_some() || s.strip_hovered_system;
                if has_active && !over_strip {
                    s.hover_dismiss_at = Some(Instant::now() + HOVER_DISMISS_GRACE);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
            if arm_timer {
                SetTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID, TIMER_HOVER_CLOSE_MS, None);
            }
            LRESULT(0)
        }
        WM_APP_HOVER_CARD_CLICK => {
            let x_px = (lparam.0 & 0xffff) as i16 as f32;
            let y_px = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            let hit = try_with_state(|s| {
                let action = s
                    .tooltip
                    .as_ref()
                    .and_then(|t| t.hit_test_button_px(x_px, y_px));
                let anchor = s.active_hover_pill.and_then(|idx| {
                    let scale = s.metrics.map_or(1.0, |m| m.scale);
                    let (sx, sy, _, _) = s.strip_pos;
                    s.strip_layout
                        .as_ref()
                        .and_then(|l| l.pill_rect(idx))
                        .map(|r| pill_screen_rect_px(sx, sy, r, scale))
                });
                (action, anchor)
            });
            if let Some((Some(action), anchor)) = hit {
                match action {
                    HoverCardAction::OpenSettings(provider) => {
                        let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                        open_settings_on_page_ui(Some(provider));
                        sync_and_paint_strip_ui();
                    }
                    HoverCardAction::OpenWidgetSettings => {
                        let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                        open_settings_on_widget_ui();
                        sync_and_paint_strip_ui();
                    }
                    HoverCardAction::HideUntilReset {
                        provider,
                        account_id,
                        title,
                        until_ms,
                    } => {
                        let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                        try_with_state(clear_hover_card_state);
                        hide_target_with_toast((provider, account_id, title, until_ms), anchor);
                        sync_and_paint_strip_ui();
                    }
                    HoverCardAction::CopyOpenCodeKey(email) => {
                        copy_opencode_key(email, anchor);
                    }
                    HoverCardAction::OpenCursor => {
                        let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                        try_with_state(clear_hover_card_state);
                        open_cursor_and_watch_ui();
                        sync_and_paint_strip_ui();
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_QUIT => {
            quit_app_ui();
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            #[derive(Clone, Copy)]
            enum TimerOp {
                None,
                Arm,
                Cancel,
            }
            let (needs_repaint, open_op, close_op) = try_with_state(|s| {
                if !s.strip_tracking_leave {
                    track_mouse_leave(hwnd);
                    s.strip_tracking_leave = true;
                }
                let scale = s.metrics.map_or(1.0, |m| m.scale);
                let (dx, dy) = cursor_dip(lparam, scale);
                let layout = s.strip_layout.as_ref();
                let hit_pill = layout.and_then(|l| l.hit_test_pill(dx, dy));
                let hit_icon = layout.and_then(|l| l.hit_test_pill_icon(dx, dy));
                let hit_hide = layout.and_then(|l| l.hit_test_pill_hide(dx, dy));
                let hit_system = layout.is_some_and(|l| l.hit_test_system(dx, dy));

                let mut open_op = TimerOp::None;
                let mut close_op = TimerOp::None;
                let prev_active = s.active_hover_pill;
                let prev_active_sys = s.active_hover_system;
                let card_already_open = prev_active.is_some() || prev_active_sys;

                if let Some(idx) = hit_pill {
                    if card_already_open {
                        // Card already open: switch cleanly to adjacent pill with 0ms delay.
                        if prev_active != Some(idx) || prev_active_sys {
                            if let Some(ref mut tip) = s.tooltip {
                                let _ = tip.set_hovered_button(None);
                            }
                        }
                        s.pending_hover_pill = None;
                        s.pending_hover_system = false;
                        s.active_hover_pill = Some(idx);
                        s.active_hover_system = false;
                        s.hover_card_hovered = false;
                        s.hover_dismiss_at = None;
                        open_op = TimerOp::Cancel;
                        close_op = TimerOp::Cancel;
                    } else if s.pending_hover_pill != Some(idx) || s.pending_hover_system {
                        // First entry onto a pill: wait HOVER_OPEN_DELAY_MS before opening card.
                        s.pending_hover_pill = Some(idx);
                        s.pending_hover_system = false;
                        open_op = TimerOp::Arm;
                    }
                } else if hit_system {
                    if card_already_open {
                        if !prev_active_sys || prev_active.is_some() {
                            if let Some(ref mut tip) = s.tooltip {
                                let _ = tip.set_hovered_button(None);
                            }
                        }
                        s.pending_hover_pill = None;
                        s.pending_hover_system = false;
                        s.active_hover_pill = None;
                        s.active_hover_system = true;
                        s.hover_card_hovered = false;
                        s.hover_dismiss_at = None;
                        open_op = TimerOp::Cancel;
                        close_op = TimerOp::Cancel;
                    } else if !s.pending_hover_system || s.pending_hover_pill.is_some() {
                        s.pending_hover_pill = None;
                        s.pending_hover_system = true;
                        open_op = TimerOp::Arm;
                    }
                } else {
                    if s.pending_hover_pill.is_some() || s.pending_hover_system {
                        s.pending_hover_pill = None;
                        s.pending_hover_system = false;
                        open_op = TimerOp::Cancel;
                    }
                    if card_already_open && !s.hover_card_hovered {
                        s.hover_dismiss_at = Some(Instant::now() + HOVER_DISMISS_GRACE);
                        close_op = TimerOp::Arm;
                    }
                }

                let changed = !s.strip_hovered
                    || hit_pill != s.strip_hovered_pill
                    || hit_icon != s.strip_hovered_icon_pill
                    || hit_hide != s.strip_hovered_hide_pill
                    || hit_system != s.strip_hovered_system
                    || prev_active != s.active_hover_pill
                    || prev_active_sys != s.active_hover_system;
                if changed {
                    s.strip_hovered = true;
                    s.strip_hovered_pill = hit_pill;
                    s.strip_hovered_icon_pill = hit_icon;
                    s.strip_hovered_hide_pill = hit_hide;
                    s.strip_hovered_system = hit_system;
                }
                (changed, open_op, close_op)
            })
            .unwrap_or((false, TimerOp::None, TimerOp::None));
            match open_op {
                TimerOp::None => {}
                TimerOp::Cancel => {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
                }
                TimerOp::Arm => {
                    SetTimer(Some(hwnd), TIMER_HOVER_OPEN_ID, HOVER_OPEN_DELAY_MS, None);
                }
            }
            match close_op {
                TimerOp::None => {}
                TimerOp::Cancel => {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                }
                TimerOp::Arm => {
                    SetTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID, TIMER_HOVER_CLOSE_MS, None);
                }
            }
            if needs_repaint {
                sync_and_paint_strip_ui();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
            let (needs_repaint, arm_close_timer) = try_with_state(|s| {
                s.strip_tracking_leave = false;
                s.pending_hover_pill = None;
                s.pending_hover_system = false;
                let arm_timer = if (s.active_hover_pill.is_some() || s.active_hover_system)
                    && !s.hover_card_hovered
                {
                    s.hover_dismiss_at = Some(Instant::now() + HOVER_DISMISS_GRACE);
                    true
                } else {
                    false
                };
                if s.strip_hovered
                    || s.strip_hovered_pill.is_some()
                    || s.strip_hovered_icon_pill.is_some()
                    || s.strip_hovered_hide_pill.is_some()
                    || s.strip_hovered_system
                {
                    s.strip_hovered = false;
                    s.strip_hovered_pill = None;
                    s.strip_hovered_icon_pill = None;
                    s.strip_hovered_hide_pill = None;
                    s.strip_hovered_system = false;
                    update_active_tooltip(s);
                    (true, arm_timer)
                } else {
                    (false, arm_timer)
                }
            })
            .unwrap_or((false, false));
            if arm_close_timer {
                SetTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID, TIMER_HOVER_CLOSE_MS, None);
            }
            if needs_repaint {
                sync_and_paint_strip_ui();
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let action = try_with_state(|s| {
                let scale = s.metrics.map_or(1.0, |m| m.scale);
                let (dx, dy) = cursor_dip(lparam, scale);
                let (sx, sy, _, _) = s.strip_pos;
                let anchor_for = |s: &AppState, idx: usize| {
                    s.strip_layout
                        .as_ref()
                        .and_then(|l| l.pill_rect(idx))
                        .map(|r| pill_screen_rect_px(sx, sy, r, scale))
                };

                // 1. Hovered `×` chip on an exhausted pill: hide until it resets.
                if let Some(item_idx) = s
                    .strip_layout
                    .as_ref()
                    .and_then(|l| l.hit_test_pill_hide(dx, dy))
                {
                    if let Some(target) = hide_target_for(s, item_idx) {
                        return StripClickAction::HideUntilReset {
                            target,
                            anchor: anchor_for(s, item_idx),
                        };
                    }
                }

                // 2. Provider icon actions (OpenCode copy key, Cursor sign-in).
                if let Some(item_idx) = s
                    .strip_layout
                    .as_ref()
                    .and_then(|l| l.hit_test_pill_icon(dx, dy))
                {
                    let (snapshot, _) = s.shared.get();
                    if let Some(item) = snapshot.items.get(item_idx) {
                        if item.provider == ProviderKind::OpenCode {
                            return StripClickAction::CopyOpenCodeKey {
                                email: item.account_id.clone(),
                                anchor: anchor_for(s, item_idx),
                            };
                        }
                        if item.provider == ProviderKind::Cursor && item.can_open_cursor {
                            return StripClickAction::OpenCursor;
                        }
                    }
                }

                // 3. `+N` overflow badge, compact summary, or empty state opens Settings directly.
                if s.strip_layout
                    .as_ref()
                    .is_some_and(|l| l.hit_test_overflow_or_empty(dx, dy))
                {
                    return StripClickAction::OpenSettings;
                }

                // 4. Clicking a provider or System pill opens its hover card immediately without waiting 1s.
                if let Some(idx) = s
                    .strip_layout
                    .as_ref()
                    .and_then(|l| l.hit_test_pill(dx, dy))
                {
                    s.pending_hover_pill = None;
                    s.pending_hover_system = false;
                    s.active_hover_pill = Some(idx);
                    s.active_hover_system = false;
                    s.hover_card_hovered = false;
                    s.hover_dismiss_at = None;
                    return StripClickAction::OpenHoverCard;
                }
                if s.strip_layout
                    .as_ref()
                    .is_some_and(|l| l.hit_test_system(dx, dy))
                {
                    s.pending_hover_pill = None;
                    s.pending_hover_system = false;
                    s.active_hover_pill = None;
                    s.active_hover_system = true;
                    s.hover_card_hovered = false;
                    s.hover_dismiss_at = None;
                    return StripClickAction::OpenHoverCard;
                }

                StripClickAction::None
            })
            .unwrap_or(StripClickAction::None);

            match action {
                StripClickAction::None => {}
                StripClickAction::OpenHoverCard => {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                    sync_and_paint_strip_ui();
                }
                StripClickAction::OpenSettings => open_settings_ui(),
                StripClickAction::CopyOpenCodeKey { email, anchor } => {
                    copy_opencode_key(email, anchor);
                }
                StripClickAction::OpenCursor => open_cursor_and_watch_ui(),
                StripClickAction::HideUntilReset { target, anchor } => {
                    hide_target_with_toast(target, anchor);
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
            show_quick_context_menu(hwnd);
            LRESULT(0)
        }
        WM_TRAY_ICON => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => open_settings_ui(),
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                    show_quick_context_menu(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            STRIP_VISIBLE_FLAG.store(false, Ordering::Release);
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Show a native Win32 context menu at the cursor (for right-clicking the System Tray icon
/// or right-clicking the taskbar strip) offering Settings, Refresh, Update, and Quit.
///
/// Must ALWAYS be invoked strictly outside any `STATE` `RefCell` borrow.
fn show_quick_context_menu(hwnd: HWND) {
    let Some((poller_tx, updater, ready_ver)) = try_with_state(|s| {
        clear_hover_card_state(s);
        let ready = s.updater.ready_version();
        (s.poller_tx.clone(), s.updater.clone(), ready)
    }) else {
        return;
    };

    let update_label: Vec<u16> = match ready_ver {
        Some(ref ver) => format!("Install update v{ver}\0").encode_utf16().collect(),
        None => "Check for updates\0".encode_utf16().collect(),
    };

    // SAFETY: TrackPopupMenu runs strictly outside any STATE RefCell borrow.
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUICK_SETTINGS, w!("Settings…"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUICK_REFRESH, w!("Refresh now"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_QUICK_UPDATE,
            PCWSTR(update_label.as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUICK_QUIT, w!("Quit UsageDeck"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        )
        .0 as usize;
        // Microsoft Win32 requirement for tray popup menus so clicking outside dismisses cleanly.
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        picked
    };

    match cmd {
        MENU_QUICK_SETTINGS => open_settings_ui(),
        MENU_QUICK_REFRESH => {
            if let Some(ref tx) = poller_tx {
                let _ = tx.send(PollerCmd::RefreshNow);
            }
            post_toast_from_any_thread("Refreshing usage…".to_string());
        }
        MENU_QUICK_UPDATE => {
            if updater.ready_version().is_some() {
                if let Err(e) = updater.install_now() {
                    log::warn!("install_now from quick menu failed: {e}");
                    let _ = updater.check_now();
                }
            } else {
                let _ = updater.check_now();
                post_toast_from_any_thread("Checking for updates…".to_string());
            }
        }
        MENU_QUICK_QUIT => {
            quit_app_ui();
            return;
        }
        _ => {}
    }

    ensure_strip_above_tray_now();
    post_ensure_zorder();
}

/// Create the taskbar strip window, interactive hover card window, and tray icon,
/// then run the Win32 message loop until quit.
pub fn run_ui(
    shared: Arc<SharedSnapshot>,
    updater: UpdaterHandle,
    demo: bool,
    open_settings_on_start: bool,
    just_updated: bool,
) -> Result<()> {
    let renderer = Renderer::new()?;
    let tooltip = TooltipWindow::new().ok();
    let tray_icon = load_app_icon();

    // SAFETY: Win32 class registration, window creation, timer, and standard message loop.
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let hand_cursor = LoadCursorW(None, IDC_HAND)?;

        let strip_class = w!("UsageDeckTaskbarStrip");
        let wc_strip = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(strip_wnd_proc),
            hInstance: hinstance.into(),
            hCursor: hand_cursor,
            lpszClassName: strip_class,
            ..Default::default()
        };
        RegisterClassExW(&wc_strip);

        let strip_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            strip_class,
            w!("UsageDeck"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;

        let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarCreated"));
        TASKBAR_CREATED_MSG.store(taskbar_created_msg, Ordering::Relaxed);
        add_tray_icon(strip_hwnd, tray_icon);
        STRIP_HWND_RAW.store(strip_hwnd.0 as isize, Ordering::Release);
        if let Some(ref tip) = tooltip {
            tip.set_owner_hwnd(strip_hwnd);
        }

        let poller_tx = crate::poller::start_poller(strip_hwnd.0 as isize, shared.clone(), demo);

        STATE.with(|cell| {
            *cell.borrow_mut() = Some(AppState {
                renderer,
                tooltip,
                shared,
                poller_tx: Some(poller_tx),
                updater,
                strip_hwnd,
                tray_icon,
                metrics: None,
                strip_anchor: StableStripAnchor::default(),
                strip_layout: None,
                strip_pos: (0, 0, 0, 0),
                strip_hovered: false,
                strip_hovered_pill: None,
                strip_hovered_icon_pill: None,
                strip_hovered_hide_pill: None,
                strip_hovered_system: false,
                strip_tracking_leave: false,
                pending_hover_pill: None,
                pending_hover_system: false,
                active_hover_pill: None,
                active_hover_system: false,
                hover_card_hovered: false,
                hover_dismiss_at: None,
                fullscreen_debounce: fullscreen::Debounce::default(),
                strip_visible: false,
                toast: None,
                update_flash_pending: just_updated,
                update_flash_started_at: None,
                last_status_log: None,
            });
        });

        sync_and_paint_strip_ui();
        if open_settings_on_start {
            open_settings_ui();
        }
        SetTimer(Some(strip_hwnd), TIMER_SYNC_ID, TIMER_SYNC_MS, None);
        SetTimer(Some(strip_hwnd), TIMER_ZORDER_ID, TIMER_ZORDER_MS, None);

        // Re-raise the strip the instant Explorer lifts `Shell_TrayWnd` (foreground changes,
        // taskbar button title updates when switching Settings tabs, etc.) instead of waiting
        // for the next sync tick.
        let hook_fg = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        let hook_reorder = SetWinEventHook(
            EVENT_OBJECT_REORDER,
            EVENT_OBJECT_REORDER,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        log::info!("UI message loop started");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !hook_fg.is_invalid() {
            let _ = UnhookWinEvent(hook_fg);
        }
        if !hook_reorder.is_invalid() {
            let _ = UnhookWinEvent(hook_reorder);
        }
        STRIP_VISIBLE_FLAG.store(false, Ordering::Release);

        STATE.with(|cell| {
            if let Some(s) = cell.borrow_mut().take() {
                let _ = DestroyIcon(s.tray_icon);
            }
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_bridge_keeps_card_open_across_gap_and_closes_outside() {
        let card = RECT {
            left: 100,
            top: 600,
            right: 452,
            bottom: 980,
        };
        let pill = RECT {
            left: 160,
            top: 992,
            right: 390,
            bottom: 1036,
        };

        // Inside the card.
        assert!(cursor_in_hover_bridge(POINT { x: 250, y: 800 }, card, pill));
        // Inside the taskbar pill.
        assert!(cursor_in_hover_bridge(
            POINT { x: 250, y: 1015 },
            card,
            pill
        ));
        // Inside the 12px vertical gap between card bottom and pill top.
        assert!(cursor_in_hover_bridge(POINT { x: 250, y: 986 }, card, pill));
        // Diagonal travel between pill and wider card edge inside bridge.
        assert!(cursor_in_hover_bridge(POINT { x: 110, y: 986 }, card, pill));
        // Far away to the right or above the card.
        assert!(!cursor_in_hover_bridge(
            POINT { x: 600, y: 800 },
            card,
            pill
        ));
        assert!(!cursor_in_hover_bridge(
            POINT { x: 250, y: 500 },
            card,
            pill
        ));
    }
}
