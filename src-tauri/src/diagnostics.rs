//! The in-app diagnostics: what the last Cursor attempt and the last update check did, which
//! Claude models have no price, and which reset fields Claude's last usage reply held, as rows for the settings window and as a plain-text report the
//! user can copy. It answers "why is the card
//! stale" without log files.
//!
//! The report holds no token, key or account name: tokens appear only as a hash prefix and an
//! expiry, server replies are redacted where they are recorded (`cursor_auth::redact`), and the
//! finished text is redacted once more.

use crate::providers::cursor;
use crate::providers::cursor_auth::redact;
use crate::providers::cursor_diag::CursorAttempt;
use crate::updater::{UpdateStatus, Updater};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    app_version: String,
    /// e.g. "Windows 11 Pro 24H2 (build 26100)"
    system: String,
    /// None: nothing was tried yet since UsageDeck started
    cursor: Option<CursorAttempt>,
    /// why the Cursor card has no fresh usage, or None when it has
    cursor_reason: Option<String>,
    updater: UpdateStatus,
    /// the updater's lines of the report, for Settings → About → Updates
    updater_report: String,
    /// everything above as text, for the clipboard
    report: String,
}

/// The Windows edition, feature update and build. `ProductName` still says "Windows 10" on
/// Windows 11, whose builds start at 22000.
fn system_label(product: Option<&str>, release: Option<&str>, build: Option<&str>) -> String {
    let eleven = build
        .and_then(|b| b.trim().parse::<u32>().ok())
        .is_some_and(|b| b >= 22_000);
    let mut name = product.unwrap_or("Windows").trim().to_string();
    if eleven {
        name = name.replacen("Windows 10", "Windows 11", 1);
    }
    if let Some(release) = release.map(str::trim).filter(|r| !r.is_empty()) {
        name += &format!(" {release}");
    }
    match build.map(str::trim).filter(|b| !b.is_empty()) {
        Some(build) => format!("{name} (build {build})"),
        None => name,
    }
}

fn system() -> String {
    let key = crate::win::uninstall::wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let value = |name: &str| crate::win::uninstall::string_value(HKEY_LOCAL_MACHINE, &key, name);
    system_label(
        value("ProductName").as_deref(),
        value("DisplayVersion").as_deref(),
        value("CurrentBuild").as_deref(),
    )
}

fn stamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map_or_else(
        || "?".into(),
        |t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        },
    )
}

fn report_text(
    app_version: &str,
    system: &str,
    generated: &str,
    cursor: Option<&str>,
    updater: &str,
    claude_spend: &str,
    claude_resets: &str,
) -> String {
    let text = format!(
        "UsageDeck diagnostics (holds no token, key or account name)\n\
         App: UsageDeck {app_version}\n\
         System: {system}\n\
         Generated: {generated}\n\
         \n\
         Cursor\n\
         {}\n\
         \n\
         Updater\n\
         {updater}\n\
         \n\
         Claude spend\n\
         {claude_spend}\n\
         \n\
         Claude resets\n\
         {claude_resets}",
        cursor.unwrap_or("No attempt yet since UsageDeck started")
    );
    redact(&text)
}

fn collect(app: &AppHandle) -> Diagnostics {
    let app_version = app.package_info().version.to_string();
    let system = system();
    let cursor = cursor::last_attempt();
    let updater = app.state::<Updater>().status();
    let updater_report = updater.report(|ms| stamp(ms as i64));
    let claude_spend = app.try_state::<crate::Core>().map_or_else(
        || "Logs: not started".to_string(),
        |core| core.spend.spend(crate::claude_logs::now_ms()).report(),
    );
    let claude_resets = crate::providers::claude::last_resets_report().map_or_else(
        || "No usage reply yet since UsageDeck started".to_string(),
        |(at, fields)| format!("Usage reply: {}\n{fields}", stamp(at)),
    );
    let report = report_text(
        &app_version,
        &system,
        &stamp(chrono::Utc::now().timestamp_millis()),
        cursor.as_ref().map(|c| c.report(&chrono::Local)).as_deref(),
        &updater_report,
        &claude_spend,
        &claude_resets,
    );
    Diagnostics {
        cursor_reason: cursor.as_ref().and_then(|c| c.reason(&chrono::Local)),
        app_version,
        system,
        cursor,
        updater,
        updater_report,
        report,
    }
}

#[tauri::command]
pub fn get_diagnostics(app: AppHandle) -> Diagnostics {
    collect(&app)
}

/// Copy the report to the clipboard from Rust, like `copy_key`: the page needs no clipboard access.
#[tauri::command]
pub fn copy_diagnostics(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(collect(&app).report)
        .map_err(|e| {
            log::warn!("could not copy the diagnostics: {e}");
            "could not copy the diagnostics".to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_11_is_named_by_its_build() {
        assert_eq!(
            system_label(Some("Windows 10 Pro"), Some("24H2"), Some("26100")),
            "Windows 11 Pro 24H2 (build 26100)"
        );
        assert_eq!(
            system_label(Some("Windows 10 Home"), Some("22H2"), Some("19045")),
            "Windows 10 Home 22H2 (build 19045)"
        );
        assert_eq!(system_label(None, None, None), "Windows");
    }

    #[test]
    fn the_report_survives_redaction_and_a_leaked_token_does_not() {
        let cursor = "Request: POST api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage -> 401 (unauthenticated: Not logged in)\n\
                      Token: stored as safe-storage-v1; decrypt: ok; sha256 1a2b3c4d; token valid until Dec 3 (2026-12-03 10:00:00 +00:00)";
        let updater = "Running version: 1.4.4\nLast check: 2026-11-11 14:02:00 +00:00 (manual) -> up to date; latest release seen: 1.4.4\nStatus: none (idle)";
        let text = report_text(
            "1.4.4",
            "Windows 11 Pro 24H2 (build 26100)",
            "2026-11-11 14:03:00 +00:00",
            Some(cursor),
            updater,
            "Not priced (last 7 days): claude-opus-6 (4 replies, not in the dollar figures)",
            "Keys: cedar_ember object, juniper_tide null, iguana_necktie null, amber_gauge absent",
        );
        assert!(text.contains(cursor));
        assert!(text.contains(updater));
        assert!(text.ends_with(
            "\n\nClaude spend\nNot priced (last 7 days): claude-opus-6 (4 replies, not in the dollar figures)\n\n\
             Claude resets\nKeys: cedar_ember object, juniper_tide null, iguana_necktie null, amber_gauge absent"
        ));
        assert!(text.starts_with(
            "UsageDeck diagnostics (holds no token, key or account name)\nApp: UsageDeck 1.4.4\n"
        ));

        let jwt =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhdXRoMHx1c2VyXzAxQUJDIn0.c2lnbmF0dXJlc2lnbmF0dXJl";
        let leaked = report_text(
            "1.4.4",
            "Windows",
            "now",
            Some(&format!("oops {jwt}")),
            updater,
            "",
            "",
        );
        assert!(!leaked.contains("eyJ"));
        assert!(leaked.contains("oops [redacted]"));
        assert!(
            report_text("1.4.4", "Windows", "now", None, updater, "", "")
                .contains("Cursor\nNo attempt yet since UsageDeck started")
        );
    }
}
