//! Windows 10/11 taskbar geometry, theme, DPI, and placement helpers.
//!
//! Locates `Shell_TrayWnd` and `TrayNotifyWnd` without injecting into `explorer.exe`,
//! detects taskbar light/dark theme from the registry, checks auto-hide visibility,
//! and computes exact physical-pixel coordinates for the taskbar strip and flyout.

use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowRect, IsWindowVisible,
};

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

/// Read Windows' taskbar theme (`SystemUsesLightTheme == 1` -> Light, else Dark).
pub fn read_taskbar_theme() -> TaskbarTheme {
    let mut val: u32 = 0;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: plain registry read into stack-owned `u32`.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut u32 as *mut std::ffi::c_void),
            Some(&mut size),
        )
    };
    if status.0 == 0 && val == 1 {
        TaskbarTheme::Light
    } else {
        TaskbarTheme::Dark
    }
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
        let notify_hwnd = FindWindowExW(Some(tray_hwnd), None, w!("TrayNotifyWnd"), None).ok()?;
        if notify_hwnd.is_invalid() {
            return None;
        }

        let mut tray_rect = RECT::default();
        GetWindowRect(tray_hwnd, &mut tray_rect).ok()?;
        let mut notify_rect = RECT::default();
        GetWindowRect(notify_hwnd, &mut notify_rect).ok()?;

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
        // On Windows 10, `ReBarWindow32` / `MSTaskSwWClass` gives the exact task band rect.
        // On Windows 11, the XAML island spans the taskbar, so we reserve a generous safe zone
        // for the centered (or left-aligned) app icons while capping cleanly on ultrawide monitors.
        let tray_w = (tray_rect.right - tray_rect.left).max(0);
        let right_edge = notify_rect.left - (8.0 * scale).round() as i32;
        let left_bound = if is_taskbar_centered() {
            // Leave the center 28% of the taskbar clear for Start + pinned/open apps.
            tray_rect.left + ((tray_w as f32) * 0.64) as i32
        } else {
            // Left-aligned taskbar: reserve left 42% for Start, search, and open windows.
            tray_rect.left + ((tray_w as f32) * 0.42) as i32
        };

        let available_px = (right_edge - left_bound).max(0);
        let available_width_dip = (available_px as f32 / scale).min(520.0);

        Some(TaskbarMetrics {
            tray_hwnd,
            tray_rect,
            notify_rect,
            monitor_rect,
            dpi,
            scale,
            theme: read_taskbar_theme(),
            hidden,
            available_width_dip,
        })
    }
}
