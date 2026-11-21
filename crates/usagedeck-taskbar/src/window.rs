//! Native Win32 windowing layer:
//! - TaskbarStripWindow (`WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST`)
//! - TooltipWindow (Direct2D hover tooltip anchored above each taskbar pill)
//! - FlyoutWindow (anchored above the right-side taskbar strip, dismisses on blur or Esc)
//! - System Tray icon (`Shell_NotifyIconW`) with native context menu & `TaskbarCreated` recovery
//! - Wiring for native Settings dialog, clipboard key copy, hide-until-reset, and auto-updater.

use crate::clipboard;
use crate::layout::{
    compute_flyout_layout, compute_strip_layout, FlyoutAction, FlyoutLayout, StripLayout, StripMode,
};
use crate::poller::{PollerCmd, SharedSnapshot, WM_APP_SNAPSHOT};
use crate::render::Renderer;
use crate::settings_dialog;
use crate::taskbar::{query_taskbar, TaskbarMetrics};
use crate::tooltip::{pill_screen_rect_px, TooltipWindow};
use crate::updater::UpdaterHandle;
use std::cell::RefCell;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use usagedeck_core::store::{CredentialVault, Store};
use usagedeck_core::view::ProviderKind;
use usagedeck_core::win::fullscreen;
use usagedeck_core::{default_data_dir, default_log_dir};
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    GetWindow, LoadCursorW, LoadIconW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TrackPopupMenu, TranslateMessage,
    GW_HWNDPREV, HICON, HWND_TOPMOST, IDC_HAND, IDI_APPLICATION, LR_DEFAULTCOLOR, MF_SEPARATOR,
    MF_STRING, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW,
    SW_SHOWNOACTIVATE, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, WA_INACTIVE,
    WM_ACTIVATE, WM_APP, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP,
    WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const WM_MOUSELEAVE: u32 = 0x02A3;

const WM_TRAY_ICON: u32 = WM_APP + 2;
const TIMER_SYNC_ID: usize = 1;
const TIMER_SYNC_MS: u32 = 300;

const MENU_TOGGLE_FLYOUT: usize = 1001;
const MENU_REFRESH: usize = 1002;
const MENU_SHOW_HIDDEN: usize = 1003;
const MENU_SETTINGS: usize = 1004;
const MENU_CHECK_UPDATES: usize = 1005;
const MENU_OPEN_DATA: usize = 1006;
const MENU_OPEN_LOGS: usize = 1007;
const MENU_QUIT: usize = 1008;

const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/icon.ico");

struct AppState {
    renderer: Renderer,
    tooltip: Option<TooltipWindow>,
    shared: Arc<SharedSnapshot>,
    poller_tx: Option<Sender<PollerCmd>>,
    updater: UpdaterHandle,
    strip_hwnd: HWND,
    flyout_hwnd: HWND,
    tray_icon: HICON,
    taskbar_created_msg: u32,
    metrics: Option<TaskbarMetrics>,
    strip_layout: Option<StripLayout>,
    flyout_layout: Option<FlyoutLayout>,
    strip_pos: (i32, i32, i32, i32),
    strip_hovered: bool,
    strip_hovered_pill: Option<usize>,
    strip_tracking_leave: bool,
    flyout_visible: bool,
    flyout_hovered_action: Option<FlyoutAction>,
    flyout_tracking_leave: bool,
    fullscreen_debounce: fullscreen::Debounce,
    strip_visible: bool,
}

thread_local! {
    static STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut AppState) -> R) -> Option<R> {
    STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        borrow.as_mut().map(f)
    })
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
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
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

fn open_folder(dir: Option<std::path::PathBuf>) {
    let Some(dir) = dir else { return };
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
}

