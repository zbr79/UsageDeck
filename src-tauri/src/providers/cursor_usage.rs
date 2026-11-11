//! Cursor's DashboardService replies as `CursorUsage`, and the details logged when a call fails.
//!
//! Cursor's Settings → Plan & Usage shows two included-usage buckets. "Cursor Models" is
//! `planUsage.autoPercentUsed` and "Other Models" is `planUsage.apiPercentUsed`: Cursor takes the
//! titles from its server-sent `auto_spillover_ui_config` (older builds fell back to "Auto" and
//! "API"), and the fields behind them are the same. Both reset at `billingCycleEnd`.
//!
//! No Windows or app dependencies, so the unit tests also run on Linux.

use super::cursor_auth::excerpt;
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

/// One included-usage bucket. The replies are proto3 JSON, which leaves out zero fields, so at the
/// start of a billing cycle a bucket is simply absent: it reads as 0 unless `totalPercentUsed`
/// says something was used, in which case the reply has a shape this build does not know.
fn bucket(plan_usage: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    let number = |k: &str| plan_usage.get(k).and_then(number_or_null);
    number(key).or_else(|| {
        let other_known = ["autoPercentUsed", "apiPercentUsed"]
            .iter()
            .any(|k| number(k).is_some());
        let nothing_used = number("totalPercentUsed").is_none_or(|total| total == 0.0);
        (other_known || nothing_used).then_some(0.0)
    })
}

/// The names of a JSON object's fields, for a diagnostic that says what a reply held instead of
/// what was expected. Field names are not secrets; values never appear.
fn keys(value: &Value) -> String {
    match value.as_object() {
        Some(map) if map.is_empty() => "an empty object".into(),
        Some(map) => {
            let names: Vec<&str> = map.keys().map(String::as_str).take(12).collect();
            format!("fields: {}", names.join(", "))
        }
        None => "not an object".into(),
    }
}

/// The usage of one account from GetCurrentPeriodUsage. `plan` (GetPlanInfo) and `hard_limit`
/// (GetHardLimit) are None when their call failed. Err says what the reply lacked.
pub fn usage_from_dashboard(
    usage: &[u8],
    plan: Option<&Value>,
    hard_limit: Option<&Value>,
    account_name: Option<String>,
) -> Result<CursorUsage, String> {
    let reply: Value =
        serde_json::from_slice(usage).map_err(|_| "the reply is not JSON".to_string())?;
    let Some(plan_usage) = reply["planUsage"].as_object() else {
        return Err(format!("no planUsage; {}", keys(&reply)));
    };
    Ok(CursorUsage {
        cursor_models_percent_used: bucket(plan_usage, "autoPercentUsed"),
        other_models_percent_used: bucket(plan_usage, "apiPercentUsed"),
        resets_at: to_iso(&reply["billingCycleEnd"]),
        on_demand: hard_limit.map(|h| on_demand(h, &reply["spendLimitUsage"])),
        account_name,
        plan_name: plan.and_then(|p| cursor_plan_name(&p["planInfo"]["planName"])),
    })
}

/// The same usage from `GET https://cursor.com/api/usage-summary`, the reply behind Cursor's web
/// dashboard: `individualUsage.plan` holds the two percentages, `individualUsage.onDemand` the
/// spend and limit in cents, `membershipType` the plan. Err says what the reply lacked.
pub fn usage_from_summary(
    body: &[u8],
    account_name: Option<String>,
) -> Result<CursorUsage, String> {
    let reply: Value =
        serde_json::from_slice(body).map_err(|_| "the reply is not JSON".to_string())?;
    let individual = &reply["individualUsage"];
    let Some(plan) = individual["plan"].as_object() else {
        let held = if individual.is_object() {
            individual
        } else {
            &reply
        };
        return Err(format!("no individualUsage.plan; {}", keys(held)));
    };
    let on_demand = individual["onDemand"].as_object().map(|od| {
        let cents = |k: &str| od.get(k).and_then(number_or_null);
        if od.get("enabled").and_then(Value::as_bool) == Some(false) {
            OnDemand::Disabled
        } else {
            OnDemand::Enabled {
                spent_usd: cents("used").unwrap_or(0.0) / 100.0,
                limit_usd: cents("limit").map(|c| c / 100.0),
            }
        }
    });
    Ok(CursorUsage {
        cursor_models_percent_used: bucket(plan, "autoPercentUsed"),
        other_models_percent_used: bucket(plan, "apiPercentUsed"),
        resets_at: to_iso(&reply["billingCycleEnd"]),
        on_demand,
        account_name,
        plan_name: string_or_null(&reply["membershipType"]).map(|m| membership_name(&m)),
    })
}

