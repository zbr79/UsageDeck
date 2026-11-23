//! Double-buffered Direct2D / DirectWrite renderer for the native UsageDeck
//! Settings window.
//!
//! Renders to an off-screen 32-bit premultiplied BGRA DIB section bound to an
//! `ID2D1DCRenderTarget`, rasterizes smooth antialiased gradient area + polyline
//! curves for Claude's Spend & Pace chart directly onto the DIB pixels, and
//! blits atomically via `BitBlt(..., SRCCOPY)` inside `BeginPaint`/`EndPaint`
//! for zero flicker at any DPI.

use crate::icons::IconAtlas;
use crate::settings_dialog::layout::{
    AccountRowRender, Action, ButtonStyle, ContentItem, DipRect, SettingsLayout, SpendChartRender,
    SIDEBAR_W_DIP,
};
use crate::settings_dialog::theme::{rgba, with_alpha, BadgeColors, SettingsPalette};
use usagedeck_core::store::TaskbarMeterStyle;
use usagedeck_core::view::{BarSeverity, ProviderKind};
use windows::core::{w, Result};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1StrokeStyle,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EndPaint,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ, PAINTSTRUCT,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

struct ChartDrawJob {
    plot_rect_client: DipRect,
    clip_rect_client: DipRect,
    points: Vec<(f64, f64)>,
    all_rgb: (u8, u8, u8),
    fable_rgb: (u8, u8, u8),
}

pub struct SettingsRenderer {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    icons: IconAtlas,
    fmt_page_title: IDWriteTextFormat,
    fmt_section: IDWriteTextFormat,
    fmt_nav: IDWriteTextFormat,
    fmt_nav_active: IDWriteTextFormat,
    fmt_nav_group: IDWriteTextFormat,
    fmt_btn: IDWriteTextFormat,
    fmt_btn_sm: IDWriteTextFormat,
    fmt_badge: IDWriteTextFormat,
    fmt_table_hdr: IDWriteTextFormat,
    fmt_row_name: IDWriteTextFormat,
    fmt_cell_pct: IDWriteTextFormat,
    fmt_cell_reset: IDWriteTextFormat,
    fmt_toast: IDWriteTextFormat,
}

