//! Native Direct2D/DirectWrite layered hover popup window for UsageDeck.
//!
//! Supports two Fluent Windows 11 presentation modes without stealing focus
//! (`WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST`):
//!
//! 1. **Compact toast / micro-tooltip** (`show`): crisp rounded pill for status toasts
//!    (`"Key copied"`, `"Hidden until reset"`).
//! 2. **Clean Fluent Hover Card** (`show_provider_card`): provider header with inline rate/reset
//!    badges and top-right Settings button, plus enabled usage bars in the active Dark or Light theme.

use crate::icons::IconAtlas;
use crate::layout::DipRect;
use crate::taskbar::{TaskbarMetrics, TaskbarTheme};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use usagedeck_core::sysmon::{
    format_bytes_compact, format_memory_summary, AttributedConsumer, SystemSnapshot,
};
use usagedeck_core::view::{BarSeverity, BarView, ProviderItem, ProviderKind};
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1StrokeStyle,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LoadCursorW, PostMessageW, RegisterClassExW,
    SetCursor, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, IDC_ARROW, IDC_HAND,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA,
    WM_APP, WM_LBUTTONUP, WM_MOUSEMOVE, WM_SETCURSOR, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const WM_NCHITTEST: u32 = 0x0084;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_MOUSELEAVE: u32 = 0x02A3;
const HTCLIENT: isize = 1;
const HTTRANSPARENT: isize = -1;
const MA_NOACTIVATE: isize = 3;

/// Posted to the strip window whenever the cursor moves inside an interactive hover card.
pub const WM_APP_HOVER_CARD_MOVE: u32 = WM_APP + 5;
/// Posted to the strip window when the cursor leaves an interactive hover card.
pub const WM_APP_HOVER_CARD_LEAVE: u32 = WM_APP + 6;
/// Posted to the strip window on left-click release inside an interactive hover card.
pub const WM_APP_HOVER_CARD_CLICK: u32 = WM_APP + 7;

static TOOLTIP_INTERACTIVE: AtomicBool = AtomicBool::new(false);
static TOOLTIP_OVER_BUTTON: AtomicBool = AtomicBool::new(false);
static TOOLTIP_TRACKING_LEAVE: AtomicBool = AtomicBool::new(false);
static TOOLTIP_OWNER_RAW: AtomicIsize = AtomicIsize::new(0);

/// Clickable action embedded in the provider or system hover card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoverCardAction {
    OpenSettings(ProviderKind),
    OpenWidgetSettings,
    HideUntilReset {
        provider: ProviderKind,
        account_id: String,
        title: String,
        until_ms: u64,
    },
    CopyOpenCodeKey(String),
    OpenCursor,
}

// Compact toast / micro-tooltip layout constants.
const TOOLTIP_MAX_TEXT_WIDTH_DIP: f32 = 340.0;
const TOOLTIP_PAD_X_DIP: f32 = 10.0;
const TOOLTIP_PAD_Y_DIP: f32 = 6.0;
const TOOLTIP_RADIUS_DIP: f32 = 7.0;
const TOOLTIP_GAP_DIP: f32 = 6.0;
const TOOLTIP_MARGIN_DIP: f32 = 8.0;

// Rich Fluent Hover Detail Card layout constants.
const CARD_WIDTH_DIP: f32 = 352.0;
const CARD_MIN_WIDTH_DIP: f32 = 220.0;
const CARD_PAD_X_DIP: f32 = 14.0;
const CARD_PAD_Y_DIP: f32 = 12.0;
const CARD_BOTTOM_PAD_DIP: f32 = 12.0;
const CARD_RADIUS_DIP: f32 = 10.0;
const CARD_GAP_DIP: f32 = 8.0;
const CARD_MARGIN_DIP: f32 = 8.0;

const HEADER_ROW_H_DIP: f32 = 24.0;
const HEADER_ICON_DIP: f32 = 20.0;
const HEADER_ICON_GAP_DIP: f32 = 8.0;
const HEADER_SUB_H_DIP: f32 = 14.0;
const HEADER_MIN_TITLE_W_DIP: f32 = 52.0;

const BADGE_H_DIP: f32 = 18.0;
const BADGE_PAD_X_DIP: f32 = 6.5;
const BADGE_GAP_DIP: f32 = 5.0;
const BADGE_RADIUS_DIP: f32 = 5.0;

const SECTION_GAP_DIP: f32 = 10.0;
const BAR_GAP_DIP: f32 = 10.0;
const BAR_META_H_DIP: f32 = 16.0;
const BAR_PCT_W_DIP: f32 = 38.0;
const BAR_TRACK_GAP_DIP: f32 = 4.0;
const BAR_TRACK_H_DIP: f32 = 6.0;
const BAR_TRACK_RADIUS_DIP: f32 = 3.0;
const BAR_PACE_TICK_W_DIP: f32 = 1.5;
const BAR_PACE_TICK_H_DIP: f32 = 10.0;

const FOOTER_DIVIDER_GAP_DIP: f32 = 7.0;
const FOOTER_MIN_H_DIP: f32 = 24.0;
const FOOTER_BTN_H_DIP: f32 = 24.0;
const FOOTER_BTN_PAD_X_DIP: f32 = 9.0;
const FOOTER_BTN_GAP_DIP: f32 = 6.0;
const FOOTER_BTN_RADIUS_DIP: f32 = 6.0;

#[derive(Clone, Copy)]
struct HoverPalette {
    card_bg: D2D1_COLOR_F,
    card_border: D2D1_COLOR_F,
    top_highlight: D2D1_COLOR_F,
    text_primary: D2D1_COLOR_F,
    text_muted: D2D1_COLOR_F,
    text_caption: D2D1_COLOR_F,
    track_bg: D2D1_COLOR_F,
    pace_tick: D2D1_COLOR_F,
    ok: D2D1_COLOR_F,
    warn: D2D1_COLOR_F,
    danger: D2D1_COLOR_F,
    muted: D2D1_COLOR_F,
    accent: D2D1_COLOR_F,
    divider: D2D1_COLOR_F,
    badge_neutral_bg: D2D1_COLOR_F,
    badge_neutral_border: D2D1_COLOR_F,
}

impl HoverPalette {
    fn for_theme(theme: TaskbarTheme) -> Self {
        match theme {
            TaskbarTheme::Dark => Self {
                card_bg: rgba(24, 25, 30, 0.97),
                card_border: rgba(255, 255, 255, 0.14),
                top_highlight: rgba(255, 255, 255, 0.06),
                text_primary: rgba(244, 244, 246, 0.98),
                text_muted: rgba(161, 161, 170, 0.92),
                text_caption: rgba(113, 113, 122, 0.95),
                track_bg: rgba(255, 255, 255, 0.12),
                pace_tick: rgba(255, 255, 255, 0.65),
                ok: rgba(16, 185, 129, 1.0),
                warn: rgba(245, 158, 11, 1.0),
                danger: rgba(239, 68, 68, 1.0),
                muted: rgba(113, 113, 122, 1.0),
                accent: rgba(96, 165, 250, 1.0),
                divider: rgba(255, 255, 255, 0.09),
                badge_neutral_bg: rgba(255, 255, 255, 0.08),
                badge_neutral_border: rgba(255, 255, 255, 0.12),
            },
            TaskbarTheme::Light => Self {
                card_bg: rgba(250, 251, 253, 0.98),
                card_border: rgba(0, 0, 0, 0.12),
                top_highlight: rgba(255, 255, 255, 0.85),
                text_primary: rgba(20, 20, 24, 0.96),
                text_muted: rgba(82, 82, 91, 0.90),
                text_caption: rgba(113, 113, 122, 0.95),
                track_bg: rgba(24, 24, 28, 0.12),
                pace_tick: rgba(24, 24, 28, 0.55),
                ok: rgba(5, 150, 105, 1.0),
                warn: rgba(217, 119, 6, 1.0),
                danger: rgba(220, 38, 38, 1.0),
                muted: rgba(113, 113, 122, 1.0),
                accent: rgba(37, 99, 235, 1.0),
                divider: rgba(0, 0, 0, 0.09),
                badge_neutral_bg: rgba(0, 0, 0, 0.055),
                badge_neutral_border: rgba(0, 0, 0, 0.10),
            },
        }
    }

    fn severity_color(&self, sev: BarSeverity) -> D2D1_COLOR_F {
        match sev {
            BarSeverity::Ok => self.ok,
            BarSeverity::Warn => self.warn,
            BarSeverity::Danger => self.danger,
            BarSeverity::Muted => self.muted,
        }
    }

    fn percent_color(&self, sev: BarSeverity) -> D2D1_COLOR_F {
        match sev {
            BarSeverity::Danger => self.danger,
            BarSeverity::Warn => self.warn,
            BarSeverity::Muted => self.text_muted,
            BarSeverity::Ok => self.text_primary,
        }
    }

    fn tinted_pill(
        tone: D2D1_COLOR_F,
        theme: TaskbarTheme,
    ) -> (D2D1_COLOR_F, D2D1_COLOR_F, D2D1_COLOR_F) {
        let (bg_a, border_a) = match theme {
            TaskbarTheme::Dark => (0.16, 0.34),
            TaskbarTheme::Light => (0.12, 0.28),
        };
        (with_alpha(tone, bg_a), with_alpha(tone, border_a), tone)
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

fn with_alpha(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a, ..c }
}

/// Mix `base` with white in sRGB space (`base_weight * base + (1 - base_weight) * white`).
fn mix_with_white(base: D2D1_COLOR_F, base_weight: f32) -> D2D1_COLOR_F {
    let w = base_weight.clamp(0.0, 1.0);
    let white = 1.0 - w;
    D2D1_COLOR_F {
        r: (base.r * w + white).clamp(0.0, 1.0),
        g: (base.g * w + white).clamp(0.0, 1.0),
        b: (base.b * w + white).clamp(0.0, 1.0),
        a: base.a,
    }
}

fn to_d2d_rect(r: DipRect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x,
        top: r.y,
        right: r.x + r.w,
        bottom: r.y + r.h,
    }
}

