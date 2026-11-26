//! Pure-Rust native Direct2D/DirectWrite Settings window for UsageDeck
//! (zero WebView2 / zero Tauri / zero Node).
//!
//! Faithfully recreates the v1.5.15 WinUI-inspired Settings experience:
//! - Resizable, maximizable top-level OS window (`960×680` DIP default, `640×480` DIP min)
//!   with native DWM dark/light caption bar (`DWMWA_USE_IMMERSIVE_DARK_MODE`) and
//!   persisted window placement (`settings-window.json`).
//! - Fixed `232` DIP left navigation sidebar (`General`, `Widget`, `Accounts` group in
//!   `provider_order`, plus bottom-pinned `Update available` button when ready).
//! - Scrollable right-hand content pane covering all 7 pages:
//!   `General`, `Widget`, `Claude`, `Codex`, `Cursor`, `Grok Bot`, and `OpenCode`.
//! - Instant `<16ms` open using saved `settings.json`, followed by non-blocking background
//!   `poll_settings_snapshot(&core)` loading for live account tables, Claude Spend & Pace
//!   charts, and Cursor diagnostics.
//! - Clean native Win32 modal sub-dialogs for Rename, Add/Replace OpenCode API key,
//!   Claude hourly spend target, and Remove confirmation — always launched strictly
//!   outside any `SETTINGS_STATE` `RefCell` borrow.

mod layout;
mod modals;
mod render;
pub(crate) mod theme;

use crate::autostart;
use crate::clipboard;
use layout::{
    build_layout, provider_bars, Action, LayoutInput, PageId, SettingsLayout, DEFAULT_WIN_H_DIP,
    DEFAULT_WIN_W_DIP, MIN_WIN_H_DIP, MIN_WIN_W_DIP,
};
use render::SettingsRenderer;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};
use theme::{apply_titlebar_theme, resolve_is_dark, SettingsPalette, SpendRange};
use usagedeck_core::providers::{SeenAccounts, UsageCache};
use usagedeck_core::store::{
    normalize_ymd_date, read_json, write_json_atomic, Account, CredentialVault, Settings, Store,
    WidgetProvider, OPENCODE_KEY_PREFIXES, SEEN_ACCOUNTS_FILE, USAGE_CACHE_FILE,
};
use usagedeck_core::updater::release::{is_newer, parse_release, Version};
use usagedeck_core::view::{
    cached_settings_snapshot, poll_settings_snapshot, ProviderKind, SettingsSnapshot,
};
use usagedeck_core::{default_data_dir, default_log_dir, Core};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MonitorFromRect, MonitorFromWindow, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_NEXT, VK_PRIOR, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, GetClientRect, GetCursorPos,
    GetWindowPlacement, IsZoomed, KillTimer, LoadCursorW, LoadIconW, PostMessageW,
    RegisterClassExW, SendMessageW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos,
    SetWindowTextW, ShowWindow, TrackPopupMenu, HICON, IDC_ARROW, IDC_HAND, IDI_APPLICATION,
    LR_DEFAULTCOLOR, MF_SEPARATOR, MF_STRING, MINMAXINFO, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    SW_SHOW, SW_SHOWMAXIMIZED, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN,
    WINDOWPLACEMENT, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_APP, WM_CLOSE, WM_DPICHANGED,
    WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_SETTINGCHANGE, WM_SIZE,
    WM_TIMER, WNDCLASSEXW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCACTIVATE: u32 = 0x0086;
const WM_SETFOCUS: u32 = 0x0007;
const WM_APP_UPDATE_STATUS: u32 = WM_APP + 20;
const WM_APP_SETTINGS_DATA: u32 = WM_APP + 21;
const WM_APP_UPDATE_READY: u32 = WM_APP + 22;

const TIMER_TOAST_ID: usize = 301;
const TIMER_TOAST_MS: u32 = 2500;
const TIMER_POLL_ID: usize = 302;
const TIMER_POLL_MS: u32 = 60_000;

const MENU_TOGGLE_HIDE: usize = 4001;
const MENU_RENAME: usize = 4002;
const MENU_COPY_KEY: usize = 4003;
const MENU_REPLACE_KEY: usize = 4004;
const MENU_REFRESH: usize = 4005;
const MENU_REMOVE: usize = 4006;
const MENU_TOGGLE_UNSUBSCRIBED: usize = 4007;
const MENU_SET_SUB_END: usize = 4008;

const MAX_KEY_LENGTH: usize = 512;
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/zbr79/UsageDeck/releases/latest";
const USER_AGENT: &str = "UsageDeck-updater";
const SETTINGS_WINDOW_FILE: &str = "settings-window.json";

const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/icon.ico");

static CLASS_REGISTERED: Once = Once::new();
static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);
static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
static POLL_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

static PENDING_UPDATE_TEXT: Mutex<Option<(String, bool)>> = Mutex::new(None);
static PENDING_UPDATE_READY: Mutex<Option<Option<String>>> = Mutex::new(None);
static LATEST_UPDATE_READY: Mutex<Option<String>> = Mutex::new(None);
static LATEST_UPDATE_STATUS: Mutex<Option<String>> = Mutex::new(None);
static PENDING_SETTINGS_DATA: Mutex<Option<SettingsSnapshot>> = Mutex::new(None);
static LAST_SETTINGS_SNAPSHOT: Mutex<Option<SettingsSnapshot>> = Mutex::new(None);
static SHARED_SETTINGS_CORE: OnceLock<Option<Arc<Core>>> = OnceLock::new();

struct SettingsState {
    hwnd: HWND,
    app_icon: HICON,
    dpi: u32,
    client_w_dip: f32,
    client_h_dip: f32,
    renderer: SettingsRenderer,
    store: Option<Store>,
    rt: Option<tokio::runtime::Runtime>,
    settings: Settings,
    snapshot: Option<SettingsSnapshot>,
    loading: bool,
    page: PageId,
    expanded_snoozed: Option<WidgetProvider>,
    spend_range: SpendRange,
    scroll_y: f32,
    hovered_action: Option<Action>,
    scrollbar_hovered: bool,
    scrollbar_dragging: bool,
    scrollbar_drag_offset: f32,
    tracking_leave: bool,
    update_status_text: String,
    update_checking: bool,
    update_ready_version: Option<String>,
    toast: Option<String>,
    layout: SettingsLayout,
    on_changed: Arc<dyn Fn() + Send + Sync>,
    on_check_updates: Option<Arc<dyn Fn(HWND) + Send + Sync>>,
}

impl Drop for SettingsState {
    fn drop(&mut self) {
        if !self.app_icon.is_invalid() {
            // SAFETY: destroys custom HICON loaded for this window.
            unsafe {
                let _ = DestroyIcon(self.app_icon);
            }
        }
    }
}

thread_local! {
    static SETTINGS_STATE: RefCell<Option<SettingsState>> = const { RefCell::new(None) };
}

fn try_with_state<R>(f: impl FnOnce(&mut SettingsState) -> R) -> Option<R> {
    SETTINGS_STATE.with(|cell| {
        let Ok(mut borrow) = cell.try_borrow_mut() else {
            return None;
        };
        borrow.as_mut().map(f)
    })
}

/// Returns `true` while the UsageDeck Settings window is open.
///
/// The background auto-updater checks this to defer silent restarts while the
/// user is editing preferences or API keys.
pub fn is_settings_open() -> bool {
    SETTINGS_OPEN.load(Ordering::Acquire)
}

/// Validate an OpenCode API key against the `oc_sk_` / `sk-` prefix rules from `usagedeck-core`.
pub fn is_valid_opencode_key(key: &str) -> bool {
    OPENCODE_KEY_PREFIXES
        .iter()
        .any(|p| key.strip_prefix(p).is_some_and(|rest| !rest.is_empty()))
        && key.encode_utf16().count() <= MAX_KEY_LENGTH
        && !key.chars().any(char::is_whitespace)
}

/// Update the version/update status label on an open Settings window from any thread.
pub fn set_update_status_text(text: &str) {
    if let Ok(mut slot) = LATEST_UPDATE_STATUS.lock() {
        *slot = Some(text.to_string());
    }
    post_update_status(text.to_string(), false);
}

/// Signal whether an update is downloaded and ready to install from any thread.
///
/// Lights up the emerald dot on the `General` nav item, shows the bottom-pinned
/// `Update available` sidebar button, and switches the General page button to primary.
pub fn set_update_ready(version: Option<&str>) {
    let owned = version.map(str::to_string);
    if let Ok(mut latest) = LATEST_UPDATE_READY.lock() {
        *latest = owned.clone();
    }
    if let Ok(mut slot) = PENDING_UPDATE_READY.lock() {
        *slot = Some(owned);
    }
    let raw = SETTINGS_HWND.load(Ordering::Acquire);
    if raw != 0 {
        let hwnd = HWND(raw as *mut std::ffi::c_void);
        // SAFETY: `PostMessageW` is thread-safe for custom `WM_APP` notifications.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_UPDATE_READY, WPARAM(0), LPARAM(0));
        }
    }
}

