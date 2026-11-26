//! Pure-Rust DIP layout and hit-testing math for the Variant A taskbar strip
//! and the anchored Windows 11 flyout panel. Zero Win32 dependencies so every
//! layout rule and crowding fallback is unit-tested directly.

use usagedeck_core::view::{
    parse_iso_ms, BarSeverity, ProviderItem, ProviderKind, TaskbarSnapshot,
};

/// Outer DIP height of the taskbar strip window (fits inside Win11 48 DIP and Win10 40 DIP bars).
pub const STRIP_HEIGHT_DIP: f32 = 38.0;
pub const PILL_HEIGHT_DIP: f32 = 34.0;
pub const PILL_GAP_DIP: f32 = 7.0;
pub const STRIP_PAD_X_DIP: f32 = 3.0;

pub const STANDARD_PILL_WIDTH_DIP: f32 = 86.0;
pub const CLAUDE_EXTENDED_PILL_WIDTH_DIP: f32 = 124.0;
pub const WIDE_PAD_X_DIP: f32 = 6.0;
pub const WIDE_ICON_COL_DIP: f32 = 16.0;
pub const WIDE_RATE_ICON_COL_DIP: f32 = 28.0;
pub const WIDE_ICON_GAP_DIP: f32 = 5.0;
pub const WIDE_METER_COL_DIP: f32 = 65.0;
pub const WIDE_SINGLE_METER_COL_DIP: f32 = 70.0;
pub const WIDE_METER_GAP_DIP: f32 = 6.0;
pub const OVERFLOW_BADGE_WIDTH_DIP: f32 = 28.0;
pub const SYSTEM_PILL_WIDTH_DIP: f32 = 88.0;
pub const COMPACT_SUMMARY_WIDTH_DIP: f32 = 52.0;
pub const MIN_VISIBLE_WIDTH_DIP: f32 = 64.0;

