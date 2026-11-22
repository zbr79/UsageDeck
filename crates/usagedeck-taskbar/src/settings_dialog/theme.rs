//! Theme palette, Windows dark-mode detection, DWM titlebar styling, and
//! number/duration formatting helpers for the native UsageDeck Settings window.
//!
//! Color tokens faithfully reproduce the v1.5.15 WinUI-inspired dark and light
//! palettes from `src/settings/App.tsx` and `src/ui/palette.ts`.

use usagedeck_core::store::PanelTheme;
use usagedeck_core::view::{BarSeverity, SettingsPaceData};
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeTone {
    Blue,
    Green,
    Violet,
    Amber,
}

#[derive(Clone, Copy)]
pub struct BadgeColors {
    pub bg: D2D1_COLOR_F,
    pub fg: D2D1_COLOR_F,
    pub border: D2D1_COLOR_F,
}

#[derive(Clone, Copy)]
pub struct SettingsPalette {
    pub is_light: bool,
    pub page: D2D1_COLOR_F,
    pub card: D2D1_COLOR_F,
    pub card_border: D2D1_COLOR_F,
    pub nav_hover: D2D1_COLOR_F,
    pub nav_active: D2D1_COLOR_F,
    pub accent: D2D1_COLOR_F,
    pub switch_on: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub text_secondary: D2D1_COLOR_F,
    pub muted: D2D1_COLOR_F,
    pub surface: D2D1_COLOR_F,
    pub surface_hover: D2D1_COLOR_F,
    pub border: D2D1_COLOR_F,
    pub input_border: D2D1_COLOR_F,
    pub primary_bg: D2D1_COLOR_F,
    pub primary_hover: D2D1_COLOR_F,
    pub primary_fg: D2D1_COLOR_F,
    pub bar_ok: D2D1_COLOR_F,
    pub bar_warn: D2D1_COLOR_F,
    pub bar_danger: D2D1_COLOR_F,
    pub bar_dim: D2D1_COLOR_F,
    pub track: D2D1_COLOR_F,
    pub warn_text: D2D1_COLOR_F,
    pub chart_all: D2D1_COLOR_F,
    pub chart_fable: D2D1_COLOR_F,
    pub chart_all_rgb: (u8, u8, u8),
    pub chart_fable_rgb: (u8, u8, u8),
    pub scrollbar_thumb: D2D1_COLOR_F,
    pub scrollbar_thumb_hover: D2D1_COLOR_F,
    pub toast_bg: D2D1_COLOR_F,
    pub toast_border: D2D1_COLOR_F,
}

pub const fn rgb(r: u8, g: u8, b: u8) -> D2D1_COLOR_F {
    rgba(r, g, b, 1.0)
}

pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

pub fn with_alpha(c: D2D1_COLOR_F, alpha_scale: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.r,
        g: c.g,
        b: c.b,
        a: (c.a * alpha_scale).clamp(0.0, 1.0),
    }
}