fn post_update_status(text: String, still_checking: bool) {
    if let Ok(mut slot) = PENDING_UPDATE_TEXT.lock() {
        *slot = Some((text, still_checking));
    }
    let raw = SETTINGS_HWND.load(Ordering::Acquire);
    if raw != 0 {
        let hwnd = HWND(raw as *mut std::ffi::c_void);
        // SAFETY: `PostMessageW` is thread-safe for custom `WM_APP` notifications.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_UPDATE_STATUS, WPARAM(0), LPARAM(0));
        }
    }
}

/// Open the native Direct2D UsageDeck Settings window on a specific provider page (or General
/// when `provider` is `None`), bringing any existing window to front and switching to that page.
///
/// `on_changed` is invoked whenever settings, provider toggles/order, bar visibility,
/// or OpenCode accounts are saved. When `on_check_updates` is `None`, the dialog performs
/// a built-in GitHub latest-release check in a background thread.
pub fn open_settings_dialog_on_page(
    owner: HWND,
    provider: Option<ProviderKind>,
    on_changed: Box<dyn Fn() + Send + Sync>,
    on_check_updates: Option<Box<dyn Fn(HWND) + Send + Sync>>,
) {
    let target_page = provider.map_or(PageId::General, PageId::Provider);
    open_settings_dialog_on_target_page(
        owner,
        target_page,
        provider.is_some(),
        on_changed,
        on_check_updates,
    );
}

/// Open the native Direct2D UsageDeck Settings window directly on the **Widget** page
/// (used by the System CPU/Memory hover card's Settings button).
pub fn open_settings_dialog_on_widget(
    owner: HWND,
    on_changed: Box<dyn Fn() + Send + Sync>,
    on_check_updates: Option<Box<dyn Fn(HWND) + Send + Sync>>,
) {
    open_settings_dialog_on_target_page(owner, PageId::Widget, true, on_changed, on_check_updates);
}

fn open_settings_dialog_on_target_page(
    owner: HWND,
    target_page: PageId,
    switch_existing_page: bool,
    on_changed: Box<dyn Fn() + Send + Sync>,
    on_check_updates: Option<Box<dyn Fn(HWND) + Send + Sync>>,
) {
    // Single-instance behavior: bring the open Settings window to front (and switch page if requested).
    let existing = SETTINGS_HWND.load(Ordering::Acquire);
    if existing != 0 {
        let hwnd = HWND(existing as *mut std::ffi::c_void);
        let mut new_title: Option<String> = None;
        if switch_existing_page {
            try_with_state(|state| {
                if state.page != target_page {
                    state.page = target_page;
                    state.scroll_y = 0.0;
                    state.hovered_action = None;
                    state.rebuild_layout();
                    state.invalidate();
                    new_title = Some(target_page.window_title());
                }
            });
        }
        // SAFETY: updates caption, restores, and focuses our own live Settings window outside borrow.
        unsafe {
            if let Some(title) = new_title {
                let w = wide(&title);
                let _ = SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
            }
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        crate::window::ensure_strip_above_tray_now();
        crate::window::post_ensure_zorder();
        return;
    }

    let Ok(renderer) = SettingsRenderer::new() else {
        log::warn!("settings_dialog: failed to initialize Direct2D/DirectWrite renderer");
        return;
    };

    let on_changed: Arc<dyn Fn() + Send + Sync> = Arc::from(on_changed);
    let on_check_updates: Option<Arc<dyn Fn(HWND) + Send + Sync>> = on_check_updates.map(Arc::from);

    let store = default_data_dir().map(|dir| Store::new(dir, Box::new(CredentialVault)));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok();
    let settings = store.as_ref().map(Store::get_settings).unwrap_or_default();
    theme::set_active_app_skin(settings.app_skin);
    let is_dark = resolve_is_dark(settings.panel_theme);

    let initial_page = target_page;
    let initial_title = wide(&initial_page.window_title());

    // SAFETY: Win32 window class registration & creation on the UI thread.
    let (hwnd, app_icon, dpi, client_w_dip, client_h_dip, start_maximized) = unsafe {
        let Ok(hinstance) = GetModuleHandleW(None) else {
            log::warn!("settings_dialog: GetModuleHandleW failed");
            return;
        };
        let class_name = w!("UsageDeckSettingsWindowD2D");
        CLASS_REGISTERED.call_once(|| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(settings_wnd_proc),
                hInstance: hinstance.into(),
                hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassExW(&wc);
        });

        let owner_dpi = if !owner.is_invalid() {
            let d = GetDpiForWindow(owner);
            if d == 0 {
                96
            } else {
                d
            }
        } else {
            96
        };
        let scale = owner_dpi as f32 / 96.0;
        let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
        let ex_style = WINDOW_EX_STYLE(0);

        let (x, y, win_w, win_h, maximized) =
            compute_initial_placement(owner, style, ex_style, scale);

        let Ok(hwnd) = CreateWindowExW(
            ex_style,
            class_name,
            PCWSTR(initial_title.as_ptr()),
            style,
            x,
            y,
            win_w,
            win_h,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) else {
            log::warn!("settings_dialog: CreateWindowExW failed");
            return;
        };

        let app_icon = load_app_icon();
        if !app_icon.is_invalid() {
            const WM_SETICON: u32 = 0x0080;
            const ICON_SMALL: usize = 0;
            const ICON_BIG: usize = 1;
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL)),
                Some(LPARAM(app_icon.0 as isize)),
            );
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG)),
                Some(LPARAM(app_icon.0 as isize)),
            );
        }

        apply_titlebar_theme(hwnd, is_dark);

        let actual_dpi = {
            let d = GetDpiForWindow(hwnd);
            if d == 0 {
                owner_dpi
            } else {
                d
            }
        };
        let actual_scale = actual_dpi as f32 / 96.0;
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let cw = ((rc.right - rc.left).max(1) as f32) / actual_scale;
        let ch = ((rc.bottom - rc.top).max(1) as f32) / actual_scale;

        let _ = SetTimer(Some(hwnd), TIMER_POLL_ID, TIMER_POLL_MS, None);
        (hwnd, app_icon, actual_dpi, cw, ch, maximized)
    };

    let update_ready_version = LATEST_UPDATE_READY.lock().ok().and_then(|g| g.clone());
    let update_status_text = LATEST_UPDATE_STATUS
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default();

    let app_version = env!("CARGO_PKG_VERSION");
    let initial_snapshot = LAST_SETTINGS_SNAPSHOT
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .map(|mut snap| {
            snap.settings = settings.clone();
            snap
        })
        .or_else(|| default_data_dir().map(|dir| cached_settings_snapshot(&dir, settings.clone())));
    let initial_layout = build_layout(&LayoutInput {
        client_w: client_w_dip,
        client_h: client_h_dip,
        page: initial_page,
        settings: &settings,
        snapshot: initial_snapshot.as_ref(),
        loading: true,
        app_version,
        update_status_text: &update_status_text,
        update_checking: false,
        update_ready_version: update_ready_version.as_deref(),
        expanded_snoozed: None,
        spend_range: SpendRange::Hours24,
        scroll_y: 0.0,
        now_ms: now_ms_u64(),
        toast: None,
    });

    let state = SettingsState {
        hwnd,
        app_icon,
        dpi,
        client_w_dip,
        client_h_dip,
        renderer,
        store,
        rt,
        settings,
        snapshot: initial_snapshot,
        loading: true,
        page: initial_page,
        expanded_snoozed: None,
        spend_range: SpendRange::Hours24,
        scroll_y: 0.0,
        hovered_action: None,
        scrollbar_hovered: false,
        scrollbar_dragging: false,
        scrollbar_drag_offset: 0.0,
        tracking_leave: false,
        update_status_text,
        update_checking: false,
        update_ready_version,
        toast: None,
        layout: initial_layout,
        on_changed,
        on_check_updates,
    };

    SETTINGS_OPEN.store(true, Ordering::Release);
    SETTINGS_HWND.store(hwnd.0 as isize, Ordering::Release);
    SETTINGS_STATE.with(|cell| {
        *cell.borrow_mut() = Some(state);
    });

    // Show window strictly OUTSIDE any RefCell borrow!
    // SAFETY: shows and focuses our own Settings window.
    unsafe {
        let _ = ShowWindow(
            hwnd,
            if start_maximized {
                SW_SHOWMAXIMIZED
            } else {
                SW_SHOW
            },
        );
        let _ = SetForegroundWindow(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    // Kick off non-blocking background snapshot poll immediately.
    spawn_settings_poll();
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn now_ms_u64() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn load_app_icon() -> HICON {
    if APP_ICON_BYTES.len() >= 6 {
        let count = u16::from_le_bytes([APP_ICON_BYTES[4], APP_ICON_BYTES[5]]) as usize;
        if count > 0 && APP_ICON_BYTES.len() >= 6 + count * 16 {
            let mut best: Option<(usize, usize, i32)> = None;
            for i in 0..count {
                let off = 6 + i * 16;
                let w = if APP_ICON_BYTES[off] == 0 {
                    256
                } else {
                    i32::from(APP_ICON_BYTES[off])
                };
                let size = u32::from_le_bytes([
                    APP_ICON_BYTES[off + 8],
                    APP_ICON_BYTES[off + 9],
                    APP_ICON_BYTES[off + 10],
                    APP_ICON_BYTES[off + 11],
                ]) as usize;
                let data_off = u32::from_le_bytes([
                    APP_ICON_BYTES[off + 12],
                    APP_ICON_BYTES[off + 13],
                    APP_ICON_BYTES[off + 14],
                    APP_ICON_BYTES[off + 15],
                ]) as usize;
                if data_off
                    .checked_add(size)
                    .is_none_or(|end| end > APP_ICON_BYTES.len())
                {
                    continue;
                }
                let score = (w - 32).abs();
                if best.is_none_or(|(_, _, bs)| score < bs) {
                    best = Some((data_off, size, score));
                }
            }
            if let Some((data_off, size, _)) = best {
                let slice = &APP_ICON_BYTES[data_off..data_off + size];
                // SAFETY: bounded ICO slice from embedded asset.
                if let Ok(icon) = unsafe {
                    CreateIconFromResourceEx(slice, true, 0x0003_0000, 32, 32, LR_DEFAULTCOLOR)
                } {
                    return icon;
                }
            }
        }
    }
    HICON::default()
}

// ---- Window Geometry Persistence (`settings-window.json`) -----------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SavedPlacement {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    maximized: bool,
}

fn load_saved_placement() -> Option<SavedPlacement> {
    let dir = default_data_dir()?;
    let val = read_json(&dir.join(SETTINGS_WINDOW_FILE))?;
    let x = val.get("x")?.as_i64()? as i32;
    let y = val.get("y")?.as_i64()? as i32;
    let width = val.get("width")?.as_i64()? as i32;
    let height = val.get("height")?.as_i64()? as i32;
    if width <= 0 || height <= 0 {
        return None;
    }
    let maximized = val
        .get("maximized")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some(SavedPlacement {
        x,
        y,
        width,
        height,
        maximized,
    })
}

fn save_window_placement(hwnd: HWND) {
    let Some(dir) = default_data_dir() else {
        return;
    };
    let mut wp = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: queries standard Win32 placement struct for our own window.
    if unsafe { GetWindowPlacement(hwnd, &mut wp) }.is_err() {
        return;
    }
    let rc = wp.rcNormalPosition;
    let width = rc.right - rc.left;
    let height = rc.bottom - rc.top;
    if width <= 0 || height <= 0 || rc.left <= -30_000 || rc.top <= -30_000 {
        return;
    }
    // SAFETY: queries standard Win32 maximized state for our own window.
    let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
    let payload = serde_json::json!({
        "x": rc.left,
        "y": rc.top,
        "width": width,
        "height": height,
        "maximized": maximized,
    });
    let _ = write_json_atomic(&dir.join(SETTINGS_WINDOW_FILE), &payload);
}

fn clamp_rect_to_work_area(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    min_w: i32,
    min_h: i32,
    wa: RECT,
) -> (i32, i32, i32, i32) {
    let wa_w = (wa.right - wa.left).max(min_w);
    let wa_h = (wa.bottom - wa.top).max(min_h);
    let cw = w.clamp(min_w.min(wa_w), wa_w);
    let ch = h.clamp(min_h.min(wa_h), wa_h);
    let cx = x.clamp(wa.left, wa.right - cw);
    let cy = y.clamp(wa.top, wa.bottom - ch);
    (cx, cy, cw, ch)
}

unsafe fn compute_initial_placement(
    owner: HWND,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    scale: f32,
) -> (i32, i32, i32, i32, bool) {
    let mut def_rc = RECT {
        left: 0,
        top: 0,
        right: (DEFAULT_WIN_W_DIP * scale).round() as i32,
        bottom: (DEFAULT_WIN_H_DIP * scale).round() as i32,
    };
    let _ = AdjustWindowRectEx(&mut def_rc, style, false, ex_style);
    let def_w = def_rc.right - def_rc.left;
    let def_h = def_rc.bottom - def_rc.top;

    let mut min_rc = RECT {
        left: 0,
        top: 0,
        right: (MIN_WIN_W_DIP * scale).round() as i32,
        bottom: (MIN_WIN_H_DIP * scale).round() as i32,
    };
    let _ = AdjustWindowRectEx(&mut min_rc, style, false, ex_style);
    let min_w = min_rc.right - min_rc.left;
    let min_h = min_rc.bottom - min_rc.top;

    if let Some(saved) = load_saved_placement() {
        let probe = RECT {
            left: saved.x,
            top: saved.y,
            right: saved.x + saved.width,
            bottom: saved.y + saved.height,
        };
        let mon = MonitorFromRect(&probe, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            let (cx, cy, cw, ch) = clamp_rect_to_work_area(
                saved.x,
                saved.y,
                saved.width,
                saved.height,
                min_w,
                min_h,
                mi.rcWork,
            );
            return (cx, cy, cw, ch, saved.maximized);
        }
    }

    let mon = MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(mon, &mut mi).as_bool() {
        let wa = mi.rcWork;
        let x = wa.left + ((wa.right - wa.left - def_w) / 2).max(0);
        let y = wa.top + ((wa.bottom - wa.top - def_h) / 2).max(0);
        let (cx, cy, cw, ch) = clamp_rect_to_work_area(x, y, def_w, def_h, min_w, min_h, wa);
        (cx, cy, cw, ch, false)
    } else {
        (120, 100, def_w, def_h, false)
    }
}

// ---- Background Polling & Built-in Update Check ---------------------------------------------

fn shared_settings_core() -> Option<Arc<Core>> {
    SHARED_SETTINGS_CORE
        .get_or_init(|| {
            let data_dir = default_data_dir()?;
            let core = Arc::new(Core::new(Store::new(data_dir, Box::new(CredentialVault))));
            usagedeck_core::claude_logs::start_scanner(core.spend.clone());
            Some(core)
        })
        .clone()
}

fn spawn_settings_poll() {
    if POLL_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("usagedeck-settings-poll".into())
        .spawn(|| {
            let snap = (|| -> Option<SettingsSnapshot> {
                let core = shared_settings_core()?;
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()?;
                Some(rt.block_on(poll_settings_snapshot(&core)))
            })();

            POLL_IN_FLIGHT.store(false, Ordering::Release);

            if let Some(snapshot) = snap {
                if let Ok(mut last) = LAST_SETTINGS_SNAPSHOT.lock() {
                    *last = Some(snapshot.clone());
                }
                if let Ok(mut slot) = PENDING_SETTINGS_DATA.lock() {
                    *slot = Some(snapshot);
                }
            }
            let raw = SETTINGS_HWND.load(Ordering::Acquire);
            if raw != 0 {
                let hwnd = HWND(raw as *mut std::ffi::c_void);
                // SAFETY: `PostMessageW` is thread-safe for custom `WM_APP` notifications.
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_APP_SETTINGS_DATA, WPARAM(0), LPARAM(0));
                }
            }
        });
}