fn rounded_rect(r: DipRect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: to_d2d_rect(r),
        radiusX: radius,
        radiusY: radius,
    }
}

fn provider_badge_style(
    kind: ProviderKind,
    theme: TaskbarTheme,
) -> (&'static str, D2D1_COLOR_F, D2D1_COLOR_F) {
    let dark = theme == TaskbarTheme::Dark;
    match kind {
        ProviderKind::Claude => (
            "C",
            rgba(217, 119, 87, if dark { 0.28 } else { 0.22 }),
            if dark {
                rgba(242, 160, 128, 1.0)
            } else {
                rgba(180, 83, 52, 1.0)
            },
        ),
        ProviderKind::Cursor => (
            "▲",
            rgba(56, 189, 248, if dark { 0.24 } else { 0.20 }),
            if dark {
                rgba(103, 211, 255, 1.0)
            } else {
                rgba(3, 105, 161, 1.0)
            },
        ),
        ProviderKind::Codex => (
            "◎",
            rgba(161, 161, 170, if dark { 0.24 } else { 0.22 }),
            if dark {
                rgba(228, 228, 231, 1.0)
            } else {
                rgba(39, 39, 42, 1.0)
            },
        ),
        ProviderKind::Grok => (
            "G",
            rgba(168, 85, 247, if dark { 0.25 } else { 0.20 }),
            if dark {
                rgba(216, 180, 254, 1.0)
            } else {
                rgba(126, 34, 206, 1.0)
            },
        ),
        ProviderKind::OpenCode => (
            "O",
            rgba(16, 185, 129, if dark { 0.24 } else { 0.20 }),
            if dark {
                rgba(110, 231, 183, 1.0)
            } else {
                rgba(4, 120, 87, 1.0)
            },
        ),
        ProviderKind::Antigravity => (
            "A",
            rgba(96, 165, 250, if dark { 0.24 } else { 0.20 }),
            if dark {
                rgba(147, 197, 253, 1.0)
            } else {
                rgba(29, 78, 216, 1.0)
            },
        ),
    }
}

fn friendly_bar_label(label: &str) -> &str {
    match label {
        "5h" | "Rolling" => "5-hour",
        "Weekly" => "Weekly",
        "Monthly" => "Monthly",
        "1P" => "Cursor models",
        "API" => "API models",
        other => other,
    }
}

fn combine_reset_parts(short: Option<&str>, local: Option<&str>) -> Option<String> {
    let s = short.map(str::trim).filter(|v| !v.is_empty());
    let l = local.map(str::trim).filter(|v| !v.is_empty());
    match (s, l) {
        (Some(sv), Some(lv)) if !lv.eq_ignore_ascii_case(sv) => Some(format!("{sv} · {lv}")),
        (Some(sv), _) => Some(sv.to_string()),
        (None, Some(lv)) => Some(lv.to_string()),
        (None, None) => None,
    }
}

fn pick_header_subtitle(
    title: &str,
    display_email: Option<&str>,
    subtitle: Option<&str>,
) -> Option<String> {
    let title_norm = title.trim();
    if let Some(email) = display_email
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case(title_norm))
    {
        return Some(email.to_string());
    }
    subtitle
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case(title_norm))
        .map(str::to_string)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FooterStyle {
    HintAccent,
    HintOk,
    HintDanger,
    Exhausted,
    Muted,
}

fn resolve_footer(item: &ProviderItem, footer_hint: Option<&str>) -> (String, FooterStyle) {
    if let Some(hint) = footer_hint.map(str::trim).filter(|s| !s.is_empty()) {
        let lower = hint.to_ascii_lowercase();
        let style = if hint.contains('×') || lower.contains("hide") {
            FooterStyle::HintDanger
        } else if lower.contains("copy") {
            FooterStyle::HintOk
        } else {
            FooterStyle::HintAccent
        };
        return (hint.to_string(), style);
    }

    if item.exhausted && item.exhausted_until_ms.is_some() {
        return ("Limit full".to_string(), FooterStyle::Exhausted);
    }

    (String::new(), FooterStyle::Muted)
}