impl SettingsRenderer {
    pub fn new() -> Result<Self> {
        // SAFETY: creating single-threaded Direct2D and shared DirectWrite factories on UI thread.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let icons = IconAtlas::new();

            let fmt_page_title = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                26.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;
            let fmt_section = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                14.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;
            let fmt_nav = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_NORMAL,
                13.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;
            let fmt_nav_active = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                13.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;
            let fmt_nav_group = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                12.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;
            let fmt_btn = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_MEDIUM,
                13.0,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
                false,
            )?;
            let fmt_btn_sm = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_MEDIUM,
                12.0,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
                false,
            )?;
            let fmt_badge = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                11.0,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
                false,
            )?;
            let fmt_table_hdr = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_NORMAL,
                12.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;
            let fmt_row_name = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                13.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;
            let fmt_cell_pct = make_fmt(
                &dwrite,
                w!("Consolas"),
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                12.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;
            let fmt_cell_reset = make_fmt(
                &dwrite,
                w!("Consolas"),
                DWRITE_FONT_WEIGHT_NORMAL,
                11.5,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                false,
                false,
            )?;
            let fmt_toast = make_fmt(
                &dwrite,
                w!("Segoe UI"),
                DWRITE_FONT_WEIGHT_MEDIUM,
                12.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
                false,
            )?;

            Ok(Self {
                d2d,
                dwrite,
                icons,
                fmt_page_title,
                fmt_section,
                fmt_nav,
                fmt_nav_active,
                fmt_nav_group,
                fmt_btn,
                fmt_btn_sm,
                fmt_badge,
                fmt_table_hdr,
                fmt_row_name,
                fmt_cell_pct,
                fmt_cell_reset,
                fmt_toast,
            })
        }
    }

    pub fn paint_window(
        &self,
        hwnd: HWND,
        dpi: u32,
        pal: &SettingsPalette,
        layout: &SettingsLayout,
        hovered: Option<&Action>,
        scrollbar_hovered: bool,
    ) {
        // SAFETY: standard Win32 BeginPaint / off-screen DIB + Direct2D / BitBlt / EndPaint.
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let paint_dc = BeginPaint(hwnd, &mut ps);
            if paint_dc.is_invalid() {
                return;
            }

            let mut rc = RECT::default();
            if GetClientRect(hwnd, &mut rc).is_ok() {
                let width_px = (rc.right - rc.left).max(0);
                let height_px = (rc.bottom - rc.top).max(0);
                if width_px > 0 && height_px > 0 {
                    let _ = self.paint_to_dc(
                        paint_dc,
                        width_px,
                        height_px,
                        dpi,
                        pal,
                        layout,
                        hovered,
                        scrollbar_hovered,
                    );
                }
            }

            let _ = EndPaint(hwnd, &ps);
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn paint_to_dc(
        &self,
        paint_dc: windows::Win32::Graphics::Gdi::HDC,
        width_px: i32,
        height_px: i32,
        dpi: u32,
        pal: &SettingsPalette,
        layout: &SettingsLayout,
        hovered: Option<&Action>,
        scrollbar_hovered: bool,
    ) -> Result<()> {
        let mem_dc = CreateCompatibleDC(Some(paint_dc));
        if mem_dc.is_invalid() {
            return Ok(());
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width_px,
                biHeight: -height_px, // top-down 32bpp BGRA
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap =
            match CreateDIBSection(Some(paint_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bmp) => bmp,
                Err(e) => {
                    let _ = DeleteDC(mem_dc);
                    return Err(e);
                }
            };
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

        let mut chart_jobs: Vec<ChartDrawJob> = Vec::new();
        let draw_result = (|| -> Result<()> {
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
            rt.Clear(Some(&pal.page));

            let body_res = self.draw_scene(
                &rt,
                pal,
                layout,
                hovered,
                scrollbar_hovered,
                &mut chart_jobs,
            );
            let end_res = rt.EndDraw(None, None);
            body_res?;
            end_res?;

            // Rasterize smooth antialiased spend chart curves directly into the BGRA DIB.
            if !bits.is_null() && !chart_jobs.is_empty() {
                let scale = dpi as f32 / 96.0;
                let byte_len = (width_px as usize)
                    .saturating_mul(height_px as usize)
                    .saturating_mul(4);
                let pixel_slice = std::slice::from_raw_parts_mut(bits.cast::<u8>(), byte_len);
                let toast_rect = layout.toast.as_ref().map(|(r, _)| *r);
                for job in &chart_jobs {
                    rasterize_spend_chart(
                        pixel_slice,
                        width_px as usize,
                        height_px as usize,
                        scale,
                        job,
                        toast_rect,
                    );
                }
            }

            // Re-draw toast on top if needed (already painted in Direct2D above charts because
            // charts are inside content pane and toast is bottom-right; re-checking overlap).
            let _ = BitBlt(
                paint_dc,
                0,
                0,
                width_px,
                height_px,
                Some(mem_dc),
                0,
                0,
                SRCCOPY,
            );
            Ok(())
        })();

        SelectObject(mem_dc, old_obj);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem_dc);
        draw_result
    }

    fn draw_scene(
        &self,
        rt: &ID2D1DCRenderTarget,
        pal: &SettingsPalette,
        layout: &SettingsLayout,
        hovered: Option<&Action>,
        scrollbar_hovered: bool,
        chart_jobs: &mut Vec<ChartDrawJob>,
    ) -> Result<()> {
        // ---- Left Sidebar ----
        self.fill_rect(
            rt,
            DipRect::new(0.0, 0.0, SIDEBAR_W_DIP, layout.client_h),
            pal.page,
        )?;
        self.fill_rect(
            rt,
            DipRect::new(SIDEBAR_W_DIP - 1.0, 0.0, 1.0, layout.client_h),
            pal.card_border,
        )?;

        for row in &layout.nav_rows {
            let is_hover = hovered == Some(&row.action);
            if row.selected {
                self.fill_rounded(rt, row.rect, 4.0, pal.nav_active)?;
                // 3x16 emerald vertical active pill on left edge.
                let pill = DipRect::new(
                    row.rect.x,
                    row.rect.y + (row.rect.h - 16.0) * 0.5,
                    3.0,
                    16.0,
                );
                self.fill_rounded(rt, pill, 1.5, pal.accent)?;
            } else if is_hover {
                self.fill_rounded(rt, row.rect, 4.0, pal.nav_hover)?;
            }

            let mut text_x = row.rect.x + 12.0;
            if let Some(kind) = row.provider_icon {
                let icon_rect = DipRect::new(
                    row.rect.x + 12.0,
                    row.rect.y + (row.rect.h - 16.0) * 0.5,
                    16.0,
                    16.0,
                );
                self.draw_provider_icon(rt, kind, icon_rect, pal);
                text_x = icon_rect.x + icon_rect.w + 10.0;
            }

            let right_pad = if row.show_dot { 24.0 } else { 10.0 };
            let text_rect = DipRect::new(
                text_x,
                row.rect.y,
                (row.rect.x + row.rect.w - text_x - right_pad).max(20.0),
                row.rect.h,
            );
            let fmt = if row.selected {
                &self.fmt_nav_active
            } else {
                &self.fmt_nav
            };
            self.draw_text(rt, row.label, fmt, text_rect, pal.text)?;

            if row.show_dot {
                let dot = DipRect::new(
                    row.rect.x + row.rect.w - 18.0,
                    row.rect.y + (row.rect.h - 8.0) * 0.5,
                    8.0,
                    8.0,
                );
                self.fill_rounded(rt, dot, 4.0, pal.accent)?;
            }
        }

        self.draw_text(
            rt,
            "Accounts",
            &self.fmt_nav_group,
            layout.nav_accounts_label,
            pal.muted,
        )?;

        if let Some((ref rect, ref label)) = layout.nav_update_btn {
            let is_hover = hovered == Some(&Action::CheckUpdates);
            self.fill_rounded(
                rt,
                *rect,
                4.0,
                if is_hover {
                    pal.nav_active
                } else {
                    pal.nav_hover
                },
            )?;
            self.stroke_rounded(rt, *rect, 4.0, 1.0, with_alpha(pal.accent, 0.45))?;
            let arrow_rect = DipRect::new(rect.x + 10.0, rect.y, 16.0, rect.h);
            self.draw_text(rt, "↓", &self.fmt_nav_active, arrow_rect, pal.accent)?;
            let lbl_rect = DipRect::new(rect.x + 28.0, rect.y, (rect.w - 36.0).max(20.0), rect.h);
            self.draw_text(rt, label, &self.fmt_nav_active, lbl_rect, pal.text)?;
        }

        // ---- Right Content Pane (clipped & vertically scrolled) ----
        // SAFETY: PushAxisAlignedClip is balanced by PopAxisAlignedClip below.
        unsafe {
            let clip = layout.content_viewport.to_d2d();
            rt.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        let scroll = layout.scroll_y;
        let vp_top = layout.content_viewport.y;
        let vp_bot = layout.content_viewport.y + layout.content_viewport.h;

        let mut content_res: Result<()> = Ok(());
        for item in &layout.content_items {
            if let Err(e) = self.draw_content_item(
                rt,
                pal,
                item,
                scroll,
                vp_top,
                vp_bot,
                hovered,
                layout.content_viewport,
                chart_jobs,
            ) {
                content_res = Err(e);
                break;
            }
        }

        // SAFETY: matches PushAxisAlignedClip above.
        unsafe {
            rt.PopAxisAlignedClip();
        }
        content_res?;

        // ---- Custom Scrollbar ----
        if let Some(ref sb) = layout.scrollbar {
            let color = if scrollbar_hovered {
                pal.scrollbar_thumb_hover
            } else {
                pal.scrollbar_thumb
            };
            self.fill_rounded(rt, sb.thumb, 3.0, color)?;
        }

        // ---- Bottom-right inline toast ----
        if let Some((ref rect, ref msg)) = layout.toast {
            self.fill_rounded(rt, *rect, 6.0, pal.toast_bg)?;
            self.stroke_rounded(rt, *rect, 6.0, 1.0, pal.toast_border)?;
            let dot = DipRect::new(rect.x + 12.0, rect.y + (rect.h - 7.0) * 0.5, 7.0, 7.0);
            self.fill_rounded(rt, dot, 3.5, pal.accent)?;
            let text_rect = DipRect::new(rect.x + 26.0, rect.y, (rect.w - 36.0).max(20.0), rect.h);
            let toast_fg = if pal.is_light {
                rgba(255, 255, 255, 0.96)
            } else {
                pal.text
            };
            self.draw_text(rt, msg, &self.fmt_toast, text_rect, toast_fg)?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_content_item(
        &self,
        rt: &ID2D1DCRenderTarget,
        pal: &SettingsPalette,
        item: &ContentItem,
        scroll: f32,
        vp_top: f32,
        vp_bot: f32,
        hovered: Option<&Action>,
        viewport: DipRect,
        chart_jobs: &mut Vec<ChartDrawJob>,
    ) -> Result<()> {
        let visible = |r: DipRect| r.y + r.h >= vp_top - 4.0 && r.y <= vp_bot + 4.0;

        match item {
            ContentItem::PageHeader {
                rect,
                icon,
                title,
                subtitle,
            } => {
                let r = rect.offset_y(-scroll);
                if !visible(r) {
                    return Ok(());
                }
                let mut tx = r.x;
                if let Some(kind) = *icon {
                    let irect = DipRect::new(r.x, r.y + (r.h - 32.0) * 0.5, 32.0, 32.0);
                    self.draw_provider_icon(rt, kind, irect, pal);
                    tx = irect.x + 44.0;
                }
                let tw = (r.x + r.w - tx - 140.0).max(120.0);
                if let Some(sub) = subtitle {
                    self.draw_text(
                        rt,
                        title,
                        &self.fmt_page_title,
                        DipRect::new(tx, r.y - 2.0, tw, 32.0),
                        pal.text,
                    )?;
                    self.draw_dynamic_text(
                        rt,
                        sub,
                        12.0,
                        false,
                        false,
                        false,
                        false,
                        false,
                        DipRect::new(tx, r.y + 30.0, tw, 18.0),
                        pal.muted,
                    )?;
                } else {
                    self.draw_text(
                        rt,
                        title,
                        &self.fmt_page_title,
                        DipRect::new(tx, r.y, tw, r.h),
                        pal.text,
                    )?;
                }
            }
            ContentItem::SectionHeading { rect, text } => {
                let r = rect.offset_y(-scroll);
                if visible(r) {
                    self.draw_text(rt, text, &self.fmt_section, r, pal.text)?;
                }
            }
            ContentItem::Card { rect } => {
                let r = rect.offset_y(-scroll);
                if visible(r) {
                    self.fill_rounded(rt, r, 4.0, pal.card)?;
                    self.stroke_rounded(rt, r, 4.0, 1.0, pal.card_border)?;
                }
            }
            ContentItem::Divider { rect } => {
                let r = rect.offset_y(-scroll);
                if visible(r) {
                    self.fill_rect(rt, r, pal.card_border)?;
                }
            }
            ContentItem::AppIcon { rect } => {
                let r = rect.offset_y(-scroll);
                if visible(r) && !self.icons.draw_app_icon(rt, r.to_d2d()) {
                    self.fill_rounded(rt, r, 4.0, pal.accent)?;
                    self.draw_text(rt, "U", &self.fmt_badge, r, pal.primary_fg)?;
                }
            }
            ContentItem::ProviderIcon { rect, kind } => {
                let r = rect.offset_y(-scroll);
                if visible(r) {
                    self.draw_provider_icon(rt, *kind, r, pal);
                }
            }
            ContentItem::Text {
                rect,
                text,
                size,
                semi_bold,
                muted,
                warn,
                align_right,
                align_center,
                mono,
                wrap,
            } => {
                let r = rect.offset_y(-scroll);
                if visible(r) {
                    let color = if *warn {
                        pal.warn_text
                    } else if *muted {
                        pal.muted
                    } else {
                        pal.text
                    };
                    self.draw_dynamic_text(
                        rt,
                        text,
                        *size,
                        *semi_bold,
                        *mono,
                        *align_right,
                        *align_center,
                        *wrap,
                        r,
                        color,
                    )?;
                }
            }
            ContentItem::Button {
                rect,
                label,
                style,
                disabled,
                action,
            } => {
                let r = rect.offset_y(-scroll);
                if visible(r) {
                    let is_hover = !disabled && hovered == Some(action);
                    self.draw_button(rt, pal, r, label, *style, *disabled, is_hover)?;
                }
            }
            ContentItem::Toggle {
                rect,
                checked,
                action,
            } => {
                let r = rect.offset_y(-scroll);
                if visible(r) {
                    let is_hover = hovered == Some(action);
                    self.draw_toggle(rt, pal, r, *checked, is_hover)?;
                }
            }
            ContentItem::SegmentedControl { track, options } => {
                let tr = track.offset_y(-scroll);
                if !visible(tr) {
                    return Ok(());
                }
                self.fill_rounded(rt, tr, 6.0, pal.surface)?;
                self.stroke_rounded(rt, tr, 6.0, 1.0, pal.border)?;
                for opt in options {
                    let or = opt.rect.offset_y(-scroll);
                    let is_hover = hovered == Some(&opt.action);
                    if opt.selected {
                        self.fill_rounded(rt, or, 4.0, pal.primary_bg)?;
                        self.draw_text(rt, opt.label, &self.fmt_btn, or, pal.primary_fg)?;
                    } else {
                        if is_hover {
                            self.fill_rounded(rt, or, 4.0, pal.surface_hover)?;
                        }
                        self.draw_text(
                            rt,
                            opt.label,
                            &self.fmt_btn,
                            or,
                            if is_hover { pal.text } else { pal.muted },
                        )?;
                    }
                }
            }
            ContentItem::BarChipRow { chips } => {
                for chip in chips {
                    let r = chip.rect.offset_y(-scroll);
                    if !visible(r) {
                        continue;
                    }
                    let is_hover = !chip.disabled && hovered == Some(&chip.action);
                    let alpha = if chip.disabled { 0.55 } else { 1.0 };
                    let bg = if is_hover {
                        pal.surface_hover
                    } else if chip.checked {
                        pal.surface
                    } else {
                        rgba(0, 0, 0, 0.0)
                    };
                    let border = if chip.checked {
                        with_alpha(pal.border, alpha)
                    } else {
                        with_alpha(pal.card_border, alpha)
                    };
                    if bg.a > 0.0 {
                        self.fill_rounded(rt, r, 14.0, with_alpha(bg, alpha))?;
                    }
                    self.stroke_rounded(rt, r, 14.0, 1.0, border)?;

                    if chip.checked {
                        let chk_rect = DipRect::new(r.x + 8.0, r.y, 14.0, r.h);
                        self.draw_text(
                            rt,
                            "✓",
                            &self.fmt_btn_sm,
                            chk_rect,
                            with_alpha(pal.accent, alpha),
                        )?;
                    }
                    let lbl_rect = DipRect::new(
                        r.x + if chip.checked { 22.0 } else { 10.0 },
                        r.y,
                        (r.w - if chip.checked { 30.0 } else { 20.0 }).max(20.0),
                        r.h,
                    );
                    let fg = if chip.checked {
                        with_alpha(pal.text, alpha)
                    } else {
                        with_alpha(pal.muted, alpha)
                    };
                    self.draw_dynamic_text(
                        rt,
                        &chip.label,
                        12.0,
                        false,
                        false,
                        false,
                        false,
                        false,
                        lbl_rect,
                        fg,
                    )?;
                }
            }
            ContentItem::AccountTable {
                card_rect,
                header_rect,
                account_col_x,
                account_col_w,
                cols,
                extra_header,
                rows,
            } => {
                let cr = card_rect.offset_y(-scroll);
                if !visible(cr) {
                    return Ok(());
                }
                self.fill_rounded(rt, cr, 4.0, pal.card)?;
                self.stroke_rounded(rt, cr, 4.0, 1.0, pal.card_border)?;

                let hr = header_rect.offset_y(-scroll);
                if visible(hr) {
                    self.draw_text(
                        rt,
                        "Account",
                        &self.fmt_table_hdr,
                        DipRect::new(*account_col_x, hr.y, *account_col_w, hr.h),
                        pal.muted,
                    )?;
                    for col in cols {
                        self.draw_text(
                            rt,
                            &col.header,
                            &self.fmt_table_hdr,
                            DipRect::new(col.x, hr.y, col.w, hr.h),
                            pal.muted,
                        )?;
                    }
                    if let Some((ex, ew, ref eh)) = extra_header {
                        self.draw_text(
                            rt,
                            eh,
                            &self.fmt_table_hdr,
                            DipRect::new(*ex, hr.y, *ew, hr.h),
                            pal.muted,
                        )?;
                    }
                }

                for row in rows {
                    let rr = row.rect.offset_y(-scroll);
                    if !visible(rr) {
                        continue;
                    }
                    self.draw_account_row(
                        rt,
                        pal,
                        row,
                        rr,
                        *account_col_x,
                        *account_col_w,
                        hovered,
                    )?;
                }
            }
            ContentItem::SpendChart(chart) => {
                self.draw_spend_chart_card(
                    rt, pal, chart, scroll, vp_top, vp_bot, viewport, chart_jobs,
                )?;
            }
            ContentItem::MeterStyleCard {
                rect,
                style,
                title,
                subtitle,
                selected,
                action,
            } => {
                let r = rect.offset_y(-scroll);
                if visible(r) {
                    let is_hover = hovered == Some(action);
                    self.draw_meter_style_card(
                        rt, pal, r, *style, title, subtitle, *selected, is_hover,
                    )?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_meter_style_card(
        &self,
        rt: &ID2D1DCRenderTarget,
        pal: &SettingsPalette,
        r: DipRect,
        style: TaskbarMeterStyle,
        title: &str,
        subtitle: &str,
        selected: bool,
        hovered: bool,
    ) -> Result<()> {
        let bg = if selected {
            with_alpha(pal.accent, 0.08)
        } else if hovered {
            pal.surface_hover
        } else {
            pal.card
        };
        let border = if selected {
            pal.accent
        } else if hovered {
            pal.border
        } else {
            pal.card_border
        };
        let border_w = if selected { 1.5 } else { 1.0 };
        self.fill_rounded(rt, r, 6.0, bg)?;
        self.stroke_rounded(rt, r, 6.0, border_w, border)?;

        // Radio indicator circle on the left.
        let radio = DipRect::new(r.x + 16.0, r.y + (r.h - 16.0) * 0.5, 16.0, 16.0);
        self.stroke_rounded(
            rt,
            radio,
            8.0,
            1.5,
            if selected { pal.accent } else { pal.border },
        )?;
        if selected {
            let dot = DipRect::new(radio.x + 4.0, radio.y + 4.0, 8.0, 8.0);
            self.fill_rounded(rt, dot, 4.0, pal.accent)?;
        }

        // Right-aligned dark taskbar miniature preview (`198 × 44 DIP`).
        let preview_w = 198.0;
        let preview_h = 44.0;
        let preview = DipRect::new(
            r.x + r.w - 16.0 - preview_w,
            r.y + (r.h - preview_h) * 0.5,
            preview_w,
            preview_h,
        );
        self.fill_rounded(rt, preview, 6.0, rgba(38, 28, 32, 0.96))?;
        self.stroke_rounded(rt, preview, 6.0, 1.0, rgba(255, 255, 255, 0.14))?;

        let sample_meters: [(&str, &str, f32, &str); 2] = match style {
            TaskbarMeterStyle::ClockBelow => {
                [("R", "3%", 0.08, "4h 33m"), ("W", "31%", 0.31, "5d")]
            }
            TaskbarMeterStyle::MinimalBelow => {
                [("R", "3%", 0.08, "4.5h"), ("W", "31%", 0.31, "5d")]
            }
            TaskbarMeterStyle::ResetAtBelow => {
                [("R", "3%", 0.08, "→ 8:40p"), ("W", "31%", 0.31, "→ Sat")]
            }
            TaskbarMeterStyle::PillChip => [("R", "3%", 0.08, "4h 33m"), ("W", "31%", 0.31, "5d")],
        };

        let col_w = 85.0;
        let col_gap = 10.0;
        let start_x = preview.x + (preview.w - (col_w * 2.0 + col_gap)) * 0.5;
        let p_text = rgba(255, 255, 255, 0.96);
        let p_muted = rgba(232, 232, 240, 0.80);
        let p_sub = rgba(232, 232, 240, 0.64);
        let p_track = rgba(255, 255, 255, 0.22);
        let p_ok = rgba(0, 212, 146, 1.0);

        for (idx, (tag, pct, frac, time_str)) in sample_meters.iter().enumerate() {
            let mx = start_x + idx as f32 * (col_w + col_gap);
            if style == TaskbarMeterStyle::PillChip {
                let chip = DipRect::new(mx, preview.y + 8.0, 14.0, 13.0);
                self.fill_rounded(rt, chip, 3.0, rgba(255, 255, 255, 0.14))?;
                self.draw_dynamic_text(
                    rt, tag, 8.5, true, false, false, true, false, chip, p_text,
                )?;
                self.draw_dynamic_text(
                    rt,
                    time_str,
                    8.5,
                    false,
                    false,
                    false,
                    false,
                    false,
                    DipRect::new(mx + 17.0, preview.y + 8.0, col_w - 42.0, 13.0),
                    p_sub,
                )?;
                self.draw_dynamic_text(
                    rt,
                    pct,
                    9.5,
                    true,
                    false,
                    true,
                    false,
                    false,
                    DipRect::new(mx + col_w - 26.0, preview.y + 7.5, 26.0, 13.5),
                    p_text,
                )?;
                let track = DipRect::new(mx, preview.y + 27.0, col_w, 4.5);
                self.fill_rounded(rt, track, 2.25, p_track)?;
                self.fill_rounded(
                    rt,
                    DipRect::new(track.x, track.y, (track.w * *frac).max(4.0), track.h),
                    2.25,
                    p_ok,
                )?;
            } else {
                self.draw_dynamic_text(
                    rt,
                    tag,
                    9.5,
                    true,
                    false,
                    false,
                    false,
                    false,
                    DipRect::new(mx, preview.y + 4.5, 20.0, 13.0),
                    p_muted,
                )?;
                self.draw_dynamic_text(
                    rt,
                    pct,
                    9.5,
                    true,
                    false,
                    true,
                    false,
                    false,
                    DipRect::new(mx + col_w - 28.0, preview.y + 4.5, 28.0, 13.0),
                    p_text,
                )?;
                let track = DipRect::new(mx, preview.y + 19.5, col_w, 4.0);
                self.fill_rounded(rt, track, 2.0, p_track)?;
                self.fill_rounded(
                    rt,
                    DipRect::new(track.x, track.y, (track.w * *frac).max(4.0), track.h),
                    2.0,
                    p_ok,
                )?;
                self.draw_dynamic_text(
                    rt,
                    time_str,
                    8.5,
                    false,
                    false,
                    false,
                    false,
                    false,
                    DipRect::new(mx, preview.y + 25.5, col_w, 12.0),
                    p_sub,
                )?;
            }
        }

        // Title + subtitle on the left of the preview box.
        let text_x = radio.x + radio.w + 14.0;
        let text_w = (preview.x - text_x - 16.0).max(80.0);
        self.draw_dynamic_text(
            rt,
            title,
            14.0,
            true,
            false,
            false,
            false,
            false,
            DipRect::new(text_x, r.y + 18.0, text_w, 20.0),
            pal.text,
        )?;
        self.draw_dynamic_text(
            rt,
            subtitle,
            12.0,
            false,
            false,
            false,
            false,
            true,
            DipRect::new(text_x, r.y + 40.0, text_w, 34.0),
            pal.muted,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_account_row(
        &self,
        rt: &ID2D1DCRenderTarget,
        pal: &SettingsPalette,
        row: &AccountRowRender,
        rr: DipRect,
        account_col_x: f32,
        account_col_w: f32,
        hovered: Option<&Action>,
    ) -> Result<()> {
        // Top row separator.
        self.fill_rect(
            rt,
            DipRect::new(rr.x + 1.0, rr.y, rr.w - 2.0, 1.0),
            pal.card_border,
        )?;

        // Active emerald left indicator pill (3x20).
        if row.active {
            let pill = DipRect::new(rr.x + 1.0, rr.y + (rr.h - 20.0) * 0.5, 3.0, 20.0);
            self.fill_rounded(rt, pill, 1.5, pal.accent)?;
        }

        let alpha = if row.hidden { 0.60 } else { 1.0 };

        // Account column: name + optional hidden tag + warning + plan badge.
        let mut right_limit = account_col_x + account_col_w;
        if let Some((ref plan_label, tone)) = row.plan {
            let bw = (plan_label.chars().count() as f32 * 6.5 + 14.0).clamp(30.0, 74.0);
            let bx = (right_limit - bw).max(account_col_x + 40.0);
            let brect = DipRect::new(bx, rr.y + (rr.h - 18.0) * 0.5, bw, 18.0);
            let bc = pal.badge(tone);
            self.draw_badge(rt, brect, plan_label, bc, alpha)?;
            right_limit = bx - 6.0;
        }
        if let Some(ref warn_txt) = row.warning {
            let ww = (warn_txt.chars().count() as f32 * 6.4 + 6.0).clamp(44.0, 88.0);
            let wx = (right_limit - ww).max(account_col_x + 36.0);
            self.draw_dynamic_text(
                rt,
                warn_txt,
                11.5,
                true,
                false,
                true,
                false,
                false,
                DipRect::new(wx, rr.y, ww, rr.h),
                with_alpha(pal.warn_text, alpha),
            )?;
            right_limit = wx - 6.0;
        }
        let name_text = if row.hidden {
            format!("{} (hidden)", row.name)
        } else {
            row.name.clone()
        };
        let name_w = (right_limit - account_col_x).max(30.0);
        self.draw_text(
            rt,
            &name_text,
            &self.fmt_row_name,
            DipRect::new(account_col_x, rr.y, name_w, rr.h),
            with_alpha(pal.text, alpha),
        )?;

        // Usage cells or spanning note.
        if let Some((nx, nw, ref note_text)) = row.note {
            self.draw_dynamic_text(
                rt,
                note_text,
                12.0,
                false,
                false,
                false,
                false,
                false,
                DipRect::new(nx, rr.y, nw, rr.h),
                with_alpha(pal.muted, alpha),
            )?;
        } else {
            for cell in &row.cells {
                let top_y = rr.y + 12.0;
                let pct_color = if cell.dimmed {
                    with_alpha(pal.muted, alpha)
                } else if cell.severity == BarSeverity::Danger {
                    with_alpha(pal.bar_danger, alpha)
                } else {
                    with_alpha(pal.text, alpha)
                };
                self.draw_text(
                    rt,
                    &cell.percent_text,
                    &self.fmt_cell_pct,
                    DipRect::new(cell.x, top_y, cell.w * 0.52, 16.0),
                    pct_color,
                )?;
                if let Some(ref reset) = cell.reset_text {
                    self.draw_text(
                        rt,
                        reset,
                        &self.fmt_cell_reset,
                        DipRect::new(cell.x + cell.w * 0.42, top_y, cell.w * 0.58, 16.0),
                        with_alpha(pal.muted, alpha),
                    )?;
                }
                let track = DipRect::new(cell.x, top_y + 20.0, cell.w, 4.0);
                self.fill_rounded(rt, track, 2.0, with_alpha(pal.track, alpha))?;
                if let Some(pct) = cell.percent_val {
                    let frac = (pct as f32 / 100.0).clamp(0.0, 1.0);
                    let fw = (track.w * frac).max(if frac > 0.0 { 3.0 } else { 0.0 });
                    if fw > 0.0 {
                        let fill = DipRect::new(track.x, track.y, fw, track.h);
                        let fc = with_alpha(pal.bar_color(cell.severity, cell.dimmed), alpha);
                        self.fill_rounded(rt, fill, 2.0, fc)?;
                    }
                }
            }
        }

        // Extra column (Resets / API spend).
        if let Some((ex, ew, ref etext, is_muted)) = row.extra {
            let fg = if is_muted {
                with_alpha(pal.muted, alpha)
            } else {
                with_alpha(pal.text_secondary, alpha)
            };
            self.draw_dynamic_text(
                rt,
                etext,
                12.0,
                false,
                false,
                false,
                false,
                false,
                DipRect::new(ex, rr.y, ew, rr.h),
                fg,
            )?;
        }

        // "⋯" action button.
        let mbtn = DipRect::new(
            row.menu_btn.x,
            rr.y + (rr.h - 28.0) * 0.5,
            row.menu_btn.w,
            28.0,
        );
        let is_hover = hovered == Some(&row.menu_action);
        self.draw_button(rt, pal, mbtn, "⋯", ButtonStyle::GhostIcon, false, is_hover)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_spend_chart_card(
        &self,
        rt: &ID2D1DCRenderTarget,
        pal: &SettingsPalette,
        chart: &SpendChartRender,
        scroll: f32,
        vp_top: f32,
        vp_bot: f32,
        viewport: DipRect,
        chart_jobs: &mut Vec<ChartDrawJob>,
    ) -> Result<()> {
        let cr = chart.card_rect.offset_y(-scroll);
        if cr.y + cr.h < vp_top - 4.0 || cr.y > vp_bot + 4.0 {
            return Ok(());
        }

        self.fill_rounded(rt, cr, 4.0, pal.card)?;
        self.stroke_rounded(rt, cr, 4.0, 1.0, pal.card_border)?;

        // Card header title.
        self.draw_dynamic_text(
            rt,
            &chart.header_title,
            14.0,
            false,
            false,
            false,
            false,
            false,
            DipRect::new(cr.x + 16.0, cr.y + 18.0, (cr.w - 200.0).max(80.0), 22.0),
            pal.text,
        )?;

        let plot = chart.plot_rect.offset_y(-scroll);

        // Baseline + subtle horizontal midpoint guide.
        self.fill_rect(
            rt,
            DipRect::new(plot.x, plot.y + plot.h - 1.0, plot.w, 1.0),
            pal.border,
        )?;
        self.fill_rect(
            rt,
            DipRect::new(plot.x, plot.y + plot.h * 0.5, plot.w, 1.0),
            with_alpha(pal.border, 0.35),
        )?;

        // Queue antialiased DIB rasterization for the filled area + All & Fable curves.
        if chart.points.len() >= 2 {
            chart_jobs.push(ChartDrawJob {
                plot_rect_client: plot,
                clip_rect_client: viewport,
                points: chart.points.clone(),
                all_rgb: pal.chart_all_rgb,
                fable_rgb: pal.chart_fable_rgb,
            });
        }

        // Peak rate label top-left of plot.
        self.draw_dynamic_text(
            rt,
            &chart.peak_label,
            11.5,
            false,
            true,
            false,
            false,
            false,
            DipRect::new(plot.x, plot.y, 96.0, 16.0),
            pal.muted,
        )?;

        // Bottom axis labels + centered legend (All / Fable).
        let axis_y = plot.y + plot.h + 6.0;
        self.draw_dynamic_text(
            rt,
            &chart.left_label,
            11.5,
            false,
            false,
            false,
            false,
            false,
            DipRect::new(plot.x, axis_y, 110.0, 16.0),
            pal.muted,
        )?;
        self.draw_dynamic_text(
            rt,
            "now",
            11.5,
            false,
            false,
            true,
            false,
            false,
            DipRect::new(plot.x + plot.w - 60.0, axis_y, 60.0, 16.0),
            pal.muted,
        )?;

        let legend_cx = plot.x + plot.w * 0.5;
        let all_sw = DipRect::new(legend_cx - 52.0, axis_y + 7.0, 12.0, 2.5);
        self.fill_rounded(rt, all_sw, 1.0, pal.chart_all)?;
        self.draw_dynamic_text(
            rt,
            "All",
            11.5,
            false,
            false,
            false,
            false,
            false,
            DipRect::new(all_sw.x + 16.0, axis_y, 28.0, 16.0),
            pal.muted,
        )?;
        let fab_sw = DipRect::new(legend_cx + 4.0, axis_y + 7.0, 12.0, 2.5);
        self.fill_rounded(rt, fab_sw, 1.0, pal.chart_fable)?;
        self.draw_dynamic_text(
            rt,
            "Fable",
            11.5,
            false,
            false,
            false,
            false,
            false,
            DipRect::new(fab_sw.x + 16.0, axis_y, 44.0, 16.0),
            pal.muted,
        )?;

        // Model breakdown list below the axis.
        if !chart.models.is_empty() {
            let mut my = axis_y + 26.0;
            for (model_name, right_str) in &chart.models {
                self.draw_dynamic_text(
                    rt,
                    model_name,
                    12.0,
                    false,
                    false,
                    false,
                    false,
                    false,
                    DipRect::new(plot.x, my, (plot.w - 130.0).max(80.0), 18.0),
                    pal.text,
                )?;
                self.draw_dynamic_text(
                    rt,
                    right_str,
                    12.0,
                    false,
                    true,
                    true,
                    false,
                    false,
                    DipRect::new(plot.x + plot.w - 126.0, my, 126.0, 18.0),
                    pal.text_secondary,
                )?;
                my += 20.0;
            }
        }
        Ok(())
    }

    fn draw_toggle(
        &self,
        rt: &ID2D1DCRenderTarget,
        pal: &SettingsPalette,
        rect: DipRect,
        checked: bool,
        hovered: bool,
    ) -> Result<()> {
        let radius = rect.h * 0.5;
        if checked {
            self.fill_rounded(rt, rect, radius, pal.switch_on)?;
            self.stroke_rounded(rt, rect, radius, 1.0, pal.switch_on)?;
        } else {
            if hovered {
                self.fill_rounded(rt, rect, radius, pal.surface_hover)?;
            }
            self.stroke_rounded(rt, rect, radius, 1.0, pal.input_border)?;
        }

        let thumb_d = if hovered { 13.5 } else { 12.0 };
        let thumb_y = rect.y + (rect.h - thumb_d) * 0.5;
        let thumb_x = if checked {
            rect.x + rect.w - thumb_d - 3.5
        } else {
            rect.x + 3.5
        };
        let thumb_color = if checked {
            rgba(255, 255, 255, 1.0)
        } else {
            pal.text_secondary
        };
        self.fill_rounded(
            rt,
            DipRect::new(thumb_x, thumb_y, thumb_d, thumb_d),
            thumb_d * 0.5,
            thumb_color,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_button(
        &self,
        rt: &ID2D1DCRenderTarget,
        pal: &SettingsPalette,
        rect: DipRect,
        label: &str,
        style: ButtonStyle,
        disabled: bool,
        hovered: bool,
    ) -> Result<()> {
        let alpha = if disabled { 0.48 } else { 1.0 };
        match style {
            ButtonStyle::Primary => {
                let bg = if hovered {
                    pal.primary_hover
                } else {
                    pal.primary_bg
                };
                self.fill_rounded(rt, rect, 4.0, with_alpha(bg, alpha))?;
                self.draw_text(
                    rt,
                    label,
                    &self.fmt_btn,
                    rect,
                    with_alpha(pal.primary_fg, alpha),
                )?;
            }
            ButtonStyle::Secondary => {
                let bg = if hovered {
                    pal.surface_hover
                } else {
                    pal.surface
                };
                self.fill_rounded(rt, rect, 4.0, with_alpha(bg, alpha))?;
                self.stroke_rounded(rt, rect, 4.0, 1.0, with_alpha(pal.border, alpha))?;
                self.draw_text(rt, label, &self.fmt_btn, rect, with_alpha(pal.text, alpha))?;
            }
            ButtonStyle::Pill => {
                if hovered {
                    self.fill_rounded(rt, rect, rect.h * 0.5, pal.surface_hover)?;
                }
                self.stroke_rounded(rt, rect, rect.h * 0.5, 1.0, pal.border)?;
                self.draw_text(
                    rt,
                    label,
                    &self.fmt_btn_sm,
                    rect,
                    if hovered { pal.text } else { pal.muted },
                )?;
            }
            ButtonStyle::GhostIcon => {
                if hovered {
                    self.fill_rounded(rt, rect, 4.0, pal.surface_hover)?;
                }
                let fg = if disabled {
                    with_alpha(pal.muted, 0.35)
                } else if hovered {
                    pal.text
                } else {
                    pal.muted
                };
                self.draw_text(rt, label, &self.fmt_btn, rect, fg)?;
            }
        }
        Ok(())
    }

    fn draw_badge(
        &self,
        rt: &ID2D1DCRenderTarget,
        rect: DipRect,
        label: &str,
        colors: BadgeColors,
        alpha: f32,
    ) -> Result<()> {
        let r = rect.h * 0.5;
        self.fill_rounded(rt, rect, r, with_alpha(colors.bg, alpha))?;
        self.stroke_rounded(rt, rect, r, 1.0, with_alpha(colors.border, alpha))?;
        self.draw_text(
            rt,
            label,
            &self.fmt_badge,
            rect,
            with_alpha(colors.fg, alpha),
        )
    }

    fn draw_provider_icon(
        &self,
        rt: &ID2D1DCRenderTarget,
        kind: ProviderKind,
        rect: DipRect,
        pal: &SettingsPalette,
    ) {
        if self
            .icons
            .draw_provider(rt, kind, rect.to_d2d(), pal.is_light)
        {
            return;
        }
        let (glyph, bg, fg) = match kind {
            ProviderKind::Claude => ("C", rgba(217, 119, 87, 0.25), rgba(217, 119, 87, 1.0)),
            ProviderKind::Cursor => ("▲", rgba(56, 189, 248, 0.22), rgba(56, 189, 248, 1.0)),
            ProviderKind::Codex => ("◎", rgba(161, 161, 170, 0.22), pal.text),
            ProviderKind::Grok => ("G", rgba(168, 85, 247, 0.22), rgba(168, 85, 247, 1.0)),
            ProviderKind::OpenCode => ("O", rgba(16, 185, 129, 0.22), pal.accent),
            ProviderKind::Antigravity => ("A", rgba(96, 165, 250, 0.22), rgba(96, 165, 250, 1.0)),
        };
        let _ = self.fill_rounded(rt, rect, 4.0, bg);
        let _ = self.draw_text(rt, glyph, &self.fmt_badge, rect, fg);
    }

    fn fill_rect(
        &self,
        rt: &ID2D1DCRenderTarget,
        rect: DipRect,
        color: D2D1_COLOR_F,
    ) -> Result<()> {
        // SAFETY: `rt` is inside an active BeginDraw/EndDraw pair.
        unsafe {
            let brush = rt.CreateSolidColorBrush(&color, None)?;
            rt.FillRectangle(&rect.to_d2d(), &brush);
        }
        Ok(())
    }

    fn fill_rounded(
        &self,
        rt: &ID2D1DCRenderTarget,
        rect: DipRect,
        radius: f32,
        color: D2D1_COLOR_F,
    ) -> Result<()> {
        let rr = D2D1_ROUNDED_RECT {
            rect: rect.to_d2d(),
            radiusX: radius,
            radiusY: radius,
        };
        // SAFETY: `rt` is inside an active BeginDraw/EndDraw pair.
        unsafe {
            let brush = rt.CreateSolidColorBrush(&color, None)?;
            rt.FillRoundedRectangle(&rr, &brush);
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
        let rr = D2D1_ROUNDED_RECT {
            rect: rect.to_d2d(),
            radiusX: radius,
            radiusY: radius,
        };
        let no_style: Option<&ID2D1StrokeStyle> = None;
        // SAFETY: `rt` is inside an active BeginDraw/EndDraw pair.
        unsafe {
            let brush = rt.CreateSolidColorBrush(&color, None)?;
            rt.DrawRoundedRectangle(&rr, &brush, stroke_width, no_style);
        }
        Ok(())
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
                &rect.to_d2d(),
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_dynamic_text(
        &self,
        rt: &ID2D1DCRenderTarget,
        text: &str,
        size: f32,
        semi_bold: bool,
        mono: bool,
        align_right: bool,
        align_center: bool,
        wrap: bool,
        rect: DipRect,
        color: D2D1_COLOR_F,
    ) -> Result<()> {
        let family = if mono { w!("Consolas") } else { w!("Segoe UI") };
        let weight = if semi_bold {
            DWRITE_FONT_WEIGHT_SEMI_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        let align = if align_center {
            DWRITE_TEXT_ALIGNMENT_CENTER
        } else if align_right {
            DWRITE_TEXT_ALIGNMENT_TRAILING
        } else {
            DWRITE_TEXT_ALIGNMENT_LEADING
        };
        let fmt = make_fmt(&self.dwrite, family, weight, size, align, wrap, wrap)?;
        self.draw_text(rt, text, &fmt, rect, color)
    }
}

fn make_fmt(
    dwrite: &IDWriteFactory,
    family: windows::core::PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
    align: DWRITE_TEXT_ALIGNMENT,
    wrap: bool,
    top_align: bool,
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
        fmt.SetParagraphAlignment(if top_align {
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR
        } else {
            DWRITE_PARAGRAPH_ALIGNMENT_CENTER
        })?;
        fmt.SetWordWrapping(if wrap {
            DWRITE_WORD_WRAPPING_WRAP
        } else {
            DWRITE_WORD_WRAPPING_NO_WRAP
        })?;
        Ok(fmt)
    }
}

// ---- Smooth Antialiased Spend Chart Rasterizer (BGRA premultiplied DIB) ---------------------

fn blend_pixel_bgra(buf: &mut [u8], idx: usize, rgb: (u8, u8, u8), alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    if a <= 0.002 || idx + 3 >= buf.len() {
        return;
    }
    let inv = 1.0 - a;
    let b = f32::from(rgb.2) * a + f32::from(buf[idx]) * inv;
    let g = f32::from(rgb.1) * a + f32::from(buf[idx + 1]) * inv;
    let r = f32::from(rgb.0) * a + f32::from(buf[idx + 2]) * inv;
    buf[idx] = b.round().clamp(0.0, 255.0) as u8;
    buf[idx + 1] = g.round().clamp(0.0, 255.0) as u8;
    buf[idx + 2] = r.round().clamp(0.0, 255.0) as u8;
    buf[idx + 3] = 255;
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let len_sq = vx * vx + vy * vy;
    if len_sq <= 1e-6 {
        return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
    }
    let t = ((wx * vx + wy * vy) / len_sq).clamp(0.0, 1.0);
    let proj_x = ax + t * vx;
    let proj_y = ay + t * vy;
    ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
}

fn rasterize_spend_chart(
    buf: &mut [u8],
    stride_px: usize,
    height_px: usize,
    scale: f32,
    job: &ChartDrawJob,
    toast_rect: Option<DipRect>,
) {
    let n = job.points.len();
    if n < 2 {
        return;
    }

    let plot_x0 = job.plot_rect_client.x * scale;
    let plot_y0 = job.plot_rect_client.y * scale;
    let plot_w = (job.plot_rect_client.w * scale).max(1.0);
    let plot_h = (job.plot_rect_client.h * scale).max(1.0);
    let plot_y1 = plot_y0 + plot_h - scale;

    let clip_x0 = (job.clip_rect_client.x * scale).max(0.0);
    let clip_y0 = (job.clip_rect_client.y * scale).max(0.0);
    let clip_x1 = ((job.clip_rect_client.x + job.clip_rect_client.w) * scale).min(stride_px as f32);
    let clip_y1 = ((job.clip_rect_client.y + job.clip_rect_client.h) * scale).min(height_px as f32);

    let toast_px = toast_rect.map(|r| {
        (
            r.x * scale,
            r.y * scale,
            (r.x + r.w) * scale,
            (r.y + r.h) * scale,
        )
    });
    let in_toast = |x: i32, y: i32| -> bool {
        toast_px.is_some_and(|(tx0, ty0, tx1, ty1)| {
            let xf = x as f32 + 0.5;
            let yf = y as f32 + 0.5;
            xf >= tx0 && xf <= tx1 && yf >= ty0 && yf <= ty1
        })
    };

    let min_x = plot_x0.max(clip_x0).floor() as i32;
    let max_x = (plot_x0 + plot_w).min(clip_x1).ceil() as i32;
    let min_y = (plot_y0 - 2.0 * scale).max(clip_y0).floor() as i32;
    let max_y = (plot_y1 + 2.0 * scale).min(clip_y1).ceil() as i32;
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let max_val = job.points.iter().map(|(t, _)| *t).fold(1.0_f64, f64::max) * 1.1;

    let map_x = |i: usize| plot_x0 + (i as f32 / (n - 1) as f32) * plot_w;
    let map_y = |v: f64| plot_y1 - ((v / max_val).clamp(0.0, 1.0) as f32) * (plot_h - 4.0 * scale);

    let all_pts: Vec<(f32, f32)> = job
        .points
        .iter()
        .enumerate()
        .map(|(i, (t, _))| (map_x(i), map_y(*t)))
        .collect();
    let fab_pts: Vec<(f32, f32)> = job
        .points
        .iter()
        .enumerate()
        .map(|(i, (_, f))| (map_x(i), map_y(*f)))
        .collect();

    // 1. Vertical gradient area fill under `All` curve (0.30 alpha at curve -> 0.00 at bottom).
    for x in min_x..max_x {
        let xf = x as f32 + 0.5;
        let t_global = ((xf - plot_x0) / plot_w).clamp(0.0, 1.0) * (n - 1) as f32;
        let idx = (t_global.floor() as usize).min(n - 2);
        let frac = t_global - idx as f32;
        let curve_y = all_pts[idx].1 + (all_pts[idx + 1].1 - all_pts[idx].1) * frac;
        let start_y = curve_y.max(min_y as f32).floor() as i32;
        let end_y = plot_y1.min(max_y as f32).ceil() as i32;
        for y in start_y..end_y {
            if y < 0 || x < 0 || in_toast(x, y) {
                continue;
            }
            let yf = y as f32 + 0.5;
            let cov = (yf - curve_y).clamp(0.0, 1.0);
            let grad = (1.0 - ((yf - plot_y0) / plot_h).clamp(0.0, 1.0)) * 0.30;
            let off = (y as usize * stride_px + x as usize) * 4;
            blend_pixel_bgra(buf, off, job.all_rgb, cov * grad);
        }
    }

    // 2. Smooth antialiased 2.0 DIP polyline for `All` and `Fable`.
    let half_w = 1.0 * scale;
    let feather = 1.0_f32;
    let stroke_series = |buf: &mut [u8], pts: &[(f32, f32)], rgb: (u8, u8, u8)| {
        for seg in pts.windows(2) {
            let (ax, ay) = seg[0];
            let (bx, by) = seg[1];
            let sx0 = (ax.min(bx) - half_w - feather).max(min_x as f32).floor() as i32;
            let sx1 = (ax.max(bx) + half_w + feather).min(max_x as f32).ceil() as i32;
            let sy0 = (ay.min(by) - half_w - feather).max(min_y as f32).floor() as i32;
            let sy1 = (ay.max(by) + half_w + feather).min(max_y as f32).ceil() as i32;
            for y in sy0..sy1 {
                if y < 0 {
                    continue;
                }
                for x in sx0..sx1 {
                    if x < 0 || in_toast(x, y) {
                        continue;
                    }
                    let d = dist_to_segment(x as f32 + 0.5, y as f32 + 0.5, ax, ay, bx, by);
                    if d < half_w + feather {
                        let a = ((half_w + feather - d) / feather).clamp(0.0, 1.0);
                        let off = (y as usize * stride_px + x as usize) * 4;
                        blend_pixel_bgra(buf, off, rgb, a);
                    }
                }
            }
        }
    };

    stroke_series(buf, &all_pts, job.all_rgb);
    stroke_series(buf, &fab_pts, job.fable_rgb);
}