fn run_builtin_update_check() {
    let _ = std::thread::Builder::new()
        .name("usagedeck-update-check".into())
        .spawn(|| {
            let current_raw = env!("CARGO_PKG_VERSION");
            let current_ver: Option<Version> = current_raw
                .split('-')
                .next()
                .and_then(|base| base.parse().ok());

            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    post_update_status(format!("Update check failed: {e}"), false);
                    return;
                }
            };

            let data_dir = default_data_dir().unwrap_or_else(std::env::temp_dir);
            let core = Core::new(Store::new(data_dir, Box::new(CredentialVault)));

            let outcome = rt.block_on(async {
                let resp = core
                    .http
                    .get(LATEST_RELEASE_URL)
                    .header("User-Agent", USER_AGENT)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                    .map_err(|e| format!("network error: {e}"))?;
                let resp = resp
                    .error_for_status()
                    .map_err(|e| format!("GitHub error: {e}"))?;
                let body = resp.text().await.map_err(|e| format!("read error: {e}"))?;
                parse_release(&body).map_err(|e| e.to_string())
            });

            match outcome {
                Ok(release) => {
                    if let Some(cur) = current_ver {
                        if is_newer(cur, release.version) {
                            let ver_str = release.version.to_string();
                            set_update_ready(Some(&ver_str));
                            post_update_status(
                                format!("Update v{ver_str} available (current v{current_raw})"),
                                false,
                            );
                            return;
                        }
                    }
                    post_update_status(format!("Up to date (v{current_raw})"), false);
                }
                Err(msg) => {
                    post_update_status(format!("Couldn't check for updates ({msg})"), false);
                }
            }
        });
}

// ---- State Helpers --------------------------------------------------------------------------

impl SettingsState {
    fn rebuild_layout(&mut self) {
        let app_version = self
            .snapshot
            .as_ref()
            .map(|s| s.app_version.as_str())
            .unwrap_or(env!("CARGO_PKG_VERSION"));
        self.layout = build_layout(&LayoutInput {
            client_w: self.client_w_dip,
            client_h: self.client_h_dip,
            page: self.page,
            settings: &self.settings,
            snapshot: self.snapshot.as_ref(),
            loading: self.loading,
            app_version,
            update_status_text: &self.update_status_text,
            update_checking: self.update_checking,
            update_ready_version: self.update_ready_version.as_deref(),
            expanded_snoozed: self.expanded_snoozed,
            spend_range: self.spend_range,
            scroll_y: self.scroll_y,
            now_ms: now_ms_u64(),
            toast: self.toast.as_deref(),
        });
        self.scroll_y = self.layout.scroll_y;
    }

