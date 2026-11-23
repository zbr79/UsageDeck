//! Pure-Rust view model for the Windows taskbar strip, anchored flyout, and Settings window.
//!
//! Converts raw provider responses + Claude Code spend readings into compact,
//! ready-to-render `TaskbarSnapshot` data (Variant A: icon + up to 2 bars + %
//! on the taskbar, with Claude's live `$/h` rate, Fable shading, per-model tooltips,
//! and full bar details in the flyout), plus `SettingsSnapshot` for all 7 Settings pages.

use crate::claude_logs::{self, ModelShare, ModelUsd};
use crate::history;
use crate::providers::antigravity::AntigravityUsage;
use crate::providers::claude::{ClaudeScopedWindow, ClaudeUsage};
use crate::providers::claude_resets::{ClaudeResetGroup, ClaudeResetKind, ClaudeResets};
use crate::providers::codex::CodexUsage;
use crate::providers::cursor_auth::redact;
use crate::providers::cursor_diag::{CursorAttempt, DbHow};
use crate::providers::cursor_usage::{CursorUsage, ModelSpend as CursorModelSpend, OnDemand};
use crate::providers::grok::GrokUsage;
use crate::providers::opencode::{OpenCodeUsage, UsageWindow};
use crate::store::{Settings, TaskbarMeterStyle, ThemeMode, WidgetProvider};
use crate::Core;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt::Display;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    Claude,
    Cursor,
    Codex,
    Grok,
    OpenCode,
    Antigravity,
}

