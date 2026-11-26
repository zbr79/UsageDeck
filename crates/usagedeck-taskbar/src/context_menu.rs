//! Native Fluent Direct2D/DirectWrite layered context menu for UsageDeck.
//!
//! Replaces the legacy Win32 `TrackPopupMenu` for both the hidden System Tray icon and
//! taskbar strip / hover-card right-clicks. Matches UsageDeck's active Dark/Light theme and
//! `AppSkin` (`Default` / `GeminiGem`), stays non-blocking via `WM_APP_CONTEXT_MENU_*`
//! messages (no nested modal loops), and dismisses cleanly on outside click (`WM_ACTIVATE`),
//! `Esc`, or item selection.

use crate::layout::DipRect;
use crate::taskbar::TaskbarTheme;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use usagedeck_core::store::AppSkin;
use usagedeck_core::view::ProviderKind;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
    MonitorFromPoint, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_RETURN,
    VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LoadCursorW, PostMessageW, RegisterClassExW,
    SetCursor, SetForegroundWindow, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST,
    IDC_ARROW, IDC_HAND, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, ULW_ALPHA,
    WM_ACTIVATE, WM_APP, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SETCURSOR, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const HTCLIENT: isize = 1;
const WA_INACTIVE: usize = 0;

/// Posted to the strip window when the cursor moves over the context menu (`lparam` = client px).
pub const WM_APP_CONTEXT_MENU_MOVE: u32 = WM_APP + 10;
/// Posted to the strip window when the cursor leaves the context menu.
pub const WM_APP_CONTEXT_MENU_LEAVE: u32 = WM_APP + 11;
/// Posted to the strip window on left-click release inside the context menu (`lparam` = client px).
pub const WM_APP_CONTEXT_MENU_CLICK: u32 = WM_APP + 12;
/// Posted to the strip window on key press inside the context menu (`wparam` = virtual key code).
pub const WM_APP_CONTEXT_MENU_KEY: u32 = WM_APP + 13;
/// Posted to the strip window when the context menu loses activation and should dismiss.
pub const WM_APP_CONTEXT_MENU_DISMISS: u32 = WM_APP + 14;

static MENU_OWNER_RAW: AtomicIsize = AtomicIsize::new(0);
static MENU_ACTIVE: AtomicBool = AtomicBool::new(false);
static MENU_GENERATION: AtomicUsize = AtomicUsize::new(0);
static MENU_OVER_ITEM: AtomicBool = AtomicBool::new(false);
static MENU_TRACKING_LEAVE: AtomicBool = AtomicBool::new(false);

const MENU_PAD_X_DIP: f32 = 6.0;
const MENU_PAD_Y_DIP: f32 = 6.0;
const MENU_RADIUS_DIP: f32 = 10.0;
const MENU_ITEM_H_DIP: f32 = 30.0;
const MENU_ITEM_PAD_X_DIP: f32 = 10.0;
const MENU_ITEM_RADIUS_DIP: f32 = 6.0;
const MENU_HEADER_H_DIP: f32 = 22.0;
const MENU_HEADER_GAP_DIP: f32 = 2.0;
const MENU_SEP_H_DIP: f32 = 9.0;
const MENU_SEP_INSET_DIP: f32 = 8.0;
const MENU_MIN_W_DIP: f32 = 184.0;
const MENU_MAX_W_DIP: f32 = 268.0;
const MENU_SCREEN_MARGIN_DIP: f32 = 8.0;
const MENU_ANCHOR_GAP_DIP: f32 = 6.0;

/// Action executed when the user selects a context menu item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextMenuAction {
    OpenProvider {
        provider: ProviderKind,
        watch_cursor: bool,
    },
    OpenProviderSettings(ProviderKind),
    OpenTaskManager,
    OpenWidgetSettings,
    OpenSettings,
    CopyOpenCodeKey(String),
    InstallAntigravityCli,
    SignInAntigravityCli,
    HideUntilReset {
        provider: ProviderKind,
        account_id: String,
        title: String,
        until_ms: u64,
    },
    RefreshNow,
    CopyDiagnostics,
    CheckOrInstallUpdate,
    QuitApp,
}

/// Visual tone for an actionable menu row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItemTone {
    Normal,
    Accent,
    Danger,
}

/// Declarative entry inside a Fluent context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextMenuItem {
    Header(String),
    Separator,
    Action {
        label: String,
        action: ContextMenuAction,
        tone: MenuItemTone,
    },
}