    fn invalidate(&self) {
        // SAFETY: invalidates our own window for repaint.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some(msg.into());
        // SAFETY: sets/replaces auto-hide toast timer on our own HWND.
        unsafe {
            let _ = SetTimer(Some(self.hwnd), TIMER_TOAST_ID, TIMER_TOAST_MS, None);
        }
        self.rebuild_layout();
        self.invalidate();
    }

    fn save_settings(
        &mut self,
        change: impl FnOnce(Settings) -> Settings,
    ) -> Result<Settings, String> {
        let (Some(store), Some(rt)) = (&self.store, &self.rt) else {
            return Err("Data folder unavailable (APPDATA is not set).".to_string());
        };
        let next = rt
            .block_on(store.update_settings(change))
            .map_err(|e| format!("Failed to save settings: {e}"))?;
        self.settings = next.clone();
        if let Some(ref mut snap) = self.snapshot {
            snap.settings = next.clone();
        }
        Ok(next)
    }

    fn hit_at(&self, x_dip: f32, y_dip: f32) -> Option<(Action, bool)> {
        for h in self.layout.hits.iter().rev() {
            if h.rect.contains(x_dip, y_dip) {
                return Some((h.action.clone(), h.enabled));
            }
        }
        None
    }

    fn row_context_at(&self, x_dip: f32, y_dip: f32) -> Option<Action> {
        for h in self.layout.row_context_hits.iter().rev() {
            if h.rect.contains(x_dip, y_dip) {
                return Some(h.action.clone());
            }
        }
        None
    }

    fn update_account_row_hidden(
        &mut self,
        provider: ProviderKind,
        account_id: &str,
        hidden: bool,
    ) {
        if let Some(ref mut snap) = self.snapshot {
            let list = match provider {
                ProviderKind::Claude => &mut snap.claude,
                ProviderKind::Codex => &mut snap.codex,
                ProviderKind::Cursor => &mut snap.cursor,
                ProviderKind::Grok => &mut snap.grok,
                ProviderKind::OpenCode => &mut snap.opencode,
                ProviderKind::Antigravity => &mut snap.antigravity,
            };
            for row in list.iter_mut() {
                if row.id.eq_ignore_ascii_case(account_id) {
                    row.hidden = hidden;
                }
            }
        }
    }

    fn update_account_row_nickname(
        &mut self,
        provider: ProviderKind,
        account_id: &str,
        nickname: Option<String>,
    ) {
        if let Some(ref mut snap) = self.snapshot {
            let list = match provider {
                ProviderKind::Claude => &mut snap.claude,
                ProviderKind::Codex => &mut snap.codex,
                ProviderKind::Cursor => &mut snap.cursor,
                ProviderKind::Grok => &mut snap.grok,
                ProviderKind::OpenCode => &mut snap.opencode,
                ProviderKind::Antigravity => &mut snap.antigravity,
            };
            for row in list.iter_mut() {
                if row.id.eq_ignore_ascii_case(account_id) {
                    row.nickname = nickname.clone();
                }
            }
        }
    }
}

fn open_folder(dir: Option<PathBuf>) {
    let Some(dir) = dir else { return };
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
}

fn disabled_account_key(provider: ProviderKind, account_id: &str) -> String {
    match provider {
        ProviderKind::OpenCode => account_id.trim().to_lowercase(),
        _ => format!("{}:{}", provider.key_prefix(), account_id.trim()).to_lowercase(),
    }
}

fn display_name_key(provider: ProviderKind, account_id: &str) -> String {
    match provider {
        ProviderKind::OpenCode => account_id.trim().to_string(),
        _ => format!("{}:{}", provider.key_prefix(), account_id.trim()),
    }
}

fn subscription_account_keys(provider: ProviderKind, account_id: &str) -> (String, Option<String>) {
    let trimmed = account_id.trim().to_lowercase();
    let scoped = format!("{}:{trimmed}", provider.key_prefix());
    let bare = (provider == ProviderKind::OpenCode).then_some(trimmed);
    (scoped, bare)
}

// ---- Action Dispatch (Always Runs Win32 Modals / Menus / Callbacks Outside Borrow!) ---------

enum DeferredAction {
    None,
    NotifyChanged(Arc<dyn Fn() + Send + Sync>),
    RunCustomUpdate(Arc<dyn Fn(HWND) + Send + Sync>),
    RunBuiltinUpdate,
    PollSnapshot,
    OpenAccountMenu {
        provider: ProviderKind,
        account_id: String,
        label: String,
        nickname: Option<String>,
        hidden: bool,
        unsubscribed: bool,
        subscription_ends_at: Option<String>,
        is_dark: bool,
    },
    PromptAddOpenCode {
        is_dark: bool,
    },
    PromptHourlyTarget {
        is_dark: bool,
        current: Option<u32>,
    },
    QuitApp,
}

