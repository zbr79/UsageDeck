//! Pure-Rust CLI parser and install/update/uninstall decision engine.
//!
//! Kept completely free of Windows API calls so every parsing rule, version comparison,
//! backwards-compatibility transform, and settings merge can be unit-tested on any platform.

use serde_json::{Map, Value};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// Product and registry constants shared across installer modules.
pub const PRODUCT_NAME: &str = "UsageDeck";
pub const MAIN_BINARY_NAME: &str = "usagedeck.exe";
pub const UNINSTALLER_NAME: &str = "UsageDeck-Uninstall.exe";
pub const APP_DATA_FOLDER: &str = "com.usagedeck.app";
pub const KEYRING_SERVICE: &str = "UsageDeck";

/// Policy for managing the Desktop `.lnk` shortcut during installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopShortcutPolicy {
    /// Create or overwrite the desktop shortcut.
    Create,
    /// Only refresh the desktop shortcut if it already exists (never re-add one the user removed).
    UpdateIfExists,
    /// Remove the desktop shortcut if it exists.
    Remove,
    /// Do not create or modify any desktop shortcut (`/NS`).
    Never,
}

/// Parsed command-line options from NSIS-compatible UsageDeck installer flags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliArgs {
    /// `/S` (or `/P` upgraded to silent when `/UPDATE` is present).
    pub silent: bool,
    /// `/P` without `/S` and without `/UPDATE`.
    pub passive: bool,
    /// `/R` — launch UsageDeck after installation succeeds.
    pub relaunch: bool,
    /// `/UPDATE` — update mode (kill running app, replace binary, preserve user data, relaunch).
    pub update: bool,
    /// `/NS` — do not create Start Menu or Desktop shortcuts.
    pub no_shortcuts: bool,
    /// `/UNINSTALL` — uninstall UsageDeck.
    pub uninstall: bool,
    /// `/DELETE-DATA` or `/PURGE` — when uninstalling, also wipe `%APPDATA%`/`%LOCALAPPDATA%`
    /// folders and Windows Credential Manager `UsageDeck` entries.
    pub delete_data: bool,
    /// `/D=<folder>` — custom target installation directory.
    pub install_dir: Option<PathBuf>,
}

/// Concrete headless installation/update plan derived from CLI flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub silent: bool,
    #[allow(dead_code)]
    pub passive: bool,
    pub update: bool,
    pub relaunch: bool,
    pub create_start_menu_shortcut: bool,
    pub desktop_shortcut: DesktopShortcutPolicy,
    /// `Some(bool)` explicitly sets Run key + `settings.json` `startAtLogin`;
    /// `None` leaves the user's existing autostart configuration untouched.
    pub start_with_windows: Option<bool>,
    pub install_dir: Option<PathBuf>,
}

/// Concrete headless uninstallation plan derived from CLI flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallPlan {
    pub silent: bool,
    pub delete_data: bool,
    pub install_dir: Option<PathBuf>,
}

/// Top-level launch decision for `UsageDeck-Setup.exe` / `UsageDeck-Uninstall.exe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    /// No arguments passed (double-clicked): open the native Win32/Direct2D setup window.
    Gui,
    /// Headless installation or update triggered by `/S`, `/P`, `/UPDATE`, `/R`, `/D=...`, `/NS`.
    HeadlessInstall(InstallPlan),
    /// Headless uninstallation triggered by `/UNINSTALL`.
    HeadlessUninstall(UninstallPlan),
}

/// Extract the argument substring from a raw Windows command line (`GetCommandLineW`),
/// stripping the leading program name (quoted or unquoted) and any separating spaces/tabs.
pub fn command_line_args(command_line: &str) -> &str {
    let rest = match command_line.strip_prefix('"') {
        Some(quoted) => quoted.find('"').map_or("", |end| &quoted[end + 1..]),
        None => command_line
            .find([' ', '\t'])
            .map_or("", |end| &command_line[end..]),
    };
    rest.trim_start_matches([' ', '\t'])
}