/// Screen placement anchor for the context menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAnchor {
    /// Centered above a taskbar pill rect (matching the hover card's vertical alignment).
    AboveRect {
        rect: RECT,
        tray_top: i32,
        monitor_rect: RECT,
    },
    /// Anchored cleanly near the cursor (used for hidden System Tray icon and hover-card right-clicks).
    AtCursor { pt: POINT },
}

/// Result of handling a keyboard event inside the open context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuKeyOutcome {
    Ignored,
    SelectionChanged,
    Dismiss,
    Activate(ContextMenuAction),
}

#[derive(Clone, Copy)]
struct MenuPalette {
    bg: D2D1_COLOR_F,
    border: D2D1_COLOR_F,
    text_primary: D2D1_COLOR_F,
    text_header: D2D1_COLOR_F,
    accent: D2D1_COLOR_F,
    danger: D2D1_COLOR_F,
    divider: D2D1_COLOR_F,
    hover_bg: D2D1_COLOR_F,
    accent_hover_bg: D2D1_COLOR_F,
    danger_hover_bg: D2D1_COLOR_F,
}

impl MenuPalette {
    fn for_theme(theme: TaskbarTheme, skin: AppSkin) -> Self {
        let is_gem = skin == AppSkin::GeminiGem;
        match (theme, is_gem) {
            (TaskbarTheme::Dark, true) => Self {
                bg: rgba(22, 24, 28, 0.98),
                border: rgba(0, 212, 146, 0.48),
                text_primary: rgba(244, 244, 246, 0.98),
                text_header: rgba(154, 160, 172, 0.90),
                accent: rgba(0, 212, 146, 1.0),
                danger: rgba(248, 113, 113, 1.0),
                divider: rgba(255, 255, 255, 0.09),
                hover_bg: rgba(0, 212, 146, 0.13),
                accent_hover_bg: rgba(0, 212, 146, 0.20),
                danger_hover_bg: rgba(239, 68, 68, 0.16),
            },
            (TaskbarTheme::Light, true) => Self {
                bg: rgba(252, 253, 254, 0.985),
                border: rgba(0, 153, 102, 0.40),
                text_primary: rgba(22, 24, 28, 0.96),
                text_header: rgba(82, 88, 102, 0.88),
                accent: rgba(0, 153, 102, 1.0),
                danger: rgba(220, 38, 38, 1.0),
                divider: rgba(15, 23, 42, 0.09),
                hover_bg: rgba(0, 153, 102, 0.10),
                accent_hover_bg: rgba(0, 153, 102, 0.16),
                danger_hover_bg: rgba(220, 38, 38, 0.10),
            },
            (TaskbarTheme::Dark, false) => Self {
                bg: rgba(24, 25, 30, 0.975),
                border: rgba(255, 255, 255, 0.14),
                text_primary: rgba(244, 244, 246, 0.98),
                text_header: rgba(161, 161, 170, 0.88),
                accent: rgba(96, 165, 250, 1.0),
                danger: rgba(248, 113, 113, 1.0),
                divider: rgba(255, 255, 255, 0.09),
                hover_bg: rgba(255, 255, 255, 0.09),
                accent_hover_bg: rgba(96, 165, 250, 0.16),
                danger_hover_bg: rgba(239, 68, 68, 0.16),
            },
            (TaskbarTheme::Light, false) => Self {
                bg: rgba(250, 251, 253, 0.985),
                border: rgba(0, 0, 0, 0.13),
                text_primary: rgba(20, 20, 24, 0.96),
                text_header: rgba(82, 82, 91, 0.86),
                accent: rgba(37, 99, 235, 1.0),
                danger: rgba(220, 38, 38, 1.0),
                divider: rgba(0, 0, 0, 0.09),
                hover_bg: rgba(0, 0, 0, 0.06),
                accent_hover_bg: rgba(37, 99, 235, 0.11),
                danger_hover_bg: rgba(220, 38, 38, 0.10),
            },
        }
    }
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a,
    }
}

#[derive(Clone, Debug)]
enum LaidOutRow {
    Header {
        text: String,
        rect: DipRect,
    },
    Separator {
        y: f32,
    },
    Action {
        label: String,
        action: ContextMenuAction,
        tone: MenuItemTone,
        rect: DipRect,
    },
}

#[derive(Clone, Debug)]
struct MenuLayout {
    width_dip: f32,
    height_dip: f32,
    rows: Vec<LaidOutRow>,
}