fn open_settings(state: &mut AppState) {
    hide_flyout(state);
    hide_tooltip(state);
    let poller_tx = state.poller_tx.clone();
    let updater_for_btn = state.updater.clone();
    state.updater.set_pause_auto_install(true);

    settings_dialog::open_settings_dialog_with_updater(
        state.strip_hwnd,
        Box::new(move || {
            if let Some(ref tx) = poller_tx {
                let _ = tx.send(PollerCmd::RefreshNow);
            }
        }),
        Some(Box::new(move |_hwnd| {
            let _ = updater_for_btn.check_now();
        })),
    );
}

fn show_context_menu(state: &mut AppState) {
    hide_tooltip(state);
    // SAFETY: standard Win32 popup menu anchored at the current cursor position.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let flyout_label = if state.flyout_visible {
            w!("Hide UsageDeck")
        } else {
            w!("Show UsageDeck")
        };
        let _ = AppendMenuW(menu, MF_STRING, MENU_TOGGLE_FLYOUT, flyout_label);
        let _ = AppendMenuW(menu, MF_STRING, MENU_REFRESH, w!("Refresh now"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_SHOW_HIDDEN, w!("Show hidden cards"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS, w!("Settings..."));
        let _ = AppendMenuW(menu, MF_STRING, MENU_CHECK_UPDATES, w!("Check for updates"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN_DATA, w!("Open data folder"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN_LOGS, w!("Open log folder"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(state.strip_hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
            pt.x,
            pt.y,
            Some(0),
            state.strip_hwnd,
            None,
        );
        let _ = DestroyMenu(menu);

        match cmd.0 as usize {
            MENU_TOGGLE_FLYOUT => toggle_flyout(state),
            MENU_REFRESH => request_refresh(state),
            MENU_SHOW_HIDDEN => clear_snoozed_cards(state),
            MENU_SETTINGS => open_settings(state),
            MENU_CHECK_UPDATES => {
                open_settings(state);
                let _ = state.updater.check_now();
            }
            MENU_OPEN_DATA => open_folder(default_data_dir()),
            MENU_OPEN_LOGS => open_folder(default_log_dir()),
            MENU_QUIT => quit_app(state),
            _ => {}
        }
    }
}

fn request_refresh(state: &AppState) {
    if let Some(ref tx) = state.poller_tx {
        let _ = tx.send(PollerCmd::RefreshNow);
    }
}

fn clear_snoozed_cards(state: &AppState) {
    let poller_tx = state.poller_tx.clone();
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
            s.snoozed_until.clear();
            s
        }));
        if let Some(tx) = poller_tx {
            let _ = tx.send(PollerCmd::RefreshNow);
        }
    });
}

fn hide_card_until_reset(
    state: &AppState,
    provider: ProviderKind,
    account_id: String,
    until_ms: u64,
) {
    let poller_tx = state.poller_tx.clone();
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

fn copy_opencode_key(email: String) {
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
        let accounts = rt.block_on(store.get_accounts());
        let lower = email.to_lowercase();
        if let Some(acc) = accounts
            .into_iter()
            .find(|a| a.email.to_lowercase() == lower)
        {
            if !acc.key.is_empty() {
                if let Err(e) = clipboard::copy_text(&acc.key) {
                    log::warn!("clipboard copy failed: {e}");
                }
            }
        }
    });
}

fn quit_app(state: &mut AppState) {
    hide_tooltip(state);
    state.updater.stop();
    if let Some(ref tx) = state.poller_tx {
        let _ = tx.send(PollerCmd::Stop);
    }
    remove_tray_icon(state.strip_hwnd);
    // SAFETY: destroys our windows and terminates the Win32 message loop cleanly.
    unsafe {
        let _ = DestroyWindow(state.flyout_hwnd);
        let _ = DestroyWindow(state.strip_hwnd);
        PostQuitMessage(0);
    }
}

fn hide_tooltip(state: &mut AppState) {
    if let Some(ref mut tip) = state.tooltip {
        tip.hide();
    }
}