impl ProviderKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Cursor => "Cursor",
            Self::Codex => "Codex",
            Self::Grok => "Grok",
            Self::OpenCode => "OpenCode",
            Self::Antigravity => "Antigravity",
        }
    }

    pub fn key_prefix(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::Antigravity => "antigravity",
        }
    }

    pub fn cache_section(self) -> &'static str {
        match self {
            Self::Grok => "grokbot",
            _ => self.key_prefix(),
        }
    }

    pub fn from_widget_provider(wp: WidgetProvider) -> Self {
        match wp {
            WidgetProvider::Claude => Self::Claude,
            WidgetProvider::Cursor => Self::Cursor,
            WidgetProvider::Codex => Self::Codex,
            WidgetProvider::Grok => Self::Grok,
            WidgetProvider::Opencode => Self::OpenCode,
            WidgetProvider::Antigravity => Self::Antigravity,
        }
    }

    pub fn to_widget_provider(self) -> WidgetProvider {
        match self {
            Self::Claude => WidgetProvider::Claude,
            Self::Cursor => WidgetProvider::Cursor,
            Self::Codex => WidgetProvider::Codex,
            Self::Grok => WidgetProvider::Grok,
            Self::OpenCode => WidgetProvider::Opencode,
            Self::Antigravity => WidgetProvider::Antigravity,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarSeverity {
    Ok,
    Warn,
    Danger,
    Muted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarView {
    pub key: String,
    pub label: String,
    pub percent: Option<f64>,
    pub resets_at: Option<String>,
    pub reset_short: Option<String>,
    /// Local wall-clock reset label (e.g. `"Today 1:30 AM"`, `"Tomorrow 9:00 AM"`, `"Thu 9:00 PM"`).
    pub reset_at_local: Option<String>,
    /// Expected linear time-elapsed percentage (`0.0..=100.0`) inside the bar's window.
    pub pace_pct: Option<f64>,
    pub severity: BarSeverity,
    /// Fable's portion of the bar in percentage points (0..=percent), drawn in a lighter shade.
    pub fable_segment_pct: Option<f64>,
    /// Per-model breakdown lines for bar hover tooltips (e.g. `"Opus 4.5 ≈54 pts"`).
    pub tooltip_lines: Vec<String>,
}

impl BarView {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        percent: Option<f64>,
        resets_at: Option<String>,
        now_ms: i64,
        dimmed: bool,
    ) -> Self {
        let key = key.into();
        let passed = resets_at
            .as_deref()
            .and_then(parse_iso_ms)
            .is_some_and(|ms| ms <= now_ms);
        let effective_pct = if passed {
            None
        } else {
            percent
                .filter(|p| p.is_finite())
                .map(|p| p.clamp(0.0, 100.0))
        };
        let reset_short = if passed {
            None
        } else {
            resets_at
                .as_deref()
                .and_then(|iso| format_reset_short(iso, now_ms))
        };
        let reset_at_local = if passed {
            None
        } else {
            resets_at
                .as_deref()
                .and_then(|iso| format_reset_local(iso, now_ms))
        };
        let pace_pct = if passed {
            None
        } else {
            compute_pace_pct(&key, resets_at.as_deref(), now_ms)
        };
        let severity = if dimmed {
            BarSeverity::Muted
        } else {
            severity_of(effective_pct)
        };
        Self {
            key,
            label: label.into(),
            percent: effective_pct,
            resets_at,
            reset_short,
            reset_at_local,
            pace_pct,
            severity,
            fable_segment_pct: None,
            tooltip_lines: Vec::new(),
        }
    }

    pub fn percent_label(&self) -> String {
        match self.percent {
            Some(p) => {
                let rounded = p.round() as i32;
                if rounded >= 100 {
                    "100".to_string()
                } else {
                    format!("{rounded}%")
                }
            }
            None => "--".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderItem {
    pub provider: ProviderKind,
    pub account_id: String,
    pub title: String,
    /// Strictly 1 or 2 bars rendered inline on the Windows taskbar (Variant A).
    pub taskbar_bars: Vec<BarView>,
    /// All visible bars rendered inside the click flyout.
    pub flyout_bars: Vec<BarView>,
    /// All non-empty raw bars for this provider (including bars hidden from the compact strip).
    pub hover_bars: Vec<BarView>,
    /// Structured `(label, value)` detail rows rendered inside the hover card.
    pub hover_details: Vec<(String, String)>,
    /// Compact badge showing resets in hand (e.g. `Some("↺ 2")` for Claude).
    pub resets_badge: Option<String>,
    /// Exact future epoch-ms timestamp when an exhausted card resets.
    pub exhausted_until_ms: Option<u64>,
    /// Inline live spend rate (currently Claude Code `$/h`, e.g. `"$42/h"`).
    pub spend_rate_text: Option<String>,
    /// Severity colour of `spend_rate_text` against `settings.claude_hourly_target`.
    pub spend_rate_severity: BarSeverity,
    /// True for 2nd+ OpenCode accounts: omitted from the compact taskbar strip, shown in flyout.
    pub secondary: bool,
    /// Email or account label for flyout hints and tooltips.
    pub display_email: Option<String>,
    /// Compact primary reset timer shown inline for Claude (and in flyout headers).
    pub reset_short: Option<String>,
    /// Extra compact status shown in the flyout header (e.g. `"API $10.8/$20"` or `"re-auth"`).
    pub subtitle: Option<String>,
    pub auth_expired: bool,
    pub stale: bool,
    pub exhausted: bool,
    pub can_open_cursor: bool,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskbarSnapshot {
    pub items: Vec<ProviderItem>,
    pub updated_at_ms: i64,
    pub hide_in_fullscreen: bool,
    pub is_demo: bool,
    pub theme_mode: ThemeMode,
    /// Number of exhausted cards currently hidden via manual snooze or `auto_hide_exhausted`.
    pub snoozed_count: usize,
    /// Selected taskbar meter layout & reset-time style.
    pub taskbar_meter_style: TaskbarMeterStyle,
    /// Whether the local CPU & Memory taskbar monitor is enabled in Settings.
    #[serde(default = "default_true_snap")]
    pub system_monitor_enabled: bool,
    /// Latest live CPU & Memory telemetry + parent-app breakdown when enabled.
    #[serde(default)]
    pub system: Option<crate::sysmon::SystemSnapshot>,
}

fn default_true_snap() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsAccountRow {
    pub provider: ProviderKind,
    pub id: String,
    pub label: String,
    pub nickname: Option<String>,
    pub active: bool,
    pub hidden: bool,
    pub lapsed: bool,
    pub auth_expired: bool,
    pub stale: bool,
    pub plan: Option<String>,
    pub warning: Option<String>,
    pub bars: Vec<BarView>,
    pub extra_text: Option<String>,
    pub note: Option<String>,
    pub cached_at: Option<i64>,
    #[serde(default)]
    pub unsubscribed: bool,
    #[serde(default)]
    pub subscription_ends_at: Option<String>,
    #[serde(default)]
    pub subscription_ends_label: Option<String>,
    #[serde(default)]
    pub subscription_ends_soon: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsPaceData {
    pub last_hour: f64,
    pub last_5h: f64,
    pub today: f64,
    pub last_7d: f64,
    pub backfilling: bool,
    pub minutes: Vec<f64>,
    pub minutes_fable: Vec<f64>,
    pub quarters: Vec<f64>,
    pub quarters_fable: Vec<f64>,
    /// `(model, usd24h, share)` sorted largest first.
    pub by_model: Vec<(String, f64, f64)>,
    pub unpriced_models: Vec<String>,
    pub unpriced_replies: u64,
    pub usd_per_weekly_pct: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsSnapshot {
    pub settings: Settings,
    pub claude: Vec<SettingsAccountRow>,
    pub codex: Vec<SettingsAccountRow>,
    pub cursor: Vec<SettingsAccountRow>,
    pub grok: Vec<SettingsAccountRow>,
    pub opencode: Vec<SettingsAccountRow>,
    pub antigravity: Vec<SettingsAccountRow>,
    pub pace: SettingsPaceData,
    /// `(label, value, is_bad)` rows for Settings -> Cursor -> Diagnostics.
    pub cursor_diag_rows: Vec<(String, String, bool)>,
    pub diagnostics_text: String,
    pub app_version: String,
}

pub fn severity_of(percent: Option<f64>) -> BarSeverity {
    match percent {
        None => BarSeverity::Muted,
        Some(p) if p >= 100.0 => BarSeverity::Danger,
        Some(p) if p >= 80.0 => BarSeverity::Warn,
        Some(_) => BarSeverity::Ok,
    }
}

/// Colour tone of Claude's hourly spend rate against `settings.claude_hourly_target`.
///
/// No target or rate below `$0.50/h` -> `Muted`; below 80% of target -> `Warn`;
/// within 80%..=120% -> `Ok`; above 120% -> `Danger`.
pub fn spend_rate_severity(rate_usd_per_hour: f64, target: Option<u32>) -> BarSeverity {
    let Some(target) = target.filter(|&t| t > 0) else {
        return BarSeverity::Muted;
    };
    if !rate_usd_per_hour.is_finite() || rate_usd_per_hour < 0.5 {
        return BarSeverity::Muted;
    }
    let t = f64::from(target);
    if rate_usd_per_hour < 0.8 * t {
        BarSeverity::Warn
    } else if rate_usd_per_hour <= 1.2 * t {
        BarSeverity::Ok
    } else {
        BarSeverity::Danger
    }
}

pub fn parse_iso_ms(iso: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
}

fn is_passed(iso: Option<&str>, now_ms: i64) -> bool {
    iso.and_then(parse_iso_ms).is_some_and(|ms| ms <= now_ms)
}

/// Compact reset countdown suited for the taskbar and flyout (`45m`, `2h14m`, `4d`, `9d`).
pub fn format_reset_short(iso: &str, now_ms: i64) -> Option<String> {
    let target = parse_iso_ms(iso)?;
    let diff_ms = target - now_ms;
    if diff_ms <= 0 {
        return None;
    }
    let total_mins = (diff_ms + 30_000) / 60_000;
    if total_mins < 60 {
        return Some(format!("{}m", total_mins.max(1)));
    }
    let hours = total_mins / 60;
    let mins = total_mins % 60;
    if hours < 24 {
        if mins == 0 {
            Some(format!("{hours}h"))
        } else {
            Some(format!("{hours}h{mins:02}m"))
        }
    } else {
        let days = hours / 24;
        let rem_h = hours % 24;
        if days < 3 && rem_h > 0 {
            Some(format!("{days}d{rem_h}h"))
        } else {
            Some(format!("{days}d"))
        }
    }
}

/// Local wall-clock reset label (`"Today 1:30 AM"`, `"Tomorrow 9:00 AM"`, `"Thu 9:00 PM"`).
pub fn format_reset_local(iso: &str, now_ms: i64) -> Option<String> {
    format_reset_local_tz(&Local, iso, now_ms)
}

fn format_reset_local_tz<Tz: TimeZone>(tz: &Tz, iso: &str, now_ms: i64) -> Option<String>
where
    Tz::Offset: Display,
{
    let target_ms = parse_iso_ms(iso)?;
    if target_ms <= now_ms {
        return None;
    }
    let target_dt = DateTime::<Utc>::from_timestamp_millis(target_ms)?.with_timezone(tz);
    let now_dt = DateTime::<Utc>::from_timestamp_millis(now_ms)?.with_timezone(tz);
    let days_ahead = target_dt
        .date_naive()
        .signed_duration_since(now_dt.date_naive())
        .num_days();
    let time = target_dt.format("%-I:%M %p");
    let label = match days_ahead {
        0 => format!("Today {time}"),
        1 => format!("Tomorrow {time}"),
        2..=6 => format!("{} {time}", target_dt.format("%a")),
        _ => format!("{}, {time}", target_dt.format("%b %-d")),
    };
    Some(label)
}

fn bar_window_ms(key: &str) -> Option<i64> {
    match key {
        "fiveHour" | "rolling" => Some(5 * 3_600_000),
        "weekly" | "scoped" => Some(7 * 86_400_000),
        "monthly" | "firstParty" | "api" => Some(30 * 86_400_000),
        _ => None,
    }
}

/// Expected linear time-elapsed percentage (`0.0..=100.0`) inside the bar's window.
pub fn compute_pace_pct(key: &str, resets_at: Option<&str>, now_ms: i64) -> Option<f64> {
    let window_ms = bar_window_ms(key)?;
    let resets_ms = resets_at.and_then(parse_iso_ms)?;
    let remaining = resets_ms - now_ms;
    if remaining > 0 && remaining < window_ms {
        Some((((window_ms - remaining) as f64 / window_ms as f64) * 100.0).clamp(0.0, 100.0))
    } else {
        None
    }
}

/// Format Claude Code hourly spend rate (`$42/h`, `$4.5/h`). Returns `None` below `$0.50/h`.
pub fn format_rate_usd(usd_per_hour: f64) -> Option<String> {
    if !usd_per_hour.is_finite() || usd_per_hour < 0.5 {
        return None;
    }
    if usd_per_hour >= 10.0 {
        Some(format!("${:.0}/h", usd_per_hour.round()))
    } else {
        Some(format!("${usd_per_hour:.1}/h"))
    }
}

/// `"$12.34"`, or `"$50"` for whole dollars (matches `src/lib/format.ts:formatUsd`).
fn format_usd_exact(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 {
        format!("${v:.0}")
    } else {
        format!("${v:.2}")
    }
}

fn format_u64_commas(mut n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let mut parts = Vec::new();
    while n >= 1_000 {
        parts.push(format!("{:03}", n % 1_000));
        n /= 1_000;
    }
    parts.push(n.to_string());
    parts.reverse();
    parts.join(",")
}

/// Compact USD for spend summaries (`"$1,120"`, `"$210"`, `"$4.50"`).
fn format_usd_compact(v: f64) -> String {
    if !v.is_finite() || v <= 0.0 {
        return "$0".to_string();
    }
    if v >= 10.0 {
        format!("${}", format_u64_commas(v.round() as u64))
    } else {
        format_usd_exact(v)
    }
}

fn format_claude_spend_summary(spend: &claude_logs::Spend) -> Option<String> {
    if spend.backfilling {
        return None;
    }
    let buckets = [
        ("1h", spend.last_hour),
        ("5h", spend.last5h),
        ("Today", spend.today),
        ("7d", spend.last7d),
    ];
    let parts: Vec<String> = buckets
        .into_iter()
        .filter(|(_, v)| v.is_finite() && *v >= 0.05)
        .map(|(label, v)| format!("{label} {}", format_usd_compact(v)))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `"claude-opus-5-5"` -> `"Opus 5.5"`, `"claude-fable-5-2-20261001"` -> `"Fable 5.2"`,
/// `"claude-haiku-5"` -> `"Haiku 5"`; other ids stay as they are.
pub fn model_label(id: &str) -> String {
    let Some(rest) = id.strip_prefix("claude-") else {
        return id.to_string();
    };
    let Some((family, after_family)) = rest.split_once('-') else {
        return id.to_string();
    };
    if family.is_empty() || !family.bytes().all(|b| b.is_ascii_lowercase()) {
        return id.to_string();
    }
    let major_len = after_family
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if major_len == 0 {
        return id.to_string();
    }
    let major = &after_family[..major_len];
    let tail = &after_family[major_len..];
    let minor = tail.strip_prefix('-').and_then(|s| {
        let n = s.bytes().take_while(|b| b.is_ascii_digit()).count();
        (n > 0).then_some(&s[..n])
    });
    let mut cap = String::with_capacity(family.len());
    let mut chars = family.chars();
    if let Some(first) = chars.next() {
        cap.push(first.to_ascii_uppercase());
        cap.push_str(chars.as_str());
    }
    match minor {
        Some(min) => format!("{cap} {major}.{min}"),
        None => format!("{cap} {major}"),
    }
}

/// Whole percent, or one decimal under 1 (`"54%"`, `"0.4%"`).
fn format_pct_share(v: f64) -> String {
    let abs = v.abs();
    if abs > 0.0 && abs < 1.0 {
        format!("{v:.1}%")
    } else {
        format!("{:.0}%", v.round())
    }
}

fn five_hour_start(resets_at: Option<&str>, now_ms: i64) -> i64 {
    let window = 5 * history::HOUR_MS;
    resets_at
        .and_then(parse_iso_ms)
        .filter(|&r| r > now_ms && r <= now_ms + window)
        .map_or(now_ms - window, |r| r - window)
}

fn claude_five_hour_tooltip_lines(pct: f64, by_model: &[ModelUsd]) -> Vec<String> {
    if !pct.is_finite() || pct <= 0.0 {
        return Vec::new();
    }
    let total: f64 = by_model.iter().filter(|m| m.usd > 0.0).map(|m| m.usd).sum();
    if !total.is_finite() || total <= 0.0 {
        return Vec::new();
    }
    by_model
        .iter()
        .filter(|m| m.usd > 0.0)
        .map(|m| {
            let share_pct = (pct * m.usd) / total;
            format!("{} ≈{}", model_label(&m.model), format_pct_share(share_pct))
        })
        .collect()
}

fn claude_weekly_tooltip_lines(pct: f64, scoped: Option<&ClaudeScopedWindow>) -> Vec<String> {
    if !pct.is_finite() || pct <= 0.0 {
        return Vec::new();
    }
    let Some(scoped) = scoped else {
        return Vec::new();
    };
    let Some(share) = scoped.share_pct_of_weekly.filter(|&v| v > 0.0) else {
        return Vec::new();
    };
    let own = share.min(pct);
    vec![
        format!("{} {}", scoped.label, format_pct_share(own)),
        format!("Other models {}", format_pct_share(pct - own)),
    ]
}

fn cursor_api_tooltip_lines(by_model: &[CursorModelSpend]) -> Vec<String> {
    let mut lines: Vec<String> = by_model
        .iter()
        .take(3)
        .map(|m| format!("{} {}", m.model, format_usd_exact(m.usd)))
        .collect();
    if by_model.len() > 3 {
        let rest: f64 = by_model.iter().skip(3).map(|m| m.usd).sum();
        lines.push(format!("other {}", format_usd_exact(rest)));
    }
    lines
}

/// Choose at most 2 bars for the taskbar strip (Variant A).
///
/// When a provider has 3+ visible bars (e.g. OpenCode R/W/M or Claude 5h/W/Fable),
/// keeps the primary bar and promotes whichever remaining bar is highest (most urgent),
/// preserving original display order.
pub fn pick_taskbar_bars(bars: &[BarView]) -> Vec<BarView> {
    if bars.len() <= 2 {
        return bars.to_vec();
    }
    // Always keep index 0 (primary short-window bar) unless another pair is strictly exhausted,
    // and pair it with the highest-percent remaining bar.
    let mut best_rest = 1usize;
    let mut best_val = bars[1].percent.unwrap_or(-1.0);
    for (idx, bar) in bars.iter().enumerate().skip(2) {
        let val = bar.percent.unwrap_or(-1.0);
        if val > best_val {
            best_val = val;
            best_rest = idx;
        }
    }
    vec![bars[0].clone(), bars[best_rest].clone()]
}

fn filter_hidden_bars(bars: Vec<BarView>, hidden: &[String]) -> Vec<BarView> {
    if hidden.is_empty() || bars.len() <= 1 {
        return bars;
    }
    let set: HashSet<&str> = hidden.iter().map(String::as_str).collect();
    if bars.iter().all(|b| set.contains(b.key.as_str())) {
        return bars;
    }
    bars.into_iter()
        .filter(|b| !set.contains(b.key.as_str()))
        .collect()
}

fn is_auth_expired(row: &Value) -> bool {
    row.get("authExpired")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || row.get("error").and_then(Value::as_str) == Some("auth-rejected")
}

/// OpenCode stores bare lowercased email in `disabled_accounts`; tracked providers store
/// `"<prefix>:<id>"` case-insensitively.
fn is_disabled(settings: &Settings, provider: ProviderKind, account_id: &str) -> bool {
    let key = match provider {
        ProviderKind::OpenCode => account_id.trim().to_lowercase(),
        _ => format!("{}:{}", provider.key_prefix(), account_id.trim()).to_lowercase(),
    };
    settings
        .disabled_accounts
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&key))
}

fn subscription_account_keys(provider: ProviderKind, account_id: &str) -> (String, Option<String>) {
    let trimmed = account_id.trim().to_lowercase();
    let scoped = format!("{}:{trimmed}", provider.key_prefix());
    let bare = (provider == ProviderKind::OpenCode).then_some(trimmed);
    (scoped, bare)
}

fn is_marked_unsubscribed(settings: &Settings, provider: ProviderKind, account_id: &str) -> bool {
    let (scoped, bare) = subscription_account_keys(provider, account_id);
    settings.unsubscribed_accounts.iter().any(|entry| {
        entry.eq_ignore_ascii_case(&scoped)
            || bare
                .as_deref()
                .is_some_and(|b| entry.eq_ignore_ascii_case(b))
    })
}

fn lookup_manual_subscription_end(
    settings: &Settings,
    provider: ProviderKind,
    account_id: &str,
) -> Option<String> {
    let (scoped, bare) = subscription_account_keys(provider, account_id);
    settings
        .subscription_ends_at
        .iter()
        .find(|(k, _)| {
            k.eq_ignore_ascii_case(&scoped)
                || bare.as_deref().is_some_and(|b| k.eq_ignore_ascii_case(b))
        })
        .and_then(|(_, v)| crate::store::normalize_ymd_date(v))
}

fn naive_date_for_ms(ms: i64) -> Option<NaiveDate> {
    let dt = DateTime::from_timestamp_millis(ms)?;
    #[cfg(test)]
    {
        Some(dt.date_naive())
    }
    #[cfg(not(test))]
    {
        Some(dt.with_timezone(&Local).date_naive())
    }
}

fn iso_to_naive_date(iso: &str) -> Option<NaiveDate> {
    if let Some(ms) = parse_iso_ms(iso) {
        if let Some(d) = naive_date_for_ms(ms) {
            return Some(d);
        }
    }
    let ymd = crate::store::normalize_ymd_date(iso)?;
    NaiveDate::parse_from_str(&ymd, "%Y-%m-%d").ok()
}

/// True when the account should be excluded from the taskbar strip because the user explicitly
/// marked it unsubscribed or its user-recorded subscription end date has already passed.
fn is_unsubscribed_effective(
    settings: &Settings,
    provider: ProviderKind,
    account_id: &str,
    now_ms: i64,
) -> bool {
    if is_marked_unsubscribed(settings, provider, account_id) {
        return true;
    }
    let Some(today) = naive_date_for_ms(now_ms) else {
        return false;
    };
    lookup_manual_subscription_end(settings, provider, account_id)
        .and_then(|ymd| NaiveDate::parse_from_str(&ymd, "%Y-%m-%d").ok())
        .is_some_and(|end| end < today)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SubscriptionStatus {
    unsubscribed: bool,
    ends_at: Option<String>,
    ends_label: Option<String>,
    ends_soon: bool,
}

fn resolve_subscription_status(
    settings: &Settings,
    provider: ProviderKind,
    account_id: &str,
    auto_end_iso: Option<&str>,
    now_ms: i64,
) -> SubscriptionStatus {
    let marked = is_marked_unsubscribed(settings, provider, account_id);
    let manual_date = lookup_manual_subscription_end(settings, provider, account_id)
        .and_then(|ymd| NaiveDate::parse_from_str(&ymd, "%Y-%m-%d").ok());
    let auto_date = auto_end_iso.and_then(iso_to_naive_date);
    let Some(today) = naive_date_for_ms(now_ms) else {
        return SubscriptionStatus {
            unsubscribed: marked,
            ends_at: manual_date
                .or(auto_date)
                .map(|d| d.format("%Y-%m-%d").to_string()),
            ends_label: None,
            ends_soon: false,
        };
    };

    let manual_expired = manual_date.is_some_and(|d| d < today);
    let unsubscribed = marked || manual_expired;
    let effective_date = manual_date.or(auto_date);
    let ends_at = effective_date.map(|d| d.format("%Y-%m-%d").to_string());

    let (ends_label, ends_soon) = match effective_date {
        Some(date) => {
            let days = (date - today).num_days();
            if manual_date.is_some() || unsubscribed {
                if days < 0 {
                    (Some(format!("Ended {}", date.format("%b %-d"))), true)
                } else if days == 0 {
                    (Some("Ends today".to_string()), true)
                } else {
                    (
                        Some(format!("Ends {} ({}d)", date.format("%b %-d"), days)),
                        days <= 3,
                    )
                }
            } else if days < 0 {
                // Auto-detected cycle timestamp already passed (stale cache); wait for next poll.
                (None, false)
            } else if days == 0 {
                (Some("Ends today".to_string()), true)
            } else {
                (
                    Some(format!("Ends {} ({}d)", date.format("%b %-d"), days)),
                    days <= 3,
                )
            }
        }
        None => (None, false),
    };

    SubscriptionStatus {
        unsubscribed,
        ends_at,
        ends_label,
        ends_soon,
    }
}

/// Whether a manual snooze timestamp is currently active (in the future) for `provider:account_id`.
fn manual_snooze_active(
    settings: &Settings,
    provider: ProviderKind,
    account_id: &str,
    now_ms: i64,
) -> bool {
    let key = format!("{}:{}", provider.key_prefix(), account_id.trim()).to_lowercase();
    settings
        .snoozed_until
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&key))
        .is_some_and(|(_, &until)| (until as i64) > now_ms)
}

/// Snoozes use `"<prefix>:<id>"` for all providers (including `opencode:<email>`), matched
/// case-insensitively, and only hide the card while it is still exhausted so an early reset
/// brings the card back immediately (`src/lib/used-up.ts:isSnoozed`). When
/// `settings.auto_hide_exhausted` is enabled, any exhausted card is hidden automatically.
fn is_snoozed(
    settings: &Settings,
    provider: ProviderKind,
    account_id: &str,
    exhausted: bool,
    now_ms: i64,
) -> bool {
    exhausted
        && (settings.auto_hide_exhausted
            || manual_snooze_active(settings, provider, account_id, now_ms))
}

fn non_empty_hover_bars(bars: Vec<BarView>) -> Vec<BarView> {
    let non_empty: Vec<BarView> = bars
        .iter()
        .filter(|b| b.percent.is_some() || b.resets_at.is_some())
        .cloned()
        .collect();
    if non_empty.is_empty() {
        bars
    } else {
        non_empty
    }
}

/// Exact future epoch-ms timestamp when an exhausted card resets: latest future `resets_at` among
/// full (`Danger`) bars (excluding Claude's scoped bar), falling back to the latest future
/// `resets_at` among `flyout_bars`. Returns `None` when `!exhausted` or no future reset exists.
fn compute_exhausted_until_ms(
    exhausted: bool,
    flyout_bars: &[BarView],
    now_ms: i64,
) -> Option<u64> {
    if !exhausted {
        return None;
    }
    let danger_max = flyout_bars
        .iter()
        .filter(|b| b.key != "scoped" && b.severity == BarSeverity::Danger)
        .filter_map(|b| b.resets_at.as_deref().and_then(parse_iso_ms))
        .filter(|&ms| ms > now_ms)
        .max();
    let until = danger_max.or_else(|| {
        flyout_bars
            .iter()
            .filter_map(|b| b.resets_at.as_deref().and_then(parse_iso_ms))
            .filter(|&ms| ms > now_ms)
            .max()
    })?;
    u64::try_from(until).ok()
}

fn lookup_display_name(settings: &Settings, key: &str) -> Option<String> {
    let raw = settings.display_names.get(key).or_else(|| {
        settings
            .display_names
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    });
    raw.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Account title matching `src/widget/_components/cards.tsx:accountTitle`.
fn tracked_account_title(
    settings: &Settings,
    provider: ProviderKind,
    account_id: &str,
    label: Option<&str>,
    multiple: bool,
) -> String {
    let provider_name = provider.display_name();
    if settings.show_provider_names {
        return provider_name.to_string();
    }
    let scoped_key = format!("{}:{}", provider.key_prefix(), account_id.trim());
    if let Some(nick) = lookup_display_name(settings, &scoped_key) {
        return nick;
    }
    if multiple {
        if let Some(lbl) = label.filter(|s| !s.is_empty()) {
            return lbl.to_string();
        }
    }
    if let Some(nick) = lookup_display_name(settings, provider.key_prefix()) {
        return nick;
    }
    if provider == ProviderKind::Cursor {
        if let Some(lbl) = label.filter(|s| !s.is_empty()) {
            return lbl.to_string();
        }
    }
    provider_name.to_string()
}

fn opencode_account_title(settings: &Settings, email: &str) -> String {
    if settings.show_provider_names {
        return ProviderKind::OpenCode.display_name().to_string();
    }
    if let Some(nick) = lookup_display_name(settings, email) {
        return nick;
    }
    let short: String = email.chars().take(4).collect();
    if short.is_empty() {
        ProviderKind::OpenCode.display_name().to_string()
    } else {
        short
    }
}

fn hidden_bars_for(settings: &Settings, provider: ProviderKind) -> Vec<String> {
    settings
        .hidden_bars
        .get(&provider.to_widget_provider())
        .cloned()
        .unwrap_or_default()
}

fn is_provider_enabled(settings: &Settings, wp: WidgetProvider) -> bool {
    match wp {
        WidgetProvider::Claude => settings.claude_enabled,
        WidgetProvider::Cursor => settings.cursor_enabled,
        WidgetProvider::Codex => settings.codex_enabled,
        WidgetProvider::Grok => settings.grok_enabled,
        WidgetProvider::Opencode => settings.opencode_enabled,
        WidgetProvider::Antigravity => settings.antigravity_enabled,
    }
}

fn is_unknown_opencode_window(w: &UsageWindow, stale: bool, now_ms: i64) -> bool {
    stale && is_passed(Some(w.resets_at.as_str()), now_ms)
}

fn is_exhausted_opencode_window(w: &UsageWindow, stale: bool, now_ms: i64) -> bool {
    if is_unknown_opencode_window(w, stale, now_ms) {
        return false;
    }
    w.percent >= 100.0 || w.status == "exhausted"
}

fn is_opencode_account_exhausted(u: &OpenCodeUsage, stale: bool, now_ms: i64) -> bool {
    is_exhausted_opencode_window(&u.monthly, stale, now_ms)
        || is_exhausted_opencode_window(&u.weekly, stale, now_ms)
}

fn opencode_usable_again_at(u: &OpenCodeUsage, stale: bool, now_ms: i64) -> i64 {
    let mut latest: Option<i64> = None;
    for w in [&u.monthly, &u.weekly] {
        if is_exhausted_opencode_window(w, stale, now_ms) {
            if let Some(ms) = parse_iso_ms(&w.resets_at) {
                latest = Some(latest.map_or(ms, |cur| cur.max(ms)));
            }
        }
    }
    latest.unwrap_or(0)
}

/// Poll all enabled providers concurrently and assemble a `TaskbarSnapshot`.
pub async fn poll_snapshot(core: &Core) -> TaskbarSnapshot {
    let settings = core.store.get_settings();
    let now_ms = claude_logs::now_ms();

    let (claude_rows, cursor_resp, codex_rows, grok_rows, opencode_rows, antigravity_rows) = tokio::join!(
        async {
            if settings.claude_enabled {
                crate::providers::claude::accounts(core).await
            } else {
                Vec::new()
            }
        },
        async {
            if settings.cursor_enabled {
                Some(crate::providers::cursor::response(core).await)
            } else {
                None
            }
        },
        async {
            if settings.codex_enabled {
                crate::providers::codex::accounts(core).await
            } else {
                Vec::new()
            }
        },
        async {
            if settings.grok_enabled {
                crate::providers::grok::accounts(core).await
            } else {
                Vec::new()
            }
        },
        async {
            if settings.opencode_enabled {
                crate::providers::opencode::accounts(core).await
            } else {
                Vec::new()
            }
        },
        async {
            if settings.antigravity_enabled {
                crate::providers::antigravity::accounts(core).await
            } else {
                Vec::new()
            }
        },
    );

    // Claude Code hourly spend rate + hover spend summary (reads local transcripts already scanned).
    let claude_spend_info = if settings.claude_enabled {
        let spend = core.spend.spend(now_ms);
        if spend.backfilling {
            ClaudeSpendInfo {
                rate_text: None,
                rate_severity: BarSeverity::Muted,
                summary: None,
            }
        } else {
            let rate = if spend.rate60m >= 0.5 {
                spend.rate60m
            } else if spend.rate15m >= 0.5 {
                spend.rate15m
            } else {
                spend.rate60m
            };
            ClaudeSpendInfo {
                rate_text: format_rate_usd(rate),
                rate_severity: spend_rate_severity(spend.rate60m, settings.claude_hourly_target),
                summary: format_claude_spend_summary(&spend),
            }
        }
    } else {
        ClaudeSpendInfo {
            rate_text: None,
            rate_severity: BarSeverity::Muted,
            summary: None,
        }
    };

    let mut items = Vec::new();
    let mut snoozed_count = 0usize;
    for &wp in &settings.provider_order {
        if !is_provider_enabled(&settings, wp) {
            continue;
        }
        let (built, hidden_count) = match wp {
            WidgetProvider::Claude => build_claude_items(
                &claude_rows,
                &settings,
                &claude_spend_info,
                now_ms,
                |five_start| {
                    (
                        core.spend.fable_share(five_start, now_ms),
                        core.spend.models_between(five_start, now_ms),
                    )
                },
            ),
            WidgetProvider::Cursor => match cursor_resp {
                Some(ref resp) => build_cursor_items(resp, &settings, now_ms),
                None => (Vec::new(), 0),
            },
            WidgetProvider::Codex => build_codex_items(&codex_rows, &settings, now_ms),
            WidgetProvider::Grok => build_grok_items(&grok_rows, &settings, now_ms),
            WidgetProvider::Opencode => build_opencode_items(&opencode_rows, &settings, now_ms),
            WidgetProvider::Antigravity => {
                build_antigravity_items(&antigravity_rows, &settings, now_ms)
            }
        };
        items.extend(built);
        snoozed_count += hidden_count;
    }

    TaskbarSnapshot {
        items,
        updated_at_ms: now_ms,
        hide_in_fullscreen: settings.hide_in_fullscreen,
        is_demo: false,
        theme_mode: settings.theme_mode,
        snoozed_count,
        taskbar_meter_style: settings.taskbar_meter_style,
        system_monitor_enabled: settings.system_monitor_enabled,
        system: None,
    }
}

struct ClaudeSpendInfo {
    rate_text: Option<String>,
    rate_severity: BarSeverity,
    summary: Option<String>,
}

struct PendingItem {
    account_id: String,
    display_email: Option<String>,
    flyout_bars: Vec<BarView>,
    taskbar_bars: Vec<BarView>,
    hover_bars: Vec<BarView>,
    hover_details: Vec<(String, String)>,
    resets_badge: Option<String>,
    exhausted_until_ms: Option<u64>,
    spend_rate_text: Option<String>,
    spend_rate_severity: BarSeverity,
    reset_short: Option<String>,
    subtitle: Option<String>,
    auth_expired: bool,
    stale: bool,
    exhausted: bool,
    can_open_cursor: bool,
}

fn finalize_tracked_items(
    provider: ProviderKind,
    settings: &Settings,
    pending: Vec<PendingItem>,
) -> Vec<ProviderItem> {
    let multiple = pending.len() > 1;
    pending
        .into_iter()
        .map(|p| {
            let title = tracked_account_title(
                settings,
                provider,
                &p.account_id,
                p.display_email.as_deref(),
                multiple,
            );
            let tooltip = build_tooltip(
                &title,
                &p.flyout_bars,
                p.spend_rate_text.as_deref(),
                p.subtitle.as_deref(),
            );
            ProviderItem {
                provider,
                account_id: p.account_id,
                title,
                taskbar_bars: p.taskbar_bars,
                flyout_bars: p.flyout_bars,
                hover_bars: p.hover_bars,
                hover_details: p.hover_details,
                resets_badge: p.resets_badge,
                exhausted_until_ms: p.exhausted_until_ms,
                spend_rate_text: p.spend_rate_text,
                spend_rate_severity: p.spend_rate_severity,
                secondary: false,
                display_email: if settings.show_provider_names {
                    None
                } else {
                    p.display_email
                },
                reset_short: p.reset_short,
                subtitle: p.subtitle,
                auth_expired: p.auth_expired,
                stale: p.stale,
                exhausted: p.exhausted,
                can_open_cursor: p.can_open_cursor,
                tooltip,
            }
        })
        .collect()
}

fn build_claude_items<F>(
    rows: &[Value],
    settings: &Settings,
    spend_info: &ClaudeSpendInfo,
    now_ms: i64,
    mut five_hour_spend: F,
) -> (Vec<ProviderItem>, usize)
where
    F: FnMut(i64) -> (Option<ModelShare>, Vec<ModelUsd>),
{
    let hidden = hidden_bars_for(settings, ProviderKind::Claude);
    let mut pending = Vec::new();
    let mut snoozed = 0usize;

    for row in rows {
        if !row.get("active").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let id = row.get("id").and_then(Value::as_str).unwrap_or("claude");
        if is_disabled(settings, ProviderKind::Claude, id)
            || is_unsubscribed_effective(settings, ProviderKind::Claude, id, now_ms)
        {
            continue;
        }
        let auth_expired = is_auth_expired(row);
        let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
        let usage: Option<ClaudeUsage> = row
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let Some(u) = usage else { continue };
        if !u.signed_in {
            continue;
        }

        let is_primary = pending.is_empty();
        let (fable_5h, models_5h) = if is_primary {
            let start = five_hour_start(u.five_hour.resets_at.as_deref(), now_ms);
            five_hour_spend(start)
        } else {
            (None, Vec::new())
        };

        let mut five_bar = BarView::new(
            "fiveHour",
            "5h",
            u.five_hour.percent,
            u.five_hour.resets_at.clone(),
            now_ms,
            auth_expired,
        );
        if let Some(bar_pct) = five_bar.percent {
            if !auth_expired && settings.fable_shade {
                if let Some(share) = fable_5h {
                    if share.total_usd > 0.0 && share.usd > 0.0 && bar_pct > 0.0 {
                        five_bar.fable_segment_pct =
                            Some(bar_pct * (share.usd / share.total_usd).min(1.0));
                    }
                }
            }
            if !auth_expired {
                five_bar.tooltip_lines = claude_five_hour_tooltip_lines(bar_pct, &models_5h);
            }
        }

        let mut weekly_bar = BarView::new(
            "weekly",
            "Weekly",
            u.weekly.percent,
            u.weekly.resets_at.clone(),
            now_ms,
            auth_expired,
        );
        let first_scoped = u.weekly_scoped.first();
        if let Some(bar_pct) = weekly_bar.percent {
            if !auth_expired && settings.fable_shade {
                weekly_bar.fable_segment_pct = first_scoped
                    .and_then(|s| s.share_pct_of_weekly)
                    .filter(|&v| v > 0.0 && bar_pct > 0.0)
                    .map(|v| v.min(bar_pct));
            }
            if !auth_expired {
                weekly_bar.tooltip_lines = claude_weekly_tooltip_lines(bar_pct, first_scoped);
            }
        }

        let mut raw_bars = vec![five_bar, weekly_bar];
        for scoped in &u.weekly_scoped {
            let mut bar = BarView::new(
                "scoped",
                scoped.label.clone(),
                scoped.percent,
                scoped.resets_at.clone(),
                now_ms,
                auth_expired,
            );
            if !auth_expired {
                if let Some(allowance) = scoped
                    .allowance_pct_of_weekly
                    .filter(|v| v.is_finite() && *v > 0.0)
                {
                    let line = match scoped
                        .share_pct_of_weekly
                        .filter(|v| v.is_finite() && *v >= 0.0)
                    {
                        Some(share) => format!(
                            "{}% of {}% weekly allowance",
                            share.round() as i64,
                            allowance.round() as i64
                        ),
                        None => format!("{}% weekly allowance", allowance.round() as i64),
                    };
                    bar.tooltip_lines.push(line);
                }
            }
            raw_bars.push(bar);
        }

        let flyout_bars = filter_hidden_bars(raw_bars, &hidden);
        let hover_bars = non_empty_hover_bars(flyout_bars.clone());

        // Only check visible fiveHour and weekly bars; explicitly exclude scoped (Fable).
        let exhausted = flyout_bars
            .iter()
            .filter(|b| b.key == "fiveHour" || b.key == "weekly")
            .any(|b| b.severity == BarSeverity::Danger);

        if is_snoozed(settings, ProviderKind::Claude, id, exhausted, now_ms) {
            snoozed += 1;
            continue;
        }

        let taskbar_bars = pick_taskbar_bars(&flyout_bars);
        let reset_short = flyout_bars
            .first()
            .and_then(|b| b.reset_short.clone())
            .or_else(|| flyout_bars.get(1).and_then(|b| b.reset_short.clone()));
        let subtitle = if auth_expired {
            Some("re-auth".to_string())
        } else {
            None
        };
        let display_email = row
            .get("label")
            .and_then(Value::as_str)
            .or(u.email.as_deref())
            .map(str::to_string);

        let mut hover_details = Vec::new();
        if is_primary {
            if let Some(ref summary) = spend_info.summary {
                hover_details.push(("Spend".to_string(), summary.clone()));
            }
            if let Some(target) = settings.claude_hourly_target {
                hover_details.push(("Target".to_string(), format!("${target}/h")));
            }
        }
        if let Some(resets_text) = claude_resets_detail_text(u.resets.as_ref()) {
            hover_details.push(("Resets".to_string(), resets_text));
        }
        let plan = u
            .plan_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(claude_plan_label)
            .or_else(|| {
                u.subscription_type
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| capitalize(&claude_plan_label(s)))
            });
        if let Some(plan) = plan {
            hover_details.push(("Plan".to_string(), plan));
        }
        if let Some(ref email) = display_email {
            if !email.is_empty() {
                hover_details.push(("Account".to_string(), email.clone()));
            }
        }
        let resets_badge = claude_resets_badge(&u);
        let exhausted_until_ms = compute_exhausted_until_ms(exhausted, &flyout_bars, now_ms);

        pending.push(PendingItem {
            account_id: id.to_string(),
            display_email,
            flyout_bars,
            taskbar_bars,
            hover_bars,
            hover_details,
            resets_badge,
            exhausted_until_ms,
            spend_rate_text: if is_primary {
                spend_info.rate_text.clone()
            } else {
                None
            },
            spend_rate_severity: if is_primary {
                spend_info.rate_severity
            } else {
                BarSeverity::Muted
            },
            reset_short,
            subtitle,
            auth_expired,
            stale,
            exhausted,
            can_open_cursor: false,
        });
    }

    (
        finalize_tracked_items(ProviderKind::Claude, settings, pending),
        snoozed,
    )
}

fn build_cursor_items(
    resp: &Value,
    settings: &Settings,
    now_ms: i64,
) -> (Vec<ProviderItem>, usize) {
    let Some(rows) = resp.get("accounts").and_then(Value::as_array) else {
        return (Vec::new(), 0);
    };
    let hidden = hidden_bars_for(settings, ProviderKind::Cursor);
    let mut pending = Vec::new();
    let mut snoozed = 0usize;

    for row in rows {
        if !row.get("active").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let id = row.get("id").and_then(Value::as_str).unwrap_or("cursor");
        if is_disabled(settings, ProviderKind::Cursor, id)
            || is_unsubscribed_effective(settings, ProviderKind::Cursor, id, now_ms)
        {
            continue;
        }
        let auth_expired = is_auth_expired(row);
        let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
        let usage: Option<CursorUsage> = row
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let Some(u) = usage else { continue };

        let fp_bar = BarView::new(
            "firstParty",
            "1P",
            u.cursor_models_percent_used,
            u.resets_at.clone(),
            now_ms,
            auth_expired,
        );
        let mut api_bar = BarView::new(
            "api",
            "API",
            u.other_models_percent_used,
            u.resets_at.clone(),
            now_ms,
            auth_expired,
        );
        let cycle_passed = is_passed(u.resets_at.as_deref(), now_ms);
        if !cycle_passed {
            if let Some(ref spend) = u.api_spend {
                api_bar.tooltip_lines = cursor_api_tooltip_lines(&spend.by_model);
            }
        }

        let raw_bars = vec![fp_bar, api_bar];
        let flyout_bars = filter_hidden_bars(raw_bars, &hidden);
        let hover_bars = non_empty_hover_bars(flyout_bars.clone());

        let known_bars: Vec<&BarView> =
            flyout_bars.iter().filter(|b| b.percent.is_some()).collect();
        let all_full =
            !known_bars.is_empty() && known_bars.iter().all(|b| b.severity == BarSeverity::Danger);
        let on_demand_left = matches!(
            u.on_demand,
            Some(OnDemand::Enabled {
                spent_usd,
                limit_usd,
            }) if limit_usd.is_none_or(|lim| spent_usd < lim)
        );
        let exhausted = !auth_expired && all_full && !on_demand_left;

        if is_snoozed(settings, ProviderKind::Cursor, id, exhausted, now_ms) {
            snoozed += 1;
            continue;
        }

        let taskbar_bars = pick_taskbar_bars(&flyout_bars);
        let reset_short = u
            .resets_at
            .as_deref()
            .and_then(|iso| format_reset_short(iso, now_ms));

        let api_visible = flyout_bars.iter().any(|b| b.key == "api");
        let api_text = if !cycle_passed && api_visible {
            u.api_spend.as_ref().map(|s| match s.limit_usd {
                Some(lim) if lim > 0.0 => format!("API ${:.1}/${:.0}", s.spent_usd, lim),
                _ => format!("API ${:.2}", s.spent_usd),
            })
        } else {
            None
        };
        let subtitle = if auth_expired {
            Some("Open Cursor".to_string())
        } else {
            api_text
        };
        let display_email = row
            .get("label")
            .and_then(Value::as_str)
            .or(u.account_name.as_deref())
            .map(str::to_string);

        let mut hover_details = Vec::new();
        if !cycle_passed {
            if let Some(ref s) = u.api_spend {
                let val = match s.limit_usd {
                    Some(lim) if lim > 0.0 => format!("${:.2} / ${:.2}", s.spent_usd, lim),
                    _ => format!("${:.2}", s.spent_usd),
                };
                hover_details.push(("API spend".to_string(), val));
            }
        }
        if let Some(ref od) = u.on_demand {
            let val = match od {
                OnDemand::Disabled => "Off".to_string(),
                OnDemand::Enabled {
                    spent_usd,
                    limit_usd: Some(lim),
                } if *lim > 0.0 => format!("${spent_usd:.2} / ${lim:.2}"),
                OnDemand::Enabled { spent_usd, .. } if *spent_usd > 0.0 => {
                    format!("${spent_usd:.2}")
                }
                OnDemand::Enabled { .. } => "Enabled".to_string(),
            };
            hover_details.push(("On-demand".to_string(), val));
        }
        if let Some(plan) = u
            .plan_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            hover_details.push(("Plan".to_string(), plan.to_string()));
        }
        if let Some(ref email) = display_email {
            if !email.is_empty() {
                hover_details.push(("Account".to_string(), email.clone()));
            }
        }
        let exhausted_until_ms = compute_exhausted_until_ms(exhausted, &flyout_bars, now_ms);

        pending.push(PendingItem {
            account_id: id.to_string(),
            display_email,
            flyout_bars,
            taskbar_bars,
            hover_bars,
            hover_details,
            resets_badge: None,
            exhausted_until_ms,
            spend_rate_text: None,
            spend_rate_severity: BarSeverity::Muted,
            reset_short,
            subtitle,
            auth_expired,
            stale,
            exhausted,
            can_open_cursor: auth_expired,
        });
    }

    (
        finalize_tracked_items(ProviderKind::Cursor, settings, pending),
        snoozed,
    )
}

fn build_codex_items(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> (Vec<ProviderItem>, usize) {
    let hidden = hidden_bars_for(settings, ProviderKind::Codex);
    let mut pending = Vec::new();
    let mut snoozed = 0usize;

    for row in rows {
        if !row.get("active").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let id = row.get("id").and_then(Value::as_str).unwrap_or("codex");
        if is_disabled(settings, ProviderKind::Codex, id)
            || is_unsubscribed_effective(settings, ProviderKind::Codex, id, now_ms)
        {
            continue;
        }
        let auth_expired = is_auth_expired(row);
        let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
        let usage: Option<CodexUsage> = row
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let Some(u) = usage else { continue };

        let mut raw_bars = vec![BarView::new(
            "fiveHour",
            "5h",
            u.used_percent,
            u.reset_at.clone(),
            now_ms,
            auth_expired,
        )];
        if u.weekly_used_percent.is_some() || u.weekly_reset_at.is_some() {
            raw_bars.push(BarView::new(
                "weekly",
                "Weekly",
                u.weekly_used_percent,
                u.weekly_reset_at.clone(),
                now_ms,
                auth_expired,
            ));
        }
        let flyout_bars = filter_hidden_bars(raw_bars, &hidden);
        let hover_bars = non_empty_hover_bars(flyout_bars.clone());

        let five_hour_shown = flyout_bars.iter().any(|b| b.key == "fiveHour");
        let five_hour_stale_passed = stale && is_passed(u.reset_at.as_deref(), now_ms);
        let exhausted =
            (!auth_expired && five_hour_shown && u.limit_reached && !five_hour_stale_passed)
                || flyout_bars
                    .iter()
                    .any(|b| b.severity == BarSeverity::Danger);

        if is_snoozed(settings, ProviderKind::Codex, id, exhausted, now_ms) {
            snoozed += 1;
            continue;
        }

        let taskbar_bars = pick_taskbar_bars(&flyout_bars);
        let reset_short = flyout_bars.first().and_then(|b| b.reset_short.clone());
        let subtitle = if auth_expired {
            Some("re-auth".to_string())
        } else {
            None
        };
        let display_email = row
            .get("label")
            .and_then(Value::as_str)
            .or(u.email.as_deref())
            .map(str::to_string);

        let mut hover_details = Vec::new();
        if let Some(plan) = u
            .plan_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            hover_details.push(("Plan".to_string(), capitalize(plan)));
        }
        if let Some(ref email) = display_email {
            if !email.is_empty() {
                hover_details.push(("Account".to_string(), email.clone()));
            }
        }
        let exhausted_until_ms = compute_exhausted_until_ms(exhausted, &flyout_bars, now_ms);

        pending.push(PendingItem {
            account_id: id.to_string(),
            display_email,
            flyout_bars,
            taskbar_bars,
            hover_bars,
            hover_details,
            resets_badge: None,
            exhausted_until_ms,
            spend_rate_text: None,
            spend_rate_severity: BarSeverity::Muted,
            reset_short,
            subtitle,
            auth_expired,
            stale,
            exhausted,
            can_open_cursor: false,
        });
    }

    (
        finalize_tracked_items(ProviderKind::Codex, settings, pending),
        snoozed,
    )
}

fn build_grok_items(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> (Vec<ProviderItem>, usize) {
    let mut pending = Vec::new();
    let mut snoozed = 0usize;

    for row in rows {
        if !row.get("active").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let id = row.get("id").and_then(Value::as_str).unwrap_or("grok");
        if is_disabled(settings, ProviderKind::Grok, id)
            || is_unsubscribed_effective(settings, ProviderKind::Grok, id, now_ms)
        {
            continue;
        }
        let auth_expired = is_auth_expired(row);
        let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
        let usage: Option<GrokUsage> = row
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let Some(u) = usage else { continue };

        let flyout_bars = vec![BarView::new(
            "weekly",
            "Weekly",
            u.percent_used,
            u.reset_at.clone(),
            now_ms,
            auth_expired,
        )];
        let exhausted = flyout_bars
            .first()
            .is_some_and(|b| b.severity == BarSeverity::Danger);

        if is_snoozed(settings, ProviderKind::Grok, id, exhausted, now_ms) {
            snoozed += 1;
            continue;
        }

        let hover_bars = non_empty_hover_bars(flyout_bars.clone());
        let taskbar_bars = flyout_bars.clone();
        let reset_short = flyout_bars.first().and_then(|b| b.reset_short.clone());
        let subtitle = if auth_expired {
            Some("re-auth".to_string())
        } else {
            None
        };
        let display_email = row
            .get("label")
            .and_then(Value::as_str)
            .or(u.email.as_deref())
            .map(str::to_string);

        let mut hover_details = Vec::new();
        if let Some(plan) = u
            .plan_label
            .as_deref()
            .or(u.cursor_plan_name.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            hover_details.push(("Plan".to_string(), plan.to_string()));
        }
        if let Some(ref email) = display_email {
            if !email.is_empty() {
                hover_details.push(("Account".to_string(), email.clone()));
            }
        }
        let exhausted_until_ms = compute_exhausted_until_ms(exhausted, &flyout_bars, now_ms);

        pending.push(PendingItem {
            account_id: id.to_string(),
            display_email,
            flyout_bars,
            taskbar_bars,
            hover_bars,
            hover_details,
            resets_badge: None,
            exhausted_until_ms,
            spend_rate_text: None,
            spend_rate_severity: BarSeverity::Muted,
            reset_short,
            subtitle,
            auth_expired,
            stale,
            exhausted,
            can_open_cursor: false,
        });
    }

    (
        finalize_tracked_items(ProviderKind::Grok, settings, pending),
        snoozed,
    )
}

fn build_antigravity_items(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> (Vec<ProviderItem>, usize) {
    let hidden = hidden_bars_for(settings, ProviderKind::Antigravity);
    let mut pending = Vec::new();
    let mut snoozed = 0usize;

    for row in rows {
        if !row.get("active").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("antigravity");
        if is_disabled(settings, ProviderKind::Antigravity, id)
            || is_unsubscribed_effective(settings, ProviderKind::Antigravity, id, now_ms)
        {
            continue;
        }
        let auth_expired = is_auth_expired(row);
        let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
        let usage: Option<AntigravityUsage> = row
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let Some(u) = usage else { continue };

        let mut raw_bars = Vec::new();
        if u.five_hour_percent.is_some() || u.five_hour_resets_at.is_some() {
            raw_bars.push(BarView::new(
                "fiveHour",
                "5h",
                u.five_hour_percent,
                u.five_hour_resets_at.clone(),
                now_ms,
                auth_expired,
            ));
        }
        if u.weekly_percent.is_some() || u.weekly_resets_at.is_some() {
            raw_bars.push(BarView::new(
                "weekly",
                "Weekly",
                u.weekly_percent,
                u.weekly_resets_at.clone(),
                now_ms,
                auth_expired,
            ));
        }
        if raw_bars.is_empty() {
            continue;
        }

        let flyout_bars = {
            let f = filter_hidden_bars(raw_bars.clone(), &hidden);
            if f.is_empty() {
                raw_bars
            } else {
                f
            }
        };
        let hover_bars = if u.groups.len() > 1 {
            let mut pool_bars = Vec::new();
            for grp in &u.groups {
                if !hidden.iter().any(|h| h == "fiveHour")
                    && (grp.five_hour_percent.is_some() || grp.five_hour_resets_at.is_some())
                {
                    pool_bars.push(BarView::new(
                        "fiveHour",
                        format!("{} · 5h", grp.name),
                        grp.five_hour_percent,
                        grp.five_hour_resets_at.clone(),
                        now_ms,
                        auth_expired,
                    ));
                }
                if !hidden.iter().any(|h| h == "weekly")
                    && (grp.weekly_percent.is_some() || grp.weekly_resets_at.is_some())
                {
                    pool_bars.push(BarView::new(
                        "weekly",
                        format!("{} · Weekly", grp.name),
                        grp.weekly_percent,
                        grp.weekly_resets_at.clone(),
                        now_ms,
                        auth_expired,
                    ));
                }
            }
            if pool_bars.is_empty() {
                non_empty_hover_bars(flyout_bars.clone())
            } else {
                non_empty_hover_bars(pool_bars)
            }
        } else {
            non_empty_hover_bars(flyout_bars.clone())
        };
        let taskbar_bars = pick_taskbar_bars(&flyout_bars);
        let exhausted = flyout_bars
            .iter()
            .any(|b| b.severity == BarSeverity::Danger);
        if is_snoozed(settings, ProviderKind::Antigravity, id, exhausted, now_ms) {
            snoozed += 1;
            continue;
        }

        let reset_short = flyout_bars
            .first()
            .and_then(|b| b.reset_short.clone())
            .or_else(|| flyout_bars.get(1).and_then(|b| b.reset_short.clone()));
        let subtitle = if auth_expired {
            Some("re-auth".to_string())
        } else {
            None
        };
        let display_email = row
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(u.email);

        let mut hover_details = Vec::new();
        if let Some(ref email) = display_email {
            if !email.is_empty() {
                hover_details.push(("Account".to_string(), email.clone()));
            }
        }
        let exhausted_until_ms = compute_exhausted_until_ms(exhausted, &flyout_bars, now_ms);

        pending.push(PendingItem {
            account_id: id.to_string(),
            display_email,
            flyout_bars,
            taskbar_bars,
            hover_bars,
            hover_details,
            resets_badge: None,
            exhausted_until_ms,
            spend_rate_text: None,
            spend_rate_severity: BarSeverity::Muted,
            reset_short,
            subtitle,
            auth_expired,
            stale,
            exhausted,
            can_open_cursor: false,
        });
    }

    (
        finalize_tracked_items(ProviderKind::Antigravity, settings, pending),
        snoozed,
    )
}

struct OpenCodeCandidate {
    rank_exhausted: bool,
    usable_again_at: i64,
    last_change_at: i64,
    item: ProviderItem,
}

fn build_opencode_items(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> (Vec<ProviderItem>, usize) {
    let hidden = hidden_bars_for(settings, ProviderKind::OpenCode);
    let hidden_set: HashSet<&str> = hidden.iter().map(String::as_str).collect();
    let bar_shown = |key: &str| hidden.is_empty() || !hidden_set.contains(key);

    let mut candidates = Vec::new();
    let mut snoozed = 0usize;

    for row in rows {
        if row.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        // Exact parity with src/lib/ranking.ts:53: skip lapsed Go subscriptions.
        if row.get("subscriptionActive").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let Some(email) = row.get("email").and_then(Value::as_str) else {
            continue;
        };
        if is_disabled(settings, ProviderKind::OpenCode, email)
            || is_unsubscribed_effective(settings, ProviderKind::OpenCode, email, now_ms)
        {
            continue;
        }
        let auth_expired = is_auth_expired(row);
        let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
        let usage: Option<OpenCodeUsage> = row
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let Some(u) = usage else { continue };

        let weekly_full =
            bar_shown("weekly") && is_exhausted_opencode_window(&u.weekly, stale, now_ms);
        let monthly_full =
            bar_shown("monthly") && is_exhausted_opencode_window(&u.monthly, stale, now_ms);
        let card_exhausted = !auth_expired && (weekly_full || monthly_full);

        if is_snoozed(
            settings,
            ProviderKind::OpenCode,
            email,
            card_exhausted,
            now_ms,
        ) {
            snoozed += 1;
            continue;
        }

        // Shorter-window dimming (cards.tsx:249-250):
        // dim rolling when weekly or monthly is exhausted; dim weekly when monthly is exhausted.
        let mut rolling_bar = BarView::new(
            "rolling",
            "Rolling",
            Some(u.rolling.percent),
            Some(u.rolling.resets_at.clone()),
            now_ms,
            auth_expired || weekly_full || monthly_full,
        );
        if !auth_expired
            && !weekly_full
            && !monthly_full
            && rolling_bar.percent.is_some()
            && is_exhausted_opencode_window(&u.rolling, stale, now_ms)
        {
            rolling_bar.severity = BarSeverity::Danger;
        }

        let mut weekly_bar = BarView::new(
            "weekly",
            "Weekly",
            Some(u.weekly.percent),
            Some(u.weekly.resets_at.clone()),
            now_ms,
            auth_expired || monthly_full,
        );
        if !auth_expired && !monthly_full && weekly_bar.percent.is_some() && weekly_full {
            weekly_bar.severity = BarSeverity::Danger;
        }

        let mut monthly_bar = BarView::new(
            "monthly",
            "Monthly",
            Some(u.monthly.percent),
            Some(u.monthly.resets_at.clone()),
            now_ms,
            auth_expired,
        );
        if !auth_expired && monthly_bar.percent.is_some() && monthly_full {
            monthly_bar.severity = BarSeverity::Danger;
        }

        let raw_bars = vec![rolling_bar, weekly_bar, monthly_bar];
        let flyout_bars = filter_hidden_bars(raw_bars, &hidden);
        let hover_bars = non_empty_hover_bars(flyout_bars.clone());
        let taskbar_bars = pick_taskbar_bars(&flyout_bars);
        let reset_short = flyout_bars.first().and_then(|b| b.reset_short.clone());
        let title = opencode_account_title(settings, email);
        let subtitle = if auth_expired {
            Some("key rejected".to_string())
        } else if settings.show_provider_names {
            None
        } else {
            Some(email.to_string())
        };
        let tooltip = build_tooltip(&title, &flyout_bars, None, subtitle.as_deref());
        let exhausted_until_ms = compute_exhausted_until_ms(card_exhausted, &flyout_bars, now_ms);
        let mut hover_details = Vec::new();
        if !email.is_empty() {
            hover_details.push(("Account".to_string(), email.to_string()));
        }

        let rank_exhausted = is_opencode_account_exhausted(&u, stale, now_ms);
        let usable_again_at = opencode_usable_again_at(&u, stale, now_ms);
        let last_change_at = row.get("lastChangeAt").and_then(Value::as_i64).unwrap_or(0);

        candidates.push(OpenCodeCandidate {
            rank_exhausted,
            usable_again_at,
            last_change_at,
            item: ProviderItem {
                provider: ProviderKind::OpenCode,
                account_id: email.to_string(),
                title,
                taskbar_bars,
                flyout_bars,
                hover_bars,
                hover_details,
                resets_badge: None,
                exhausted_until_ms,
                spend_rate_text: None,
                spend_rate_severity: BarSeverity::Muted,
                secondary: false,
                display_email: if settings.show_provider_names {
                    None
                } else {
                    Some(email.to_string())
                },
                reset_short,
                subtitle,
                auth_expired,
                stale,
                exhausted: card_exhausted,
                can_open_cursor: false,
                tooltip,
            },
        });
    }

    // Ranking parity with src/lib/ranking.ts:54-60:
    // non-exhausted before exhausted; among exhausted, soonest usableAgainAt first;
    // among non-exhausted, highest lastChangeAt first.
    candidates.sort_by(|a, b| {
        a.rank_exhausted.cmp(&b.rank_exhausted).then_with(|| {
            if a.rank_exhausted {
                a.usable_again_at.cmp(&b.usable_again_at)
            } else {
                b.last_change_at.cmp(&a.last_change_at)
            }
        })
    });

    let total_active = candidates.len();
    let items = candidates
        .into_iter()
        .enumerate()
        .map(|(idx, mut cand)| {
            cand.item.secondary = idx > 0;
            if idx == 0 && total_active > 1 {
                cand.item.hover_details.push((
                    "Accounts".to_string(),
                    format!("{total_active} active accounts"),
                ));
            }
            cand.item
        })
        .collect();
    (items, snoozed)
}

fn build_tooltip(
    name: &str,
    bars: &[BarView],
    rate: Option<&str>,
    subtitle: Option<&str>,
) -> String {
    let mut parts = vec![name.to_string()];
    if let Some(r) = rate {
        parts.push(r.to_string());
    }
    let bar_str: Vec<String> = bars
        .iter()
        .map(|b| match &b.reset_short {
            Some(rs) => format!("{} {} ({rs})", b.label, b.percent_label()),
            None => format!("{} {}", b.label, b.percent_label()),
        })
        .collect();
    if !bar_str.is_empty() {
        parts.push(bar_str.join(" · "));
    }
    if let Some(sub) = subtitle {
        parts.push(sub.to_string());
    }
    let mut lines = vec![parts.join(" — ")];
    for bar in bars {
        if !bar.tooltip_lines.is_empty() {
            lines.push(format!("{}: {}", bar.label, bar.tooltip_lines.join(" · ")));
        }
    }
    lines.join("\n")
}

// ---- SettingsSnapshot & diagnostics helpers -------------------------------------------------

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => {
            let mut out = String::with_capacity(s.len());
            for c in first.to_uppercase() {
                out.push(c);
            }
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

fn claude_plan_label(plan: &str) -> String {
    match plan.trim().to_lowercase().as_str() {
        "max 20x" => "Ultra".to_string(),
        "max 5x" => "Max".to_string(),
        _ => plan.to_string(),
    }
}

fn claude_reset_model_word(label: &str) -> Option<&'static str> {
    for word in label.split(|c: char| !c.is_ascii_alphanumeric()) {
        if word.eq_ignore_ascii_case("fable") {
            return Some("Fable");
        }
        if word.eq_ignore_ascii_case("opus") {
            return Some("Opus");
        }
        if word.eq_ignore_ascii_case("sonnet") {
            return Some("Sonnet");
        }
        if word.eq_ignore_ascii_case("haiku") {
            return Some("Haiku");
        }
    }
    None
}

fn claude_reset_type_name(g: &ClaudeResetGroup) -> String {
    if let Some(model) = claude_reset_model_word(&g.label) {
        return format!("{model} reset");
    }
    match g.kind {
        ClaudeResetKind::Full => "Full reset".to_string(),
        ClaudeResetKind::FiveHour => "5-hour reset".to_string(),
        ClaudeResetKind::Weekly => "Weekly reset".to_string(),
        ClaudeResetKind::Other => "Reset".to_string(),
    }
}

/// `"2 Full · 1 Fable"` (matches `src/lib/claude-resets.ts:resetCellText`), or `None`.
fn claude_resets_cell_text(resets: Option<&ClaudeResets>) -> Option<String> {
    let resets = resets?;
    let mut merged: Vec<(String, bool, i64)> = Vec::new();
    let mut push_group = |name: String, paused: bool, count: i64| {
        if let Some(entry) = merged
            .iter_mut()
            .find(|(n, p, _)| *n == name && *p == paused)
        {
            entry.2 += count;
        } else {
            merged.push((name, paused, count));
        }
    };
    for g in &resets.groups {
        push_group(claude_reset_type_name(g), g.paused, g.count);
    }
    if resets.weekly_five_hour_available == Some(true) {
        push_group("5-hour reset".to_string(), false, 1);
    }
    if merged.is_empty() {
        return None;
    }
    if merged.len() == 1 && merged[0].0 == "Reset" && !merged[0].1 {
        return Some(merged[0].2.to_string());
    }
    let parts: Vec<String> = merged
        .into_iter()
        .map(|(name, paused, count)| {
            let short = name
                .strip_suffix(" reset")
                .or_else(|| name.strip_suffix(" Reset"))
                .unwrap_or(&name);
            if paused {
                format!("{count} {short} (paused)")
            } else {
                format!("{count} {short}")
            }
        })
        .collect();
    Some(parts.join(" · "))
}

/// Compact badge showing resets in hand (`Some("↺ 2")`), or `None` when zero/absent.
fn claude_resets_badge(u: &ClaudeUsage) -> Option<String> {
    let total = u
        .resets
        .as_ref()
        .map(ClaudeResets::total)
        .or(u.reset_credits)
        .unwrap_or(0);
    (total > 0).then(|| format!("↺ {total}"))
}

/// Hover-card reset grants breakdown (e.g. `"1 Fable reset · 1 Full reset"`), or `None`.
fn claude_resets_detail_text(resets: Option<&ClaudeResets>) -> Option<String> {
    let resets = resets?;
    if resets.total() <= 0 {
        return None;
    }
    let mut merged: Vec<(String, i64)> = Vec::new();
    let mut push_group = |name: String, count: i64| {
        if count <= 0 {
            return;
        }
        if let Some(entry) = merged.iter_mut().find(|(n, _)| *n == name) {
            entry.1 += count;
        } else {
            merged.push((name, count));
        }
    };
    for g in &resets.groups {
        if !g.paused {
            push_group(claude_reset_type_name(g), g.count);
        }
    }
    if resets.weekly_five_hour_available == Some(true) {
        push_group("5-hour reset".to_string(), 1);
    }
    if merged.is_empty() {
        return None;
    }
    let parts: Vec<String> = merged
        .into_iter()
        .map(|(name, count)| format!("{count} {name}"))
        .collect();
    Some(parts.join(" · "))
}

fn tracked_warning(row: &Value, active: bool) -> Option<String> {
    if is_auth_expired(row) {
        return Some("re-auth".to_string());
    }
    if active && row.get("error").and_then(Value::as_str) == Some("missing-credentials") {
        return Some("not signed in".to_string());
    }
    None
}

fn cursor_warning(row: &Value, failures: u64) -> Option<String> {
    if is_auth_expired(row) {
        return Some("re-auth".to_string());
    }
    match row.get("error").and_then(Value::as_str)? {
        "missing-credentials" => Some("not signed in".to_string()),
        "sign-in-unreadable" => Some("can't read sign-in".to_string()),
        "schema-changed" | "http" => Some("not updating".to_string()),
        "network" | "rate-limited" | "locked" if failures >= 3 => Some("not updating".to_string()),
        _ => None,
    }
}

fn build_settings_claude_rows(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> Vec<SettingsAccountRow> {
    let hidden_bars = hidden_bars_for(settings, ProviderKind::Claude);
    let mut out: Vec<SettingsAccountRow> = rows
        .iter()
        .map(|row| {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("claude")
                .to_string();
            let raw_active = row.get("active").and_then(Value::as_bool).unwrap_or(false);
            let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
            let cached_at = row.get("cachedAt").and_then(Value::as_i64);
            let auth_expired = is_auth_expired(row);
            let hidden = is_disabled(settings, ProviderKind::Claude, &id);
            let nickname = lookup_display_name(settings, &format!("claude:{id}"));
            let status =
                resolve_subscription_status(settings, ProviderKind::Claude, &id, None, now_ms);
            let active = raw_active && !status.unsubscribed;
            let warning = tracked_warning(row, raw_active);
            let usage: Option<ClaudeUsage> = row
                .get("usage")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let label = row
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| usage.as_ref().and_then(|u| u.email.as_deref()))
                .unwrap_or(&id)
                .to_string();

            let Some(u) = usage else {
                return SettingsAccountRow {
                    provider: ProviderKind::Claude,
                    id,
                    label,
                    nickname,
                    active,
                    hidden,
                    lapsed: status.unsubscribed,
                    auth_expired,
                    stale,
                    plan: status.unsubscribed.then(|| "Unsubscribed".to_string()),
                    warning,
                    bars: Vec::new(),
                    extra_text: None,
                    note: Some("Not signed in or unavailable".to_string()),
                    cached_at,
                    unsubscribed: status.unsubscribed,
                    subscription_ends_at: status.ends_at,
                    subscription_ends_label: status.ends_label,
                    subscription_ends_soon: status.ends_soon,
                };
            };
            if !u.signed_in {
                return SettingsAccountRow {
                    provider: ProviderKind::Claude,
                    id,
                    label,
                    nickname,
                    active,
                    hidden,
                    lapsed: status.unsubscribed,
                    auth_expired,
                    stale,
                    plan: status.unsubscribed.then(|| "Unsubscribed".to_string()),
                    warning,
                    bars: Vec::new(),
                    extra_text: None,
                    note: Some("Not signed in".to_string()),
                    cached_at,
                    unsubscribed: status.unsubscribed,
                    subscription_ends_at: status.ends_at,
                    subscription_ends_label: status.ends_label,
                    subscription_ends_soon: status.ends_soon,
                };
            }

            let plan_raw = u.plan_name.as_deref().unwrap_or(if u.subscribed {
                u.subscription_type.as_deref().unwrap_or("subscribed")
            } else {
                "Free"
            });
            let plan = if status.unsubscribed {
                Some("Unsubscribed".to_string())
            } else {
                Some(claude_plan_label(plan_raw))
            };
            let dimmed = auth_expired || status.unsubscribed;
            let mut raw_bars = vec![
                BarView::new(
                    "fiveHour",
                    "5h",
                    u.five_hour.percent,
                    u.five_hour.resets_at.clone(),
                    now_ms,
                    dimmed,
                ),
                BarView::new(
                    "weekly",
                    "Weekly",
                    u.weekly.percent,
                    u.weekly.resets_at.clone(),
                    now_ms,
                    dimmed,
                ),
            ];
            if let Some(scoped) = u.weekly_scoped.first() {
                if let Some(bar_pct) = raw_bars[1].percent {
                    if !dimmed && settings.fable_shade {
                        raw_bars[1].fable_segment_pct = scoped
                            .share_pct_of_weekly
                            .filter(|&v| v > 0.0 && bar_pct > 0.0)
                            .map(|v| v.min(bar_pct));
                    }
                    if !dimmed {
                        raw_bars[1].tooltip_lines =
                            claude_weekly_tooltip_lines(bar_pct, Some(scoped));
                    }
                }
                raw_bars.push(BarView::new(
                    "scoped",
                    scoped.label.clone(),
                    scoped.percent,
                    scoped.resets_at.clone(),
                    now_ms,
                    dimmed,
                ));
            }
            let bars = filter_hidden_bars(raw_bars, &hidden_bars);
            let extra_text = claude_resets_cell_text(u.resets.as_ref());

            SettingsAccountRow {
                provider: ProviderKind::Claude,
                id,
                label,
                nickname,
                active,
                hidden,
                lapsed: status.unsubscribed,
                auth_expired,
                stale,
                plan,
                warning,
                bars,
                extra_text,
                note: None,
                cached_at,
                unsubscribed: status.unsubscribed,
                subscription_ends_at: status.ends_at,
                subscription_ends_label: status.ends_label,
                subscription_ends_soon: status.ends_soon,
            }
        })
        .collect();
    out.sort_by_key(|r| r.unsubscribed);
    out
}

fn build_settings_codex_rows(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> Vec<SettingsAccountRow> {
    let hidden_bars = hidden_bars_for(settings, ProviderKind::Codex);
    let mut out: Vec<SettingsAccountRow> = rows
        .iter()
        .map(|row| {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("codex")
                .to_string();
            let raw_active = row.get("active").and_then(Value::as_bool).unwrap_or(false);
            let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
            let cached_at = row.get("cachedAt").and_then(Value::as_i64);
            let auth_expired = is_auth_expired(row);
            let hidden = is_disabled(settings, ProviderKind::Codex, &id);
            let nickname = lookup_display_name(settings, &format!("codex:{id}"));
            let status =
                resolve_subscription_status(settings, ProviderKind::Codex, &id, None, now_ms);
            let active = raw_active && !status.unsubscribed;
            let warning = tracked_warning(row, raw_active);
            let usage: Option<CodexUsage> = row
                .get("usage")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let label = row
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| usage.as_ref().and_then(|u| u.email.as_deref()))
                .unwrap_or(&id)
                .to_string();

            let Some(u) = usage else {
                return SettingsAccountRow {
                    provider: ProviderKind::Codex,
                    id,
                    label,
                    nickname,
                    active,
                    hidden,
                    lapsed: status.unsubscribed,
                    auth_expired,
                    stale,
                    plan: status.unsubscribed.then(|| "Unsubscribed".to_string()),
                    warning,
                    bars: Vec::new(),
                    extra_text: None,
                    note: Some("Not signed in or unavailable".to_string()),
                    cached_at,
                    unsubscribed: status.unsubscribed,
                    subscription_ends_at: status.ends_at,
                    subscription_ends_label: status.ends_label,
                    subscription_ends_soon: status.ends_soon,
                };
            };

            let plan = if status.unsubscribed {
                Some("Unsubscribed".to_string())
            } else {
                u.plan_type.as_deref().map(capitalize)
            };
            let dimmed = auth_expired || status.unsubscribed;
            let mut raw_bars = vec![BarView::new(
                "fiveHour",
                "5h",
                u.used_percent,
                u.reset_at.clone(),
                now_ms,
                dimmed,
            )];
            if u.weekly_used_percent.is_some() || u.weekly_reset_at.is_some() {
                raw_bars.push(BarView::new(
                    "weekly",
                    "Weekly",
                    u.weekly_used_percent,
                    u.weekly_reset_at.clone(),
                    now_ms,
                    dimmed,
                ));
            }
            let bars = filter_hidden_bars(raw_bars, &hidden_bars);
            let extra_text = u
                .reset_credits
                .filter(|c| c.is_finite() && *c > 0.0)
                .map(|c| {
                    let n = c.round() as i64;
                    if n == 1 {
                        "1 reset".to_string()
                    } else {
                        format!("{n} resets")
                    }
                });

            SettingsAccountRow {
                provider: ProviderKind::Codex,
                id,
                label,
                nickname,
                active,
                hidden,
                lapsed: status.unsubscribed,
                auth_expired,
                stale,
                plan,
                warning,
                bars,
                extra_text,
                note: None,
                cached_at,
                unsubscribed: status.unsubscribed,
                subscription_ends_at: status.ends_at,
                subscription_ends_label: status.ends_label,
                subscription_ends_soon: status.ends_soon,
            }
        })
        .collect();
    out.sort_by_key(|r| r.unsubscribed);
    out
}

fn build_settings_cursor_rows(
    resp: &Value,
    settings: &Settings,
    now_ms: i64,
) -> Vec<SettingsAccountRow> {
    let Some(rows) = resp.get("accounts").and_then(Value::as_array) else {
        return Vec::new();
    };
    let failures = resp
        .get("attempt")
        .and_then(|a| a.get("failures"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let hidden_bars = hidden_bars_for(settings, ProviderKind::Cursor);

    let mut out: Vec<SettingsAccountRow> = rows
        .iter()
        .map(|row| {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("cursor")
                .to_string();
            let raw_active = row.get("active").and_then(Value::as_bool).unwrap_or(false);
            let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
            let cached_at = row.get("cachedAt").and_then(Value::as_i64);
            let auth_expired = is_auth_expired(row);
            let hidden = is_disabled(settings, ProviderKind::Cursor, &id);
            let nickname = lookup_display_name(settings, &format!("cursor:{id}"));
            let warning = cursor_warning(row, failures);
            let usage: Option<CursorUsage> = row
                .get("usage")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let auto_end_iso = usage.as_ref().and_then(|u| u.resets_at.as_deref());
            let status = resolve_subscription_status(
                settings,
                ProviderKind::Cursor,
                &id,
                auto_end_iso,
                now_ms,
            );
            let active = raw_active && !status.unsubscribed;
            let label = row
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| usage.as_ref().and_then(|u| u.account_name.as_deref()))
                .unwrap_or(&id)
                .to_string();

            let Some(u) = usage else {
                return SettingsAccountRow {
                    provider: ProviderKind::Cursor,
                    id,
                    label,
                    nickname,
                    active,
                    hidden,
                    lapsed: status.unsubscribed,
                    auth_expired,
                    stale,
                    plan: status.unsubscribed.then(|| "Unsubscribed".to_string()),
                    warning,
                    bars: Vec::new(),
                    extra_text: None,
                    note: Some("Not signed in or unavailable".to_string()),
                    cached_at,
                    unsubscribed: status.unsubscribed,
                    subscription_ends_at: status.ends_at,
                    subscription_ends_label: status.ends_label,
                    subscription_ends_soon: status.ends_soon,
                };
            };

            let cycle_passed = is_passed(u.resets_at.as_deref(), now_ms);
            let dimmed = auth_expired || status.unsubscribed;
            let fp_bar = BarView::new(
                "firstParty",
                "1P",
                u.cursor_models_percent_used,
                u.resets_at.clone(),
                now_ms,
                dimmed,
            );
            let mut api_bar = BarView::new(
                "api",
                "API",
                u.other_models_percent_used,
                u.resets_at.clone(),
                now_ms,
                dimmed,
            );
            if !cycle_passed {
                if let Some(ref spend) = u.api_spend {
                    api_bar.tooltip_lines = cursor_api_tooltip_lines(&spend.by_model);
                }
            }
            let raw_bars = vec![fp_bar, api_bar];
            let bars = filter_hidden_bars(raw_bars, &hidden_bars);
            let extra_text = if !cycle_passed {
                u.api_spend.as_ref().map(|s| match s.limit_usd {
                    Some(lim) => {
                        format!(
                            "{} / {}",
                            format_usd_exact(s.spent_usd),
                            format_usd_exact(lim)
                        )
                    }
                    None => format_usd_exact(s.spent_usd),
                })
            } else {
                None
            };
            let plan = if status.unsubscribed {
                Some("Unsubscribed".to_string())
            } else {
                u.plan_name
            };

            SettingsAccountRow {
                provider: ProviderKind::Cursor,
                id,
                label,
                nickname,
                active,
                hidden,
                lapsed: status.unsubscribed,
                auth_expired,
                stale,
                plan,
                warning,
                bars,
                extra_text,
                note: None,
                cached_at,
                unsubscribed: status.unsubscribed,
                subscription_ends_at: status.ends_at,
                subscription_ends_label: status.ends_label,
                subscription_ends_soon: status.ends_soon,
            }
        })
        .collect();
    out.sort_by_key(|r| r.unsubscribed);
    out
}

fn build_settings_grok_rows(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> Vec<SettingsAccountRow> {
    let mut out: Vec<SettingsAccountRow> = rows
        .iter()
        .map(|row| {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("grok")
                .to_string();
            let raw_active = row.get("active").and_then(Value::as_bool).unwrap_or(false);
            let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
            let cached_at = row.get("cachedAt").and_then(Value::as_i64);
            let auth_expired = is_auth_expired(row);
            let hidden = is_disabled(settings, ProviderKind::Grok, &id);
            let nickname = lookup_display_name(settings, &format!("grok:{id}"));
            let warning = tracked_warning(row, raw_active);
            let usage: Option<GrokUsage> = row
                .get("usage")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            // Only treat Grok's reset_at as a monthly subscription end if it is > 8 days away
            // (so weekly quota resets are not mislabeled as subscription expiration).
            let auto_end_iso = usage
                .as_ref()
                .and_then(|u| u.reset_at.as_deref())
                .filter(|iso| parse_iso_ms(iso).is_some_and(|ms| ms > now_ms + 8 * 86_400_000));
            let status = resolve_subscription_status(
                settings,
                ProviderKind::Grok,
                &id,
                auto_end_iso,
                now_ms,
            );
            let active = raw_active && !status.unsubscribed;
            let label = row
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| usage.as_ref().and_then(|u| u.email.as_deref()))
                .unwrap_or(&id)
                .to_string();

            let Some(u) = usage else {
                return SettingsAccountRow {
                    provider: ProviderKind::Grok,
                    id,
                    label,
                    nickname,
                    active,
                    hidden,
                    lapsed: status.unsubscribed,
                    auth_expired,
                    stale,
                    plan: status.unsubscribed.then(|| "Unsubscribed".to_string()),
                    warning,
                    bars: Vec::new(),
                    extra_text: None,
                    note: Some("Not signed in or unavailable".to_string()),
                    cached_at,
                    unsubscribed: status.unsubscribed,
                    subscription_ends_at: status.ends_at,
                    subscription_ends_label: status.ends_label,
                    subscription_ends_soon: status.ends_soon,
                };
            };

            let dimmed = auth_expired || status.unsubscribed;
            let bars = vec![BarView::new(
                "weekly",
                "Weekly",
                u.percent_used,
                u.reset_at.clone(),
                now_ms,
                dimmed,
            )];
            let plan = if status.unsubscribed {
                Some("Unsubscribed".to_string())
            } else {
                u.plan_label.or(u.cursor_plan_name)
            };

            SettingsAccountRow {
                provider: ProviderKind::Grok,
                id,
                label,
                nickname,
                active,
                hidden,
                lapsed: status.unsubscribed,
                auth_expired,
                stale,
                plan,
                warning,
                bars,
                extra_text: None,
                note: None,
                cached_at,
                unsubscribed: status.unsubscribed,
                subscription_ends_at: status.ends_at,
                subscription_ends_label: status.ends_label,
                subscription_ends_soon: status.ends_soon,
            }
        })
        .collect();
    out.sort_by_key(|r| r.unsubscribed);
    out
}

fn build_settings_antigravity_rows(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> Vec<SettingsAccountRow> {
    let hidden_bars = hidden_bars_for(settings, ProviderKind::Antigravity);
    let mut out: Vec<SettingsAccountRow> = rows
        .iter()
        .map(|row| {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("antigravity")
                .to_string();
            let raw_active = row.get("active").and_then(Value::as_bool).unwrap_or(false);
            let auth_expired = is_auth_expired(row);
            let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
            let cached_at = row.get("cachedAt").and_then(Value::as_i64);
            let hidden = is_disabled(settings, ProviderKind::Antigravity, &id);
            let scoped_key = format!("antigravity:{}", id.trim());
            let nickname = lookup_display_name(settings, &scoped_key);
            let status =
                resolve_subscription_status(settings, ProviderKind::Antigravity, &id, None, now_ms);
            let active = raw_active && !status.unsubscribed;
            let warning = auth_expired.then(|| "re-auth".to_string());
            let usage: Option<AntigravityUsage> = row
                .get("usage")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let label = row
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| usage.as_ref().and_then(|u| u.email.clone()))
                .unwrap_or_else(|| id.clone());

            let Some(u) = usage else {
                return SettingsAccountRow {
                    provider: ProviderKind::Antigravity,
                    id,
                    label,
                    nickname,
                    active,
                    hidden,
                    lapsed: status.unsubscribed,
                    auth_expired,
                    stale,
                    plan: status.unsubscribed.then(|| "Unsubscribed".to_string()),
                    warning,
                    bars: Vec::new(),
                    extra_text: None,
                    note: Some("Not signed in or unavailable".to_string()),
                    cached_at,
                    unsubscribed: status.unsubscribed,
                    subscription_ends_at: status.ends_at,
                    subscription_ends_label: status.ends_label,
                    subscription_ends_soon: status.ends_soon,
                };
            };

            let dimmed = auth_expired || status.unsubscribed;
            let mut raw_bars = Vec::new();
            if u.five_hour_percent.is_some() || u.five_hour_resets_at.is_some() {
                raw_bars.push(BarView::new(
                    "fiveHour",
                    "5h",
                    u.five_hour_percent,
                    u.five_hour_resets_at.clone(),
                    now_ms,
                    dimmed,
                ));
            }
            if u.weekly_percent.is_some() || u.weekly_resets_at.is_some() {
                raw_bars.push(BarView::new(
                    "weekly",
                    "Weekly",
                    u.weekly_percent,
                    u.weekly_resets_at.clone(),
                    now_ms,
                    dimmed,
                ));
            }
            let bars = {
                let f = filter_hidden_bars(raw_bars.clone(), &hidden_bars);
                if f.is_empty() {
                    raw_bars
                } else {
                    f
                }
            };

            SettingsAccountRow {
                provider: ProviderKind::Antigravity,
                id,
                label,
                nickname,
                active,
                hidden,
                lapsed: status.unsubscribed,
                auth_expired,
                stale,
                plan: status.unsubscribed.then(|| "Unsubscribed".to_string()),
                warning,
                bars,
                extra_text: None,
                note: None,
                cached_at,
                unsubscribed: status.unsubscribed,
                subscription_ends_at: status.ends_at,
                subscription_ends_label: status.ends_label,
                subscription_ends_soon: status.ends_soon,
            }
        })
        .collect();
    out.sort_by_key(|r| r.unsubscribed);
    out
}

fn build_settings_opencode_rows(
    rows: &[Value],
    settings: &Settings,
    now_ms: i64,
) -> Vec<SettingsAccountRow> {
    let hidden_bars = hidden_bars_for(settings, ProviderKind::OpenCode);
    let mut out: Vec<SettingsAccountRow> = rows
        .iter()
        .map(|row| {
            let email = row.get("email").and_then(Value::as_str).unwrap_or("");
            let enabled = row.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            let hidden = !enabled || is_disabled(settings, ProviderKind::OpenCode, email);
            let api_lapsed = row.get("subscriptionActive").and_then(Value::as_bool) == Some(false);
            let auth_expired = is_auth_expired(row);
            let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
            let cached_at = row.get("cachedAt").and_then(Value::as_i64);
            let nickname = lookup_display_name(settings, email);
            let usage: Option<OpenCodeUsage> = row
                .get("usage")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            // For OpenCode Go, the monthly window reset timestamp IS the subscription cycle end.
            let auto_end_iso = usage
                .as_ref()
                .map(|u| u.monthly.resets_at.as_str())
                .filter(|s| !s.trim().is_empty());
            let status = resolve_subscription_status(
                settings,
                ProviderKind::OpenCode,
                email,
                auto_end_iso,
                now_ms,
            );
            let lapsed = api_lapsed || status.unsubscribed;
            let warning = if auth_expired {
                Some("key rejected".to_string())
            } else if api_lapsed {
                Some("not active".to_string())
            } else if row.get("error").and_then(Value::as_str) == Some("missing-credentials") {
                Some("not signed in".to_string())
            } else {
                None
            };
            let plan = if status.unsubscribed {
                Some("Unsubscribed".to_string())
            } else {
                Some("Go".to_string())
            };

            let Some(u) = usage else {
                return SettingsAccountRow {
                    provider: ProviderKind::OpenCode,
                    id: email.to_string(),
                    label: email.to_string(),
                    nickname,
                    active: !hidden && !lapsed,
                    hidden,
                    lapsed,
                    auth_expired,
                    stale,
                    plan,
                    warning,
                    bars: Vec::new(),
                    extra_text: None,
                    note: Some("Unavailable".to_string()),
                    cached_at,
                    unsubscribed: status.unsubscribed,
                    subscription_ends_at: status.ends_at,
                    subscription_ends_label: status.ends_label,
                    subscription_ends_soon: status.ends_soon,
                };
            };

            let dimmed_all = auth_expired || lapsed;
            let raw_bars = vec![
                BarView::new(
                    "rolling",
                    "Rolling",
                    Some(u.rolling.percent),
                    Some(u.rolling.resets_at.clone()),
                    now_ms,
                    dimmed_all,
                ),
                BarView::new(
                    "weekly",
                    "Weekly",
                    Some(u.weekly.percent),
                    Some(u.weekly.resets_at.clone()),
                    now_ms,
                    dimmed_all,
                ),
                BarView::new(
                    "monthly",
                    "Monthly",
                    Some(u.monthly.percent),
                    Some(u.monthly.resets_at.clone()),
                    now_ms,
                    dimmed_all,
                ),
            ];
            let bars = filter_hidden_bars(raw_bars, &hidden_bars);

            SettingsAccountRow {
                provider: ProviderKind::OpenCode,
                id: email.to_string(),
                label: email.to_string(),
                nickname,
                active: !hidden && !lapsed,
                hidden,
                lapsed,
                auth_expired,
                stale,
                plan,
                warning,
                bars,
                extra_text: None,
                note: None,
                cached_at,
                unsubscribed: status.unsubscribed,
                subscription_ends_at: status.ends_at,
                subscription_ends_label: status.ends_label,
                subscription_ends_soon: status.ends_soon,
            }
        })
        .collect();
    out.sort_by_key(|r| r.unsubscribed);
    out
}

fn format_at_tz<Tz: TimeZone>(tz: &Tz, ms: i64, now_ms: i64) -> String
where
    Tz::Offset: Display,
{
    let Some(dt) = Utc
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.with_timezone(tz))
    else {
        return "?".to_string();
    };
    let same_day = Utc
        .timestamp_millis_opt(now_ms)
        .single()
        .map(|t| t.with_timezone(tz))
        .is_some_and(|now_dt| dt.date_naive() == now_dt.date_naive());
    if same_day {
        dt.format("%H:%M").to_string()
    } else {
        dt.format("%b %-d, %H:%M").to_string()
    }
}

fn format_stamp_local(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms).map_or_else(
        || "?".to_string(),
        |t| {
            t.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        },
    )
}

/// Cursor diagnostics `(label, value, is_bad)` rows matching `src/lib/diagnostics.ts:cursorRows`.
pub fn cursor_diag_rows(attempt: &CursorAttempt, now_ms: i64) -> Vec<(String, String, bool)> {
    cursor_diag_rows_tz(attempt, now_ms, &Local)
}

fn cursor_diag_rows_tz<Tz: TimeZone>(
    attempt: &CursorAttempt,
    now_ms: i64,
    tz: &Tz,
) -> Vec<(String, String, bool)>
where
    Tz::Offset: Display,
{
    let at = |ms: i64| format_at_tz(tz, ms, now_ms);
    let ok = attempt.outcome == "ok";
    let mut rows = vec![(
        "Last attempt".to_string(),
        format!(
            "{} · {}",
            at(attempt.at_ms),
            if ok { "worked" } else { &attempt.outcome }
        ),
        !ok,
    )];

    if let Some(reason) = attempt.reason(tz) {
        rows.push(("Reason".to_string(), reason, true));
    }

    let last_success = match attempt.last_ok_ms {
        None => format!(
            "None since UsageDeck started · {} failed in a row",
            attempt.failures
        ),
        Some(ms) => at(ms),
    };
    rows.push((
        "Last success".to_string(),
        last_success,
        attempt.last_ok_ms.is_none() && attempt.failures > 0,
    ));

    let db_how = match attempt.db.how {
        DbHow::Missing => "Not found",
        DbHow::Opened => "Found · opened read-only",
        DbHow::Copied => "Found · busy, read from a private copy",
        DbHow::Locked => "Found · locked",
        DbHow::Failed => "Found · could not be read",
    };
    let mut db_val = db_how.to_string();
    if let Some(ref detail) = attempt.db.detail {
        db_val.push_str(" (");
        db_val.push_str(detail);
        db_val.push(')');
    }
    if attempt.db.reused_earlier {
        db_val.push_str(" · using the sign-in read at ");
        db_val.push_str(&at(attempt.db.read_at_ms));
    }
    rows.push((
        "state.vscdb".to_string(),
        db_val,
        matches!(attempt.db.how, DbHow::Locked | DbHow::Failed),
    ));

    if !attempt.token.present {
        rows.push((
            "Sign-in".to_string(),
            "No token stored".to_string(),
            attempt.db.how == DbHow::Opened,
        ));
    } else {
        let form = match attempt.token.form.as_deref() {
            Some("jwt") => "plain token",
            Some("opaque") => "plain token (not a JWT)",
            Some("safe-storage-v1") => "encrypted (cursor-safe-storage:v1)",
            Some(other) => other,
            None => "unknown format",
        };
        let mut sign_in_val = format!("Token {form}");
        if let Some(ref decrypt) = attempt.token.decrypt {
            sign_in_val.push_str(" · decrypt: ");
            sign_in_val.push_str(decrypt);
        }
        rows.push((
            "Sign-in".to_string(),
            sign_in_val,
            attempt.token.decrypt.as_deref().is_some_and(|d| d != "ok"),
        ));
        if let Some(ref id) = attempt.token.id {
            let expired = attempt.token.expired(attempt.at_ms).unwrap_or(false);
            let expiry = match attempt.token.expires_at_ms {
                None => "no expiry".to_string(),
                Some(exp) => format!(
                    "{} {}",
                    if expired { "expired" } else { "valid until" },
                    at(exp)
                ),
            };
            rows.push(("Token".to_string(), format!("#{id} · {expiry}"), expired));
        }
    }

    for r in &attempt.requests {
        let good = r.status.is_some_and(|s| (200..300).contains(&s));
        let mut val = r
            .status
            .map_or_else(|| "no reply".to_string(), |s| s.to_string());
        if let Some(ref detail) = r.detail {
            val.push_str(" · ");
            val.push_str(detail);
        }
        rows.push((format!("{} {}", r.method, r.endpoint), val, !good));
    }

    let usage_from = match attempt.auth_path {
        Some(path) => {
            let path_label = match path {
                "api2" => "api2.cursor.sh (bearer token)",
                "cursor.com" => "cursor.com (session cookie)",
                other => other,
            };
            format!(
                "{path_label} · parse: {}",
                attempt.parse.as_deref().unwrap_or("?")
            )
        }
        None => match attempt.parse.as_deref() {
            Some(p) => format!("Nothing · parse: {p}"),
            None => "Nothing".to_string(),
        },
    };
    rows.push((
        "Usage from".to_string(),
        usage_from,
        attempt.auth_path.is_none() && !attempt.requests.is_empty(),
    ));

    rows
}

fn system_label(product: Option<&str>, release: Option<&str>, build: Option<&str>) -> String {
    let eleven = build
        .and_then(|b| b.trim().parse::<u32>().ok())
        .is_some_and(|b| b >= 22_000);
    let mut name = product.unwrap_or("Windows").trim().to_string();
    if eleven {
        name = name.replacen("Windows 10", "Windows 11", 1);
    }
    if let Some(release) = release.map(str::trim).filter(|r| !r.is_empty()) {
        name.push(' ');
        name.push_str(release);
    }
    match build.map(str::trim).filter(|b| !b.is_empty()) {
        Some(build) => format!("{name} (build {build})"),
        None => name,
    }
}

fn current_system_label() -> String {
    let key = crate::win::uninstall::wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let value = |name: &str| {
        crate::win::uninstall::string_value(
            windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
            &key,
            name,
        )
    };
    system_label(
        value("ProductName").as_deref(),
        value("DisplayVersion").as_deref(),
        value("CurrentBuild").as_deref(),
    )
}

fn build_diagnostics_text(
    app_version: &str,
    system: &str,
    now_ms: i64,
    cursor_attempt: Option<&CursorAttempt>,
    spend_report: &str,
) -> String {
    let generated = format_stamp_local(now_ms);
    let cursor_report = cursor_attempt.map_or_else(
        || "No attempt yet since UsageDeck started".to_string(),
        |c| c.report(&Local),
    );
    let claude_resets = crate::providers::claude::last_resets_report().map_or_else(
        || "No usage reply yet since UsageDeck started".to_string(),
        |(at, fields)| format!("Usage reply: {}\n{fields}", format_stamp_local(at)),
    );
    let codex_resets = crate::providers::codex::last_resets_report().map_or_else(
        || "No usage reply yet since UsageDeck started".to_string(),
        |(at, fields)| format!("Fetched: {}\n{fields}", format_stamp_local(at)),
    );
    let header = format!(
        "UsageDeck diagnostics (holds no token, key or account name)\n\
         App: UsageDeck {app_version}\n\
         System: {system}\n\
         Generated: {generated}"
    );
    let sections = [
        ("Cursor", cursor_report.as_str()),
        ("Claude spend", spend_report),
        ("Claude resets", claude_resets.as_str()),
        ("Codex resets", codex_resets.as_str()),
    ];
    let mut blocks = Vec::with_capacity(1 + sections.len());
    blocks.push(header);
    for (title, body) in sections {
        blocks.push(format!("{title}\n{body}"));
    }
    redact(&blocks.join("\n\n"))
}

fn cached_tracked_rows(
    data_dir: &std::path::Path,
    cache_root: Option<&serde_json::Map<String, Value>>,
    provider: &str,
    section: &str,
) -> Vec<Value> {
    let seen = crate::providers::SeenAccounts::new(data_dir.join(crate::store::SEEN_ACCOUNTS_FILE));
    let remembered = seen.remembered(provider);
    let sec = cache_root
        .and_then(|m| m.get(section))
        .and_then(Value::as_object);
    let mut rows = Vec::new();
    let mut emitted = HashSet::new();

    // `SeenAccounts` keeps the newest/most-recent sign-in last; iterate newest first.
    for acc in remembered.into_iter().rev() {
        let entry = sec.and_then(|s| s.get(&acc.id));
        let usage = entry.and_then(|e| e.get("data")).cloned();
        let cached_at = entry.and_then(|e| e.get("savedAt")).and_then(Value::as_i64);
        emitted.insert(acc.id.clone());
        rows.push(serde_json::json!({
            "id": acc.id,
            "label": acc.label,
            "active": false,
            "usage": usage,
            "stale": false,
            "cachedAt": cached_at,
            "authExpired": false,
            "error": Value::Null,
        }));
    }

    // Also include any cached entries not yet recorded in seen-accounts.json.
    if let Some(sec_map) = sec {
        for (key, entry) in sec_map {
            let norm = key.trim().to_lowercase();
            if norm.is_empty() || !emitted.insert(norm.clone()) {
                continue;
            }
            let usage = entry.get("data").cloned();
            if usage.as_ref().is_none_or(Value::is_null) {
                continue;
            }
            let cached_at = entry.get("savedAt").and_then(Value::as_i64);
            rows.push(serde_json::json!({
                "id": norm,
                "label": key,
                "active": false,
                "usage": usage,
                "stale": false,
                "cachedAt": cached_at,
                "authExpired": false,
                "error": Value::Null,
            }));
        }
    }
    rows
}

fn cached_opencode_rows(
    data_dir: &std::path::Path,
    cache_root: Option<&serde_json::Map<String, Value>>,
) -> Vec<Value> {
    let emails: Vec<String> =
        match crate::store::read_json(&data_dir.join(crate::store::ACCOUNTS_FILE)) {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|r| r.get("email").and_then(Value::as_str).map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };
    let sec = cache_root
        .and_then(|m| m.get("opencode"))
        .and_then(Value::as_object);
    emails
        .into_iter()
        .map(|email| {
            let lower = email.trim().to_lowercase();
            let entry = sec.and_then(|s| s.get(&lower).or_else(|| s.get(&email)));
            let usage = entry.and_then(|e| e.get("data")).cloned();
            let cached_at = entry.and_then(|e| e.get("savedAt")).and_then(Value::as_i64);
            serde_json::json!({
                "email": email,
                "enabled": true,
                "usage": usage,
                "stale": false,
                "cachedAt": cached_at,
                "authExpired": false,
                "error": Value::Null,
            })
        })
        .collect()
}

/// Synchronously build a `SettingsSnapshot` from disk caches (`seen-accounts.json`,
/// `usage-cache.json`, `accounts.json`) with zero network calls or subprocesses (< 5ms).
/// Used so the Settings window displays accounts immediately while live refresh runs in the background.
pub fn cached_settings_snapshot(
    data_dir: &std::path::Path,
    settings: Settings,
) -> SettingsSnapshot {
    let now_ms = claude_logs::now_ms();
    let cache_val = crate::store::read_json(&data_dir.join(crate::store::USAGE_CACHE_FILE));
    let cache_root = cache_val.as_ref().and_then(Value::as_object);

    let claude_rows = cached_tracked_rows(data_dir, cache_root, "claude", "claude");
    let codex_rows = cached_tracked_rows(data_dir, cache_root, "codex", "codex");
    let cursor_rows = cached_tracked_rows(data_dir, cache_root, "cursor", "cursor");
    let cursor_resp = serde_json::json!({ "accounts": cursor_rows });
    let grok_rows = cached_tracked_rows(data_dir, cache_root, "grok", "grokbot");
    let opencode_rows = cached_opencode_rows(data_dir, cache_root);
    let antigravity_rows = cached_tracked_rows(data_dir, cache_root, "antigravity", "antigravity");

    let attempt = crate::providers::cursor::last_attempt();
    let diag_rows = attempt
        .as_ref()
        .map_or_else(Vec::new, |a| cursor_diag_rows(a, now_ms));
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let system = current_system_label();
    let diagnostics_text =
        build_diagnostics_text(&app_version, &system, now_ms, attempt.as_ref(), "");

    SettingsSnapshot {
        claude: build_settings_claude_rows(&claude_rows, &settings, now_ms),
        codex: build_settings_codex_rows(&codex_rows, &settings, now_ms),
        cursor: build_settings_cursor_rows(&cursor_resp, &settings, now_ms),
        grok: build_settings_grok_rows(&grok_rows, &settings, now_ms),
        opencode: build_settings_opencode_rows(&opencode_rows, &settings, now_ms),
        antigravity: build_settings_antigravity_rows(&antigravity_rows, &settings, now_ms),
        settings,
        pace: SettingsPaceData::default(),
        cursor_diag_rows: diag_rows,
        diagnostics_text,
        app_version,
    }
}

/// Poll all providers, Claude pace/calibration, and diagnostics for the pure-Rust Settings window.
pub async fn poll_settings_snapshot(core: &Core) -> SettingsSnapshot {
    let settings = core.store.get_settings();
    let now_ms = claude_logs::now_ms();
    let per_provider_timeout = std::time::Duration::from_secs(8);
    let data_dir = core.store.dir().to_path_buf();

    let (claude_res, cursor_res, codex_res, grok_res, opencode_res, antigravity_res) = tokio::join!(
        tokio::time::timeout(
            per_provider_timeout,
            crate::providers::claude::accounts(core)
        ),
        tokio::time::timeout(
            per_provider_timeout,
            crate::providers::cursor::response(core)
        ),
        tokio::time::timeout(
            per_provider_timeout,
            crate::providers::codex::accounts(core)
        ),
        tokio::time::timeout(per_provider_timeout, crate::providers::grok::accounts(core)),
        tokio::time::timeout(
            per_provider_timeout,
            crate::providers::opencode::accounts(core)
        ),
        tokio::time::timeout(
            per_provider_timeout,
            crate::providers::antigravity::accounts(core)
        ),
    );

    let need_fallback = claude_res.is_err()
        || cursor_res.is_err()
        || codex_res.is_err()
        || grok_res.is_err()
        || opencode_res.is_err()
        || antigravity_res.is_err();
    let cache_val = need_fallback
        .then(|| crate::store::read_json(&data_dir.join(crate::store::USAGE_CACHE_FILE)))
        .flatten();
    let cache_root = cache_val.as_ref().and_then(Value::as_object);

    let claude_rows = claude_res
        .unwrap_or_else(|_| cached_tracked_rows(&data_dir, cache_root, "claude", "claude"));
    let cursor_resp = cursor_res.unwrap_or_else(|_| {
        serde_json::json!({
            "accounts": cached_tracked_rows(&data_dir, cache_root, "cursor", "cursor")
        })
    });
    let codex_rows =
        codex_res.unwrap_or_else(|_| cached_tracked_rows(&data_dir, cache_root, "codex", "codex"));
    let grok_rows =
        grok_res.unwrap_or_else(|_| cached_tracked_rows(&data_dir, cache_root, "grok", "grokbot"));
    let opencode_rows =
        opencode_res.unwrap_or_else(|_| cached_opencode_rows(&data_dir, cache_root));
    let antigravity_rows = antigravity_res.unwrap_or_else(|_| {
        cached_tracked_rows(&data_dir, cache_root, "antigravity", "antigravity")
    });

    let spend = core.spend.spend(now_ms);
    let spend_report = spend.report();
    let total_24h: f64 = spend.by_model.iter().map(|m| m.usd24h).sum();
    let by_model: Vec<(String, f64, f64)> = spend
        .by_model
        .iter()
        .map(|m| {
            let share = if total_24h > 0.0 {
                m.usd24h / total_24h
            } else {
                0.0
            };
            (m.model.clone(), m.usd24h, share)
        })
        .collect();

    let readings = history::load_all(&core.store.file(history::HISTORY_FILE));
    let claude_hist: Vec<history::Reading> =
        readings.into_iter().filter(|r| r.p == "claude").collect();
    let active_hist = claude_hist.last().map(|r| r.a.clone());
    let others = active_hist
        .as_deref()
        .map_or_else(Vec::new, |a| history::other_account_spans(&claude_hist, a));
    let mine: Vec<history::Reading> = claude_hist
        .into_iter()
        .filter(|r| Some(&r.a) == active_hist.as_ref())
        .collect();
    let weekly = history::points_of(&mine, history::WEEKLY);
    let usd_per_weekly_pct = history::usd_per_weekly_pct(&weekly, now_ms, |t| {
        let other: f64 = others
            .iter()
            .filter(|(_, end)| *end > t)
            .map(|&(start, end)| core.spend.spend_between(start.max(t), end))
            .sum();
        (core.spend.spend_between(t, now_ms) - other).max(0.0)
    });

    let pace = SettingsPaceData {
        last_hour: spend.last_hour,
        last_5h: spend.last5h,
        today: spend.today,
        last_7d: spend.last7d,
        backfilling: spend.backfilling,
        minutes: spend.minutes,
        minutes_fable: spend.minutes_fable,
        quarters: spend.quarters,
        quarters_fable: spend.quarters_fable,
        by_model,
        unpriced_models: spend.unpriced_models,
        unpriced_replies: spend.unpriced_replies,
        usd_per_weekly_pct,
    };

    let attempt = crate::providers::cursor::last_attempt();
    let diag_rows = attempt
        .as_ref()
        .map_or_else(Vec::new, |a| cursor_diag_rows(a, now_ms));
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    let system = current_system_label();
    let diagnostics_text = build_diagnostics_text(
        &app_version,
        &system,
        now_ms,
        attempt.as_ref(),
        &spend_report,
    );

    let claude = build_settings_claude_rows(&claude_rows, &settings, now_ms);
    let codex = build_settings_codex_rows(&codex_rows, &settings, now_ms);
    let cursor = build_settings_cursor_rows(&cursor_resp, &settings, now_ms);
    let grok = build_settings_grok_rows(&grok_rows, &settings, now_ms);
    let opencode = build_settings_opencode_rows(&opencode_rows, &settings, now_ms);
    let antigravity = build_settings_antigravity_rows(&antigravity_rows, &settings, now_ms);

    SettingsSnapshot {
        settings,
        claude,
        codex,
        cursor,
        grok,
        opencode,
        antigravity,
        pace,
        cursor_diag_rows: diag_rows,
        diagnostics_text,
        app_version,
    }
}

/// Realistic preview snapshot for `--demo` mode so the taskbar strip and flyout
/// can be inspected even on a machine without live provider sign-ins.
pub fn demo_snapshot(now_ms: i64) -> TaskbarSnapshot {
    let iso_in = |mins: i64| -> Option<String> {
        DateTime::<Utc>::from_timestamp_millis(now_ms + mins * 60_000).map(|dt| dt.to_rfc3339())
    };

    let mut claude_5h = BarView::new("fiveHour", "5h", Some(64.0), iso_in(134), now_ms, false);
    claude_5h.fable_segment_pct = Some(16.0);
    claude_5h.tooltip_lines = vec!["Opus 5.5 ≈48%".to_string(), "Fable 5.2 ≈16%".to_string()];
    let mut claude_weekly = BarView::new(
        "weekly",
        "Weekly",
        Some(83.0),
        iso_in(4 * 24 * 60),
        now_ms,
        false,
    );
    claude_weekly.fable_segment_pct = Some(21.0);
    claude_weekly.tooltip_lines = vec!["Fable 21%".to_string(), "Other models 62%".to_string()];
    let claude_bars = vec![
        claude_5h,
        claude_weekly,
        BarView::new(
            "scoped",
            "Fable",
            Some(32.0),
            iso_in(4 * 24 * 60),
            now_ms,
            false,
        ),
    ];

    let mut cursor_api = BarView::new("api", "API", Some(54.0), iso_in(9 * 24 * 60), now_ms, false);
    cursor_api.tooltip_lines = vec![
        "claude-4.6-opus $7.20".to_string(),
        "gpt-5 $3.60".to_string(),
    ];
    let cursor_bars = vec![
        BarView::new(
            "firstParty",
            "1P",
            Some(38.0),
            iso_in(9 * 24 * 60),
            now_ms,
            false,
        ),
        cursor_api,
    ];
    let codex_bars = vec![
        BarView::new("fiveHour", "5h", Some(100.0), iso_in(220), now_ms, false),
        BarView::new(
            "weekly",
            "Weekly",
            Some(41.0),
            iso_in(5 * 24 * 60),
            now_ms,
            false,
        ),
    ];
    let opencode_bars = vec![
        BarView::new("rolling", "Rolling", Some(28.0), iso_in(185), now_ms, false),
        BarView::new(
            "weekly",
            "Weekly",
            Some(52.0),
            iso_in(3 * 24 * 60),
            now_ms,
            false,
        ),
        BarView::new(
            "monthly",
            "Monthly",
            Some(19.0),
            iso_in(14 * 24 * 60),
            now_ms,
            false,
        ),
    ];

    let claude_tooltip = build_tooltip("Claude", &claude_bars, Some("$42/h"), None);
    let cursor_tooltip = build_tooltip("Cursor", &cursor_bars, None, Some("API $10.8/$20"));
    let codex_tooltip = build_tooltip("Codex", &codex_bars, None, None);
    let opencode_tooltip =
        build_tooltip("OpenCode", &opencode_bars, None, Some("demo@opencode.ai"));
    let codex_exhausted_until = compute_exhausted_until_ms(true, &codex_bars, now_ms);

    TaskbarSnapshot {
        items: vec![
            ProviderItem {
                provider: ProviderKind::Claude,
                account_id: "demo@claude.ai".into(),
                title: "Claude".into(),
                taskbar_bars: pick_taskbar_bars(&claude_bars),
                flyout_bars: claude_bars.clone(),
                hover_bars: claude_bars,
                hover_details: vec![
                    (
                        "Spend".into(),
                        "1h $42 · 5h $115 · Today $210 · 7d $1,120".into(),
                    ),
                    ("Resets".into(), "1 Fable reset · 1 Full reset".into()),
                    ("Plan".into(), "Ultra".into()),
                    ("Account".into(), "demo@claude.ai".into()),
                ],
                resets_badge: Some("↺ 2".into()),
                exhausted_until_ms: None,
                spend_rate_text: Some("$42/h".into()),
                spend_rate_severity: BarSeverity::Muted,
                secondary: false,
                display_email: Some("demo@claude.ai".into()),
                reset_short: Some("2h14m".into()),
                subtitle: None,
                auth_expired: false,
                stale: false,
                exhausted: false,
                can_open_cursor: false,
                tooltip: claude_tooltip,
            },
            ProviderItem {
                provider: ProviderKind::Cursor,
                account_id: "demo@cursor.sh".into(),
                title: "Cursor".into(),
                taskbar_bars: pick_taskbar_bars(&cursor_bars),
                flyout_bars: cursor_bars.clone(),
                hover_bars: cursor_bars,
                hover_details: vec![
                    ("API spend".into(), "$10.80 / $20.00".into()),
                    ("On-demand".into(), "$4.20 / $50.00".into()),
                    ("Plan".into(), "Pro".into()),
                    ("Account".into(), "demo@cursor.sh".into()),
                ],
                resets_badge: None,
                exhausted_until_ms: None,
                spend_rate_text: None,
                spend_rate_severity: BarSeverity::Muted,
                secondary: false,
                display_email: Some("demo@cursor.sh".into()),
                reset_short: Some("9d".into()),
                subtitle: Some("API $10.8/$20".into()),
                auth_expired: false,
                stale: false,
                exhausted: false,
                can_open_cursor: false,
                tooltip: cursor_tooltip,
            },
            ProviderItem {
                provider: ProviderKind::Codex,
                account_id: "demo@openai.com".into(),
                title: "Codex".into(),
                taskbar_bars: pick_taskbar_bars(&codex_bars),
                flyout_bars: codex_bars.clone(),
                hover_bars: codex_bars,
                hover_details: vec![
                    ("Plan".into(), "Plus".into()),
                    ("Account".into(), "demo@openai.com".into()),
                ],
                resets_badge: None,
                exhausted_until_ms: codex_exhausted_until,
                spend_rate_text: None,
                spend_rate_severity: BarSeverity::Muted,
                secondary: false,
                display_email: Some("demo@openai.com".into()),
                reset_short: Some("3h40m".into()),
                subtitle: None,
                auth_expired: false,
                stale: false,
                exhausted: true,
                can_open_cursor: false,
                tooltip: codex_tooltip,
            },
            ProviderItem {
                provider: ProviderKind::OpenCode,
                account_id: "demo@opencode.ai".into(),
                title: "OpenCode".into(),
                taskbar_bars: pick_taskbar_bars(&opencode_bars),
                flyout_bars: opencode_bars.clone(),
                hover_bars: opencode_bars,
                hover_details: vec![("Account".into(), "demo@opencode.ai".into())],
                resets_badge: None,
                exhausted_until_ms: None,
                spend_rate_text: None,
                spend_rate_severity: BarSeverity::Muted,
                secondary: false,
                display_email: Some("demo@opencode.ai".into()),
                reset_short: Some("3h05m".into()),
                subtitle: Some("demo@opencode.ai".into()),
                auth_expired: false,
                stale: false,
                exhausted: false,
                can_open_cursor: false,
                tooltip: opencode_tooltip,
            },
        ],
        updated_at_ms: now_ms,
        hide_in_fullscreen: true,
        is_demo: true,
        theme_mode: ThemeMode::Auto,
        snoozed_count: 0,
        taskbar_meter_style: TaskbarMeterStyle::default(),
        system_monitor_enabled: true,
        system: Some(crate::sysmon::demo_system_snapshot()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_700_000_000_000;

    fn iso_in(mins: i64) -> String {
        DateTime::<Utc>::from_timestamp_millis(NOW + mins * 60_000)
            .unwrap()
            .to_rfc3339()
    }

    #[test]
    fn formats_reset_durations_compactly() {
        assert_eq!(format_reset_short(&iso_in(-5), NOW), None);
        assert_eq!(format_reset_short(&iso_in(45), NOW).as_deref(), Some("45m"));
        assert_eq!(format_reset_short(&iso_in(120), NOW).as_deref(), Some("2h"));
        assert_eq!(
            format_reset_short(&iso_in(134), NOW).as_deref(),
            Some("2h14m")
        );
        assert_eq!(
            format_reset_short(&iso_in(2 * 24 * 60 + 5 * 60), NOW).as_deref(),
            Some("2d5h")
        );
        assert_eq!(
            format_reset_short(&iso_in(9 * 24 * 60), NOW).as_deref(),
            Some("9d")
        );
    }

    #[test]
    fn formats_spend_rates() {
        assert_eq!(format_rate_usd(0.0), None);
        assert_eq!(format_rate_usd(0.49), None);
        assert_eq!(format_rate_usd(4.5).as_deref(), Some("$4.5/h"));
        assert_eq!(format_rate_usd(41.8).as_deref(), Some("$42/h"));
    }

    #[test]
    fn spend_rate_severity_follows_80_to_120_target_band() {
        assert_eq!(spend_rate_severity(50.0, None), BarSeverity::Muted);
        assert_eq!(spend_rate_severity(0.4, Some(50)), BarSeverity::Muted);
        assert_eq!(spend_rate_severity(39.9, Some(50)), BarSeverity::Warn);
        assert_eq!(spend_rate_severity(40.0, Some(50)), BarSeverity::Ok);
        assert_eq!(spend_rate_severity(60.0, Some(50)), BarSeverity::Ok);
        assert_eq!(spend_rate_severity(60.1, Some(50)), BarSeverity::Danger);
    }

    #[test]
    fn model_label_matches_pace_ts() {
        assert_eq!(model_label("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_label("claude-fable-5-2-20261001"), "Fable 5.2");
        assert_eq!(model_label("claude-haiku-5"), "Haiku 5");
        assert_eq!(model_label("gpt-5"), "gpt-5");
    }

    #[test]
    fn picks_two_most_urgent_bars_for_taskbar() {
        let bars = vec![
            BarView::new("rolling", "Rolling", Some(20.0), None, NOW, false),
            BarView::new("weekly", "Weekly", Some(45.0), None, NOW, false),
            BarView::new("monthly", "Monthly", Some(91.0), None, NOW, false),
        ];
        let picked = pick_taskbar_bars(&bars);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].key, "rolling");
        assert_eq!(picked[1].key, "monthly");
    }

    #[test]
    fn bar_view_computes_reset_at_local_and_pace_pct() {
        let base = Utc
            .with_ymd_and_hms(2026, 6, 1, 10, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            format_reset_local_tz(&Utc, "2026-06-01T13:30:00Z", base).as_deref(),
            Some("Today 1:30 PM")
        );
        assert_eq!(
            format_reset_local_tz(&Utc, "2026-06-02T13:30:00Z", base).as_deref(),
            Some("Tomorrow 1:30 PM")
        );
        assert_eq!(
            format_reset_local_tz(&Utc, "2026-06-04T13:30:00Z", base).as_deref(),
            Some("Thu 1:30 PM")
        );
        assert_eq!(
            format_reset_local_tz(&Utc, "2026-06-10T13:30:00Z", base).as_deref(),
            Some("Jun 10, 1:30 PM")
        );
        assert_eq!(
            format_reset_local_tz(&Utc, "2026-06-01T09:55:00Z", base),
            None
        );

        let five_h = BarView::new("fiveHour", "5h", Some(40.0), Some(iso_in(150)), NOW, false);
        assert!(five_h.reset_at_local.is_some());
        assert!((five_h.pace_pct.unwrap() - 50.0).abs() < 1e-6);

        let weekly = BarView::new(
            "weekly",
            "Weekly",
            Some(60.0),
            Some(iso_in(7 * 24 * 60 / 4)),
            NOW,
            false,
        );
        assert!((weekly.pace_pct.unwrap() - 75.0).abs() < 1e-6);

        let monthly = BarView::new(
            "monthly",
            "Monthly",
            Some(30.0),
            Some(iso_in(15 * 24 * 60)),
            NOW,
            false,
        );
        assert!((monthly.pace_pct.unwrap() - 50.0).abs() < 1e-6);

        let api = BarView::new(
            "api",
            "API",
            Some(25.0),
            Some(iso_in(15 * 24 * 60)),
            NOW,
            false,
        );
        assert!((api.pace_pct.unwrap() - 50.0).abs() < 1e-6);

        // Past reset clears both reset_at_local and pace_pct.
        let passed = BarView::new("fiveHour", "5h", Some(80.0), Some(iso_in(-5)), NOW, false);
        assert_eq!(passed.reset_at_local, None);
        assert_eq!(passed.pace_pct, None);

        // Remaining >= window_ms yields None pace_pct.
        let beyond = BarView::new("fiveHour", "5h", Some(10.0), Some(iso_in(360)), NOW, false);
        assert_eq!(beyond.pace_pct, None);

        // Unknown bar key yields None pace_pct.
        let unknown = BarView::new("custom", "Custom", Some(10.0), Some(iso_in(60)), NOW, false);
        assert_eq!(unknown.pace_pct, None);
    }

    #[test]
    fn opencode_filters_lapsed_subscriptions_and_bare_disabled_emails() {
        let rows = vec![
            json!({
                "email": "lapsed@example.com",
                "enabled": true,
                "subscriptionActive": false,
                "stale": false,
                "lastChangeAt": 500,
                "usage": {
                    "rolling": { "status": "ok", "percent": 10.0, "resetsAt": iso_in(60) },
                    "weekly":  { "status": "ok", "percent": 20.0, "resetsAt": iso_in(1440) },
                    "monthly": { "status": "ok", "percent": 30.0, "resetsAt": iso_in(10000) }
                }
            }),
            json!({
                "email": "Hidden@Example.com",
                "enabled": true,
                "subscriptionActive": true,
                "stale": false,
                "lastChangeAt": 600,
                "usage": {
                    "rolling": { "status": "ok", "percent": 10.0, "resetsAt": iso_in(60) },
                    "weekly":  { "status": "ok", "percent": 20.0, "resetsAt": iso_in(1440) },
                    "monthly": { "status": "ok", "percent": 30.0, "resetsAt": iso_in(10000) }
                }
            }),
            json!({
                "email": "active-old@example.com",
                "enabled": true,
                "subscriptionActive": null,
                "stale": false,
                "lastChangeAt": 100,
                "usage": {
                    "rolling": { "status": "ok", "percent": 15.0, "resetsAt": iso_in(60) },
                    "weekly":  { "status": "ok", "percent": 25.0, "resetsAt": iso_in(1440) },
                    "monthly": { "status": "ok", "percent": 35.0, "resetsAt": iso_in(10000) }
                }
            }),
            json!({
                "email": "active-new@example.com",
                "enabled": true,
                "subscriptionActive": true,
                "stale": false,
                "lastChangeAt": 400,
                "usage": {
                    "rolling": { "status": "ok", "percent": 5.0, "resetsAt": iso_in(60) },
                    "weekly":  { "status": "ok", "percent": 10.0, "resetsAt": iso_in(1440) },
                    "monthly": { "status": "ok", "percent": 15.0, "resetsAt": iso_in(10000) }
                }
            }),
        ];

        let mut settings = Settings::default();
        // Bare lowercased email disables OpenCode account.
        settings
            .disabled_accounts
            .push("hidden@example.com".to_string());

        let (items, snoozed) = build_opencode_items(&rows, &settings, NOW);
        assert_eq!(snoozed, 0);
        assert_eq!(items.len(), 2);
        // Most recently changed non-exhausted account ranks first and is primary.
        assert_eq!(items[0].account_id, "active-new@example.com");
        assert!(!items[0].secondary);
        assert_eq!(items[0].hover_bars.len(), 3);
        assert_eq!(
            items[0].hover_details,
            vec![
                ("Account".to_string(), "active-new@example.com".to_string()),
                ("Accounts".to_string(), "2 active accounts".to_string()),
            ]
        );
        assert_eq!(items[1].account_id, "active-old@example.com");
        assert!(items[1].secondary);
        assert_eq!(
            items[1].hover_details,
            vec![("Account".to_string(), "active-old@example.com".to_string())]
        );
    }

    #[test]
    fn opencode_ranks_exhausted_by_soonest_usable_again_and_dims_shorter_windows() {
        let rows = vec![
            json!({
                "email": "later@example.com",
                "enabled": true,
                "subscriptionActive": true,
                "stale": false,
                "lastChangeAt": 999,
                "usage": {
                    "rolling": { "status": "ok", "percent": 40.0, "resetsAt": iso_in(30) },
                    "weekly":  { "status": "exhausted", "percent": 100.0, "resetsAt": iso_in(120) },
                    "monthly": { "status": "exhausted", "percent": 100.0, "resetsAt": iso_in(5000) }
                }
            }),
            json!({
                "email": "sooner@example.com",
                "enabled": true,
                "subscriptionActive": true,
                "stale": false,
                "lastChangeAt": 100,
                "usage": {
                    "rolling": { "status": "ok", "percent": 40.0, "resetsAt": iso_in(30) },
                    "weekly":  { "status": "exhausted", "percent": 100.0, "resetsAt": iso_in(600) },
                    "monthly": { "status": "ok", "percent": 50.0, "resetsAt": iso_in(10000) }
                }
            }),
        ];

        let (items, snoozed) = build_opencode_items(&rows, &Settings::default(), NOW);
        assert_eq!(snoozed, 0);
        assert_eq!(items.len(), 2);
        // `sooner` waits 600m (weekly); `later` waits 5000m (latest of weekly+monthly).
        assert_eq!(items[0].account_id, "sooner@example.com");
        assert_eq!(items[1].account_id, "later@example.com");
        assert_eq!(
            items[0].exhausted_until_ms,
            Some((NOW + 600 * 60_000) as u64)
        );
        assert_eq!(
            items[1].exhausted_until_ms,
            Some((NOW + 5000 * 60_000) as u64)
        );

        // On `sooner`: weekly full -> rolling dimmed, weekly Danger, monthly Ok.
        assert_eq!(items[0].flyout_bars[0].severity, BarSeverity::Muted);
        assert_eq!(items[0].flyout_bars[1].severity, BarSeverity::Danger);
        assert_eq!(items[0].flyout_bars[2].severity, BarSeverity::Ok);

        // On `later`: monthly full -> rolling AND weekly dimmed, monthly Danger.
        assert_eq!(items[1].flyout_bars[0].severity, BarSeverity::Muted);
        assert_eq!(items[1].flyout_bars[1].severity, BarSeverity::Muted);
        assert_eq!(items[1].flyout_bars[2].severity, BarSeverity::Danger);
    }

    #[test]
    fn claude_fable_shade_tooltips_and_exhausted_excludes_fable() {
        let rows = vec![json!({
            "id": "alice@claude.ai",
            "label": "alice@claude.ai",
            "active": true,
            "stale": false,
            "authExpired": false,
            "usage": {
                "email": "alice@claude.ai",
                "displayName": "Alice",
                "signedIn": true,
                "subscribed": true,
                "subscriptionType": "max",
                "planName": "Max 20x",
                "fiveHour": { "percent": 80.0, "resetsAt": iso_in(180) },
                "weekly":   { "percent": 82.0, "resetsAt": iso_in(4 * 24 * 60) },
                "weeklyOpus": null,
                "weeklySonnet": null,
                "weeklyScoped": [{
                    "label": "Fable",
                    "percent": 100.0,
                    "resetsAt": iso_in(4 * 24 * 60),
                    "sharePctOfWeekly": 23.0,
                    "allowancePctOfWeekly": 50.0
                }],
                "resetCredits": 2,
                "resets": {
                    "groups": [
                        {
                            "kind": "weekly",
                            "label": "Fable weekly",
                            "count": 1,
                            "earliestExpiry": null,
                            "paused": false
                        },
                        {
                            "kind": "full",
                            "label": "Full reset",
                            "count": 1,
                            "earliestExpiry": null,
                            "paused": false
                        }
                    ],
                    "weeklyFiveHourAvailable": false
                },
                "billingType": null,
                "rateLimitTier": null,
                "orgUuid": null
            }
        })];

        let spend_info = ClaudeSpendInfo {
            rate_text: Some("$42/h".to_string()),
            rate_severity: BarSeverity::Ok,
            summary: Some("1h $42 · 5h $115 · Today $210 · 7d $1,120".to_string()),
        };
        let empty_spend = ClaudeSpendInfo {
            rate_text: None,
            rate_severity: BarSeverity::Muted,
            summary: None,
        };

        let mut settings = Settings::default();
        // Hide fiveHour so flyout_bars is [weekly, scoped] — previously `.take(2)` hit Fable!
        settings
            .hidden_bars
            .insert(WidgetProvider::Claude, vec!["fiveHour".to_string()]);

        let (items, snoozed) = build_claude_items(&rows, &settings, &spend_info, NOW, |_| {
            (
                Some(ModelShare {
                    usd: 25.0,
                    total_usd: 100.0,
                }),
                vec![
                    ModelUsd {
                        model: "claude-opus-5-5".to_string(),
                        usd: 60.0,
                    },
                    ModelUsd {
                        model: "claude-fable-5-2".to_string(),
                        usd: 40.0,
                    },
                ],
            )
        });
        assert_eq!(snoozed, 0);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        // Fable is 100% but weekly is 82% -> Claude must NOT be marked exhausted.
        assert!(!item.exhausted);
        assert_eq!(item.exhausted_until_ms, None);
        assert_eq!(item.flyout_bars.len(), 2);
        assert_eq!(item.flyout_bars[0].key, "weekly");
        // hover_bars respects settings.hidden_bars (fiveHour is hidden here).
        assert_eq!(item.hover_bars.len(), 2);
        assert_eq!(item.hover_bars[0].key, "weekly");
        assert_eq!(item.hover_bars[1].key, "scoped");
        assert_eq!(item.resets_badge.as_deref(), Some("↺ 2"));
        assert_eq!(
            item.hover_details,
            vec![
                (
                    "Spend".to_string(),
                    "1h $42 · 5h $115 · Today $210 · 7d $1,120".to_string()
                ),
                (
                    "Resets".to_string(),
                    "1 Fable reset · 1 Full reset".to_string()
                ),
                ("Plan".to_string(), "Ultra".to_string()),
                ("Account".to_string(), "alice@claude.ai".to_string()),
            ]
        );
        assert_eq!(item.flyout_bars[0].fable_segment_pct, Some(23.0));
        assert_eq!(
            item.flyout_bars[0].tooltip_lines,
            vec!["Fable 23%", "Other models 59%"]
        );
        assert!(item
            .tooltip
            .contains("Weekly: Fable 23% · Other models 59%"));

        // Now with all bars visible: check 5h Fable shade & per-model tooltip lines.
        let (items_all, _) =
            build_claude_items(&rows, &Settings::default(), &spend_info, NOW, |_| {
                (
                    Some(ModelShare {
                        usd: 25.0,
                        total_usd: 100.0,
                    }),
                    vec![
                        ModelUsd {
                            model: "claude-opus-5-5".to_string(),
                            usd: 60.0,
                        },
                        ModelUsd {
                            model: "claude-fable-5-2".to_string(),
                            usd: 40.0,
                        },
                    ],
                )
            });
        let bar_5h = &items_all[0].flyout_bars[0];
        assert_eq!(bar_5h.key, "fiveHour");
        assert_eq!(bar_5h.fable_segment_pct, Some(20.0)); // 80% * (25/100)
        assert_eq!(
            bar_5h.tooltip_lines,
            vec!["Opus 5.5 ≈48%", "Fable 5.2 ≈32%"]
        );

        // When fable_shade is disabled, segment_pct must be None.
        let no_shade = Settings {
            fable_shade: false,
            ..Settings::default()
        };
        let (items_no_shade, _) = build_claude_items(&rows, &no_shade, &empty_spend, NOW, |_| {
            (
                Some(ModelShare {
                    usd: 25.0,
                    total_usd: 100.0,
                }),
                Vec::new(),
            )
        });
        assert_eq!(items_no_shade[0].flyout_bars[0].fable_segment_pct, None);
        assert_eq!(items_no_shade[0].flyout_bars[1].fable_segment_pct, None);
    }

    #[test]
    fn snooze_comes_back_early_when_no_longer_exhausted_and_matches_case_insensitively() {
        let mut settings = Settings::default();
        settings.snoozed_until.insert(
            "Codex:User@OpenAI.com".to_string(),
            (NOW + 3_600_000) as u64,
        );

        // While exhausted -> hidden by snooze (case-insensitive key match).
        let exhausted_rows = vec![json!({
            "id": "user@openai.com",
            "active": true,
            "stale": false,
            "usage": {
                "email": "user@openai.com",
                "planType": "plus",
                "usedPercent": 100.0,
                "resetAt": iso_in(60),
                "limitReached": true
            }
        })];
        let (hidden_items, hidden_count) = build_codex_items(&exhausted_rows, &settings, NOW);
        assert!(hidden_items.is_empty());
        assert_eq!(hidden_count, 1);

        // Window resets early (now 10%, not exhausted) -> card returns immediately!
        let reset_rows = vec![json!({
            "id": "user@openai.com",
            "active": true,
            "stale": false,
            "usage": {
                "email": "user@openai.com",
                "planType": "plus",
                "usedPercent": 10.0,
                "resetAt": iso_in(300),
                "limitReached": false
            }
        })];
        let (items, snoozed) = build_codex_items(&reset_rows, &settings, NOW);
        assert_eq!(snoozed, 0);
        assert_eq!(items.len(), 1);
        assert!(!items[0].exhausted);
        assert_eq!(items[0].exhausted_until_ms, None);
        assert_eq!(
            items[0].hover_details,
            vec![
                ("Plan".to_string(), "Plus".to_string()),
                ("Account".to_string(), "user@openai.com".to_string()),
            ]
        );
    }

    #[test]
    fn auto_hide_exhausted_hides_and_restores_cards_and_counts_snoozed() {
        let settings = Settings {
            auto_hide_exhausted: true,
            ..Settings::default()
        };
        let exhausted_rows = vec![json!({
            "id": "dev@openai.com",
            "active": true,
            "stale": false,
            "usage": {
                "email": "dev@openai.com",
                "planType": "pro",
                "usedPercent": 100.0,
                "resetAt": iso_in(90),
                "weeklyUsedPercent": 40.0,
                "weeklyResetAt": iso_in(5 * 24 * 60),
                "limitReached": true
            }
        })];
        // With auto_hide_exhausted = false, card is visible and carries exhausted_until_ms.
        let (visible, snoozed_off) = build_codex_items(&exhausted_rows, &Settings::default(), NOW);
        assert_eq!(snoozed_off, 0);
        assert_eq!(visible.len(), 1);
        assert!(visible[0].exhausted);
        assert_eq!(
            visible[0].exhausted_until_ms,
            Some((NOW + 90 * 60_000) as u64)
        );
        assert_eq!(visible[0].hover_bars.len(), 2);

        // With auto_hide_exhausted = true, exhausted card is hidden and snoozed count increments.
        let (hidden, snoozed_on) = build_codex_items(&exhausted_rows, &settings, NOW);
        assert!(hidden.is_empty());
        assert_eq!(snoozed_on, 1);

        // Once usage drops below 100% (no longer exhausted), card returns immediately.
        let recovered_rows = vec![json!({
            "id": "dev@openai.com",
            "active": true,
            "stale": false,
            "usage": {
                "email": "dev@openai.com",
                "planType": "pro",
                "usedPercent": 25.0,
                "resetAt": iso_in(290),
                "weeklyUsedPercent": 40.0,
                "weeklyResetAt": iso_in(5 * 24 * 60),
                "limitReached": false
            }
        })];
        let (recovered, snoozed_after) = build_codex_items(&recovered_rows, &settings, NOW);
        assert_eq!(snoozed_after, 0);
        assert_eq!(recovered.len(), 1);
        assert!(!recovered[0].exhausted);
        assert_eq!(recovered[0].exhausted_until_ms, None);
    }

    #[test]
    fn tracked_providers_emit_multiple_active_accounts_and_apply_nicknames() {
        let rows = vec![
            json!({
                "id": "a@example.com",
                "label": "a@example.com",
                "active": true,
                "stale": false,
                "error": "auth-rejected",
                "usage": {
                    "email": "a@example.com",
                    "percentUsed": 45.0,
                    "resetAt": iso_in(1440),
                    "source": "grok-bot"
                }
            }),
            json!({
                "id": "b@example.com",
                "label": "b@example.com",
                "active": true,
                "stale": false,
                "usage": {
                    "email": "b@example.com",
                    "percentUsed": 20.0,
                    "resetAt": iso_in(1440),
                    "source": "grok-bot"
                }
            }),
        ];
        let mut settings = Settings::default();
        settings
            .display_names
            .insert("grok:a@example.com".to_string(), json!("Work Grok"));

        let (items, snoozed) = build_grok_items(&rows, &settings, NOW);
        assert_eq!(snoozed, 0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Work Grok");
        assert!(items[0].auth_expired); // via error == "auth-rejected"
        assert_eq!(items[1].title, "b@example.com"); // multiple -> falls back to label

        settings.show_provider_names = true;
        let (items_provider_names, _) = build_grok_items(&rows, &settings, NOW);
        assert_eq!(items_provider_names[0].title, "Grok");
        assert_eq!(items_provider_names[1].title, "Grok");
        assert_eq!(items_provider_names[0].display_email, None);
        assert_eq!(items_provider_names[1].display_email, None);
    }

    #[test]
    fn cursor_api_tooltips_show_top_three_models_plus_other() {
        let resp = json!({
            "accounts": [{
                "id": "dev@cursor.sh",
                "label": "dev@cursor.sh",
                "active": true,
                "stale": false,
                "usage": {
                    "cursorModelsPercentUsed": 40.0,
                    "otherModelsPercentUsed": 65.0,
                    "resetsAt": iso_in(5 * 24 * 60),
                    "accountName": "dev@cursor.sh",
                    "planName": "Pro",
                    "onDemand": {
                        "state": "enabled",
                        "spentUsd": 4.2,
                        "limitUsd": 50.0
                    },
                    "apiSpend": {
                        "spentUsd": 12.5,
                        "limitUsd": 20.0,
                        "byModel": [
                            { "model": "claude-4.6-opus", "usd": 6.0 },
                            { "model": "gpt-5", "usd": 3.5 },
                            { "model": "o3", "usd": 2.0 },
                            { "model": "gemini-2.5", "usd": 0.6 },
                            { "model": "sonnet", "usd": 0.4 }
                        ]
                    }
                }
            }]
        });

        let mut settings = Settings::default();
        settings
            .hidden_bars
            .insert(WidgetProvider::Cursor, vec!["firstParty".to_string()]);
        let (items, snoozed) = build_cursor_items(&resp, &settings, NOW);
        assert_eq!(snoozed, 0);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "dev@cursor.sh");
        assert_eq!(items[0].flyout_bars.len(), 1);
        assert_eq!(items[0].hover_bars.len(), 1);
        assert_eq!(
            items[0].hover_details,
            vec![
                ("API spend".to_string(), "$12.50 / $20.00".to_string()),
                ("On-demand".to_string(), "$4.20 / $50.00".to_string()),
                ("Plan".to_string(), "Pro".to_string()),
                ("Account".to_string(), "dev@cursor.sh".to_string()),
            ]
        );
        assert_eq!(
            items[0].flyout_bars[0].tooltip_lines,
            vec!["claude-4.6-opus $6", "gpt-5 $3.50", "o3 $2", "other $1",]
        );
        assert!(items[0].tooltip.contains("API: claude-4.6-opus $6"));
    }

    #[test]
    fn settings_rows_and_cursor_diagnostics_format_cleanly() {
        let resets = ClaudeResets {
            groups: vec![
                ClaudeResetGroup {
                    kind: ClaudeResetKind::Full,
                    label: "Full reset".to_string(),
                    count: 2,
                    earliest_expiry: None,
                    paused: false,
                },
                ClaudeResetGroup {
                    kind: ClaudeResetKind::Weekly,
                    label: "Fable weekly".to_string(),
                    count: 1,
                    earliest_expiry: None,
                    paused: false,
                },
            ],
            weekly_five_hour_available: Some(true),
        };
        assert_eq!(
            claude_resets_cell_text(Some(&resets)).as_deref(),
            Some("2 Full · 1 Fable · 1 5-hour")
        );

        let oc_rows = vec![
            json!({
                "email": "lapsed@example.com",
                "enabled": true,
                "subscriptionActive": false,
                "stale": false,
                "usage": {
                    "rolling": { "status": "ok", "percent": 10.0, "resetsAt": iso_in(60) },
                    "weekly":  { "status": "ok", "percent": 20.0, "resetsAt": iso_in(1440) },
                    "monthly": { "status": "ok", "percent": 30.0, "resetsAt": iso_in(10000) }
                }
            }),
            json!({
                "email": "badkey@example.com",
                "enabled": true,
                "error": "auth-rejected",
                "stale": false,
                "usage": null
            }),
        ];
        let oc = build_settings_opencode_rows(&oc_rows, &Settings::default(), NOW);
        assert_eq!(oc.len(), 2);
        assert!(oc[0].lapsed);
        assert!(!oc[0].active);
        assert_eq!(oc[0].warning.as_deref(), Some("not active"));
        assert_eq!(oc[0].bars[0].severity, BarSeverity::Muted);
        assert!(oc[1].auth_expired);
        assert_eq!(oc[1].warning.as_deref(), Some("key rejected"));
        assert_eq!(oc[1].note.as_deref(), Some("Unavailable"));

        let mut attempt = CursorAttempt {
            at_ms: NOW,
            outcome: "auth-rejected".to_string(),
            failures: 2,
            ..Default::default()
        };
        attempt.db.how = DbHow::Opened;
        attempt.token.present = true;
        attempt.token.form = Some("jwt".to_string());
        attempt.token.id = Some("1a2b3c4d".to_string());
        let diag = cursor_diag_rows_tz(&attempt, NOW, &Utc);
        assert_eq!(diag[0].0, "Last attempt");
        assert!(diag[0].2);
        assert!(diag
            .iter()
            .any(|(k, v, _)| k == "Sign-in" && v == "Token plain token"));
        assert!(diag
            .iter()
            .any(|(k, v, _)| k == "Token" && v.starts_with("#1a2b3c4d")));
    }

    #[test]
    fn demo_snapshot_has_four_providers_with_at_most_two_taskbar_bars() {
        let snap = demo_snapshot(NOW);
        assert_eq!(snap.items.len(), 4);
        assert_eq!(snap.snoozed_count, 0);
        assert_eq!(snap.items[0].spend_rate_text.as_deref(), Some("$42/h"));
        assert_eq!(snap.items[0].resets_badge.as_deref(), Some("↺ 2"));
        assert_eq!(snap.items[0].hover_bars.len(), 3);
        assert!(!snap.items[0].hover_details.is_empty());
        assert_eq!(
            snap.items[2].exhausted_until_ms,
            Some((NOW + 220 * 60_000) as u64)
        );
        assert_eq!(snap.theme_mode, ThemeMode::Auto);
        for item in &snap.items {
            assert!(item.taskbar_bars.len() <= 2);
            assert!(!item.flyout_bars.is_empty());
            assert!(!item.hover_bars.is_empty());
            assert!(!item.hover_details.is_empty());
        }
    }

    #[test]
    fn opencode_auto_detects_subscription_end_from_monthly_reset() {
        // NOW is 2023-11-14 22:13:20 UTC.
        let rows = vec![json!({
            "email": "active@example.com",
            "enabled": true,
            "subscriptionActive": true,
            "stale": false,
            "usage": {
                "rolling": { "status": "ok", "percent": 5.0, "resetsAt": "2023-11-15T03:00:00Z" },
                "weekly":  { "status": "ok", "percent": 15.0, "resetsAt": "2023-11-19T00:00:00Z" },
                "monthly": { "status": "ok", "percent": 40.0, "resetsAt": "2023-11-29T00:00:00Z" }
            }
        })];
        let built = build_settings_opencode_rows(&rows, &Settings::default(), NOW);
        assert_eq!(built.len(), 1);
        assert!(!built[0].unsubscribed);
        assert_eq!(built[0].plan.as_deref(), Some("Go"));
        assert_eq!(built[0].subscription_ends_at.as_deref(), Some("2023-11-29"));
        assert_eq!(
            built[0].subscription_ends_label.as_deref(),
            Some("Ends Nov 29 (15d)")
        );
        assert!(!built[0].subscription_ends_soon);

        // Ending within 3 days highlights as soon.
        let soon_rows = vec![json!({
            "email": "soon@example.com",
            "enabled": true,
            "subscriptionActive": true,
            "stale": false,
            "usage": {
                "rolling": { "status": "ok", "percent": 5.0, "resetsAt": "2023-11-15T03:00:00Z" },
                "weekly":  { "status": "ok", "percent": 15.0, "resetsAt": "2023-11-16T00:00:00Z" },
                "monthly": { "status": "ok", "percent": 40.0, "resetsAt": "2023-11-16T12:00:00Z" }
            }
        })];
        let soon = build_settings_opencode_rows(&soon_rows, &Settings::default(), NOW);
        assert_eq!(
            soon[0].subscription_ends_label.as_deref(),
            Some("Ends Nov 16 (2d)")
        );
        assert!(soon[0].subscription_ends_soon);
    }

    #[test]
    fn unsubscribed_and_expired_manual_end_override_plan_sort_bottom_and_hide_from_taskbar() {
        // NOW is 2023-11-14 UTC.
        let claude_rows = vec![
            json!({
                "id": "old@example.com",
                "label": "old@example.com",
                "active": true,
                "stale": false,
                "usage": {
                    "signedIn": true,
                    "subscribed": true,
                    "planName": "Max 20x",
                    "fiveHour": { "percent": 12.0, "resetsAt": iso_in(120) },
                    "weekly": { "percent": 40.0, "resetsAt": iso_in(1440) }
                }
            }),
            json!({
                "id": "expired@example.com",
                "label": "expired@example.com",
                "active": true,
                "stale": false,
                "usage": {
                    "signedIn": true,
                    "subscribed": true,
                    "planName": "Max 5x",
                    "fiveHour": { "percent": 20.0, "resetsAt": iso_in(120) },
                    "weekly": { "percent": 50.0, "resetsAt": iso_in(1440) }
                }
            }),
            json!({
                "id": "active@example.com",
                "label": "active@example.com",
                "active": true,
                "stale": false,
                "usage": {
                    "signedIn": true,
                    "subscribed": true,
                    "planName": "Max 20x",
                    "fiveHour": { "percent": 5.0, "resetsAt": iso_in(120) },
                    "weekly": { "percent": 10.0, "resetsAt": iso_in(1440) }
                }
            }),
        ];

        let mut settings = Settings::default();
        settings
            .unsubscribed_accounts
            .push("claude:old@example.com".to_string());
        settings.subscription_ends_at.insert(
            "claude:expired@example.com".to_string(),
            "2023-11-10".to_string(),
        );
        settings.subscription_ends_at.insert(
            "claude:active@example.com".to_string(),
            "2023-11-14".to_string(),
        );

        // Settings rows: active sorts first, unsubscribed & passed-end-date sort below with "Unsubscribed" badge.
        let rows = build_settings_claude_rows(&claude_rows, &settings, NOW);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "active@example.com");
        assert!(!rows[0].unsubscribed);
        assert!(rows[0].active);
        assert_eq!(rows[0].plan.as_deref(), Some("Ultra"));
        assert_eq!(
            rows[0].subscription_ends_label.as_deref(),
            Some("Ends today")
        );
        assert!(rows[0].subscription_ends_soon);

        assert_eq!(rows[1].id, "old@example.com");
        assert!(rows[1].unsubscribed);
        assert!(!rows[1].active);
        assert_eq!(rows[1].plan.as_deref(), Some("Unsubscribed"));
        assert_eq!(rows[1].bars[0].severity, BarSeverity::Muted);

        assert_eq!(rows[2].id, "expired@example.com");
        assert!(rows[2].unsubscribed);
        assert!(!rows[2].active);
        assert_eq!(rows[2].plan.as_deref(), Some("Unsubscribed"));
        assert_eq!(
            rows[2].subscription_ends_label.as_deref(),
            Some("Ended Nov 10")
        );
        assert!(rows[2].subscription_ends_soon);

        // Taskbar items: only active@example.com appears; old & expired are excluded automatically.
        let spend = ClaudeSpendInfo {
            rate_text: None,
            rate_severity: BarSeverity::Muted,
            summary: None,
        };
        let (items, _) =
            build_claude_items(&claude_rows, &settings, &spend, NOW, |_| (None, Vec::new()));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].account_id, "active@example.com");
    }

    #[test]
    fn antigravity_hover_card_splits_gemini_and_claude_gpt_pools() {
        let rows = vec![json!({
            "id": "dev@example.com",
            "label": "dev@example.com",
            "active": true,
            "stale": false,
            "usage": {
                "email": "dev@example.com",
                "fiveHourPercent": 100.0,
                "fiveHourResetsAt": iso_in(180),
                "weeklyPercent": 45.0,
                "weeklyResetsAt": iso_in(4320),
                "groups": [
                    {
                        "name": "Gemini",
                        "fiveHourPercent": 12.0,
                        "fiveHourResetsAt": iso_in(210),
                        "weeklyPercent": 8.0,
                        "weeklyResetsAt": iso_in(4320)
                    },
                    {
                        "name": "Claude & GPT",
                        "fiveHourPercent": 100.0,
                        "fiveHourResetsAt": iso_in(180),
                        "weeklyPercent": 45.0,
                        "weeklyResetsAt": iso_in(4320)
                    }
                ]
            }
        })];
        let (items, _) = build_antigravity_items(&rows, &Settings::default(), NOW);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].taskbar_bars.len(), 2);
        let hover_labels: Vec<&str> = items[0]
            .hover_bars
            .iter()
            .map(|b| b.label.as_str())
            .collect();
        assert_eq!(
            hover_labels,
            vec![
                "Gemini · 5h",
                "Gemini · Weekly",
                "Claude & GPT · 5h",
                "Claude & GPT · Weekly",
            ]
        );
    }

    #[test]
    fn cached_settings_snapshot_loads_accounts_synchronously_from_disk() {
        let dir =
            std::env::temp_dir().join(format!("usagedeck-cached-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::store::write_json_atomic(
            &dir.join(crate::store::SEEN_ACCOUNTS_FILE),
            &json!({
                "codex": [{ "id": "dev@example.com", "label": "dev@example.com" }],
                "antigravity": [{ "id": "ag@example.com", "label": "ag@example.com" }]
            }),
        )
        .unwrap();
        crate::store::write_json_atomic(
            &dir.join(crate::store::USAGE_CACHE_FILE),
            &json!({
                "codex": {
                    "dev@example.com": {
                        "savedAt": NOW,
                        "data": {
                            "email": "dev@example.com",
                            "planType": "plus",
                            "usedPercent": 25.0,
                            "resetAt": iso_in(120)
                        }
                    }
                },
                "antigravity": {
                    "ag@example.com": {
                        "savedAt": NOW,
                        "data": {
                            "email": "ag@example.com",
                            "fiveHourPercent": 35.0,
                            "weeklyPercent": 15.0
                        }
                    }
                }
            }),
        )
        .unwrap();
        crate::store::write_json_atomic(
            &dir.join(crate::store::ACCOUNTS_FILE),
            &json!([{ "email": "oc@example.com" }]),
        )
        .unwrap();

        let snap = cached_settings_snapshot(&dir, Settings::default());
        assert_eq!(snap.codex.len(), 1);
        assert_eq!(snap.codex[0].id, "dev@example.com");
        assert_eq!(snap.codex[0].plan.as_deref(), Some("Plus"));
        assert_eq!(snap.antigravity.len(), 1);
        assert_eq!(snap.antigravity[0].id, "ag@example.com");
        assert_eq!(snap.opencode.len(), 1);
        assert_eq!(snap.opencode[0].id, "oc@example.com");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