impl MenuLayout {
    fn compute(items: &[ContextMenuItem], content_width_dip: f32) -> Self {
        let width_dip = content_width_dip.clamp(MENU_MIN_W_DIP, MENU_MAX_W_DIP);
        let row_w = (width_dip - MENU_PAD_X_DIP * 2.0).max(40.0);
        let mut y = MENU_PAD_Y_DIP;
        let mut rows = Vec::with_capacity(items.len());

        for item in items {
            match item {
                ContextMenuItem::Header(text) => {
                    rows.push(LaidOutRow::Header {
                        text: text.clone(),
                        rect: DipRect {
                            x: MENU_PAD_X_DIP,
                            y,
                            w: row_w,
                            h: MENU_HEADER_H_DIP,
                        },
                    });
                    y += MENU_HEADER_H_DIP + MENU_HEADER_GAP_DIP;
                }
                ContextMenuItem::Separator => {
                    rows.push(LaidOutRow::Separator {
                        y: y + MENU_SEP_H_DIP * 0.5,
                    });
                    y += MENU_SEP_H_DIP;
                }
                ContextMenuItem::Action {
                    label,
                    action,
                    tone,
                } => {
                    rows.push(LaidOutRow::Action {
                        label: label.clone(),
                        action: action.clone(),
                        tone: *tone,
                        rect: DipRect {
                            x: MENU_PAD_X_DIP,
                            y,
                            w: row_w,
                            h: MENU_ITEM_H_DIP,
                        },
                    });
                    y += MENU_ITEM_H_DIP;
                }
            }
        }

        let height_dip = (y + MENU_PAD_Y_DIP).max(MENU_PAD_Y_DIP * 2.0 + MENU_ITEM_H_DIP);
        Self {
            width_dip,
            height_dip,
            rows,
        }
    }

    fn hit_test_dip(&self, dx: f32, dy: f32) -> Option<usize> {
        self.rows.iter().enumerate().find_map(|(idx, row)| {
            if let LaidOutRow::Action { rect, .. } = row {
                if rect.contains(dx, dy) {
                    return Some(idx);
                }
            }
            None
        })
    }

    fn action_at(&self, row_idx: usize) -> Option<&ContextMenuAction> {
        match self.rows.get(row_idx) {
            Some(LaidOutRow::Action { action, .. }) => Some(action),
            _ => None,
        }
    }

    fn actionable_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(idx, row)| matches!(row, LaidOutRow::Action { .. }).then_some(idx))
            .collect()
    }
}

fn step_selection(indices: &[usize], current: Option<usize>, forward: bool) -> Option<usize> {
    if indices.is_empty() {
        return None;
    }
    match current.and_then(|cur| indices.iter().position(|&idx| idx == cur)) {
        Some(pos) => {
            let next_pos = if forward {
                (pos + 1) % indices.len()
            } else {
                (pos + indices.len() - 1) % indices.len()
            };
            Some(indices[next_pos])
        }
        None => Some(if forward {
            indices[0]
        } else {
            indices[indices.len() - 1]
        }),
    }
}

fn compute_origin_px(anchor: MenuAnchor, width_px: i32, height_px: i32, scale: f32) -> (i32, i32) {
    let margin = (MENU_SCREEN_MARGIN_DIP * scale).round() as i32;
    let gap = (MENU_ANCHOR_GAP_DIP * scale).round() as i32;

    match anchor {
        MenuAnchor::AboveRect {
            rect,
            tray_top,
            monitor_rect,
        } => {
            let anchor_cx = (rect.left + rect.right) / 2;
            let anchor_top = rect.top.min(tray_top);
            let min_x = monitor_rect.left + margin;
            let max_x = (monitor_rect.right - margin - width_px).max(min_x);
            let x = (anchor_cx - width_px / 2).clamp(min_x, max_x);

            let min_y = monitor_rect.top + margin;
            let max_y = (monitor_rect.bottom - margin - height_px).max(min_y);
            let y = (anchor_top - gap - height_px).clamp(min_y, max_y);
            (x, y)
        }
        MenuAnchor::AtCursor { pt } => {
            let mon = monitor_rect_for_point(pt);
            clamp_cursor_origin(pt, width_px, height_px, mon, margin, gap)
        }
    }
}

fn clamp_cursor_origin(
    pt: POINT,
    width_px: i32,
    height_px: i32,
    mon: RECT,
    margin: i32,
    gap: i32,
) -> (i32, i32) {
    let min_x = mon.left + margin;
    let max_x = (mon.right - margin - width_px).max(min_x);
    // Prefer left-aligned at cursor; if it would overflow right, shift left cleanly.
    let ideal_x = if pt.x + width_px <= mon.right - margin {
        pt.x
    } else {
        pt.x - width_px
    };
    let x = ideal_x.clamp(min_x, max_x);

    let min_y = mon.top + margin;
    let max_y = (mon.bottom - margin - height_px).max(min_y);
    // Prefer opening above the cursor (natural for taskbar & hidden tray icon),
    // falling back below the cursor if near the top of the monitor.
    let ideal_y = if pt.y - gap - height_px >= min_y {
        pt.y - gap - height_px
    } else {
        pt.y + gap
    };
    let y = ideal_y.clamp(min_y, max_y);
    (x, y)
}

