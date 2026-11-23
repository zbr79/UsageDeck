//! Pure-Rust layout & hit-testing model for the native Direct2D Settings window.
//!
//! Computes exact DIP geometry for the fixed 232 DIP left navigation sidebar,
//! the scrollable right-hand content pane across all 7 pages (General, Widget,
//! Claude, Codex, Cursor, Grok Bot, OpenCode), custom scrollbar, and inline toast.

use crate::settings_dialog::theme::{
    calibration_text, format_share_pct, format_snooze_duration, format_usd, format_usd_rate,
    plan_badge_tone, spend_series, BadgeTone, SpendRange,
};
use usagedeck_core::store::{
    AppSkin, PanelTheme, Settings, TaskbarMeterStyle, ThemeMode, WidgetProvider,
};
use usagedeck_core::view::{
    BarSeverity, BarView, ProviderKind, SettingsAccountRow, SettingsSnapshot,
};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

pub const DEFAULT_WIN_W_DIP: f32 = 960.0;
pub const DEFAULT_WIN_H_DIP: f32 = 680.0;
pub const MIN_WIN_W_DIP: f32 = 640.0;
pub const MIN_WIN_H_DIP: f32 = 480.0;

pub const SIDEBAR_W_DIP: f32 = 232.0;
pub const MAX_CONTENT_W_DIP: f32 = 960.0;
pub const CONTENT_PAD_X_DIP: f32 = 32.0;
pub const CONTENT_PAD_TOP_DIP: f32 = 28.0;
pub const CONTENT_PAD_BOTTOM_DIP: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DipRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl DipRect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    pub fn to_d2d(self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.x,
            top: self.y,
            right: self.x + self.w,
            bottom: self.y + self.h,
        }
    }

    pub fn offset_y(self, dy: f32) -> Self {
        Self {
            y: self.y + dy,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageId {
    General,
    Widget,
    Style,
    Provider(ProviderKind),
}

impl PageId {
    pub fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Widget => "Widget",
            Self::Style => "Style",
            Self::Provider(p) => p.display_name(),
        }
    }

    pub fn window_title(self) -> String {
        format!("{} - UsageDeck Settings", self.title())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    SelectPage(PageId),
    CheckUpdates,
    OpenAccountFolder,
    OpenLogFolder,
    CopyDiagnostics,
    QuitApp,
    ToggleStartAtLogin,
    ToggleHideInFullscreen,
    ToggleAutoHideExhausted,
    SetPanelTheme(PanelTheme),
    SetAppSkin(AppSkin),
    SetWidgetTheme(ThemeMode),
    SetMeterStyle(TaskbarMeterStyle),
    MoveProviderUp(WidgetProvider),
    MoveProviderDown(WidgetProvider),
    ToggleProviderEnabled(WidgetProvider),
    ToggleSnoozedExpand(WidgetProvider),
    UnhideSnoozed(String),
    ToggleShowProviderNames,
    ToggleSystemMonitor,
    RefreshData,
    OpenAccountMenu {
        provider: ProviderKind,
        account_id: String,
        label: String,
        nickname: Option<String>,
        hidden: bool,
        unsubscribed: bool,
        subscription_ends_at: Option<String>,
    },
    ToggleBar {
        provider: WidgetProvider,
        bar_id: String,
    },
    ToggleFableShade,
    SetHourlyTarget,
    ClearHourlyTarget,
    SetSpendRange(SpendRange),
    AddOpenCodeAccount,
    ToggleDeepSeekPeakWarning,
}

#[derive(Clone, Debug)]
pub struct HitRegion {
    pub rect: DipRect,
    pub action: Action,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    GhostIcon,
    Pill,
}

#[derive(Clone, Debug)]
pub struct NavRow {
    pub rect: DipRect,
    pub label: &'static str,
    pub provider_icon: Option<ProviderKind>,
    pub selected: bool,
    pub show_dot: bool,
    pub action: Action,
}

#[derive(Clone, Debug)]
pub struct SegOption {
    pub rect: DipRect,
    pub label: &'static str,
    pub selected: bool,
    pub action: Action,
}

#[derive(Clone, Debug)]
pub struct BarChip {
    pub rect: DipRect,
    pub label: String,
    pub checked: bool,
    pub disabled: bool,
    pub action: Action,
}

#[derive(Clone, Debug)]
pub struct TableColSpec {
    pub id: String,
    pub header: String,
    pub x: f32,
    pub w: f32,
}

#[derive(Clone, Debug)]
pub struct TableCellRender {
    pub x: f32,
    pub w: f32,
    pub percent_text: String,
    pub percent_val: Option<f64>,
    pub reset_text: Option<String>,
    pub severity: BarSeverity,
    pub dimmed: bool,
}

#[derive(Clone, Debug)]
pub struct AccountRowRender {
    pub rect: DipRect,
    pub active: bool,
    pub hidden: bool,
    pub unsubscribed: bool,
    pub name: String,
    pub sub_label: Option<(String, bool)>,
    pub warning: Option<String>,
    pub plan: Option<(String, BadgeTone)>,
    pub cells: Vec<TableCellRender>,
    pub note: Option<(f32, f32, String)>,
    pub extra: Option<(f32, f32, String, bool)>,
    pub menu_btn: DipRect,
    pub menu_action: Action,
}

#[derive(Clone, Debug)]
pub struct SpendChartRender {
    pub card_rect: DipRect,
    pub header_title: String,
    pub plot_rect: DipRect,
    pub points: Vec<(f64, f64)>,
    pub peak_label: String,
    pub left_label: String,
    pub models: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub enum ContentItem {
    PageHeader {
        rect: DipRect,
        icon: Option<ProviderKind>,
        title: String,
        subtitle: Option<String>,
    },
    SectionHeading {
        rect: DipRect,
        text: String,
    },
    Card {
        rect: DipRect,
    },
    Divider {
        rect: DipRect,
    },
    AppIcon {
        rect: DipRect,
    },
    ProviderIcon {
        rect: DipRect,
        kind: ProviderKind,
    },
    Text {
        rect: DipRect,
        text: String,
        size: f32,
        semi_bold: bool,
        muted: bool,
        warn: bool,
        align_right: bool,
        align_center: bool,
        mono: bool,
        wrap: bool,
    },
    Button {
        rect: DipRect,
        label: String,
        style: ButtonStyle,
        disabled: bool,
        action: Action,
    },
    Toggle {
        rect: DipRect,
        checked: bool,
        action: Action,
    },
    SegmentedControl {
        track: DipRect,
        options: Vec<SegOption>,
    },
    BarChipRow {
        chips: Vec<BarChip>,
    },
    AccountTable {
        card_rect: DipRect,
        header_rect: DipRect,
        account_col_x: f32,
        account_col_w: f32,
        cols: Vec<TableColSpec>,
        extra_header: Option<(f32, f32, String)>,
        rows: Vec<AccountRowRender>,
    },
    SpendChart(SpendChartRender),
    MeterStyleCard {
        rect: DipRect,
        style: TaskbarMeterStyle,
        title: &'static str,
        subtitle: &'static str,
        selected: bool,
        action: Action,
    },
}

#[derive(Clone, Debug)]
pub struct ScrollbarGeometry {
    pub track: DipRect,
    pub thumb: DipRect,
    pub max_scroll: f32,
}

#[derive(Clone, Debug)]
pub struct SettingsLayout {
    pub client_h: f32,
    pub nav_rows: Vec<NavRow>,
    pub nav_accounts_label: DipRect,
    pub nav_update_btn: Option<(DipRect, String)>,
    pub content_viewport: DipRect,
    pub scroll_y: f32,
    pub content_height: f32,
    pub content_items: Vec<ContentItem>,
    pub scrollbar: Option<ScrollbarGeometry>,
    pub hits: Vec<HitRegion>,
    pub row_context_hits: Vec<HitRegion>,
    pub toast: Option<(DipRect, String)>,
}

pub struct LayoutInput<'a> {
    pub client_w: f32,
    pub client_h: f32,
    pub page: PageId,
    pub settings: &'a Settings,
    pub snapshot: Option<&'a SettingsSnapshot>,
    pub loading: bool,
    pub app_version: &'a str,
    pub update_status_text: &'a str,
    pub update_checking: bool,
    pub update_ready_version: Option<&'a str>,
    pub expanded_snoozed: Option<WidgetProvider>,
    pub spend_range: SpendRange,
    pub scroll_y: f32,
    pub now_ms: u64,
    pub toast: Option<&'a str>,
}

pub fn provider_enabled(settings: &Settings, wp: WidgetProvider) -> bool {
    match wp {
        WidgetProvider::Claude => settings.claude_enabled,
        WidgetProvider::Cursor => settings.cursor_enabled,
        WidgetProvider::Codex => settings.codex_enabled,
        WidgetProvider::Grok => settings.grok_enabled,
        WidgetProvider::Opencode => settings.opencode_enabled,
        WidgetProvider::Antigravity => settings.antigravity_enabled,
    }
}

pub fn provider_bars(
    wp: WidgetProvider,
    scoped_label: Option<&str>,
) -> Vec<(&'static str, String)> {
    match wp {
        WidgetProvider::Opencode => vec![
            ("rolling", "Rolling (R)".to_string()),
            ("weekly", "Weekly (W)".to_string()),
            ("monthly", "Monthly (M)".to_string()),
        ],
        WidgetProvider::Cursor => vec![
            ("firstParty", "First Party".to_string()),
            ("api", "API".to_string()),
        ],
        WidgetProvider::Grok => vec![("weekly", "Weekly".to_string())],
        WidgetProvider::Codex | WidgetProvider::Antigravity => vec![
            ("fiveHour", "5-hour (5h)".to_string()),
            ("weekly", "Weekly".to_string()),
        ],
        WidgetProvider::Claude => vec![
            ("fiveHour", "5-hour (5h)".to_string()),
            ("weekly", "Weekly".to_string()),
            (
                "scoped",
                scoped_label
                    .map(|s| format!("{s} weekly"))
                    .unwrap_or_else(|| "Per-model weekly".to_string()),
            ),
        ],
    }
}

struct SnoozedEntry {
    key: String,
    provider: WidgetProvider,
    name: String,
    until: u64,
}

fn snoozed_entries(settings: &Settings, now_ms: u64) -> Vec<SnoozedEntry> {
    let mut out = Vec::new();
    for (key, &until) in &settings.snoozed_until {
        if until <= now_ms {
            continue;
        }
        let Some((prefix, id)) = key.split_once(':') else {
            continue;
        };
        let provider = match prefix {
            "opencode" => WidgetProvider::Opencode,
            "cursor" => WidgetProvider::Cursor,
            "grok" => WidgetProvider::Grok,
            "codex" => WidgetProvider::Codex,
            "claude" => WidgetProvider::Claude,
            "antigravity" => WidgetProvider::Antigravity,
            _ => continue,
        };
        let name = settings
            .display_names
            .get(key)
            .or_else(|| settings.display_names.get(id))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(id)
            .to_string();
        out.push(SnoozedEntry {
            key: key.clone(),
            provider,
            name,
            until,
        });
    }
    out.sort_by_key(|e| e.until);
    out
}

pub fn build_layout(input: &LayoutInput<'_>) -> SettingsLayout {
    let cw = input.client_w.max(320.0);
    let ch = input.client_h.max(240.0);
    let mut hits: Vec<HitRegion> = Vec::new();
    let mut row_context_hits: Vec<HitRegion> = Vec::new();

    // ---- 1. Left Sidebar (232 DIP fixed) ----
    let mut nav_rows = Vec::new();
    let nav_x = 10.0;
    let nav_w = SIDEBAR_W_DIP - 20.0;
    let mut ny = 14.0;
    let update_ready = input.update_ready_version.is_some();

    for (page_id, show_dot) in [
        (PageId::General, update_ready),
        (PageId::Widget, false),
        (PageId::Style, false),
    ] {
        let rect = DipRect::new(nav_x, ny, nav_w, 36.0);
        let action = Action::SelectPage(page_id);
        nav_rows.push(NavRow {
            rect,
            label: page_id.title(),
            provider_icon: None,
            selected: input.page == page_id,
            show_dot,
            action: action.clone(),
        });
        hits.push(HitRegion {
            rect,
            action,
            enabled: true,
        });
        ny += 38.0;
    }

    ny += 14.0;
    let nav_accounts_label = DipRect::new(nav_x + 12.0, ny, nav_w - 16.0, 18.0);
    ny += 22.0;

    for &wp in &input.settings.provider_order {
        let kind = ProviderKind::from_widget_provider(wp);
        let page_id = PageId::Provider(kind);
        let rect = DipRect::new(nav_x, ny, nav_w, 36.0);
        let action = Action::SelectPage(page_id);
        nav_rows.push(NavRow {
            rect,
            label: kind.display_name(),
            provider_icon: Some(kind),
            selected: input.page == page_id,
            show_dot: false,
            action: action.clone(),
        });
        hits.push(HitRegion {
            rect,
            action,
            enabled: true,
        });
        ny += 38.0;
    }

    let nav_update_btn = input.update_ready_version.map(|ver| {
        let rect = DipRect::new(nav_x, (ch - 48.0).max(ny + 8.0), nav_w, 36.0);
        hits.push(HitRegion {
            rect,
            action: Action::CheckUpdates,
            enabled: true,
        });
        let label = if ver.is_empty() {
            "Install update".to_string()
        } else {
            format!("Install v{ver}")
        };
        (rect, label)
    });

    // ---- 2. Right Content Pane ----
    let content_viewport = DipRect::new(SIDEBAR_W_DIP, 0.0, (cw - SIDEBAR_W_DIP).max(120.0), ch);
    let inner_avail = (content_viewport.w - CONTENT_PAD_X_DIP * 2.0 - 10.0).max(260.0);
    let col_w = inner_avail.min(MAX_CONTENT_W_DIP);
    let col_x = content_viewport.x + CONTENT_PAD_X_DIP;

    let mut builder = ContentBuilder::new(col_x, col_w);
    match input.page {
        PageId::General => build_general_page(&mut builder, input),
        PageId::Widget => build_widget_page(&mut builder, input),
        PageId::Style => build_style_page(&mut builder, input),
        PageId::Provider(ProviderKind::OpenCode) => build_opencode_page(&mut builder, input),
        PageId::Provider(kind) => build_provider_page(&mut builder, kind, input),
    }

    let content_height = builder.y + CONTENT_PAD_BOTTOM_DIP;
    let max_scroll = (content_height - ch).max(0.0);
    let scroll_y = input.scroll_y.clamp(0.0, max_scroll);

    // Shift content-space hit regions into client-space and clip to content_viewport.
    for h in builder.hits {
        let shifted = h.rect.offset_y(-scroll_y);
        if shifted.y + shifted.h > content_viewport.y
            && shifted.y < content_viewport.y + content_viewport.h
        {
            hits.push(HitRegion {
                rect: shifted,
                action: h.action,
                enabled: h.enabled,
            });
        }
    }
    for h in builder.row_context_hits {
        let shifted = h.rect.offset_y(-scroll_y);
        if shifted.y + shifted.h > content_viewport.y
            && shifted.y < content_viewport.y + content_viewport.h
        {
            row_context_hits.push(HitRegion {
                rect: shifted,
                action: h.action,
                enabled: h.enabled,
            });
        }
    }

    // Custom scrollbar geometry when overflowing.
    let scrollbar = if max_scroll > 0.5 {
        let track = DipRect::new(cw - 10.0, 8.0, 6.0, (ch - 16.0).max(32.0));
        let ratio = (ch / content_height).clamp(0.08, 1.0);
        let thumb_h = (track.h * ratio).max(28.0);
        let travel = (track.h - thumb_h).max(0.0);
        let thumb_y = track.y + (scroll_y / max_scroll) * travel;
        Some(ScrollbarGeometry {
            track,
            thumb: DipRect::new(track.x, thumb_y, track.w, thumb_h),
            max_scroll,
        })
    } else {
        None
    };

    // Inline bottom-right toast.
    let toast = input.toast.filter(|t| !t.is_empty()).map(|msg| {
        let est_w = (msg.chars().count() as f32 * 7.2 + 32.0).clamp(130.0, 360.0);
        let rect = DipRect::new(cw - est_w - 20.0, ch - 52.0, est_w, 34.0);
        (rect, msg.to_string())
    });

    SettingsLayout {
        client_h: ch,
        nav_rows,
        nav_accounts_label,
        nav_update_btn,
        content_viewport,
        scroll_y,
        content_height,
        content_items: builder.items,
        scrollbar,
        hits,
        row_context_hits,
        toast,
    }
}

struct ContentBuilder {
    x: f32,
    w: f32,
    y: f32,
    items: Vec<ContentItem>,
    hits: Vec<HitRegion>,
    row_context_hits: Vec<HitRegion>,
}

impl ContentBuilder {
    fn new(x: f32, w: f32) -> Self {
        Self {
            x,
            w,
            y: CONTENT_PAD_TOP_DIP,
            items: Vec::new(),
            hits: Vec::new(),
            row_context_hits: Vec::new(),
        }
    }

    fn add_hit(&mut self, rect: DipRect, action: Action, enabled: bool) {
        self.hits.push(HitRegion {
            rect,
            action,
            enabled,
        });
    }

    fn add_page_header(
        &mut self,
        icon: Option<ProviderKind>,
        title: impl Into<String>,
        subtitle: Option<String>,
        right_buttons: &[(String, ButtonStyle, bool, Action)],
    ) {
        let h = if subtitle.is_some() { 48.0 } else { 40.0 };
        let rect = DipRect::new(self.x, self.y, self.w, h);
        self.items.push(ContentItem::PageHeader {
            rect,
            icon,
            title: title.into(),
            subtitle,
        });

        let mut rx = self.x + self.w;
        for (label, style, disabled, action) in right_buttons.iter().rev() {
            let bw = estimate_btn_width(label, *style);
            rx -= bw;
            let brect = DipRect::new(rx, self.y + (h - 32.0) * 0.5, bw, 32.0);
            self.items.push(ContentItem::Button {
                rect: brect,
                label: label.clone(),
                style: *style,
                disabled: *disabled,
                action: action.clone(),
            });
            self.add_hit(brect, action.clone(), !disabled);
            rx -= 8.0;
        }
        self.y += h + 20.0;
    }

    fn add_section_heading(&mut self, title: &str, top_gap: f32) {
        self.y += top_gap;
        self.items.push(ContentItem::SectionHeading {
            rect: DipRect::new(self.x, self.y, self.w, 20.0),
            text: title.to_string(),
        });
        self.y += 26.0;
    }

    fn add_toggle_row(
        &mut self,
        title: &str,
        subtitle: Option<&str>,
        checked: bool,
        action: Action,
    ) {
        let row_h = 68.0;
        let card = DipRect::new(self.x, self.y, self.w, row_h);
        self.items.push(ContentItem::Card { rect: card });

        let text_w = (self.w - 84.0).max(100.0);
        if let Some(sub) = subtitle {
            self.items.push(ContentItem::Text {
                rect: DipRect::new(self.x + 16.0, self.y + 14.0, text_w, 20.0),
                text: title.to_string(),
                size: 14.0,
                semi_bold: false,
                muted: false,
                warn: false,
                align_right: false,
                align_center: false,
                mono: false,
                wrap: false,
            });
            self.items.push(ContentItem::Text {
                rect: DipRect::new(self.x + 16.0, self.y + 36.0, text_w, 18.0),
                text: sub.to_string(),
                size: 12.0,
                semi_bold: false,
                muted: true,
                warn: false,
                align_right: false,
                align_center: false,
                mono: false,
                wrap: false,
            });
        } else {
            self.items.push(ContentItem::Text {
                rect: DipRect::new(self.x + 16.0, self.y + (row_h - 20.0) * 0.5, text_w, 20.0),
                text: title.to_string(),
                size: 14.0,
                semi_bold: false,
                muted: false,
                warn: false,
                align_right: false,
                align_center: false,
                mono: false,
                wrap: false,
            });
        }

        let toggle = DipRect::new(
            self.x + self.w - 16.0 - 40.0,
            self.y + (row_h - 20.0) * 0.5,
            40.0,
            20.0,
        );
        self.items.push(ContentItem::Toggle {
            rect: toggle,
            checked,
            action: action.clone(),
        });
        // Generous 28px vertical hit box for WinUI toggle accessibility.
        self.add_hit(
            DipRect::new(
                toggle.x - 4.0,
                toggle.y - 6.0,
                toggle.w + 8.0,
                toggle.h + 12.0,
            ),
            action,
            true,
        );
        self.y += row_h + 4.0;
    }

    fn add_segmented_row(&mut self, title: &str, options: &[(&'static str, bool, Action)]) {
        let row_h = 68.0;
        let card = DipRect::new(self.x, self.y, self.w, row_h);
        self.items.push(ContentItem::Card { rect: card });

        let seg_h = 34.0;
        let mut opt_widths = Vec::with_capacity(options.len());
        let mut total_w = 6.0;
        for (idx, (lbl, _, _)) in options.iter().enumerate() {
            let ow = (lbl.len() as f32 * 7.6 + 24.0).max(58.0);
            opt_widths.push(ow);
            total_w += ow;
            if idx + 1 < options.len() {
                total_w += 2.0;
            }
        }

        let track = DipRect::new(
            self.x + self.w - 16.0 - total_w,
            self.y + (row_h - seg_h) * 0.5,
            total_w,
            seg_h,
        );
        let text_w = (track.x - self.x - 28.0).max(80.0);
        self.items.push(ContentItem::Text {
            rect: DipRect::new(self.x + 16.0, self.y + (row_h - 20.0) * 0.5, text_w, 20.0),
            text: title.to_string(),
            size: 14.0,
            semi_bold: false,
            muted: false,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });

        let mut ox = track.x + 3.0;
        let mut built_opts = Vec::with_capacity(options.len());
        for ((lbl, selected, action), ow) in options.iter().zip(opt_widths) {
            let orect = DipRect::new(ox, track.y + 3.0, ow, seg_h - 6.0);
            built_opts.push(SegOption {
                rect: orect,
                label: lbl,
                selected: *selected,
                action: action.clone(),
            });
            self.add_hit(orect, action.clone(), true);
            ox += ow + 2.0;
        }
        self.items.push(ContentItem::SegmentedControl {
            track,
            options: built_opts,
        });
        self.y += row_h + 4.0;
    }

    fn add_bars_section(
        &mut self,
        wp: WidgetProvider,
        settings: &Settings,
        scoped_label: Option<&str>,
    ) {
        self.add_section_heading("Bars", 24.0);
        let bars = provider_bars(wp, scoped_label);
        let hidden = settings
            .hidden_bars
            .get(&wp)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let on_count = bars
            .iter()
            .filter(|(id, _)| !hidden.iter().any(|h| h == id))
            .count();

        let row_h = 68.0;
        let card = DipRect::new(self.x, self.y, self.w, row_h);
        self.items.push(ContentItem::Card { rect: card });

        let mut chip_widths = Vec::with_capacity(bars.len());
        let mut total_chips_w = 0.0;
        for (idx, (_, label)) in bars.iter().enumerate() {
            let cw = (label.chars().count() as f32 * 6.8 + 34.0).max(74.0);
            chip_widths.push(cw);
            total_chips_w += cw;
            if idx + 1 < bars.len() {
                total_chips_w += 6.0;
            }
        }

        let start_x = (self.x + self.w - 16.0 - total_chips_w).max(self.x + 130.0);
        let title_w = (start_x - self.x - 24.0).max(80.0);
        self.items.push(ContentItem::Text {
            rect: DipRect::new(self.x + 16.0, self.y + (row_h - 20.0) * 0.5, title_w, 20.0),
            text: "Widget bars".to_string(),
            size: 14.0,
            semi_bold: false,
            muted: false,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });

        let mut cx = start_x;
        let chip_y = self.y + (row_h - 28.0) * 0.5;
        let mut chips = Vec::with_capacity(bars.len());
        for ((id, label), cw) in bars.into_iter().zip(chip_widths) {
            let checked = !hidden.iter().any(|h| h == id);
            let disabled = checked && on_count <= 1;
            let crect = DipRect::new(cx, chip_y, cw, 28.0);
            let action = Action::ToggleBar {
                provider: wp,
                bar_id: id.to_string(),
            };
            chips.push(BarChip {
                rect: crect,
                label,
                checked,
                disabled,
                action: action.clone(),
            });
            self.add_hit(crect, action, !disabled);
            cx += cw + 6.0;
        }
        self.items.push(ContentItem::BarChipRow { chips });
        self.y += row_h + 4.0;

        if wp == WidgetProvider::Claude {
            self.add_toggle_row(
                "Fable color on bars",
                None,
                settings.fable_shade,
                Action::ToggleFableShade,
            );
        }
    }
}

fn estimate_btn_width(label: &str, style: ButtonStyle) -> f32 {
    match style {
        ButtonStyle::GhostIcon => 28.0,
        ButtonStyle::Pill => (label.chars().count() as f32 * 6.8 + 20.0).max(64.0),
        ButtonStyle::Primary | ButtonStyle::Secondary => {
            (label.chars().count() as f32 * 7.2 + 28.0).max(76.0)
        }
    }
}

// ---- Page 1: General ------------------------------------------------------------------------

fn build_general_page(b: &mut ContentBuilder, input: &LayoutInput<'_>) {
    b.add_page_header(None, "General", None, &[]);

    b.add_section_heading("UsageDeck", 0.0);

    // Row 1: Version + update status + Check for updates / Update available button.
    let row_h = 68.0;
    let card1 = DipRect::new(b.x, b.y, b.w, row_h);
    b.items.push(ContentItem::Card { rect: card1 });
    b.items.push(ContentItem::AppIcon {
        rect: DipRect::new(b.x + 16.0, b.y + (row_h - 20.0) * 0.5, 20.0, 20.0),
    });

    let (btn_label, btn_style, btn_disabled) = if let Some(ver) = input.update_ready_version {
        let label = if ver.is_empty() {
            "Install update".to_string()
        } else {
            format!("Install v{ver}")
        };
        (label, ButtonStyle::Primary, false)
    } else if input.update_checking {
        ("Checking…".to_string(), ButtonStyle::Secondary, true)
    } else {
        (
            "Check for updates".to_string(),
            ButtonStyle::Secondary,
            false,
        )
    };
    let bw = estimate_btn_width(&btn_label, btn_style);
    let brect = DipRect::new(b.x + b.w - 16.0 - bw, b.y + (row_h - 32.0) * 0.5, bw, 32.0);
    b.items.push(ContentItem::Button {
        rect: brect,
        label: btn_label,
        style: btn_style,
        disabled: btn_disabled,
        action: Action::CheckUpdates,
    });
    b.add_hit(brect, Action::CheckUpdates, !btn_disabled);

    let title_x = b.x + 48.0;
    let title_w = (brect.x - title_x - 16.0).max(100.0);
    let version_title = format!("Version {}", input.app_version);
    let show_status_sub = !input.update_status_text.is_empty()
        && input.update_status_text != version_title
        && input.update_status_text != format!("UsageDeck v{}", input.app_version);
    if show_status_sub {
        b.items.push(ContentItem::Text {
            rect: DipRect::new(title_x, b.y + 14.0, title_w, 20.0),
            text: version_title,
            size: 14.0,
            semi_bold: false,
            muted: false,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });
        b.items.push(ContentItem::Text {
            rect: DipRect::new(title_x, b.y + 36.0, title_w, 18.0),
            text: input.update_status_text.to_string(),
            size: 12.0,
            semi_bold: false,
            muted: true,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });
    } else {
        b.items.push(ContentItem::Text {
            rect: DipRect::new(title_x, b.y + (row_h - 20.0) * 0.5, title_w, 20.0),
            text: version_title,
            size: 14.0,
            semi_bold: false,
            muted: false,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });
    }
    b.y += row_h + 4.0;

    // Row 2: Support + Account folder / Log folder / Copy diagnostics.
    let card2 = DipRect::new(b.x, b.y, b.w, row_h);
    b.items.push(ContentItem::Card { rect: card2 });
    let support_btns = [
        ("Account folder", Action::OpenAccountFolder),
        ("Log folder", Action::OpenLogFolder),
        ("Copy diagnostics", Action::CopyDiagnostics),
    ];
    let mut rx = b.x + b.w - 16.0;
    for (lbl, act) in support_btns.iter().rev() {
        let w = estimate_btn_width(lbl, ButtonStyle::Secondary);
        rx -= w;
        let r = DipRect::new(rx, b.y + (row_h - 32.0) * 0.5, w, 32.0);
        b.items.push(ContentItem::Button {
            rect: r,
            label: (*lbl).to_string(),
            style: ButtonStyle::Secondary,
            disabled: false,
            action: act.clone(),
        });
        b.add_hit(r, act.clone(), true);
        rx -= 8.0;
    }
    b.items.push(ContentItem::Text {
        rect: DipRect::new(
            b.x + 16.0,
            b.y + (row_h - 20.0) * 0.5,
            (rx - b.x - 24.0).max(60.0),
            20.0,
        ),
        text: "Support".to_string(),
        size: 14.0,
        semi_bold: false,
        muted: false,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: false,
    });
    b.y += row_h + 26.0;

    // Behavior rows
    b.add_toggle_row(
        "Start with Windows",
        None,
        input.settings.start_at_login,
        Action::ToggleStartAtLogin,
    );
    b.add_toggle_row(
        "Hide during full-screen",
        None,
        input.settings.hide_in_fullscreen,
        Action::ToggleHideInFullscreen,
    );
    b.add_toggle_row(
        "Auto-hide full cards until reset",
        Some("Temporarily hide exhausted accounts from the taskbar until usage comes back"),
        input.settings.auto_hide_exhausted,
        Action::ToggleAutoHideExhausted,
    );

    // Theme section
    b.add_section_heading("Theme", 26.0);
    b.add_segmented_row(
        "App skin",
        &[
            (
                "Default",
                input.settings.app_skin == AppSkin::Default,
                Action::SetAppSkin(AppSkin::Default),
            ),
            (
                "Gemini Gem",
                input.settings.app_skin == AppSkin::GeminiGem,
                Action::SetAppSkin(AppSkin::GeminiGem),
            ),
        ],
    );
    b.add_segmented_row(
        "Settings theme",
        &[
            (
                "Light",
                input.settings.panel_theme == PanelTheme::Light,
                Action::SetPanelTheme(PanelTheme::Light),
            ),
            (
                "Dark",
                input.settings.panel_theme == PanelTheme::Dark,
                Action::SetPanelTheme(PanelTheme::Dark),
            ),
            (
                "System",
                input.settings.panel_theme == PanelTheme::System,
                Action::SetPanelTheme(PanelTheme::System),
            ),
        ],
    );
    b.add_segmented_row(
        "Widget theme",
        &[
            (
                "Auto",
                input.settings.theme_mode == ThemeMode::Auto,
                Action::SetWidgetTheme(ThemeMode::Auto),
            ),
            (
                "Light",
                input.settings.theme_mode == ThemeMode::Light,
                Action::SetWidgetTheme(ThemeMode::Light),
            ),
            (
                "Dark",
                input.settings.theme_mode == ThemeMode::Dark,
                Action::SetWidgetTheme(ThemeMode::Dark),
            ),
        ],
    );

    // App control row (replaces taskbar right-click Quit).
    b.y += 18.0;
    let quit_h = 54.0;
    let quit_card = DipRect::new(b.x, b.y, b.w, quit_h);
    b.items.push(ContentItem::Card { rect: quit_card });
    let quit_label = "Quit UsageDeck";
    let quit_bw = estimate_btn_width(quit_label, ButtonStyle::Secondary);
    let quit_rect = DipRect::new(
        b.x + b.w - 16.0 - quit_bw,
        b.y + (quit_h - 32.0) * 0.5,
        quit_bw,
        32.0,
    );
    b.items.push(ContentItem::Button {
        rect: quit_rect,
        label: quit_label.to_string(),
        style: ButtonStyle::Secondary,
        disabled: false,
        action: Action::QuitApp,
    });
    b.add_hit(quit_rect, Action::QuitApp, true);
    b.items.push(ContentItem::Text {
        rect: DipRect::new(
            b.x + 16.0,
            b.y + (quit_h - 20.0) * 0.5,
            (quit_rect.x - b.x - 28.0).max(80.0),
            20.0,
        ),
        text: "Exit UsageDeck and close the taskbar widget".to_string(),
        size: 13.5,
        semi_bold: false,
        muted: true,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: false,
    });
    b.y += quit_h;
}

// ---- Page 2b: Style -------------------------------------------------------------------------

fn build_style_page(b: &mut ContentBuilder, input: &LayoutInput<'_>) {
    b.add_page_header(
        None,
        "Style",
        Some(
            "Choose your app skin and how rolling (R), weekly (W), and monthly (M) meters look on your taskbar."
                .to_string(),
        ),
        &[],
    );
    b.add_section_heading("Skin", 0.0);
    b.add_segmented_row(
        "App skin",
        &[
            (
                "Default",
                input.settings.app_skin == AppSkin::Default,
                Action::SetAppSkin(AppSkin::Default),
            ),
            (
                "Gemini Gem",
                input.settings.app_skin == AppSkin::GeminiGem,
                Action::SetAppSkin(AppSkin::GeminiGem),
            ),
        ],
    );
    b.add_section_heading("Taskbar meter design", 26.0);

    let styles: [(TaskbarMeterStyle, &'static str, &'static str); 4] = [
        (
            TaskbarMeterStyle::ClockBelow,
            "Hours & minutes",
            "R / W / M + % on top, bar in the middle, clear 4h 33m · 5d countdown underneath.",
        ),
        (
            TaskbarMeterStyle::MinimalBelow,
            "Minimal rounded",
            "Single-unit rounded time (4.5h · 5d) underneath — zero minute-by-minute jitter.",
        ),
        (
            TaskbarMeterStyle::ResetAtBelow,
            "Reset wall clock",
            "Shows directly when quota resets (→ 8:40p · → Sat) — no countdown math.",
        ),
        (
            TaskbarMeterStyle::PillChip,
            "Compact pill chip",
            "2-tier layout with a subtle [R] badge + 4h 33m beside % and a thicker bar below.",
        ),
    ];

    let card_h = 84.0;
    for (style, title, subtitle) in styles {
        let rect = DipRect::new(b.x, b.y, b.w, card_h);
        let action = Action::SetMeterStyle(style);
        let selected = input.settings.taskbar_meter_style == style;
        b.items.push(ContentItem::MeterStyleCard {
            rect,
            style,
            title,
            subtitle,
            selected,
            action: action.clone(),
        });
        b.add_hit(rect, action, true);
        b.y += card_h + 10.0;
    }
}

// ---- Page 2: Widget -------------------------------------------------------------------------

fn build_widget_page(b: &mut ContentBuilder, input: &LayoutInput<'_>) {
    b.add_page_header(None, "Widget", None, &[]);
    b.add_section_heading("Providers", 0.0);

    let snoozed = snoozed_entries(input.settings, input.now_ms);
    let order = &input.settings.provider_order;

    for (idx, &wp) in order.iter().enumerate() {
        let kind = ProviderKind::from_widget_provider(wp);
        let prov_snoozed: Vec<&SnoozedEntry> =
            snoozed.iter().filter(|e| e.provider == wp).collect();
        let expanded = input.expanded_snoozed == Some(wp) && !prov_snoozed.is_empty();

        let base_h = 54.0;
        let sub_h = if expanded {
            10.0 + prov_snoozed.len() as f32 * 36.0 + 4.0
        } else {
            0.0
        };
        let card_h = base_h + sub_h;
        let card = DipRect::new(b.x, b.y, b.w, card_h);
        b.items.push(ContentItem::Card { rect: card });

        // Compact Up / Down reorder buttons on left.
        let up_rect = DipRect::new(b.x + 12.0, b.y + (base_h - 24.0) * 0.5, 24.0, 24.0);
        let up_disabled = idx == 0;
        b.items.push(ContentItem::Button {
            rect: up_rect,
            label: "↑".to_string(),
            style: ButtonStyle::GhostIcon,
            disabled: up_disabled,
            action: Action::MoveProviderUp(wp),
        });
        b.add_hit(up_rect, Action::MoveProviderUp(wp), !up_disabled);

        let dn_rect = DipRect::new(b.x + 38.0, b.y + (base_h - 24.0) * 0.5, 24.0, 24.0);
        let dn_disabled = idx + 1 == order.len();
        b.items.push(ContentItem::Button {
            rect: dn_rect,
            label: "↓".to_string(),
            style: ButtonStyle::GhostIcon,
            disabled: dn_disabled,
            action: Action::MoveProviderDown(wp),
        });
        b.add_hit(dn_rect, Action::MoveProviderDown(wp), !dn_disabled);

        // 20x20 provider icon + name.
        b.items.push(ContentItem::ProviderIcon {
            rect: DipRect::new(b.x + 72.0, b.y + (base_h - 20.0) * 0.5, 20.0, 20.0),
            kind,
        });

        // Right-side toggle.
        let toggle = DipRect::new(
            b.x + b.w - 16.0 - 40.0,
            b.y + (base_h - 20.0) * 0.5,
            40.0,
            20.0,
        );
        b.items.push(ContentItem::Toggle {
            rect: toggle,
            checked: provider_enabled(input.settings, wp),
            action: Action::ToggleProviderEnabled(wp),
        });
        b.add_hit(
            DipRect::new(
                toggle.x - 4.0,
                toggle.y - 6.0,
                toggle.w + 8.0,
                toggle.h + 12.0,
            ),
            Action::ToggleProviderEnabled(wp),
            true,
        );

        let mut right_edge = toggle.x - 12.0;

        // Optional "{N} hidden" snoozed pill button.
        if !prov_snoozed.is_empty() {
            let pill_label = format!("{} hidden", prov_snoozed.len());
            let pw = estimate_btn_width(&pill_label, ButtonStyle::Pill);
            let prect = DipRect::new(right_edge - pw, b.y + (base_h - 24.0) * 0.5, pw, 24.0);
            b.items.push(ContentItem::Button {
                rect: prect,
                label: pill_label,
                style: ButtonStyle::Pill,
                disabled: false,
                action: Action::ToggleSnoozedExpand(wp),
            });
            b.add_hit(prect, Action::ToggleSnoozedExpand(wp), true);
            right_edge = prect.x - 10.0;
        }

        b.items.push(ContentItem::Text {
            rect: DipRect::new(
                b.x + 104.0,
                b.y + (base_h - 20.0) * 0.5,
                (right_edge - (b.x + 104.0)).max(60.0),
                20.0,
            ),
            text: kind.display_name().to_string(),
            size: 14.0,
            semi_bold: false,
            muted: false,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });

        // Inline expanded snoozed rows.
        if expanded {
            b.items.push(ContentItem::Divider {
                rect: DipRect::new(b.x + 12.0, b.y + base_h, b.w - 24.0, 1.0),
            });
            let mut sy = b.y + base_h + 8.0;
            for entry in prov_snoozed {
                let unhide_w = estimate_btn_width("Unhide", ButtonStyle::Secondary);
                let unhide_rect =
                    DipRect::new(b.x + b.w - 16.0 - unhide_w, sy + 2.0, unhide_w, 28.0);
                let act = Action::UnhideSnoozed(entry.key.clone());
                b.items.push(ContentItem::Button {
                    rect: unhide_rect,
                    label: "Unhide".to_string(),
                    style: ButtonStyle::Secondary,
                    disabled: false,
                    action: act.clone(),
                });
                b.add_hit(unhide_rect, act, true);

                let rem_text = format!(
                    "back in {}",
                    format_snooze_duration(entry.until.saturating_sub(input.now_ms))
                );
                let rem_x = unhide_rect.x - 140.0;
                b.items.push(ContentItem::Text {
                    rect: DipRect::new(
                        b.x + 72.0,
                        sy + 6.0,
                        (rem_x - (b.x + 80.0)).max(60.0),
                        20.0,
                    ),
                    text: entry.name.clone(),
                    size: 13.0,
                    semi_bold: false,
                    muted: false,
                    warn: false,
                    align_right: false,
                    align_center: false,
                    mono: false,
                    wrap: false,
                });
                b.items.push(ContentItem::Text {
                    rect: DipRect::new(rem_x, sy + 6.0, 130.0, 20.0),
                    text: rem_text,
                    size: 12.0,
                    semi_bold: false,
                    muted: true,
                    warn: false,
                    align_right: true,
                    align_center: false,
                    mono: false,
                    wrap: false,
                });
                sy += 36.0;
            }
        }

        b.y += card_h + 4.0;
    }

    b.add_section_heading("Cards", 26.0);
    b.add_toggle_row(
        "Show provider names",
        None,
        input.settings.show_provider_names,
        Action::ToggleShowProviderNames,
    );

    b.add_section_heading("System", 26.0);
    b.add_toggle_row(
        "CPU & Memory monitor",
        Some("Show live CPU and memory usage on the taskbar with parent-app breakdown on hover"),
        input.settings.system_monitor_enabled,
        Action::ToggleSystemMonitor,
    );
}

// ---- Pages 3-6: Tracked Providers (Claude, Codex, Cursor, Grok Bot) -------------------------

fn rows_for_provider(snapshot: &SettingsSnapshot, kind: ProviderKind) -> &[SettingsAccountRow] {
    match kind {
        ProviderKind::Claude => &snapshot.claude,
        ProviderKind::Codex => &snapshot.codex,
        ProviderKind::Cursor => &snapshot.cursor,
        ProviderKind::Grok => &snapshot.grok,
        ProviderKind::OpenCode => &snapshot.opencode,
        ProviderKind::Antigravity => &snapshot.antigravity,
    }
}

fn build_provider_page(b: &mut ContentBuilder, kind: ProviderKind, input: &LayoutInput<'_>) {
    let rows = input.snapshot.map(|s| rows_for_provider(s, kind));
    let subtitle = match rows {
        Some(list) => Some(format!(
            "{} {}",
            list.len(),
            if list.len() == 1 {
                "account"
            } else {
                "accounts"
            }
        )),
        None => Some("Loading…".to_string()),
    };

    b.add_page_header(
        Some(kind),
        kind.display_name(),
        subtitle,
        &[(
            if input.loading {
                "Refreshing…".to_string()
            } else {
                "Refresh".to_string()
            },
            ButtonStyle::Secondary,
            input.loading,
            Action::RefreshData,
        )],
    );

    match rows {
        None => build_loading_card(b),
        Some([]) => build_empty_state_card(
            b,
            &format!(
                "UsageDeck reads {}'s own sign-in. Sign in with {} and the account shows up here.",
                kind.display_name(),
                kind.display_name()
            ),
            None,
        ),
        Some(list) => build_account_table(b, kind, list, input.settings),
    }

    let scoped_label = if kind == ProviderKind::Claude {
        rows.and_then(|list| {
            list.iter()
                .flat_map(|r| r.bars.iter())
                .find(|bar| bar.key == "scoped")
                .map(|bar| bar.label.as_str())
        })
    } else {
        None
    };

    b.add_bars_section(kind.to_widget_provider(), input.settings, scoped_label);

    if kind == ProviderKind::Cursor {
        build_cursor_diagnostics(b, input);
    }

    if kind == ProviderKind::Claude {
        build_claude_pace_section(b, input);
    }
}

// ---- Page 7: OpenCode -----------------------------------------------------------------------

fn build_opencode_page(b: &mut ContentBuilder, input: &LayoutInput<'_>) {
    let rows = input.snapshot.map(|s| s.opencode.as_slice());
    let subtitle = match rows {
        Some(list) => Some(format!(
            "{} {}",
            list.len(),
            if list.len() == 1 {
                "account"
            } else {
                "accounts"
            }
        )),
        None => Some("Loading…".to_string()),
    };

    let header_btns = vec![
        (
            if input.loading {
                "Refreshing…".to_string()
            } else {
                "Refresh".to_string()
            },
            ButtonStyle::Secondary,
            input.loading,
            Action::RefreshData,
        ),
        (
            "Add account".to_string(),
            ButtonStyle::Primary,
            false,
            Action::AddOpenCodeAccount,
        ),
    ];

    b.add_page_header(
        Some(ProviderKind::OpenCode),
        "OpenCode",
        subtitle,
        &header_btns,
    );

    match rows {
        None => build_loading_card(b),
        Some([]) => build_empty_state_card(
            b,
            "Add an OpenCode Go API key to track its usage.",
            Some(("Add account", Action::AddOpenCodeAccount)),
        ),
        Some(list) => build_account_table(b, ProviderKind::OpenCode, list, input.settings),
    }

    b.add_bars_section(WidgetProvider::Opencode, input.settings, None);

    b.y += 22.0;
    b.add_toggle_row(
        "DeepSeek peak-hour warning",
        None,
        input.settings.deepseek_peak_hour_warning,
        Action::ToggleDeepSeekPeakWarning,
    );
}

// ---- Shared Table, Empty State, Diagnostics, Spend & Pace -----------------------------------

fn build_loading_card(b: &mut ContentBuilder) {
    let h = 64.0;
    let card = DipRect::new(b.x, b.y, b.w, h);
    b.items.push(ContentItem::Card { rect: card });
    b.items.push(ContentItem::Text {
        rect: DipRect::new(b.x + 16.0, b.y + 22.0, b.w - 32.0, 20.0),
        text: "Loading…".to_string(),
        size: 13.5,
        semi_bold: false,
        muted: true,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: false,
    });
    b.y += h + 4.0;
}

fn build_empty_state_card(b: &mut ContentBuilder, message: &str, cta: Option<(&str, Action)>) {
    let h = if cta.is_some() { 140.0 } else { 98.0 };
    let card = DipRect::new(b.x, b.y, b.w, h);
    b.items.push(ContentItem::Card { rect: card });
    b.items.push(ContentItem::Text {
        rect: DipRect::new(b.x + 24.0, b.y + 22.0, b.w - 48.0, 22.0),
        text: "No accounts yet".to_string(),
        size: 14.5,
        semi_bold: true,
        muted: false,
        warn: false,
        align_right: false,
        align_center: true,
        mono: false,
        wrap: false,
    });
    b.items.push(ContentItem::Text {
        rect: DipRect::new(b.x + 32.0, b.y + 48.0, b.w - 64.0, 34.0),
        text: message.to_string(),
        size: 12.5,
        semi_bold: false,
        muted: true,
        warn: false,
        align_right: false,
        align_center: true,
        mono: false,
        wrap: true,
    });
    if let Some((lbl, action)) = cta {
        let bw = estimate_btn_width(lbl, ButtonStyle::Primary);
        let brect = DipRect::new(b.x + (b.w - bw) * 0.5, b.y + 92.0, bw, 32.0);
        b.items.push(ContentItem::Button {
            rect: brect,
            label: lbl.to_string(),
            style: ButtonStyle::Primary,
            disabled: false,
            action: action.clone(),
        });
        b.add_hit(brect, action, true);
    }
    b.y += h + 4.0;
}

fn table_bar_columns(
    kind: ProviderKind,
    rows: &[SettingsAccountRow],
    settings: &Settings,
) -> Vec<(String, String)> {
    let wp = kind.to_widget_provider();
    let hidden = settings
        .hidden_bars
        .get(&wp)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let available: Vec<(String, String)> = match kind {
        ProviderKind::OpenCode => vec![
            ("rolling".into(), "Rolling".into()),
            ("weekly".into(), "Weekly".into()),
            ("monthly".into(), "Monthly".into()),
        ],
        ProviderKind::Cursor => vec![
            ("firstParty".into(), "First Party".into()),
            ("api".into(), "API".into()),
        ],
        ProviderKind::Grok => vec![("weekly".into(), "Weekly".into())],
        ProviderKind::Antigravity => vec![
            ("fiveHour".into(), "5-hour".into()),
            ("weekly".into(), "Weekly".into()),
        ],
        ProviderKind::Codex => {
            let has_weekly = rows
                .iter()
                .any(|r| r.bars.iter().any(|bar| bar.key == "weekly"));
            if has_weekly {
                vec![
                    ("fiveHour".into(), "5-hour".into()),
                    ("weekly".into(), "Weekly".into()),
                ]
            } else {
                vec![("fiveHour".into(), "5-hour".into())]
            }
        }
        ProviderKind::Claude => {
            let scoped = rows
                .iter()
                .flat_map(|r| r.bars.iter())
                .find(|bar| bar.key == "scoped")
                .map(|bar| format!("{} weekly", bar.label));
            let mut cols = vec![
                ("fiveHour".into(), "5-hour".into()),
                ("weekly".into(), "Weekly".into()),
            ];
            if let Some(lbl) = scoped {
                cols.push(("scoped".into(), lbl));
            }
            cols
        }
    };

    let shown: Vec<(String, String)> = available
        .iter()
        .filter(|(id, _)| !hidden.iter().any(|h| h == id))
        .cloned()
        .collect();
    if shown.is_empty() {
        available
    } else {
        shown
    }
}

fn extra_column_header(kind: ProviderKind) -> Option<&'static str> {
    match kind {
        ProviderKind::Claude | ProviderKind::Codex => Some("Resets"),
        ProviderKind::Cursor => Some("API spend"),
        ProviderKind::Grok | ProviderKind::OpenCode | ProviderKind::Antigravity => None,
    }
}

fn build_account_table(
    b: &mut ContentBuilder,
    kind: ProviderKind,
    rows: &[SettingsAccountRow],
    settings: &Settings,
) {
    let col_defs = table_bar_columns(kind, rows, settings);
    let extra_hdr = extra_column_header(kind);

    let header_h = 34.0;
    let row_h = 52.0;
    let total_h = header_h + rows.len() as f32 * row_h;
    let card_rect = DipRect::new(b.x, b.y, b.w, total_h);
    let header_rect = DipRect::new(b.x, b.y, b.w, header_h);

    // Proportional column widths matching `minmax(0,1.6fr) repeat(N, minmax(0,1fr)) [minmax(0,1.4fr)] 28px`.
    let pad_l = 16.0;
    let pad_r = 12.0;
    let gap = 14.0;
    let menu_w = 28.0;
    let num_cols = col_defs.len();
    let has_extra = extra_hdr.is_some();
    let total_gaps = gap * (num_cols + usize::from(has_extra) + 1) as f32;
    let flex_w = (b.w - pad_l - pad_r - menu_w - total_gaps).max(160.0);
    let total_units = 1.6 + num_cols as f32 + if has_extra { 1.35 } else { 0.0 };

    let account_col_x = b.x + pad_l;
    let account_col_w = flex_w * (1.6 / total_units);
    let bar_col_w = flex_w * (1.0 / total_units);
    let extra_col_w = if has_extra {
        flex_w * (1.35 / total_units)
    } else {
        0.0
    };

    let mut cx = account_col_x + account_col_w + gap;
    let bars_start_x = cx;
    let mut cols = Vec::with_capacity(num_cols);
    for (id, header) in &col_defs {
        cols.push(TableColSpec {
            id: id.clone(),
            header: header.clone(),
            x: cx,
            w: bar_col_w,
        });
        cx += bar_col_w + gap;
    }
    let bars_span_w = (cx - gap - bars_start_x).max(60.0);

    let extra_header = extra_hdr.map(|h| {
        let ex = cx;
        cx += extra_col_w + gap;
        (ex, extra_col_w, h.to_string())
    });

    let menu_x = b.x + b.w - pad_r - menu_w;
    let mut built_rows = Vec::with_capacity(rows.len());

    for (idx, row) in rows.iter().enumerate() {
        let ry = b.y + header_h + idx as f32 * row_h;
        let rrect = DipRect::new(b.x, ry, b.w, row_h);
        let shown_name = row
            .nickname
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&row.label)
            .to_string();

        let dimmed_row = row.hidden || row.lapsed || row.stale || row.unsubscribed;
        let menu_action = Action::OpenAccountMenu {
            provider: kind,
            account_id: row.id.clone(),
            label: row.label.clone(),
            nickname: row.nickname.clone(),
            hidden: row.hidden,
            unsubscribed: row.unsubscribed,
            subscription_ends_at: row.subscription_ends_at.clone(),
        };
        let menu_btn = DipRect::new(menu_x, ry + (row_h - 28.0) * 0.5, menu_w, 28.0);
        b.add_hit(menu_btn, menu_action.clone(), true);
        b.row_context_hits.push(HitRegion {
            rect: rrect,
            action: menu_action.clone(),
            enabled: true,
        });

        let (cells, note) = if let Some(ref note_text) = row.note {
            (
                Vec::new(),
                Some((bars_start_x, bars_span_w, note_text.clone())),
            )
        } else {
            let cells = cols
                .iter()
                .map(|c| {
                    let bar: Option<&BarView> = row.bars.iter().find(|b| b.key == c.id);
                    match bar {
                        Some(bv) => {
                            let pct_text = match bv.percent {
                                Some(p) => format!("{}%", p.round().clamp(0.0, 100.0) as i32),
                                None => "--".to_string(),
                            };
                            TableCellRender {
                                x: c.x,
                                w: c.w,
                                percent_text: pct_text,
                                percent_val: bv.percent,
                                reset_text: bv.reset_short.clone(),
                                severity: bv.severity,
                                dimmed: dimmed_row || bv.percent.is_none(),
                            }
                        }
                        None => TableCellRender {
                            x: c.x,
                            w: c.w,
                            percent_text: "--".to_string(),
                            percent_val: None,
                            reset_text: None,
                            severity: BarSeverity::Muted,
                            dimmed: true,
                        },
                    }
                })
                .collect();
            (cells, None)
        };

        let extra = extra_header
            .as_ref()
            .map(|(ex, ew, _)| match &row.extra_text {
                Some(txt) if !txt.is_empty() => (*ex, *ew, txt.clone(), false),
                _ => {
                    let fallback = if matches!(kind, ProviderKind::Claude | ProviderKind::Codex) {
                        "No resets"
                    } else {
                        "—"
                    };
                    (*ex, *ew, fallback.to_string(), true)
                }
            });

        let plan = row
            .plan
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|p| (p.to_string(), plan_badge_tone(p)));
        let sub_label = row
            .subscription_ends_label
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|txt| (txt.clone(), row.subscription_ends_soon));

        built_rows.push(AccountRowRender {
            rect: rrect,
            active: row.active,
            hidden: row.hidden,
            unsubscribed: row.unsubscribed,
            name: shown_name,
            sub_label,
            warning: row.warning.clone(),
            plan,
            cells,
            note,
            extra,
            menu_btn,
            menu_action,
        });
    }

    b.items.push(ContentItem::AccountTable {
        card_rect,
        header_rect,
        account_col_x,
        account_col_w,
        cols,
        extra_header,
        rows: built_rows,
    });
    b.y += total_h + 4.0;
}

