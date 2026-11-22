//! Windows 10/11 taskbar geometry, theme, DPI, and placement helpers.
//!
//! Locates `Shell_TrayWnd` and `TrayNotifyWnd` without injecting into `explorer.exe`,
//! detects taskbar light/dark theme via live pixel luminance sampling (with registry
//! fallback), checks auto-hide visibility, and computes exact physical-pixel coordinates
//! for the taskbar strip and flyout.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetDC, GetMonitorInfoW, GetPixel, MonitorFromWindow, ReleaseDC, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowRect, IsWindowVisible,
};

const CLR_INVALID_RAW: u32 = 0xFFFF_FFFF;
const LUMINANCE_LIGHT_THRESHOLD: f32 = 135.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskbarTheme {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskbarMetrics {
    pub tray_hwnd: HWND,
    pub tray_rect: RECT,
    pub notify_rect: RECT,
    pub monitor_rect: RECT,
    pub dpi: u32,
    pub scale: f32,
    pub theme: TaskbarTheme,
    /// True when the taskbar is hidden off-screen (auto-hide collapsed) or invisible.
    pub hidden: bool,
    /// Available horizontal DIP width to the left of `TrayNotifyWnd` before crowding app icons.
    pub available_width_dip: f32,
}

impl TaskbarMetrics {
    /// Physical-pixel `(x, y)` for a taskbar strip of `(width_px, height_px)` anchored
    /// directly to the left of `TrayNotifyWnd`, vertically centered inside `Shell_TrayWnd`.
    pub fn strip_origin_px(&self, width_px: i32, height_px: i32) -> (i32, i32) {
        let gap_px = (6.0 * self.scale).round() as i32;
        let x = (self.notify_rect.left - gap_px - width_px).max(self.tray_rect.left + 4);
        let tray_h = self.tray_rect.bottom - self.tray_rect.top;
        let y = self.tray_rect.top + (tray_h - height_px) / 2;
        (x, y)
    }

    /// Physical-pixel `(x, y)` for an anchored flyout directly above the right-hand taskbar strip,
    /// clamped inside the taskbar's monitor.
    pub fn flyout_origin_px(
        &self,
        strip_x: i32,
        strip_w: i32,
        flyout_w: i32,
        flyout_h: i32,
    ) -> (i32, i32) {
        let margin = (10.0 * self.scale).round() as i32;
        let ideal_right = (strip_x + strip_w).min(self.monitor_rect.right - margin);
        let x = (ideal_right - flyout_w)
            .max(self.monitor_rect.left + margin)
            .min(self.monitor_rect.right - flyout_w - margin);
        let y = (self.tray_rect.top - margin - flyout_h).max(self.monitor_rect.top + margin);
        (x, y)
    }
}

fn read_personalize_dword(value_name: PCWSTR) -> Option<u32> {
    let mut val: u32 = 0;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: plain registry read into stack-owned `u32`.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            value_name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut u32 as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    if status.0 == 0 {
        Some(val)
    } else {
        None
    }
}

/// Read Windows' taskbar theme from registry (`ColorPrevalence == 1` -> Dark;
/// otherwise `SystemUsesLightTheme == 1` -> Light, else Dark).
pub fn read_taskbar_theme() -> TaskbarTheme {
    if read_personalize_dword(w!("ColorPrevalence")) == Some(1) {
        return TaskbarTheme::Dark;
    }
    if read_personalize_dword(w!("SystemUsesLightTheme")) == Some(1) {
        TaskbarTheme::Light
    } else {
        TaskbarTheme::Dark
    }
}

/// Classify raw GDI `COLORREF` (`0x00BBGGRR`) samples into Dark/Light by median perceived
/// luminance (`0.299*R + 0.587*G + 0.114*B`). Using the median prevents a single bright icon or
/// hairline border pixel from flipping a dark taskbar into Light mode.
fn classify_pixel_samples(samples: &[u32]) -> Option<TaskbarTheme> {
    let mut lums: Vec<f32> = samples
        .iter()
        .copied()
        .filter(|&c| c != CLR_INVALID_RAW)
        .map(|c| {
            let r = (c & 0xFF) as f32;
            let g = ((c >> 8) & 0xFF) as f32;
            let b = ((c >> 16) & 0xFF) as f32;
            0.299 * r + 0.587 * g + 0.114 * b
        })
        .collect();
    if lums.is_empty() {
        return None;
    }
    lums.sort_by(|a, b| a.total_cmp(b));
    let median_lum = lums[lums.len() / 2];
    if median_lum < LUMINANCE_LIGHT_THRESHOLD {
        Some(TaskbarTheme::Dark)
    } else {
        Some(TaskbarTheme::Light)
    }
}

/// Detect whether the Windows taskbar surface is Dark or Light.
///
/// First respects Windows' own taskbar icon polarity (`read_taskbar_theme`): whenever
/// `SystemUsesLightTheme == 0` or `ColorPrevalence == 1`, Windows itself renders white icons
/// (`^`, clock, tray), so we must always use `TaskbarTheme::Dark` and never let a stray bright
/// pixel (such as the Widgets/Weather tile or top border highlight) flip the strip into unreadable
/// dark-on-dark text. When Windows reports Light, we also sample pixels near `TrayNotifyWnd` so
/// dark TranslucentTB / Mica / wallpaper-tinted taskbars still automatically pick Dark mode.
pub fn sample_taskbar_theme(tray_rect: RECT, notify_rect: RECT) -> TaskbarTheme {
    if read_taskbar_theme() == TaskbarTheme::Dark {
        return TaskbarTheme::Dark;
    }
    if tray_rect.right <= tray_rect.left || tray_rect.bottom <= tray_rect.top {
        return TaskbarTheme::Light;
    }
    let min_x = tray_rect.left;
    let max_x = tray_rect.right - 1;
    let y_top = (tray_rect.top + 4).min(tray_rect.bottom - 1);
    let y_bot = (tray_rect.bottom - 4).max(tray_rect.top);
    let xs = [
        (notify_rect.left - 2).clamp(min_x, max_x),
        (notify_rect.left - 5).clamp(min_x, max_x),
        (notify_rect.left - 9).clamp(min_x, max_x),
    ];

    // SAFETY: read-only screen DC pixel sampling balanced with `ReleaseDC`.
    let samples = unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return TaskbarTheme::Light;
        }
        let pixels = [
            GetPixel(screen_dc, xs[0], y_top).0,
            GetPixel(screen_dc, xs[1], y_top).0,
            GetPixel(screen_dc, xs[2], y_top).0,
            GetPixel(screen_dc, xs[0], y_bot).0,
            GetPixel(screen_dc, xs[1], y_bot).0,
        ];
        let _ = ReleaseDC(None, screen_dc);
        pixels
    };

    classify_pixel_samples(&samples).unwrap_or(TaskbarTheme::Light)
}

