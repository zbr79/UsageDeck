//! Cursor's DashboardService replies as `CursorUsage`, and the details logged when a call fails.
//!
//! Cursor's Settings → Plan & Usage shows two included-usage buckets. "Cursor Models" is
//! `planUsage.autoPercentUsed` and "Other Models" is `planUsage.apiPercentUsed`: Cursor takes the
//! titles from its server-sent `auto_spillover_ui_config` (older builds fell back to "Auto" and
//! "API"), and the fields behind them are the same. Both reset at `billingCycleEnd`.
//!
//! No Windows or app dependencies, so the unit tests also run on Linux.

use super::util::{number_or_null, string_or_null, to_iso};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `CursorUsage` in lib/types.ts. The aliases read usage cached under the 1.4.1 names.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorUsage {
    #[serde(alias = "autoPercentUsed")]
    pub cursor_models_percent_used: Option<f64>,
    #[serde(alias = "apiPercentUsed")]
    pub other_models_percent_used: Option<f64>,
    /// When both buckets reset.
    #[serde(alias = "billingCycleEnd")]
    pub resets_at: Option<String>,
    /// None when GetHardLimit did not answer.
    pub on_demand: Option<OnDemand>,
    pub account_name: Option<String>,
    pub plan_name: Option<String>,
}

/// On-demand usage: what Cursor bills once the included buckets are used up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum OnDemand {
    Disabled,
    #[serde(rename_all = "camelCase")]
    Enabled {
        spent_usd: f64,
        /// None: no monthly limit.
        limit_usd: Option<f64>,
    },
}

/// The `hardLimit` GetHardLimit reports for an unlimited on-demand budget (i32::MAX dollars).
const UNLIMITED: f64 = 2_147_483_647.0;

/// GetCurrentPeriodUsage reply: `planUsage` must be an object; its fields stay lenient.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageReply {
    plan_usage: serde_json::Map<String, Value>,
    #[serde(default)]
    billing_cycle_end: Value,
    #[serde(default)]
    spend_limit_usage: Value,
}

/// The usage of one account. `plan` (GetPlanInfo) and `hard_limit` (GetHardLimit) are None when
/// their call failed.
pub fn cursor_usage(
    usage: &UsageReply,
    plan: Option<&Value>,
    hard_limit: Option<&Value>,
    account_name: Option<String>,
) -> CursorUsage {
    let pu = |k: &str| number_or_null(usage.plan_usage.get(k).unwrap_or(&Value::Null));
    CursorUsage {
        cursor_models_percent_used: pu("autoPercentUsed"),
        other_models_percent_used: pu("apiPercentUsed"),
        resets_at: to_iso(&usage.billing_cycle_end),
        on_demand: hard_limit.map(|h| on_demand(h, &usage.spend_limit_usage)),
        account_name,
        plan_name: plan.and_then(|p| cursor_plan_name(&p["planInfo"]["planName"])),
    }
}

/// GetHardLimit says whether on-demand usage is allowed and its monthly limit in dollars;
/// `spendLimitUsage.individualUsed` is the spend in cents, as Cursor's own page reads them. The
/// replies are proto3 JSON, which leaves out zero and false fields, so a missing field is 0 or false.
fn on_demand(hard_limit: &Value, spend: &Value) -> OnDemand {
    if hard_limit["noUsageBasedAllowed"].as_bool() == Some(true) {
        return OnDemand::Disabled;
    }
    let limit = hard_limit["hardLimit"].as_f64().unwrap_or(0.0);
    OnDemand::Enabled {
        spent_usd: spend["individualUsed"].as_f64().unwrap_or(0.0) / 100.0,
        limit_usd: (limit < UNLIMITED).then_some(limit),
    }
}

/// "20" followed by a non-word character or the end (the `\$?\s*20(?:\.00)?\b` of the old code).
fn mentions_twenty(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(1)).any(|i| {
        b[i] == b'2'
            && b[i + 1] == b'0'
            && b.get(i + 2)
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'))
    })
}

fn cursor_plan_name(value: &Value) -> Option<String> {
    let plan = string_or_null(value)?;
    let lower = plan.to_lowercase();
    Some(if lower.contains("pro") || mentions_twenty(&lower) {
        "Pro".into()
    } else {
        plan
    })
}

