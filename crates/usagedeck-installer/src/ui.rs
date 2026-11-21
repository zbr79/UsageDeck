//! Native Win32 + Direct2D/DirectWrite one-click installer window.
//!
//! Zero Tauri, zero WebView2, zero HTML/CSS: renders a crisp PerMonitorV2 DPI-aware
//! rounded dark card to an in-memory 32-bit BGRA DIB and commits atomically via
//! `UpdateLayeredWindow` (`ULW_ALPHA`).

use crate::decide::{self, Action, DesktopShortcutPolicy, InstallPlan, UninstallPlan};
use crate::install::{self, InstallOutcome};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use windows::core::{w, Result};
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
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
    MonitorFromPoint, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE, VK_RETURN, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow,
    DispatchMessageW, GetMessageW, GetWindowRect, KillTimer, LoadCursorW, LoadIconW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SetCursor, SetForegroundWindow, SetTimer, ShowWindow,
    TranslateMessage, UpdateLayeredWindow, HCURSOR, HICON, HTCAPTION, IDC_ARROW, IDC_HAND,
    IDI_APPLICATION, LR_DEFAULTCOLOR, MSG, SW_SHOW, ULW_ALPHA, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSELEAVE, WM_MOUSEMOVE,
    WM_NCLBUTTONDOWN, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW, WS_EX_APPWINDOW, WS_EX_LAYERED,
    WS_POPUP,
};

const WINDOW_W_DIP: f32 = 440.0;
const WINDOW_H_DIP: f32 = 356.0;
const CARD_PAD_DIP: f32 = 28.0;
const WM_APP_WORKER: u32 = WM_APP + 1;
const TIMER_ANIM_ID: usize = 1;
const TIMER_ANIM_MS: u32 = 33;

const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/icon.ico");

