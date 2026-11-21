//! Direct2D + DirectWrite premultiplied-alpha DIB renderer for the UsageDeck
//! taskbar strip (Variant A) and anchored Windows 11 flyout panel.
//!
//! Renders to an in-memory 32-bit BGRA DIB section and commits atomically via
//! `UpdateLayeredWindow` (`ULW_ALPHA`), giving crisp rounded corners, transparent
//! integration with Windows 11's Mica/Acrylic taskbar, and ~10-15 MB idle RAM.

use crate::layout::{
    DipRect, FlyoutAction, FlyoutLayout, PillSlot, StripLayout, StripMode, FLYOUT_CARD_PAD_DIP,
    FLYOUT_HEADER_HEIGHT_DIP, FLYOUT_PAD_DIP,
};
use crate::taskbar::TaskbarTheme;
use usagedeck_core::view::{BarSeverity, BarView, ProviderItem, ProviderKind, TaskbarSnapshot};
use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
    DWRITE_MEASURING_MODE_NATURAL,
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
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

pub struct Renderer {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    fmt_icon: IDWriteTextFormat,
    fmt_pct: IDWriteTextFormat,
    fmt_rate: IDWriteTextFormat,
    fmt_sub: IDWriteTextFormat,
    fmt_title: IDWriteTextFormat,
    fmt_btn: IDWriteTextFormat,
}

#[derive(Clone, Copy)]
struct Palette {
    pill_bg: D2D1_COLOR_F,
    pill_bg_hover: D2D1_COLOR_F,
    pill_border: D2D1_COLOR_F,
    pill_border_hover: D2D1_COLOR_F,
    pill_border_danger: D2D1_COLOR_F,
    pill_border_warn: D2D1_COLOR_F,
    flyout_bg: D2D1_COLOR_F,
    flyout_border: D2D1_COLOR_F,
    card_bg: D2D1_COLOR_F,
    card_border: D2D1_COLOR_F,
    btn_bg: D2D1_COLOR_F,
    btn_bg_hover: D2D1_COLOR_F,
    track: D2D1_COLOR_F,
    sep: D2D1_COLOR_F,
    text: D2D1_COLOR_F,
    muted: D2D1_COLOR_F,
    ok: D2D1_COLOR_F,
    warn: D2D1_COLOR_F,
    danger: D2D1_COLOR_F,
    bar_muted: D2D1_COLOR_F,
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a,
    }
}

fn palette(theme: TaskbarTheme) -> Palette {
    match theme {
        TaskbarTheme::Dark => Palette {
            pill_bg: rgba(255, 255, 255, 0.065),
            pill_bg_hover: rgba(255, 255, 255, 0.145),
            pill_border: rgba(255, 255, 255, 0.11),
            pill_border_hover: rgba(255, 255, 255, 0.28),
            pill_border_danger: rgba(239, 68, 68, 0.65),
            pill_border_warn: rgba(251, 191, 36, 0.55),
            flyout_bg: rgba(30, 29, 31, 0.96),
            flyout_border: rgba(255, 255, 255, 0.13),
            card_bg: rgba(255, 255, 255, 0.055),
            card_border: rgba(255, 255, 255, 0.09),
            btn_bg: rgba(255, 255, 255, 0.08),
            btn_bg_hover: rgba(255, 255, 255, 0.16),
            track: rgba(255, 255, 255, 0.16),
            sep: rgba(255, 255, 255, 0.14),
            text: rgba(244, 244, 245, 0.97),
            muted: rgba(212, 212, 216, 0.68),
            ok: rgba(52, 211, 153, 1.0),
            warn: rgba(251, 191, 36, 1.0),
            danger: rgba(239, 68, 68, 1.0),
            bar_muted: rgba(161, 161, 170, 0.60),
        },
        TaskbarTheme::Light => Palette {
            pill_bg: rgba(255, 255, 255, 0.68),
            pill_bg_hover: rgba(255, 255, 255, 0.95),
            pill_border: rgba(0, 0, 0, 0.10),
            pill_border_hover: rgba(0, 0, 0, 0.24),
            pill_border_danger: rgba(220, 38, 38, 0.65),
            pill_border_warn: rgba(217, 119, 6, 0.55),
            flyout_bg: rgba(247, 248, 250, 0.97),
            flyout_border: rgba(0, 0, 0, 0.12),
            card_bg: rgba(255, 255, 255, 0.88),
            card_border: rgba(0, 0, 0, 0.08),
            btn_bg: rgba(0, 0, 0, 0.06),
            btn_bg_hover: rgba(0, 0, 0, 0.12),
            track: rgba(0, 0, 0, 0.12),
            sep: rgba(0, 0, 0, 0.14),
            text: rgba(24, 24, 27, 0.96),
            muted: rgba(63, 63, 70, 0.72),
            ok: rgba(16, 163, 108, 1.0),
            warn: rgba(217, 119, 6, 1.0),
            danger: rgba(220, 38, 38, 1.0),
            bar_muted: rgba(113, 113, 122, 0.65),
        },
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
    }
}