/// Check if Windows 11 taskbar alignment is set to Center (`TaskbarAl == 1`, default on Win11).
fn is_taskbar_centered() -> bool {
    let mut val: u32 = 1;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: plain registry read into stack-owned `u32`.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"),
            w!("TaskbarAl"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut u32 as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    if status.0 == 0 {
        val != 0
    } else {
        true
    }
}

/// Query current primary taskbar geometry, visibility, DPI, theme, and safe available width.
pub fn query_taskbar() -> Option<TaskbarMetrics> {
    // SAFETY: Win32 read-only queries against standard shell window handles.
    unsafe {
        let tray_hwnd = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        if tray_hwnd.is_invalid() {
            return None;
        }

        let mut tray_rect = RECT::default();
        GetWindowRect(tray_hwnd, &mut tray_rect).ok()?;

        let mon = MonitorFromWindow(tray_hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mon_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor_rect = if GetMonitorInfoW(mon, &mut mon_info).as_bool() {
            mon_info.rcMonitor
        } else {
            tray_rect
        };

        let raw_dpi = GetDpiForWindow(tray_hwnd);
        let dpi = if raw_dpi == 0 { 96 } else { raw_dpi };
        let scale = dpi as f32 / 96.0;

        // Locate `TrayNotifyWnd` (clock + system tray). If a future Windows build renames or
        // hides it, fall back to a conservative right-hand reserve so the strip still renders.
        let mut notify_rect = RECT {
            left: (tray_rect.right - (220.0 * scale).round() as i32).max(tray_rect.left),
            top: tray_rect.top,
            right: tray_rect.right,
            bottom: tray_rect.bottom,
        };
        if let Ok(notify_hwnd) = FindWindowExW(Some(tray_hwnd), None, w!("TrayNotifyWnd"), None) {
            if !notify_hwnd.is_invalid() {
                let mut candidate = RECT::default();
                if GetWindowRect(notify_hwnd, &mut candidate).is_ok()
                    && candidate.right > candidate.left
                    && candidate.left > tray_rect.left
                {
                    notify_rect = candidate;
                }
            }
        }

        // Detect auto-hide or off-screen collapsed taskbar.
        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        let state = SHAppBarMessage(ABM_GETSTATE, &mut abd);
        let auto_hide = (state & ABS_AUTOHIDE as usize) != 0;
        let tray_h = tray_rect.bottom - tray_rect.top;
        let visible_h = (tray_rect.bottom.min(monitor_rect.bottom)
            - tray_rect.top.max(monitor_rect.top))
        .max(0);
        let hidden = !IsWindowVisible(tray_hwnd).as_bool()
            || tray_h <= 4
            || (auto_hide && visible_h < (tray_h / 2));

        // Measure free width to the left of `TrayNotifyWnd`.
        // On Windows 11, the XAML island spans the taskbar, so we reserve a safe zone for the
        // centered (or left-aligned) app icons while leaving enough room for wide side-by-side meters.
        let tray_w = (tray_rect.right - tray_rect.left).max(0);
        let right_edge = notify_rect.left - (8.0 * scale).round() as i32;
        let left_bound = if is_taskbar_centered() {
            tray_rect.left + ((tray_w as f32) * 0.42) as i32
        } else {
            tray_rect.left + ((tray_w as f32) * 0.30) as i32
        };

        let raw_available_px = (right_edge - left_bound).max(0);
        let mut available_width_dip = (raw_available_px as f32 / scale).min(860.0);
        // Never collapse to Hidden if there is clearly space left of `TrayNotifyWnd` for at
        // least the compact summary pill (e.g. narrow laptops at 175%-200% DPI).
        let total_left_of_notify_dip = (right_edge - tray_rect.left).max(0) as f32 / scale;
        if available_width_dip < 72.0 && total_left_of_notify_dip >= 140.0 {
            available_width_dip = 72.0;
        }

        let theme = if hidden {
            read_taskbar_theme()
        } else {
            sample_taskbar_theme(tray_rect, notify_rect)
        };

        Some(TaskbarMetrics {
            tray_hwnd,
            tray_rect,
            notify_rect,
            monitor_rect,
            dpi,
            scale,
            theme,
            hidden,
            available_width_dip,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colorref(r: u8, g: u8, b: u8) -> u32 {
        u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
    }

    #[test]
    fn classifies_dark_burgundy_mica_as_dark() {
        let samples = [
            colorref(62, 18, 28),
            colorref(58, 16, 26),
            colorref(64, 20, 30),
        ];
        assert_eq!(classify_pixel_samples(&samples), Some(TaskbarTheme::Dark));
    }

    #[test]
    fn classifies_light_taskbar_as_light() {
        let samples = [
            colorref(238, 242, 247),
            colorref(236, 240, 245),
            colorref(240, 244, 249),
        ];
        assert_eq!(classify_pixel_samples(&samples), Some(TaskbarTheme::Light));
    }

    #[test]
    fn ignores_invalid_samples_and_returns_none_when_all_invalid() {
        assert_eq!(
            classify_pixel_samples(&[CLR_INVALID_RAW, CLR_INVALID_RAW, CLR_INVALID_RAW]),
            None
        );
        assert_eq!(
            classify_pixel_samples(&[CLR_INVALID_RAW, colorref(25, 25, 28), CLR_INVALID_RAW]),
            Some(TaskbarTheme::Dark)
        );
    }
}