#[derive(Clone, Copy)]
struct DipRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl DipRect {
    const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    fn to_d2d(self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.x,
            top: self.y,
            right: self.x + self.w,
            bottom: self.y + self.h,
        }
    }

    fn rounded(self, radius: f32) -> D2D1_ROUNDED_RECT {
        D2D1_ROUNDED_RECT {
            rect: self.to_d2d(),
            radiusX: radius,
            radiusY: radius,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitTarget {
    Close,
    DesktopCheckbox,
    AutostartCheckbox,
    PrimaryAction,
    UninstallLink,
    DeleteDataCheckbox,
    ConfirmUninstall,
    CancelUninstall,
    LaunchApp,
    SecondaryClose,
    Retry,
}

enum Phase {
    Idle,
    ConfirmUninstall,
    Busy {
        uninstalling: bool,
        progress: f32,
        status: String,
        pulse: f32,
    },
    Success {
        uninstalled: bool,
        exe_path: Option<PathBuf>,
        warning: Option<String>,
    },
    Error {
        message: String,
        was_uninstall: bool,
    },
}

enum WorkerEvent {
    Progress(f32, String),
    InstallDone(std::result::Result<InstallOutcome, String>),
    UninstallDone(std::result::Result<(), String>),
}

struct Renderer {
    d2d: ID2D1Factory,
    fmt_title: IDWriteTextFormat,
    fmt_version: IDWriteTextFormat,
    fmt_subtitle: IDWriteTextFormat,
    fmt_label: IDWriteTextFormat,
    fmt_btn: IDWriteTextFormat,
    fmt_btn_sm: IDWriteTextFormat,
    fmt_muted_left: IDWriteTextFormat,
    fmt_link_right: IDWriteTextFormat,
    fmt_status: IDWriteTextFormat,
    fmt_wrap: IDWriteTextFormat,
    fmt_center_icon: IDWriteTextFormat,
    fmt_check: IDWriteTextFormat,
}

impl Renderer {
    fn new() -> Result<Self> {
        // SAFETY: creating single-threaded Direct2D and shared DirectWrite factories.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            let fmt_title = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                20.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_version = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_MEDIUM,
                11.5,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
            )?;
            let fmt_subtitle = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_NORMAL,
                13.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_label = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_NORMAL,
                13.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_btn = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                14.0,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
            )?;
            let fmt_btn_sm = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_MEDIUM,
                13.0,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
            )?;
            let fmt_muted_left = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_NORMAL,
                11.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_link_right = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_MEDIUM,
                12.0,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                false,
            )?;
            let fmt_status = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                15.0,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                false,
            )?;
            let fmt_wrap = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_NORMAL,
                12.5,
                DWRITE_TEXT_ALIGNMENT_LEADING,
                true,
            )?;
            let fmt_center_icon = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_BOLD,
                15.0,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
            )?;
            let fmt_check = make_format(
                &dwrite,
                DWRITE_FONT_WEIGHT_BOLD,
                12.0,
                DWRITE_TEXT_ALIGNMENT_CENTER,
                false,
            )?;

            Ok(Self {
                d2d,
                fmt_title,
                fmt_version,
                fmt_subtitle,
                fmt_label,
                fmt_btn,
                fmt_btn_sm,
                fmt_muted_left,
                fmt_link_right,
                fmt_status,
                fmt_wrap,
                fmt_center_icon,
                fmt_check,
            })
        }
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
            rt.FillRoundedRectangle(&rect.rounded(radius), &brush);
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
            rt.DrawRoundedRectangle(&rect.rounded(radius), &brush, stroke_width, no_style);
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

    fn commit_layered(
        &self,
        hwnd: HWND,
        origin: POINT,
        size: SIZE,
        dpi: u32,
        paint: impl FnOnce(&ID2D1DCRenderTarget) -> Result<()>,
    ) -> Result<()> {
        let width_px = size.cx;
        let height_px = size.cy;
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

                let pt_src = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                UpdateLayeredWindow(
                    hwnd,
                    None,
                    Some(&origin),
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
}

fn make_format(
    dwrite: &IDWriteFactory,
    weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT,
    size: f32,
    align: windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT,
    wrap: bool,
) -> Result<IDWriteTextFormat> {
    let no_coll: Option<&IDWriteFontCollection> = None;
    // SAFETY: DirectWrite COM factory call with static font family and locale strings.
    unsafe {
        let fmt = dwrite.CreateTextFormat(
            w!("Segoe UI"),
            no_coll,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        fmt.SetTextAlignment(align)?;
        fmt.SetParagraphAlignment(if wrap {
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

fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a,
    }
}

struct UiState {
    renderer: Renderer,
    hwnd: HWND,
    app_icon: HICON,
    arrow_cursor: HCURSOR,
    hand_cursor: HCURSOR,
    dpi: u32,
    win_x: i32,
    win_y: i32,
    action: Action,
    installed_version: Option<String>,
    install_dir: PathBuf,
    desktop_shortcut: bool,
    start_with_windows: bool,
    delete_data_on_uninstall: bool,
    phase: Phase,
    hovered: Option<HitTarget>,
    tracking_leave: bool,
    worker_rx: Option<Receiver<WorkerEvent>>,
}

impl UiState {
    fn scale(&self) -> f32 {
        (self.dpi as f32 / 96.0).max(1.0)
    }

    fn is_busy(&self) -> bool {
        matches!(self.phase, Phase::Busy { .. })
    }

    fn hit_regions(&self) -> Vec<(DipRect, HitTarget)> {
        let mut out = Vec::new();
        let inner_w = WINDOW_W_DIP - CARD_PAD_DIP * 2.0;

        if !self.is_busy() {
            out.push((
                DipRect::new(WINDOW_W_DIP - 40.0, 14.0, 26.0, 26.0),
                HitTarget::Close,
            ));
        }

        match &self.phase {
            Phase::Idle => {
                out.push((
                    DipRect::new(CARD_PAD_DIP, 118.0, inner_w, 40.0),
                    HitTarget::DesktopCheckbox,
                ));
                out.push((
                    DipRect::new(CARD_PAD_DIP, 162.0, inner_w, 40.0),
                    HitTarget::AutostartCheckbox,
                ));
                out.push((
                    DipRect::new(CARD_PAD_DIP, 224.0, inner_w, 44.0),
                    HitTarget::PrimaryAction,
                ));
                if self.action != Action::Install {
                    out.push((
                        DipRect::new(WINDOW_W_DIP - CARD_PAD_DIP - 78.0, 304.0, 78.0, 26.0),
                        HitTarget::UninstallLink,
                    ));
                }
            }
            Phase::ConfirmUninstall => {
                out.push((
                    DipRect::new(CARD_PAD_DIP, 166.0, inner_w, 42.0),
                    HitTarget::DeleteDataCheckbox,
                ));
                let btn_w = (inner_w - 12.0) / 2.0;
                out.push((
                    DipRect::new(CARD_PAD_DIP, 232.0, btn_w, 42.0),
                    HitTarget::ConfirmUninstall,
                ));
                out.push((
                    DipRect::new(CARD_PAD_DIP + btn_w + 12.0, 232.0, btn_w, 42.0),
                    HitTarget::CancelUninstall,
                ));
            }
            Phase::Busy { .. } => {}
            Phase::Success { uninstalled, .. } => {
                if *uninstalled {
                    out.push((
                        DipRect::new(CARD_PAD_DIP, 232.0, inner_w, 42.0),
                        HitTarget::SecondaryClose,
                    ));
                } else {
                    out.push((
                        DipRect::new(CARD_PAD_DIP, 218.0, inner_w, 44.0),
                        HitTarget::LaunchApp,
                    ));
                    out.push((
                        DipRect::new(CARD_PAD_DIP, 274.0, inner_w, 36.0),
                        HitTarget::SecondaryClose,
                    ));
                }
            }
            Phase::Error { .. } => {
                let btn_w = (inner_w - 12.0) / 2.0;
                out.push((
                    DipRect::new(CARD_PAD_DIP, 250.0, btn_w, 42.0),
                    HitTarget::Retry,
                ));
                out.push((
                    DipRect::new(CARD_PAD_DIP + btn_w + 12.0, 250.0, btn_w, 42.0),
                    HitTarget::SecondaryClose,
                ));
            }
        }

        out
    }

    fn hit_test(&self, dx: f32, dy: f32) -> Option<HitTarget> {
        self.hit_regions()
            .into_iter()
            .find(|(rect, _)| rect.contains(dx, dy))
            .map(|(_, target)| target)
    }
}

thread_local! {
    static STATE: RefCell<Option<UiState>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut UiState) -> R) -> Option<R> {
    STATE.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Open the sleek native Win32/Direct2D installer window and run the Win32 message loop.
pub fn run() -> Result<()> {
    let renderer = Renderer::new()?;
    let app_icon = load_app_icon();

    let existing = install::installed();
    let installed_version = existing.and_then(|i| i.version);
    let action = decide::action(installed_version.as_deref(), install::version());
    let install_dir = install::resolved_install_dir(None);
    let desktop_shortcut =
        action == Action::Install || install::desktop_shortcut_path().is_some_and(|p| p.exists());
    let start_with_windows = install::start_at_login();

    // SAFETY: Win32 window class registration, window creation, and message loop.
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let arrow_cursor = LoadCursorW(None, IDC_ARROW)?;
        let hand_cursor = LoadCursorW(None, IDC_HAND)?;

        let class_name = w!("UsageDeckSetupWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            hIcon: app_icon,
            hIconSm: app_icon,
            hCursor: arrow_cursor,
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_APPWINDOW,
            class_name,
            w!("UsageDeck Setup"),
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

        let dpi = {
            let raw = GetDpiForWindow(hwnd);
            if raw == 0 {
                96
            } else {
                raw
            }
        };
        let scale = (dpi as f32 / 96.0).max(1.0);
        let width_px = (WINDOW_W_DIP * scale).round() as i32;
        let height_px = (WINDOW_H_DIP * scale).round() as i32;
        let (win_x, win_y) = centered_origin(width_px, height_px);

        STATE.with(|cell| {
            *cell.borrow_mut() = Some(UiState {
                renderer,
                hwnd,
                app_icon,
                arrow_cursor,
                hand_cursor,
                dpi,
                win_x,
                win_y,
                action,
                installed_version,
                install_dir,
                desktop_shortcut,
                start_with_windows,
                delete_data_on_uninstall: false,
                phase: Phase::Idle,
                hovered: None,
                tracking_leave: false,
                worker_rx: None,
            });
        });

        with_state(paint_window);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        STATE.with(|cell| {
            if let Some(s) = cell.borrow_mut().take() {
                let _ = DestroyIcon(s.app_icon);
            }
        });
    }

    Ok(())
}

fn centered_origin(width_px: i32, height_px: i32) -> (i32, i32) {
    // SAFETY: read-only monitor geometry query on primary display.
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon, &mut info).as_bool() {
            let work = info.rcWork;
            let x = work.left + ((work.right - work.left) - width_px) / 2;
            let y = work.top + ((work.bottom - work.top) - height_px) / 2;
            return (x.max(work.left), y.max(work.top));
        }
    }
    (200, 160)
}

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
    // SAFETY: `slice` is a bounded ICO image payload.
    unsafe { CreateIconFromResourceEx(slice, true, 0x0003_0000, 32, 32, LR_DEFAULTCOLOR).ok() }
}

fn start_install(state: &mut UiState) {
    if state.is_busy() {
        return;
    }
    let plan = InstallPlan {
        silent: false,
        passive: false,
        update: state.action == Action::Update,
        relaunch: false,
        create_start_menu_shortcut: true,
        desktop_shortcut: if state.desktop_shortcut {
            DesktopShortcutPolicy::Create
        } else {
            DesktopShortcutPolicy::Remove
        },
        start_with_windows: Some(state.start_with_windows),
        install_dir: Some(state.install_dir.clone()),
    };

    let (tx, rx) = mpsc::channel();
    state.worker_rx = Some(rx);
    state.hovered = None;
    state.phase = Phase::Busy {
        uninstalling: false,
        progress: 0.08,
        status: state.action.progress_label().to_string(),
        pulse: 0.0,
    };

    let hwnd_raw = state.hwnd.0 as isize;
    // SAFETY: periodic UI animation timer while worker runs.
    unsafe {
        SetTimer(Some(state.hwnd), TIMER_ANIM_ID, TIMER_ANIM_MS, None);
    }

    std::thread::spawn(move || {
        let notify = |ev: WorkerEvent| {
            let _ = tx.send(ev);
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            // SAFETY: `PostMessageW` is thread-safe for posting `WM_APP_WORKER`.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_WORKER, WPARAM(0), LPARAM(0));
            }
        };

        let result = install::install_with_progress(&plan, |pct, msg| {
            notify(WorkerEvent::Progress(pct, msg.to_string()));
        });
        notify(WorkerEvent::InstallDone(result));
    });

    paint_window(state);
}