fn bar_color(sev: BarSeverity, pal: &Palette) -> D2D1_COLOR_F {
    match sev {
        BarSeverity::Ok => pal.ok,
        BarSeverity::Warn => pal.warn,
        BarSeverity::Danger => pal.danger,
        BarSeverity::Muted => pal.bar_muted,
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

fn rounded(r: DipRect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: to_d2d_rect(r),
        radiusX: radius,
        radiusY: radius,
    }
}

fn make_text_format(
    dwrite: &IDWriteFactory,
    family: windows::core::PCWSTR,
    weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
    size: f32,
    align: windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT,
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
        fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(fmt)
    }
}

impl Renderer {
    pub fn new() -> Result<Self> {
        // SAFETY: creating single-threaded Direct2D and shared DirectWrite factories.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let fmt_icon = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_BOLD,
                10.5,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            let fmt_pct = make_text_format(
                &dwrite,
                w!("Consolas"),
                DWRITE_FONT_WEIGHT_MEDIUM,
                10.0,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
            )?;
            let fmt_rate = make_text_format(
                &dwrite,
                w!("Consolas"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                10.5,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
            )?;
            let fmt_sub = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_NORMAL,
                9.5,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
            )?;
            let fmt_title = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                12.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
            )?;
            let fmt_btn = make_text_format(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_MEDIUM,
                10.5,
                DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;

            Ok(Self {
                d2d,
                dwrite,
                fmt_icon,
                fmt_pct,
                fmt_rate,
                fmt_sub,
                fmt_title,
                fmt_btn,
            })
        }
    }

    fn draw_text(
        &self,
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

    fn draw_text_left(
        &self,
        rt: &ID2D1DCRenderTarget,
        text: &str,
        size: f32,
        semi_bold: bool,
        rect: DipRect,
        color: D2D1_COLOR_F,
    ) -> Result<()> {
        let fmt = make_text_format(
            &self.dwrite,
            w!("Segoe UI"),
            if semi_bold {
                DWRITE_FONT_WEIGHT_SEMI_BOLD
            } else {
                DWRITE_FONT_WEIGHT_NORMAL
            },
            size,
            DWRITE_TEXT_ALIGNMENT_LEADING,
        )?;
        self.draw_text(rt, text, &fmt, rect, color)
    }

    fn fill_rounded(
        &self,
        rt: &ID2D1DCRenderTarget,
        rect: DipRect,
        radius: f32,
        color: D2D1_COLOR_F,
    ) -> Result<()> {
        // SAFETY: `rt` is inside an active BeginDraw/EndDraw pair.
        unsafe {
            let brush = rt.CreateSolidColorBrush(&color, None)?;
            rt.FillRoundedRectangle(&rounded(rect, radius), &brush);
        }
        Ok(())
    }

    fn stroke_rounded(
        &self,
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
            rt.DrawRoundedRectangle(&rounded(rect, radius), &brush, stroke_width, no_style);
        }
        Ok(())
    }

    fn commit_layered(
        &self,
        hwnd: HWND,
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
                    biHeight: -height_px, // top-down DIB
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

            let render_result = (|| -> Result<()> {
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
                    hwnd,
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

            render_result
        }
    }

    /// Paint the Variant A taskbar strip window at physical position `(x, y)`.
    ///
    /// When `hovered_pill` is `Some(item_index)`, only that provider pill receives the hover
    /// highlight so hovering individual pills matches the per-pill tooltip cleanly.
    #[allow(clippy::too_many_arguments)]
    pub fn render_strip(
        &self,
        hwnd: HWND,
        x: i32,
        y: i32,
        dpi: u32,
        theme: TaskbarTheme,
        snapshot: &TaskbarSnapshot,
        layout: &StripLayout,
        hovered: bool,
        hovered_pill: Option<usize>,
    ) -> Result<()> {
        let scale = dpi as f32 / 96.0;
        let width_px = (layout.width_dip * scale).ceil() as i32;
        let height_px = (layout.height_dip * scale).ceil() as i32;
        let pal = palette(theme);

        self.commit_layered(hwnd, (x, y), (width_px, height_px), dpi, |rt| {
            // Subtle near-invisible full-strip hit backdrop so clicks between pills still open flyout.
            self.fill_rounded(
                rt,
                DipRect {
                    x: 0.0,
                    y: 0.0,
                    w: layout.width_dip,
                    h: layout.height_dip,
                },
                6.0,
                rgba(0, 0, 0, 0.01),
            )?;

            match &layout.mode {
                StripMode::Hidden => {}
                StripMode::Empty { rect, label } => {
                    self.fill_rounded(
                        rt,
                        *rect,
                        6.0,
                        if hovered {
                            pal.pill_bg_hover
                        } else {
                            pal.pill_bg
                        },
                    )?;
                    self.stroke_rounded(
                        rt,
                        *rect,
                        6.0,
                        1.0,
                        if hovered {
                            pal.pill_border_hover
                        } else {
                            pal.pill_border
                        },
                    )?;
                    self.draw_text(rt, label, &self.fmt_btn, *rect, pal.muted)?;
                }
                StripMode::CompactSummary {
                    rect,
                    peak_percent,
                    severity,
                } => {
                    let border = match severity {
                        BarSeverity::Danger => pal.pill_border_danger,
                        BarSeverity::Warn => pal.pill_border_warn,
                        _ if hovered => pal.pill_border_hover,
                        _ => pal.pill_border,
                    };
                    self.fill_rounded(
                        rt,
                        *rect,
                        6.0,
                        if hovered {
                            pal.pill_bg_hover
                        } else {
                            pal.pill_bg
                        },
                    )?;
                    self.stroke_rounded(rt, *rect, 6.0, 1.0, border)?;
                    let dot = DipRect {
                        x: rect.x + 8.0,
                        y: rect.y + (rect.h - 8.0) * 0.5,
                        w: 8.0,
                        h: 8.0,
                    };
                    self.fill_rounded(rt, dot, 4.0, bar_color(*severity, &pal))?;
                    let pct_txt = peak_percent
                        .map(|p| format!("{:.0}%", p.round()))
                        .unwrap_or_else(|| "--".to_string());
                    self.draw_text(
                        rt,
                        &pct_txt,
                        &self.fmt_rate,
                        DipRect {
                            x: rect.x + 18.0,
                            y: rect.y,
                            w: rect.w - 25.0,
                            h: rect.h,
                        },
                        pal.text,
                    )?;
                }
                StripMode::Pills {
                    slots,
                    overflow_badge,
                } => {
                    for slot in slots {
                        if let Some(item) = snapshot.items.get(slot.item_index) {
                            let pill_hovered = hovered_pill == Some(slot.item_index);
                            self.draw_provider_pill(rt, item, slot, theme, &pal, pill_hovered)?;
                        }
                    }
                    if let Some((rect, count)) = overflow_badge {
                        let badge_hovered = hovered && hovered_pill.is_none();
                        self.fill_rounded(
                            rt,
                            *rect,
                            6.0,
                            if badge_hovered {
                                pal.pill_bg_hover
                            } else {
                                pal.pill_bg
                            },
                        )?;
                        self.stroke_rounded(
                            rt,
                            *rect,
                            6.0,
                            1.0,
                            if badge_hovered {
                                pal.pill_border_hover
                            } else {
                                pal.pill_border
                            },
                        )?;
                        let label = format!("+{count}");
                        self.draw_text(rt, &label, &self.fmt_btn, *rect, pal.muted)?;
                    }
                }
            }
            Ok(())
        })
    }

    fn draw_provider_pill(
        &self,
        rt: &ID2D1DCRenderTarget,
        item: &ProviderItem,
        slot: &PillSlot,
        theme: TaskbarTheme,
        pal: &Palette,
        hovered: bool,
    ) -> Result<()> {
        let r = slot.rect;
        let bg = if hovered {
            pal.pill_bg_hover
        } else {
            pal.pill_bg
        };
        let border = if item.exhausted {
            pal.pill_border_danger
        } else if item.auth_expired {
            pal.pill_border_warn
        } else if hovered {
            pal.pill_border_hover
        } else {
            pal.pill_border
        };

        self.fill_rounded(rt, r, 6.0, bg)?;
        self.stroke_rounded(rt, r, 6.0, 1.0, border)?;

        // Provider icon badge (16x16 DIPs).
        let (glyph, badge_bg, badge_fg) = provider_badge_style(item.provider, theme);
        let badge_rect = DipRect {
            x: r.x + 7.0,
            y: r.y + (r.h - 16.0) * 0.5,
            w: 16.0,
            h: 16.0,
        };
        self.fill_rounded(rt, badge_rect, 4.0, badge_bg)?;
        self.draw_text(rt, glyph, &self.fmt_icon, badge_rect, badge_fg)?;

        // Twin slim stacked bars + right-aligned percentage.
        let bars_x = badge_rect.x + badge_rect.w + 6.0;
        let bar_w = 34.0;
        let pct_w = 24.0;
        let bar_h = 4.0;

        let draw_bar_row = |bar: &BarView, row_y: f32| -> Result<()> {
            let track_rect = DipRect {
                x: bars_x,
                y: row_y + 3.5,
                w: bar_w,
                h: bar_h,
            };
            self.fill_rounded(rt, track_rect, 2.0, pal.track)?;
            if let Some(p) = bar.percent {
                let fill_w = (bar_w * (p as f32 / 100.0)).clamp(0.0, bar_w);
                if fill_w > 0.5 {
                    self.fill_rounded(
                        rt,
                        DipRect {
                            x: track_rect.x,
                            y: track_rect.y,
                            w: fill_w,
                            h: bar_h,
                        },
                        2.0,
                        bar_color(bar.severity, pal),
                    )?;
                }
            }
            let pct_color = match bar.severity {
                BarSeverity::Danger => pal.danger,
                BarSeverity::Warn => pal.warn,
                BarSeverity::Muted => pal.muted,
                BarSeverity::Ok => pal.text,
            };
            self.draw_text(
                rt,
                &bar.percent_label(),
                &self.fmt_pct,
                DipRect {
                    x: bars_x + bar_w + 2.0,
                    y: row_y - 1.0,
                    w: pct_w,
                    h: 12.0,
                },
                pct_color,
            )
        };

        if item.taskbar_bars.len() == 1 {
            draw_bar_row(&item.taskbar_bars[0], r.y + (r.h - 11.0) * 0.5)?;
        } else {
            if let Some(b0) = item.taskbar_bars.first() {
                draw_bar_row(b0, r.y + 4.5)?;
            }
            if let Some(b1) = item.taskbar_bars.get(1) {
                draw_bar_row(b1, r.y + 16.5)?;
            }
        }

        // Extended column for Claude: hairline divider + live `$/h` rate + compact reset time.
        if slot.extended {
            let sep_x = bars_x + bar_w + pct_w + 7.0;
            self.fill_rounded(
                rt,
                DipRect {
                    x: sep_x,
                    y: r.y + 6.0,
                    w: 1.0,
                    h: (r.h - 12.0).max(2.0),
                },
                0.5,
                pal.sep,
            )?;

            let col_x = sep_x + 4.0;
            let col_w = (r.x + r.w - 6.0 - col_x).max(20.0);
            if let Some(ref rate) = item.spend_rate_text {
                self.draw_text(
                    rt,
                    rate,
                    &self.fmt_rate,
                    DipRect {
                        x: col_x,
                        y: r.y + 3.5,
                        w: col_w,
                        h: 12.5,
                    },
                    pal.ok,
                )?;
            }
            if let Some(ref reset) = item.reset_short {
                let top = if item.spend_rate_text.is_some() {
                    r.y + 16.5
                } else {
                    r.y + (r.h - 12.0) * 0.5
                };
                self.draw_text(
                    rt,
                    reset,
                    &self.fmt_sub,
                    DipRect {
                        x: col_x,
                        y: top,
                        w: col_w,
                        h: 12.0,
                    },
                    pal.muted,
                )?;
            }
        }

        Ok(())
    }

    /// Paint the anchored Windows 11 Fluent flyout above the taskbar strip.
    #[allow(clippy::too_many_arguments)]
    pub fn render_flyout(
        &self,
        hwnd: HWND,
        x: i32,
        y: i32,
        dpi: u32,
        theme: TaskbarTheme,
        snapshot: &TaskbarSnapshot,
        layout: &FlyoutLayout,
        hovered_action: Option<&FlyoutAction>,
    ) -> Result<()> {
        let scale = dpi as f32 / 96.0;
        let width_px = (layout.width_dip * scale).ceil() as i32;
        let height_px = (layout.height_dip * scale).ceil() as i32;
        let pal = palette(theme);

        self.commit_layered(hwnd, (x, y), (width_px, height_px), dpi, |rt| {
            let outer = DipRect {
                x: 0.5,
                y: 0.5,
                w: layout.width_dip - 1.0,
                h: layout.height_dip - 1.0,
            };
            self.fill_rounded(rt, outer, 12.0, pal.flyout_bg)?;
            self.stroke_rounded(rt, outer, 12.0, 1.0, pal.flyout_border)?;

            // Header title
            let title = if snapshot.is_demo {
                "UsageDeck · Demo"
            } else {
                "UsageDeck"
            };
            let title_x = FLYOUT_PAD_DIP + 2.0;
            let title_w = layout
                .header_buttons
                .first()
                .map_or(140.0, |b| (b.rect.x - title_x - 6.0).max(48.0));
            self.draw_text(
                rt,
                title,
                &self.fmt_title,
                DipRect {
                    x: title_x,
                    y: FLYOUT_PAD_DIP,
                    w: title_w,
                    h: FLYOUT_HEADER_HEIGHT_DIP,
                },
                pal.text,
            )?;

            // Header action buttons (Refresh, Settings, Data, Quit)
            for btn in &layout.header_buttons {
                let is_hovered = hovered_action == Some(&btn.action);
                let bg = if is_hovered {
                    pal.btn_bg_hover
                } else {
                    pal.btn_bg
                };
                self.fill_rounded(rt, btn.rect, 5.0, bg)?;
                if is_hovered {
                    self.stroke_rounded(rt, btn.rect, 5.0, 1.0, pal.pill_border_hover)?;
                }
                self.draw_text(rt, btn.label, &self.fmt_btn, btn.rect, pal.text)?;
            }

            if snapshot.items.is_empty() {
                let box_rect = DipRect {
                    x: FLYOUT_PAD_DIP,
                    y: FLYOUT_PAD_DIP + FLYOUT_HEADER_HEIGHT_DIP + 8.0,
                    w: layout.width_dip - FLYOUT_PAD_DIP * 2.0,
                    h: 60.0,
                };
                self.fill_rounded(rt, box_rect, 8.0, pal.card_bg)?;
                self.stroke_rounded(rt, box_rect, 8.0, 1.0, pal.card_border)?;
                self.draw_text(
                    rt,
                    "No signed-in providers found. Open Settings to add keys.",
                    &self.fmt_btn,
                    box_rect,
                    pal.muted,
                )?;
                return Ok(());
            }

            // Provider cards (full bars + resets + spend details + per-card quick actions)
            for card in &layout.cards {
                let Some(item) = snapshot.items.get(card.item_index) else {
                    continue;
                };
                let r = card.rect;
                let border = if item.exhausted {
                    pal.pill_border_danger
                } else if item.auth_expired {
                    pal.pill_border_warn
                } else {
                    pal.card_border
                };
                self.fill_rounded(rt, r, 8.0, pal.card_bg)?;
                self.stroke_rounded(rt, r, 8.0, 1.0, border)?;

                let (glyph, badge_bg, badge_fg) = provider_badge_style(item.provider, theme);
                let badge = DipRect {
                    x: r.x + FLYOUT_CARD_PAD_DIP,
                    y: r.y + FLYOUT_CARD_PAD_DIP + 1.0,
                    w: 16.0,
                    h: 16.0,
                };
                self.fill_rounded(rt, badge, 4.0, badge_bg)?;
                self.draw_text(rt, glyph, &self.fmt_icon, badge, badge_fg)?;

                // Render right-aligned quick-action chips (Open Cursor, Copy key, Hide).
                for btn in &card.buttons {
                    let is_hovered = hovered_action == Some(&btn.action);
                    let bg = if is_hovered {
                        pal.btn_bg_hover
                    } else {
                        pal.btn_bg
                    };
                    let (chip_border, chip_fg) = match &btn.action {
                        FlyoutAction::OpenCursor => (pal.pill_border_warn, pal.warn),
                        FlyoutAction::HideUntilReset { .. } => (pal.pill_border_danger, pal.danger),
                        FlyoutAction::CopyOpenCodeKey(_) => (
                            if is_hovered {
                                pal.pill_border_hover
                            } else {
                                pal.pill_border
                            },
                            pal.text,
                        ),
                        _ => (pal.pill_border, pal.text),
                    };
                    self.fill_rounded(rt, btn.rect, 4.0, bg)?;
                    self.stroke_rounded(rt, btn.rect, 4.0, 1.0, chip_border)?;
                    self.draw_text(rt, btn.label, &self.fmt_btn, btn.rect, chip_fg)?;
                }

                // Provider title + inline spend rate or subtitle, bounded cleanly left of chips/reset.
                let title_x = badge.x + badge.w + 6.0;
                let title_w = match item.provider {
                    ProviderKind::OpenCode | ProviderKind::Grok => 62.0,
                    _ => 50.0,
                };
                self.draw_text_left(
                    rt,
                    &item.title,
                    11.5,
                    true,
                    DipRect {
                        x: title_x,
                        y: r.y + FLYOUT_CARD_PAD_DIP,
                        w: title_w,
                        h: 18.0,
                    },
                    pal.text,
                )?;

                // Optional reset summary placed left of any chips (or right-aligned if no chips).
                let mut right_bound_x = card.right_content_x;
                if let Some(ref rs) = item.reset_short {
                    if card.buttons.is_empty() {
                        let reset_w = 84.0;
                        let reset_x = right_bound_x - reset_w;
                        self.draw_text(
                            rt,
                            &format!("Resets {rs}"),
                            &self.fmt_sub,
                            DipRect {
                                x: reset_x,
                                y: r.y + FLYOUT_CARD_PAD_DIP,
                                w: reset_w,
                                h: 18.0,
                            },
                            pal.muted,
                        )?;
                        right_bound_x = reset_x - 4.0;
                    } else if item.subtitle.is_none() && item.spend_rate_text.is_none() {
                        let reset_w = 54.0;
                        let reset_x = right_bound_x - 6.0 - reset_w;
                        if reset_x > title_x + title_w + 4.0 {
                            self.draw_text(
                                rt,
                                rs,
                                &self.fmt_sub,
                                DipRect {
                                    x: reset_x,
                                    y: r.y + FLYOUT_CARD_PAD_DIP,
                                    w: reset_w,
                                    h: 18.0,
                                },
                                pal.muted,
                            )?;
                            right_bound_x = reset_x - 4.0;
                        }
                    }
                }

                let meta_x = title_x + title_w + 2.0;
                let meta_w = (right_bound_x - 6.0 - meta_x).max(0.0);
                if meta_w >= 28.0 {
                    if let Some(ref rate) = item.spend_rate_text {
                        self.draw_text_left(
                            rt,
                            rate,
                            11.0,
                            true,
                            DipRect {
                                x: meta_x,
                                y: r.y + FLYOUT_CARD_PAD_DIP,
                                w: meta_w,
                                h: 18.0,
                            },
                            pal.ok,
                        )?;
                    } else if let Some(ref sub) = item.subtitle {
                        if !item.can_open_cursor {
                            self.draw_text_left(
                                rt,
                                sub,
                                10.0,
                                false,
                                DipRect {
                                    x: meta_x,
                                    y: r.y + FLYOUT_CARD_PAD_DIP,
                                    w: meta_w,
                                    h: 18.0,
                                },
                                if item.auth_expired {
                                    pal.warn
                                } else {
                                    pal.muted
                                },
                            )?;
                        }
                    }
                }

                // Bar columns (1..=3 equal-width columns across the card)
                let count = item.flyout_bars.len().max(1);
                let grid_x = r.x + FLYOUT_CARD_PAD_DIP;
                let grid_y = r.y + FLYOUT_CARD_PAD_DIP + 24.0;
                let grid_w = r.w - FLYOUT_CARD_PAD_DIP * 2.0;
                let col_gap = 8.0;
                let col_w = (grid_w - col_gap * (count as f32 - 1.0)) / count as f32;

                for (i, bar) in item.flyout_bars.iter().enumerate() {
                    let cx = grid_x + i as f32 * (col_w + col_gap);
                    self.draw_text_left(
                        rt,
                        &bar.label,
                        10.0,
                        false,
                        DipRect {
                            x: cx,
                            y: grid_y,
                            w: col_w * 0.65,
                            h: 14.0,
                        },
                        pal.muted,
                    )?;
                    let pct_color = match bar.severity {
                        BarSeverity::Danger => pal.danger,
                        BarSeverity::Warn => pal.warn,
                        BarSeverity::Muted => pal.muted,
                        BarSeverity::Ok => pal.text,
                    };
                    self.draw_text(
                        rt,
                        &bar.percent_label(),
                        &self.fmt_pct,
                        DipRect {
                            x: cx + col_w * 0.5,
                            y: grid_y,
                            w: col_w * 0.5,
                            h: 14.0,
                        },
                        pct_color,
                    )?;

                    let bar_rect = DipRect {
                        x: cx,
                        y: grid_y + 17.0,
                        w: col_w,
                        h: 5.0,
                    };
                    self.fill_rounded(rt, bar_rect, 2.5, pal.track)?;
                    if let Some(p) = bar.percent {
                        let fill_w = (col_w * (p as f32 / 100.0)).clamp(0.0, col_w);
                        if fill_w > 0.5 {
                            self.fill_rounded(
                                rt,
                                DipRect {
                                    x: bar_rect.x,
                                    y: bar_rect.y,
                                    w: fill_w,
                                    h: 5.0,
                                },
                                2.5,
                                bar_color(bar.severity, &pal),
                            )?;
                        }
                    }
                }
            }

            Ok(())
        })
    }
}