impl SettingsPalette {
    pub fn for_mode(is_dark: bool) -> Self {
        if is_dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    pub const fn dark() -> Self {
        Self {
            is_light: false,
            page: rgb(0x20, 0x20, 0x20),
            card: rgb(0x2b, 0x2b, 0x2b),
            card_border: rgb(0x1d, 0x1d, 0x1d),
            nav_hover: rgba(255, 255, 255, 0.06),
            nav_active: rgba(255, 255, 255, 0.09),
            accent: rgb(0x00, 0xd4, 0x92),
            switch_on: rgb(0x00, 0x99, 0x66),
            text: rgb(0xf4, 0xf4, 0xf5),
            text_secondary: rgb(0xd4, 0xd4, 0xd8),
            muted: rgb(0x9f, 0x9f, 0xa9),
            surface: rgb(0x27, 0x27, 0x2a),
            surface_hover: rgb(0x3f, 0x3f, 0x46),
            border: rgb(0x3f, 0x3f, 0x46),
            input_border: rgb(0x71, 0x71, 0x7b),
            primary_bg: rgb(0xe4, 0xe4, 0xe7),
            primary_hover: rgb(0xd4, 0xd4, 0xd8),
            primary_fg: rgb(0x18, 0x18, 0x1b),
            bar_ok: rgb(0x00, 0xd4, 0x92),
            bar_warn: rgb(0xff, 0xb9, 0x00),
            bar_danger: rgb(0xff, 0x64, 0x67),
            bar_dim: rgb(0x71, 0x71, 0x7b),
            track: rgba(255, 255, 255, 0.15),
            warn_text: rgb(0xff, 0xd2, 0x30),
            chart_all: rgb(0x8e, 0xc5, 0xff),
            chart_fable: rgb(0xff, 0xd2, 0x30),
            chart_all_rgb: (0x8e, 0xc5, 0xff),
            chart_fable_rgb: (0xff, 0xd2, 0x30),
            scrollbar_thumb: rgba(255, 255, 255, 0.22),
            scrollbar_thumb_hover: rgba(255, 255, 255, 0.36),
            toast_bg: rgb(0x27, 0x27, 0x2a),
            toast_border: rgb(0x52, 0x52, 0x5b),
        }
    }

    pub const fn light() -> Self {
        Self {
            is_light: true,
            page: rgb(0xf3, 0xf3, 0xf3),
            card: rgb(0xfb, 0xfb, 0xfb),
            card_border: rgb(0xe5, 0xe5, 0xe5),
            nav_hover: rgba(0, 0, 0, 0.04),
            nav_active: rgba(0, 0, 0, 0.06),
            accent: rgb(0x00, 0x99, 0x66),
            switch_on: rgb(0x00, 0x99, 0x66),
            text: rgb(0x18, 0x18, 0x1b),
            text_secondary: rgb(0x3f, 0x3f, 0x46),
            muted: rgb(0x52, 0x52, 0x5c),
            surface: rgb(0xf4, 0xf4, 0xf5),
            surface_hover: rgb(0xe4, 0xe4, 0xe7),
            border: rgb(0xd4, 0xd4, 0xd8),
            input_border: rgb(0x71, 0x71, 0x7b),
            primary_bg: rgb(0x18, 0x18, 0x1b),
            primary_hover: rgb(0x3f, 0x3f, 0x46),
            primary_fg: rgb(0xff, 0xff, 0xff),
            bar_ok: rgb(0x00, 0x7a, 0x55),
            bar_warn: rgb(0xbb, 0x4d, 0x00),
            bar_danger: rgb(0xc1, 0x00, 0x07),
            bar_dim: rgb(0x71, 0x71, 0x7b),
            track: rgba(113, 113, 123, 0.20),
            warn_text: rgb(0x97, 0x3c, 0x00),
            chart_all: rgb(0x19, 0x3c, 0xb8),
            chart_fable: rgb(0x97, 0x3c, 0x00),
            chart_all_rgb: (0x19, 0x3c, 0xb8),
            chart_fable_rgb: (0x97, 0x3c, 0x00),
            scrollbar_thumb: rgba(0, 0, 0, 0.22),
            scrollbar_thumb_hover: rgba(0, 0, 0, 0.36),
            toast_bg: rgb(0x18, 0x18, 0x1b),
            toast_border: rgb(0x3f, 0x3f, 0x46),
        }
    }

    pub fn bar_color(&self, sev: BarSeverity, dimmed: bool) -> D2D1_COLOR_F {
        if dimmed {
            return self.bar_dim;
        }
        match sev {
            BarSeverity::Ok => self.bar_ok,
            BarSeverity::Warn => self.bar_warn,
            BarSeverity::Danger => self.bar_danger,
            BarSeverity::Muted => self.bar_dim,
        }
    }

    pub fn badge(&self, tone: BadgeTone) -> BadgeColors {
        if self.is_light {
            match tone {
                BadgeTone::Blue => BadgeColors {
                    bg: rgb(0xdb, 0xea, 0xfe),
                    fg: rgb(0x19, 0x3c, 0xb8),
                    border: rgb(0x8e, 0xc5, 0xff),
                },
                BadgeTone::Green => BadgeColors {
                    bg: rgb(0xd0, 0xfa, 0xe5),
                    fg: rgb(0x00, 0x60, 0x45),
                    border: rgb(0x5e, 0xe9, 0xb5),
                },
                BadgeTone::Violet => BadgeColors {
                    bg: rgb(0xed, 0xe9, 0xfe),
                    fg: rgb(0x5d, 0x0e, 0xc0),
                    border: rgb(0xc4, 0xb4, 0xff),
                },
                BadgeTone::Amber => BadgeColors {
                    bg: rgb(0xfe, 0xf3, 0xc6),
                    fg: rgb(0x97, 0x3c, 0x00),
                    border: rgb(0xff, 0xd2, 0x30),
                },
            }
        } else {
            match tone {
                BadgeTone::Blue => BadgeColors {
                    bg: rgba(59, 130, 246, 0.15),
                    fg: rgb(0x8e, 0xc5, 0xff),
                    border: rgba(96, 165, 250, 0.30),
                },
                BadgeTone::Green => BadgeColors {
                    bg: rgba(16, 185, 129, 0.15),
                    fg: rgb(0x5e, 0xe9, 0xb5),
                    border: rgba(52, 211, 153, 0.30),
                },
                BadgeTone::Violet => BadgeColors {
                    bg: rgba(139, 92, 246, 0.15),
                    fg: rgb(0xc4, 0xb4, 0xff),
                    border: rgba(167, 139, 250, 0.30),
                },
                BadgeTone::Amber => BadgeColors {
                    bg: rgba(245, 158, 11, 0.15),
                    fg: rgb(0xff, 0xd2, 0x30),
                    border: rgba(251, 191, 36, 0.30),
                },
            }
        }
    }
}

pub fn plan_badge_tone(plan: &str) -> BadgeTone {
    let lower = plan.to_ascii_lowercase();
    if lower == "free" || lower == "go" {
        BadgeTone::Green
    } else if lower.contains("ultra") || lower.contains("max") {
        BadgeTone::Amber
    } else if lower.contains("team") || lower.contains("enterprise") {
        BadgeTone::Violet
    } else {
        BadgeTone::Blue
    }
}

/// Read Windows' app light/dark preference (`AppsUseLightTheme == 1` -> light).
pub fn system_prefers_dark() -> bool {
    let mut val: u32 = 0;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: plain registry query into stack-owned `u32`.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut val as *mut u32).cast::<std::ffi::c_void>()),
            Some(&mut size),
        )
    };
    !(status.0 == 0 && val == 1)
}

