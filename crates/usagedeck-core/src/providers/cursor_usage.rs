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
    /// None when no API usage was read: none this cycle, or the usage events did not answer.
    pub api_spend: Option<ApiSpend>,
}

/// The API bucket's dollars this billing cycle and the models they went to, summed from the usage
/// events (`chargedCents`, what Cursor takes from the plan for each request).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiSpend {
    pub spent_usd: f64,
    /// The plan's included usage (`limit`), None when the usage reply had none.
    pub limit_usd: Option<f64>,
    /// Largest first.
    pub by_model: Vec<ModelSpend>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpend {
    pub model: String,
    pub usd: f64,
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
        api_spend: None,
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
        api_spend: None,
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

// ---- usage events: where the API dollars went ------------------------------------------------

/// What the usage reply (GetCurrentPeriodUsage or usage-summary) tells the usage-events request.
#[derive(Clone, Debug, PartialEq)]
pub struct Cycle {
    pub start_ms: i64,
    pub limit_usd: Option<f64>,
    /// `autoBucketModels`: the models billed to the first-party bucket. Only GetCurrentPeriodUsage
    /// lists them.
    pub first_party: Option<Vec<String>>,
}

/// None when the reply names no billing-cycle start.
pub fn cycle_of(usage_body: &[u8]) -> Option<Cycle> {
    let reply: Value = serde_json::from_slice(usage_body).ok()?;
    let start = to_iso(&reply["billingCycleStart"])?;
    let start_ms = chrono::DateTime::parse_from_rfc3339(&start)
        .ok()?
        .timestamp_millis();
    let limit_cents = number_or_null(&reply["planUsage"]["limit"])
        .or_else(|| number_or_null(&reply["individualUsage"]["plan"]["limit"]));
    let first_party = reply["autoBucketModels"].as_array().map(|models| {
        models
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    });
    Some(Cycle {
        start_ms,
        limit_usd: limit_cents.map(|c| c / 100.0),
        first_party,
    })
}

/// One usage event: one request, its model and what it took from the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageEvent {
    pub model: Option<String>,
    pub kind: Option<String>,
    pub charged_cents: f64,
}

/// One page of `POST cursor.com/api/dashboard/get-filtered-usage-events`.
#[derive(Clone, Debug, PartialEq)]
pub struct EventsPage {
    /// `totalUsageEventsCount`: the events of the whole query, on every page.
    pub total: Option<u64>,
    pub events: Vec<UsageEvent>,
}

/// A number Cursor may send as a string (`"3.5"`, `"1793000000000"`).
fn lenient_number(value: &Value) -> Option<f64> {
    number_or_null(value).or_else(|| {
        value
            .as_str()?
            .trim()
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite())
    })
}

/// Err says what the page lacked. Proto3 JSON leaves out an empty event list and a zero
/// `chargedCents`, so either reads as nothing.
pub fn events_page(body: &[u8]) -> Result<EventsPage, String> {
    let reply: Value =
        serde_json::from_slice(body).map_err(|_| "the reply is not JSON".to_string())?;
    let Some(object) = reply.as_object() else {
        return Err("the reply is not an object".into());
    };
    let total = object
        .get("totalUsageEventsCount")
        .and_then(lenient_number)
        .filter(|t| *t >= 0.0)
        .map(|t| t as u64);
    let events = match object.get("usageEventsDisplay") {
        Some(Value::Array(events)) => events,
        None if object.is_empty() || total == Some(0) => {
            return Ok(EventsPage {
                total,
                events: Vec::new(),
            })
        }
        _ => return Err(format!("no usageEventsDisplay; {}", keys(&reply))),
    };
    let events = events
        .iter()
        .map(|e| UsageEvent {
            model: string_or_null(&e["model"]),
            kind: string_or_null(&e["kind"]),
            charged_cents: lenient_number(&e["chargedCents"])
                .filter(|c| *c > 0.0)
                .unwrap_or(0.0),
        })
        .collect();
    Ok(EventsPage { total, events })
}

/// Whether the pages read so far hold every event: `sent` came on the last page, `read` on all.
/// Cursor may send fewer events than asked for per page, so a short page ends the read only when
/// the reply gave no total.
pub fn events_complete(total: Option<u64>, sent: usize, read: usize, page_size: usize) -> bool {
    match total {
        Some(total) => sent == 0 || read as u64 >= total,
        None => sent < page_size,
    }
}

/// Whether a model is billed to the first-party bucket ("Cursor Models"). Without Cursor's list
/// (the cursor.com path), Auto and Composer are.
fn is_first_party(model: &str, listed: Option<&[String]>) -> bool {
    match listed {
        Some(models) => models.iter().any(|m| m == model),
        None => model == "default" || model == "auto" || model.starts_with("composer"),
    }
}