fn build_cursor_diagnostics(b: &mut ContentBuilder, input: &LayoutInput<'_>) {
    b.add_section_heading("Diagnostics", 26.0);

    let diag_rows: &[(String, String, bool)] = input
        .snapshot
        .map(|s| s.cursor_diag_rows.as_slice())
        .unwrap_or(&[]);

    let row_h = 22.0;
    let body_h = if diag_rows.is_empty() {
        26.0
    } else {
        diag_rows.len() as f32 * row_h + 4.0
    };
    let card_h = 16.0 + 36.0 + 12.0 + body_h + 12.0 + 18.0 + 16.0;
    let card = DipRect::new(b.x, b.y, b.w, card_h);
    b.items.push(ContentItem::Card { rect: card });

    let copy_w = estimate_btn_width("Copy diagnostics", ButtonStyle::Secondary);
    let ref_w = estimate_btn_width("Refresh", ButtonStyle::Secondary);
    let copy_rect = DipRect::new(b.x + b.w - 16.0 - copy_w, b.y + 16.0, copy_w, 30.0);
    let ref_rect = DipRect::new(copy_rect.x - 8.0 - ref_w, b.y + 16.0, ref_w, 30.0);

    b.items.push(ContentItem::Button {
        rect: ref_rect,
        label: "Refresh".to_string(),
        style: ButtonStyle::Secondary,
        disabled: input.loading,
        action: Action::RefreshData,
    });
    b.add_hit(ref_rect, Action::RefreshData, !input.loading);

    b.items.push(ContentItem::Button {
        rect: copy_rect,
        label: "Copy diagnostics".to_string(),
        style: ButtonStyle::Secondary,
        disabled: false,
        action: Action::CopyDiagnostics,
    });
    b.add_hit(copy_rect, Action::CopyDiagnostics, true);

    b.items.push(ContentItem::Text {
        rect: DipRect::new(
            b.x + 16.0,
            b.y + 14.0,
            (ref_rect.x - b.x - 28.0).max(100.0),
            36.0,
        ),
        text: "What UsageDeck did the last time it read Cursor's usage. Tokens are never shown: only the first characters of a hash.".to_string(),
        size: 12.0,
        semi_bold: false,
        muted: true,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: true,
    });

    let mut dy = b.y + 62.0;
    if diag_rows.is_empty() {
        b.items.push(ContentItem::Text {
            rect: DipRect::new(b.x + 16.0, dy, b.w - 32.0, 22.0),
            text: "Nothing tried yet since UsageDeck started.".to_string(),
            size: 13.0,
            semi_bold: false,
            muted: true,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });
        dy += 26.0;
    } else {
        let label_w = ((b.w - 48.0) * 0.38).clamp(120.0, 280.0);
        let val_x = b.x + 16.0 + label_w + 16.0;
        let val_w = (b.x + b.w - 16.0 - val_x).max(100.0);
        for (label, value, is_bad) in diag_rows {
            b.items.push(ContentItem::Text {
                rect: DipRect::new(b.x + 16.0, dy, label_w, row_h),
                text: label.clone(),
                size: 12.0,
                semi_bold: false,
                muted: true,
                warn: false,
                align_right: false,
                align_center: false,
                mono: false,
                wrap: false,
            });
            b.items.push(ContentItem::Text {
                rect: DipRect::new(val_x, dy, val_w, row_h),
                text: value.clone(),
                size: 12.0,
                semi_bold: *is_bad,
                muted: false,
                warn: *is_bad,
                align_right: false,
                align_center: false,
                mono: false,
                wrap: false,
            });
            dy += row_h;
        }
    }

    b.items.push(ContentItem::Text {
        rect: DipRect::new(b.x + 16.0, dy + 10.0, b.w - 32.0, 18.0),
        text: format!("UsageDeck {} · Windows", input.app_version),
        size: 11.5,
        semi_bold: false,
        muted: true,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: false,
    });

    b.y += card_h + 4.0;
}