pub fn resolve_is_dark(panel_theme: PanelTheme) -> bool {
    match panel_theme {
        PanelTheme::Dark => true,
        PanelTheme::Light => false,
        PanelTheme::System => system_prefers_dark(),
    }
}

/// Apply native dark or light caption bar via DWM (`DWMWA_USE_IMMERSIVE_DARK_MODE`).
pub fn apply_titlebar_theme(hwnd: HWND, is_dark: bool) {
    if hwnd.is_invalid() {
        return;
    }
    let dark_val: i32 = i32::from(is_dark);
    // SAFETY: `dark_val` is a valid 4-byte Win32 `BOOL` for `DWMWA_USE_IMMERSIVE_DARK_MODE`.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark_val as *const i32).cast::<std::ffi::c_void>(),
            std::mem::size_of::<i32>() as u32,
        );
    }
}

// ---- Formatting & spend chart helpers -------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendRange {
    Min15,
    Hour1,
    Hours24,
}

impl SpendRange {
    pub fn title(self) -> &'static str {
        match self {
            Self::Min15 => "Last 15 minutes",
            Self::Hour1 => "Last hour",
            Self::Hours24 => "Last 24 hours",
        }
    }

    pub fn left_axis_label(self) -> &'static str {
        match self {
            Self::Min15 => "15 min ago",
            Self::Hour1 => "1 h ago",
            Self::Hours24 => "24 h ago",
        }
    }
}

/// Format dollars like v1.5.15: one decimal under `$10`, comma-grouped whole dollars otherwise.
pub fn format_usd(v: f64) -> String {
    if !v.is_finite() {
        return "$0".to_string();
    }
    let sign = if v < 0.0 { "-" } else { "" };
    let abs = v.abs();
    if abs < 10.0 {
        let s = format!("{abs:.1}");
        if s == "10.0" {
            return format!("{sign}$10");
        }
        return format!("{sign}${s}");
    }
    let rounded = abs.round() as u64;
    format!("{sign}${}", format_u64_commas(rounded))
}

fn format_u64_commas(n: u64) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn format_usd_rate(v: f64) -> String {
    format!("{}/h", format_usd(v))
}

/// Format a model share (handles either `0.0..=1.0` fraction or `0.0..=100.0` percent cleanly).
pub fn format_share_pct(share: f64) -> String {
    if !share.is_finite() || share <= 0.0 {
        return "0%".to_string();
    }
    let pct = if share <= 1.0 { share * 100.0 } else { share };
    if pct < 1.0 {
        format!("{pct:.1}%")
    } else {
        format!("{}%", pct.round() as u64)
    }
}

pub fn calibration_text(usd_per_weekly_pct: Option<f64>) -> String {
    match usd_per_weekly_pct {
        Some(v) if v.is_finite() && v > 0.0 => {
            format!("1% of your weekly limit ≈ {} of usage", format_usd(v))
        }
        _ => "Not measured yet".to_string(),
    }
}

/// Format remaining snooze duration like v1.5.15 (`"45m"`, `"2h 14m"`, `"3d 4h"`).
pub fn format_snooze_duration(remaining_ms: u64) -> String {
    let total_mins = remaining_ms.saturating_add(30_000) / 60_000;
    if total_mins < 60 {
        return format!("{}m", total_mins.max(1));
    }
    let hours = total_mins / 60;
    let mins = total_mins % 60;
    if hours < 24 {
        if mins == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {mins}m")
        }
    } else {
        let days = hours / 24;
        let rem_h = hours % 24;
        if rem_h == 0 {
            format!("{days}d")
        } else {
            format!("{days}d {rem_h}h")
        }
    }
}

/// Convert `SettingsPaceData` buckets (newest-first USD per bucket) into oldest-first
/// `(all_usd_per_hour, fable_usd_per_hour)` series points for the selected chart range.
pub fn spend_series(pace: &SettingsPaceData, range: SpendRange) -> Vec<(f64, f64)> {
    let (values, fable, count, per_hour) = match range {
        SpendRange::Min15 => (&pace.minutes, &pace.minutes_fable, 15usize, 60.0),
        SpendRange::Hour1 => (&pace.minutes, &pace.minutes_fable, pace.minutes.len(), 60.0),
        SpendRange::Hours24 => (
            &pace.quarters,
            &pace.quarters_fable,
            pace.quarters.len(),
            4.0,
        ),
    };
    let take = count.min(values.len());
    let mut pts: Vec<(f64, f64)> = (0..take)
        .map(|i| {
            let total = (values[i] * per_hour).max(0.0);
            let fab = (fable.get(i).copied().unwrap_or(0.0) * per_hour).max(0.0);
            (total, fab)
        })
        .collect();
    pts.reverse();
    pts
}