/// The API bucket's spend from this cycle's events: every event outside the first-party bucket
/// that Cursor did not mark as errored, summed per model. None when it comes to nothing.
pub fn api_spend(events: &[UsageEvent], cycle: &Cycle) -> Option<ApiSpend> {
    let mut by_model: Vec<ModelSpend> = Vec::new();
    for event in events {
        let model = event.model.as_deref().unwrap_or("unknown");
        let errored = event.kind.as_deref().is_some_and(|k| k.contains("ERRORED"));
        if errored
            || event.charged_cents <= 0.0
            || is_first_party(model, cycle.first_party.as_deref())
        {
            continue;
        }
        let usd = event.charged_cents / 100.0;
        match by_model.iter_mut().find(|m| m.model == model) {
            Some(m) => m.usd += usd,
            None => by_model.push(ModelSpend {
                model: model.to_string(),
                usd,
            }),
        }
    }
    by_model.sort_by(|a, b| b.usd.total_cmp(&a.usd).then_with(|| a.model.cmp(&b.model)));
    let spent_usd: f64 = by_model.iter().map(|m| m.usd).sum();
    (spent_usd > 0.0).then_some(ApiSpend {
        spent_usd,
        limit_usd: cycle.limit_usd,
        by_model,
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

    const EVENTS: &[u8] = include_bytes!("../../tests/fixtures/cursor/usage-events.json");

    fn ok_cycle() -> Cycle {
        let v: Value = serde_json::from_slice(OK).unwrap();
        cycle_of(&serde_json::to_vec(&v["usage"]).unwrap()).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_cycle_comes_from_either_usage_reply() {
        assert_eq!(
            ok_cycle(),
            Cycle {
                start_ms: 1_791_756_973_000,
                limit_usd: Some(20.0),
                first_party: Some(vec![
                    "default".into(),
                    "composer-1.5".into(),
                    "composer-2".into()
                ]),
            }
        );
        let summary = cycle_of(include_bytes!(
            "../../tests/fixtures/cursor/usage-summary.json"
        ))
        .unwrap();
        assert_eq!(summary.start_ms, 1_791_756_973_000);
        assert_eq!(summary.limit_usd, Some(20.0));
        assert_eq!(summary.first_party, None);
        assert_eq!(cycle_of(br#"{"planUsage":{}}"#), None);
        assert_eq!(cycle_of(b"<html>"), None);
    }

    #[test]
    fn api_spend_sums_the_events_outside_the_first_party_bucket_per_model() {
        let page = events_page(EVENTS).unwrap();
        assert_eq!(page.total, Some(7));
        assert_eq!(page.events.len(), 7);
        let spend = api_spend(&page.events, &ok_cycle()).unwrap();
        let models: Vec<&str> = spend.by_model.iter().map(|m| m.model.as_str()).collect();
        // composer-2 and default are first-party; the errored gpt-5 request cost nothing
        assert_eq!(
            models,
            ["claude-4.6-opus-high-thinking", "gpt-5", "gemini-3-pro"]
        );
        assert!(close(spend.by_model[0].usd, 3.0503), "{spend:?}");
        assert!(close(spend.by_model[1].usd, 0.5));
        assert!(close(spend.spent_usd, 3.5723));
        assert_eq!(spend.limit_usd, Some(20.0));
        let v = serde_json::to_value(&spend).unwrap();
        assert!(v.get("spentUsd").is_some() && v.get("limitUsd").is_some());
        assert_eq!(
            v["byModel"][1],
            serde_json::json!({ "model": "gpt-5", "usd": 0.5 })
        );
    }

    #[test]
    fn without_cursors_model_list_auto_and_composer_are_first_party() {
        let page = events_page(EVENTS).unwrap();
        let cycle = Cycle {
            first_party: None,
            ..ok_cycle()
        };
        let spend = api_spend(&page.events, &cycle).unwrap();
        assert!(close(spend.spent_usd, 3.5723));
        let only_composer = UsageEvent {
            model: Some("composer-2".into()),
            kind: None,
            charged_cents: 40.0,
        };
        assert_eq!(api_spend(&[only_composer], &cycle), None);
        assert_eq!(api_spend(&[], &cycle), None);
    }

    #[test]
    fn an_events_page_without_events_is_empty_and_an_odd_one_says_what_it_held() {
        assert_eq!(
            events_page(br#"{"totalUsageEventsCount":0}"#)
                .unwrap()
                .events,
            vec![]
        );
        assert_eq!(events_page(b"{}").unwrap().events, vec![]);
        assert_eq!(
            events_page(br#"{"totalUsageEventsCount":3,"message":"blocked"}"#),
            Err("no usageEventsDisplay; fields: totalUsageEventsCount, message".into())
        );
        assert_eq!(
            events_page(b"<!doctype html>"),
            Err("the reply is not JSON".into())
        );
        let odd = events_page(
            br#"{"totalUsageEventsCount":"2","usageEventsDisplay":[{"model":"m","chargedCents":"$0.05"},{"chargedCents":-5}]}"#,
        )
        .unwrap();
        assert_eq!(odd.total, Some(2));
        assert!(odd.events.iter().all(|e| e.charged_cents == 0.0));
        assert_eq!(odd.events[1].model, None);
    }

    #[test]
    fn the_events_are_complete_at_the_total_even_when_cursor_sends_short_pages() {
        // asked for 500, Cursor sent 100 of 250
        assert!(!events_complete(Some(250), 100, 100, 500));
        assert!(events_complete(Some(250), 50, 250, 500));
        assert!(
            events_complete(Some(250), 0, 200, 500),
            "an empty page ends it"
        );
        assert!(events_complete(None, 120, 620, 500));
        assert!(!events_complete(None, 500, 500, 500));
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