/// Normalize installer arguments for v1.1.0–v1.3.1 updater compatibility:
/// when `/UPDATE` and `/P` both appear as whole words outside quotes before `/D=`,
/// rewrite `/P` to `/S` so older updaters run silently without a passive progress window.
pub fn normalize_updater_args(args: &str) -> Cow<'_, str> {
    let (options_end, _) = find_dir_option_boundary(args);
    let options = &args[..options_end];
    let words = unquoted_words(options);
    let has = |flag: &str| words.iter().any(|(_, w)| w.eq_ignore_ascii_case(flag));
    if !has("/UPDATE") || !has("/P") {
        return Cow::Borrowed(args);
    }
    let mut rewritten = args.to_string();
    for (start, word) in words {
        if word.eq_ignore_ascii_case("/P") {
            rewritten.replace_range(start..start + word.len(), "/S");
        }
    }
    Cow::Owned(rewritten)
}

/// Parse NSIS-compatible Windows command-line arguments (everything after the executable name).
///
/// Follows NSIS `/D=` rules: `/D=<folder>` must be the final switch and consumes the rest of the
/// command line raw (so paths with spaces work unquoted, and flags appearing inside a `/D=` path
/// are not interpreted as options). Also applies the v1.1.0–v1.3.1 `/P` + `/UPDATE` -> silent rule.
pub fn parse_args(args: &str) -> CliArgs {
    let trimmed = args.trim_start_matches([' ', '\t']);
    if trimmed.is_empty() {
        return CliArgs::default();
    }

    // Apply v1.1.0-v1.3.1 backwards-compatibility rule (`/P` + `/UPDATE` -> `/S`) first.
    let normalized = normalize_updater_args(trimmed);
    let (options_end, raw_dir) = find_dir_option_boundary(&normalized);
    let options = &normalized[..options_end];

    let mut parsed = CliArgs {
        install_dir: raw_dir.and_then(clean_dir_arg),
        ..CliArgs::default()
    };

    for (_, word) in unquoted_words(options) {
        if word.eq_ignore_ascii_case("/S") {
            parsed.silent = true;
        } else if word.eq_ignore_ascii_case("/P") {
            parsed.passive = true;
        } else if word.eq_ignore_ascii_case("/R") {
            parsed.relaunch = true;
        } else if word.eq_ignore_ascii_case("/UPDATE") {
            parsed.update = true;
        } else if word.eq_ignore_ascii_case("/NS") {
            parsed.no_shortcuts = true;
        } else if word.eq_ignore_ascii_case("/UNINSTALL") {
            parsed.uninstall = true;
        } else if word.eq_ignore_ascii_case("/DELETE-DATA")
            || word.eq_ignore_ascii_case("/DELETEDATA")
            || word.eq_ignore_ascii_case("/PURGE")
        {
            parsed.delete_data = true;
        }
    }

    // `/S` always dominates `/P` when both are explicitly specified.
    if parsed.silent {
        parsed.passive = false;
    }

    parsed
}

/// Decide whether to open the interactive GUI or execute headlessly from raw arguments.
pub fn decide_launch(args: &str) -> LaunchMode {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return LaunchMode::Gui;
    }

    let parsed = parse_args(trimmed);
    if parsed.uninstall {
        return LaunchMode::HeadlessUninstall(UninstallPlan {
            silent: parsed.silent,
            delete_data: parsed.delete_data,
            install_dir: parsed.install_dir,
        });
    }

    let desktop_shortcut = if parsed.no_shortcuts {
        DesktopShortcutPolicy::Never
    } else if parsed.update {
        DesktopShortcutPolicy::UpdateIfExists
    } else {
        DesktopShortcutPolicy::Create
    };

    LaunchMode::HeadlessInstall(InstallPlan {
        silent: parsed.silent,
        passive: parsed.passive,
        update: parsed.update,
        // `/UPDATE` always relaunches UsageDeck after replacing the binary, even if `/R` was omitted.
        relaunch: parsed.relaunch || parsed.update,
        create_start_menu_shortcut: !parsed.no_shortcuts,
        desktop_shortcut,
        start_with_windows: None,
        install_dir: parsed.install_dir,
    })
}