fn make_text_format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
    align: DWRITE_TEXT_ALIGNMENT,
    wrap: bool,
) -> Result<IDWriteTextFormat> {
    let no_coll: Option<&IDWriteFontCollection> = None;
    // SAFETY: DirectWrite COM factory call with valid static strings.
    unsafe {
        let fmt = dwrite.CreateTextFormat(
            family,
            no_coll,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        fmt.SetTextAlignment(align)?;
        fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        fmt.SetWordWrapping(if wrap {
            DWRITE_WORD_WRAPPING_WRAP
        } else {
            DWRITE_WORD_WRAPPING_NO_WRAP
        })?;
        Ok(fmt)
    }
}

fn fill_rounded(
    rt: &ID2D1DCRenderTarget,
    rect: DipRect,
    radius: f32,
    color: D2D1_COLOR_F,
) -> Result<()> {
    // SAFETY: `rt` is inside an active BeginDraw/EndDraw pair.
    unsafe {
        let brush = rt.CreateSolidColorBrush(&color, None)?;
        rt.FillRoundedRectangle(&rounded_rect(rect, radius), &brush);
    }
    Ok(())
}

fn stroke_rounded(
    rt: &ID2D1DCRenderTarget,
    rect: DipRect,
    radius: f32,
    stroke_width: f32,
    color: D2D1_COLOR_F,
) -> Result<()> {
    let no_style: Option<&ID2D1StrokeStyle> = None;
    // SAFETY: `rt` is inside an active BeginDraw/EndDraw pair.
    unsafe {
        let brush = rt.CreateSolidColorBrush(&color, None)?;
        rt.DrawRoundedRectangle(&rounded_rect(rect, radius), &brush, stroke_width, no_style);
    }
    Ok(())
}

fn draw_text(
    rt: &ID2D1DCRenderTarget,
    text: &str,
    fmt: &IDWriteTextFormat,
    rect: DipRect,
    color: D2D1_COLOR_F,
) -> Result<()> {
    let wide: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: `rt` is inside an active BeginDraw/EndDraw pair.
    unsafe {
        let brush = rt.CreateSolidColorBrush(&color, None)?;
        rt.DrawText(
            &wide,
            fmt,
            &to_d2d_rect(rect),
            &brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    Ok(())
}

/// Convert a strip-relative DIP pill rectangle into physical screen pixel coordinates.
pub fn pill_screen_rect_px(strip_x: i32, strip_y: i32, pill_rect: DipRect, scale: f32) -> RECT {
    let left = strip_x + (pill_rect.x * scale).round() as i32;
    let top = strip_y + (pill_rect.y * scale).round() as i32;
    let right = strip_x + ((pill_rect.x + pill_rect.w) * scale).round() as i32;
    let bottom = strip_y + ((pill_rect.y + pill_rect.h) * scale).round() as i32;
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

unsafe extern "system" fn tooltip_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let interactive = TOOLTIP_INTERACTIVE.load(Ordering::Acquire);
    match msg {
        WM_NCHITTEST => LRESULT(if interactive { HTCLIENT } else { HTTRANSPARENT }),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE),
        WM_SETCURSOR if interactive && (lparam.0 & 0xffff) == HTCLIENT => {
            let cursor_id = if TOOLTIP_OVER_BUTTON.load(Ordering::Acquire) {
                IDC_HAND
            } else {
                IDC_ARROW
            };
            if let Ok(cur) = LoadCursorW(None, cursor_id) {
                SetCursor(Some(cur));
            }
            LRESULT(1)
        }
        WM_MOUSEMOVE if interactive => {
            if !TOOLTIP_TRACKING_LEAVE.swap(true, Ordering::AcqRel) {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
            let owner = TOOLTIP_OWNER_RAW.load(Ordering::Acquire);
            if owner != 0 {
                let owner_hwnd = HWND(owner as *mut std::ffi::c_void);
                let _ = PostMessageW(Some(owner_hwnd), WM_APP_HOVER_CARD_MOVE, WPARAM(0), lparam);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            TOOLTIP_TRACKING_LEAVE.store(false, Ordering::Release);
            TOOLTIP_OVER_BUTTON.store(false, Ordering::Release);
            let owner = TOOLTIP_OWNER_RAW.load(Ordering::Acquire);
            if owner != 0 {
                let owner_hwnd = HWND(owner as *mut std::ffi::c_void);
                let _ = PostMessageW(
                    Some(owner_hwnd),
                    WM_APP_HOVER_CARD_LEAVE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            LRESULT(0)
        }
        WM_LBUTTONUP if interactive => {
            let owner = TOOLTIP_OWNER_RAW.load(Ordering::Acquire);
            if owner != 0 {
                let owner_hwnd = HWND(owner as *mut std::ffi::c_void);
                let _ = PostMessageW(Some(owner_hwnd), WM_APP_HOVER_CARD_CLICK, WPARAM(0), lparam);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[derive(PartialEq)]
enum TooltipCacheKey {
    Compact {
        text: String,
        anchor_cx: i32,
        anchor_top: i32,
        dpi: u32,
        theme: TaskbarTheme,
    },
    ProviderCard {
        item: Box<ProviderItem>,
        anchor_cx: i32,
        anchor_top: i32,
        dpi: u32,
        theme: TaskbarTheme,
        footer_text: String,
        footer_style: FooterStyle,
        hovered_button: Option<HoverCardAction>,
    },
    SystemCard {
        snapshot: Box<SystemSnapshot>,
        anchor_cx: i32,
        anchor_top: i32,
        dpi: u32,
        theme: TaskbarTheme,
        hovered_button: Option<HoverCardAction>,
    },
}

struct BadgeSpec {
    text: String,
    bg: D2D1_COLOR_F,
    border: D2D1_COLOR_F,
    fg: D2D1_COLOR_F,
}

struct HeaderBadge {
    text: String,
    rect: DipRect,
    bg: D2D1_COLOR_F,
    border: D2D1_COLOR_F,
    fg: D2D1_COLOR_F,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HoverButtonTone {
    Accent,
    Neutral,
    Danger,
    Warn,
}

#[derive(Clone)]
struct HoverCardButton {
    label: String,
    rect: DipRect,
    action: HoverCardAction,
    tone: HoverButtonTone,
}

struct MeasuredBar<'a> {
    bar: &'a BarView,
    y: f32,
    label: String,
    label_w: f32,
    reset_text: Option<String>,
}

struct CardLayout<'a> {
    card_w_dip: f32,
    card_h_dip: f32,
    content_x: f32,
    content_w: f32,
    icon_rect: DipRect,
    title_text: String,
    title_rect: DipRect,
    subtitle: Option<(String, DipRect)>,
    badges: Vec<HeaderBadge>,
    bars: Vec<MeasuredBar<'a>>,
    divider_y: Option<f32>,
    footer_text: String,
    footer_style: FooterStyle,
    footer_rect: DipRect,
    buttons: Vec<HoverCardButton>,
}

/// Native Win32/Direct2D hover popup anchored above taskbar pills.
pub struct TooltipWindow {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    fmt_compact: IDWriteTextFormat,
    fmt_icon_fallback: IDWriteTextFormat,
    fmt_title: IDWriteTextFormat,
    fmt_subtitle: IDWriteTextFormat,
    fmt_badge: IDWriteTextFormat,
    fmt_bar_label: IDWriteTextFormat,
    fmt_bar_reset: IDWriteTextFormat,
    fmt_bar_pct: IDWriteTextFormat,
    fmt_footer: IDWriteTextFormat,
    fmt_footer_bold: IDWriteTextFormat,
    fmt_button: IDWriteTextFormat,
    fmt_section_cap: IDWriteTextFormat,
    fmt_proc_name: IDWriteTextFormat,
    fmt_proc_val: IDWriteTextFormat,
    visible: bool,
    last_key: Option<TooltipCacheKey>,
    card_rect_px: Option<RECT>,
    card_scale: f32,
    card_buttons: Vec<HoverCardButton>,
    hovered_button: Option<HoverCardAction>,
}

impl TooltipWindow {
    /// Register the tooltip window class and create the hidden layered popup HWND.
    pub fn new() -> Result<Self> {
        // SAFETY: Direct2D/DirectWrite factories and layered Win32 popup lifecycle.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let fmt_compact = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_NORMAL,
                11.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?;
            let fmt_icon_fallback = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_BOLD,
                11.5,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
            )?;
            let fmt_title = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                13.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_subtitle = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_NORMAL,
                10.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_badge = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                9.5,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
            )?;
            let fmt_bar_label = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                11.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_bar_reset = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_NORMAL,
                10.0,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                false,
            )?;
            let fmt_bar_pct = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_BOLD,
                11.5,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                false,
            )?;
            let fmt_footer = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_NORMAL,
                10.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_footer_bold = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                10.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_button = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                10.5,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
            )?;
            let fmt_section_cap = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                9.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_proc_name = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_NORMAL,
                10.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_proc_val = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                10.5,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                false,
            )?;

            let hinstance = GetModuleHandleW(None)?;
            let class_name = w!("UsageDeckTooltipPopup");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(tooltip_wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class_name,
                w!("UsageDeck Tooltip"),
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
                fmt_compact,
                fmt_icon_fallback,
                fmt_title,
                fmt_subtitle,
                fmt_badge,
                fmt_bar_label,
                fmt_bar_reset,
                fmt_bar_pct,
                fmt_footer,
                fmt_footer_bold,
                fmt_button,
                fmt_section_cap,
                fmt_proc_name,
                fmt_proc_val,
                visible: false,
                last_key: None,
                card_rect_px: None,
                card_scale: 1.0,
                card_buttons: Vec::new(),
                hovered_button: None,
            })
        }
    }

    /// Bind the taskbar strip HWND that receives interactive hover card mouse/click events.
    pub fn set_owner_hwnd(&self, owner: HWND) {
        TOOLTIP_OWNER_RAW.store(owner.0 as isize, Ordering::Release);
    }

    /// Current screen-pixel bounding box when displaying an interactive provider hover card.
    pub fn card_screen_rect_px(&self) -> Option<RECT> {
        if self.visible && TOOLTIP_INTERACTIVE.load(Ordering::Acquire) {
            self.card_rect_px
        } else {
            None
        }
    }

    /// Hit-test interactive hover card buttons using client physical coordinates from `WM_MOUSEMOVE`/`WM_LBUTTONUP`.
    pub fn hit_test_button_px(&self, x_px: f32, y_px: f32) -> Option<HoverCardAction> {
        let scale = self.card_scale.max(0.5);
        let dx = x_px / scale;
        let dy = y_px / scale;
        self.card_buttons
            .iter()
            .find(|btn| btn.rect.contains(dx, dy))
            .map(|btn| btn.action.clone())
    }

    /// Update hovered button state. Returns `true` if visual hover state changed.
    pub fn set_hovered_button(&mut self, next: Option<HoverCardAction>) -> bool {
        TOOLTIP_OVER_BUTTON.store(next.is_some(), Ordering::Release);
        if self.hovered_button != next {
            self.hovered_button = next;
            true
        } else {
            false
        }
    }

    /// Show or update a compact toast / micro-tooltip above `anchor_rect_px` (physical screen
    /// pixels) in the active `theme` without stealing focus.
    pub fn show(
        &mut self,
        text: &str,
        anchor_rect_px: RECT,
        metrics: &TaskbarMetrics,
        theme: TaskbarTheme,
    ) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            self.hide();
            return;
        }

        TOOLTIP_INTERACTIVE.store(false, Ordering::Release);
        TOOLTIP_OVER_BUTTON.store(false, Ordering::Release);
        self.card_rect_px = None;
        self.card_buttons.clear();
        self.hovered_button = None;

        let anchor_cx = (anchor_rect_px.left + anchor_rect_px.right) / 2;
        let anchor_top = anchor_rect_px.top.min(metrics.tray_rect.top);
        let key = TooltipCacheKey::Compact {
            text: trimmed.to_string(),
            anchor_cx,
            anchor_top,
            dpi: metrics.dpi,
            theme,
        };
        if self.visible && self.last_key.as_ref() == Some(&key) {
            return;
        }

        if let Err(e) = self.paint_compact(trimmed, anchor_cx, anchor_top, metrics, theme) {
            log::warn!("tooltip render failed: {e}");
            return;
        }
        self.last_key = Some(key);
        self.reveal_topmost();
    }

    /// Show or update the interactive Windows 11 Fluent Hover Card for `item` anchored cleanly
    /// above `anchor_rect_px` (physical screen pixels) in the active `theme`.
    #[allow(clippy::too_many_arguments)]
    pub fn show_provider_card(
        &mut self,
        item: &ProviderItem,
        icons: &IconAtlas,
        anchor_rect_px: RECT,
        metrics: &TaskbarMetrics,
        theme: TaskbarTheme,
        footer_hint: Option<&str>,
    ) {
        let anchor_cx = (anchor_rect_px.left + anchor_rect_px.right) / 2;
        let anchor_top = anchor_rect_px.top.min(metrics.tray_rect.top);
        let (footer_text, footer_style) = resolve_footer(item, footer_hint);

        let key = TooltipCacheKey::ProviderCard {
            item: Box::new(item.clone()),
            anchor_cx,
            anchor_top,
            dpi: metrics.dpi,
            theme,
            footer_text: footer_text.clone(),
            footer_style,
            hovered_button: self.hovered_button.clone(),
        };
        if self.visible
            && TOOLTIP_INTERACTIVE.load(Ordering::Acquire)
            && self.last_key.as_ref() == Some(&key)
        {
            return;
        }

        let pal = HoverPalette::for_theme(theme);
        let layout = match self.measure_card(item, metrics, theme, &pal, footer_text, footer_style)
        {
            Ok(l) => l,
            Err(e) => {
                log::warn!("hover card measure failed: {e}");
                return;
            }
        };

        let card_rect_px = match self.paint_card(
            item,
            icons,
            &layout,
            anchor_cx,
            anchor_top,
            metrics,
            theme,
            &pal,
            self.hovered_button.as_ref(),
        ) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("hover card render failed: {e}");
                return;
            }
        };
        TOOLTIP_INTERACTIVE.store(true, Ordering::Release);
        self.card_rect_px = Some(card_rect_px);
        self.card_scale = metrics.scale;
        self.card_buttons = layout.buttons;
        self.last_key = Some(key);
        self.reveal_topmost();
    }

    /// Show or update the interactive System CPU & Memory hover card with parent-program
    /// attribution (`Claude (56)`, `WSL`, game titles, etc.) anchored above `anchor_rect_px`.
    pub fn show_system_card(
        &mut self,
        sys: &SystemSnapshot,
        anchor_rect_px: RECT,
        metrics: &TaskbarMetrics,
        theme: TaskbarTheme,
    ) {
        let anchor_cx = (anchor_rect_px.left + anchor_rect_px.right) / 2;
        let anchor_top = anchor_rect_px.top.min(metrics.tray_rect.top);

        let key = TooltipCacheKey::SystemCard {
            snapshot: Box::new(sys.clone()),
            anchor_cx,
            anchor_top,
            dpi: metrics.dpi,
            theme,
            hovered_button: self.hovered_button.clone(),
        };
        if self.visible
            && TOOLTIP_INTERACTIVE.load(Ordering::Acquire)
            && self.last_key.as_ref() == Some(&key)
        {
            return;
        }

        let pal = HoverPalette::for_theme(theme);
        let (card_rect_px, buttons) = match self.paint_system_card(
            sys,
            anchor_cx,
            anchor_top,
            metrics,
            theme,
            &pal,
            self.hovered_button.as_ref(),
        ) {
            Ok(res) => res,
            Err(e) => {
                log::warn!("system hover card render failed: {e}");
                return;
            }
        };
        TOOLTIP_INTERACTIVE.store(true, Ordering::Release);
        self.card_rect_px = Some(card_rect_px);
        self.card_scale = metrics.scale;
        self.card_buttons = buttons;
        self.last_key = Some(key);
        self.reveal_topmost();
    }

    /// Hide the tooltip or hover card if currently visible.
    pub fn hide(&mut self) {
        TOOLTIP_INTERACTIVE.store(false, Ordering::Release);
        TOOLTIP_OVER_BUTTON.store(false, Ordering::Release);
        TOOLTIP_TRACKING_LEAVE.store(false, Ordering::Release);
        self.card_rect_px = None;
        self.card_buttons.clear();
        self.hovered_button = None;
        if self.visible {
            // SAFETY: hides our own tooltip HWND.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
            self.last_key = None;
        }
    }

    fn reveal_topmost(&mut self) {
        // SAFETY: non-activating topmost show on our own tooltip HWND.
        unsafe {
            if !self.visible {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.visible = true;
            }
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    fn measure_text(
        &self,
        text: &str,
        fmt: &IDWriteTextFormat,
        max_w: f32,
        max_h: f32,
    ) -> Result<(f32, f32)> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: DirectWrite text layout measurement with valid UTF-16 slice.
        unsafe {
            let layout =
                self.dwrite
                    .CreateTextLayout(&wide, fmt, max_w.max(1.0), max_h.max(1.0))?;
            let mut tm = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut tm)?;
            Ok((tm.widthIncludingTrailingWhitespace, tm.height))
        }
    }

    fn truncate_to_width(&self, text: &str, fmt: &IDWriteTextFormat, max_w: f32) -> String {
        if max_w < 8.0 {
            return String::new();
        }
        if let Ok((w, _)) = self.measure_text(text, fmt, 2000.0, 40.0) {
            if w <= max_w {
                return text.to_string();
            }
        } else {
            return text.to_string();
        }

        let boundaries: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        let mut lo = 0usize;
        let mut hi = boundaries.len();
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let prefix = text[..boundaries[mid - 1]].trim_end();
            let candidate = format!("{prefix}…");
            let fits = self
                .measure_text(&candidate, fmt, 2000.0, 40.0)
                .is_ok_and(|(w, _)| w <= max_w);
            if fits {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        if lo == 0 {
            "…".to_string()
        } else {
            let prefix = text[..boundaries[lo - 1]].trim_end();
            format!("{prefix}…")
        }
    }

    fn commit_layered(
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

    fn paint_compact(
        &self,
        text: &str,
        anchor_cx: i32,
        anchor_top: i32,
        metrics: &TaskbarMetrics,
        theme: TaskbarTheme,
    ) -> Result<()> {
        let (raw_w, raw_h) =
            self.measure_text(text, &self.fmt_compact, TOOLTIP_MAX_TEXT_WIDTH_DIP, 160.0)?;
        let text_w_dip = raw_w.clamp(24.0, TOOLTIP_MAX_TEXT_WIDTH_DIP).ceil();
        let text_h_dip = raw_h.clamp(14.0, 140.0).ceil();

        let box_w_dip = text_w_dip + TOOLTIP_PAD_X_DIP * 2.0 + 2.0;
        let box_h_dip = text_h_dip + TOOLTIP_PAD_Y_DIP * 2.0 + 2.0;
        let width_px = (box_w_dip * metrics.scale).ceil() as i32;
        let height_px = (box_h_dip * metrics.scale).ceil() as i32;
        if width_px <= 0 || height_px <= 0 {
            return Ok(());
        }

        let margin_px = (TOOLTIP_MARGIN_DIP * metrics.scale).round() as i32;
        let gap_px = (TOOLTIP_GAP_DIP * metrics.scale).round() as i32;
        let min_x = metrics.monitor_rect.left + margin_px;
        let max_x = (metrics.monitor_rect.right - width_px - margin_px).max(min_x);
        let x = (anchor_cx - width_px / 2).clamp(min_x, max_x);
        let min_y = metrics.monitor_rect.top + margin_px;
        let max_y = (metrics.monitor_rect.bottom - height_px - margin_px).max(min_y);
        let y = (anchor_top - gap_px - height_px).clamp(min_y, max_y);

        let pal = HoverPalette::for_theme(theme);

        self.commit_layered((x, y), (width_px, height_px), metrics.dpi, |rt| {
            let outer = DipRect {
                x: 0.5,
                y: 0.5,
                w: box_w_dip - 1.0,
                h: box_h_dip - 1.0,
            };
            fill_rounded(rt, outer, TOOLTIP_RADIUS_DIP, pal.card_bg)?;
            fill_rounded(
                rt,
                DipRect {
                    x: TOOLTIP_RADIUS_DIP,
                    y: 1.0,
                    w: (box_w_dip - TOOLTIP_RADIUS_DIP * 2.0).max(0.0),
                    h: 1.0,
                },
                0.5,
                pal.top_highlight,
            )?;
            stroke_rounded(rt, outer, TOOLTIP_RADIUS_DIP, 1.0, pal.card_border)?;

            let text_rect = DipRect {
                x: TOOLTIP_PAD_X_DIP + 1.0,
                y: TOOLTIP_PAD_Y_DIP + 1.0,
                w: (box_w_dip - TOOLTIP_PAD_X_DIP * 2.0 - 2.0).max(1.0),
                h: (box_h_dip - TOOLTIP_PAD_Y_DIP * 2.0 - 2.0).max(1.0),
            };
            draw_text(rt, text, &self.fmt_compact, text_rect, pal.text_primary)
        })
    }

    fn measure_card<'a>(
        &self,
        item: &'a ProviderItem,
        metrics: &TaskbarMetrics,
        theme: TaskbarTheme,
        pal: &HoverPalette,
        footer_text: String,
        footer_style: FooterStyle,
    ) -> Result<CardLayout<'a>> {
        let scale = metrics.scale.max(0.5);
        let mon_w_dip = (metrics.monitor_rect.right - metrics.monitor_rect.left) as f32 / scale;
        let card_w_dip =
            CARD_WIDTH_DIP.min((mon_w_dip - CARD_MARGIN_DIP * 2.0).max(CARD_MIN_WIDTH_DIP));
        let content_x = CARD_PAD_X_DIP;
        let content_w = (card_w_dip - CARD_PAD_X_DIP * 2.0).max(120.0);

        let header_y = CARD_PAD_Y_DIP;
        let icon_rect = DipRect {
            x: content_x,
            y: header_y + (HEADER_ROW_H_DIP - HEADER_ICON_DIP) * 0.5,
            w: HEADER_ICON_DIP,
            h: HEADER_ICON_DIP,
        };
        let title_x = content_x + HEADER_ICON_DIP + HEADER_ICON_GAP_DIP;

        // Top-right Settings button.
        let (settings_tw, _) =
            self.measure_text("Settings", &self.fmt_button, 120.0, FOOTER_BTN_H_DIP)?;
        let settings_bw = (settings_tw.ceil() + FOOTER_BTN_PAD_X_DIP * 2.0).max(58.0);
        let settings_btn = HoverCardButton {
            label: "Settings".to_string(),
            rect: DipRect {
                x: content_x + content_w - settings_bw,
                y: header_y + (HEADER_ROW_H_DIP - FOOTER_BTN_H_DIP) * 0.5,
                w: settings_bw,
                h: FOOTER_BTN_H_DIP,
            },
            action: HoverCardAction::OpenSettings(item.provider),
            tone: HoverButtonTone::Accent,
        };
        let header_right_limit = settings_btn.rect.x - 8.0;

        // Collect inline header badges placed directly to the right of the provider name:
        // price/spend rate -> reset badge -> status badge.
        let mut badge_specs: Vec<BadgeSpec> = Vec::new();
        if let Some(rate) = item
            .spend_rate_text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let (bg, border, fg) = if item.spend_rate_severity == BarSeverity::Muted {
                (
                    pal.badge_neutral_bg,
                    pal.badge_neutral_border,
                    pal.text_muted,
                )
            } else {
                HoverPalette::tinted_pill(pal.severity_color(item.spend_rate_severity), theme)
            };
            badge_specs.push(BadgeSpec {
                text: rate.to_string(),
                bg,
                border,
                fg,
            });
        }
        if let Some(badge) = item
            .resets_badge
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            badge_specs.push(BadgeSpec {
                text: badge.to_string(),
                bg: pal.badge_neutral_bg,
                border: pal.badge_neutral_border,
                fg: pal.text_muted,
            });
        }
        if item.exhausted {
            let (bg, border, fg) = HoverPalette::tinted_pill(pal.danger, theme);
            badge_specs.push(BadgeSpec {
                text: "Limit full".to_string(),
                bg,
                border,
                fg,
            });
        } else if item.auth_expired {
            let (bg, border, fg) = HoverPalette::tinted_pill(pal.warn, theme);
            badge_specs.push(BadgeSpec {
                text: "Sign in".to_string(),
                bg,
                border,
                fg,
            });
        } else if item.stale {
            badge_specs.push(BadgeSpec {
                text: "Stale".to_string(),
                bg: pal.badge_neutral_bg,
                border: pal.badge_neutral_border,
                fg: pal.text_muted,
            });
        }

        let mut measured_specs: Vec<(BadgeSpec, f32)> = Vec::with_capacity(badge_specs.len());
        for spec in badge_specs {
            let (tw, _) = self.measure_text(&spec.text, &self.fmt_badge, 140.0, BADGE_H_DIP)?;
            let bw = (tw.ceil() + BADGE_PAD_X_DIP * 2.0).max(26.0);
            measured_specs.push((spec, bw));
        }
        let total_badges_w: f32 = measured_specs
            .iter()
            .enumerate()
            .map(|(i, (_, bw))| bw + if i > 0 { BADGE_GAP_DIP } else { 0.0 })
            .sum();

        let avail_left_w = (header_right_limit - title_x).max(HEADER_MIN_TITLE_W_DIP);
        let max_title_w = if total_badges_w > 0.0 {
            (avail_left_w - total_badges_w - 6.0).max(HEADER_MIN_TITLE_W_DIP)
        } else {
            avail_left_w
        };
        let title_text = self.truncate_to_width(&item.title, &self.fmt_title, max_title_w);
        let (actual_tw, _) =
            self.measure_text(&title_text, &self.fmt_title, max_title_w, HEADER_ROW_H_DIP)?;
        let title_w = (actual_tw.ceil() + 2.0).min(max_title_w);
        let title_rect = DipRect {
            x: title_x,
            y: header_y,
            w: title_w,
            h: HEADER_ROW_H_DIP,
        };

        let mut badges: Vec<HeaderBadge> = Vec::with_capacity(measured_specs.len());
        let badge_y = header_y + (HEADER_ROW_H_DIP - BADGE_H_DIP) * 0.5;
        let mut badge_x = title_x + title_w + 6.0;
        for (spec, bw) in measured_specs {
            if badge_x + bw > header_right_limit + 1.0 {
                break;
            }
            badges.push(HeaderBadge {
                text: spec.text,
                rect: DipRect {
                    x: badge_x,
                    y: badge_y,
                    w: bw,
                    h: BADGE_H_DIP,
                },
                bg: spec.bg,
                border: spec.border,
                fg: spec.fg,
            });
            badge_x += bw + BADGE_GAP_DIP;
        }

        // Optional subtitle below header (email only when show_provider_names is false, or re-auth).
        let raw_sub = pick_header_subtitle(
            &item.title,
            item.display_email.as_deref(),
            item.subtitle.as_deref(),
        );
        let (subtitle, mut cursor_y) = if let Some(sub) = raw_sub {
            let sub_max_w = (header_right_limit - title_x).max(40.0);
            let trunc_sub = self.truncate_to_width(&sub, &self.fmt_subtitle, sub_max_w);
            let sub_y = header_y + HEADER_ROW_H_DIP - 1.0;
            (
                Some((
                    trunc_sub,
                    DipRect {
                        x: title_x,
                        y: sub_y,
                        w: sub_max_w,
                        h: HEADER_SUB_H_DIP,
                    },
                )),
                sub_y + HEADER_SUB_H_DIP,
            )
        } else {
            (None, header_y + HEADER_ROW_H_DIP)
        };

        // Usage Bars section (already filtered by settings.hidden_bars; no per-model splits).
        let active_bars = if !item.hover_bars.is_empty() {
            &item.hover_bars
        } else {
            &item.flyout_bars
        };
        let mut bars: Vec<MeasuredBar<'a>> = Vec::with_capacity(active_bars.len());
        if !active_bars.is_empty() {
            cursor_y += SECTION_GAP_DIP;
            let usable_meta_w = (content_w - BAR_PCT_W_DIP - 6.0).max(60.0);

            for (idx, bar) in active_bars.iter().enumerate() {
                if idx > 0 {
                    cursor_y += BAR_GAP_DIP;
                }
                let bar_y = cursor_y;
                let raw_label = friendly_bar_label(&bar.label);
                let raw_reset =
                    combine_reset_parts(bar.reset_short.as_deref(), bar.reset_at_local.as_deref());

                let (label, label_w, reset_text) = if let Some(ref rs) = raw_reset {
                    let max_lbl_w = (usable_meta_w * 0.48).max(52.0);
                    let lbl = self.truncate_to_width(raw_label, &self.fmt_bar_label, max_lbl_w);
                    let (lw, _) =
                        self.measure_text(&lbl, &self.fmt_bar_label, max_lbl_w, BAR_META_H_DIP)?;
                    let actual_lbl_w = (lw.ceil() + 2.0).min(max_lbl_w);
                    let reset_avail_w = (usable_meta_w - actual_lbl_w - 6.0).max(0.0);
                    let reset = if reset_avail_w >= 20.0 {
                        Some(self.truncate_to_width(rs, &self.fmt_bar_reset, reset_avail_w))
                    } else {
                        None
                    };
                    (lbl, actual_lbl_w, reset)
                } else {
                    let lbl = self.truncate_to_width(raw_label, &self.fmt_bar_label, usable_meta_w);
                    (lbl, usable_meta_w, None)
                };

                cursor_y += BAR_META_H_DIP + BAR_TRACK_GAP_DIP + BAR_TRACK_H_DIP;

                bars.push(MeasuredBar {
                    bar,
                    y: bar_y,
                    label,
                    label_w,
                    reset_text,
                });
            }
        }

        // Contextual footer row (only shown when a contextual action or hint is present).
        let mut buttons: Vec<HoverCardButton> = vec![settings_btn];
        let mut footer_btn_specs: Vec<(String, HoverCardAction, HoverButtonTone)> = Vec::new();
        if item.exhausted {
            if let Some(until_ms) = item.exhausted_until_ms {
                footer_btn_specs.push((
                    "Hide".to_string(),
                    HoverCardAction::HideUntilReset {
                        provider: item.provider,
                        account_id: item.account_id.clone(),
                        title: item.title.clone(),
                        until_ms,
                    },
                    HoverButtonTone::Danger,
                ));
            }
        } else if item.provider == ProviderKind::OpenCode {
            footer_btn_specs.push((
                "Copy key".to_string(),
                HoverCardAction::CopyOpenCodeKey(item.account_id.clone()),
                HoverButtonTone::Neutral,
            ));
        } else if item.provider == ProviderKind::Cursor && item.can_open_cursor {
            footer_btn_specs.push((
                "Sign in".to_string(),
                HoverCardAction::OpenCursor,
                HoverButtonTone::Warn,
            ));
        }

        let (divider_y, footer_text, footer_rect) =
            if !footer_btn_specs.is_empty() || !footer_text.is_empty() {
                cursor_y += SECTION_GAP_DIP;
                let div_y = cursor_y;
                cursor_y += 1.0 + FOOTER_DIVIDER_GAP_DIP;
                let footer_h = FOOTER_MIN_H_DIP.max(FOOTER_BTN_H_DIP);

                let mut footer_buttons: Vec<HoverCardButton> =
                    Vec::with_capacity(footer_btn_specs.len());
                let mut btn_right = content_x + content_w;
                let btn_y = cursor_y + (footer_h - FOOTER_BTN_H_DIP) * 0.5;
                for (label, action, tone) in footer_btn_specs.into_iter().rev() {
                    let (tw, _) =
                        self.measure_text(&label, &self.fmt_button, 120.0, FOOTER_BTN_H_DIP)?;
                    let bw = (tw.ceil() + FOOTER_BTN_PAD_X_DIP * 2.0).max(46.0);
                    let bx = btn_right - bw;
                    footer_buttons.push(HoverCardButton {
                        label,
                        rect: DipRect {
                            x: bx,
                            y: btn_y,
                            w: bw,
                            h: FOOTER_BTN_H_DIP,
                        },
                        action,
                        tone,
                    });
                    btn_right = bx - FOOTER_BTN_GAP_DIP;
                }
                footer_buttons.reverse();
                buttons.extend(footer_buttons);

                let footer_avail_w = (btn_right - content_x).max(0.0);
                let footer_fmt = if footer_style == FooterStyle::Muted {
                    &self.fmt_footer
                } else {
                    &self.fmt_footer_bold
                };
                let trunc_footer = if footer_avail_w >= 28.0 && !footer_text.is_empty() {
                    self.truncate_to_width(&footer_text, footer_fmt, footer_avail_w)
                } else {
                    String::new()
                };
                let frect = DipRect {
                    x: content_x,
                    y: cursor_y,
                    w: footer_avail_w,
                    h: footer_h,
                };
                cursor_y += footer_h;
                (Some(div_y), trunc_footer, frect)
            } else {
                (None, String::new(), DipRect::default())
            };

        let card_h_dip = cursor_y + CARD_BOTTOM_PAD_DIP;

        Ok(CardLayout {
            card_w_dip,
            card_h_dip,
            content_x,
            content_w,
            icon_rect,
            title_text,
            title_rect,
            subtitle,
            badges,
            bars,
            divider_y,
            footer_text,
            footer_style,
            footer_rect,
            buttons,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_card(
        &self,
        item: &ProviderItem,
        icons: &IconAtlas,
        layout: &CardLayout<'_>,
        anchor_cx: i32,
        anchor_top: i32,
        metrics: &TaskbarMetrics,
        theme: TaskbarTheme,
        pal: &HoverPalette,
        hovered_button: Option<&HoverCardAction>,
    ) -> Result<RECT> {
        let width_px = (layout.card_w_dip * metrics.scale).ceil() as i32;
        let height_px = (layout.card_h_dip * metrics.scale).ceil() as i32;
        if width_px <= 0 || height_px <= 0 {
            return Ok(RECT::default());
        }

        let margin_px = (CARD_MARGIN_DIP * metrics.scale).round() as i32;
        let gap_px = (CARD_GAP_DIP * metrics.scale).round() as i32;
        let min_x = metrics.monitor_rect.left + margin_px;
        let max_x = (metrics.monitor_rect.right - width_px - margin_px).max(min_x);
        let x = (anchor_cx - width_px / 2).clamp(min_x, max_x);
        let min_y = metrics.monitor_rect.top + margin_px;
        let max_y = (metrics.monitor_rect.bottom - height_px - margin_px).max(min_y);
        let y = (anchor_top - gap_px - height_px).clamp(min_y, max_y);

        self.commit_layered((x, y), (width_px, height_px), metrics.dpi, |rt| {
            // Card outer surface + top inner specular highlight + outer border.
            let outer = DipRect {
                x: 0.5,
                y: 0.5,
                w: layout.card_w_dip - 1.0,
                h: layout.card_h_dip - 1.0,
            };
            fill_rounded(rt, outer, CARD_RADIUS_DIP, pal.card_bg)?;
            fill_rounded(
                rt,
                DipRect {
                    x: CARD_RADIUS_DIP,
                    y: 1.0,
                    w: (layout.card_w_dip - CARD_RADIUS_DIP * 2.0).max(0.0),
                    h: 1.0,
                },
                0.5,
                pal.top_highlight,
            )?;
            stroke_rounded(rt, outer, CARD_RADIUS_DIP, 1.0, pal.card_border)?;

            // 1. Header Row: 20x20 provider icon (or colored fallback), title, subtitle, badges.
            let light_surface = theme == TaskbarTheme::Light;
            if !icons.draw_provider(
                rt,
                item.provider,
                to_d2d_rect(layout.icon_rect),
                light_surface,
            ) {
                let (glyph, bg, fg) = provider_badge_style(item.provider, theme);
                fill_rounded(rt, layout.icon_rect, 5.0, bg)?;
                draw_text(rt, glyph, &self.fmt_icon_fallback, layout.icon_rect, fg)?;
            }

            draw_text(
                rt,
                &layout.title_text,
                &self.fmt_title,
                layout.title_rect,
                pal.text_primary,
            )?;
            if let Some((ref sub_text, sub_rect)) = layout.subtitle {
                draw_text(rt, sub_text, &self.fmt_subtitle, sub_rect, pal.text_muted)?;
            }
            for badge in &layout.badges {
                fill_rounded(rt, badge.rect, BADGE_RADIUS_DIP, badge.bg)?;
                stroke_rounded(rt, badge.rect, BADGE_RADIUS_DIP, 1.0, badge.border)?;
                draw_text(rt, &badge.text, &self.fmt_badge, badge.rect, badge.fg)?;
            }

            // 2. Usage Bars Section.
            let pct_x = layout.content_x + layout.content_w - BAR_PCT_W_DIP;
            for mb in &layout.bars {
                // Top metadata row: friendly label | reset info | bold percentage.
                draw_text(
                    rt,
                    &mb.label,
                    &self.fmt_bar_label,
                    DipRect {
                        x: layout.content_x,
                        y: mb.y,
                        w: mb.label_w,
                        h: BAR_META_H_DIP,
                    },
                    pal.text_primary,
                )?;

                if let Some(ref reset_text) = mb.reset_text {
                    let reset_x = layout.content_x + mb.label_w + 4.0;
                    let reset_w = (pct_x - 4.0 - reset_x).max(0.0);
                    if reset_w >= 16.0 {
                        draw_text(
                            rt,
                            reset_text,
                            &self.fmt_bar_reset,
                            DipRect {
                                x: reset_x,
                                y: mb.y,
                                w: reset_w,
                                h: BAR_META_H_DIP,
                            },
                            pal.text_muted,
                        )?;
                    }
                }

                let pct_label = mb.bar.percent_label();
                draw_text(
                    rt,
                    &pct_label,
                    &self.fmt_bar_pct,
                    DipRect {
                        x: pct_x,
                        y: mb.y,
                        w: BAR_PCT_W_DIP,
                        h: BAR_META_H_DIP,
                    },
                    pal.percent_color(mb.bar.severity),
                )?;

                // Progress track + severity fill + optional Fable lighter right-end segment + pace tick.
                let track_rect = DipRect {
                    x: layout.content_x,
                    y: mb.y + BAR_META_H_DIP + BAR_TRACK_GAP_DIP,
                    w: layout.content_w,
                    h: BAR_TRACK_H_DIP,
                };
                draw_card_bar_track(rt, track_rect, mb.bar, theme, pal)?;
            }

            // 3. Optional Footer Row: 1px horizontal divider hairline + left status caption.
            if let Some(div_y) = layout.divider_y {
                fill_rounded(
                    rt,
                    DipRect {
                        x: layout.content_x,
                        y: div_y,
                        w: layout.content_w,
                        h: 1.0,
                    },
                    0.5,
                    pal.divider,
                )?;
            }

            if !layout.footer_text.is_empty() && layout.footer_rect.w >= 28.0 {
                let (footer_fmt, footer_color) = match layout.footer_style {
                    FooterStyle::HintAccent => (&self.fmt_footer_bold, pal.accent),
                    FooterStyle::HintOk => (&self.fmt_footer_bold, pal.ok),
                    FooterStyle::HintDanger => (&self.fmt_footer_bold, pal.danger),
                    FooterStyle::Exhausted => (&self.fmt_footer_bold, with_alpha(pal.danger, 0.92)),
                    FooterStyle::Muted => (&self.fmt_footer, pal.text_caption),
                };
                draw_text(
                    rt,
                    &layout.footer_text,
                    footer_fmt,
                    layout.footer_rect,
                    footer_color,
                )?;
            }

            let dark = theme == TaskbarTheme::Dark;
            for btn in &layout.buttons {
                let is_hovered = hovered_button == Some(&btn.action);
                let (bg, border, fg) = match btn.tone {
                    HoverButtonTone::Accent => (
                        with_alpha(
                            pal.accent,
                            if is_hovered {
                                if dark {
                                    0.32
                                } else {
                                    0.22
                                }
                            } else if dark {
                                0.18
                            } else {
                                0.12
                            },
                        ),
                        with_alpha(
                            pal.accent,
                            if is_hovered {
                                0.78
                            } else if dark {
                                0.44
                            } else {
                                0.34
                            },
                        ),
                        if is_hovered && dark {
                            pal.text_primary
                        } else {
                            pal.accent
                        },
                    ),
                    HoverButtonTone::Neutral => (
                        if is_hovered {
                            if dark {
                                rgba(255, 255, 255, 0.16)
                            } else {
                                rgba(0, 0, 0, 0.10)
                            }
                        } else {
                            pal.badge_neutral_bg
                        },
                        if is_hovered {
                            if dark {
                                rgba(255, 255, 255, 0.28)
                            } else {
                                rgba(0, 0, 0, 0.22)
                            }
                        } else {
                            pal.badge_neutral_border
                        },
                        pal.text_primary,
                    ),
                    HoverButtonTone::Danger => (
                        with_alpha(pal.danger, if is_hovered { 0.28 } else { 0.16 }),
                        with_alpha(pal.danger, if is_hovered { 0.70 } else { 0.38 }),
                        if is_hovered && dark {
                            pal.text_primary
                        } else {
                            pal.danger
                        },
                    ),
                    HoverButtonTone::Warn => (
                        with_alpha(pal.warn, if is_hovered { 0.28 } else { 0.16 }),
                        with_alpha(pal.warn, if is_hovered { 0.70 } else { 0.38 }),
                        if is_hovered && dark {
                            pal.text_primary
                        } else {
                            pal.warn
                        },
                    ),
                };
                fill_rounded(rt, btn.rect, FOOTER_BTN_RADIUS_DIP, bg)?;
                stroke_rounded(rt, btn.rect, FOOTER_BTN_RADIUS_DIP, 1.0, border)?;
                draw_text(rt, &btn.label, &self.fmt_button, btn.rect, fg)?;
            }
            Ok(())
        })?;

        Ok(RECT {
            left: x,
            top: y,
            right: x + width_px,
            bottom: y + height_px,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_system_card(
        &self,
        sys: &SystemSnapshot,
        anchor_cx: i32,
        anchor_top: i32,
        metrics: &TaskbarMetrics,
        theme: TaskbarTheme,
        pal: &HoverPalette,
        hovered_button: Option<&HoverCardAction>,
    ) -> Result<(RECT, Vec<HoverCardButton>)> {
        const SECTION_CAP_H: f32 = 14.0;
        const SECTION_CAP_GAP: f32 = 4.0;
        const PROC_ROW_H: f32 = 20.0;
        const MINI_BAR_W: f32 = 56.0;
        const MINI_BAR_H: f32 = 4.0;
        const CPU_VAL_W: f32 = 42.0;
        const MEM_VAL_W: f32 = 56.0;

        let scale = metrics.scale.max(0.5);
        let mon_w_dip = (metrics.monitor_rect.right - metrics.monitor_rect.left) as f32 / scale;
        let card_w_dip =
            CARD_WIDTH_DIP.min((mon_w_dip - CARD_MARGIN_DIP * 2.0).max(CARD_MIN_WIDTH_DIP));
        let content_x = CARD_PAD_X_DIP;
        let content_w = (card_w_dip - CARD_PAD_X_DIP * 2.0).max(120.0);

        let header_y = CARD_PAD_Y_DIP;
        let icon_rect = DipRect {
            x: content_x,
            y: header_y + (HEADER_ROW_H_DIP - HEADER_ICON_DIP) * 0.5,
            w: HEADER_ICON_DIP,
            h: HEADER_ICON_DIP,
        };
        let title_x = content_x + HEADER_ICON_DIP + HEADER_ICON_GAP_DIP;

        // Top-right Settings button opening Settings -> Widget.
        let (settings_tw, _) =
            self.measure_text("Settings", &self.fmt_button, 120.0, FOOTER_BTN_H_DIP)?;
        let settings_bw = (settings_tw.ceil() + FOOTER_BTN_PAD_X_DIP * 2.0).max(58.0);
        let settings_btn = HoverCardButton {
            label: "Settings".to_string(),
            rect: DipRect {
                x: content_x + content_w - settings_bw,
                y: header_y + (HEADER_ROW_H_DIP - FOOTER_BTN_H_DIP) * 0.5,
                w: settings_bw,
                h: FOOTER_BTN_H_DIP,
            },
            action: HoverCardAction::OpenWidgetSettings,
            tone: HoverButtonTone::Accent,
        };
        let header_right_limit = settings_btn.rect.x - 8.0;

        let title_text = "System";
        let (title_tw, _) =
            self.measure_text(title_text, &self.fmt_title, 120.0, HEADER_ROW_H_DIP)?;
        let title_w = (title_tw.ceil() + 2.0).min((header_right_limit - title_x).max(40.0));
        let title_rect = DipRect {
            x: title_x,
            y: header_y,
            w: title_w,
            h: HEADER_ROW_H_DIP,
        };

        // Optional inline memory summary badge right beside "System" (e.g. "24.6 / 64 GB").
        let mem_summary = format_memory_summary(sys.memory_used_bytes, sys.memory_total_bytes);
        let mem_badge: Option<HeaderBadge> = if !mem_summary.is_empty() {
            let (bw_raw, _) =
                self.measure_text(&mem_summary, &self.fmt_badge, 140.0, BADGE_H_DIP)?;
            let bw = (bw_raw.ceil() + BADGE_PAD_X_DIP * 2.0).max(32.0);
            let bx = title_x + title_w + 6.0;
            if bx + bw <= header_right_limit + 1.0 {
                Some(HeaderBadge {
                    text: mem_summary,
                    rect: DipRect {
                        x: bx,
                        y: header_y + (HEADER_ROW_H_DIP - BADGE_H_DIP) * 0.5,
                        w: bw,
                        h: BADGE_H_DIP,
                    },
                    bg: pal.badge_neutral_bg,
                    border: pal.badge_neutral_border,
                    fg: pal.text_muted,
                })
            } else {
                None
            }
        } else {
            None
        };

        // Vertical cursor layout.
        let mut cursor_y = header_y + HEADER_ROW_H_DIP + SECTION_GAP_DIP;
        let cpu_bar_y = cursor_y;
        cursor_y += BAR_META_H_DIP + BAR_TRACK_GAP_DIP + BAR_TRACK_H_DIP + BAR_GAP_DIP;
        let mem_bar_y = cursor_y;
        cursor_y += BAR_META_H_DIP + BAR_TRACK_GAP_DIP + BAR_TRACK_H_DIP + SECTION_GAP_DIP;

        let divider_y = cursor_y;
        cursor_y += 1.0 + 9.0;

        let cpu_cap_y = cursor_y;
        cursor_y += SECTION_CAP_H + SECTION_CAP_GAP;
        let cpu_rows_y = cursor_y;
        let cpu_row_count = sys.top_cpu.len().max(1);
        cursor_y += cpu_row_count as f32 * PROC_ROW_H + 9.0;

        let mem_cap_y = cursor_y;
        cursor_y += SECTION_CAP_H + SECTION_CAP_GAP;
        let mem_rows_y = cursor_y;
        let mem_row_count = sys.top_memory.len().max(1);
        cursor_y += mem_row_count as f32 * PROC_ROW_H;

        let card_h_dip = cursor_y + CARD_BOTTOM_PAD_DIP;
        let width_px = (card_w_dip * metrics.scale).ceil() as i32;
        let height_px = (card_h_dip * metrics.scale).ceil() as i32;
        if width_px <= 0 || height_px <= 0 {
            return Ok((RECT::default(), Vec::new()));
        }

        let margin_px = (CARD_MARGIN_DIP * metrics.scale).round() as i32;
        let gap_px = (CARD_GAP_DIP * metrics.scale).round() as i32;
        let min_x = metrics.monitor_rect.left + margin_px;
        let max_x = (metrics.monitor_rect.right - width_px - margin_px).max(min_x);
        let x = (anchor_cx - width_px / 2).clamp(min_x, max_x);
        let min_y = metrics.monitor_rect.top + margin_px;
        let max_y = (metrics.monitor_rect.bottom - height_px - margin_px).max(min_y);
        let y = (anchor_top - gap_px - height_px).clamp(min_y, max_y);

        let buttons = vec![settings_btn];
        let dark = theme == TaskbarTheme::Dark;

        self.commit_layered((x, y), (width_px, height_px), metrics.dpi, |rt| {
            // Card surface + top inner highlight + border.
            let outer = DipRect {
                x: 0.5,
                y: 0.5,
                w: card_w_dip - 1.0,
                h: card_h_dip - 1.0,
            };
            fill_rounded(rt, outer, CARD_RADIUS_DIP, pal.card_bg)?;
            fill_rounded(
                rt,
                DipRect {
                    x: CARD_RADIUS_DIP,
                    y: 1.0,
                    w: (card_w_dip - CARD_RADIUS_DIP * 2.0).max(0.0),
                    h: 1.0,
                },
                0.5,
                pal.top_highlight,
            )?;
            stroke_rounded(rt, outer, CARD_RADIUS_DIP, 1.0, pal.card_border)?;

            // 1. Header: rounded PC badge, "System", memory badge, top-right Settings button.
            let (icon_bg, icon_border, icon_fg) = HoverPalette::tinted_pill(pal.accent, theme);
            fill_rounded(rt, icon_rect, 5.0, icon_bg)?;
            stroke_rounded(rt, icon_rect, 5.0, 1.0, icon_border)?;
            draw_text(rt, "PC", &self.fmt_badge, icon_rect, icon_fg)?;

            draw_text(
                rt,
                title_text,
                &self.fmt_title,
                title_rect,
                pal.text_primary,
            )?;
            if let Some(ref badge) = mem_badge {
                fill_rounded(rt, badge.rect, BADGE_RADIUS_DIP, badge.bg)?;
                stroke_rounded(rt, badge.rect, BADGE_RADIUS_DIP, 1.0, badge.border)?;
                draw_text(rt, &badge.text, &self.fmt_badge, badge.rect, badge.fg)?;
            }

            // 2. Live CPU & Memory summary bars.
            let draw_summary_bar = |label: &str, pct: f64, y_pos: f32| -> Result<()> {
                let clamped = pct.clamp(0.0, 100.0);
                let sev = system_load_severity(clamped);
                let pct_x = content_x + content_w - BAR_PCT_W_DIP;
                draw_text(
                    rt,
                    label,
                    &self.fmt_bar_label,
                    DipRect {
                        x: content_x,
                        y: y_pos,
                        w: (pct_x - content_x - 4.0).max(40.0),
                        h: BAR_META_H_DIP,
                    },
                    pal.text_primary,
                )?;
                let pct_str = format!("{:.0}%", clamped.round());
                draw_text(
                    rt,
                    &pct_str,
                    &self.fmt_bar_pct,
                    DipRect {
                        x: pct_x,
                        y: y_pos,
                        w: BAR_PCT_W_DIP,
                        h: BAR_META_H_DIP,
                    },
                    pal.percent_color(sev),
                )?;
                let track_rect = DipRect {
                    x: content_x,
                    y: y_pos + BAR_META_H_DIP + BAR_TRACK_GAP_DIP,
                    w: content_w,
                    h: BAR_TRACK_H_DIP,
                };
                fill_rounded(rt, track_rect, BAR_TRACK_RADIUS_DIP, pal.track_bg)?;
                let fill_w = (track_rect.w * (clamped as f32 / 100.0)).clamp(0.0, track_rect.w);
                if fill_w > 0.5 {
                    fill_rounded(
                        rt,
                        DipRect {
                            x: track_rect.x,
                            y: track_rect.y,
                            w: fill_w,
                            h: track_rect.h,
                        },
                        BAR_TRACK_RADIUS_DIP.min(fill_w * 0.5),
                        pal.severity_color(sev),
                    )?;
                }
                Ok(())
            };
            draw_summary_bar("CPU", sys.cpu_percent, cpu_bar_y)?;
            draw_summary_bar("Memory", sys.memory_percent, mem_bar_y)?;

            // 3. Hairline divider before attributed program breakdowns.
            fill_rounded(
                rt,
                DipRect {
                    x: content_x,
                    y: divider_y,
                    w: content_w,
                    h: 1.0,
                },
                0.5,
                pal.divider,
            )?;

            // 4. TOP CPU attributed programs.
            draw_text(
                rt,
                "TOP CPU",
                &self.fmt_section_cap,
                DipRect {
                    x: content_x,
                    y: cpu_cap_y,
                    w: content_w,
                    h: SECTION_CAP_H,
                },
                pal.text_caption,
            )?;
            if sys.top_cpu.is_empty() {
                draw_text(
                    rt,
                    "System idle",
                    &self.fmt_subtitle,
                    DipRect {
                        x: content_x,
                        y: cpu_rows_y,
                        w: content_w,
                        h: PROC_ROW_H,
                    },
                    pal.text_muted,
                )?;
            } else {
                let val_x = content_x + content_w - CPU_VAL_W;
                let bar_x = val_x - 8.0 - MINI_BAR_W;
                let name_w = (bar_x - 8.0 - content_x).max(60.0);
                for (idx, consumer) in sys.top_cpu.iter().enumerate() {
                    let row_y = cpu_rows_y + idx as f32 * PROC_ROW_H;
                    let display = self.truncate_to_width(
                        &consumer.display_name(),
                        &self.fmt_proc_name,
                        name_w,
                    );
                    draw_text(
                        rt,
                        &display,
                        &self.fmt_proc_name,
                        DipRect {
                            x: content_x,
                            y: row_y,
                            w: name_w,
                            h: PROC_ROW_H,
                        },
                        pal.text_primary,
                    )?;
                    let frac = (consumer.cpu_percent / 100.0).clamp(0.0, 1.0) as f32;
                    let row_sev = if consumer.cpu_percent >= 60.0 {
                        BarSeverity::Danger
                    } else if consumer.cpu_percent >= 30.0 {
                        BarSeverity::Warn
                    } else {
                        BarSeverity::Ok
                    };
                    let track = DipRect {
                        x: bar_x,
                        y: row_y + (PROC_ROW_H - MINI_BAR_H) * 0.5,
                        w: MINI_BAR_W,
                        h: MINI_BAR_H,
                    };
                    fill_rounded(rt, track, MINI_BAR_H * 0.5, pal.track_bg)?;
                    let fill_w = (track.w * frac).clamp(0.0, track.w);
                    if fill_w > 0.5 {
                        fill_rounded(
                            rt,
                            DipRect {
                                x: track.x,
                                y: track.y,
                                w: fill_w,
                                h: track.h,
                            },
                            (MINI_BAR_H * 0.5).min(fill_w * 0.5),
                            pal.severity_color(row_sev),
                        )?;
                    }
                    let val_str = format_consumer_cpu(consumer);
                    draw_text(
                        rt,
                        &val_str,
                        &self.fmt_proc_val,
                        DipRect {
                            x: val_x,
                            y: row_y,
                            w: CPU_VAL_W,
                            h: PROC_ROW_H,
                        },
                        pal.percent_color(row_sev),
                    )?;
                }
            }

            // 5. TOP MEMORY attributed programs.
            draw_text(
                rt,
                "TOP MEMORY",
                &self.fmt_section_cap,
                DipRect {
                    x: content_x,
                    y: mem_cap_y,
                    w: content_w,
                    h: SECTION_CAP_H,
                },
                pal.text_caption,
            )?;
            if sys.top_memory.is_empty() {
                draw_text(
                    rt,
                    "No active apps",
                    &self.fmt_subtitle,
                    DipRect {
                        x: content_x,
                        y: mem_rows_y,
                        w: content_w,
                        h: PROC_ROW_H,
                    },
                    pal.text_muted,
                )?;
            } else {
                let val_x = content_x + content_w - MEM_VAL_W;
                let bar_x = val_x - 8.0 - MINI_BAR_W;
                let name_w = (bar_x - 8.0 - content_x).max(60.0);
                let total_mem = sys.memory_total_bytes.max(1) as f64;
                for (idx, consumer) in sys.top_memory.iter().enumerate() {
                    let row_y = mem_rows_y + idx as f32 * PROC_ROW_H;
                    let display = self.truncate_to_width(
                        &consumer.display_name(),
                        &self.fmt_proc_name,
                        name_w,
                    );
                    draw_text(
                        rt,
                        &display,
                        &self.fmt_proc_name,
                        DipRect {
                            x: content_x,
                            y: row_y,
                            w: name_w,
                            h: PROC_ROW_H,
                        },
                        pal.text_primary,
                    )?;
                    let frac = (consumer.memory_bytes as f64 / total_mem).clamp(0.0, 1.0) as f32;
                    let bar_col = if frac >= 0.45 {
                        pal.danger
                    } else if frac >= 0.25 {
                        pal.warn
                    } else {
                        pal.accent
                    };
                    let track = DipRect {
                        x: bar_x,
                        y: row_y + (PROC_ROW_H - MINI_BAR_H) * 0.5,
                        w: MINI_BAR_W,
                        h: MINI_BAR_H,
                    };
                    fill_rounded(rt, track, MINI_BAR_H * 0.5, pal.track_bg)?;
                    let fill_w = (track.w * frac).clamp(0.0, track.w);
                    if fill_w > 0.5 {
                        fill_rounded(
                            rt,
                            DipRect {
                                x: track.x,
                                y: track.y,
                                w: fill_w,
                                h: track.h,
                            },
                            (MINI_BAR_H * 0.5).min(fill_w * 0.5),
                            bar_col,
                        )?;
                    }
                    let val_str = format_bytes_compact(consumer.memory_bytes);
                    draw_text(
                        rt,
                        &val_str,
                        &self.fmt_proc_val,
                        DipRect {
                            x: val_x,
                            y: row_y,
                            w: MEM_VAL_W,
                            h: PROC_ROW_H,
                        },
                        pal.text_primary,
                    )?;
                }
            }

            // 6. Top-right Settings button.
            for btn in &buttons {
                let is_hovered = hovered_button == Some(&btn.action);
                let bg = with_alpha(
                    pal.accent,
                    if is_hovered {
                        if dark {
                            0.32
                        } else {
                            0.22
                        }
                    } else if dark {
                        0.18
                    } else {
                        0.12
                    },
                );
                let border = with_alpha(
                    pal.accent,
                    if is_hovered {
                        0.78
                    } else if dark {
                        0.44
                    } else {
                        0.34
                    },
                );
                let fg = if is_hovered && dark {
                    pal.text_primary
                } else {
                    pal.accent
                };
                fill_rounded(rt, btn.rect, FOOTER_BTN_RADIUS_DIP, bg)?;
                stroke_rounded(rt, btn.rect, FOOTER_BTN_RADIUS_DIP, 1.0, border)?;
                draw_text(rt, &btn.label, &self.fmt_button, btn.rect, fg)?;
            }

            Ok(())
        })?;

        Ok((
            RECT {
                left: x,
                top: y,
                right: x + width_px,
                bottom: y + height_px,
            },
            buttons,
        ))
    }
}

fn system_load_severity(pct: f64) -> BarSeverity {
    if pct >= 90.0 {
        BarSeverity::Danger
    } else if pct >= 70.0 {
        BarSeverity::Warn
    } else {
        BarSeverity::Ok
    }
}

fn format_consumer_cpu(c: &AttributedConsumer) -> String {
    let pct = c.cpu_percent.clamp(0.0, 100.0);
    if pct >= 10.0 {
        format!("{:.0}%", pct.round())
    } else {
        format!("{pct:.1}%")
    }
}

fn draw_card_bar_track(
    rt: &ID2D1DCRenderTarget,
    track_rect: DipRect,
    bar: &BarView,
    theme: TaskbarTheme,
    pal: &HoverPalette,
) -> Result<()> {
    fill_rounded(rt, track_rect, BAR_TRACK_RADIUS_DIP, pal.track_bg)?;

    if let Some(pct) = bar.percent.filter(|p| p.is_finite()) {
        let clamped_pct = pct.clamp(0.0, 100.0);
        let fill_w = (track_rect.w * (clamped_pct as f32 / 100.0)).clamp(0.0, track_rect.w);
        if fill_w > 0.5 {
            let base = pal.severity_color(bar.severity);
            fill_rounded(
                rt,
                DipRect {
                    x: track_rect.x,
                    y: track_rect.y,
                    w: fill_w,
                    h: track_rect.h,
                },
                BAR_TRACK_RADIUS_DIP.min(fill_w * 0.5),
                base,
            )?;

            if let Some(seg) = bar
                .fable_segment_pct
                .filter(|&s| s.is_finite() && s > 0.0 && clamped_pct > 0.0)
            {
                let seg_ratio = (seg / clamped_pct).clamp(0.0, 1.0) as f32;
                let seg_w = (fill_w * seg_ratio).clamp(0.0, fill_w);
                if seg_w > 0.5 {
                    let base_weight = match theme {
                        TaskbarTheme::Dark => 0.55,
                        TaskbarTheme::Light => 0.62,
                    };
                    let lighter = mix_with_white(base, base_weight);
                    fill_rounded(
                        rt,
                        DipRect {
                            x: track_rect.x + fill_w - seg_w,
                            y: track_rect.y,
                            w: seg_w,
                            h: track_rect.h,
                        },
                        BAR_TRACK_RADIUS_DIP.min(seg_w * 0.5),
                        lighter,
                    )?;
                }
            }
        }
    }

    if let Some(pace) = bar
        .pace_pct
        .filter(|&p| p.is_finite() && (0.0..=100.0).contains(&p))
    {
        let pace_center = track_rect.x + track_rect.w * (pace as f32 / 100.0);
        let tick_x = (pace_center - BAR_PACE_TICK_W_DIP * 0.5).clamp(
            track_rect.x,
            track_rect.x + track_rect.w - BAR_PACE_TICK_W_DIP,
        );
        let tick_y = track_rect.y + (track_rect.h - BAR_PACE_TICK_H_DIP) * 0.5;
        fill_rounded(
            rt,
            DipRect {
                x: tick_x,
                y: tick_y,
                w: BAR_PACE_TICK_W_DIP,
                h: BAR_PACE_TICK_H_DIP,
            },
            BAR_PACE_TICK_W_DIP * 0.5,
            pal.pace_tick,
        )?;
    }

    Ok(())
}

impl Drop for TooltipWindow {
    fn drop(&mut self) {
        if !self.hwnd.is_invalid() {
            // SAFETY: destroys our own tooltip HWND when dropped.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_labels_expand_compact_keys() {
        assert_eq!(friendly_bar_label("5h"), "5-hour");
        assert_eq!(friendly_bar_label("Rolling"), "5-hour");
        assert_eq!(friendly_bar_label("Weekly"), "Weekly");
        assert_eq!(friendly_bar_label("Monthly"), "Monthly");
        assert_eq!(friendly_bar_label("1P"), "Cursor models");
        assert_eq!(friendly_bar_label("API"), "API models");
        assert_eq!(friendly_bar_label("Fable"), "Fable");
    }

    #[test]
    fn combine_reset_parts_dedupes_and_formats_cleanly() {
        assert_eq!(
            combine_reset_parts(Some("3h 56m"), Some("Today 1:30 AM")).as_deref(),
            Some("3h 56m · Today 1:30 AM")
        );
        assert_eq!(
            combine_reset_parts(Some("3h 56m"), None).as_deref(),
            Some("3h 56m")
        );
        assert_eq!(
            combine_reset_parts(None, Some("Thu 9:00 PM")).as_deref(),
            Some("Thu 9:00 PM")
        );
        assert_eq!(
            combine_reset_parts(Some("2h"), Some("2h")).as_deref(),
            Some("2h")
        );
        assert_eq!(combine_reset_parts(Some("  "), Some("")), None);
    }

    #[test]
    fn pick_header_subtitle_prefers_distinct_email_and_hides_when_absent() {
        assert_eq!(
            pick_header_subtitle("Claude", Some("a@b.com"), None).as_deref(),
            Some("a@b.com")
        );
        assert_eq!(pick_header_subtitle("Claude", None, None), None);
        assert_eq!(pick_header_subtitle("a@b.com", Some("a@b.com"), None), None);
        assert_eq!(
            pick_header_subtitle("Cursor", None, Some("re-auth")).as_deref(),
            Some("re-auth")
        );
    }
}