fn start_uninstall(state: &mut UiState) {
    if state.is_busy() {
        return;
    }
    let plan = UninstallPlan {
        silent: false,
        delete_data: state.delete_data_on_uninstall,
        install_dir: Some(state.install_dir.clone()),
    };

    let (tx, rx) = mpsc::channel();
    state.worker_rx = Some(rx);
    state.hovered = None;
    state.phase = Phase::Busy {
        uninstalling: true,
        progress: 0.08,
        status: "Uninstalling UsageDeck…".to_string(),
        pulse: 0.0,
    };

    let hwnd_raw = state.hwnd.0 as isize;
    // SAFETY: periodic UI animation timer while worker runs.
    unsafe {
        SetTimer(Some(state.hwnd), TIMER_ANIM_ID, TIMER_ANIM_MS, None);
    }

    std::thread::spawn(move || {
        let notify = |ev: WorkerEvent| {
            let _ = tx.send(ev);
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            // SAFETY: `PostMessageW` is thread-safe for posting `WM_APP_WORKER`.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_WORKER, WPARAM(0), LPARAM(0));
            }
        };

        let result = install::uninstall_with_progress(&plan, |pct, msg| {
            notify(WorkerEvent::Progress(pct, msg.to_string()));
        });
        notify(WorkerEvent::UninstallDone(result));
    });

    paint_window(state);
}