/// Locate an unquoted `/D=` option and return `(options_end_byte, Some(raw_dir_slice))`.
fn find_dir_option_boundary(args: &str) -> (usize, Option<&str>) {
    let mut in_quotes = false;
    for (start, word) in split_words(args) {
        if !in_quotes
            && word
                .get(..3)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("/D="))
        {
            return (start, Some(&args[start + 3..]));
        }
        for b in word.bytes() {
            if b == b'"' {
                in_quotes = !in_quotes;
            }
        }
    }
    (args.len(), None)
}

fn clean_dir_arg(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed)
        .trim();
    if unquoted.is_empty() {
        None
    } else {
        Some(PathBuf::from(unquoted))
    }
}

/// Words split on ASCII space and tab outside double-quoted spans, paired with byte offsets.
fn unquoted_words(s: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut in_quotes = false;
    for (start, word) in split_words(s) {
        if !in_quotes && !word.is_empty() {
            words.push((start, word));
        }
        for b in word.bytes() {
            if b == b'"' {
                in_quotes = !in_quotes;
            }
        }
    }
    words
}

/// The words of `s` split at spaces and tabs, each with its starting byte offset.
fn split_words(s: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = 0usize;
    s.split([' ', '\t']).map(move |word| {
        let at = start;
        start += word.len() + 1;
        (at, word)
    })
}

/// Parsed semantic version (`major.minor.patch`, ignoring optional `v` prefix and `-pre`/`+build`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Parse `1.2.3`, optionally with a leading `v` or a `-pre`/`+build` suffix.
    pub fn parse(s: &str) -> Option<Version> {
        let core = s.trim().trim_start_matches(['v', 'V']);
        let core = core.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
        let version = Version {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next()??,
        };
        parts.next().is_none().then_some(version)
    }
}

/// Action the interactive installer presents based on any currently installed version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Install,
    Update,
    Reinstall,
    Downgrade,
}

impl Action {
    pub fn button_label(self) -> &'static str {
        match self {
            Action::Install => "Install UsageDeck",
            Action::Update => "Update UsageDeck",
            Action::Reinstall => "Reinstall UsageDeck",
            Action::Downgrade => "Downgrade UsageDeck",
        }
    }

    pub fn progress_label(self) -> &'static str {
        match self {
            Action::Install => "Installing UsageDeck…",
            Action::Update => "Updating UsageDeck…",
            Action::Reinstall => "Reinstalling UsageDeck…",
            Action::Downgrade => "Downgrading UsageDeck…",
        }
    }

    pub fn success_label(self) -> &'static str {
        match self {
            Action::Install => "UsageDeck is installed",
            Action::Update => "UsageDeck is updated",
            Action::Reinstall => "UsageDeck is reinstalled",
            Action::Downgrade => "UsageDeck is downgraded",
        }
    }
}

/// Decide the primary action from the currently installed `DisplayVersion` and this installer's version.
///
/// An installed copy whose version cannot be parsed is treated as older (`Action::Update`),
/// matching v1.x NSIS behavior.
pub fn action(installed: Option<&str>, this: Version) -> Action {
    let Some(installed) = installed.filter(|v| !v.trim().is_empty()) else {
        return Action::Install;
    };
    match Version::parse(installed).map(|v| v.cmp(&this)) {
        Some(Ordering::Equal) => Action::Reinstall,
        Some(Ordering::Greater) => Action::Downgrade,
        Some(Ordering::Less) | None => Action::Update,
    }
}

/// Resolve the target installation directory.
///
/// Precedence:
/// 1. Explicit `/D=<folder>` CLI argument
/// 2. Existing `InstallLocation` in the HKCU Uninstall registry key
/// 3. Default per-user folder: `%LOCALAPPDATA%\UsageDeck`
pub fn target_install_dir(
    cli_dir: Option<&Path>,
    registry_install_location: Option<&str>,
    local_app_data: &Path,
) -> PathBuf {
    if let Some(dir) = cli_dir.filter(|p| !p.as_os_str().is_empty()) {
        return dir.to_path_buf();
    }
    if let Some(dir) = registry_install_location
        .map(|l| l.trim().trim_matches('"').trim())
        .filter(|l| !l.is_empty())
    {
        return PathBuf::from(dir);
    }
    local_app_data.join(PRODUCT_NAME)
}

