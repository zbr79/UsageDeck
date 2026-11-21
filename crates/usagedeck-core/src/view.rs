//! Pure-Rust view model for the Windows taskbar strip and anchored flyout.
//!
//! Converts raw provider responses + Claude Code spend readings into compact,
//! ready-to-render `TaskbarSnapshot` data (Variant A: icon + up to 2 bars + %
//! on the taskbar, with Claude's live `$/h` rate and full bar details in the flyout).

use crate::claude_logs;
use crate::providers::claude::ClaudeUsage;
use crate::providers::codex::CodexUsage;
use crate::providers::cursor_usage::{CursorUsage, OnDemand};
use crate::providers::grok::GrokUsage;
use crate::providers::opencode::OpenCodeUsage;
use crate::store::{Settings, WidgetProvider};
use crate::Core;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    Claude,
    Cursor,
    Codex,
    Grok,
    OpenCode,
}

impl ProviderKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Cursor => "Cursor",
            Self::Codex => "Codex",
            Self::Grok => "Grok Bot",
            Self::OpenCode => "OpenCode",
        }
    }

    pub fn key_prefix(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
        }
    }

    pub fn from_widget_provider(wp: WidgetProvider) -> Self {
        match wp {
            WidgetProvider::Claude => Self::Claude,
            WidgetProvider::Cursor => Self::Cursor,
            WidgetProvider::Codex => Self::Codex,
            WidgetProvider::Grok => Self::Grok,
            WidgetProvider::Opencode => Self::OpenCode,
        }
    }

    pub fn to_widget_provider(self) -> WidgetProvider {
        match self {
            Self::Claude => WidgetProvider::Claude,
            Self::Cursor => WidgetProvider::Cursor,
            Self::Codex => WidgetProvider::Codex,
            Self::Grok => WidgetProvider::Grok,
            Self::OpenCode => WidgetProvider::Opencode,
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
    pub severity: BarSeverity,
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
        let severity = if dimmed {
            BarSeverity::Muted
        } else {
            severity_of(effective_pct)
        };
        Self {
            key: key.into(),
            label: label.into(),
            percent: effective_pct,
            resets_at,
            reset_short,
            severity,
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
    /// Inline live spend rate (currently Claude Code `$/h`, e.g. `"$42/h"`).
    pub spend_rate_text: Option<String>,
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
}

pub fn severity_of(percent: Option<f64>) -> BarSeverity {
    match percent {
        None => BarSeverity::Muted,
        Some(p) if p >= 100.0 => BarSeverity::Danger,
        Some(p) if p >= 80.0 => BarSeverity::Warn,
        Some(_) => BarSeverity::Ok,
    }
}

pub fn parse_iso_ms(iso: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
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
    bars.into_iter()
        .filter(|b| !set.contains(b.key.as_str()))
        .collect()
}

fn is_disabled(settings: &Settings, provider: ProviderKind, account_id: &str) -> bool {
    let key = format!("{}:{}", provider.key_prefix(), account_id).to_lowercase();
    settings
        .disabled_accounts
        .iter()
        .any(|d| d.eq_ignore_ascii_case(&key))
}

fn is_snoozed(settings: &Settings, provider: ProviderKind, account_id: &str, now_ms: i64) -> bool {
    let key = format!("{}:{}", provider.key_prefix(), account_id).to_lowercase();
    settings
        .snoozed_until
        .get(&key)
        .is_some_and(|&until| (until as i64) > now_ms)
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
    }
}

/// Poll all enabled providers concurrently and assemble a `TaskbarSnapshot`.
pub async fn poll_snapshot(core: &Core) -> TaskbarSnapshot {
    let settings = core.store.get_settings();
    let now_ms = claude_logs::now_ms();

    let (claude_rows, cursor_resp, codex_rows, grok_rows, opencode_rows) = tokio::join!(
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
    );

    // Claude Code hourly spend rate (reads local transcripts already scanned in background).
    let claude_rate = if settings.claude_enabled {
        let spend = core.spend.spend(now_ms);
        let rate = if spend.rate15m >= 0.5 {
            spend.rate15m
        } else {
            spend.rate60m
        };
        format_rate_usd(rate)
    } else {
        None
    };

    let mut items = Vec::new();
    for &wp in &settings.provider_order {
        if !is_provider_enabled(&settings, wp) {
            continue;
        }
        match wp {
            WidgetProvider::Claude => {
                if let Some(item) =
                    build_claude_item(&claude_rows, &settings, claude_rate.clone(), now_ms)
                {
                    items.push(item);
                }
            }
            WidgetProvider::Cursor => {
                if let Some(ref resp) = cursor_resp {
                    if let Some(item) = build_cursor_item(resp, &settings, now_ms) {
                        items.push(item);
                    }
                }
            }
            WidgetProvider::Codex => {
                if let Some(item) = build_codex_item(&codex_rows, &settings, now_ms) {
                    items.push(item);
                }
            }
            WidgetProvider::Grok => {
                if let Some(item) = build_grok_item(&grok_rows, &settings, now_ms) {
                    items.push(item);
                }
            }
            WidgetProvider::Opencode => {
                if let Some(item) = build_opencode_item(&opencode_rows, &settings, now_ms) {
                    items.push(item);
                }
            }
        }
    }

    TaskbarSnapshot {
        items,
        updated_at_ms: now_ms,
        hide_in_fullscreen: settings.hide_in_fullscreen,
        is_demo: false,
    }
}

fn find_active_row(rows: &[Value]) -> Option<&Value> {
    rows.iter()
        .find(|r| r.get("active").and_then(Value::as_bool).unwrap_or(false))
}

fn build_claude_item(
    rows: &[Value],
    settings: &Settings,
    spend_rate_text: Option<String>,
    now_ms: i64,
) -> Option<ProviderItem> {
    let row = find_active_row(rows)?;
    let id = row.get("id").and_then(Value::as_str).unwrap_or("claude");
    if is_disabled(settings, ProviderKind::Claude, id)
        || is_snoozed(settings, ProviderKind::Claude, id, now_ms)
    {
        return None;
    }
    let auth_expired = row
        .get("authExpired")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
    let usage: Option<ClaudeUsage> = row
        .get("usage")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let u = usage?;
    if !u.signed_in {
        return None;
    }

    let mut raw_bars = vec![
        BarView::new(
            "fiveHour",
            "5h",
            u.five_hour.percent,
            u.five_hour.resets_at.clone(),
            now_ms,
            auth_expired,
        ),
        BarView::new(
            "weekly",
            "Weekly",
            u.weekly.percent,
            u.weekly.resets_at.clone(),
            now_ms,
            auth_expired,
        ),
    ];
    for scoped in &u.weekly_scoped {
        raw_bars.push(BarView::new(
            "scoped",
            scoped.label.clone(),
            scoped.percent,
            scoped.resets_at.clone(),
            now_ms,
            auth_expired,
        ));
    }

    let hidden = hidden_bars_for(settings, ProviderKind::Claude);
    let flyout_bars = {
        let f = filter_hidden_bars(raw_bars.clone(), &hidden);
        if f.is_empty() {
            raw_bars
        } else {
            f
        }
    };
    let taskbar_bars = pick_taskbar_bars(&flyout_bars);
    let reset_short = flyout_bars
        .first()
        .and_then(|b| b.reset_short.clone())
        .or_else(|| flyout_bars.get(1).and_then(|b| b.reset_short.clone()));
    let exhausted = flyout_bars
        .iter()
        .take(2)
        .any(|b| b.severity == BarSeverity::Danger);
    let subtitle = if auth_expired {
        Some("re-auth".to_string())
    } else {
        None
    };
    let tooltip = build_tooltip(
        "Claude",
        &flyout_bars,
        spend_rate_text.as_deref(),
        subtitle.as_deref(),
    );

    Some(ProviderItem {
        provider: ProviderKind::Claude,
        account_id: id.to_string(),
        title: "Claude".to_string(),
        taskbar_bars,
        flyout_bars,
        spend_rate_text,
        reset_short,
        subtitle,
        auth_expired,
        stale,
        exhausted,
        can_open_cursor: false,
        tooltip,
    })
}

fn build_cursor_item(resp: &Value, settings: &Settings, now_ms: i64) -> Option<ProviderItem> {
    let rows = resp.get("accounts")?.as_array()?;
    let row = find_active_row(rows)?;
    let id = row.get("id").and_then(Value::as_str).unwrap_or("cursor");
    if is_disabled(settings, ProviderKind::Cursor, id)
        || is_snoozed(settings, ProviderKind::Cursor, id, now_ms)
    {
        return None;
    }
    let auth_expired = row
        .get("authExpired")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
    let usage: Option<CursorUsage> = row
        .get("usage")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let u = usage?;

    let raw_bars = vec![
        BarView::new(
            "firstParty",
            "1P",
            u.cursor_models_percent_used,
            u.resets_at.clone(),
            now_ms,
            auth_expired,
        ),
        BarView::new(
            "api",
            "API",
            u.other_models_percent_used,
            u.resets_at.clone(),
            now_ms,
            auth_expired,
        ),
    ];
    let hidden = hidden_bars_for(settings, ProviderKind::Cursor);
    let flyout_bars = {
        let f = filter_hidden_bars(raw_bars.clone(), &hidden);
        if f.is_empty() {
            raw_bars
        } else {
            f
        }
    };
    let taskbar_bars = pick_taskbar_bars(&flyout_bars);
    let reset_short = u
        .resets_at
        .as_deref()
        .and_then(|iso| format_reset_short(iso, now_ms));

    let api_text = u.api_spend.as_ref().map(|s| match s.limit_usd {
        Some(lim) if lim > 0.0 => format!("API ${:.1}/${:.0}", s.spent_usd, lim),
        _ => format!("API ${:.2}", s.spent_usd),
    });
    let subtitle = if auth_expired {
        Some("Open Cursor".to_string())
    } else {
        api_text
    };

    let both_full = flyout_bars
        .iter()
        .all(|b| b.severity == BarSeverity::Danger)
        && !flyout_bars.is_empty();
    let on_demand_left = matches!(
        u.on_demand,
        Some(OnDemand::Enabled {
            spent_usd,
            limit_usd,
        }) if limit_usd.is_none_or(|lim| lim > spent_usd)
    );
    let exhausted = both_full && !on_demand_left;
    let tooltip = build_tooltip("Cursor", &flyout_bars, None, subtitle.as_deref());

    Some(ProviderItem {
        provider: ProviderKind::Cursor,
        account_id: id.to_string(),
        title: "Cursor".to_string(),
        taskbar_bars,
        flyout_bars,
        spend_rate_text: None,
        reset_short,
        subtitle,
        auth_expired,
        stale,
        exhausted,
        can_open_cursor: auth_expired,
        tooltip,
    })
}

fn build_codex_item(rows: &[Value], settings: &Settings, now_ms: i64) -> Option<ProviderItem> {
    let row = find_active_row(rows)?;
    let id = row.get("id").and_then(Value::as_str).unwrap_or("codex");
    if is_disabled(settings, ProviderKind::Codex, id)
        || is_snoozed(settings, ProviderKind::Codex, id, now_ms)
    {
        return None;
    }
    let auth_expired = row
        .get("authExpired")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
    let usage: Option<CodexUsage> = row
        .get("usage")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let u = usage?;

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
    let hidden = hidden_bars_for(settings, ProviderKind::Codex);
    let flyout_bars = {
        let f = filter_hidden_bars(raw_bars.clone(), &hidden);
        if f.is_empty() {
            raw_bars
        } else {
            f
        }
    };
    let taskbar_bars = pick_taskbar_bars(&flyout_bars);
    let reset_short = flyout_bars.first().and_then(|b| b.reset_short.clone());
    let exhausted = u.limit_reached
        || flyout_bars
            .iter()
            .any(|b| b.severity == BarSeverity::Danger);
    let subtitle = if auth_expired {
        Some("re-auth".to_string())
    } else {
        None
    };
    let tooltip = build_tooltip("Codex", &flyout_bars, None, subtitle.as_deref());

    Some(ProviderItem {
        provider: ProviderKind::Codex,
        account_id: id.to_string(),
        title: "Codex".to_string(),
        taskbar_bars,
        flyout_bars,
        spend_rate_text: None,
        reset_short,
        subtitle,
        auth_expired,
        stale,
        exhausted,
        can_open_cursor: false,
        tooltip,
    })
}

fn build_grok_item(rows: &[Value], settings: &Settings, now_ms: i64) -> Option<ProviderItem> {
    let row = find_active_row(rows)?;
    let id = row.get("id").and_then(Value::as_str).unwrap_or("grok");
    if is_disabled(settings, ProviderKind::Grok, id)
        || is_snoozed(settings, ProviderKind::Grok, id, now_ms)
    {
        return None;
    }
    let auth_expired = row
        .get("authExpired")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
    let usage: Option<GrokUsage> = row
        .get("usage")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let u = usage?;

    let flyout_bars = vec![BarView::new(
        "weekly",
        "Weekly",
        u.percent_used,
        u.reset_at.clone(),
        now_ms,
        auth_expired,
    )];
    let taskbar_bars = flyout_bars.clone();
    let reset_short = flyout_bars.first().and_then(|b| b.reset_short.clone());
    let exhausted = flyout_bars
        .first()
        .is_some_and(|b| b.severity == BarSeverity::Danger);
    let subtitle = if auth_expired {
        Some("re-auth".to_string())
    } else {
        None
    };
    let tooltip = build_tooltip("Grok Bot", &flyout_bars, None, subtitle.as_deref());

    Some(ProviderItem {
        provider: ProviderKind::Grok,
        account_id: id.to_string(),
        title: "Grok Bot".to_string(),
        taskbar_bars,
        flyout_bars,
        spend_rate_text: None,
        reset_short,
        subtitle,
        auth_expired,
        stale,
        exhausted,
        can_open_cursor: false,
        tooltip,
    })
}

fn build_opencode_item(rows: &[Value], settings: &Settings, now_ms: i64) -> Option<ProviderItem> {
    // Pick the top-ranked enabled, non-snoozed OpenCode account:
    // non-exhausted accounts rank ahead of exhausted ones, preferring most recently changed.
    let mut best: Option<(bool, i64, ProviderItem)> = None;
    let hidden = hidden_bars_for(settings, ProviderKind::OpenCode);

    for row in rows {
        let enabled = row.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        if !enabled {
            continue;
        }
        let email = row.get("email").and_then(Value::as_str)?;
        if is_disabled(settings, ProviderKind::OpenCode, email)
            || is_snoozed(settings, ProviderKind::OpenCode, email, now_ms)
        {
            continue;
        }
        let auth_expired = row
            .get("authExpired")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let stale = row.get("stale").and_then(Value::as_bool).unwrap_or(false);
        let usage: Option<OpenCodeUsage> = row
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let Some(u) = usage else { continue };

        let raw_bars = vec![
            BarView::new(
                "rolling",
                "Rolling",
                Some(u.rolling.percent),
                Some(u.rolling.resets_at.clone()),
                now_ms,
                auth_expired,
            ),
            BarView::new(
                "weekly",
                "Weekly",
                Some(u.weekly.percent),
                Some(u.weekly.resets_at.clone()),
                now_ms,
                auth_expired,
            ),
            BarView::new(
                "monthly",
                "Monthly",
                Some(u.monthly.percent),
                Some(u.monthly.resets_at.clone()),
                now_ms,
                auth_expired,
            ),
        ];
        let flyout_bars = {
            let f = filter_hidden_bars(raw_bars.clone(), &hidden);
            if f.is_empty() {
                raw_bars
            } else {
                f
            }
        };
        let taskbar_bars = pick_taskbar_bars(&flyout_bars);
        let exhausted = flyout_bars
            .iter()
            .filter(|b| b.key == "weekly" || b.key == "monthly")
            .any(|b| b.severity == BarSeverity::Danger);
        let last_change = row.get("lastChangeAt").and_then(Value::as_i64).unwrap_or(0);
        let reset_short = flyout_bars.first().and_then(|b| b.reset_short.clone());
        let subtitle = if auth_expired {
            Some("key rejected".to_string())
        } else {
            Some(email.to_string())
        };
        let tooltip = build_tooltip("OpenCode", &flyout_bars, None, subtitle.as_deref());

        let item = ProviderItem {
            provider: ProviderKind::OpenCode,
            account_id: email.to_string(),
            title: "OpenCode".to_string(),
            taskbar_bars,
            flyout_bars,
            spend_rate_text: None,
            reset_short,
            subtitle,
            auth_expired,
            stale,
            exhausted,
            can_open_cursor: false,
            tooltip,
        };

        let replace = match &best {
            None => true,
            Some((best_ex, best_chg, _)) => {
                (!exhausted && *best_ex) || (exhausted == *best_ex && last_change > *best_chg)
            }
        };
        if replace {
            best = Some((exhausted, last_change, item));
        }
    }

    best.map(|(_, _, item)| item)
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
    parts.join(" — ")
}

/// Realistic preview snapshot for `--demo` mode so the taskbar strip and flyout
/// can be inspected even on a machine without live provider sign-ins.
pub fn demo_snapshot(now_ms: i64) -> TaskbarSnapshot {
    let iso_in = |mins: i64| -> Option<String> {
        DateTime::<Utc>::from_timestamp_millis(now_ms + mins * 60_000).map(|dt| dt.to_rfc3339())
    };

    let claude_bars = vec![
        BarView::new("fiveHour", "5h", Some(64.0), iso_in(134), now_ms, false),
        BarView::new(
            "weekly",
            "Weekly",
            Some(83.0),
            iso_in(4 * 24 * 60),
            now_ms,
            false,
        ),
        BarView::new(
            "fable",
            "Fable",
            Some(32.0),
            iso_in(4 * 24 * 60),
            now_ms,
            false,
        ),
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
        BarView::new("api", "API", Some(54.0), iso_in(9 * 24 * 60), now_ms, false),
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

    TaskbarSnapshot {
        items: vec![
            ProviderItem {
                provider: ProviderKind::Claude,
                account_id: "demo@claude.ai".into(),
                title: "Claude".into(),
                taskbar_bars: pick_taskbar_bars(&claude_bars),
                flyout_bars: claude_bars,
                spend_rate_text: Some("$42/h".into()),
                reset_short: Some("2h14m".into()),
                subtitle: None,
                auth_expired: false,
                stale: false,
                exhausted: false,
                can_open_cursor: false,
                tooltip: "Claude — $42/h — 5h 64% (2h14m) · Weekly 83% (4d)".into(),
            },
            ProviderItem {
                provider: ProviderKind::Cursor,
                account_id: "demo@cursor.sh".into(),
                title: "Cursor".into(),
                taskbar_bars: pick_taskbar_bars(&cursor_bars),
                flyout_bars: cursor_bars,
                spend_rate_text: None,
                reset_short: Some("9d".into()),
                subtitle: Some("API $10.8/$20".into()),
                auth_expired: false,
                stale: false,
                exhausted: false,
                can_open_cursor: false,
                tooltip: "Cursor — 1P 38% (9d) · API 54% (9d) — API $10.8/$20".into(),
            },
            ProviderItem {
                provider: ProviderKind::Codex,
                account_id: "demo@openai.com".into(),
                title: "Codex".into(),
                taskbar_bars: pick_taskbar_bars(&codex_bars),
                flyout_bars: codex_bars,
                spend_rate_text: None,
                reset_short: Some("3h40m".into()),
                subtitle: None,
                auth_expired: false,
                stale: false,
                exhausted: true,
                can_open_cursor: false,
                tooltip: "Codex — 5h 100 (3h40m) · Weekly 41% (5d)".into(),
            },
            ProviderItem {
                provider: ProviderKind::OpenCode,
                account_id: "demo@opencode.ai".into(),
                title: "OpenCode".into(),
                taskbar_bars: pick_taskbar_bars(&opencode_bars),
                flyout_bars: opencode_bars,
                spend_rate_text: None,
                reset_short: Some("3h05m".into()),
                subtitle: Some("demo@opencode.ai".into()),
                auth_expired: false,
                stale: false,
                exhausted: false,
                can_open_cursor: false,
                tooltip: "OpenCode — Rolling 28% (3h05m) · Weekly 52% (3d) · Monthly 19% (14d)"
                    .into(),
            },
        ],
        updated_at_ms: now_ms,
        hide_in_fullscreen: true,
        is_demo: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_reset_durations_compactly() {
        let now = 1_700_000_000_000i64;
        let iso = |mins: i64| {
            DateTime::<Utc>::from_timestamp_millis(now + mins * 60_000)
                .unwrap()
                .to_rfc3339()
        };
        assert_eq!(format_reset_short(&iso(-5), now), None);
        assert_eq!(format_reset_short(&iso(45), now).as_deref(), Some("45m"));
        assert_eq!(format_reset_short(&iso(120), now).as_deref(), Some("2h"));
        assert_eq!(format_reset_short(&iso(134), now).as_deref(), Some("2h14m"));
        assert_eq!(
            format_reset_short(&iso(2 * 24 * 60 + 5 * 60), now).as_deref(),
            Some("2d5h")
        );
        assert_eq!(
            format_reset_short(&iso(9 * 24 * 60), now).as_deref(),
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
    fn picks_two_most_urgent_bars_for_taskbar() {
        let now = 1_700_000_000_000i64;
        let bars = vec![
            BarView::new("rolling", "Rolling", Some(20.0), None, now, false),
            BarView::new("weekly", "Weekly", Some(45.0), None, now, false),
            BarView::new("monthly", "Monthly", Some(91.0), None, now, false),
        ];
        let picked = pick_taskbar_bars(&bars);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].key, "rolling");
        assert_eq!(picked[1].key, "monthly");
    }

    #[test]
    fn demo_snapshot_has_four_providers_with_at_most_two_taskbar_bars() {
        let snap = demo_snapshot(1_700_000_000_000);
        assert_eq!(snap.items.len(), 4);
        assert_eq!(snap.items[0].spend_rate_text.as_deref(), Some("$42/h"));
        for item in &snap.items {
            assert!(item.taskbar_bars.len() <= 2);
            assert!(!item.flyout_bars.is_empty());
        }
    }
}