fn drain_worker_events(state: &mut UiState) {
    let mut events = Vec::new();
    if let Some(ref rx) = state.worker_rx {
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
    }

    let mut finished = false;
    for ev in events {
        match ev {
            WorkerEvent::Progress(pct, msg) => {
                if let Phase::Busy {
                    progress, status, ..
                } = &mut state.phase
                {
                    *progress = pct.clamp(0.0, 1.0);
                    *status = msg;
                }
            }
            WorkerEvent::InstallDone(Ok(outcome)) => {
                state.install_dir = outcome.install_dir;
                state.phase = Phase::Success {
                    uninstalled: false,
                    exe_path: Some(outcome.exe_path),
                    warning: outcome.warning,
                };
                finished = true;
            }
            WorkerEvent::InstallDone(Err(msg)) => {
                state.phase = Phase::Error {
                    message: msg,
                    was_uninstall: false,
                };
                finished = true;
            }
            WorkerEvent::UninstallDone(Ok(())) => {
                state.phase = Phase::Success {
                    uninstalled: true,
                    exe_path: None,
                    warning: None,
                };
                finished = true;
            }
            WorkerEvent::UninstallDone(Err(msg)) => {
                state.phase = Phase::Error {
                    message: msg,
                    was_uninstall: true,
                };
                finished = true;
            }
        }
    }

    if finished {
        state.worker_rx = None;
        // SAFETY: stop animation timer once background operation finishes.
        unsafe {
            let _ = KillTimer(Some(state.hwnd), TIMER_ANIM_ID);
        }
    }

    paint_window(state);
}

fn close_window(state: &UiState) {
    if state.is_busy() {
        return;
    }
    // SAFETY: destroys our own window and exits the message loop.
    unsafe {
        let _ = DestroyWindow(state.hwnd);
    }
}

fn handle_click(state: &mut UiState, target: HitTarget) {
    match target {
        HitTarget::Close | HitTarget::SecondaryClose => close_window(state),
        HitTarget::DesktopCheckbox => {
            state.desktop_shortcut = !state.desktop_shortcut;
            paint_window(state);
        }
        HitTarget::AutostartCheckbox => {
            state.start_with_windows = !state.start_with_windows;
            paint_window(state);
        }
        HitTarget::PrimaryAction => start_install(state),
        HitTarget::UninstallLink => {
            state.delete_data_on_uninstall = false;
            state.hovered = None;
            state.phase = Phase::ConfirmUninstall;
            paint_window(state);
        }
        HitTarget::DeleteDataCheckbox => {
            state.delete_data_on_uninstall = !state.delete_data_on_uninstall;
            paint_window(state);
        }
        HitTarget::ConfirmUninstall => start_uninstall(state),
        HitTarget::CancelUninstall => {
            state.hovered = None;
            state.phase = Phase::Idle;
            paint_window(state);
        }
        HitTarget::LaunchApp => {
            let exe = match &state.phase {
                Phase::Success {
                    exe_path: Some(p), ..
                } => p.clone(),
                _ => install::installed_exe(),
            };
            match install::launch_app(&exe) {
                Ok(()) => close_window(state),
                Err(e) => {
                    state.phase = Phase::Error {
                        message: format!("Could not start {}: {e}", exe.display()),
                        was_uninstall: false,
                    };
                    paint_window(state);
                }
            }
        }
        HitTarget::Retry => {
            let was_uninstall = matches!(
                state.phase,
                Phase::Error {
                    was_uninstall: true,
                    ..
                }
            );
            if was_uninstall {
                start_uninstall(state);
            } else {
                start_install(state);
            }
        }
    }
}