/// Resolve the installed app executable path from Uninstall registry values or default `%LOCALAPPDATA%`.
pub fn installed_exe(
    install_location: Option<&str>,
    main_binary_name: Option<&str>,
    local_app_data: &Path,
) -> PathBuf {
    let dir = target_install_dir(None, install_location, local_app_data);
    let exe = main_binary_name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(MAIN_BINARY_NAME);
    dir.join(exe)
}

/// Format the `UninstallString` registered under HKCU Uninstall.
pub fn uninstall_command(install_dir: &Path) -> String {
    format!(
        "\"{}\" /UNINSTALL",
        install_dir.join(UNINSTALLER_NAME).display()
    )
}

/// Format the `QuietUninstallString` registered under HKCU Uninstall.
pub fn quiet_uninstall_command(install_dir: &Path) -> String {
    format!(
        "\"{}\" /UNINSTALL /S",
        install_dir.join(UNINSTALLER_NAME).display()
    )
}

/// Format the `DisplayIcon` registry value pointing at the installed executable's primary icon.
pub fn display_icon_value(install_dir: &Path) -> String {
    format!("{},0", install_dir.join(MAIN_BINARY_NAME).display())
}

/// Read `startAtLogin` from `%APPDATA%\com.usagedeck.app\settings.json`; defaults to `true`.
pub fn start_at_login(settings_json: Option<&str>) -> bool {
    settings_json
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.get("startAtLogin").and_then(Value::as_bool))
        .unwrap_or(true)
}

/// Produce an updated `settings.json` with `startAtLogin` set to `on`, or `None` if no write is
/// needed (already matches `on`, or the existing file is not a valid JSON object).
pub fn settings_with_start_at_login(settings_json: Option<&str>, on: bool) -> Option<String> {
    let mut settings = match settings_json {
        None => Map::new(),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => map,
            _ => return None,
        },
    };
    if start_at_login(settings_json) == on {
        return None;
    }
    settings.insert("startAtLogin".into(), Value::Bool(on));
    serde_json::to_string_pretty(&settings).ok()
}

