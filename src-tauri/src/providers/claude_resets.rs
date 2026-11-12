//! Claude's limit resets, as the `oauth/usage` reply lists them when asked. Two programs:
//!
//! - `cedar_ember` (asked for with `?cedar_ember=1`): banked grants, each holding `resets_left`
//!   resets that clear the limits its `clears` names ("Full reset", "5-hour reset", ...), with an
//!   optional `ends_at` expiry and a `paused` flag. Only `eligible: true` lists them.
//! - `juniper_tide` (reported only by the read Claude Code makes at a limit, `?at_wall=1`): the
//!   once-a-week free reset of the 5-hour limit, offered only to accounts in its `reset` arm.
//!
//! The names are Anthropic's obfuscated feature keys, read the way Claude Code and other usage
//! trackers read them. Nothing here redeems a reset. Everything is read leniently: an odd grant
//! is skipped, never a failed reply.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Which limits a reset clears. Ordered as the tooltip lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaudeResetKind {
    Full,
    FiveHour,
    Weekly,
    Other,
}

impl ClaudeResetKind {
    fn default_label(self) -> &'static str {
        match self {
            Self::Full => "Full reset",
            Self::FiveHour => "5-hour reset",
            Self::Weekly => "Weekly reset",
            Self::Other => "Reset",
        }
    }

    /// From the limit names in `clears`: the 5-hour and a weekly limit is a full reset.
    fn of_clears(clears: &[&str]) -> Option<Self> {
        let session = clears.contains(&"five_hour");
        let weekly = clears.iter().any(|c| c.starts_with("seven_day"));
        match (session, weekly) {
            (true, true) => Some(Self::Full),
            (true, false) => Some(Self::FiveHour),
            (false, true) => Some(Self::Weekly),
            (false, false) => None,
        }
    }

    fn of_label(label: &str) -> Self {
        let l = label.to_lowercase();
        if l.contains("full") {
            Self::Full
        } else if ["5-hour", "5 hour", "five-hour", "five hour", "session"]
            .iter()
            .any(|w| l.contains(w))
        {
            Self::FiveHour
        } else if l.contains("week") {
            Self::Weekly
        } else {
            Self::Other
        }
    }
}

/// The banked resets of one kind and name. Paused grants are their own group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeResetGroup {
    pub kind: ClaudeResetKind,
    /// Claude's name for the grant, else one from its kind, e.g. "Full reset".
    pub label: String,
    /// Sum of `resets_left` (1 for a grant that does not say).
    pub count: i64,
    /// The soonest `ends_at` of the group's grants, or None when none expires.
    pub earliest_expiry: Option<String>,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeResets {
    pub groups: Vec<ClaudeResetGroup>,
    /// The weekly free 5-hour reset: true available, false used this week, None not reported.
    pub weekly_five_hour_available: Option<bool>,
}

impl ClaudeResets {
    /// Resets in hand: unpaused banked resets, plus the weekly 5-hour reset while it is available.
    pub fn total(&self) -> i64 {
        let banked: i64 = self
            .groups
            .iter()
            .filter(|g| !g.paused)
            .map(|g| g.count)
            .sum();
        banked + i64::from(self.weekly_five_hour_available == Some(true))
    }
}

fn iso(d: DateTime<Utc>) -> String {
    d.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn time_of(v: Option<&Value>) -> Option<DateTime<Utc>> {
    let t = v?.as_str()?;
    DateTime::parse_from_rfc3339(t.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn clears_of(grant: &Map<String, Value>) -> Vec<&str> {
    grant
        .get("clears")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).collect())
}

fn label_of(grant: &Map<String, Value>) -> Option<&str> {
    grant
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `resets_left`, or 1 when the grant does not say.
fn resets_left_of(grant: &Map<String, Value>) -> Option<i64> {
    match grant.get("resets_left") {
        None | Some(Value::Null) => Some(1),
        Some(v) => v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)),
    }
}