fn paint_window(state: &mut UiState) {
    // Sync our cached window origin in case the user dragged the card.
    let mut cur_rect = RECT::default();
    // SAFETY: reading bounds of our own live HWND.
    if unsafe { GetWindowRect(state.hwnd, &mut cur_rect) }.is_ok()
        && (cur_rect.right - cur_rect.left) > 1
    {
        state.win_x = cur_rect.left;
        state.win_y = cur_rect.top;
    }

    let scale = state.scale();
    let width_px = (WINDOW_W_DIP * scale).round() as i32;
    let height_px = (WINDOW_H_DIP * scale).round() as i32;

    let _ = state.renderer.commit_layered(
        state.hwnd,
        POINT {
            x: state.win_x,
            y: state.win_y,
        },
        SIZE {
            cx: width_px,
            cy: height_px,
        },
        state.dpi,
        |rt| draw_card(state, rt),
    );
}

fn draw_card(state: &UiState, rt: &ID2D1DCRenderTarget) -> Result<()> {
    let r = &state.renderer;
    let inner_w = WINDOW_W_DIP - CARD_PAD_DIP * 2.0;

    // Palette matching UsageDeck's dark native design.
    let bg = rgba(24, 24, 27, 0.985);
    let surface = rgba(39, 39, 42, 0.95);
    let surface_hover = rgba(52, 52, 58, 0.98);
    let border = rgba(255, 255, 255, 0.13);
    let text = rgba(244, 244, 245, 0.98);
    let text_sec = rgba(212, 212, 216, 0.86);
    let muted = rgba(161, 161, 170, 0.85);
    let accent = rgba(0, 212, 146, 1.0);
    let accent_hover = rgba(42, 227, 166, 1.0);
    let accent_soft = rgba(0, 212, 146, 0.14);
    let accent_fg = rgba(9, 9, 11, 1.0);
    let danger = rgba(239, 68, 68, 1.0);
    let danger_hover = rgba(248, 113, 113, 1.0);
    let danger_soft = rgba(239, 68, 68, 0.16);
    let danger_text = rgba(252, 165, 165, 1.0);

    // Card background & crisp border.
    let card = DipRect::new(0.5, 0.5, WINDOW_W_DIP - 1.0, WINDOW_H_DIP - 1.0);
    r.fill_rounded(rt, card, 14.0, bg)?;
    // Subtle top emerald accent glow strip.
    r.fill_rounded(
        rt,
        DipRect::new(16.0, 1.0, WINDOW_W_DIP - 32.0, 2.5),
        1.25,
        rgba(0, 212, 146, 0.55),
    )?;
    r.stroke_rounded(rt, card, 14.0, 1.0, border)?;

    // Top-right close button (`×`).
    let close_rect = DipRect::new(WINDOW_W_DIP - 40.0, 14.0, 26.0, 26.0);
    if !state.is_busy() {
        if state.hovered == Some(HitTarget::Close) {
            r.fill_rounded(rt, close_rect, 6.0, surface_hover)?;
        }
        r.draw_text(
            rt,
            "×",
            &r.fmt_center_icon,
            close_rect,
            if state.hovered == Some(HitTarget::Close) {
                text
            } else {
                muted
            },
        )?;
    }

    // Brand header: crisp vector emblem (three stacked usage bars) + title + version pill.
    let badge = DipRect::new(CARD_PAD_DIP, 28.0, 48.0, 48.0);
    r.fill_rounded(rt, badge, 12.0, rgba(32, 34, 38, 1.0))?;
    r.stroke_rounded(rt, badge, 12.0, 1.0, rgba(0, 212, 146, 0.42))?;
    r.fill_rounded(
        rt,
        DipRect::new(CARD_PAD_DIP + 10.0, 40.0, 28.0, 5.5),
        2.75,
        accent,
    )?;
    r.fill_rounded(
        rt,
        DipRect::new(CARD_PAD_DIP + 10.0, 49.5, 20.0, 5.5),
        2.75,
        rgba(56, 189, 248, 0.95),
    )?;
    r.fill_rounded(
        rt,
        DipRect::new(CARD_PAD_DIP + 10.0, 59.0, 13.0, 5.5),
        2.75,
        rgba(167, 139, 250, 0.95),
    )?;

    let title_x = CARD_PAD_DIP + 62.0;
    r.draw_text(
        rt,
        "UsageDeck",
        &r.fmt_title,
        DipRect::new(title_x, 27.0, 140.0, 28.0),
        text,
    )?;

    // Version pill next to title.
    let ver_label = format!("v{}", install::VERSION);
    let ver_w = (ver_label.len() as f32 * 6.8 + 16.0).clamp(52.0, 124.0);
    let ver_pill = DipRect::new(title_x + 112.0, 31.0, ver_w, 20.0);
    r.fill_rounded(rt, ver_pill, 10.0, accent_soft)?;
    r.draw_text(rt, &ver_label, &r.fmt_version, ver_pill, accent)?;

    let subtitle = match &state.installed_version {
        Some(v) if state.action != Action::Install => format!("Installed: v{v}"),
        _ => "AI usage, always in view".to_string(),
    };
    r.draw_text(
        rt,
        &subtitle,
        &r.fmt_subtitle,
        DipRect::new(title_x, 54.0, inner_w - 62.0, 22.0),
        muted,
    )?;

    // Hairline divider below header.
    r.fill_rounded(
        rt,
        DipRect::new(CARD_PAD_DIP, 94.0, inner_w, 1.0),
        0.5,
        rgba(255, 255, 255, 0.08),
    )?;

    match &state.phase {
        Phase::Idle => {
            draw_checkbox(
                r,
                rt,
                DipRect::new(CARD_PAD_DIP, 118.0, inner_w, 40.0),
                "Create desktop shortcut",
                state.desktop_shortcut,
                state.hovered == Some(HitTarget::DesktopCheckbox),
                surface,
                surface_hover,
                border,
                accent,
                accent_fg,
                text,
            )?;
            draw_checkbox(
                r,
                rt,
                DipRect::new(CARD_PAD_DIP, 162.0, inner_w, 40.0),
                "Start UsageDeck with Windows",
                state.start_with_windows,
                state.hovered == Some(HitTarget::AutostartCheckbox),
                surface,
                surface_hover,
                border,
                accent,
                accent_fg,
                text,
            )?;

            let primary_rect = DipRect::new(CARD_PAD_DIP, 224.0, inner_w, 44.0);
            let primary_bg = if state.hovered == Some(HitTarget::PrimaryAction) {
                accent_hover
            } else {
                accent
            };
            r.fill_rounded(rt, primary_rect, 9.0, primary_bg)?;
            r.draw_text(
                rt,
                state.action.button_label(),
                &r.fmt_btn,
                primary_rect,
                accent_fg,
            )?;

            let loc_w = if state.action != Action::Install {
                inner_w - 86.0
            } else {
                inner_w
            };
            let loc_text = format!("Folder: {}", state.install_dir.display());
            r.draw_text(
                rt,
                &loc_text,
                &r.fmt_muted_left,
                DipRect::new(CARD_PAD_DIP, 304.0, loc_w, 26.0),
                muted,
            )?;

            if state.action != Action::Install {
                let link_rect = DipRect::new(WINDOW_W_DIP - CARD_PAD_DIP - 78.0, 304.0, 78.0, 26.0);
                let link_color = if state.hovered == Some(HitTarget::UninstallLink) {
                    danger_text
                } else {
                    muted
                };
                r.draw_text(rt, "Uninstall", &r.fmt_link_right, link_rect, link_color)?;
            }
        }
        Phase::ConfirmUninstall => {
            r.draw_text(
                rt,
                "Uninstall UsageDeck?",
                &r.fmt_status,
                DipRect::new(CARD_PAD_DIP, 114.0, inner_w, 24.0),
                text,
            )?;
            r.draw_text(
                rt,
                "Removes UsageDeck, shortcuts, and Windows startup entries.",
                &r.fmt_subtitle,
                DipRect::new(CARD_PAD_DIP, 138.0, inner_w, 20.0),
                muted,
            )?;

            draw_checkbox(
                r,
                rt,
                DipRect::new(CARD_PAD_DIP, 166.0, inner_w, 42.0),
                "Delete settings, logs & saved API keys",
                state.delete_data_on_uninstall,
                state.hovered == Some(HitTarget::DeleteDataCheckbox),
                surface,
                surface_hover,
                border,
                danger,
                text,
                text,
            )?;

            let btn_w = (inner_w - 12.0) / 2.0;
            let confirm_rect = DipRect::new(CARD_PAD_DIP, 232.0, btn_w, 42.0);
            r.fill_rounded(
                rt,
                confirm_rect,
                9.0,
                if state.hovered == Some(HitTarget::ConfirmUninstall) {
                    danger_hover
                } else {
                    danger
                },
            )?;
            r.draw_text(rt, "Uninstall", &r.fmt_btn, confirm_rect, text)?;

            let cancel_rect = DipRect::new(CARD_PAD_DIP + btn_w + 12.0, 232.0, btn_w, 42.0);
            r.fill_rounded(
                rt,
                cancel_rect,
                9.0,
                if state.hovered == Some(HitTarget::CancelUninstall) {
                    surface_hover
                } else {
                    surface
                },
            )?;
            r.stroke_rounded(rt, cancel_rect, 9.0, 1.0, border)?;
            r.draw_text(rt, "Cancel", &r.fmt_btn_sm, cancel_rect, text_sec)?;
        }
        Phase::Busy {
            uninstalling,
            progress,
            status,
            pulse,
        } => {
            let track = DipRect::new(CARD_PAD_DIP, 160.0, inner_w, 8.0);
            r.fill_rounded(rt, track, 4.0, surface)?;

            let fill_w = (inner_w * progress.clamp(0.06, 1.0)).max(24.0);
            let bar_color = if *uninstalling { danger } else { accent };
            r.fill_rounded(
                rt,
                DipRect::new(CARD_PAD_DIP, 160.0, fill_w, 8.0),
                4.0,
                bar_color,
            )?;

            // Animated highlight traveling along the filled portion of the bar.
            let shimmer_w = (fill_w * 0.28).clamp(18.0, 64.0);
            let shimmer_x = CARD_PAD_DIP + ((fill_w - shimmer_w).max(0.0) * pulse);
            r.fill_rounded(
                rt,
                DipRect::new(shimmer_x, 160.0, shimmer_w.min(fill_w), 8.0),
                4.0,
                rgba(255, 255, 255, 0.28),
            )?;

            r.draw_text(
                rt,
                status,
                &r.fmt_status,
                DipRect::new(CARD_PAD_DIP, 184.0, inner_w, 26.0),
                text,
            )?;
            r.draw_text(
                rt,
                "Please wait — this only takes a few seconds.",
                &r.fmt_subtitle,
                DipRect::new(CARD_PAD_DIP, 212.0, inner_w, 22.0),
                muted,
            )?;
        }
        Phase::Success {
            uninstalled,
            warning,
            ..
        } => {
            let icon_rect = DipRect::new(CARD_PAD_DIP, 126.0, 32.0, 32.0);
            r.fill_rounded(rt, icon_rect, 16.0, accent_soft)?;
            r.stroke_rounded(rt, icon_rect, 16.0, 1.25, accent)?;
            r.draw_text(rt, "✓", &r.fmt_center_icon, icon_rect, accent)?;

            let headline = if *uninstalled {
                "UsageDeck has been uninstalled"
            } else {
                state.action.success_label()
            };
            r.draw_text(
                rt,
                headline,
                &r.fmt_status,
                DipRect::new(CARD_PAD_DIP + 44.0, 129.0, inner_w - 44.0, 26.0),
                text,
            )?;

            let note = warning.as_deref().unwrap_or(if *uninstalled {
                "All selected UsageDeck files and shortcuts have been removed."
            } else {
                "Ready to monitor your AI usage right from the Windows taskbar."
            });
            r.draw_text(
                rt,
                note,
                &r.fmt_wrap,
                DipRect::new(CARD_PAD_DIP, 168.0, inner_w, 40.0),
                if warning.is_some() {
                    rgba(251, 191, 36, 0.95)
                } else {
                    muted
                },
            )?;

            if *uninstalled {
                let close_btn = DipRect::new(CARD_PAD_DIP, 232.0, inner_w, 42.0);
                r.fill_rounded(
                    rt,
                    close_btn,
                    9.0,
                    if state.hovered == Some(HitTarget::SecondaryClose) {
                        surface_hover
                    } else {
                        surface
                    },
                )?;
                r.stroke_rounded(rt, close_btn, 9.0, 1.0, border)?;
                r.draw_text(rt, "Close", &r.fmt_btn, close_btn, text)?;
            } else {
                let launch_btn = DipRect::new(CARD_PAD_DIP, 218.0, inner_w, 44.0);
                r.fill_rounded(
                    rt,
                    launch_btn,
                    9.0,
                    if state.hovered == Some(HitTarget::LaunchApp) {
                        accent_hover
                    } else {
                        accent
                    },
                )?;
                r.draw_text(rt, "Launch UsageDeck", &r.fmt_btn, launch_btn, accent_fg)?;

                let close_btn = DipRect::new(CARD_PAD_DIP, 274.0, inner_w, 36.0);
                r.fill_rounded(
                    rt,
                    close_btn,
                    8.0,
                    if state.hovered == Some(HitTarget::SecondaryClose) {
                        surface_hover
                    } else {
                        surface
                    },
                )?;
                r.stroke_rounded(rt, close_btn, 8.0, 1.0, border)?;
                r.draw_text(rt, "Close", &r.fmt_btn_sm, close_btn, text_sec)?;
            }
        }
        Phase::Error { message, .. } => {
            let icon_rect = DipRect::new(CARD_PAD_DIP, 122.0, 32.0, 32.0);
            r.fill_rounded(rt, icon_rect, 16.0, danger_soft)?;
            r.stroke_rounded(rt, icon_rect, 16.0, 1.25, danger)?;
            r.draw_text(rt, "!", &r.fmt_center_icon, icon_rect, danger_text)?;

            r.draw_text(
                rt,
                "Setup could not complete",
                &r.fmt_status,
                DipRect::new(CARD_PAD_DIP + 44.0, 125.0, inner_w - 44.0, 26.0),
                danger_text,
            )?;

            r.draw_text(
                rt,
                message,
                &r.fmt_wrap,
                DipRect::new(CARD_PAD_DIP, 166.0, inner_w, 72.0),
                text_sec,
            )?;

            let btn_w = (inner_w - 12.0) / 2.0;
            let retry_btn = DipRect::new(CARD_PAD_DIP, 250.0, btn_w, 42.0);
            r.fill_rounded(
                rt,
                retry_btn,
                9.0,
                if state.hovered == Some(HitTarget::Retry) {
                    accent_hover
                } else {
                    accent
                },
            )?;
            r.draw_text(rt, "Try again", &r.fmt_btn, retry_btn, accent_fg)?;

            let close_btn = DipRect::new(CARD_PAD_DIP + btn_w + 12.0, 250.0, btn_w, 42.0);
            r.fill_rounded(
                rt,
                close_btn,
                9.0,
                if state.hovered == Some(HitTarget::SecondaryClose) {
                    surface_hover
                } else {
                    surface
                },
            )?;
            r.stroke_rounded(rt, close_btn, 9.0, 1.0, border)?;
            r.draw_text(rt, "Close", &r.fmt_btn_sm, close_btn, text_sec)?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_checkbox(
    r: &Renderer,
    rt: &ID2D1DCRenderTarget,
    row: DipRect,
    label: &str,
    checked: bool,
    hovered: bool,
    surface: D2D1_COLOR_F,
    surface_hover: D2D1_COLOR_F,
    border: D2D1_COLOR_F,
    checked_bg: D2D1_COLOR_F,
    check_fg: D2D1_COLOR_F,
    text: D2D1_COLOR_F,
) -> Result<()> {
    r.fill_rounded(rt, row, 9.0, if hovered { surface_hover } else { surface })?;
    r.stroke_rounded(rt, row, 9.0, 1.0, border)?;

    let box_size = 18.0;
    let box_rect = DipRect::new(
        row.x + 12.0,
        row.y + (row.h - box_size) / 2.0,
        box_size,
        box_size,
    );
    if checked {
        r.fill_rounded(rt, box_rect, 5.0, checked_bg)?;
        r.draw_text(rt, "✓", &r.fmt_check, box_rect, check_fg)?;
    } else {
        r.fill_rounded(rt, box_rect, 5.0, rgba(24, 24, 27, 0.9))?;
        r.stroke_rounded(rt, box_rect, 5.0, 1.25, rgba(161, 161, 170, 0.7))?;
    }

    r.draw_text(
        rt,
        label,
        &r.fmt_label,
        DipRect::new(row.x + 40.0, row.y, row.w - 52.0, row.h),
        text,
    )?;
    Ok(())
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

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_APP_WORKER => {
            with_state(drain_worker_events);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_ANIM_ID {
                with_state(|s| {
                    if let Phase::Busy { pulse, .. } = &mut s.phase {
                        *pulse = (*pulse + 0.035) % 1.0;
                        paint_window(s);
                    }
                });
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            with_state(|s| {
                if !s.tracking_leave {
                    track_mouse_leave(hwnd);
                    s.tracking_leave = true;
                }
                let (dx, dy) = cursor_dip(lparam, s.scale());
                let hit = s.hit_test(dx, dy);
                if hit != s.hovered {
                    s.hovered = hit;
                    paint_window(s);
                }
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_state(|s| {
                s.tracking_leave = false;
                if s.hovered.is_some() {
                    s.hovered = None;
                    paint_window(s);
                }
            });
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xffff) == 1 {
                let cursor = with_state(|s| {
                    if s.hovered.is_some() {
                        s.hand_cursor
                    } else {
                        s.arrow_cursor
                    }
                });
                if let Some(c) = cursor {
                    SetCursor(Some(c));
                    return LRESULT(1);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_LBUTTONDOWN => {
            let on_control = with_state(|s| {
                let (dx, dy) = cursor_dip(lparam, s.scale());
                s.hit_test(dx, dy).is_some()
            })
            .unwrap_or(false);
            if !on_control {
                let _ = ReleaseCapture();
                return DefWindowProcW(
                    hwnd,
                    WM_NCLBUTTONDOWN,
                    WPARAM(HTCAPTION as usize),
                    LPARAM(0),
                );
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            with_state(|s| {
                let (dx, dy) = cursor_dip(lparam, s.scale());
                if let Some(target) = s.hit_test(dx, dy) {
                    handle_click(s, target);
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0;
            with_state(|s| {
                if vk == usize::from(VK_ESCAPE.0) {
                    if matches!(s.phase, Phase::ConfirmUninstall) {
                        s.phase = Phase::Idle;
                        paint_window(s);
                    } else {
                        close_window(s);
                    }
                } else if vk == usize::from(VK_RETURN.0) || vk == usize::from(VK_SPACE.0) {
                    match &s.phase {
                        Phase::Idle => handle_click(s, HitTarget::PrimaryAction),
                        Phase::Success {
                            uninstalled: false, ..
                        } => handle_click(s, HitTarget::LaunchApp),
                        Phase::Success {
                            uninstalled: true, ..
                        } => handle_click(s, HitTarget::SecondaryClose),
                        Phase::Error { .. } => handle_click(s, HitTarget::Retry),
                        _ => {}
                    }
                }
            });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let new_dpi = (wparam.0 & 0xffff) as u32;
            let suggested_ptr = lparam.0 as *const RECT;
            let suggested = (!suggested_ptr.is_null()).then(|| *suggested_ptr);
            with_state(|s| {
                if new_dpi >= 96 {
                    s.dpi = new_dpi;
                }
                if let Some(rect) = suggested {
                    s.win_x = rect.left;
                    s.win_y = rect.top;
                }
                paint_window(s);
            });
            LRESULT(0)
        }
        WM_CLOSE => {
            let busy = with_state(|s| s.is_busy()).unwrap_or(false);
            if !busy {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), TIMER_ANIM_ID);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