/// Format the value written under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\UsageDeck`.
pub fn run_entry(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Return `true` if a Windows Credential Manager generic credential target belongs to UsageDeck.
///
/// The `keyring` crate stores entries as `<user>.<service>` where `<service>` is `"UsageDeck"`.
pub fn is_usagedeck_credential_target(target: &str) -> bool {
    let trimmed = target.trim();
    if trimmed.eq_ignore_ascii_case(KEYRING_SERVICE) {
        return true;
    }
    let suffix = format!(".{KEYRING_SERVICE}");
    trimmed.len() > suffix.len()
        && trimmed
            .get(trimmed.len() - suffix.len()..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(&suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    const V200: Version = Version {
        major: 2,
        minor: 0,
        patch: 0,
    };

    #[test]
    fn older_updater_passive_flag_becomes_silent() {
        let cmd = r#""C:\Users\a b\AppData\Local\Temp\UsageDeck-1.4.0-setup.exe" /P /R /UPDATE"#;
        let args = command_line_args(cmd);
        assert_eq!(args, "/P /R /UPDATE");
        assert_eq!(normalize_updater_args(args), "/S /R /UPDATE");

        assert_eq!(
            decide_launch(args),
            LaunchMode::HeadlessInstall(InstallPlan {
                silent: true,
                passive: false,
                update: true,
                relaunch: true,
                create_start_menu_shortcut: true,
                desktop_shortcut: DesktopShortcutPolicy::UpdateIfExists,
                start_with_windows: None,
                install_dir: None,
            })
        );
    }

    #[test]
    fn passive_becomes_silent_only_when_update_is_outside_quotes_and_before_dir() {
        for (args, expected_norm) in [
            ("/P /R /UPDATE", "/S /R /UPDATE"),
            ("/P /UPDATE", "/S /UPDATE"),
            ("/UPDATE\t/p", "/UPDATE\t/S"),
            ("/P  /update /R", "/S  /update /R"),
            (
                r"/P /UPDATE /D=C:\Apps\UsageDeck",
                r"/S /UPDATE /D=C:\Apps\UsageDeck",
            ),
            ("/S /R /UPDATE", "/S /R /UPDATE"),
            ("/P", "/P"),
            ("/P /R", "/P /R"),
            ("/UPDATE", "/UPDATE"),
            ("/PASSIVE /UPDATE", "/PASSIVE /UPDATE"),
            (r"/S /D=C:\x /P /UPDATE", r"/S /D=C:\x /P /UPDATE"),
            (r"/P /D=C:\x /UPDATE y", r"/P /D=C:\x /UPDATE y"),
            (r"/D=C:\x /P /UPDATE", r"/D=C:\x /P /UPDATE"),
            (r#"/UPDATE /ARGS "a /P b""#, r#"/UPDATE /ARGS "a /P b""#),
            (r#"/ARGS "a b" /P /UPDATE"#, r#"/ARGS "a b" /S /UPDATE"#),
        ] {
            assert_eq!(normalize_updater_args(args), expected_norm, "{args}");
        }
    }

    #[test]
    fn v14_plus_silent_updater_flags_parse_cleanly() {
        let cmd = r#""C:\Users\me\AppData\Local\Temp\UsageDeck-Setup.exe" /S /R /UPDATE"#;
        let args = command_line_args(cmd);
        assert_eq!(args, "/S /R /UPDATE");
        assert_eq!(
            decide_launch(args),
            LaunchMode::HeadlessInstall(InstallPlan {
                silent: true,
                passive: false,
                update: true,
                relaunch: true,
                create_start_menu_shortcut: true,
                desktop_shortcut: DesktopShortcutPolicy::UpdateIfExists,
                start_with_windows: None,
                install_dir: None,
            })
        );
    }

    #[test]
    fn plain_start_opens_native_gui() {
        for cmd in [
            "UsageDeck-Setup.exe",
            "UsageDeck-Setup.exe   ",
            r#""C:\Downloads\UsageDeck-Setup.exe""#,
            r#""C:\Downloads\UsageDeck-Setup.exe" "#,
            r#""unterminated"#,
            "",
        ] {
            assert_eq!(
                decide_launch(command_line_args(cmd)),
                LaunchMode::Gui,
                "{cmd}"
            );
        }
    }

    #[test]
    fn scripted_install_flags_and_dir_with_spaces() {
        let args = command_line_args(r"setup.exe /S /NS /D=C:\Program Files\UsageDeck");
        assert_eq!(args, r"/S /NS /D=C:\Program Files\UsageDeck");
        assert_eq!(
            decide_launch(args),
            LaunchMode::HeadlessInstall(InstallPlan {
                silent: true,
                passive: false,
                update: false,
                relaunch: false,
                create_start_menu_shortcut: false,
                desktop_shortcut: DesktopShortcutPolicy::Never,
                start_with_windows: None,
                install_dir: Some(PathBuf::from(r"C:\Program Files\UsageDeck")),
            })
        );
    }

    #[test]
    fn quoted_dir_argument_is_unquoted() {
        let parsed = parse_args(r#"/S /R /D="D:\My Apps\UsageDeck""#);
        assert!(parsed.silent);
        assert!(parsed.relaunch);
        assert_eq!(
            parsed.install_dir,
            Some(PathBuf::from(r"D:\My Apps\UsageDeck"))
        );
    }

    #[test]
    fn flags_after_dir_belong_to_dir_path() {
        let parsed = parse_args(r"/P /D=C:\x /UPDATE /NS");
        assert!(parsed.passive);
        assert!(!parsed.silent);
        assert!(!parsed.update);
        assert!(!parsed.no_shortcuts);
        assert_eq!(parsed.install_dir, Some(PathBuf::from(r"C:\x /UPDATE /NS")));
    }

    #[test]
    fn update_alone_implies_relaunch() {
        assert_eq!(
            decide_launch("/UPDATE"),
            LaunchMode::HeadlessInstall(InstallPlan {
                silent: false,
                passive: false,
                update: true,
                relaunch: true,
                create_start_menu_shortcut: true,
                desktop_shortcut: DesktopShortcutPolicy::UpdateIfExists,
                start_with_windows: None,
                install_dir: None,
            })
        );
    }

    #[test]
    fn passive_without_update_stays_passive() {
        assert_eq!(
            decide_launch("/P /R"),
            LaunchMode::HeadlessInstall(InstallPlan {
                silent: false,
                passive: true,
                update: false,
                relaunch: true,
                create_start_menu_shortcut: true,
                desktop_shortcut: DesktopShortcutPolicy::Create,
                start_with_windows: None,
                install_dir: None,
            })
        );
    }

    #[test]
    fn silent_dominates_passive_if_both_passed() {
        let parsed = parse_args("/S /P");
        assert!(parsed.silent);
        assert!(!parsed.passive);
    }

    #[test]
    fn uninstall_flags_parse_with_optional_data_deletion() {
        assert_eq!(
            decide_launch("/UNINSTALL"),
            LaunchMode::HeadlessUninstall(UninstallPlan {
                silent: false,
                delete_data: false,
                install_dir: None,
            })
        );

        assert_eq!(
            decide_launch("/uninstall /s /delete-data"),
            LaunchMode::HeadlessUninstall(UninstallPlan {
                silent: true,
                delete_data: true,
                install_dir: None,
            })
        );

        assert_eq!(
            decide_launch(r"/UNINSTALL /S /PURGE /D=D:\Tools\UsageDeck"),
            LaunchMode::HeadlessUninstall(UninstallPlan {
                silent: true,
                delete_data: true,
                install_dir: Some(PathBuf::from(r"D:\Tools\UsageDeck")),
            })
        );
    }

    #[test]
    fn versions_parse_including_prerelease_and_v_prefix() {
        assert_eq!(Version::parse("2.0.0"), Some(V200));
        assert_eq!(Version::parse("v2.0.0"), Some(V200));
        assert_eq!(Version::parse("V2.0.0-alpha.1"), Some(V200));
        assert_eq!(Version::parse("2.0.0+build.9"), Some(V200));
        assert_eq!(Version::parse("2.0"), None);
        assert_eq!(Version::parse("2.0.0.1"), None);
        assert_eq!(Version::parse("nope"), None);
    }

    #[test]
    fn button_action_follows_installed_version() {
        assert_eq!(action(None, V200), Action::Install);
        assert_eq!(action(Some(""), V200), Action::Install);
        assert_eq!(action(Some("   "), V200), Action::Install);
        assert_eq!(action(Some("1.5.15"), V200), Action::Update);
        assert_eq!(action(Some("1.10.2"), V200), Action::Update);
        assert_eq!(action(Some("unparseable"), V200), Action::Update);
        assert_eq!(action(Some("2.0.0"), V200), Action::Reinstall);
        assert_eq!(action(Some("2.0.0-alpha.1"), V200), Action::Reinstall);
        assert_eq!(action(Some("2.1.0"), V200), Action::Downgrade);
    }

    #[test]
    fn target_install_dir_precedence() {
        let local = Path::new(r"C:\Users\alice\AppData\Local");
        assert_eq!(
            target_install_dir(
                Some(Path::new(r"E:\Custom")),
                Some(r#""D:\Existing""#),
                local
            ),
            PathBuf::from(r"E:\Custom")
        );
        assert_eq!(
            target_install_dir(None, Some(r#""D:\Existing""#), local),
            PathBuf::from(r"D:\Existing")
        );
        assert_eq!(
            target_install_dir(None, None, local),
            local.join("UsageDeck")
        );
        assert_eq!(
            target_install_dir(Some(Path::new("")), Some(r#""""#), local),
            local.join("UsageDeck")
        );
    }

    #[test]
    fn installed_exe_respects_uninstall_key_and_fallback() {
        let local = Path::new(r"C:\Users\a\AppData\Local");
        assert_eq!(
            installed_exe(Some(r#""D:\Apps\UsageDeck""#), Some("usagedeck.exe"), local),
            PathBuf::from(r"D:\Apps\UsageDeck").join("usagedeck.exe")
        );
        assert_eq!(
            installed_exe(None, None, local),
            local.join("UsageDeck").join("usagedeck.exe")
        );
    }

    #[test]
    fn uninstall_registry_strings_are_quoted_correctly() {
        let dir = Path::new(r"C:\Users\Jane Doe\AppData\Local\UsageDeck");
        let expected_uninst = dir.join(UNINSTALLER_NAME);
        let expected_exe = dir.join(MAIN_BINARY_NAME);
        assert_eq!(
            uninstall_command(dir),
            format!("\"{}\" /UNINSTALL", expected_uninst.display())
        );
        assert_eq!(
            quiet_uninstall_command(dir),
            format!("\"{}\" /UNINSTALL /S", expected_uninst.display())
        );
        assert_eq!(
            display_icon_value(dir),
            format!("{},0", expected_exe.display())
        );
    }

    #[test]
    fn start_at_login_and_settings_merge_preserve_keys() {
        assert!(start_at_login(None));
        assert!(start_at_login(Some("{}")));
        assert!(start_at_login(Some("not json")));
        assert!(!start_at_login(Some(r#"{"startAtLogin": false}"#)));

        assert_eq!(settings_with_start_at_login(None, true), None);
        assert_eq!(
            settings_with_start_at_login(Some(r#"{"startAtLogin":false}"#), false),
            None
        );

        let off: Value =
            serde_json::from_str(&settings_with_start_at_login(None, false).unwrap()).unwrap();
        assert_eq!(off, serde_json::json!({ "startAtLogin": false }));

        let existing = r#"{"themeMode":"dark","startAtLogin":false,"displayNames":{"a":"b"}}"#;
        let on = settings_with_start_at_login(Some(existing), true).unwrap();
        let keys: Vec<_> = serde_json::from_str::<Map<String, Value>>(&on)
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(
            keys,
            vec![
                ("themeMode".into(), Value::from("dark")),
                ("startAtLogin".into(), Value::Bool(true)),
                ("displayNames".into(), serde_json::json!({ "a": "b" })),
            ]
        );

        assert_eq!(settings_with_start_at_login(Some("{oops"), false), None);
        assert_eq!(settings_with_start_at_login(Some("[1]"), false), None);
    }

    #[test]
    fn run_entry_quotes_exe_path() {
        let exe = Path::new(r"C:\Users\a b\AppData\Local\UsageDeck\usagedeck.exe");
        assert_eq!(
            run_entry(exe),
            r#""C:\Users\a b\AppData\Local\UsageDeck\usagedeck.exe""#
        );
    }

    #[test]
    fn credential_target_matching_identifies_only_usagedeck_entries() {
        assert!(is_usagedeck_credential_target(
            "alice@example.com.UsageDeck"
        ));
        assert!(is_usagedeck_credential_target("bob.usagedeck"));
        assert!(is_usagedeck_credential_target("UsageDeck"));
        assert!(!is_usagedeck_credential_target(".UsageDeck"));
        assert!(!is_usagedeck_credential_target("other.App"));
        assert!(!is_usagedeck_credential_target("UsageDeckBackup"));
        assert!(!is_usagedeck_credential_target(""));
        assert!(!is_usagedeck_credential_target("🦀🦀🦀🦀"));
        assert!(is_usagedeck_credential_target("ユーザー.UsageDeck"));
    }

    #[test]
    fn non_ascii_tokens_do_not_panic_during_arg_parsing() {
        let parsed = parse_args("/S 🦀 /D=C:\\ユーザー\\UsageDeck");
        assert!(parsed.silent);
        assert_eq!(
            parsed.install_dir,
            Some(PathBuf::from(r"C:\ユーザー\UsageDeck"))
        );
    }
}
