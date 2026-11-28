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
use crate::context_menu::{
    ContextMenuAction, ContextMenuItem, ContextMenuWindow, MenuAnchor, MenuItemTone,
    MenuKeyOutcome, WM_APP_CONTEXT_MENU_CLICK, WM_APP_CONTEXT_MENU_DISMISS,
    WM_APP_CONTEXT_MENU_KEY, WM_APP_CONTEXT_MENU_LEAVE, WM_APP_CONTEXT_MENU_MOVE,
};
use crate::layout::{
    compute_strip_layout, exhausted_until_ms, PillSlot, PillStyle, StripLayout, StripMode,
};
use crate::poller::{PollerCmd, SharedSnapshot, WM_APP_SNAPSHOT};
use crate::render::{compute_update_flash_frame, Renderer};
use crate::settings_dialog;
use crate::taskbar::{query_taskbar, StableStripAnchor, TaskbarMetrics, TaskbarTheme};
use crate::tooltip::{
    pill_screen_rect_px, HoverCardAction, TooltipWindow, WM_APP_HOVER_CARD_CLICK,
    WM_APP_HOVER_CARD_LEAVE, WM_APP_HOVER_CARD_MOVE, WM_APP_HOVER_CARD_RCLICK,
};
use crate::updater::UpdaterHandle;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usagedeck_core::default_data_dir;
use usagedeck_core::providers::antigravity::AntigravitySetupIssue;
use usagedeck_core::store::{
    CredentialVault, Store, ThemeMode, WidgetProvider, DEFAULT_PROVIDER_ORDER,
};
use usagedeck_core::view::{cached_settings_snapshot, ProviderItem, ProviderKind};
use usagedeck_core::win::{fullscreen, launch};
use windows::core::{w, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetWindow, KillTimer, LoadCursorW, LoadIconW,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetTimer,
    SetWindowPos, ShowWindow, TranslateMessage, CS_DBLCLKS, GW_HWNDPREV, HICON, HWND_TOPMOST,
    IDC_HAND, IDI_APPLICATION, LR_DEFAULTCOLOR, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_CONTEXTMENU,
    WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN,
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
const DRAG_THRESHOLD_DIP: f32 = 6.0;

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
    context_menu: Option<ContextMenuWindow>,
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
    /// Target `(pill_idx, is_system)` for which hover-card reopen is suppressed until the
    /// cursor moves to a different target or leaves the strip (prevents hover-card pop-back
    /// right after closing a context menu).
    suppress_hover_target: Option<(Option<usize>, bool)>,
    suppress_next_lbuttonup: bool,
    drag_source_pill: Option<usize>,
    drag_start_x_dip: f32,
    drag_active: bool,
    drag_target_slot: Option<usize>,
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
    let Some((strip_hwnd, menu_to_hide, poller_tx, updater_for_btn)) = try_with_state(|state| {
        clear_hover_card_state(state);
        let menu_hwnd = state
            .context_menu
            .as_mut()
            .and_then(ContextMenuWindow::prepare_hide);
        state.updater.set_pause_auto_install(true);
        (
            state.strip_hwnd,
            menu_hwnd,
            state.poller_tx.clone(),
            state.updater.clone(),
        )
    }) else {
        return;
    };
    if let Some(menu_hwnd) = menu_to_hide {
        ContextMenuWindow::hide_hwnd(menu_hwnd);
    }

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
    let Some((strip_hwnd, menu_to_hide, poller_tx, updater_for_btn)) = try_with_state(|state| {
        clear_hover_card_state(state);
        let menu_hwnd = state
            .context_menu
            .as_mut()
            .and_then(ContextMenuWindow::prepare_hide);
        state.updater.set_pause_auto_install(true);
        (
            state.strip_hwnd,
            menu_hwnd,
            state.poller_tx.clone(),
            state.updater.clone(),
        )
    }) else {
        return;
    };
    if let Some(menu_hwnd) = menu_to_hide {
        ContextMenuWindow::hide_hwnd(menu_hwnd);
    }

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

fn launch_antigravity_setup_ui(sign_in_only: bool, anchor: Option<RECT>) {
    let strip_hwnd_raw = try_with_state(|s| s.strip_hwnd.0 as isize).unwrap_or(0);
    let res = if sign_in_only {
        usagedeck_core::providers::antigravity::launch_agy_signin_terminal()
    } else {
        usagedeck_core::providers::antigravity::launch_agy_install_terminal()
    };
    match res {
        Ok(()) => {
            try_with_state(|s| {
                if let Some(ref tx) = s.poller_tx {
                    let _ = tx.send(PollerCmd::WatchAntigravitySetup);
                }
            });
            if strip_hwnd_raw != 0 {
                let msg = if sign_in_only {
                    "Opened Antigravity CLI sign-in — UsageDeck will refresh automatically"
                } else {
                    "Opened Antigravity CLI installer — UsageDeck will refresh automatically"
                };
                post_toast(strip_hwnd_raw, msg.to_string(), anchor);
            }
        }
        Err(e) => {
            log::warn!("antigravity guided setup failed: {e}");
            if strip_hwnd_raw != 0 {
                post_toast(
                    strip_hwnd_raw,
                    format!("Could not launch Antigravity setup: {e}"),
                    anchor,
                );
            }
        }
    }
}

fn launch_provider_from_strip(provider: ProviderKind, watch_cursor: bool, anchor: Option<RECT>) {
    let strip_hwnd_raw = try_with_state(|s| s.strip_hwnd.0 as isize).unwrap_or(0);
    let poller_tx = if watch_cursor {
        try_with_state(|s| s.poller_tx.clone()).flatten()
    } else {
        None
    };
    std::thread::spawn(move || match launch::open_provider_app_or_web(provider) {
        Ok(outcome) => {
            if let Some(tx) = poller_tx {
                let _ = tx.send(PollerCmd::WatchCursorSignIn);
            }
            if let launch::LaunchOutcome::OpenedWeb(_) = outcome {
                if strip_hwnd_raw != 0 {
                    post_toast(
                        strip_hwnd_raw,
                        format!("Opened {} dashboard", provider.display_name()),
                        anchor,
                    );
                }
            }
        }
        Err(e) => {
            log::warn!(
                "double-click launch {} failed: {e}",
                provider.display_name()
            );
            if strip_hwnd_raw != 0 {
                post_toast(
                    strip_hwnd_raw,
                    format!("Could not open {}", provider.display_name()),
                    anchor,
                );
            }
        }
    });
}

fn launch_task_manager_from_strip(anchor: Option<RECT>) {
    let strip_hwnd_raw = try_with_state(|s| s.strip_hwnd.0 as isize).unwrap_or(0);
    std::thread::spawn(move || {
        if let Err(e) = launch::open_task_manager() {
            log::warn!("double-click Task Manager failed: {e}");
            if strip_hwnd_raw != 0 {
                post_toast(strip_hwnd_raw, "Could not open Task Manager", anchor);
            }
        }
    });
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

/// Given the visible taskbar pill slots, the dragged slot's index `src_pos`, and the cursor's
/// horizontal strip coordinate `dx` (in DIPs), compute which slot index the card will drop onto.
fn compute_drag_target_slot(slots: &[PillSlot], src_pos: usize, dx: f32) -> usize {
    if slots.is_empty() || src_pos >= slots.len() {
        return src_pos;
    }
    for (i, slot) in slots.iter().enumerate().take(src_pos) {
        let mid = slot.rect.x + slot.rect.w * 0.5;
        if dx <= mid {
            return i;
        }
    }
    for i in ((src_pos + 1)..slots.len()).rev() {
        let mid = slots[i].rect.x + slots[i].rect.w * 0.5;
        if dx >= mid {
            return i;
        }
    }
    src_pos
}

/// Move `src` in `order` directly before (`insert_after == false`) or after
/// (`insert_after == true`) `dst`. Returns `true` if `order` changed.
fn reorder_provider_order(
    order: &mut Vec<WidgetProvider>,
    src: WidgetProvider,
    dst: WidgetProvider,
    insert_after: bool,
) -> bool {
    if src == dst {
        return false;
    }
    for wp in DEFAULT_PROVIDER_ORDER {
        if !order.contains(&wp) {
            order.push(wp);
        }
    }
    let before = order.clone();
    let Some(src_idx) = order.iter().position(|&p| p == src) else {
        return false;
    };
    order.remove(src_idx);
    let Some(dst_idx) = order.iter().position(|&p| p == dst) else {
        *order = before;
        return false;
    };
    let insert_idx = if insert_after { dst_idx + 1 } else { dst_idx };
    order.insert(insert_idx, src);
    *order != before
}

/// Optimistically update the live taskbar snapshot order and persist `settings.provider_order`
/// on a background thread so dragging cards feels instantaneous.
fn reorder_providers_ui(src: ProviderKind, dst: ProviderKind, insert_after: bool) {
    if src == dst {
        return;
    }
    let updated_live =
        try_with_state(|state| state.shared.reorder_provider(src, dst, insert_after))
            .unwrap_or(false);
    if !updated_live {
        return;
    }
    let src_wp = src.to_widget_provider();
    let dst_wp = dst.to_widget_provider();
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
            reorder_provider_order(&mut s.provider_order, src_wp, dst_wp, insert_after);
            s
        }));
    });
}