/// What to log after the status of a failed call: the Connect error in the body, and on a 401
/// whether the stored access token had expired (`exp`, seconds). An expired token says Cursor has
/// not written a renewed one yet; a valid one says the server refused the request itself.
pub fn failure_detail(status: u16, body: &[u8], exp: Option<i64>, now: i64) -> String {
    let mut out = String::new();
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        let code = v["code"].as_str().unwrap_or("?");
        let message: String = v["message"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(160)
            .collect();
        out = format!(" ({code}: {message})");
    }
    if status == 401 {
        out += &match exp.and_then(|e| chrono::DateTime::from_timestamp(e, 0).map(|at| (e, at))) {
            Some((e, at)) if e <= now => format!("; the stored access token expired at {at}"),
            Some((_, at)) => format!("; the stored access token is valid until {at}"),
            None => "; the stored access token has no expiry".into(),
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &[u8] = include_bytes!("../../tests/fixtures/cursor/ok.json");
    const ALL_USED: &[u8] = include_bytes!("../../tests/fixtures/cursor/all-used.json");

    fn parse(fixture: &[u8]) -> CursorUsage {
        let v: Value = serde_json::from_slice(fixture).unwrap();
        let usage: UsageReply = serde_json::from_value(v["usage"].clone()).unwrap();
        let hard_limit = v.get("hardLimit");
        cursor_usage(
            &usage,
            Some(&v["plan"]),
            hard_limit,
            Some("user@example.com".into()),
        )
    }

    #[test]
    fn ok_fixture_reads_both_buckets_and_the_on_demand_budget() {
        let u = parse(OK);
        assert_eq!(u.cursor_models_percent_used, Some(100.0));
        assert!((u.other_models_percent_used.unwrap() - 18.977777777777778).abs() < 1e-9);
        assert_eq!(u.resets_at.as_deref(), Some("2026-11-11T22:16:13.000Z"));
        assert_eq!(
            u.on_demand,
            Some(OnDemand::Enabled {
                spent_usd: 0.0,
                limit_usd: Some(50.0)
            })
        );
        assert_eq!(u.plan_name.as_deref(), Some("Pro"));
        assert_eq!(u.account_name.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn all_used_fixture_with_on_demand_off_is_disabled() {
        let u = parse(ALL_USED);
        assert_eq!(u.cursor_models_percent_used, Some(100.0));
        assert_eq!(u.other_models_percent_used, Some(100.0));
        assert_eq!(u.on_demand, Some(OnDemand::Disabled));
        assert_eq!(u.resets_at.as_deref(), Some("2026-11-11T22:16:13.000Z"));
    }

    #[test]
    fn on_demand_spend_unlimited_missing_fields_and_no_reply() {
        let spent = serde_json::json!({ "individualUsed": 1234, "individualLimit": 5000 });
        assert_eq!(
            on_demand(&serde_json::json!({ "hardLimit": 50 }), &spent),
            OnDemand::Enabled {
                spent_usd: 12.34,
                limit_usd: Some(50.0)
            }
        );
        let spend = serde_json::json!({ "limitType": "user" });
        let unlimited = serde_json::json!({ "hardLimit": 2147483647 });
        assert_eq!(
            on_demand(&unlimited, &spend),
            OnDemand::Enabled {
                spent_usd: 0.0,
                limit_usd: None
            }
        );
        // proto3 JSON leaves out a zero limit and a false flag
        assert_eq!(
            on_demand(&serde_json::json!({}), &spend),
            OnDemand::Enabled {
                spent_usd: 0.0,
                limit_usd: Some(0.0)
            }
        );
        let v: Value = serde_json::from_slice(OK).unwrap();
        let usage: UsageReply = serde_json::from_value(v["usage"].clone()).unwrap();
        assert_eq!(cursor_usage(&usage, None, None, None).on_demand, None);
    }

    #[test]
    fn usage_cached_by_1_4_1_keeps_its_numbers() {
        let old = r#"{"autoPercentUsed":100,"apiPercentUsed":20,"totalPercentUsed":60,
            "billingCycleEnd":"2026-11-11T22:16:13.000Z","totalSpend":null,"limit":null,
            "bonusSpend":null,"accountName":"a@example.com","planName":"Pro"}"#;
        let u: CursorUsage = serde_json::from_str(old).unwrap();
        assert_eq!(u.cursor_models_percent_used, Some(100.0));
        assert_eq!(u.other_models_percent_used, Some(20.0));
        assert_eq!(u.resets_at.as_deref(), Some("2026-11-11T22:16:13.000Z"));
        assert_eq!(u.on_demand, None);
    }

    #[test]
    fn serialises_with_the_names_the_frontend_reads() {
        let u = CursorUsage {
            on_demand: Some(OnDemand::Enabled {
                spent_usd: 1.5,
                limit_usd: None,
            }),
            ..Default::default()
        };
        let v = serde_json::to_value(&u).unwrap();
        assert!(v.get("cursorModelsPercentUsed").is_some());
        assert!(v.get("otherModelsPercentUsed").is_some());
        assert!(v.get("resetsAt").is_some());
        assert_eq!(
            v["onDemand"],
            serde_json::json!({ "state": "enabled", "spentUsd": 1.5, "limitUsd": null })
        );
        let off = serde_json::to_value(OnDemand::Disabled).unwrap();
        assert_eq!(off, serde_json::json!({ "state": "disabled" }));
    }

    #[test]
    fn plan_names() {
        let name = |s: &str| cursor_plan_name(&Value::from(s));
        assert_eq!(name("$20/mo").as_deref(), Some("Pro"));
        assert_eq!(name("Business").as_deref(), Some("Business"));
        assert_eq!(name("plan 200").as_deref(), Some("plan 200"));
    }

    #[test]
    fn failure_detail_names_the_error_and_the_token_expiry() {
        let body = br#"{"code":"unauthenticated","message":"Not logged in"}"#;
        assert_eq!(
            failure_detail(401, body, Some(100), 200),
            " (unauthenticated: Not logged in); the stored access token expired at 1970-01-01 00:01:40 UTC"
        );
        assert_eq!(
            failure_detail(401, b"", Some(300), 200),
            "; the stored access token is valid until 1970-01-01 00:05:00 UTC"
        );
        assert_eq!(
            failure_detail(401, b"<html>", None, 200),
            "; the stored access token has no expiry"
        );
        assert_eq!(failure_detail(503, b"", Some(100), 200), "");
    }
}