fn build_claude_pace_section(b: &mut ContentBuilder, input: &LayoutInput<'_>) {
    b.add_section_heading("Spend & pace", 28.0);

    // Row 1: Hourly spend target.
    let row_h = 68.0;
    let card = DipRect::new(b.x, b.y, b.w, row_h);
    b.items.push(ContentItem::Card { rect: card });

    let clear_w = estimate_btn_width("Clear", ButtonStyle::Secondary);
    let set_w = estimate_btn_width("Set target…", ButtonStyle::Secondary);
    let clear_disabled = input.settings.claude_hourly_target.is_none();
    let clear_rect = DipRect::new(
        b.x + b.w - 16.0 - clear_w,
        b.y + (row_h - 32.0) * 0.5,
        clear_w,
        32.0,
    );
    let set_rect = DipRect::new(
        clear_rect.x - 8.0 - set_w,
        b.y + (row_h - 32.0) * 0.5,
        set_w,
        32.0,
    );
    b.items.push(ContentItem::Button {
        rect: set_rect,
        label: "Set target…".to_string(),
        style: ButtonStyle::Secondary,
        disabled: false,
        action: Action::SetHourlyTarget,
    });
    b.add_hit(set_rect, Action::SetHourlyTarget, true);

    b.items.push(ContentItem::Button {
        rect: clear_rect,
        label: "Clear".to_string(),
        style: ButtonStyle::Secondary,
        disabled: clear_disabled,
        action: Action::ClearHourlyTarget,
    });
    b.add_hit(clear_rect, Action::ClearHourlyTarget, !clear_disabled);

    let target_text = match input.settings.claude_hourly_target {
        Some(v) => format!("${v}/h"),
        None => "Off".to_string(),
    };
    let val_rect = DipRect::new(set_rect.x - 92.0, b.y + (row_h - 20.0) * 0.5, 80.0, 20.0);
    b.items.push(ContentItem::Text {
        rect: val_rect,
        text: target_text,
        size: 14.0,
        semi_bold: input.settings.claude_hourly_target.is_some(),
        muted: input.settings.claude_hourly_target.is_none(),
        warn: false,
        align_right: true,
        align_center: false,
        mono: true,
        wrap: false,
    });

    b.items.push(ContentItem::Text {
        rect: DipRect::new(
            b.x + 16.0,
            b.y + (row_h - 20.0) * 0.5,
            (val_rect.x - b.x - 24.0).max(80.0),
            20.0,
        ),
        text: "Hourly spend target".to_string(),
        size: 14.0,
        semi_bold: false,
        muted: false,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: false,
    });
    b.y += row_h + 4.0;

    let Some(snap) = input.snapshot else {
        build_loading_card(b);
        return;
    };
    let pace = &snap.pace;

    // 4-stat summary card.
    let stat_h = if pace.backfilling { 104.0 } else { 82.0 };
    let stat_card = DipRect::new(b.x, b.y, b.w, stat_h);
    b.items.push(ContentItem::Card { rect: stat_card });
    let fmt_stat = |v: f64| {
        if pace.backfilling {
            "$—".to_string()
        } else {
            format_usd(v)
        }
    };
    let stats = [
        ("Last hour", fmt_stat(pace.last_hour)),
        ("Last 5 h", fmt_stat(pace.last_5h)),
        ("Today", fmt_stat(pace.today)),
        ("Last 7 days", fmt_stat(pace.last_7d)),
    ];
    let cell_w = (b.w - 32.0) / 4.0;
    for (i, (label, val)) in stats.iter().enumerate() {
        let sx = b.x + 16.0 + i as f32 * cell_w;
        b.items.push(ContentItem::Text {
            rect: DipRect::new(sx, b.y + 16.0, cell_w - 8.0, 16.0),
            text: (*label).to_string(),
            size: 12.0,
            semi_bold: false,
            muted: true,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });
        b.items.push(ContentItem::Text {
            rect: DipRect::new(sx, b.y + 34.0, cell_w - 8.0, 28.0),
            text: val.clone(),
            size: 20.0,
            semi_bold: true,
            muted: false,
            warn: false,
            align_right: false,
            align_center: false,
            mono: true,
            wrap: false,
        });
    }
    if pace.backfilling {
        b.items.push(ContentItem::Text {
            rect: DipRect::new(b.x + 16.0, b.y + 72.0, b.w - 32.0, 18.0),
            text: "Reading Claude Code logs for the first time…".to_string(),
            size: 12.0,
            semi_bold: false,
            muted: true,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: false,
        });
    }
    b.y += stat_h + 4.0;

    // Spend Chart card.
    let points = spend_series(pace, input.spend_range);
    let peak = points.iter().map(|(t, _)| *t).fold(1.0_f64, f64::max);
    let models: Vec<(String, String)> = pace
        .by_model
        .iter()
        .filter(|(_, usd, _)| *usd > 0.0)
        .map(|(m, usd, share)| {
            (
                m.clone(),
                format!("{} · {}", format_usd(*usd), format_share_pct(*share)),
            )
        })
        .collect();

    let model_list_h = if models.is_empty() {
        0.0
    } else {
        12.0 + models.len() as f32 * 20.0
    };
    let chart_card_h = 16.0 + 34.0 + 10.0 + 112.0 + 22.0 + model_list_h + 16.0;
    let chart_card = DipRect::new(b.x, b.y, b.w, chart_card_h);

    // Range segmented control (15m | 1h | 24h) inside chart header.
    let seg_opts = [
        (
            "15m",
            input.spend_range == SpendRange::Min15,
            Action::SetSpendRange(SpendRange::Min15),
        ),
        (
            "1h",
            input.spend_range == SpendRange::Hour1,
            Action::SetSpendRange(SpendRange::Hour1),
        ),
        (
            "24h",
            input.spend_range == SpendRange::Hours24,
            Action::SetSpendRange(SpendRange::Hours24),
        ),
    ];
    let opt_w = 50.0;
    let seg_w = opt_w * 3.0 + 10.0;
    let seg_track = DipRect::new(b.x + b.w - 16.0 - seg_w, b.y + 14.0, seg_w, 30.0);
    let mut built_seg = Vec::with_capacity(3);
    for (i, (lbl, sel, act)) in seg_opts.into_iter().enumerate() {
        let r = DipRect::new(
            seg_track.x + 3.0 + i as f32 * (opt_w + 2.0),
            seg_track.y + 3.0,
            opt_w,
            24.0,
        );
        built_seg.push(SegOption {
            rect: r,
            label: lbl,
            selected: sel,
            action: act.clone(),
        });
        b.add_hit(r, act, true);
    }

    let plot_rect = DipRect::new(b.x + 16.0, b.y + 60.0, b.w - 32.0, 112.0);
    b.items.push(ContentItem::SpendChart(SpendChartRender {
        card_rect: chart_card,
        header_title: input.spend_range.title().to_string(),
        plot_rect,
        points,
        peak_label: format_usd_rate(peak),
        left_label: input.spend_range.left_axis_label().to_string(),
        models,
    }));
    b.items.push(ContentItem::SegmentedControl {
        track: seg_track,
        options: built_seg,
    });
    b.y += chart_card_h + 4.0;

    // Calibration row.
    let cal_h = 68.0;
    let cal_card = DipRect::new(b.x, b.y, b.w, cal_h);
    b.items.push(ContentItem::Card { rect: cal_card });
    let cal_value = calibration_text(pace.usd_per_weekly_pct);
    let cal_val_w = (cal_value.chars().count() as f32 * 7.2 + 12.0).clamp(130.0, 290.0);
    b.items.push(ContentItem::Text {
        rect: DipRect::new(
            b.x + 16.0,
            b.y + 14.0,
            (b.w - cal_val_w - 48.0).max(120.0),
            20.0,
        ),
        text: "Calibration".to_string(),
        size: 14.0,
        semi_bold: false,
        muted: false,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: false,
    });
    b.items.push(ContentItem::Text {
        rect: DipRect::new(
            b.x + 16.0,
            b.y + 36.0,
            (b.w - cal_val_w - 48.0).max(120.0),
            18.0,
        ),
        text: "Dollars spent ÷ weekly % gained in this weekly window".to_string(),
        size: 12.0,
        semi_bold: false,
        muted: true,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: false,
    });
    b.items.push(ContentItem::Text {
        rect: DipRect::new(
            b.x + b.w - 16.0 - cal_val_w,
            b.y + (cal_h - 20.0) * 0.5,
            cal_val_w,
            20.0,
        ),
        text: cal_value,
        size: 13.5,
        semi_bold: false,
        muted: pace.usd_per_weekly_pct.is_none(),
        warn: false,
        align_right: true,
        align_center: false,
        mono: false,
        wrap: false,
    });
    b.y += cal_h + 10.0;

    if !pace.unpriced_models.is_empty() {
        let reply_word = if pace.unpriced_replies == 1 {
            "reply"
        } else {
            "replies"
        };
        b.items.push(ContentItem::Text {
            rect: DipRect::new(b.x, b.y, b.w, 20.0),
            text: format!(
                "Not priced: {} ({} {reply_word}, not in the dollar figures)",
                pace.unpriced_models.join(", "),
                pace.unpriced_replies
            ),
            size: 12.0,
            semi_bold: false,
            muted: true,
            warn: false,
            align_right: false,
            align_center: false,
            mono: false,
            wrap: true,
        });
        b.y += 22.0;
    }

    b.items.push(ContentItem::Text {
        rect: DipRect::new(b.x, b.y, b.w, 32.0),
        text: "Dollars cover Claude Code on this PC (API-equivalent, not your bill). Percentages cover all your Claude use.".to_string(),
        size: 12.0,
        semi_bold: false,
        muted: true,
        warn: false,
        align_right: false,
        align_center: false,
        mono: false,
        wrap: true,
    });
    b.y += 32.0;
}