/// The banked grants of an eligible `cedar_ember` block, grouped by kind, name and paused.
/// Spent, expired and not-yet-started grants are left out.
fn groups_of(cedar: Option<&Value>, now: DateTime<Utc>) -> Vec<ClaudeResetGroup> {
    let Some(cedar) = cedar.and_then(Value::as_object) else {
        return Vec::new();
    };
    if cedar.get("eligible").and_then(Value::as_bool) != Some(true) {
        return Vec::new();
    }
    let grants = cedar
        .get("grants")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut groups: BTreeMap<(ClaudeResetKind, String, bool), ClaudeResetGroup> = BTreeMap::new();
    let mut expiries: BTreeMap<(ClaudeResetKind, String, bool), DateTime<Utc>> = BTreeMap::new();
    for grant in grants.iter().filter_map(Value::as_object) {
        let Some(count) = resets_left_of(grant).filter(|n| *n > 0) else {
            continue;
        };
        let ends_at = time_of(grant.get("ends_at"));
        if ends_at.is_some_and(|e| e <= now)
            || time_of(grant.get("starts_at")).is_some_and(|s| s > now)
        {
            continue;
        }
        let label = label_of(grant);
        let kind = ClaudeResetKind::of_clears(&clears_of(grant))
            .unwrap_or_else(|| label.map_or(ClaudeResetKind::Other, ClaudeResetKind::of_label));
        let label = label.unwrap_or(kind.default_label()).to_string();
        let paused = grant.get("paused").and_then(Value::as_bool) == Some(true);
        let key = (kind, label.clone(), paused);
        groups
            .entry(key.clone())
            .or_insert(ClaudeResetGroup {
                kind,
                label,
                count: 0,
                earliest_expiry: None,
                paused,
            })
            .count += count;
        if let Some(e) = ends_at {
            expiries
                .entry(key)
                .and_modify(|soonest| *soonest = (*soonest).min(e))
                .or_insert(e);
        }
    }
    groups
        .into_iter()
        .map(|(key, mut group)| {
            group.earliest_expiry = expiries.get(&key).copied().map(iso);
            group
        })
        .collect()
}

/// The weekly free 5-hour reset, when `juniper_tide` offers it (its `reset` arm only).
fn weekly_five_hour_of(juniper: Option<&Value>) -> Option<bool> {
    let j = juniper?.as_object()?;
    let offered = j.get("eligible").and_then(Value::as_bool) == Some(true)
        && j.get("arm").and_then(Value::as_str) == Some("reset");
    offered.then(|| j.get("available").and_then(Value::as_bool) == Some(true))
}

/// The resets a usage reply lists, or None when it lists none (ineligible, null, or absent).
pub fn resets_of(reply: &Map<String, Value>, now: DateTime<Utc>) -> Option<ClaudeResets> {
    let resets = ClaudeResets {
        groups: groups_of(reply.get("cedar_ember"), now),
        weekly_five_hour_available: weekly_five_hour_of(reply.get("juniper_tide")),
    };
    (!resets.groups.is_empty() || resets.weekly_five_hour_available.is_some()).then_some(resets)
}

/// The read Claude Code makes at the 5-hour limit, the only one that reports `juniper_tide`.
pub enum AtWallRead {
    /// The 5-hour limit was not reached, so it was not asked.
    NotAsked,
    Failed,
    Read(Map<String, Value>),
}

/// "absent", "null", "object" or the JSON type of a reply key.
fn presence(reply: &Map<String, Value>, key: &str) -> &'static str {
    match reply.get(key) {
        None => "absent",
        Some(Value::Null) => "null",
        Some(Value::Object(_)) => "object",
        Some(Value::Array(_)) => "array",
        Some(_) => "scalar",
    }
}

fn text(v: Option<&Value>) -> String {
    match v {
        None => "-".into(),
        Some(Value::String(s)) => format!("{s:?}"),
        Some(v) => v.to_string(),
    }
}

fn program_line(block: Option<&Value>) -> String {
    let Some(b) = block.and_then(Value::as_object) else {
        return "no block".into();
    };
    format!(
        "eligible {}; ineligible_reason {}; arm {}; available {}; grants {}",
        text(b.get("eligible")),
        text(b.get("ineligible_reason")),
        text(b.get("arm")),
        text(b.get("available")),
        b.get("grants")
            .and_then(Value::as_array)
            .map_or_else(|| "-".into(), |g| g.len().to_string()),
    )
}

const MAX_GRANT_LINES: usize = 20;