fn update_strip_tooltip(state: &mut AppState) {
    if state.flyout_visible || !state.strip_hovered || !state.strip_visible {
        hide_tooltip(state);
        return;
    }
    let (Some(metrics), Some(layout), Some(pill_idx)) = (
        state.metrics,
        state.strip_layout.as_ref(),
        state.strip_hovered_pill,
    ) else {
        hide_tooltip(state);
        return;
    };
    let (snapshot, _) = state.shared.get();
    let (Some(item), Some(pill_rect)) = (snapshot.items.get(pill_idx), layout.pill_rect(pill_idx))
    else {
        hide_tooltip(state);
        return;
    };
    let (sx, sy, _, _) = state.strip_pos;
    let anchor = pill_screen_rect_px(sx, sy, pill_rect, metrics.scale);
    if let Some(ref mut tip) = state.tooltip {
        tip.show(&item.tooltip, anchor, &metrics);
    }
}

fn sync_and_paint_strip(state: &mut AppState) {
    // Keep auto-updater pause flag synchronized with the native Settings window open state.
    state
        .updater
        .set_pause_auto_install(settings_dialog::is_settings_open());

    let Some(metrics) = query_taskbar() else {
        hide_strip(state);
        return;
    };
    state.metrics = Some(metrics);

    let (snapshot, loading) = state.shared.get();

    // Hide during full-screen apps (debounced over 2 ticks, reusing usagedeck_core::win::fullscreen).
    if snapshot.hide_in_fullscreen {
        let reading = fullscreen::read(Some(metrics.tray_hwnd));
        let _ = state.fullscreen_debounce.update(reading.full_screen);
    } else {
        state.fullscreen_debounce.reset();
    }

    if metrics.hidden || state.fullscreen_debounce.state() {
        hide_strip(state);
        return;
    }

    let layout = compute_strip_layout(&snapshot, metrics.available_width_dip, loading);
    if matches!(layout.mode, StripMode::Hidden) {
        state.strip_layout = Some(layout);
        hide_strip(state);
        return;
    }

    let width_px = (layout.width_dip * metrics.scale).ceil() as i32;
    let height_px = (layout.height_dip * metrics.scale).ceil() as i32;
    let (x, y) = metrics.strip_origin_px(width_px, height_px);
    state.strip_pos = (x, y, width_px, height_px);

    if let Err(e) = state.renderer.render_strip(
        state.strip_hwnd,
        x,
        y,
        metrics.dpi,
        metrics.theme,
        &snapshot,
        &layout,
        state.strip_hovered,
        state.strip_hovered_pill,
    ) {
        log::warn!("render_strip failed: {e}");
    }
    state.strip_layout = Some(layout);

    // Ensure the strip stays visible and ordered above `Shell_TrayWnd` without stealing focus.
    // SAFETY: non-activating topmost Z-order maintenance on our own HWND.
    unsafe {
        if !state.strip_visible {
            let _ = ShowWindow(state.strip_hwnd, SW_SHOWNOACTIVATE);
            state.strip_visible = true;
        }
        let needs_raise = !is_above_tray(state.strip_hwnd, metrics.tray_hwnd);
        if needs_raise {
            let _ = SetWindowPos(
                state.strip_hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    update_strip_tooltip(state);

    if state.flyout_visible {
        paint_flyout(state);
    }
}

fn hide_strip(state: &mut AppState) {
    hide_tooltip(state);
    if state.strip_visible {
        // SAFETY: hides our own strip window.
        unsafe {
            let _ = ShowWindow(state.strip_hwnd, SW_HIDE);
        }
        state.strip_visible = false;
    }
}

/// True if `strip` appears above `tray` in the Win32 Z-order chain.
unsafe fn is_above_tray(strip: HWND, tray: HWND) -> bool {
    let mut cur = tray;
    for _ in 0..32 {
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

fn toggle_flyout(state: &mut AppState) {
    if state.flyout_visible {
        hide_flyout(state);
    } else {
        show_flyout(state);
    }
}

fn show_flyout(state: &mut AppState) {
    hide_tooltip(state);
    state.flyout_visible = true;
    state.flyout_hovered_action = None;
    paint_flyout(state);
    // SAFETY: shows and focuses our own flyout so clicking outside triggers `WM_ACTIVATE(WA_INACTIVE)`.
    unsafe {
        let _ = ShowWindow(state.flyout_hwnd, SW_SHOW);
        let _ = SetForegroundWindow(state.flyout_hwnd);
    }
}

fn hide_flyout(state: &mut AppState) {
    if state.flyout_visible {
        state.flyout_visible = false;
        state.flyout_hovered_action = None;
        // SAFETY: hides our own flyout window.
        unsafe {
            let _ = ShowWindow(state.flyout_hwnd, SW_HIDE);
        }
    }
}

fn paint_flyout(state: &mut AppState) {
    let Some(metrics) = state.metrics.or_else(query_taskbar) else {
        return;
    };
    let (snapshot, _) = state.shared.get();
    let layout: FlyoutLayout = compute_flyout_layout(&snapshot);
    let fw = (layout.width_dip * metrics.scale).ceil() as i32;
    let fh = (layout.height_dip * metrics.scale).ceil() as i32;
    let (sx, _, sw, _) = state.strip_pos;
    let anchor_x = if sw > 0 {
        sx
    } else {
        metrics.notify_rect.left - fw
    };
    let anchor_w = if sw > 0 { sw } else { fw };
    let (fx, fy) = metrics.flyout_origin_px(anchor_x, anchor_w, fw, fh);

    if let Err(e) = state.renderer.render_flyout(
        state.flyout_hwnd,
        fx,
        fy,
        metrics.dpi,
        metrics.theme,
        &snapshot,
        &layout,
        state.flyout_hovered_action.as_ref(),
    ) {
        log::warn!("render_flyout failed: {e}");
    }
    state.flyout_layout = Some(layout);
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

unsafe extern "system" fn strip_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let is_taskbar_created =
        with_state(|s| s.taskbar_created_msg != 0 && msg == s.taskbar_created_msg).unwrap_or(false);
    if is_taskbar_created {
        with_state(|s| {
            add_tray_icon(s.strip_hwnd, s.tray_icon);
            sync_and_paint_strip(s);
        });
        return LRESULT(0);
    }

    match msg {
        WM_TIMER if wparam.0 == TIMER_SYNC_ID => {
            with_state(sync_and_paint_strip);
            LRESULT(0)
        }
        WM_APP_SNAPSHOT | WM_SETTINGCHANGE => {
            with_state(sync_and_paint_strip);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            with_state(|s| {
                if !s.strip_tracking_leave {
                    track_mouse_leave(hwnd);
                    s.strip_tracking_leave = true;
                }
                let scale = s.metrics.map_or(1.0, |m| m.scale);
                let (dx, dy) = cursor_dip(lparam, scale);
                let hit_pill = s
                    .strip_layout
                    .as_ref()
                    .and_then(|l| l.hit_test_pill(dx, dy));
                if !s.strip_hovered || hit_pill != s.strip_hovered_pill {
                    s.strip_hovered = true;
                    s.strip_hovered_pill = hit_pill;
                    sync_and_paint_strip(s);
                }
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_state(|s| {
                s.strip_tracking_leave = false;
                if s.strip_hovered || s.strip_hovered_pill.is_some() {
                    s.strip_hovered = false;
                    s.strip_hovered_pill = None;
                    hide_tooltip(s);
                    sync_and_paint_strip(s);
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with_state(toggle_flyout);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            with_state(show_context_menu);
            LRESULT(0)
        }
        WM_TRAY_ICON => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => {
                    with_state(toggle_flyout);
                }
                WM_RBUTTONUP => {
                    with_state(show_context_menu);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn flyout_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ACTIVATE => {
            let active_state = (wparam.0 & 0xffff) as u32;
            if active_state == WA_INACTIVE {
                with_state(hide_flyout);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 == usize::from(VK_ESCAPE.0) {
                with_state(hide_flyout);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            with_state(|s| {
                if !s.flyout_tracking_leave {
                    track_mouse_leave(hwnd);
                    s.flyout_tracking_leave = true;
                }
                let scale = s.metrics.map_or(1.0, |m| m.scale);
                let (dx, dy) = cursor_dip(lparam, scale);
                let hit = s.flyout_layout.as_ref().and_then(|l| l.hit_test(dx, dy));
                if hit != s.flyout_hovered_action {
                    s.flyout_hovered_action = hit;
                    paint_flyout(s);
                }
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_state(|s| {
                s.flyout_tracking_leave = false;
                if s.flyout_hovered_action.is_some() {
                    s.flyout_hovered_action = None;
                    paint_flyout(s);
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with_state(|s| {
                let scale = s.metrics.map_or(1.0, |m| m.scale);
                let (dx, dy) = cursor_dip(lparam, scale);
                let hit = s.flyout_layout.as_ref().and_then(|l| l.hit_test(dx, dy));
                match hit {
                    Some(FlyoutAction::Refresh) => request_refresh(s),
                    Some(FlyoutAction::OpenSettings) => open_settings(s),
                    Some(FlyoutAction::OpenDataFolder) => {
                        hide_flyout(s);
                        open_folder(default_data_dir());
                    }
                    Some(FlyoutAction::OpenCursor) => {
                        hide_flyout(s);
                        if let Err(e) = usagedeck_core::providers::cursor::open_cursor() {
                            log::warn!("open_cursor failed: {e}");
                        }
                    }
                    Some(FlyoutAction::CopyOpenCodeKey(email)) => {
                        copy_opencode_key(email);
                    }
                    Some(FlyoutAction::HideUntilReset {
                        provider,
                        account_id,
                        until_ms,
                    }) => {
                        hide_card_until_reset(s, provider, account_id, until_ms);
                    }
                    Some(FlyoutAction::Quit) => quit_app(s),
                    None => {}
                }
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Create the taskbar strip window, tooltip window, flyout window, and tray icon,
/// then run the Win32 message loop until quit.
pub fn run_ui(
    shared: Arc<SharedSnapshot>,
    updater: UpdaterHandle,
    demo: bool,
    open_settings_on_start: bool,
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

        let flyout_class = w!("UsageDeckFlyoutPanel");
        let wc_flyout = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(flyout_wnd_proc),
            hInstance: hinstance.into(),
            hCursor: hand_cursor,
            lpszClassName: flyout_class,
            ..Default::default()
        };
        RegisterClassExW(&wc_flyout);

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

        let flyout_hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            flyout_class,
            w!("UsageDeck Flyout"),
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
        add_tray_icon(strip_hwnd, tray_icon);

        let poller_tx = crate::poller::start_poller(strip_hwnd.0 as isize, shared.clone(), demo);

        STATE.with(|cell| {
            *cell.borrow_mut() = Some(AppState {
                renderer,
                tooltip,
                shared,
                poller_tx: Some(poller_tx),
                updater,
                strip_hwnd,
                flyout_hwnd,
                tray_icon,
                taskbar_created_msg,
                metrics: None,
                strip_layout: None,
                flyout_layout: None,
                strip_pos: (0, 0, 0, 0),
                strip_hovered: false,
                strip_hovered_pill: None,
                strip_tracking_leave: false,
                flyout_visible: false,
                flyout_hovered_action: None,
                flyout_tracking_leave: false,
                fullscreen_debounce: fullscreen::Debounce::default(),
                strip_visible: false,
            });
        });

        with_state(sync_and_paint_strip);
        if open_settings_on_start {
            with_state(open_settings);
        }
        SetTimer(Some(strip_hwnd), TIMER_SYNC_ID, TIMER_SYNC_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        STATE.with(|cell| {
            if let Some(s) = cell.borrow_mut().take() {
                let _ = DestroyIcon(s.tray_icon);
            }
        });
    }

    Ok(())
}