/// "pro" as "Pro", "free_trial" as "Free trial"; any Pro tier is "Pro", like `cursor_plan_name`.
fn membership_name(membership: &str) -> String {
    let words = membership.replace('_', " ").to_lowercase();
    if words.contains("pro") {
        return "Pro".into();
    }
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => words,
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

/// What a failed reply said, for the diagnostics: its Connect error (`code: message`), else the
/// first 200 characters of its body. Token-like text is redacted either way.
pub fn reply_detail(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        if let Some(code) = v["code"].as_str() {
            let message = v["message"].as_str().unwrap_or("");
            return Some(excerpt(&format!("{code}: {message}"), 200));
        }
    }
    let shown = excerpt(&text, 200);
    (!shown.is_empty()).then_some(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &[u8] = include_bytes!("../../tests/fixtures/cursor/ok.json");
    const ALL_USED: &[u8] = include_bytes!("../../tests/fixtures/cursor/all-used.json");

    fn parse(fixture: &[u8]) -> CursorUsage {
        let v: Value = serde_json::from_slice(fixture).unwrap();
        usage_from_dashboard(
            &serde_json::to_vec(&v["usage"]).unwrap(),
            Some(&v["plan"]),
            v.get("hardLimit"),
            Some("user@example.com".into()),
        )
        .unwrap()
    }

    fn dashboard(usage: Value) -> Result<CursorUsage, String> {
        usage_from_dashboard(&serde_json::to_vec(&usage).unwrap(), None, None, None)
    }

    #[test]
    fn a_reply_without_plan_usage_says_what_it_held() {
        let invalid: Value =
            serde_json::from_slice(include_bytes!("../../tests/fixtures/cursor/invalid.json"))
                .unwrap();
        assert_eq!(
            dashboard(invalid["usage"].clone()),
            Err("no planUsage; fields: billingCycleEnd, planUsage".into())
        );
        assert_eq!(
            dashboard(serde_json::json!({})),
            Err("no planUsage; an empty object".into())
        );
        assert_eq!(
            usage_from_dashboard(b"<html>", None, None, None),
            Err("the reply is not JSON".into())
        );
    }

    #[test]
    fn a_bucket_left_out_at_the_start_of_a_cycle_is_zero() {
        let fresh = dashboard(serde_json::json!({ "planUsage": { "limit": 2000 } })).unwrap();
        assert_eq!(fresh.cursor_models_percent_used, Some(0.0));
        assert_eq!(fresh.other_models_percent_used, Some(0.0));
        let one = dashboard(serde_json::json!({
            "planUsage": { "autoPercentUsed": 12.5, "totalPercentUsed": 6 }
        }))
        .unwrap();
        assert_eq!(one.cursor_models_percent_used, Some(12.5));
        assert_eq!(one.other_models_percent_used, Some(0.0));
        // usage is reported, but not in the fields this build reads: unknown, never a false 0%
        let renamed =
            dashboard(serde_json::json!({ "planUsage": { "totalPercentUsed": 40 } })).unwrap();
        assert_eq!(renamed.cursor_models_percent_used, None);
        assert_eq!(renamed.other_models_percent_used, None);
    }

    #[test]
    fn the_web_summary_gives_the_same_buckets_reset_and_on_demand() {
        let body = include_bytes!("../../tests/fixtures/cursor/usage-summary.json");
        let u = usage_from_summary(body, Some("user@example.com".into())).unwrap();
        assert_eq!(u.cursor_models_percent_used, Some(100.0));
        assert_eq!(u.other_models_percent_used, Some(100.0));
        assert_eq!(u.resets_at.as_deref(), Some("2026-11-11T22:16:13.000Z"));
        assert_eq!(u.on_demand, Some(OnDemand::Disabled));
        assert_eq!(u.plan_name.as_deref(), Some("Pro"));
        assert_eq!(u.account_name.as_deref(), Some("user@example.com"));

        let spending = serde_json::json!({
            "billingCycleEnd": "2026-12-11T22:16:13.000Z",
            "membershipType": "free_trial",
            "individualUsage": {
                "plan": { "autoPercentUsed": 3, "apiPercentUsed": 1.5, "totalPercentUsed": 2 },
                "onDemand": { "enabled": true, "used": 1234, "limit": 5000, "remaining": 3766 }
            }
        });
        let u = usage_from_summary(&serde_json::to_vec(&spending).unwrap(), None).unwrap();
        assert_eq!(
            u.on_demand,
            Some(OnDemand::Enabled {
                spent_usd: 12.34,
                limit_usd: Some(50.0)
            })
        );
        assert_eq!(u.plan_name.as_deref(), Some("Free trial"));
        assert_eq!(u.other_models_percent_used, Some(1.5));
    }

    #[test]
    fn a_web_summary_without_a_plan_says_what_it_held() {
        assert_eq!(
            usage_from_summary(br#"{"error":"not_authenticated"}"#, None),
            Err("no individualUsage.plan; fields: error".into())
        );
        assert_eq!(
            usage_from_summary(br#"{"individualUsage":{"overall":{}}}"#, None),
            Err("no individualUsage.plan; fields: overall".into())
        );
        assert_eq!(
            usage_from_summary(b"<!doctype html>", None),
            Err("the reply is not JSON".into())
        );
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
        assert_eq!(dashboard(v["usage"].clone()).unwrap().on_demand, None);
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
    fn reply_detail_names_the_connect_error_or_quotes_the_body_without_tokens() {
        let body = br#"{"code":"unauthenticated","message":"Not logged in"}"#;
        assert_eq!(
            reply_detail(body).as_deref(),
            Some("unauthenticated: Not logged in")
        );
        assert_eq!(
            reply_detail(b"<html>\n<title>Just a moment...</title>").as_deref(),
            Some("<html> <title>Just a moment...</title>")
        );
        let leaked = br#"{"code":"invalid_argument","message":"token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhIn0.c2ln was refused"}"#;
        assert_eq!(
            reply_detail(leaked).as_deref(),
            Some("invalid_argument: token [redacted] was refused")
        );
        assert_eq!(reply_detail(b""), None);
        assert_eq!(reply_detail(&[b'a'; 500]).unwrap(), "[redacted]");
    }
}