fn clear_drag_state(state: &mut AppState) {
    state.drag_source_pill = None;
    state.drag_start_x_dip = 0.0;
    state.drag_active = false;
    state.drag_target_slot = None;
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
    let Some((strip_hwnd, menu_to_hide)) = try_with_state(|state| {
        hide_tooltip(state);
        let menu_hwnd = state
            .context_menu
            .as_mut()
            .and_then(ContextMenuWindow::prepare_hide);
        state.updater.stop();
        if let Some(ref tx) = state.poller_tx {
            let _ = tx.send(PollerCmd::Stop);
        }
        (state.strip_hwnd, menu_hwnd)
    }) else {
        return;
    };
    if let Some(menu_hwnd) = menu_to_hide {
        ContextMenuWindow::hide_hwnd(menu_hwnd);
    }

    remove_tray_icon(strip_hwnd);
    // SAFETY: destroys our strip window outside any `STATE` borrow and exits the message loop cleanly.
    unsafe {
        let _ = DestroyWindow(strip_hwnd);
        PostQuitMessage(0);
    }
}

fn is_context_menu_open(state: &AppState) -> bool {
    state
        .context_menu
        .as_ref()
        .is_some_and(ContextMenuWindow::is_open)
}

fn dismiss_context_menu_ui() -> bool {
    let to_hide = try_with_state(|state| {
        state
            .context_menu
            .as_mut()
            .and_then(ContextMenuWindow::prepare_hide)
    })
    .flatten();
    if let Some(menu_hwnd) = to_hide {
        ContextMenuWindow::hide_hwnd(menu_hwnd);
        true
    } else {
        false
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
    if is_context_menu_open(state) {
        clear_hover_card_state(state);
        return;
    }
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
        let drag_state = if state.drag_active {
            state.drag_source_pill.zip(state.drag_target_slot)
        } else {
            None
        };
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
            drag_state,
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
                if !s.strip_visible || is_context_menu_open(s) {
                    s.pending_hover_pill = None;
                    s.pending_hover_system = false;
                    return false;
                }
                if let Some(idx) = s.pending_hover_pill.take() {
                    if s.strip_hovered_pill == Some(idx)
                        && s.suppress_hover_target != Some((Some(idx), false))
                    {
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
                    if s.strip_hovered_system && s.suppress_hover_target != Some((None, true)) {
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
                    HoverCardAction::InstallAntigravityCli => {
                        let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                        try_with_state(clear_hover_card_state);
                        launch_antigravity_setup_ui(false, anchor);
                        sync_and_paint_strip_ui();
                    }
                    HoverCardAction::SignInAntigravityCli => {
                        let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                        try_with_state(clear_hover_card_state);
                        launch_antigravity_setup_ui(true, anchor);
                        sync_and_paint_strip_ui();
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_HOVER_CARD_RCLICK => {
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let target = try_with_state(|s| {
                let active_pill = s.active_hover_pill;
                let active_sys = s.active_hover_system;
                s.suppress_hover_target = Some((active_pill, active_sys));
                clear_hover_card_state(s);
                (active_pill, active_sys)
            });
            if let Some((active_pill, active_sys)) = target {
                if let Some(pill_idx) = active_pill {
                    show_pill_context_menu(pill_idx, Some(MenuAnchor::AtCursor { pt }));
                } else if active_sys {
                    show_system_context_menu(Some(MenuAnchor::AtCursor { pt }));
                } else {
                    show_tray_context_menu(pt);
                }
            }
            LRESULT(0)
        }
        WM_APP_CONTEXT_MENU_MOVE => {
            let x_px = (lparam.0 & 0xffff) as i16 as f32;
            let y_px = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            try_with_state(|s| {
                if let Some(ref mut menu) = s.context_menu {
                    menu.handle_mouse_move(x_px, y_px);
                }
            });
            LRESULT(0)
        }
        WM_APP_CONTEXT_MENU_LEAVE => {
            try_with_state(|s| {
                if let Some(ref mut menu) = s.context_menu {
                    menu.handle_mouse_leave();
                }
            });
            LRESULT(0)
        }
        WM_APP_CONTEXT_MENU_CLICK => {
            let x_px = (lparam.0 & 0xffff) as i16 as f32;
            let y_px = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
            let picked = try_with_state(|s| {
                let menu = s.context_menu.as_mut()?;
                let action = menu.handle_click(x_px, y_px)?;
                let anchor = menu.anchor_rect();
                let hide_hwnd = menu.prepare_hide();
                Some((action, anchor, hide_hwnd))
            })
            .flatten();
            if let Some((action, anchor, hide_hwnd)) = picked {
                if let Some(menu_hwnd) = hide_hwnd {
                    ContextMenuWindow::hide_hwnd(menu_hwnd);
                }
                dispatch_context_menu_action(action, anchor);
            }
            LRESULT(0)
        }
        WM_APP_CONTEXT_MENU_KEY => {
            let vk = (wparam.0 & 0xffff) as u16;
            enum KeyDispatch {
                None,
                Dismiss(Option<HWND>),
                Activate(ContextMenuAction, Option<RECT>, Option<HWND>),
            }
            let outcome = try_with_state(|s| {
                let Some(ref mut menu) = s.context_menu else {
                    return KeyDispatch::None;
                };
                match menu.handle_key(vk) {
                    MenuKeyOutcome::Ignored | MenuKeyOutcome::SelectionChanged => KeyDispatch::None,
                    MenuKeyOutcome::Dismiss => KeyDispatch::Dismiss(menu.prepare_hide()),
                    MenuKeyOutcome::Activate(action) => {
                        let anchor = menu.anchor_rect();
                        KeyDispatch::Activate(action, anchor, menu.prepare_hide())
                    }
                }
            })
            .unwrap_or(KeyDispatch::None);

            match outcome {
                KeyDispatch::None => {}
                KeyDispatch::Dismiss(hide_hwnd) => {
                    if let Some(menu_hwnd) = hide_hwnd {
                        ContextMenuWindow::hide_hwnd(menu_hwnd);
                    }
                    ensure_strip_above_tray_now();
                    post_ensure_zorder();
                }
                KeyDispatch::Activate(action, anchor, hide_hwnd) => {
                    if let Some(menu_hwnd) = hide_hwnd {
                        ContextMenuWindow::hide_hwnd(menu_hwnd);
                    }
                    dispatch_context_menu_action(action, anchor);
                }
            }
            LRESULT(0)
        }
        WM_APP_CONTEXT_MENU_DISMISS => {
            if ContextMenuWindow::is_current_generation(wparam.0) && dismiss_context_menu_ui() {
                ensure_strip_above_tray_now();
                post_ensure_zorder();
            }
            LRESULT(0)
        }
        WM_APP_QUIT => {
            quit_app_ui();
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if dismiss_context_menu_ui() {
                try_with_state(|s| {
                    s.suppress_next_lbuttonup = true;
                });
                return LRESULT(0);
            }
            let should_capture = try_with_state(|s| {
                let scale = s.metrics.map_or(1.0, |m| m.scale);
                let (dx, dy) = cursor_dip(lparam, scale);
                let layout = s.strip_layout.as_ref()?;
                let StripMode::Pills { slots, .. } = &layout.mode else {
                    return None;
                };
                if slots.len() < 2 {
                    return None;
                }
                if layout.hit_test_pill_hide(dx, dy).is_some() {
                    return None;
                }
                let hit_pill = layout.hit_test_pill(dx, dy)?;
                if let Some(icon_idx) = layout.hit_test_pill_icon(dx, dy) {
                    let (snapshot, _) = s.shared.get();
                    if let Some(item) = snapshot.items.get(icon_idx) {
                        if item.provider == ProviderKind::OpenCode
                            || (item.provider == ProviderKind::Cursor && item.can_open_cursor)
                        {
                            return None;
                        }
                    }
                }
                s.drag_source_pill = Some(hit_pill);
                s.drag_start_x_dip = dx;
                s.drag_active = false;
                s.drag_target_slot = None;
                Some(())
            })
            .flatten()
            .is_some();
            if should_capture {
                let _ = SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            let needs_repaint = try_with_state(|s| {
                let was_active = s.drag_active;
                clear_drag_state(s);
                was_active
            })
            .unwrap_or(false);
            if needs_repaint {
                sync_and_paint_strip_ui();
            }
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

                // Horizontal drag-to-reorder tracking for taskbar provider cards.
                if let Some(src_item) = s.drag_source_pill {
                    let mut just_started = false;
                    if !s.drag_active {
                        if (dx - s.drag_start_x_dip).abs() < DRAG_THRESHOLD_DIP {
                            return (false, TimerOp::None, TimerOp::None);
                        }
                        s.drag_active = true;
                        just_started = true;
                        clear_hover_card_state(s);
                    }
                    let new_target = s.strip_layout.as_ref().and_then(|l| match &l.mode {
                        StripMode::Pills { slots, .. } => slots
                            .iter()
                            .position(|sl| sl.item_index == src_item)
                            .map(|src_pos| compute_drag_target_slot(slots, src_pos, dx)),
                        _ => None,
                    });
                    let changed = just_started || s.drag_target_slot != new_target;
                    s.drag_target_slot = new_target;
                    return (changed, TimerOp::Cancel, TimerOp::Cancel);
                }

                let layout = s.strip_layout.as_ref();
                let hit_pill = layout.and_then(|l| l.hit_test_pill(dx, dy));
                let hit_icon = layout.and_then(|l| l.hit_test_pill_icon(dx, dy));
                let hit_hide = layout.and_then(|l| l.hit_test_pill_hide(dx, dy));
                let hit_system = layout.is_some_and(|l| l.hit_test_system(dx, dy));

                // Never arm or open a hover card while our Fluent context menu is active.
                if is_context_menu_open(s) {
                    s.pending_hover_pill = None;
                    s.pending_hover_system = false;
                    return (false, TimerOp::Cancel, TimerOp::Cancel);
                }

                // Clear post-menu hover suppression as soon as the cursor moves to a new target.
                let current_target = (hit_pill, hit_system);
                if s.suppress_hover_target.is_some()
                    && s.suppress_hover_target != Some(current_target)
                {
                    s.suppress_hover_target = None;
                }
                let hover_suppressed = s.suppress_hover_target == Some(current_target)
                    && (hit_pill.is_some() || hit_system);

                let mut open_op = TimerOp::None;
                let mut close_op = TimerOp::None;
                let prev_active = s.active_hover_pill;
                let prev_active_sys = s.active_hover_system;
                let card_already_open = prev_active.is_some() || prev_active_sys;

                if hover_suppressed {
                    if s.pending_hover_pill.is_some() || s.pending_hover_system {
                        s.pending_hover_pill = None;
                        s.pending_hover_system = false;
                        open_op = TimerOp::Cancel;
                    }
                } else if let Some(idx) = hit_pill {
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
                s.suppress_hover_target = None;
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
        WM_LBUTTONDBLCLK => {
            let was_capturing = try_with_state(|s| {
                let c = s.drag_source_pill.is_some();
                clear_drag_state(s);
                c
            })
            .unwrap_or(false);
            if was_capturing {
                let _ = ReleaseCapture();
            }
            if dismiss_context_menu_ui() {
                return LRESULT(0);
            }
            enum DblClickAction {
                None,
                LaunchProvider {
                    provider: ProviderKind,
                    watch_cursor: bool,
                    anchor: Option<RECT>,
                },
                LaunchTaskManager {
                    anchor: Option<RECT>,
                },
            }

            let action = try_with_state(|s| {
                let scale = s.metrics.map_or(1.0, |m| m.scale);
                let (dx, dy) = cursor_dip(lparam, scale);
                let (sx, sy, _, _) = s.strip_pos;

                // Never treat a double-click on the tiny exhausted `×` chip as an app launch.
                if s.strip_layout
                    .as_ref()
                    .and_then(|l| l.hit_test_pill_hide(dx, dy))
                    .is_some()
                {
                    return DblClickAction::None;
                }

                if let Some(item_idx) = s
                    .strip_layout
                    .as_ref()
                    .and_then(|l| l.hit_test_pill(dx, dy))
                {
                    let (snapshot, _) = s.shared.get();
                    if let Some(item) = snapshot.items.get(item_idx) {
                        let anchor = s
                            .strip_layout
                            .as_ref()
                            .and_then(|l| l.pill_rect(item_idx))
                            .map(|r| pill_screen_rect_px(sx, sy, r, scale));
                        s.pending_hover_pill = None;
                        s.pending_hover_system = false;
                        s.active_hover_pill = None;
                        s.active_hover_system = false;
                        s.hover_card_hovered = false;
                        s.hover_dismiss_at = None;
                        s.suppress_next_lbuttonup = true;
                        return DblClickAction::LaunchProvider {
                            provider: item.provider,
                            watch_cursor: item.provider == ProviderKind::Cursor
                                && item.can_open_cursor,
                            anchor,
                        };
                    }
                }

                if s.strip_layout
                    .as_ref()
                    .is_some_and(|l| l.hit_test_system(dx, dy))
                {
                    let anchor = s
                        .strip_layout
                        .as_ref()
                        .and_then(|l| l.system_rect())
                        .map(|r| pill_screen_rect_px(sx, sy, r, scale));
                    s.pending_hover_pill = None;
                    s.pending_hover_system = false;
                    s.active_hover_pill = None;
                    s.active_hover_system = false;
                    s.hover_card_hovered = false;
                    s.hover_dismiss_at = None;
                    s.suppress_next_lbuttonup = true;
                    return DblClickAction::LaunchTaskManager { anchor };
                }

                DblClickAction::None
            })
            .unwrap_or(DblClickAction::None);

            match action {
                DblClickAction::None => {}
                DblClickAction::LaunchProvider {
                    provider,
                    watch_cursor,
                    anchor,
                } => {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                    sync_and_paint_strip_ui();
                    launch_provider_from_strip(provider, watch_cursor, anchor);
                }
                DblClickAction::LaunchTaskManager { anchor } => {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                    sync_and_paint_strip_ui();
                    launch_task_manager_from_strip(anchor);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (was_capturing, was_active_drag, drag_move) = try_with_state(|s| {
                let capturing = s.drag_source_pill.is_some();
                let active = s.drag_active;
                let move_op = if active {
                    if let (Some(src_idx), Some(dst_slot_pos)) =
                        (s.drag_source_pill, s.drag_target_slot)
                    {
                        s.strip_layout.as_ref().and_then(|l| match &l.mode {
                            StripMode::Pills { slots, .. } => {
                                let src_slot_pos =
                                    slots.iter().position(|sl| sl.item_index == src_idx)?;
                                let dst_slot = slots.get(dst_slot_pos)?;
                                if src_slot_pos == dst_slot_pos {
                                    return None;
                                }
                                let (snap, _) = s.shared.get();
                                let src_prov = snap.items.get(src_idx)?.provider;
                                let dst_prov = snap.items.get(dst_slot.item_index)?.provider;
                                Some((src_prov, dst_prov, dst_slot_pos > src_slot_pos))
                            }
                            _ => None,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                clear_drag_state(s);
                (capturing, active, move_op)
            })
            .unwrap_or((false, false, None));
            if was_capturing {
                let _ = ReleaseCapture();
            }
            if was_active_drag {
                if let Some((src_prov, dst_prov, insert_after)) = drag_move {
                    reorder_providers_ui(src_prov, dst_prov, insert_after);
                }
                sync_and_paint_strip_ui();
                return LRESULT(0);
            }
            if dismiss_context_menu_ui() {
                return LRESULT(0);
            }
            let action = try_with_state(|s| {
                s.suppress_hover_target = None;
                if s.suppress_next_lbuttonup {
                    s.suppress_next_lbuttonup = false;
                    return StripClickAction::None;
                }
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
        WM_RBUTTONDOWN => {
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
            let _ = dismiss_context_menu_ui();
            let was_capturing = try_with_state(|s| {
                let c = s.drag_source_pill.is_some();
                clear_drag_state(s);
                clear_hover_card_state(s);
                c
            })
            .unwrap_or(false);
            if was_capturing {
                let _ = ReleaseCapture();
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
            let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
            let target = try_with_state(|s| {
                clear_hover_card_state(s);
                let scale = s.metrics.map_or(1.0, |m| m.scale);
                let (dx, dy) = cursor_dip(lparam, scale);
                let layout = s.strip_layout.as_ref();
                let hit_pill = layout.and_then(|l| l.hit_test_pill(dx, dy));
                let hit_system = layout.is_some_and(|l| l.hit_test_system(dx, dy));
                if hit_pill.is_some() || hit_system {
                    s.suppress_hover_target = Some((hit_pill, hit_system));
                }
                (hit_pill, hit_system)
            });
            if let Some((hit_pill, hit_system)) = target {
                if let Some(idx) = hit_pill {
                    show_pill_context_menu(idx, None);
                } else if hit_system {
                    show_system_context_menu(None);
                } else {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    show_tray_context_menu(pt);
                }
            }
            LRESULT(0)
        }
        WM_TRAY_ICON => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => {
                    let _ = dismiss_context_menu_ui();
                    open_settings_ui();
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_OPEN_ID);
                    let _ = KillTimer(Some(hwnd), TIMER_HOVER_CLOSE_ID);
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    show_tray_context_menu(pt);
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

fn update_menu_item(ready_ver: Option<&str>) -> ContextMenuItem {
    match ready_ver {
        Some(ver) => ContextMenuItem::Action {
            label: format!("Install update v{ver}"),
            action: ContextMenuAction::CheckOrInstallUpdate,
            tone: MenuItemTone::Accent,
        },
        None => ContextMenuItem::Action {
            label: "Check for updates".to_string(),
            action: ContextMenuAction::CheckOrInstallUpdate,
            tone: MenuItemTone::Normal,
        },
    }
}

fn build_provider_menu_items(
    item: &ProviderItem,
    updated_at_ms: i64,
    ready_ver: Option<&str>,
) -> Vec<ContextMenuItem> {
    let provider = item.provider;
    let header = if item.title.trim().is_empty() {
        provider.display_name().to_string()
    } else {
        item.title.clone()
    };

    let mut items = vec![
        ContextMenuItem::Header(header),
        ContextMenuItem::Action {
            label: format!("Open {}", provider.display_name()),
            action: ContextMenuAction::OpenProvider {
                provider,
                watch_cursor: provider == ProviderKind::Cursor && item.can_open_cursor,
            },
            tone: MenuItemTone::Normal,
        },
    ];

    if let Some(issue) = item.antigravity_setup_issue {
        match issue {
            AntigravitySetupIssue::NeedsAgyCli => items.push(ContextMenuItem::Action {
                label: "Install Antigravity CLI".to_string(),
                action: ContextMenuAction::InstallAntigravityCli,
                tone: MenuItemTone::Accent,
            }),
            AntigravitySetupIssue::NeedsAgySignIn => items.push(ContextMenuItem::Action {
                label: "Sign in to Antigravity".to_string(),
                action: ContextMenuAction::SignInAntigravityCli,
                tone: MenuItemTone::Accent,
            }),
        }
    } else if provider == ProviderKind::OpenCode && !item.account_id.is_empty() {
        items.push(ContextMenuItem::Action {
            label: "Copy API key".to_string(),
            action: ContextMenuAction::CopyOpenCodeKey(item.account_id.clone()),
            tone: MenuItemTone::Normal,
        });
    } else if let Some(until_ms) = exhausted_until_ms(item, updated_at_ms) {
        items.push(ContextMenuItem::Action {
            label: "Hide until reset".to_string(),
            action: ContextMenuAction::HideUntilReset {
                provider,
                account_id: item.account_id.clone(),
                title: item.title.clone(),
                until_ms,
            },
            tone: MenuItemTone::Normal,
        });
    }

    items.push(ContextMenuItem::Action {
        label: "Settings".to_string(),
        action: ContextMenuAction::OpenProviderSettings(provider),
        tone: MenuItemTone::Normal,
    });
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Action {
        label: "Refresh now".to_string(),
        action: ContextMenuAction::RefreshNow,
        tone: MenuItemTone::Normal,
    });
    items.push(update_menu_item(ready_ver));
    items.push(ContextMenuItem::Action {
        label: "Quit UsageDeck".to_string(),
        action: ContextMenuAction::QuitApp,
        tone: MenuItemTone::Danger,
    });
    items
}

fn build_system_menu_items(ready_ver: Option<&str>) -> Vec<ContextMenuItem> {
    vec![
        ContextMenuItem::Header("System".to_string()),
        ContextMenuItem::Action {
            label: "Open Task Manager".to_string(),
            action: ContextMenuAction::OpenTaskManager,
            tone: MenuItemTone::Normal,
        },
        ContextMenuItem::Action {
            label: "Widget settings".to_string(),
            action: ContextMenuAction::OpenWidgetSettings,
            tone: MenuItemTone::Normal,
        },
        ContextMenuItem::Separator,
        ContextMenuItem::Action {
            label: "Refresh now".to_string(),
            action: ContextMenuAction::RefreshNow,
            tone: MenuItemTone::Normal,
        },
        update_menu_item(ready_ver),
        ContextMenuItem::Action {
            label: "Quit UsageDeck".to_string(),
            action: ContextMenuAction::QuitApp,
            tone: MenuItemTone::Danger,
        },
    ]
}

fn build_tray_menu_items(ready_ver: Option<&str>) -> Vec<ContextMenuItem> {
    vec![
        ContextMenuItem::Header(format!("UsageDeck v{}", env!("CARGO_PKG_VERSION"))),
        ContextMenuItem::Action {
            label: "Settings".to_string(),
            action: ContextMenuAction::OpenSettings,
            tone: MenuItemTone::Normal,
        },
        ContextMenuItem::Action {
            label: "Refresh now".to_string(),
            action: ContextMenuAction::RefreshNow,
            tone: MenuItemTone::Normal,
        },
        ContextMenuItem::Action {
            label: "Copy diagnostics".to_string(),
            action: ContextMenuAction::CopyDiagnostics,
            tone: MenuItemTone::Normal,
        },
        update_menu_item(ready_ver),
        ContextMenuItem::Separator,
        ContextMenuItem::Action {
            label: "Quit UsageDeck".to_string(),
            action: ContextMenuAction::QuitApp,
            tone: MenuItemTone::Danger,
        },
    ]
}

fn show_pill_context_menu(pill_idx: usize, override_anchor: Option<MenuAnchor>) {
    let menu_hwnd = try_with_state(|s| {
        clear_hover_card_state(s);
        let (snapshot, _) = s.shared.get();
        let item = snapshot.items.get(pill_idx)?;
        let ready = s.updater.ready_version();
        let items = build_provider_menu_items(item, snapshot.updated_at_ms, ready.as_deref());
        let metrics = s.metrics.or_else(query_taskbar);
        let (dpi, theme, pill_anchor_rect, anchor) = if let Some(m) = metrics {
            let theme = effective_theme(snapshot.theme_mode, m.theme);
            let (sx, sy, _, _) = s.strip_pos;
            let pill_rect = s
                .strip_layout
                .as_ref()
                .and_then(|l| l.pill_rect(pill_idx))
                .map(|r| pill_screen_rect_px(sx, sy, r, m.scale));
            let anchor = override_anchor.unwrap_or_else(|| match pill_rect {
                Some(rect) => MenuAnchor::AboveRect {
                    rect,
                    tray_top: m.tray_rect.top,
                    monitor_rect: m.monitor_rect,
                },
                None => {
                    let mut pt = POINT::default();
                    // SAFETY: standard Win32 cursor query.
                    unsafe {
                        let _ = GetCursorPos(&mut pt);
                    }
                    MenuAnchor::AtCursor { pt }
                }
            });
            (m.dpi, theme, pill_rect, anchor)
        } else {
            let mut pt = POINT::default();
            // SAFETY: standard Win32 cursor query.
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            (
                96,
                effective_theme(snapshot.theme_mode, TaskbarTheme::Dark),
                None,
                override_anchor.unwrap_or(MenuAnchor::AtCursor { pt }),
            )
        };
        s.context_menu
            .as_mut()?
            .prepare_show(&items, anchor, pill_anchor_rect, dpi, theme)
    })
    .flatten();

    if let Some(hwnd) = menu_hwnd {
        ContextMenuWindow::activate_hwnd(hwnd);
    }
}

fn show_system_context_menu(override_anchor: Option<MenuAnchor>) {
    let menu_hwnd = try_with_state(|s| {
        clear_hover_card_state(s);
        let (snapshot, _) = s.shared.get();
        let ready = s.updater.ready_version();
        let items = build_system_menu_items(ready.as_deref());
        let metrics = s.metrics.or_else(query_taskbar);
        let (dpi, theme, sys_anchor_rect, anchor) = if let Some(m) = metrics {
            let theme = effective_theme(snapshot.theme_mode, m.theme);
            let (sx, sy, _, _) = s.strip_pos;
            let sys_rect = s
                .strip_layout
                .as_ref()
                .and_then(StripLayout::system_rect)
                .map(|r| pill_screen_rect_px(sx, sy, r, m.scale));
            let anchor = override_anchor.unwrap_or_else(|| match sys_rect {
                Some(rect) => MenuAnchor::AboveRect {
                    rect,
                    tray_top: m.tray_rect.top,
                    monitor_rect: m.monitor_rect,
                },
                None => {
                    let mut pt = POINT::default();
                    // SAFETY: standard Win32 cursor query.
                    unsafe {
                        let _ = GetCursorPos(&mut pt);
                    }
                    MenuAnchor::AtCursor { pt }
                }
            });
            (m.dpi, theme, sys_rect, anchor)
        } else {
            let mut pt = POINT::default();
            // SAFETY: standard Win32 cursor query.
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }
            (
                96,
                effective_theme(snapshot.theme_mode, TaskbarTheme::Dark),
                None,
                override_anchor.unwrap_or(MenuAnchor::AtCursor { pt }),
            )
        };
        s.context_menu
            .as_mut()?
            .prepare_show(&items, anchor, sys_anchor_rect, dpi, theme)
    })
    .flatten();

    if let Some(hwnd) = menu_hwnd {
        ContextMenuWindow::activate_hwnd(hwnd);
    }
}

fn show_tray_context_menu(pt: POINT) {
    let menu_hwnd = try_with_state(|s| {
        clear_hover_card_state(s);
        let (snapshot, _) = s.shared.get();
        let ready = s.updater.ready_version();
        let items = build_tray_menu_items(ready.as_deref());
        let metrics = s.metrics.or_else(query_taskbar);
        let (dpi, theme) = match metrics {
            Some(m) => (m.dpi, effective_theme(snapshot.theme_mode, m.theme)),
            None => (96, effective_theme(snapshot.theme_mode, TaskbarTheme::Dark)),
        };
        s.context_menu
            .as_mut()?
            .prepare_show(&items, MenuAnchor::AtCursor { pt }, None, dpi, theme)
    })
    .flatten();

    if let Some(hwnd) = menu_hwnd {
        ContextMenuWindow::activate_hwnd(hwnd);
    }
}

fn copy_diagnostics_from_menu(anchor: Option<RECT>) {
    let strip_hwnd_raw = STRIP_HWND_RAW.load(Ordering::Acquire);
    std::thread::spawn(move || {
        let fallback = format!("UsageDeck v{} (Windows)", env!("CARGO_PKG_VERSION"));
        let text = default_data_dir()
            .and_then(|dir| {
                let store = Store::new(dir.clone(), Box::new(CredentialVault));
                let settings = store.get_settings();
                let snap = cached_settings_snapshot(&dir, settings);
                (!snap.diagnostics_text.is_empty()).then_some(snap.diagnostics_text)
            })
            .unwrap_or(fallback);

        let msg = match clipboard::copy_text(&text) {
            Ok(()) => "Diagnostics copied",
            Err(e) => {
                log::warn!("copy diagnostics from context menu failed: {e}");
                "Couldn't copy diagnostics"
            }
        };
        if strip_hwnd_raw != 0 {
            post_toast(strip_hwnd_raw, msg, anchor);
        }
    });
}

/// Dispatch an action selected from the Fluent context menu.
///
/// Must ALWAYS be invoked strictly outside any `STATE` `RefCell` borrow.
fn dispatch_context_menu_action(action: ContextMenuAction, anchor: Option<RECT>) {
    match action {
        ContextMenuAction::OpenProvider {
            provider,
            watch_cursor,
        } => {
            launch_provider_from_strip(provider, watch_cursor, anchor);
        }
        ContextMenuAction::OpenProviderSettings(provider) => {
            open_settings_on_page_ui(Some(provider));
        }
        ContextMenuAction::OpenTaskManager => {
            launch_task_manager_from_strip(anchor);
        }
        ContextMenuAction::OpenWidgetSettings => {
            open_settings_on_widget_ui();
        }
        ContextMenuAction::OpenSettings => {
            open_settings_ui();
        }
        ContextMenuAction::CopyOpenCodeKey(email) => {
            copy_opencode_key(email, anchor);
        }
        ContextMenuAction::InstallAntigravityCli => {
            launch_antigravity_setup_ui(false, anchor);
        }
        ContextMenuAction::SignInAntigravityCli => {
            launch_antigravity_setup_ui(true, anchor);
        }
        ContextMenuAction::HideUntilReset {
            provider,
            account_id,
            title,
            until_ms,
        } => {
            hide_target_with_toast((provider, account_id, title, until_ms), anchor);
        }
        ContextMenuAction::RefreshNow => {
            let poller_tx = try_with_state(|s| s.poller_tx.clone()).flatten();
            if let Some(ref tx) = poller_tx {
                let _ = tx.send(PollerCmd::RefreshNow);
            }
            post_toast_from_any_thread("Refreshing usage…".to_string());
        }
        ContextMenuAction::CopyDiagnostics => {
            copy_diagnostics_from_menu(anchor);
        }
        ContextMenuAction::CheckOrInstallUpdate => {
            if let Some(updater) = try_with_state(|s| s.updater.clone()) {
                if updater.ready_version().is_some() {
                    if let Err(e) = updater.install_now() {
                        log::warn!("install_now from context menu failed: {e}");
                        let _ = updater.check_now();
                    }
                } else {
                    let _ = updater.check_now();
                    post_toast_from_any_thread("Checking for updates…".to_string());
                }
            }
        }
        ContextMenuAction::QuitApp => {
            quit_app_ui();
            return;
        }
    }

    ensure_strip_above_tray_now();
    post_ensure_zorder();
}

/// Create the taskbar strip window, interactive hover card window, Fluent context menu,
/// and tray icon, then run the Win32 message loop until quit.
pub fn run_ui(
    shared: Arc<SharedSnapshot>,
    updater: UpdaterHandle,
    demo: bool,
    open_settings_on_start: bool,
    just_updated: bool,
) -> Result<()> {
    let renderer = Renderer::new()?;
    let tooltip = TooltipWindow::new().ok();
    let context_menu = ContextMenuWindow::new().ok();
    let tray_icon = load_app_icon();

    // SAFETY: Win32 class registration, window creation, timer, and standard message loop.
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let hand_cursor = LoadCursorW(None, IDC_HAND)?;

        let strip_class = w!("UsageDeckTaskbarStrip");
        let wc_strip = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
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
        if let Some(ref menu) = context_menu {
            menu.set_owner_hwnd(strip_hwnd);
        }

        let poller_tx = crate::poller::start_poller(strip_hwnd.0 as isize, shared.clone(), demo);

        STATE.with(|cell| {
            *cell.borrow_mut() = Some(AppState {
                renderer,
                tooltip,
                context_menu,
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
                suppress_hover_target: None,
                suppress_next_lbuttonup: false,
                drag_source_pill: None,
                drag_start_x_dip: 0.0,
                drag_active: false,
                drag_target_slot: None,
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

    #[test]
    fn tray_and_system_menus_have_expected_actions() {
        let tray = build_tray_menu_items(Some("2.16.5"));
        assert!(
            matches!(tray.first(), Some(ContextMenuItem::Header(h)) if h.starts_with("UsageDeck v"))
        );
        assert!(tray.iter().any(|it| matches!(
            it,
            ContextMenuItem::Action {
                action: ContextMenuAction::CopyDiagnostics,
                ..
            }
        )));
        assert!(tray.iter().any(|it| matches!(
            it,
            ContextMenuItem::Action {
                label,
                action: ContextMenuAction::CheckOrInstallUpdate,
                tone: MenuItemTone::Accent,
            } if label == "Install update v2.16.5"
        )));

        let sys = build_system_menu_items(None);
        assert!(sys.iter().any(|it| matches!(
            it,
            ContextMenuItem::Action {
                action: ContextMenuAction::OpenTaskManager,
                ..
            }
        )));
        assert!(sys.iter().any(|it| matches!(
            it,
            ContextMenuItem::Action {
                action: ContextMenuAction::OpenWidgetSettings,
                ..
            }
        )));
    }

    #[test]
    fn drag_target_slot_uses_adjacent_midpoints() {
        use crate::layout::DipRect;
        let mk = |item_index: usize, x: f32, w: f32| PillSlot {
            item_index,
            rect: DipRect {
                x,
                y: 2.0,
                w,
                h: 34.0,
            },
            icon_rect: DipRect {
                x,
                y: 2.0,
                w: 16.0,
                h: 34.0,
            },
            hide_rect: None,
            extended: false,
            style: PillStyle::WideMeters,
            meter_rects: Vec::new(),
        };
        let slots = vec![mk(0, 0.0, 100.0), mk(1, 107.0, 100.0), mk(2, 214.0, 100.0)];

        // Dragging slot 0 rightwards: mid(1) = 157, mid(2) = 264.
        assert_eq!(compute_drag_target_slot(&slots, 0, 120.0), 0);
        assert_eq!(compute_drag_target_slot(&slots, 0, 158.0), 1);
        assert_eq!(compute_drag_target_slot(&slots, 0, 265.0), 2);

        // Dragging slot 2 leftwards: mid(1) = 157, mid(0) = 50.
        assert_eq!(compute_drag_target_slot(&slots, 2, 190.0), 2);
        assert_eq!(compute_drag_target_slot(&slots, 2, 156.0), 1);
        assert_eq!(compute_drag_target_slot(&slots, 2, 48.0), 0);

        // Dragging middle slot 1 stays at 1 inside (50, 264).
        assert_eq!(compute_drag_target_slot(&slots, 1, 150.0), 1);
        assert_eq!(compute_drag_target_slot(&slots, 1, 49.0), 0);
        assert_eq!(compute_drag_target_slot(&slots, 1, 265.0), 2);
    }

    #[test]
    fn reorder_provider_order_moves_before_and_after() {
        let mut order = DEFAULT_PROVIDER_ORDER.to_vec();
        // Drag Opencode after Codex (rightward).
        assert!(reorder_provider_order(
            &mut order,
            WidgetProvider::Opencode,
            WidgetProvider::Codex,
            true
        ));
        assert_eq!(
            order,
            vec![
                WidgetProvider::Cursor,
                WidgetProvider::Grok,
                WidgetProvider::Codex,
                WidgetProvider::Opencode,
                WidgetProvider::Claude,
                WidgetProvider::Antigravity,
            ]
        );

        // Drag Antigravity before Codex (leftward).
        assert!(reorder_provider_order(
            &mut order,
            WidgetProvider::Antigravity,
            WidgetProvider::Codex,
            false
        ));
        assert_eq!(
            order,
            vec![
                WidgetProvider::Cursor,
                WidgetProvider::Grok,
                WidgetProvider::Antigravity,
                WidgetProvider::Codex,
                WidgetProvider::Opencode,
                WidgetProvider::Claude,
            ]
        );

        // Same provider is a no-op.
        assert!(!reorder_provider_order(
            &mut order,
            WidgetProvider::Codex,
            WidgetProvider::Codex,
            true
        ));
    }
}