fn execute_action(hwnd: HWND, action: Action) {
    let mut deferred = DeferredAction::None;
    let mut new_title: Option<String> = None;
    let mut new_dark_titlebar: Option<bool> = None;

    try_with_state(|state| match action {
        Action::SelectPage(page) => {
            if state.page != page {
                state.page = page;
                state.scroll_y = 0.0;
                state.hovered_action = None;
                state.rebuild_layout();
                state.invalidate();
                new_title = Some(page.window_title());
            }
        }
        Action::CheckUpdates => {
            if let Some(cb) = state.on_check_updates.clone() {
                state.update_status_text = if state.update_ready_version.is_some() {
                    "Installing update…".to_string()
                } else {
                    "Checking for updates…".to_string()
                };
                state.update_checking = state.update_ready_version.is_none();
                state.rebuild_layout();
                state.invalidate();
                deferred = DeferredAction::RunCustomUpdate(cb);
            } else {
                state.update_status_text = "Checking for updates…".to_string();
                state.update_checking = true;
                state.rebuild_layout();
                state.invalidate();
                deferred = DeferredAction::RunBuiltinUpdate;
            }
        }
        Action::OpenAccountFolder => {
            open_folder(default_data_dir());
        }
        Action::OpenLogFolder => {
            open_folder(default_log_dir());
        }
        Action::CopyDiagnostics => {
            let text = state
                .snapshot
                .as_ref()
                .map(|s| s.diagnostics_text.clone())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("UsageDeck v{} (Windows)", env!("CARGO_PKG_VERSION")));
            match clipboard::copy_text(&text) {
                Ok(()) => state.show_toast("Diagnostics copied"),
                Err(e) => {
                    log::warn!("copy diagnostics failed: {e}");
                    state.show_toast("Couldn't copy diagnostics");
                }
            }
        }
        Action::ToggleStartAtLogin => {
            let next_val = !state.settings.start_at_login;
            match state.save_settings(|mut s| {
                s.start_at_login = next_val;
                s
            }) {
                Ok(_) => {
                    autostart::set_start_at_login(next_val);
                    state.show_toast(if next_val {
                        "Starts with Windows"
                    } else {
                        "No longer starts with Windows"
                    });
                    deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
                }
                Err(e) => {
                    log::warn!("{e}");
                    state.show_toast("Couldn't save setting");
                }
            }
        }
        Action::ToggleHideInFullscreen => {
            let next_val = !state.settings.hide_in_fullscreen;
            match state.save_settings(|mut s| {
                s.hide_in_fullscreen = next_val;
                s
            }) {
                Ok(_) => {
                    state.show_toast(if next_val {
                        "Hides during full-screen apps"
                    } else {
                        "Stays visible during full-screen apps"
                    });
                    deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
                }
                Err(e) => {
                    log::warn!("{e}");
                    state.show_toast("Couldn't save setting");
                }
            }
        }
        Action::ToggleAutoHideExhausted => {
            let next_val = !state.settings.auto_hide_exhausted;
            match state.save_settings(|mut s| {
                s.auto_hide_exhausted = next_val;
                s
            }) {
                Ok(_) => {
                    state.show_toast(if next_val {
                        "Full cards auto-hide until reset"
                    } else {
                        "Full cards stay visible"
                    });
                    deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
                }
                Err(e) => {
                    log::warn!("{e}");
                    state.show_toast("Couldn't save setting");
                }
            }
        }
        Action::SetPanelTheme(theme) => {
            if state.settings.panel_theme == theme {
                return;
            }
            if state
                .save_settings(|mut s| {
                    s.panel_theme = theme;
                    s
                })
                .is_ok()
            {
                theme::set_active_app_skin(state.settings.app_skin);
                new_dark_titlebar = Some(resolve_is_dark(theme));
                state.rebuild_layout();
                state.invalidate();
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::SetAppSkin(skin) => {
            if state.settings.app_skin == skin {
                return;
            }
            if state
                .save_settings(|mut s| {
                    s.app_skin = skin;
                    s
                })
                .is_ok()
            {
                theme::set_active_app_skin(skin);
                new_dark_titlebar = Some(resolve_is_dark(state.settings.panel_theme));
                state.rebuild_layout();
                state.show_toast(match skin {
                    usagedeck_core::store::AppSkin::Default => "Switched to Default skin",
                    usagedeck_core::store::AppSkin::GeminiGem => "Switched to Gemini Gem skin",
                });
                state.invalidate();
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::SetWidgetTheme(mode) => {
            if state.settings.theme_mode == mode {
                return;
            }
            if state
                .save_settings(|mut s| {
                    s.theme_mode = mode;
                    s
                })
                .is_ok()
            {
                state.rebuild_layout();
                state.invalidate();
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::SetMeterStyle(style) => {
            if state.settings.taskbar_meter_style == style {
                return;
            }
            if state
                .save_settings(|mut s| {
                    s.taskbar_meter_style = style;
                    s
                })
                .is_ok()
            {
                state.show_toast("Taskbar style updated");
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::QuitApp => {
            deferred = DeferredAction::QuitApp;
        }
        Action::MoveProviderUp(wp) => {
            if let Some(pos) = state.settings.provider_order.iter().position(|p| *p == wp) {
                if pos > 0
                    && state
                        .save_settings(|mut s| {
                            if let Some(p) = s.provider_order.iter().position(|x| *x == wp) {
                                if p > 0 {
                                    s.provider_order.swap(p, p - 1);
                                }
                            }
                            s
                        })
                        .is_ok()
                {
                    state.rebuild_layout();
                    state.invalidate();
                    deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
                }
            }
        }
        Action::MoveProviderDown(wp) => {
            let len = state.settings.provider_order.len();
            if let Some(pos) = state.settings.provider_order.iter().position(|p| *p == wp) {
                if pos + 1 < len
                    && state
                        .save_settings(|mut s| {
                            if let Some(p) = s.provider_order.iter().position(|x| *x == wp) {
                                if p + 1 < s.provider_order.len() {
                                    s.provider_order.swap(p, p + 1);
                                }
                            }
                            s
                        })
                        .is_ok()
                {
                    state.rebuild_layout();
                    state.invalidate();
                    deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
                }
            }
        }
        Action::ToggleProviderEnabled(wp) => {
            let name = ProviderKind::from_widget_provider(wp).display_name();
            let next_val = !layout::provider_enabled(&state.settings, wp);
            if state
                .save_settings(|mut s| {
                    match wp {
                        WidgetProvider::Claude => s.claude_enabled = next_val,
                        WidgetProvider::Cursor => s.cursor_enabled = next_val,
                        WidgetProvider::Codex => s.codex_enabled = next_val,
                        WidgetProvider::Grok => s.grok_enabled = next_val,
                        WidgetProvider::Opencode => s.opencode_enabled = next_val,
                        WidgetProvider::Antigravity => s.antigravity_enabled = next_val,
                    }
                    s
                })
                .is_ok()
            {
                state.show_toast(format!(
                    "{name} {} widget",
                    if next_val { "shown on" } else { "hidden from" }
                ));
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::ToggleSnoozedExpand(wp) => {
            state.expanded_snoozed = if state.expanded_snoozed == Some(wp) {
                None
            } else {
                Some(wp)
            };
            state.rebuild_layout();
            state.invalidate();
        }
        Action::UnhideSnoozed(key) => {
            if state
                .save_settings(|mut s| {
                    s.snoozed_until.remove(&key);
                    s
                })
                .is_ok()
            {
                state.show_toast("Card is back on the widget");
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::ToggleShowProviderNames => {
            let next_val = !state.settings.show_provider_names;
            if state
                .save_settings(|mut s| {
                    s.show_provider_names = next_val;
                    s
                })
                .is_ok()
            {
                state.show_toast(if next_val {
                    "Cards show provider names"
                } else {
                    "Cards show nicknames"
                });
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::ToggleSystemMonitor => {
            let next_val = !state.settings.system_monitor_enabled;
            if state
                .save_settings(|mut s| {
                    s.system_monitor_enabled = next_val;
                    s
                })
                .is_ok()
            {
                state.show_toast(if next_val {
                    "System monitor shown on taskbar"
                } else {
                    "System monitor hidden from taskbar"
                });
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::RefreshData => {
            usagedeck_core::providers::antigravity::clear_cache();
            state.loading = true;
            state.rebuild_layout();
            state.invalidate();
            deferred = DeferredAction::PollSnapshot;
        }
        Action::InstallAntigravityCli => {
            usagedeck_core::providers::antigravity::clear_cache();
            match usagedeck_core::providers::antigravity::launch_agy_install_terminal() {
                Ok(()) => {
                    state.show_toast(
                        "Opened Antigravity CLI installer — click Refresh once finished",
                    );
                    deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
                }
                Err(e) => {
                    log::warn!("install antigravity cli failed: {e}");
                    state.show_toast(format!("Could not launch installer: {e}"));
                }
            }
        }
        Action::SignInAntigravityCli => {
            usagedeck_core::providers::antigravity::clear_cache();
            match usagedeck_core::providers::antigravity::launch_agy_signin_terminal() {
                Ok(()) => {
                    state.show_toast(
                        "Opened Antigravity CLI sign-in — click Refresh once signed in",
                    );
                    deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
                }
                Err(e) => {
                    log::warn!("signin antigravity cli failed: {e}");
                    state.show_toast(format!("Could not launch agy sign-in: {e}"));
                }
            }
        }
        Action::OpenAccountMenu {
            provider,
            account_id,
            label,
            nickname,
            hidden,
            unsubscribed,
            subscription_ends_at,
        } => {
            deferred = DeferredAction::OpenAccountMenu {
                provider,
                account_id,
                label,
                nickname,
                hidden,
                unsubscribed,
                subscription_ends_at,
                is_dark: resolve_is_dark(state.settings.panel_theme),
            };
        }
        Action::ToggleBar { provider, bar_id } => {
            let all_ids: Vec<&'static str> = provider_bars(provider, None)
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            let mut hidden: Vec<String> = state
                .settings
                .hidden_bars
                .get(&provider)
                .cloned()
                .unwrap_or_default();
            if let Some(pos) = hidden.iter().position(|id| id == &bar_id) {
                hidden.remove(pos);
            } else {
                // Never allow hiding every bar of a provider.
                let remaining_on = all_ids
                    .iter()
                    .filter(|id| **id != bar_id && !hidden.iter().any(|h| h == *id))
                    .count();
                if remaining_on == 0 {
                    return;
                }
                hidden.push(bar_id);
            }
            if state
                .save_settings(|mut s| {
                    if hidden.is_empty() {
                        s.hidden_bars.remove(&provider);
                    } else {
                        s.hidden_bars.insert(provider, hidden);
                    }
                    s
                })
                .is_ok()
            {
                state.rebuild_layout();
                state.invalidate();
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::ToggleFableShade => {
            let next_val = !state.settings.fable_shade;
            if state
                .save_settings(|mut s| {
                    s.fable_shade = next_val;
                    s
                })
                .is_ok()
            {
                state.show_toast(if next_val {
                    "Fable shown in a lighter shade"
                } else {
                    "Bars back to one color"
                });
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::SetHourlyTarget => {
            deferred = DeferredAction::PromptHourlyTarget {
                is_dark: resolve_is_dark(state.settings.panel_theme),
                current: state.settings.claude_hourly_target,
            };
        }
        Action::ClearHourlyTarget => {
            if state.settings.claude_hourly_target.is_some()
                && state
                    .save_settings(|mut s| {
                        s.claude_hourly_target = None;
                        s
                    })
                    .is_ok()
            {
                state.show_toast("Hourly spend target off");
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
        Action::SetSpendRange(range) => {
            if state.spend_range != range {
                state.spend_range = range;
                state.rebuild_layout();
                state.invalidate();
            }
        }
        Action::AddOpenCodeAccount => {
            deferred = DeferredAction::PromptAddOpenCode {
                is_dark: resolve_is_dark(state.settings.panel_theme),
            };
        }
        Action::ToggleDeepSeekPeakWarning => {
            let next_val = !state.settings.deepseek_peak_hour_warning;
            if state
                .save_settings(|mut s| {
                    s.deepseek_peak_hour_warning = next_val;
                    s
                })
                .is_ok()
            {
                state.show_toast(if next_val {
                    "Peak-hour warning on"
                } else {
                    "Peak-hour warning off"
                });
                deferred = DeferredAction::NotifyChanged(state.on_changed.clone());
            }
        }
    });

    if let Some(title) = new_title {
        let w = wide(&title);
        // SAFETY: updates our own window caption outside RefCell borrow.
        unsafe {
            let _ = SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
        }
        crate::window::ensure_strip_above_tray_now();
        crate::window::post_ensure_zorder();
    }
    if let Some(dark) = new_dark_titlebar {
        apply_titlebar_theme(hwnd, dark);
    }

    match deferred {
        DeferredAction::None => {}
        DeferredAction::NotifyChanged(cb) => cb(),
        DeferredAction::RunCustomUpdate(cb) => cb(hwnd),
        DeferredAction::RunBuiltinUpdate => run_builtin_update_check(),
        DeferredAction::PollSnapshot => spawn_settings_poll(),
        DeferredAction::QuitApp => {
            close_settings_window(hwnd);
            crate::window::post_quit_app();
        }
        DeferredAction::PromptAddOpenCode { is_dark } => {
            if let Some((email, key)) = modals::prompt_opencode_key(hwnd, is_dark, None) {
                save_opencode_account(email, key, "Account added");
            }
        }
        DeferredAction::PromptHourlyTarget { is_dark, current } => {
            if let Some(target) = modals::prompt_hourly_target(hwnd, is_dark, current) {
                let mut cb = None;
                try_with_state(|state| {
                    if state
                        .save_settings(|mut s| {
                            s.claude_hourly_target = target;
                            s
                        })
                        .is_ok()
                    {
                        let msg = match target {
                            Some(v) => format!("Hourly spend target ${v}/h"),
                            None => "Hourly spend target off".to_string(),
                        };
                        state.show_toast(msg);
                        cb = Some(state.on_changed.clone());
                    }
                });
                if let Some(cb) = cb {
                    cb();
                }
            }
        }
        DeferredAction::OpenAccountMenu {
            provider,
            account_id,
            label,
            nickname,
            hidden,
            unsubscribed,
            subscription_ends_at,
            is_dark,
        } => {
            run_account_context_menu(
                hwnd,
                provider,
                account_id,
                label,
                nickname,
                hidden,
                unsubscribed,
                subscription_ends_at,
                is_dark,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_account_context_menu(
    hwnd: HWND,
    provider: ProviderKind,
    account_id: String,
    label: String,
    nickname: Option<String>,
    hidden: bool,
    unsubscribed: bool,
    subscription_ends_at: Option<String>,
    is_dark: bool,
) {
    // SAFETY: TrackPopupMenu runs strictly outside any SETTINGS_STATE borrow.
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let hide_label = wide(if hidden {
            "Show on widget"
        } else {
            "Hide from widget"
        });
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TOGGLE_HIDE,
            PCWSTR(hide_label.as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_RENAME, w!("Rename…"));
        let sub_end_label = wide(if subscription_ends_at.is_some() {
            "Edit subscription end…"
        } else {
            "Set subscription end…"
        });
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_SET_SUB_END,
            PCWSTR(sub_end_label.as_ptr()),
        );
        let unsub_label = wide(if unsubscribed {
            "Mark as subscribed"
        } else {
            "Mark as unsubscribed"
        });
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            MENU_TOGGLE_UNSUBSCRIBED,
            PCWSTR(unsub_label.as_ptr()),
        );
        if provider == ProviderKind::OpenCode {
            let _ = AppendMenuW(menu, MF_STRING, MENU_COPY_KEY, w!("Copy API key"));
            let _ = AppendMenuW(menu, MF_STRING, MENU_REPLACE_KEY, w!("Replace API key…"));
        }
        let _ = AppendMenuW(menu, MF_STRING, MENU_REFRESH, w!("Refresh now"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_REMOVE, w!("Remove account…"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        )
        .0 as usize;
        let _ = DestroyMenu(menu);
        picked
    };

    match cmd {
        MENU_TOGGLE_HIDE => {
            let key = disabled_account_key(provider, &account_id);
            let next_hidden = !hidden;
            let mut cb = None;
            try_with_state(|state| {
                if state
                    .save_settings(|mut s| {
                        s.disabled_accounts
                            .retain(|e| !e.eq_ignore_ascii_case(&key));
                        if next_hidden {
                            s.disabled_accounts.push(key.clone());
                        }
                        s
                    })
                    .is_ok()
                {
                    state.update_account_row_hidden(provider, &account_id, next_hidden);
                    state.show_toast(if next_hidden {
                        "Account hidden from widget"
                    } else {
                        "Account shown on widget"
                    });
                    cb = Some(state.on_changed.clone());
                }
            });
            if let Some(cb) = cb {
                cb();
            }
        }
        MENU_RENAME => {
            let cur_nick = nickname.as_deref().unwrap_or("");
            if let Some(next_nick) = modals::prompt_rename(hwnd, is_dark, &label, cur_nick) {
                let key = display_name_key(provider, &account_id);
                let trimmed = next_nick.trim().to_string();
                let mut cb = None;
                try_with_state(|state| {
                    if state
                        .save_settings(|mut s| {
                            s.display_names.retain(|k, _| !k.eq_ignore_ascii_case(&key));
                            if !trimmed.is_empty() {
                                s.display_names.insert(
                                    key.clone(),
                                    serde_json::Value::String(trimmed.clone()),
                                );
                            }
                            s
                        })
                        .is_ok()
                    {
                        let stored = (!trimmed.is_empty()).then_some(trimmed.clone());
                        state.update_account_row_nickname(provider, &account_id, stored);
                        state.show_toast(if trimmed.is_empty() {
                            "Name reset"
                        } else {
                            "Account renamed"
                        });
                        cb = Some(state.on_changed.clone());
                    }
                });
                if let Some(cb) = cb {
                    cb();
                }
            }
        }
        MENU_SET_SUB_END => {
            if let Some(next_date) = modals::prompt_subscription_end(
                hwnd,
                is_dark,
                &label,
                subscription_ends_at.as_deref(),
            ) {
                let (scoped, bare) = subscription_account_keys(provider, &account_id);
                let mut cb = None;
                let mut poll = false;
                try_with_state(|state| {
                    if state
                        .save_settings(|mut s| {
                            s.subscription_ends_at.retain(|k, _| {
                                !k.eq_ignore_ascii_case(&scoped)
                                    && bare.as_deref().is_none_or(|b| !k.eq_ignore_ascii_case(b))
                            });
                            if let Some(ref ymd) = next_date {
                                s.subscription_ends_at.insert(scoped.clone(), ymd.clone());
                            }
                            s
                        })
                        .is_ok()
                    {
                        state.loading = true;
                        state.show_toast(if next_date.is_some() {
                            "Subscription end saved"
                        } else {
                            "Subscription end cleared"
                        });
                        cb = Some(state.on_changed.clone());
                        poll = true;
                    }
                });
                if let Some(cb) = cb {
                    cb();
                }
                if poll {
                    spawn_settings_poll();
                }
            }
        }
        MENU_TOGGLE_UNSUBSCRIBED => {
            let (scoped, bare) = subscription_account_keys(provider, &account_id);
            let next_unsub = !unsubscribed;
            let today = chrono::Local::now().date_naive();
            let mut cb = None;
            let mut poll = false;
            try_with_state(|state| {
                if state
                    .save_settings(|mut s| {
                        s.unsubscribed_accounts.retain(|e| {
                            !e.eq_ignore_ascii_case(&scoped)
                                && bare.as_deref().is_none_or(|b| !e.eq_ignore_ascii_case(b))
                        });
                        if next_unsub {
                            s.unsubscribed_accounts.push(scoped.clone());
                        } else {
                            // Clear any expired manual end date so re-subscribing takes effect.
                            s.subscription_ends_at.retain(|k, v| {
                                let matches_acc = k.eq_ignore_ascii_case(&scoped)
                                    || bare.as_deref().is_some_and(|b| k.eq_ignore_ascii_case(b));
                                if !matches_acc {
                                    return true;
                                }
                                normalize_ymd_date(v)
                                    .and_then(|ymd| {
                                        chrono::NaiveDate::parse_from_str(&ymd, "%Y-%m-%d").ok()
                                    })
                                    .is_none_or(|d| d >= today)
                            });
                        }
                        s
                    })
                    .is_ok()
                {
                    if let Some(ref mut snap) = state.snapshot {
                        let list = match provider {
                            ProviderKind::Claude => &mut snap.claude,
                            ProviderKind::Codex => &mut snap.codex,
                            ProviderKind::Cursor => &mut snap.cursor,
                            ProviderKind::Grok => &mut snap.grok,
                            ProviderKind::OpenCode => &mut snap.opencode,
                            ProviderKind::Antigravity => &mut snap.antigravity,
                        };
                        for row in list.iter_mut() {
                            if row.id.eq_ignore_ascii_case(&account_id) {
                                row.unsubscribed = next_unsub;
                                row.lapsed = next_unsub;
                                if next_unsub {
                                    row.active = false;
                                    row.plan = Some("Unsubscribed".to_string());
                                }
                            }
                        }
                    }
                    state.loading = true;
                    state.show_toast(if next_unsub {
                        "Marked as unsubscribed"
                    } else {
                        "Marked as subscribed"
                    });
                    cb = Some(state.on_changed.clone());
                    poll = true;
                }
            });
            if let Some(cb) = cb {
                cb();
            }
            if poll {
                spawn_settings_poll();
            }
        }
        MENU_COPY_KEY => {
            try_with_state(|state| {
                let accounts = match (&state.store, &state.rt) {
                    (Some(store), Some(rt)) => rt.block_on(store.get_accounts()),
                    _ => Vec::new(),
                };
                let found = accounts
                    .into_iter()
                    .find(|a| a.email.eq_ignore_ascii_case(&account_id));
                match found {
                    Some(acc) if !acc.key.is_empty() => match clipboard::copy_text(&acc.key) {
                        Ok(()) => state.show_toast("Key copied"),
                        Err(e) => {
                            log::warn!("copy key failed: {e}");
                            state.show_toast("Couldn't copy key");
                        }
                    },
                    _ => state.show_toast("No key stored in Credential Manager"),
                }
            });
        }
        MENU_REPLACE_KEY => {
            if let Some((email, key)) =
                modals::prompt_opencode_key(hwnd, is_dark, Some(&account_id))
            {
                save_opencode_account(email, key, "API key updated");
            }
        }
        MENU_REFRESH => {
            let mut cb = None;
            try_with_state(|state| {
                state.loading = true;
                state.rebuild_layout();
                state.invalidate();
                cb = Some(state.on_changed.clone());
            });
            if let Some(cb) = cb {
                cb();
            }
            spawn_settings_poll();
        }
        MENU_REMOVE => {
            let display_name = nickname
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&label);
            let is_oc = provider == ProviderKind::OpenCode;
            if modals::confirm_remove_account(hwnd, provider.display_name(), display_name, is_oc) {
                if is_oc {
                    remove_opencode_account(&account_id);
                } else {
                    remove_tracked_account(provider, &account_id);
                }
            }
        }
        _ => {}
    }
}

fn save_opencode_account(email: String, key: String, toast_ok: &str) {
    let mut cb = None;
    let mut poll = false;
    try_with_state(|state| {
        let res = match (&state.store, &state.rt) {
            (Some(store), Some(rt)) => {
                let lower = email.to_lowercase();
                let email_c = email.clone();
                let key_c = key.clone();
                rt.block_on(store.update_accounts(|mut accounts| {
                    match accounts
                        .iter_mut()
                        .find(|a| a.email.to_lowercase() == lower)
                    {
                        Some(existing) => existing.key = key_c,
                        None => accounts.push(Account {
                            email: email_c,
                            key: key_c,
                        }),
                    }
                    accounts
                }))
                .map_err(|e| format!("Could not save OpenCode account: {e}"))
            }
            _ => Err("Data folder unavailable.".to_string()),
        };
        match res {
            Ok(_) => {
                state.loading = true;
                state.show_toast(toast_ok);
                cb = Some(state.on_changed.clone());
                poll = true;
            }
            Err(e) => {
                log::warn!("{e}");
                state.show_toast("Couldn't save account");
            }
        }
    });
    if let Some(cb) = cb {
        cb();
    }
    if poll {
        spawn_settings_poll();
    }
}

fn remove_opencode_account(email: &str) {
    let mut cb = None;
    let mut poll = false;
    try_with_state(|state| {
        let lower = email.to_lowercase();
        let res = match (&state.store, &state.rt) {
            (Some(store), Some(rt)) => {
                let first = rt.block_on(store.update_accounts(|accounts| {
                    accounts
                        .into_iter()
                        .filter(|a| a.email.to_lowercase() != lower)
                        .collect()
                }));
                match first {
                    Ok(_) => rt
                        .block_on(store.forget_account(email))
                        .map(|next| {
                            state.settings = next;
                        })
                        .map_err(|e| format!("Failed to clean account settings: {e}")),
                    Err(e) => Err(format!("Failed to remove account: {e}")),
                }
            }
            _ => Err("Data folder unavailable.".to_string()),
        };
        match res {
            Ok(()) => {
                if let Some(ref mut snap) = state.snapshot {
                    snap.opencode.retain(|r| !r.id.eq_ignore_ascii_case(email));
                }
                state.loading = true;
                state.show_toast("Account removed");
                cb = Some(state.on_changed.clone());
                poll = true;
            }
            Err(e) => {
                log::warn!("{e}");
                state.show_toast("Couldn't remove account");
            }
        }
    });
    if let Some(cb) = cb {
        cb();
    }
    if poll {
        spawn_settings_poll();
    }
}

fn remove_tracked_account(provider: ProviderKind, account_id: &str) {
    let mut cb = None;
    let mut poll = false;
    try_with_state(|state| {
        let res = match (&state.store, &state.rt) {
            (Some(store), Some(rt)) => {
                let seen = SeenAccounts::new(store.file(SEEN_ACCOUNTS_FILE));
                let cache = UsageCache::new(store.file(USAGE_CACHE_FILE));
                let id_str = account_id.trim().to_string();
                let id_lower = id_str.to_lowercase();
                rt.block_on(async {
                    seen.forget(provider.key_prefix(), std::slice::from_ref(&id_str))
                        .await;
                    cache.remove(provider.cache_section(), &id_lower).await;
                    if id_lower != id_str {
                        cache.remove(provider.cache_section(), &id_str).await;
                    }
                    store
                        .forget_tracked_account(provider.key_prefix(), &id_str)
                        .await
                })
                .map(|next| {
                    state.settings = next;
                })
                .map_err(|e| format!("Failed to remove account: {e}"))
            }
            _ => Err("Data folder unavailable.".to_string()),
        };
        match res {
            Ok(()) => {
                if let Some(ref mut snap) = state.snapshot {
                    let list = match provider {
                        ProviderKind::Claude => &mut snap.claude,
                        ProviderKind::Codex => &mut snap.codex,
                        ProviderKind::Cursor => &mut snap.cursor,
                        ProviderKind::Grok => &mut snap.grok,
                        ProviderKind::OpenCode => &mut snap.opencode,
                        ProviderKind::Antigravity => &mut snap.antigravity,
                    };
                    list.retain(|r| !r.id.eq_ignore_ascii_case(account_id));
                }
                state.loading = true;
                state.show_toast("Account removed");
                cb = Some(state.on_changed.clone());
                poll = true;
            }
            Err(e) => {
                log::warn!("{e}");
                state.show_toast("Couldn't remove account");
            }
        }
    });
    if let Some(cb) = cb {
        cb();
    }
    if poll {
        spawn_settings_poll();
    }
}

// ---- Win32 Window Procedure -----------------------------------------------------------------

fn mouse_xy_dip(lparam: LPARAM, dpi: u32) -> (f32, f32) {
    let x_px = (lparam.0 & 0xffff) as i16 as f32;
    let y_px = ((lparam.0 >> 16) & 0xffff) as i16 as f32;
    let scale = (dpi as f32 / 96.0).max(0.5);
    (x_px / scale, y_px / scale)
}

fn close_settings_window(hwnd: HWND) {
    save_window_placement(hwnd);
    // SAFETY: destroys our own Settings window cleanly without quitting the app.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn settings_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_ACTIVATE | WM_NCACTIVATE | WM_SETFOCUS => {
            crate::window::ensure_strip_above_tray_now();
            crate::window::post_ensure_zorder();
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_GETMINMAXINFO => {
            if lparam.0 != 0 {
                let dpi = {
                    let d = GetDpiForWindow(hwnd);
                    if d == 0 {
                        96
                    } else {
                        d
                    }
                };
                let scale = dpi as f32 / 96.0;
                let mut min_rc = RECT {
                    left: 0,
                    top: 0,
                    right: (MIN_WIN_W_DIP * scale).round() as i32,
                    bottom: (MIN_WIN_H_DIP * scale).round() as i32,
                };
                let _ = AdjustWindowRectEx(
                    &mut min_rc,
                    WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                    false,
                    WINDOW_EX_STYLE(0),
                );
                let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
                mmi.ptMinTrackSize.x = min_rc.right - min_rc.left;
                mmi.ptMinTrackSize.y = min_rc.bottom - min_rc.top;
            }
            LRESULT(0)
        }
        WM_SIZE => {
            try_with_state(|state| {
                let mut rc = RECT::default();
                if GetClientRect(hwnd, &mut rc).is_ok() {
                    let scale = (state.dpi as f32 / 96.0).max(0.5);
                    state.client_w_dip = ((rc.right - rc.left).max(1) as f32) / scale;
                    state.client_h_dip = ((rc.bottom - rc.top).max(1) as f32) / scale;
                    state.rebuild_layout();
                    state.invalidate();
                }
            });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam.0 & 0xffff) as u32;
            if lparam.0 != 0 {
                let suggested = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            try_with_state(|state| {
                if new_dpi > 0 {
                    state.dpi = new_dpi;
                }
                let mut rc = RECT::default();
                if GetClientRect(hwnd, &mut rc).is_ok() {
                    let scale = (state.dpi as f32 / 96.0).max(0.5);
                    state.client_w_dip = ((rc.right - rc.left).max(1) as f32) / scale;
                    state.client_h_dip = ((rc.bottom - rc.top).max(1) as f32) / scale;
                }
                state.rebuild_layout();
                state.invalidate();
            });
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            let theme_dark = try_with_state(|state| {
                let dark = resolve_is_dark(state.settings.panel_theme);
                state.invalidate();
                dark
            });
            if let Some(dark) = theme_dark {
                apply_titlebar_theme(hwnd, dark);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_PAINT => {
            try_with_state(|state| {
                let is_dark = resolve_is_dark(state.settings.panel_theme);
                let pal = SettingsPalette::for_mode_and_skin(is_dark, state.settings.app_skin);
                state.renderer.paint_window(
                    hwnd,
                    state.dpi,
                    &pal,
                    &state.layout,
                    state.hovered_action.as_ref(),
                    state.scrollbar_hovered || state.scrollbar_dragging,
                );
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let mut need_track = false;
            try_with_state(|state| {
                if !state.tracking_leave {
                    state.tracking_leave = true;
                    need_track = true;
                }
                let (x, y) = mouse_xy_dip(lparam, state.dpi);

                if state.scrollbar_dragging {
                    if let Some(ref sb) = state.layout.scrollbar {
                        let travel = (sb.track.h - sb.thumb.h).max(1.0);
                        let new_thumb_y = (y - state.scrollbar_drag_offset)
                            .clamp(sb.track.y, sb.track.y + travel);
                        let frac = (new_thumb_y - sb.track.y) / travel;
                        state.scroll_y = frac * sb.max_scroll;
                        state.rebuild_layout();
                        state.invalidate();
                    }
                    return;
                }

                let sb_hover = state.layout.scrollbar.as_ref().is_some_and(|sb| {
                    x >= sb.track.x - 4.0
                        && x <= sb.track.x + sb.track.w + 4.0
                        && y >= sb.track.y
                        && y <= sb.track.y + sb.track.h
                });
                let next_hover = if sb_hover {
                    None
                } else {
                    state.hit_at(x, y).and_then(|(a, en)| en.then_some(a))
                };

                if next_hover != state.hovered_action || sb_hover != state.scrollbar_hovered {
                    state.hovered_action = next_hover;
                    state.scrollbar_hovered = sb_hover;
                    state.invalidate();
                }
            });
            if need_track {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            try_with_state(|state| {
                state.tracking_leave = false;
                if state.hovered_action.is_some() || state.scrollbar_hovered {
                    state.hovered_action = None;
                    state.scrollbar_hovered = false;
                    state.invalidate();
                }
            });
            LRESULT(0)
        }
        WM_SETCURSOR => {
            // `HTCLIENT == 1` in Win32 WM_SETCURSOR LOWORD(lParam).
            if (lparam.0 & 0xffff) == 1 {
                let hand = try_with_state(|state| {
                    state.hovered_action.is_some()
                        || state.scrollbar_hovered
                        || state.scrollbar_dragging
                })
                .unwrap_or(false);
                if let Ok(cur) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(cur));
                }
                return LRESULT(1);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16 as f32;
            try_with_state(|state| {
                let step = -(delta / 120.0) * 48.0;
                let max_scroll = (state.layout.content_height - state.client_h_dip).max(0.0);
                let next = (state.scroll_y + step).clamp(0.0, max_scroll);
                if (next - state.scroll_y).abs() > 0.1 {
                    state.scroll_y = next;
                    state.rebuild_layout();
                    state.invalidate();
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let start_capture = try_with_state(|state| {
                let (x, y) = mouse_xy_dip(lparam, state.dpi);
                if let Some(ref sb) = state.layout.scrollbar {
                    if x >= sb.track.x - 4.0
                        && x <= sb.track.x + sb.track.w + 4.0
                        && y >= sb.track.y
                        && y <= sb.track.y + sb.track.h
                    {
                        if y >= sb.thumb.y && y <= sb.thumb.y + sb.thumb.h {
                            state.scrollbar_drag_offset = y - sb.thumb.y;
                        } else {
                            let travel = (sb.track.h - sb.thumb.h).max(1.0);
                            let centered =
                                (y - sb.thumb.h * 0.5).clamp(sb.track.y, sb.track.y + travel);
                            state.scroll_y = ((centered - sb.track.y) / travel) * sb.max_scroll;
                            state.scrollbar_drag_offset = sb.thumb.h * 0.5;
                            state.rebuild_layout();
                            state.invalidate();
                        }
                        state.scrollbar_dragging = true;
                        return true;
                    }
                }
                false
            })
            .unwrap_or(false);
            if start_capture {
                let _ = SetCapture(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let mut release_cap = false;
            let clicked: Option<Action> = try_with_state(|state| {
                if state.scrollbar_dragging {
                    state.scrollbar_dragging = false;
                    release_cap = true;
                    state.invalidate();
                    return None;
                }
                let (x, y) = mouse_xy_dip(lparam, state.dpi);
                state.hit_at(x, y).and_then(|(act, en)| en.then_some(act))
            })
            .flatten();
            if release_cap {
                let _ = ReleaseCapture();
            }
            if let Some(act) = clicked {
                execute_action(hwnd, act);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let ctx_action = try_with_state(|state| {
                let (x, y) = mouse_xy_dip(lparam, state.dpi);
                state.row_context_at(x, y)
            })
            .flatten();
            if let Some(act) = ctx_action {
                execute_action(hwnd, act);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0;
            let ctrl_down = GetKeyState(i32::from(VK_CONTROL.0)) < 0;
            if vk == usize::from(VK_ESCAPE.0) || (ctrl_down && vk == usize::from(b'W')) {
                close_settings_window(hwnd);
                return LRESULT(0);
            }
            let scroll_delta = if vk == usize::from(VK_DOWN.0) {
                Some(48.0)
            } else if vk == usize::from(VK_UP.0) {
                Some(-48.0)
            } else if vk == usize::from(VK_NEXT.0) {
                Some(320.0)
            } else if vk == usize::from(VK_PRIOR.0) {
                Some(-320.0)
            } else if vk == usize::from(VK_HOME.0) {
                Some(-100_000.0)
            } else if vk == usize::from(VK_END.0) {
                Some(100_000.0)
            } else {
                None
            };
            if let Some(delta) = scroll_delta {
                try_with_state(|state| {
                    let max_scroll = (state.layout.content_height - state.client_h_dip).max(0.0);
                    let next = (state.scroll_y + delta).clamp(0.0, max_scroll);
                    if (next - state.scroll_y).abs() > 0.1 {
                        state.scroll_y = next;
                        state.rebuild_layout();
                        state.invalidate();
                    }
                });
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_TOAST_ID {
                let _ = KillTimer(Some(hwnd), TIMER_TOAST_ID);
                try_with_state(|state| {
                    if state.toast.take().is_some() {
                        state.rebuild_layout();
                        state.invalidate();
                    }
                });
                return LRESULT(0);
            }
            if wparam.0 == TIMER_POLL_ID {
                spawn_settings_poll();
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_APP_SETTINGS_DATA => {
            let incoming = PENDING_SETTINGS_DATA
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            try_with_state(|state| {
                state.loading = false;
                if let Some(snap) = incoming {
                    state.settings = snap.settings.clone();
                    state.snapshot = Some(snap);
                }
                state.rebuild_layout();
                state.invalidate();
            });
            LRESULT(0)
        }
        WM_APP_UPDATE_STATUS => {
            let pending = PENDING_UPDATE_TEXT
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            if let Some((text, checking)) = pending {
                try_with_state(|state| {
                    state.update_status_text = text;
                    state.update_checking = checking;
                    state.rebuild_layout();
                    state.invalidate();
                });
            }
            LRESULT(0)
        }
        WM_APP_UPDATE_READY => {
            let pending = PENDING_UPDATE_READY
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            if let Some(ver) = pending {
                try_with_state(|state| {
                    state.update_ready_version = ver;
                    state.update_checking = false;
                    state.rebuild_layout();
                    state.invalidate();
                });
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            close_settings_window(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_TOAST_ID);
            let _ = KillTimer(Some(hwnd), TIMER_POLL_ID);
            SETTINGS_OPEN.store(false, Ordering::Release);
            SETTINGS_HWND.store(0, Ordering::Release);
            SETTINGS_STATE.with(|cell| {
                if let Ok(mut borrow) = cell.try_borrow_mut() {
                    if let Some(mut st) = borrow.take() {
                        if let Some(snap) = st.snapshot.take() {
                            if let Ok(mut last) = LAST_SETTINGS_SNAPSHOT.lock() {
                                *last = Some(snap);
                            }
                        }
                    }
                }
            });
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use theme::{format_snooze_duration, format_usd, spend_series};
    use usagedeck_core::view::SettingsPaceData;

    #[test]
    fn validates_opencode_keys() {
        assert!(is_valid_opencode_key("oc_sk_abc123"));
        assert!(is_valid_opencode_key("sk-xyz987"));
        assert!(!is_valid_opencode_key(""));
        assert!(!is_valid_opencode_key("oc_sk_"));
        assert!(!is_valid_opencode_key("sk-"));
        assert!(!is_valid_opencode_key("bad_prefix_123"));
        assert!(!is_valid_opencode_key("oc_sk_has space"));
    }

    #[test]
    fn formats_usd_and_snooze_durations() {
        assert_eq!(format_usd(0.0), "$0.0");
        assert_eq!(format_usd(7.42), "$7.4");
        assert_eq!(format_usd(9.96), "$10");
        assert_eq!(format_usd(1293.4), "$1,293");
        assert_eq!(format_snooze_duration(45 * 60_000), "45m");
        assert_eq!(format_snooze_duration(134 * 60_000), "2h 14m");
        assert_eq!(format_snooze_duration(26 * 3_600_000), "1d 2h");
    }

    #[test]
    fn converts_pace_buckets_to_oldest_first_hourly_rates() {
        let pace = SettingsPaceData {
            minutes: vec![1.0, 0.5, 0.25],
            minutes_fable: vec![0.5, 0.0, 0.25],
            quarters: vec![10.0, 5.0],
            quarters_fable: vec![2.5, 0.0],
            ..Default::default()
        };
        assert_eq!(
            spend_series(&pace, SpendRange::Min15),
            vec![(15.0, 15.0), (30.0, 0.0), (60.0, 30.0)]
        );
        assert_eq!(
            spend_series(&pace, SpendRange::Hours24),
            vec![(20.0, 0.0), (40.0, 10.0)]
        );
    }

    #[test]
    fn clamps_window_rect_inside_work_area() {
        let wa = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(
            clamp_rect_to_work_area(1500, 800, 960, 680, 640, 480, wa),
            (960, 360, 960, 680)
        );
        assert_eq!(
            clamp_rect_to_work_area(-100, -50, 960, 680, 640, 480, wa),
            (0, 0, 960, 680)
        );
    }
}