/// The reset fields of the last usage reply, for the diagnostics report: which keys came back and
/// each grant's name, limits, count, dates and paused flag. Grant ids and other fields are left out.
pub fn report(reply: &Map<String, Value>, at_wall: &AtWallRead) -> String {
    let keys = [
        "cedar_ember",
        "juniper_tide",
        "iguana_necktie",
        "amber_gauge",
    ]
    .iter()
    .map(|k| format!("{k} {}", presence(reply, k)))
    .collect::<Vec<_>>()
    .join(", ");
    let mut lines = vec![
        format!("Keys: {keys}"),
        format!("cedar_ember: {}", program_line(reply.get("cedar_ember"))),
    ];
    let grants = reply
        .get("cedar_ember")
        .and_then(|c| c.get("grants"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for grant in grants.iter().take(MAX_GRANT_LINES) {
        let Some(g) = grant.as_object() else {
            lines.push("  grant: not an object".into());
            continue;
        };
        lines.push(format!(
            "  grant: label {}; clears {}; resets_left {}; resets_total {}; starts_at {}; ends_at {}; paused {}; usable_now {}",
            text(g.get("label")),
            text(g.get("clears")),
            text(g.get("resets_left")),
            text(g.get("resets_total")),
            text(g.get("starts_at")),
            text(g.get("ends_at")),
            text(g.get("paused")),
            text(g.get("usable_now")),
        ));
    }
    if grants.len() > MAX_GRANT_LINES {
        lines.push(format!("  ... {} more", grants.len() - MAX_GRANT_LINES));
    }
    lines.push(match at_wall {
        AtWallRead::NotAsked => {
            "juniper_tide (at-limit read): not asked, 5-hour limit not reached".into()
        }
        AtWallRead::Failed => "juniper_tide (at-limit read): the read failed".into(),
        AtWallRead::Read(r) => format!(
            "juniper_tide (at-limit read): {}",
            program_line(r.get("juniper_tide"))
        ),
    });
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        "2026-11-12T12:00:00Z".parse().unwrap()
    }

    fn resets(reply: Value) -> Option<ClaudeResets> {
        resets_of(reply.as_object().unwrap(), now())
    }

    /// A `cedar_ember` block shaped like the trackers' fixtures (usage-monitor resets.test.ts).
    fn cedar(grants: Value) -> Value {
        json!({ "cedar_ember": { "eligible": true, "next_grant_id": "g1", "grants": grants } })
    }

    #[test]
    fn grants_group_by_kind_and_name_with_the_soonest_expiry() {
        let r = resets(cedar(json!([
            { "id": "g1", "label": "Full reset", "resets_left": 1, "clears": ["five_hour", "seven_day"], "ends_at": "2026-12-03T00:00:00Z", "usable_now": true },
            { "id": "g2", "label": "Full reset", "resets_left": 1, "clears": ["five_hour", "seven_day"], "ends_at": "2026-11-20T08:00:00Z" },
            { "id": "g3", "resets_left": 1, "clears": ["five_hour"] },
            { "id": "g4", "clears": ["seven_day", "seven_day_opus"] },
            { "id": "g5", "label": " Welcome back ", "resets_left": 2 },
        ])))
        .unwrap();
        let groups: Vec<_> = r
            .groups
            .iter()
            .map(|g| {
                (
                    g.kind,
                    g.label.as_str(),
                    g.count,
                    g.earliest_expiry.as_deref(),
                    g.paused,
                )
            })
            .collect();
        assert_eq!(
            groups,
            vec![
                (
                    ClaudeResetKind::Full,
                    "Full reset",
                    2,
                    Some("2026-11-20T08:00:00.000Z"),
                    false
                ),
                (ClaudeResetKind::FiveHour, "5-hour reset", 1, None, false),
                (ClaudeResetKind::Weekly, "Weekly reset", 1, None, false),
                (ClaudeResetKind::Other, "Welcome back", 2, None, false),
            ]
        );
        assert_eq!(r.weekly_five_hour_available, None);
        assert_eq!(r.total(), 6);
    }

    #[test]
    fn spent_expired_unstarted_and_odd_grants_are_left_out_and_paused_ones_not_counted() {
        let r = resets(cedar(json!([
            { "id": "live", "label": "5-hour reset", "resets_left": 1 },
            { "id": "paused", "label": "5-hour reset", "resets_left": 1, "paused": true, "ends_at": "2026-12-01T00:00:00Z" },
            { "id": "gone", "resets_left": 3, "ends_at": "2026-11-01T00:00:00Z" },
            { "id": "later", "resets_left": 1, "starts_at": "2026-11-30T00:00:00Z" },
            { "id": "empty", "resets_left": 0 },
            { "id": "odd", "resets_left": "lots" },
            "not an object",
        ])))
        .unwrap();
        assert_eq!(
            r.groups,
            vec![
                ClaudeResetGroup {
                    kind: ClaudeResetKind::FiveHour,
                    label: "5-hour reset".into(),
                    count: 1,
                    earliest_expiry: None,
                    paused: false,
                },
                ClaudeResetGroup {
                    kind: ClaudeResetKind::FiveHour,
                    label: "5-hour reset".into(),
                    count: 1,
                    earliest_expiry: Some("2026-12-01T00:00:00.000Z".into()),
                    paused: true,
                },
            ]
        );
        assert_eq!(r.total(), 1);
    }

    #[test]
    fn ineligible_null_or_absent_is_no_resets() {
        assert_eq!(resets(json!({})), None);
        assert_eq!(
            resets(json!({ "cedar_ember": null, "juniper_tide": null })),
            None
        );
        assert_eq!(
            resets(
                json!({ "cedar_ember": { "eligible": false, "ineligible_reason": "surface", "grants": [{ "id": "g", "resets_left": 1 }] } })
            ),
            None
        );
        assert_eq!(resets(cedar(json!([]))), None);
    }

    #[test]
    fn the_weekly_five_hour_reset_counts_only_while_available_in_the_reset_arm() {
        let juniper = |j: Value| resets(json!({ "juniper_tide": j }));
        assert_eq!(
            juniper(json!({ "eligible": false, "ineligible_reason": "not_at_wall" })),
            None
        );
        assert_eq!(
            juniper(json!({ "eligible": true, "arm": "control", "available": true })),
            None
        );
        let available =
            juniper(json!({ "eligible": true, "arm": "reset", "available": true })).unwrap();
        assert_eq!(available.weekly_five_hour_available, Some(true));
        assert_eq!(available.total(), 1);
        let used = juniper(json!({ "eligible": true, "arm": "reset", "available": false, "next_available_at": "2026-11-15T00:00:00Z" })).unwrap();
        assert_eq!(used.weekly_five_hour_available, Some(false));
        assert_eq!(used.total(), 0);
    }

    #[test]
    fn serialises_in_camel_case() {
        let r = resets(json!({
            "cedar_ember": { "eligible": true, "grants": [{ "id": "g", "label": "Full reset", "resets_left": 1, "clears": ["five_hour", "seven_day"] }] },
            "juniper_tide": { "eligible": true, "arm": "reset", "available": true },
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({
                "groups": [{ "kind": "full", "label": "Full reset", "count": 1, "earliestExpiry": null, "paused": false }],
                "weeklyFiveHourAvailable": true,
            })
        );
    }

    #[test]
    fn the_report_names_fields_but_never_grant_ids() {
        let reply = json!({
            "cedar_ember": { "eligible": true, "next_grant_id": "grant_secret_1", "grants": [
                { "id": "grant_secret_1", "label": "Full reset", "resets_left": 2, "clears": ["five_hour", "seven_day"], "ends_at": "2026-12-03T00:00:00Z", "paused": false },
            ] },
            "juniper_tide": null,
            "iguana_necktie": { "limit_dollars": 50 },
        });
        let text = report(reply.as_object().unwrap(), &AtWallRead::NotAsked);
        assert!(!text.contains("grant_secret"), "{text}");
        assert_eq!(
            text,
            "Keys: cedar_ember object, juniper_tide null, iguana_necktie object, amber_gauge absent\n\
             cedar_ember: eligible true; ineligible_reason -; arm -; available -; grants 1\n  \
             grant: label \"Full reset\"; clears [\"five_hour\",\"seven_day\"]; resets_left 2; resets_total -; starts_at -; ends_at \"2026-12-03T00:00:00Z\"; paused false; usable_now -\n\
             juniper_tide (at-limit read): not asked, 5-hour limit not reached"
        );
        let wall =
            json!({ "juniper_tide": { "eligible": false, "ineligible_reason": "not_at_wall" } });
        let text = report(
            json!({ "cedar_ember": { "eligible": false, "ineligible_reason": "surface" } })
                .as_object()
                .unwrap(),
            &AtWallRead::Read(wall.as_object().unwrap().clone()),
        );
        assert!(text.contains("cedar_ember: eligible false; ineligible_reason \"surface\"; arm -; available -; grants -"));
        assert!(text.ends_with("juniper_tide (at-limit read): eligible false; ineligible_reason \"not_at_wall\"; arm -; available -; grants -"));
    }
}
