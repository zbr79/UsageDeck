//! Lightweight native Direct2D/DirectWrite layered hover tooltip window for
//! taskbar provider pills.
//!
//! Renders a Fluent dark/light rounded tooltip directly above the hovered pill
//! without stealing focus (`WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW`).

use crate::layout::DipRect;
use crate::taskbar::{TaskbarMetrics, TaskbarTheme};
use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1StrokeStyle, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SetWindowPos, ShowWindow,
    UpdateLayeredWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const WM_NCHITTEST: u32 = 0x0084;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const HTTRANSPARENT: isize = -1;
const MA_NOACTIVATE: isize = 3;

const TOOLTIP_MAX_TEXT_WIDTH_DIP: f32 = 340.0;
const TOOLTIP_PAD_X_DIP: f32 = 10.0;
const TOOLTIP_PAD_Y_DIP: f32 = 6.0;
const TOOLTIP_RADIUS_DIP: f32 = 7.0;
const TOOLTIP_GAP_DIP: f32 = 6.0;
const TOOLTIP_MARGIN_DIP: f32 = 8.0;

fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a,
    }
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
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Native Win32/Direct2D hover tooltip popup anchored above taskbar pills.
pub struct TooltipWindow {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    fmt: IDWriteTextFormat,
    visible: bool,
    last_key: Option<(String, i32, i32, u32, TaskbarTheme)>,
}

impl TooltipWindow {
    /// Register the tooltip window class and create the hidden layered popup HWND.
    pub fn new() -> Result<Self> {
        // SAFETY: Direct2D/DirectWrite factories and layered Win32 popup lifecycle.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let no_coll: Option<&IDWriteFontCollection> = None;
            let fmt = dwrite.CreateTextFormat(
                w!("Segoe UI"),
                no_coll,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                11.5,
                w!("en-us"),
            )?;
            fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
            fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            fmt.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;

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
                WS_EX_LAYERED
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOPMOST
                    | WS_EX_TRANSPARENT,
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
                fmt,
                visible: false,
                last_key: None,
            })
        }
    }

    /// Show or update the tooltip above `anchor_rect_px` (in physical screen pixels)
    /// without stealing focus from the active application.
    pub fn show(&mut self, text: &str, anchor_rect_px: RECT, metrics: &TaskbarMetrics) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            self.hide();
            return;
        }

        let anchor_cx = (anchor_rect_px.left + anchor_rect_px.right) / 2;
        let anchor_top = anchor_rect_px.top.min(metrics.tray_rect.top);
        let key = (
            trimmed.to_string(),
            anchor_cx,
            anchor_top,
            metrics.dpi,
            metrics.theme,
        );
        if self.visible && self.last_key.as_ref() == Some(&key) {
            return;
        }

        if let Err(e) = self.paint_and_position(trimmed, anchor_cx, anchor_top, metrics) {
            log::warn!("tooltip render failed: {e}");
            return;
        }
        self.last_key = Some(key);

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

    /// Hide the tooltip if currently visible.
    pub fn hide(&mut self) {
        if self.visible {
            // SAFETY: hides our own tooltip HWND.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
            self.last_key = None;
        }
    }

    fn paint_and_position(
        &self,
        text: &str,
        anchor_cx: i32,
        anchor_top: i32,
        metrics: &TaskbarMetrics,
    ) -> Result<()> {
        let wide: Vec<u16> = text.encode_utf16().collect();

        // Measure text dimensions via DirectWrite layout.
        let (text_w_dip, text_h_dip) = unsafe {
            let layout = self.dwrite.CreateTextLayout(
                &wide,
                &self.fmt,
                TOOLTIP_MAX_TEXT_WIDTH_DIP,
                160.0,
            )?;
            let mut tm = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut tm)?;
            (
                tm.widthIncludingTrailingWhitespace
                    .clamp(24.0, TOOLTIP_MAX_TEXT_WIDTH_DIP)
                    .ceil(),
                tm.height.clamp(14.0, 140.0).ceil(),
            )
        };

        let box_w_dip = text_w_dip + TOOLTIP_PAD_X_DIP * 2.0 + 2.0;
        let box_h_dip = text_h_dip + TOOLTIP_PAD_Y_DIP * 2.0 + 2.0;
        let width_px = (box_w_dip * metrics.scale).ceil() as i32;
        let height_px = (box_h_dip * metrics.scale).ceil() as i32;
        if width_px <= 0 || height_px <= 0 {
            return Ok(());
        }

        let margin_px = (TOOLTIP_MARGIN_DIP * metrics.scale).round() as i32;
        let gap_px = (TOOLTIP_GAP_DIP * metrics.scale).round() as i32;
        let x = (anchor_cx - width_px / 2)
            .max(metrics.monitor_rect.left + margin_px)
            .min(metrics.monitor_rect.right - width_px - margin_px);
        let y = (anchor_top - gap_px - height_px).max(metrics.monitor_rect.top + margin_px);

        let (bg, border, fg) = match metrics.theme {
            TaskbarTheme::Dark => (
                rgba(32, 31, 34, 0.96),
                rgba(255, 255, 255, 0.16),
                rgba(244, 244, 245, 0.98),
            ),
            TaskbarTheme::Light => (
                rgba(250, 250, 252, 0.97),
                rgba(0, 0, 0, 0.14),
                rgba(24, 24, 27, 0.96),
            ),
        };

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
                dpiX: metrics.dpi as f32,
                dpiY: metrics.dpi as f32,
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

                let outer = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: 0.5,
                        top: 0.5,
                        right: box_w_dip - 0.5,
                        bottom: box_h_dip - 0.5,
                    },
                    radiusX: TOOLTIP_RADIUS_DIP,
                    radiusY: TOOLTIP_RADIUS_DIP,
                };
                let bg_brush = rt.CreateSolidColorBrush(&bg, None)?;
                let border_brush = rt.CreateSolidColorBrush(&border, None)?;
                let fg_brush = rt.CreateSolidColorBrush(&fg, None)?;
                let no_style: Option<&ID2D1StrokeStyle> = None;

                rt.FillRoundedRectangle(&outer, &bg_brush);
                rt.DrawRoundedRectangle(&outer, &border_brush, 1.0, no_style);

                let text_rect = D2D_RECT_F {
                    left: TOOLTIP_PAD_X_DIP + 1.0,
                    top: TOOLTIP_PAD_Y_DIP + 1.0,
                    right: box_w_dip - TOOLTIP_PAD_X_DIP - 1.0,
                    bottom: box_h_dip - TOOLTIP_PAD_Y_DIP - 1.0,
                };
                rt.DrawText(
                    &wide,
                    &self.fmt,
                    &text_rect,
                    &fg_brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );

                rt.EndDraw(None, None)?;

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
