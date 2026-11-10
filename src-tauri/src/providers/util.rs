//! Small value helpers of lib/time.ts, shared by the Codex, Cursor and Grok ports.

use serde_json::Value;

/// Seconds or milliseconds since the epoch, a numeric string, or a date string, as an ISO time
/// in the `Date.toISOString()` format. Anything else is None.
pub(crate) fn to_iso(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => {
            let v = n.as_f64().filter(|v| v.is_finite())?;
            let ms = if v < 1e12 { v * 1000.0 } else { v };
            let d = chrono::DateTime::from_timestamp_millis(ms.trunc() as i64)?;
            Some(d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        }
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                return None;
            }
            if let Ok(n) = t.parse::<f64>() {
                if t.chars().all(|c| c.is_ascii_digit() || c == '.') {
                    return to_iso(&serde_json::json!(n));
                }
            }
            let d = chrono::DateTime::parse_from_rfc3339(t).ok()?;
            Some(
                d.with_timezone(&chrono::Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            )
        }
        _ => None,
    }
}

pub(crate) fn number_or_null(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite())
}

pub(crate) fn string_or_null(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}