fn monitor_rect_for_point(pt: POINT) -> RECT {
    // SAFETY: `MonitorFromPoint` and `GetMonitorInfoW` with initialized `cbSize`.
    unsafe {
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut info).as_bool() {
            return info.rcMonitor;
        }
    }
    RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    }
}

unsafe extern "system" fn context_menu_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SETCURSOR if (lparam.0 & 0xffff) == HTCLIENT => {
            let cursor_id = if MENU_OVER_ITEM.load(Ordering::Acquire) {
                IDC_HAND
            } else {
                IDC_ARROW
            };
            if let Ok(cur) = LoadCursorW(None, cursor_id) {
                SetCursor(Some(cur));
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            if !MENU_TRACKING_LEAVE.swap(true, Ordering::AcqRel) {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
            let owner = MENU_OWNER_RAW.load(Ordering::Acquire);
            if owner != 0 {
                let owner_hwnd = HWND(owner as *mut std::ffi::c_void);
                let _ = PostMessageW(
                    Some(owner_hwnd),
                    WM_APP_CONTEXT_MENU_MOVE,
                    WPARAM(0),
                    lparam,
                );
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            MENU_TRACKING_LEAVE.store(false, Ordering::Release);
            MENU_OVER_ITEM.store(false, Ordering::Release);
            let owner = MENU_OWNER_RAW.load(Ordering::Acquire);
            if owner != 0 {
                let owner_hwnd = HWND(owner as *mut std::ffi::c_void);
                let _ = PostMessageW(
                    Some(owner_hwnd),
                    WM_APP_CONTEXT_MENU_LEAVE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let owner = MENU_OWNER_RAW.load(Ordering::Acquire);
            if owner != 0 {
                let owner_hwnd = HWND(owner as *mut std::ffi::c_void);
                let _ = PostMessageW(
                    Some(owner_hwnd),
                    WM_APP_CONTEXT_MENU_CLICK,
                    WPARAM(0),
                    lparam,
                );
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN | WM_RBUTTONUP => LRESULT(0),
        WM_KEYDOWN => {
            let owner = MENU_OWNER_RAW.load(Ordering::Acquire);
            if owner != 0 {
                let owner_hwnd = HWND(owner as *mut std::ffi::c_void);
                let _ = PostMessageW(Some(owner_hwnd), WM_APP_CONTEXT_MENU_KEY, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            if (wparam.0 & 0xffff) == WA_INACTIVE && MENU_ACTIVE.swap(false, Ordering::AcqRel) {
                let gen_id = MENU_GENERATION.load(Ordering::Acquire);
                let owner = MENU_OWNER_RAW.load(Ordering::Acquire);
                if owner != 0 {
                    let owner_hwnd = HWND(owner as *mut std::ffi::c_void);
                    let _ = PostMessageW(
                        Some(owner_hwnd),
                        WM_APP_CONTEXT_MENU_DISMISS,
                        WPARAM(gen_id),
                        LPARAM(0),
                    );
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Native Direct2D layered popup menu window.
pub struct ContextMenuWindow {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    fmt_header: IDWriteTextFormat,
    fmt_item: IDWriteTextFormat,
    fmt_item_bold: IDWriteTextFormat,
    visible: bool,
    origin_px: (i32, i32),
    dpi: u32,
    scale: f32,
    theme: TaskbarTheme,
    skin: AppSkin,
    layout: Option<MenuLayout>,
    selected_row: Option<usize>,
    anchor_rect: Option<RECT>,
}

impl ContextMenuWindow {
    /// Register the popup menu window class and create the hidden layered popup HWND.
    pub fn new() -> Result<Self> {
        // SAFETY: Direct2D/DirectWrite factories and layered Win32 popup lifecycle.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let fmt_header =
                make_text_format(&dwrite, w!("Segoe UI"), DWRITE_FONT_WEIGHT_SEMI_BOLD, 10.5)?;
            let fmt_item =
                make_text_format(&dwrite, w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, 12.0)?;
            let fmt_item_bold =
                make_text_format(&dwrite, w!("Segoe UI"), DWRITE_FONT_WEIGHT_SEMI_BOLD, 12.0)?;

            let hinstance = GetModuleHandleW(None)?;
            let class_name = w!("UsageDeckContextMenuPopup");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(context_menu_wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class_name,
                w!("UsageDeck Menu"),
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

            Ok(Self {
                hwnd,
                d2d,
                dwrite,
                fmt_header,
                fmt_item,
                fmt_item_bold,
                visible: false,
                origin_px: (0, 0),
                dpi: 96,
                scale: 1.0,
                theme: TaskbarTheme::Dark,
                skin: AppSkin::Default,
                layout: None,
                selected_row: None,
                anchor_rect: None,
            })
        }
    }

    /// Bind the taskbar strip HWND that receives menu mouse/key/dismiss events.
    pub fn set_owner_hwnd(&self, owner: HWND) {
        MENU_OWNER_RAW.store(owner.0 as isize, Ordering::Release);
    }

    /// Whether the context menu is currently visible.
    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// Optional screen rect of the pill that spawned this menu (for follow-up toasts).
    pub fn anchor_rect(&self) -> Option<RECT> {
        self.anchor_rect
    }

    /// Measure, lay out, and render the menu offscreen via `UpdateLayeredWindow`.
    ///
    /// Returns the menu HWND to reveal & activate via [`activate_hwnd`] strictly outside
    /// any `STATE` `RefCell` borrow.
    pub fn prepare_show(
        &mut self,
        items: &[ContextMenuItem],
        anchor: MenuAnchor,
        anchor_rect: Option<RECT>,
        dpi: u32,
        theme: TaskbarTheme,
    ) -> Option<HWND> {
        let dpi = dpi.max(96);
        let scale = dpi as f32 / 96.0;
        let skin = crate::settings_dialog::theme::active_app_skin();

        let mut max_text_w = 0.0f32;
        for item in items {
            match item {
                ContextMenuItem::Header(text) => {
                    if let Ok((w, _)) = self.measure_text(text, &self.fmt_header) {
                        max_text_w = max_text_w.max(w);
                    }
                }
                ContextMenuItem::Action { label, tone, .. } => {
                    let fmt = if *tone == MenuItemTone::Accent {
                        &self.fmt_item_bold
                    } else {
                        &self.fmt_item
                    };
                    if let Ok((w, _)) = self.measure_text(label, fmt) {
                        max_text_w = max_text_w.max(w);
                    }
                }
                ContextMenuItem::Separator => {}
            }
        }

        let content_w = max_text_w + (MENU_PAD_X_DIP + MENU_ITEM_PAD_X_DIP) * 2.0 + 8.0;
        let layout = MenuLayout::compute(items, content_w);
        let width_px = (layout.width_dip * scale).ceil().max(1.0) as i32;
        let height_px = (layout.height_dip * scale).ceil().max(1.0) as i32;
        let origin_px = compute_origin_px(anchor, width_px, height_px, scale);

        self.origin_px = origin_px;
        self.dpi = dpi;
        self.scale = scale;
        self.theme = theme;
        self.skin = skin;
        self.layout = Some(layout);
        self.selected_row = None;
        self.anchor_rect = anchor_rect;
        MENU_OVER_ITEM.store(false, Ordering::Release);
        MENU_TRACKING_LEAVE.store(false, Ordering::Release);

        if let Err(e) = self.paint() {
            log::warn!("context menu render failed: {e}");
            return None;
        }
        self.visible = true;
        Some(self.hwnd)
    }

    /// Reveal and foreground the menu HWND. MUST be called outside any `STATE` `RefCell` borrow.
    pub fn activate_hwnd(hwnd: HWND) {
        MENU_GENERATION.fetch_add(1, Ordering::AcqRel);
        // SAFETY: standard Win32 topmost activation on our own popup HWND.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            let _ = SetForegroundWindow(hwnd);
        }
        MENU_ACTIVE.store(true, Ordering::Release);
    }

    /// True if `gen_id` matches the most recently activated menu generation.
    pub fn is_current_generation(gen_id: usize) -> bool {
        MENU_GENERATION.load(Ordering::Acquire) == gen_id
    }

    /// Reset open state and return the HWND to hide via [`hide_hwnd`] outside any `STATE` borrow.
    pub fn prepare_hide(&mut self) -> Option<HWND> {
        MENU_ACTIVE.store(false, Ordering::Release);
        MENU_OVER_ITEM.store(false, Ordering::Release);
        MENU_TRACKING_LEAVE.store(false, Ordering::Release);
        self.selected_row = None;
        self.layout = None;
        self.anchor_rect = None;
        if self.visible {
            self.visible = false;
            Some(self.hwnd)
        } else {
            None
        }
    }

    /// Hide the menu HWND. MUST be called outside any `STATE` `RefCell` borrow.
    pub fn hide_hwnd(hwnd: HWND) {
        // SAFETY: hides our own popup HWND.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    /// Handle cursor move inside the menu (`x_px`, `y_px` in client physical pixels).
    pub fn handle_mouse_move(&mut self, x_px: f32, y_px: f32) {
        if !self.visible {
            return;
        }
        let scale = self.scale.max(0.5);
        let hit = self
            .layout
            .as_ref()
            .and_then(|l| l.hit_test_dip(x_px / scale, y_px / scale));
        MENU_OVER_ITEM.store(hit.is_some(), Ordering::Release);
        if self.selected_row != hit {
            self.selected_row = hit;
            let _ = self.paint();
        }
    }

    /// Handle cursor leaving the menu.
    pub fn handle_mouse_leave(&mut self) {
        if !self.visible {
            return;
        }
        MENU_OVER_ITEM.store(false, Ordering::Release);
        if self.selected_row.is_some() {
            self.selected_row = None;
            let _ = self.paint();
        }
    }

    /// Hit-test a left-click inside the menu and return the clicked action, if any.
    pub fn handle_click(&self, x_px: f32, y_px: f32) -> Option<ContextMenuAction> {
        if !self.visible {
            return None;
        }
        let scale = self.scale.max(0.5);
        let layout = self.layout.as_ref()?;
        let row_idx = layout.hit_test_dip(x_px / scale, y_px / scale)?;
        layout.action_at(row_idx).cloned()
    }

    /// Handle keyboard navigation and activation inside the menu.
    pub fn handle_key(&mut self, vk: u16) -> MenuKeyOutcome {
        if !self.visible {
            return MenuKeyOutcome::Ignored;
        }
        let Some(layout) = self.layout.as_ref() else {
            return MenuKeyOutcome::Ignored;
        };
        if vk == VK_ESCAPE.0 {
            return MenuKeyOutcome::Dismiss;
        }
        if vk == VK_RETURN.0 || vk == VK_SPACE.0 {
            if let Some(action) = self.selected_row.and_then(|idx| layout.action_at(idx)) {
                return MenuKeyOutcome::Activate(action.clone());
            }
            return MenuKeyOutcome::Ignored;
        }

        let indices = layout.actionable_indices();
        let next = if vk == VK_DOWN.0 {
            step_selection(&indices, self.selected_row, true)
        } else if vk == VK_UP.0 {
            step_selection(&indices, self.selected_row, false)
        } else if vk == VK_HOME.0 {
            indices.first().copied()
        } else if vk == VK_END.0 {
            indices.last().copied()
        } else {
            return MenuKeyOutcome::Ignored;
        };

        if self.selected_row != next {
            self.selected_row = next;
            let _ = self.paint();
            MenuKeyOutcome::SelectionChanged
        } else {
            MenuKeyOutcome::Ignored
        }
    }

    fn measure_text(&self, text: &str, fmt: &IDWriteTextFormat) -> Result<(f32, f32)> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: DirectWrite text layout measurement with valid UTF-16 slice.
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, fmt, 600.0, 40.0)?;
            let mut tm = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut tm)?;
            Ok((tm.widthIncludingTrailingWhitespace, tm.height))
        }
    }

    fn paint(&self) -> Result<()> {
        let Some(layout) = self.layout.as_ref() else {
            return Ok(());
        };
        let width_px = (layout.width_dip * self.scale).ceil().max(1.0) as i32;
        let height_px = (layout.height_dip * self.scale).ceil().max(1.0) as i32;
        let pal = MenuPalette::for_theme(self.theme, self.skin);
        let selected = self.selected_row;

        self.render_surface(self.origin_px, (width_px, height_px), self.dpi, |rt| {
            // SAFETY: Direct2D drawing commands inside active BeginDraw/EndDraw frame.
            unsafe {
                let bg_brush = rt.CreateSolidColorBrush(&pal.bg, None)?;
                let border_brush = rt.CreateSolidColorBrush(&pal.border, None)?;
                let text_brush = rt.CreateSolidColorBrush(&pal.text_primary, None)?;
                let header_brush = rt.CreateSolidColorBrush(&pal.text_header, None)?;
                let accent_brush = rt.CreateSolidColorBrush(&pal.accent, None)?;
                let danger_brush = rt.CreateSolidColorBrush(&pal.danger, None)?;
                let divider_brush = rt.CreateSolidColorBrush(&pal.divider, None)?;
                let hover_brush = rt.CreateSolidColorBrush(&pal.hover_bg, None)?;
                let accent_hover_brush = rt.CreateSolidColorBrush(&pal.accent_hover_bg, None)?;
                let danger_hover_brush = rt.CreateSolidColorBrush(&pal.danger_hover_bg, None)?;

                let outer = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: 0.5,
                        top: 0.5,
                        right: layout.width_dip - 0.5,
                        bottom: layout.height_dip - 0.5,
                    },
                    radiusX: MENU_RADIUS_DIP,
                    radiusY: MENU_RADIUS_DIP,
                };
                rt.FillRoundedRectangle(&outer, &bg_brush);
                rt.DrawRoundedRectangle(&outer, &border_brush, 1.0, None);

                for (idx, row) in layout.rows.iter().enumerate() {
                    match row {
                        LaidOutRow::Header { text, rect } => {
                            let wide: Vec<u16> = text.encode_utf16().collect();
                            let text_rc = D2D_RECT_F {
                                left: rect.x + MENU_ITEM_PAD_X_DIP,
                                top: rect.y,
                                right: rect.x + rect.w - MENU_ITEM_PAD_X_DIP,
                                bottom: rect.y + rect.h,
                            };
                            rt.DrawText(
                                &wide,
                                &self.fmt_header,
                                &text_rc,
                                &header_brush,
                                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                                DWRITE_MEASURING_MODE_NATURAL,
                            );
                        }
                        LaidOutRow::Separator { y } => {
                            let div_rc = D2D_RECT_F {
                                left: MENU_PAD_X_DIP + MENU_SEP_INSET_DIP,
                                top: *y - 0.5,
                                right: layout.width_dip - MENU_PAD_X_DIP - MENU_SEP_INSET_DIP,
                                bottom: *y + 0.5,
                            };
                            rt.FillRectangle(&div_rc, &divider_brush);
                        }
                        LaidOutRow::Action {
                            label, tone, rect, ..
                        } => {
                            let is_sel = selected == Some(idx);
                            if is_sel {
                                let fill = match tone {
                                    MenuItemTone::Normal => &hover_brush,
                                    MenuItemTone::Accent => &accent_hover_brush,
                                    MenuItemTone::Danger => &danger_hover_brush,
                                };
                                let pill = D2D1_ROUNDED_RECT {
                                    rect: D2D_RECT_F {
                                        left: rect.x,
                                        top: rect.y,
                                        right: rect.x + rect.w,
                                        bottom: rect.y + rect.h,
                                    },
                                    radiusX: MENU_ITEM_RADIUS_DIP,
                                    radiusY: MENU_ITEM_RADIUS_DIP,
                                };
                                rt.FillRoundedRectangle(&pill, fill);
                            }

                            let (fmt, brush) = match tone {
                                MenuItemTone::Normal => (&self.fmt_item, &text_brush),
                                MenuItemTone::Accent => (&self.fmt_item_bold, &accent_brush),
                                MenuItemTone::Danger => {
                                    if is_sel {
                                        (&self.fmt_item, &danger_brush)
                                    } else {
                                        (&self.fmt_item, &text_brush)
                                    }
                                }
                            };
                            let wide: Vec<u16> = label.encode_utf16().collect();
                            let text_rc = D2D_RECT_F {
                                left: rect.x + MENU_ITEM_PAD_X_DIP,
                                top: rect.y,
                                right: rect.x + rect.w - MENU_ITEM_PAD_X_DIP,
                                bottom: rect.y + rect.h,
                            };
                            rt.DrawText(
                                &wide,
                                fmt,
                                &text_rc,
                                brush,
                                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                                DWRITE_MEASURING_MODE_NATURAL,
                            );
                        }
                    }
                }
                Ok(())
            }
        })
    }

    fn render_surface(
        &self,
        origin: (i32, i32),
        size: (i32, i32),
        dpi: u32,
        paint: impl FnOnce(&ID2D1DCRenderTarget) -> Result<()>,
    ) -> Result<()> {
        let (x, y) = origin;
        let (width_px, height_px) = size;
        if width_px <= 0 || height_px <= 0 {
            return Ok(());
        }

        // SAFETY: GDI DIB + Direct2D DC render target + UpdateLayeredWindow lifecycle.
        unsafe {
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen_dc));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width_px,
                    biHeight: -height_px,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(Some(screen_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old_obj = SelectObject(mem_dc, HGDIOBJ(bitmap.0));

            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: Default::default(),
            };

            let render_res = (|| -> Result<()> {
                let rt = self.d2d.CreateDCRenderTarget(&props)?;
                let bind_rect = RECT {
                    left: 0,
                    top: 0,
                    right: width_px,
                    bottom: height_px,
                };
                rt.BindDC(mem_dc, &bind_rect)?;
                rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                rt.BeginDraw();
                rt.Clear(Some(&D2D1_COLOR_F {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }));

                let draw_res = paint(&rt);
                let end_res = rt.EndDraw(None, None);
                draw_res?;
                end_res?;

                let pt_dst = POINT { x, y };
                let pt_src = POINT { x: 0, y: 0 };
                let size = SIZE {
                    cx: width_px,
                    cy: height_px,
                };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                UpdateLayeredWindow(
                    self.hwnd,
                    None,
                    Some(&pt_dst),
                    Some(&size),
                    Some(mem_dc),
                    Some(&pt_src),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                )?;
                Ok(())
            })();

            SelectObject(mem_dc, old_obj);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);

            render_res
        }
    }
}

impl Drop for ContextMenuWindow {
    fn drop(&mut self) {
        MENU_ACTIVE.store(false, Ordering::Release);
        // SAFETY: destroys our own menu popup HWND.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe fn make_text_format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size_dip: f32,
) -> Result<IDWriteTextFormat> {
    let fmt = dwrite.CreateTextFormat(
        family,
        None::<&IDWriteFontCollection>,
        weight,
        DWRITE_FONT_STYLE_NORMAL,
        DWRITE_FONT_STRETCH_NORMAL,
        size_dip,
        w!("en-US"),
    )?;
    fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
    fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    Ok(fmt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_items() -> Vec<ContextMenuItem> {
        vec![
            ContextMenuItem::Header("UsageDeck v2.16.4".to_string()),
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
            ContextMenuItem::Separator,
            ContextMenuItem::Action {
                label: "Quit UsageDeck".to_string(),
                action: ContextMenuAction::QuitApp,
                tone: MenuItemTone::Danger,
            },
        ]
    }

    #[test]
    fn layout_skips_header_and_separator_in_hit_testing() {
        let layout = MenuLayout::compute(&sample_items(), 200.0);
        assert_eq!(layout.width_dip, 200.0);

        // Header sits at y = 6..28 -> non-clickable.
        assert_eq!(layout.hit_test_dip(40.0, 14.0), None);
        // First action ("Settings") starts at 6 + 22 + 2 = 30..60.
        assert_eq!(layout.hit_test_dip(40.0, 42.0), Some(1));
        assert_eq!(layout.action_at(1), Some(&ContextMenuAction::OpenSettings));
        // Second action ("Refresh now") sits at 60..90.
        assert_eq!(layout.hit_test_dip(40.0, 72.0), Some(2));
        // Separator sits at 90..99 -> non-clickable.
        assert_eq!(layout.hit_test_dip(40.0, 94.0), None);
        // Quit action sits at 99..129.
        assert_eq!(layout.hit_test_dip(40.0, 110.0), Some(4));
        assert_eq!(layout.action_at(4), Some(&ContextMenuAction::QuitApp));
    }

    #[test]
    fn keyboard_selection_wraps_only_over_actionable_rows() {
        let layout = MenuLayout::compute(&sample_items(), 200.0);
        let indices = layout.actionable_indices();
        assert_eq!(indices, vec![1, 2, 4]);

        assert_eq!(step_selection(&indices, None, true), Some(1));
        assert_eq!(step_selection(&indices, Some(1), true), Some(2));
        assert_eq!(step_selection(&indices, Some(2), true), Some(4));
        assert_eq!(step_selection(&indices, Some(4), true), Some(1));

        assert_eq!(step_selection(&indices, None, false), Some(4));
        assert_eq!(step_selection(&indices, Some(1), false), Some(4));
        assert_eq!(step_selection(&indices, Some(4), false), Some(2));
    }

    #[test]
    fn cursor_origin_clamps_inside_monitor_and_prefers_above() {
        let mon = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        // Bottom-right tray icon click: shifts left of right edge and opens above cursor.
        let (x, y) = clamp_cursor_origin(POINT { x: 1860, y: 1040 }, 200, 180, mon, 8, 6);
        assert!(x + 200 <= mon.right - 8);
        assert!(y + 180 <= 1040);

        // Near top of screen: falls back below cursor without clipping above monitor.
        let (x2, y2) = clamp_cursor_origin(POINT { x: 100, y: 40 }, 200, 180, mon, 8, 6);
        assert_eq!(x2, 100);
        assert_eq!(y2, 46);
    }
}