pub const FLYOUT_WIDTH_DIP: f32 = 340.0;
pub const FLYOUT_PAD_DIP: f32 = 12.0;
pub const FLYOUT_HEADER_HEIGHT_DIP: f32 = 28.0;
pub const FLYOUT_CARD_GAP_DIP: f32 = 8.0;
pub const FLYOUT_CARD_PAD_DIP: f32 = 10.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DipRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl DipRect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillStyle {
    /// Wide Option 3 side-by-side meters: icon (+ optional rate below) + 1..=2 side-by-side
    /// meter columns, each showing `label + reset_time` on top-left, `%` on top-right, and a
    /// full-width progress bar underneath!
    WideMeters,
    /// Compact fallback when taskbar space is tight: icon + stacked bars + % (still borderless).
    CompactStacked,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PillSlot {
    pub item_index: usize,
    pub rect: DipRect,
    /// Clickable icon region on the left of each pill (e.g. OpenCode click-to-copy).
    pub icon_rect: DipRect,
    /// Optional top-right `×` hide-until-reset chip rect for exhausted pills with a known reset.
    pub hide_rect: Option<DipRect>,
    /// True when the pill includes the rate/reset column (`CompactStacked`) or inline rate (`WideMeters`).
    pub extended: bool,
    /// Active layout style selected for this slot.
    pub style: PillStyle,
    /// Per-meter column rects (1..=2 entries matching `item.taskbar_bars`) in `WideMeters` mode.
    pub meter_rects: Vec<DipRect>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StripMode {
    /// Not enough room on the taskbar — hide strip and rely on tray icon.
    Hidden,
    /// Single ultra-compact summary pill showing peak usage across providers.
    CompactSummary {
        rect: DipRect,
        peak_percent: Option<f64>,
        severity: BarSeverity,
    },
    /// One or more provider slots side by side, plus optional `+N` overflow badge and System CPU/MEM pill.
    Pills {
        slots: Vec<PillSlot>,
        overflow_badge: Option<(DipRect, usize)>,
        system_slot: Option<DipRect>,
    },
    /// Empty / loading placeholder pill so the user immediately sees the strip is alive.
    Empty { rect: DipRect, label: &'static str },
}

#[derive(Clone, Debug, PartialEq)]
pub struct StripLayout {
    pub width_dip: f32,
    pub height_dip: f32,
    pub mode: StripMode,
}

impl StripLayout {
    /// Return the `snapshot.items` index of the provider pill under `(px, py)` (in strip DIPs).
    pub fn hit_test_pill(&self, px: f32, py: f32) -> Option<usize> {
        match &self.mode {
            StripMode::Pills { slots, .. } => slots
                .iter()
                .find(|slot| slot.rect.contains(px, py))
                .map(|slot| slot.item_index),
            StripMode::Hidden | StripMode::CompactSummary { .. } | StripMode::Empty { .. } => None,
        }
    }

    /// Return the `snapshot.items` index of the provider pill whose clickable icon region
    /// contains `(px, py)` (in strip DIPs).
    pub fn hit_test_pill_icon(&self, px: f32, py: f32) -> Option<usize> {
        match &self.mode {
            StripMode::Pills { slots, .. } => slots
                .iter()
                .find(|slot| slot.icon_rect.contains(px, py))
                .map(|slot| slot.item_index),
            StripMode::Hidden | StripMode::CompactSummary { .. } | StripMode::Empty { .. } => None,
        }
    }

    /// Return the `snapshot.items` index of the exhausted provider pill whose top-right
    /// hide-until-reset `×` chip contains `(px, py)` (in strip DIPs).
    pub fn hit_test_pill_hide(&self, px: f32, py: f32) -> Option<usize> {
        match &self.mode {
            StripMode::Pills { slots, .. } => slots
                .iter()
                .find(|slot| slot.hide_rect.is_some_and(|hr| hr.contains(px, py)))
                .map(|slot| slot.item_index),
            StripMode::Hidden | StripMode::CompactSummary { .. } | StripMode::Empty { .. } => None,
        }
    }

    /// Return the strip-relative DIP rectangle for a rendered provider pill by `item_index`.
    pub fn pill_rect(&self, item_index: usize) -> Option<DipRect> {
        match &self.mode {
            StripMode::Pills { slots, .. } => slots
                .iter()
                .find(|slot| slot.item_index == item_index)
                .map(|slot| slot.rect),
            StripMode::Hidden | StripMode::CompactSummary { .. } | StripMode::Empty { .. } => None,
        }
    }

    /// True if `(px, py)` hits the stacked CPU & Memory system monitor slot.
    pub fn hit_test_system(&self, px: f32, py: f32) -> bool {
        match &self.mode {
            StripMode::Pills { system_slot, .. } => system_slot.is_some_and(|r| r.contains(px, py)),
            StripMode::Hidden | StripMode::CompactSummary { .. } | StripMode::Empty { .. } => false,
        }
    }

    /// Return the strip-relative DIP rectangle for the stacked CPU & Memory system monitor slot.
    pub fn system_rect(&self) -> Option<DipRect> {
        match &self.mode {
            StripMode::Pills { system_slot, .. } => *system_slot,
            StripMode::Hidden | StripMode::CompactSummary { .. } | StripMode::Empty { .. } => None,
        }
    }

    /// True if `(px, py)` hits the `+N` overflow badge, compact summary, or empty status pill.
    pub fn hit_test_overflow_or_empty(&self, px: f32, py: f32) -> bool {
        match &self.mode {
            StripMode::Pills { overflow_badge, .. } => {
                overflow_badge.is_some_and(|(r, _)| r.contains(px, py))
            }
            StripMode::CompactSummary { rect, .. } | StripMode::Empty { rect, .. } => {
                rect.contains(px, py)
            }
            StripMode::Hidden => false,
        }
    }
}

/// Compute whether a provider pill should use the extended compact layout
/// (Claude with inline `$/h` rate or compact reset time).
pub fn is_extended_pill(item: &ProviderItem) -> bool {
    item.provider == ProviderKind::Claude
        && (item.spend_rate_text.is_some() || item.reset_short.is_some())
}

/// Width of a provider slot in `PillStyle::CompactStacked`.
pub fn pill_width(item: &ProviderItem) -> f32 {
    if is_extended_pill(item) {
        CLAUDE_EXTENDED_PILL_WIDTH_DIP
    } else {
        STANDARD_PILL_WIDTH_DIP
    }
}

/// Width of a provider slot in `PillStyle::WideMeters`:
/// - 2-bar provider without rate: `6 + 16 + 5 + 65 + 6 + 65 + 6 = 169` DIP
/// - 2-bar provider with rate (Claude): `6 + 28 + 5 + 65 + 6 + 65 + 6 = 181` DIP
/// - 1-bar provider without rate (Grok): `6 + 16 + 5 + 70 + 6 = 103` DIP
pub fn wide_pill_width(item: &ProviderItem) -> f32 {
    let icon_col = if item.spend_rate_text.is_some() {
        WIDE_RATE_ICON_COL_DIP
    } else {
        WIDE_ICON_COL_DIP
    };
    let bars = item.taskbar_bars.len().min(2);
    let meters_w = match bars {
        0 => 44.0,
        1 => WIDE_SINGLE_METER_COL_DIP,
        _ => WIDE_METER_COL_DIP * 2.0 + WIDE_METER_GAP_DIP,
    };
    WIDE_PAD_X_DIP * 2.0 + icon_col + WIDE_ICON_GAP_DIP + meters_w
}

fn pack_wide(primary_items: &[(usize, &ProviderItem)], usable: f32) -> Vec<(usize, f32, bool)> {
    let total = primary_items.len();
    let mut chosen: Vec<(usize, f32, bool)> = Vec::new();
    let mut used_w = 0.0f32;

    for (pos, &(idx, item)) in primary_items.iter().enumerate() {
        let extended = item.spend_rate_text.is_some();
        let w = wide_pill_width(item);
        let gap = if chosen.is_empty() { 0.0 } else { PILL_GAP_DIP };
        let remaining_after = total - (pos + 1);
        let reserve_overflow = if remaining_after > 0 {
            PILL_GAP_DIP + OVERFLOW_BADGE_WIDTH_DIP
        } else {
            0.0
        };

        if used_w + gap + w + reserve_overflow <= usable {
            used_w += gap + w;
            chosen.push((idx, w, extended));
        } else {
            break;
        }
    }

    chosen
}

fn pack_compact(primary_items: &[(usize, &ProviderItem)], usable: f32) -> Vec<(usize, f32, bool)> {
    let total = primary_items.len();
    let mut chosen: Vec<(usize, f32, bool)> = Vec::new();
    let mut used_w = 0.0f32;

    for (pos, &(idx, item)) in primary_items.iter().enumerate() {
        let extended = is_extended_pill(item);
        let w = pill_width(item);
        let gap = if chosen.is_empty() { 0.0 } else { PILL_GAP_DIP };
        let remaining_after = total - (pos + 1);
        let reserve_overflow = if remaining_after > 0 {
            PILL_GAP_DIP + OVERFLOW_BADGE_WIDTH_DIP
        } else {
            0.0
        };

        if used_w + gap + w + reserve_overflow <= usable {
            used_w += gap + w;
            chosen.push((idx, w, extended));
        } else if extended
            && chosen.is_empty()
            && STANDARD_PILL_WIDTH_DIP + reserve_overflow <= usable
        {
            // Fall back to standard width for Claude if space is tight.
            used_w += STANDARD_PILL_WIDTH_DIP;
            chosen.push((idx, STANDARD_PILL_WIDTH_DIP, false));
        } else {
            break;
        }
    }

    chosen
}

/// Compute the taskbar strip layout given the available horizontal gap left of the system tray.
pub fn compute_strip_layout(
    snapshot: &TaskbarSnapshot,
    max_width_dip: f32,
    loading: bool,
) -> StripLayout {
    let pill_y = (STRIP_HEIGHT_DIP - PILL_HEIGHT_DIP) * 0.5;

    if max_width_dip < MIN_VISIBLE_WIDTH_DIP {
        return StripLayout {
            width_dip: 0.0,
            height_dip: STRIP_HEIGHT_DIP,
            mode: StripMode::Hidden,
        };
    }

    // Only primary items appear in the taskbar strip; secondary accounts (e.g. extra
    // OpenCode accounts) are shown exclusively inside the click flyout.
    let primary_items: Vec<(usize, &ProviderItem)> = snapshot
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| !item.secondary)
        .collect();

    let usable = max_width_dip - STRIP_PAD_X_DIP * 2.0;
    let want_system = snapshot.system_monitor_enabled && snapshot.system.is_some();

    if primary_items.is_empty() {
        if want_system && usable >= SYSTEM_PILL_WIDTH_DIP {
            return StripLayout {
                width_dip: SYSTEM_PILL_WIDTH_DIP + STRIP_PAD_X_DIP * 2.0,
                height_dip: STRIP_HEIGHT_DIP,
                mode: StripMode::Pills {
                    slots: Vec::new(),
                    overflow_badge: None,
                    system_slot: Some(DipRect {
                        x: STRIP_PAD_X_DIP,
                        y: pill_y,
                        w: SYSTEM_PILL_WIDTH_DIP,
                        h: PILL_HEIGHT_DIP,
                    }),
                },
            };
        }
        let w = 96.0f32.min(max_width_dip);
        return StripLayout {
            width_dip: w,
            height_dip: STRIP_HEIGHT_DIP,
            mode: StripMode::Empty {
                rect: DipRect {
                    x: STRIP_PAD_X_DIP,
                    y: pill_y,
                    w: (w - STRIP_PAD_X_DIP * 2.0).max(40.0),
                    h: PILL_HEIGHT_DIP,
                },
                label: if loading { "Loading..." } else { "UsageDeck" },
            },
        };
    }

    let include_system =
        want_system && usable >= SYSTEM_PILL_WIDTH_DIP + PILL_GAP_DIP + STANDARD_PILL_WIDTH_DIP;
    let provider_usable = if include_system {
        (usable - SYSTEM_PILL_WIDTH_DIP - PILL_GAP_DIP).max(0.0)
    } else {
        usable
    };
    let total = primary_items.len();

    // Progressive collapse strategy:
    // 1. Prefer fitting all visible providers in `PillStyle::WideMeters`.
    // 2. If not all fit in `WideMeters`, still use `WideMeters` if at least 2 fit (or total == 1).
    // 3. Otherwise switch to `PillStyle::CompactStacked` (with `+N` overflow if needed).
    // 4. If `< 1` pill fits even in compact mode, fall back to `CompactSummary`.
    let wide_chosen = pack_wide(&primary_items, provider_usable);
    let (style, chosen) = if wide_chosen.len() == total || wide_chosen.len() >= 2 {
        (PillStyle::WideMeters, wide_chosen)
    } else {
        (
            PillStyle::CompactStacked,
            pack_compact(&primary_items, provider_usable),
        )
    };

    if chosen.is_empty() {
        // Collapse to a single ultra-compact summary pill.
        let mut peak: Option<f64> = None;
        let mut worst = BarSeverity::Muted;
        for &(_, item) in &primary_items {
            for bar in &item.taskbar_bars {
                if let Some(p) = bar.percent {
                    peak = Some(peak.map_or(p, |cur| cur.max(p)));
                }
                worst = worse_severity(worst, bar.severity);
            }
        }
        let w = COMPACT_SUMMARY_WIDTH_DIP.min(usable);
        return StripLayout {
            width_dip: w + STRIP_PAD_X_DIP * 2.0,
            height_dip: STRIP_HEIGHT_DIP,
            mode: StripMode::CompactSummary {
                rect: DipRect {
                    x: STRIP_PAD_X_DIP,
                    y: pill_y,
                    w,
                    h: PILL_HEIGHT_DIP,
                },
                peak_percent: peak,
                severity: worst,
            },
        };
    }

    let mut x = STRIP_PAD_X_DIP;
    let mut slots = Vec::with_capacity(chosen.len());
    for (idx, w, extended) in chosen {
        let rect = DipRect {
            x,
            y: pill_y,
            w,
            h: PILL_HEIGHT_DIP,
        };
        let item = &snapshot.items[idx];
        let (icon_rect, meter_rects) = match style {
            PillStyle::WideMeters => {
                let icon_col_w = if item.spend_rate_text.is_some() {
                    WIDE_RATE_ICON_COL_DIP
                } else {
                    WIDE_ICON_COL_DIP
                };
                let icon_rect = DipRect {
                    x: rect.x + WIDE_PAD_X_DIP - 2.0,
                    y: rect.y + 2.0,
                    w: icon_col_w + 4.0,
                    h: rect.h - 4.0,
                };
                let bar_count = item.taskbar_bars.len().min(2);
                let meter_w = if bar_count <= 1 {
                    WIDE_SINGLE_METER_COL_DIP
                } else {
                    WIDE_METER_COL_DIP
                };
                let meters_start_x = rect.x + WIDE_PAD_X_DIP + icon_col_w + WIDE_ICON_GAP_DIP;
                let mut meters = Vec::with_capacity(bar_count);
                for i in 0..bar_count {
                    meters.push(DipRect {
                        x: meters_start_x + i as f32 * (meter_w + WIDE_METER_GAP_DIP),
                        y: rect.y,
                        w: meter_w,
                        h: rect.h,
                    });
                }
                (icon_rect, meters)
            }
            PillStyle::CompactStacked => {
                let icon_rect = DipRect {
                    x: rect.x + 4.0,
                    y: rect.y + 2.0,
                    w: 22.0,
                    h: rect.h - 4.0,
                };
                (icon_rect, Vec::new())
            }
        };
        let hide_rect = exhausted_until_ms(item, snapshot.updated_at_ms).map(|_| DipRect {
            x: rect.x + rect.w - 17.0,
            y: rect.y + 1.0,
            w: 16.0,
            h: 16.0,
        });
        slots.push(PillSlot {
            item_index: idx,
            rect,
            icon_rect,
            hide_rect,
            extended,
            style,
            meter_rects,
        });
        x += w + PILL_GAP_DIP;
    }

    let hidden_count = total.saturating_sub(slots.len());
    let overflow_badge = if hidden_count > 0 {
        let rect = DipRect {
            x,
            y: pill_y,
            w: OVERFLOW_BADGE_WIDTH_DIP,
            h: PILL_HEIGHT_DIP,
        };
        x += OVERFLOW_BADGE_WIDTH_DIP + PILL_GAP_DIP;
        Some((rect, hidden_count))
    } else {
        None
    };

    let system_slot = if include_system {
        let rect = DipRect {
            x,
            y: pill_y,
            w: SYSTEM_PILL_WIDTH_DIP,
            h: PILL_HEIGHT_DIP,
        };
        x += SYSTEM_PILL_WIDTH_DIP + PILL_GAP_DIP;
        Some(rect)
    } else {
        None
    };

    let width_dip = (x - PILL_GAP_DIP + STRIP_PAD_X_DIP).max(0.0);

    StripLayout {
        width_dip,
        height_dip: STRIP_HEIGHT_DIP,
        mode: StripMode::Pills {
            slots,
            overflow_badge,
            system_slot,
        },
    }
}

fn worse_severity(a: BarSeverity, b: BarSeverity) -> BarSeverity {
    let rank = |s: BarSeverity| match s {
        BarSeverity::Muted => 0,
        BarSeverity::Ok => 1,
        BarSeverity::Warn => 2,
        BarSeverity::Danger => 3,
    };
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

// ---- Flyout layout -------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlyoutAction {
    Refresh,
    OpenSettings,
    OpenDataFolder,
    OpenCursor,
    CopyOpenCodeKey(String),
    HideUntilReset {
        provider: ProviderKind,
        account_id: String,
        until_ms: u64,
    },
    Quit,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct FlyoutButton {
    pub action: FlyoutAction,
    pub rect: DipRect,
    pub label: &'static str,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct FlyoutCardSlot {
    pub item_index: usize,
    pub rect: DipRect,
    /// Clickable icon region at the top-left of the card.
    pub icon_rect: DipRect,
    /// Optional click action bound to the card's icon (e.g. copy OpenCode API key).
    pub icon_action: Option<FlyoutAction>,
    /// Right-aligned quick-action chips in the card header (Open Cursor, Copy key, Hide).
    pub buttons: Vec<FlyoutButton>,
    /// Left edge (in flyout DIPs) of the leftmost chip on the right side of the card header.
    pub right_content_x: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct FlyoutLayout {
    pub width_dip: f32,
    pub height_dip: f32,
    pub cards: Vec<FlyoutCardSlot>,
    pub header_buttons: Vec<FlyoutButton>,
    /// Per-bar hover hit regions `(rect, item_index, bar_index)` for per-model tooltips.
    pub bar_rects: Vec<(DipRect, usize, usize)>,
}

#[allow(dead_code)]
impl FlyoutLayout {
    pub fn hit_test(&self, px: f32, py: f32) -> Option<FlyoutAction> {
        for btn in &self.header_buttons {
            if btn.rect.contains(px, py) {
                return Some(btn.action.clone());
            }
        }
        for card in &self.cards {
            for btn in &card.buttons {
                if btn.rect.contains(px, py) {
                    return Some(btn.action.clone());
                }
            }
            if let Some(action) = card
                .icon_action
                .as_ref()
                .filter(|_| card.icon_rect.contains(px, py))
            {
                return Some(action.clone());
            }
        }
        None
    }

    /// Return `(item_index, bar_index)` for the flyout bar column containing `(px, py)` (in flyout DIPs).
    pub fn hit_test_bar(&self, px: f32, py: f32) -> Option<(usize, usize)> {
        self.bar_rects
            .iter()
            .find(|(rect, _, _)| rect.contains(px, py))
            .map(|&(_, item_idx, bar_idx)| (item_idx, bar_idx))
    }

    /// Return the flyout-relative DIP rectangle for a rendered bar column `(item_index, bar_index)`.
    #[allow(dead_code)]
    pub fn bar_rect(&self, item_index: usize, bar_index: usize) -> Option<DipRect> {
        self.bar_rects
            .iter()
            .find(|&&(_, i, b)| i == item_index && b == bar_index)
            .map(|&(rect, _, _)| rect)
    }
}

/// Latest future reset timestamp (epoch ms) among an exhausted card's full (`Danger`) bars,
/// falling back to the earliest future reset among any visible bar if no `Danger` bar carries a
/// reset timestamp (e.g. Codex `limit_reached`). Returns `None` when the card is not exhausted
/// or has no future reset time.
pub fn exhausted_until_ms(item: &ProviderItem, now_ms: i64) -> Option<u64> {
    if !item.exhausted {
        return None;
    }
    if let Some(ms) = item.exhausted_until_ms {
        if i64::try_from(ms).is_ok_and(|t| t > now_ms) {
            return Some(ms);
        }
    }
    let danger_max = item
        .flyout_bars
        .iter()
        .filter(|b| b.severity == BarSeverity::Danger)
        .filter_map(|b| b.resets_at.as_deref().and_then(parse_iso_ms))
        .filter(|&ms| ms > now_ms)
        .max();
    let until = danger_max.or_else(|| {
        item.flyout_bars
            .iter()
            .filter_map(|b| b.resets_at.as_deref().and_then(parse_iso_ms))
            .filter(|&ms| ms > now_ms)
            .min()
    })?;
    u64::try_from(until).ok()
}

/// Compute the anchored flyout panel layout and interactive hit rects.
#[allow(dead_code)]
pub fn compute_flyout_layout(snapshot: &TaskbarSnapshot) -> FlyoutLayout {
    let w = FLYOUT_WIDTH_DIP;
    let inner_w = w - FLYOUT_PAD_DIP * 2.0;

    // Header action buttons right-aligned inside the top header row.
    let btn_h = 22.0;
    let btn_gap = 4.0;
    let btn_y = FLYOUT_PAD_DIP + (FLYOUT_HEADER_HEIGHT_DIP - btn_h) * 0.5;
    let quit_rect = DipRect {
        x: w - FLYOUT_PAD_DIP - 36.0,
        y: btn_y,
        w: 36.0,
        h: btn_h,
    };
    let folder_rect = DipRect {
        x: quit_rect.x - btn_gap - 38.0,
        y: btn_y,
        w: 38.0,
        h: btn_h,
    };
    let settings_rect = DipRect {
        x: folder_rect.x - btn_gap - 54.0,
        y: btn_y,
        w: 54.0,
        h: btn_h,
    };
    let refresh_rect = DipRect {
        x: settings_rect.x - btn_gap - 48.0,
        y: btn_y,
        w: 48.0,
        h: btn_h,
    };

    let header_buttons = vec![
        FlyoutButton {
            action: FlyoutAction::Refresh,
            rect: refresh_rect,
            label: "Refresh",
        },
        FlyoutButton {
            action: FlyoutAction::OpenSettings,
            rect: settings_rect,
            label: "Settings",
        },
        FlyoutButton {
            action: FlyoutAction::OpenDataFolder,
            rect: folder_rect,
            label: "Data",
        },
        FlyoutButton {
            action: FlyoutAction::Quit,
            rect: quit_rect,
            label: "Quit",
        },
    ];

    let mut y = FLYOUT_PAD_DIP + FLYOUT_HEADER_HEIGHT_DIP + 8.0;
    let mut cards = Vec::with_capacity(snapshot.items.len());
    let mut bar_rects = Vec::new();

    if snapshot.items.is_empty() {
        // Space for the empty-state message box.
        y += 64.0;
    } else {
        for (idx, item) in snapshot.items.iter().enumerate() {
            // Card height: top header row (20) + gap (6) + bar grid (1 row for 1..=3 bars = 26)
            // + vertical padding (20).
            let card_h = FLYOUT_CARD_PAD_DIP * 2.0 + 20.0 + 6.0 + 26.0;
            let rect = DipRect {
                x: FLYOUT_PAD_DIP,
                y,
                w: inner_w,
                h: card_h,
            };

            let icon_rect = DipRect {
                x: rect.x + FLYOUT_CARD_PAD_DIP - 2.0,
                y: rect.y + FLYOUT_CARD_PAD_DIP - 1.0,
                w: 20.0,
                h: 20.0,
            };
            let icon_action =
                if item.provider == ProviderKind::OpenCode && !item.account_id.is_empty() {
                    Some(FlyoutAction::CopyOpenCodeKey(item.account_id.clone()))
                } else {
                    None
                };

            let chip_h = 20.0;
            let chip_y = rect.y + FLYOUT_CARD_PAD_DIP - 1.0;
            let chip_gap = 5.0;
            let mut cursor_x = rect.x + rect.w - FLYOUT_CARD_PAD_DIP;
            let mut buttons = Vec::new();

            let mut push_chip = |chip_w: f32, label: &'static str, action: FlyoutAction| {
                let x = cursor_x - chip_w;
                buttons.push(FlyoutButton {
                    action,
                    rect: DipRect {
                        x,
                        y: chip_y,
                        w: chip_w,
                        h: chip_h,
                    },
                    label,
                });
                cursor_x = x - chip_gap;
            };

            if item.can_open_cursor {
                push_chip(78.0, "Open Cursor", FlyoutAction::OpenCursor);
            }
            if let Some(until_ms) = exhausted_until_ms(item, snapshot.updated_at_ms) {
                push_chip(
                    38.0,
                    "Hide",
                    FlyoutAction::HideUntilReset {
                        provider: item.provider,
                        account_id: item.account_id.clone(),
                        until_ms,
                    },
                );
            }
            if item.provider == ProviderKind::OpenCode && !item.account_id.is_empty() {
                push_chip(
                    56.0,
                    "Copy key",
                    FlyoutAction::CopyOpenCodeKey(item.account_id.clone()),
                );
            }

            buttons.reverse();
            let right_content_x = buttons
                .first()
                .map_or(rect.x + rect.w - FLYOUT_CARD_PAD_DIP, |b| b.rect.x);

            // Mirror render_flyout's bar column grid for per-model hover tooltips.
            let bar_count = item.flyout_bars.len().max(1);
            let grid_x = rect.x + FLYOUT_CARD_PAD_DIP;
            let grid_y = rect.y + FLYOUT_CARD_PAD_DIP + 24.0;
            let grid_w = rect.w - FLYOUT_CARD_PAD_DIP * 2.0;
            let col_gap = 8.0;
            let col_w = (grid_w - col_gap * (bar_count as f32 - 1.0)) / bar_count as f32;
            for bar_idx in 0..item.flyout_bars.len() {
                let cx = grid_x + bar_idx as f32 * (col_w + col_gap);
                bar_rects.push((
                    DipRect {
                        x: cx,
                        y: grid_y,
                        w: col_w,
                        h: 24.0,
                    },
                    idx,
                    bar_idx,
                ));
            }

            cards.push(FlyoutCardSlot {
                item_index: idx,
                rect,
                icon_rect,
                icon_action,
                buttons,
                right_content_x,
            });
            y += card_h + FLYOUT_CARD_GAP_DIP;
        }
        y -= FLYOUT_CARD_GAP_DIP;
    }

    FlyoutLayout {
        width_dip: w,
        height_dip: y + FLYOUT_PAD_DIP,
        cards,
        header_buttons,
        bar_rects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use usagedeck_core::view::{demo_snapshot, BarView};

    #[test]
    fn packs_all_four_demo_providers_in_wide_meters_when_space_allows() {
        let mut snap = demo_snapshot(1_700_000_000_000);
        snap.system = None;
        // Claude (181) + 3 * 169 + 3 * 7 gap + 6 pad = 715 DIP
        let layout = compute_strip_layout(&snap, 740.0, false);
        match layout.mode {
            StripMode::Pills {
                slots,
                overflow_badge,
                system_slot,
            } => {
                assert_eq!(slots.len(), 4);
                assert!(overflow_badge.is_none());
                assert!(system_slot.is_none());
                assert!(slots.iter().all(|s| s.style == PillStyle::WideMeters));
                assert!(slots[0].extended);
                assert_eq!(slots[0].rect.w, 181.0);
                assert!(!slots[1].extended);
                assert_eq!(slots[1].rect.w, 169.0);
                for slot in &slots {
                    assert_eq!(slot.meter_rects.len(), 2);
                    assert_eq!(slot.meter_rects[0].w, WIDE_METER_COL_DIP);
                    assert_eq!(slot.meter_rects[1].w, WIDE_METER_COL_DIP);
                    assert!(slot.meter_rects[0].x + slot.meter_rects[0].w < slot.meter_rects[1].x);
                }
            }
            other => panic!("expected Pills, got {other:?}"),
        }
    }

    #[test]
    fn packs_system_pill_alongside_providers_and_alone() {
        let mut snap = demo_snapshot(1_700_000_000_000);
        let layout = compute_strip_layout(&snap, 840.0, false);
        match layout.mode {
            StripMode::Pills {
                ref slots,
                system_slot,
                ..
            } => {
                assert!(!slots.is_empty());
                let sys = system_slot.expect("system_slot should be placed");
                assert_eq!(sys.w, SYSTEM_PILL_WIDTH_DIP);
                assert!(layout.hit_test_system(sys.x + 10.0, sys.y + 10.0));
                assert_eq!(layout.system_rect(), Some(sys));
                assert!(layout.width_dip <= 840.0);
            }
            ref other => panic!("expected Pills, got {other:?}"),
        }

        // Standalone System pill when all AI providers are hidden/empty.
        snap.items.clear();
        let solo = compute_strip_layout(&snap, 300.0, false);
        match solo.mode {
            StripMode::Pills {
                slots,
                overflow_badge,
                system_slot,
            } => {
                assert!(slots.is_empty());
                assert!(overflow_badge.is_none());
                assert_eq!(system_slot.map(|r| r.w), Some(SYSTEM_PILL_WIDTH_DIP));
            }
            other => panic!("expected standalone System pill, got {other:?}"),
        }
    }

    #[test]
    fn single_bar_provider_uses_103_dip_wide_meter_slot() {
        let now = 1_700_000_000_000i64;
        let mut snap = demo_snapshot(now);
        snap.system = None;
        let grok = ProviderItem {
            provider: ProviderKind::Grok,
            account_id: "grok".into(),
            title: "Grok Bot".into(),
            taskbar_bars: vec![BarView::new("main", "5h", Some(32.0), None, now, false)],
            flyout_bars: vec![],
            hover_bars: vec![],
            hover_details: vec![],
            resets_badge: None,
            exhausted_until_ms: None,
            spend_rate_text: None,
            spend_rate_severity: BarSeverity::Muted,
            secondary: false,
            display_email: None,
            reset_short: None,
            subtitle: None,
            auth_expired: false,
            stale: false,
            exhausted: false,
            can_open_cursor: false,
            antigravity_setup_issue: None,
            tooltip: String::new(),
        };
        snap.items = vec![grok];
        let layout = compute_strip_layout(&snap, 400.0, false);
        match layout.mode {
            StripMode::Pills {
                slots,
                overflow_badge,
                ..
            } => {
                assert_eq!(slots.len(), 1);
                assert!(overflow_badge.is_none());
                assert_eq!(slots[0].style, PillStyle::WideMeters);
                assert_eq!(slots[0].rect.w, 103.0);
                assert_eq!(slots[0].meter_rects.len(), 1);
                assert_eq!(slots[0].meter_rects[0].w, WIDE_SINGLE_METER_COL_DIP);
            }
            other => panic!("expected Pills, got {other:?}"),
        }
    }

    #[test]
    fn keeps_wide_meters_with_overflow_when_at_least_two_fit() {
        let mut snap = demo_snapshot(1_700_000_000_000);
        snap.system = None;
        // At 680 DIP cap, 3 demo providers (181 + 169 + 169 + 3*7 + 28 + 6 = 574 DIP) fit in WideMeters + `+1`,
        // while all 4 (715 DIP) do not.
        let layout = compute_strip_layout(&snap, 680.0, false);
        match layout.mode {
            StripMode::Pills {
                slots,
                overflow_badge,
                ..
            } => {
                assert_eq!(slots.len(), 3);
                assert_eq!(overflow_badge.map(|(_, n)| n), Some(1));
                assert!(slots.iter().all(|s| s.style == PillStyle::WideMeters));
                assert!(layout.width_dip <= 680.0);
            }
            other => panic!("expected WideMeters Pills with overflow, got {other:?}"),
        }
    }

    #[test]
    fn falls_back_to_compact_stacked_when_fewer_than_two_wide_fit() {
        let mut snap = demo_snapshot(1_700_000_000_000);
        snap.system = None;
        // At 260 DIP, only 1 wide pill would fit (< 2), so we collapse to CompactStacked:
        // Claude (124) + 7 + Cursor (86) + 7 + badge (28) + 6 pad = 258 DIP <= 260.
        let layout = compute_strip_layout(&snap, 260.0, false);
        match layout.mode {
            StripMode::Pills {
                slots,
                overflow_badge,
                ..
            } => {
                assert_eq!(slots.len(), 2);
                assert_eq!(overflow_badge.map(|(_, n)| n), Some(2));
                assert!(slots.iter().all(|s| s.style == PillStyle::CompactStacked));
                assert!(slots.iter().all(|s| s.meter_rects.is_empty()));
                assert!(layout.width_dip <= 260.0);
            }
            other => panic!("expected CompactStacked Pills with overflow, got {other:?}"),
        }
    }

    #[test]
    fn collapses_to_compact_summary_when_very_tight() {
        let snap = demo_snapshot(1_700_000_000_000);
        let layout = compute_strip_layout(&snap, 80.0, false);
        match layout.mode {
            StripMode::CompactSummary {
                peak_percent,
                severity,
                ..
            } => {
                assert_eq!(peak_percent, Some(100.0));
                assert_eq!(severity, BarSeverity::Danger);
            }
            other => panic!("expected CompactSummary, got {other:?}"),
        }
    }

    #[test]
    fn hides_completely_when_below_minimum_width() {
        let snap = demo_snapshot(1_700_000_000_000);
        let layout = compute_strip_layout(&snap, 40.0, false);
        assert_eq!(layout.mode, StripMode::Hidden);
        assert_eq!(layout.hit_test_pill(10.0, 10.0), None);
    }

    #[test]
    fn strip_hit_tests_individual_pills_and_gaps() {
        let snap = demo_snapshot(1_700_000_000_000);
        let layout = compute_strip_layout(&snap, 1040.0, false);
        for idx in 0..4 {
            let rect = layout.pill_rect(idx).expect("pill rect exists");
            assert_eq!(
                layout.hit_test_pill(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5),
                Some(idx)
            );
        }
        let sys_rect = layout.system_rect().expect("system rect exists");
        assert!(
            layout.hit_test_system(sys_rect.x + sys_rect.w * 0.5, sys_rect.y + sys_rect.h * 0.5)
        );
        assert_eq!(
            layout.hit_test_pill(sys_rect.x + sys_rect.w * 0.5, sys_rect.y + sys_rect.h * 0.5),
            None
        );
        // Gap between pill 0 and pill 1 should return None.
        let p0 = layout.pill_rect(0).unwrap();
        let p1 = layout.pill_rect(1).unwrap();
        let gap_x = (p0.x + p0.w + p1.x) * 0.5;
        assert_eq!(layout.hit_test_pill(gap_x, p0.y + 10.0), None);
        // Outside strip height returns None.
        assert_eq!(
            layout.hit_test_pill(p0.x + 5.0, STRIP_HEIGHT_DIP + 4.0),
            None
        );
    }

    #[test]
    fn flyout_hit_tests_header_buttons() {
        let snap = demo_snapshot(1_700_000_000_000);
        let fl = compute_flyout_layout(&snap);
        assert_eq!(fl.cards.len(), 4);

        let expected = [
            FlyoutAction::Refresh,
            FlyoutAction::OpenSettings,
            FlyoutAction::OpenDataFolder,
            FlyoutAction::Quit,
        ];
        assert_eq!(fl.header_buttons.len(), expected.len());
        for (btn, want) in fl.header_buttons.iter().zip(expected.iter()) {
            assert_eq!(
                fl.hit_test(btn.rect.x + btn.rect.w * 0.5, btn.rect.y + btn.rect.h * 0.5),
                Some(want.clone())
            );
        }
        // Header buttons must not overlap one another.
        for pair in fl.header_buttons.windows(2) {
            assert!(pair[0].rect.x + pair[0].rect.w < pair[1].rect.x);
        }
    }

    #[test]
    fn flyout_hit_tests_opencode_copy_key_and_exhausted_hide_chips() {
        let now = 1_700_000_000_000i64;
        let snap = demo_snapshot(now);
        let fl = compute_flyout_layout(&snap);

        // Claude (0) and Cursor (1) are neither exhausted nor OpenCode -> no card buttons.
        assert!(fl.cards[0].buttons.is_empty());
        assert!(fl.cards[1].buttons.is_empty());

        // Codex (2) is exhausted in demo_snapshot -> has HideUntilReset chip.
        assert_eq!(fl.cards[2].buttons.len(), 1);
        let hide_btn = &fl.cards[2].buttons[0];
        assert_eq!(hide_btn.label, "Hide");
        assert_eq!(
            fl.hit_test(hide_btn.rect.x + 5.0, hide_btn.rect.y + 5.0),
            Some(FlyoutAction::HideUntilReset {
                provider: ProviderKind::Codex,
                account_id: "demo@openai.com".into(),
                until_ms: (now + 220 * 60_000) as u64,
            })
        );

        // OpenCode (3) has CopyOpenCodeKey chip.
        assert_eq!(fl.cards[3].buttons.len(), 1);
        let copy_btn = &fl.cards[3].buttons[0];
        assert_eq!(copy_btn.label, "Copy key");
        assert_eq!(
            fl.hit_test(copy_btn.rect.x + 5.0, copy_btn.rect.y + 5.0),
            Some(FlyoutAction::CopyOpenCodeKey("demo@opencode.ai".into()))
        );
    }

    #[test]
    fn flyout_stacks_multiple_card_chips_without_overlap() {
        let now = 1_700_000_000_000i64;
        let mut snap = demo_snapshot(now);
        // Make Cursor require sign-in and OpenCode exhausted so both chips appear on OpenCode.
        snap.items[1].can_open_cursor = true;
        snap.items[3].exhausted = true;
        snap.items[3].flyout_bars[1].severity = BarSeverity::Danger;

        let fl = compute_flyout_layout(&snap);

        // Cursor card has OpenCursor button.
        let cursor_btn = &fl.cards[1].buttons[0];
        assert_eq!(
            fl.hit_test(cursor_btn.rect.x + 4.0, cursor_btn.rect.y + 4.0),
            Some(FlyoutAction::OpenCursor)
        );

        // Exhausted OpenCode card has both Copy key and Hide chips side by side with no overlap.
        assert_eq!(fl.cards[3].buttons.len(), 2);
        let b0 = &fl.cards[3].buttons[0];
        let b1 = &fl.cards[3].buttons[1];
        assert!(b0.rect.x + b0.rect.w < b1.rect.x);
        assert_eq!(
            fl.hit_test(b0.rect.x + 4.0, b0.rect.y + 4.0),
            Some(FlyoutAction::CopyOpenCodeKey("demo@opencode.ai".into()))
        );
        assert_eq!(
            fl.hit_test(b1.rect.x + 4.0, b1.rect.y + 4.0),
            Some(FlyoutAction::HideUntilReset {
                provider: ProviderKind::OpenCode,
                account_id: "demo@opencode.ai".into(),
                until_ms: (now + 3 * 24 * 60 * 60_000) as u64,
            })
        );
    }

    #[test]
    fn exhausted_until_ms_picks_latest_danger_reset() {
        let now = 1_700_000_000_000i64;
        let iso = |mins: i64| {
            DateTime::<Utc>::from_timestamp_millis(now + mins * 60_000).map(|dt| dt.to_rfc3339())
        };
        let item = ProviderItem {
            provider: ProviderKind::Claude,
            account_id: "a@b.com".into(),
            title: "Claude".into(),
            taskbar_bars: vec![],
            flyout_bars: vec![
                BarView::new("fiveHour", "5h", Some(100.0), iso(90), now, false),
                BarView::new("weekly", "Weekly", Some(100.0), iso(24 * 60), now, false),
            ],
            hover_bars: vec![],
            hover_details: vec![],
            resets_badge: None,
            exhausted_until_ms: None,
            spend_rate_text: None,
            spend_rate_severity: BarSeverity::Muted,
            reset_short: None,
            subtitle: None,
            auth_expired: false,
            stale: false,
            exhausted: true,
            can_open_cursor: false,
            antigravity_setup_issue: None,
            secondary: false,
            display_email: None,
            tooltip: String::new(),
        };
        assert_eq!(
            exhausted_until_ms(&item, now),
            Some((now + 24 * 60 * 60_000) as u64)
        );
    }

    #[test]
    fn strip_hit_tests_pill_icon_regions() {
        let snap = demo_snapshot(1_700_000_000_000);
        for width in [920.0, 440.0] {
            let layout = compute_strip_layout(&snap, width, false);
            let StripMode::Pills { ref slots, .. } = layout.mode else {
                panic!("expected Pills mode at width {width}");
            };
            for slot in slots {
                // Center of icon_rect hits that pill's icon.
                let ix = slot.icon_rect.x + slot.icon_rect.w * 0.5;
                let iy = slot.icon_rect.y + slot.icon_rect.h * 0.5;
                assert_eq!(layout.hit_test_pill_icon(ix, iy), Some(slot.item_index));
                // Right-hand bar area of the pill hits the pill, NOT the icon.
                let bar_x = slot.rect.x + slot.rect.w - 10.0;
                assert_eq!(layout.hit_test_pill(bar_x, iy), Some(slot.item_index));
                assert_eq!(layout.hit_test_pill_icon(bar_x, iy), None);
            }
        }
    }

    #[test]
    fn strip_filters_secondary_items_while_flyout_keeps_all() {
        let now = 1_700_000_000_000i64;
        let mut snap = demo_snapshot(now);
        snap.system = None;
        let mut extra = snap.items[3].clone();
        extra.account_id = "second@opencode.ai".into();
        extra.secondary = true;
        extra.display_email = Some("second@opencode.ai".into());
        snap.items.push(extra);

        let strip = compute_strip_layout(&snap, 920.0, false);
        match strip.mode {
            StripMode::Pills {
                slots,
                overflow_badge,
                ..
            } => {
                // Only the 4 primary items appear on the taskbar strip; no +1 overflow.
                assert_eq!(slots.len(), 4);
                assert!(overflow_badge.is_none());
                assert!(slots.iter().all(|s| !snap.items[s.item_index].secondary));
            }
            other => panic!("expected Pills, got {other:?}"),
        }

        // Flyout renders ALL 5 items including the secondary OpenCode account.
        let fl = compute_flyout_layout(&snap);
        assert_eq!(fl.cards.len(), 5);
        assert_eq!(fl.cards[4].item_index, 4);
    }

    #[test]
    fn flyout_hit_tests_opencode_icon_and_bars() {
        let now = 1_700_000_000_000i64;
        let snap = demo_snapshot(now);
        let fl = compute_flyout_layout(&snap);

        // Clicking OpenCode's card icon returns CopyOpenCodeKey just like its Copy key chip.
        let oc_icon = fl.cards[3].icon_rect;
        assert_eq!(
            fl.hit_test(oc_icon.x + oc_icon.w * 0.5, oc_icon.y + oc_icon.h * 0.5),
            Some(FlyoutAction::CopyOpenCodeKey("demo@opencode.ai".into()))
        );
        // Non-OpenCode card icons do not trigger an action.
        let claude_icon = fl.cards[0].icon_rect;
        assert_eq!(
            fl.hit_test(
                claude_icon.x + claude_icon.w * 0.5,
                claude_icon.y + claude_icon.h * 0.5
            ),
            None
        );

        // Every flyout bar has a non-overlapping bar_rect that hit_test_bar resolves.
        let expected_bars: usize = snap.items.iter().map(|i| i.flyout_bars.len()).sum();
        assert_eq!(fl.bar_rects.len(), expected_bars);
        for &(rect, item_idx, bar_idx) in &fl.bar_rects {
            assert_eq!(
                fl.hit_test_bar(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5),
                Some((item_idx, bar_idx))
            );
            assert_eq!(fl.bar_rect(item_idx, bar_idx), Some(rect));
        }
        // Header area is not a bar.
        assert_eq!(
            fl.hit_test_bar(FLYOUT_PAD_DIP + 10.0, FLYOUT_PAD_DIP + 5.0),
            None
        );
    }

    #[test]
    fn strip_hit_tests_exhausted_pill_hide_chip() {
        let now = 1_700_000_000_000i64;
        let snap = demo_snapshot(now);
        let layout = compute_strip_layout(&snap, 1040.0, false);
        let StripMode::Pills { ref slots, .. } = layout.mode else {
            panic!("expected Pills mode");
        };
        // Non-exhausted pills (Claude 0, Cursor 1, OpenCode 3) have no hide_rect.
        for idx in [0usize, 1, 3] {
            let slot = &slots[idx];
            assert_eq!(slot.hide_rect, None);
            assert_eq!(
                layout.hit_test_pill_hide(slot.rect.x + slot.rect.w - 8.0, slot.rect.y + 8.0),
                None
            );
        }
        // Exhausted pill (Codex 2) exposes a 16x16 top-right hide chip and hit-tests cleanly.
        let codex = &slots[2];
        let hr = codex.hide_rect.expect("exhausted pill has hide_rect");
        assert_eq!(hr.w, 16.0);
        assert_eq!(hr.h, 16.0);
        assert_eq!(
            layout.hit_test_pill_hide(hr.x + hr.w * 0.5, hr.y + hr.h * 0.5),
            Some(2)
        );
        // Center of the pill outside the top-right chip does not hit the hide chip.
        assert_eq!(
            layout.hit_test_pill_hide(codex.rect.x + 24.0, codex.rect.y + codex.rect.h * 0.5),
            None
        );
    }
}
