//! Decisions the installer makes from what it reads off the system, kept free of Windows APIs so
//! they run as unit tests anywhere.

use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// What a start of `UsageDeck-Setup.exe` does.
#[derive(Debug, PartialEq, Eq)]
pub enum Launch<'a> {
    /// Show the custom installer window.
    CustomUi,
    /// Run the embedded NSIS installer with its own wizard (it installs WebView2 when missing).
    NsisUi,
    /// Run the embedded NSIS installer with exactly these arguments and no window of our own.
    PassThrough(&'a str),
}

/// Any argument means an NSIS-style caller: the v1.1.0 and v1.2.0 updaters run the downloaded
/// `UsageDeck-Setup.exe` with `/P /R /UPDATE` (src-tauri/src/updater.rs INSTALLER_ARGS), and
/// scripted installs pass `/S`, `/D=...` and the like. Those must behave exactly as the bare NSIS
/// installer did, so the custom window only opens on a plain double-click. Without WebView2 the
/// window cannot render, and the NSIS wizard installs WebView2 itself.
pub fn launch(args: &str, webview2_installed: bool) -> Launch<'_> {
    let args = args.trim_start();
    if !args.is_empty() {
        Launch::PassThrough(args)
    } else if webview2_installed {
        Launch::CustomUi
    } else {
        Launch::NsisUi
    }
}

/// The arguments in a Windows command line (`GetCommandLineW`), exactly as typed: everything
/// after the program name. The program name follows its own rule (quotes delimit it, no
/// escapes), unlike the arguments, which NSIS reads raw (`/D=` must be last and unquoted).
pub fn command_line_args(command_line: &str) -> &str {
    let rest = match command_line.strip_prefix('"') {
        Some(quoted) => quoted.find('"').map_or("", |end| &quoted[end + 1..]),
        None => command_line
            .find([' ', '\t'])
            .map_or("", |end| &command_line[end..]),
    };
    rest.trim_start_matches([' ', '\t'])
}

/// WebView2's documented install check: the runtime's `pv` value under one of its EdgeUpdate
/// client keys is present and not `0.0.0.0`.
/// https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#detect-if-a-webview2-runtime-is-already-installed
pub fn webview2_installed<S: AsRef<str>>(pv_values: impl IntoIterator<Item = Option<S>>) -> bool {
    pv_values.into_iter().flatten().any(|pv| {
        let pv = pv.as_ref().trim();
        !pv.is_empty() && pv != "0.0.0.0"
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// `1.2.3`, optionally with a `v` prefix or a `-pre`/`+build` suffix (ignored).
    pub fn parse(s: &str) -> Option<Version> {
        let core = s.trim().trim_start_matches('v');
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

/// What the big button does, given the version already installed (the uninstall key's
/// `DisplayVersion`) and the version this installer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Install,
    Update,
    Reinstall,
    Downgrade,
}

/// An installed copy whose version cannot be read counts as older, as in the NSIS template's
/// reinstall page ("older or unknown version installed").
pub fn action(installed: Option<&str>, this: Version) -> Action {
    let Some(installed) = installed.filter(|v| !v.trim().is_empty()) else {
        return Action::Install;
    };
    match Version::parse(installed).map(|v| v.cmp(&this)) {
        Some(std::cmp::Ordering::Equal) => Action::Reinstall,
        Some(std::cmp::Ordering::Greater) => Action::Downgrade,
        Some(std::cmp::Ordering::Less) | None => Action::Update,
    }
}

/// The installed app's exe, from the uninstall key the NSIS template writes
/// (`InstallLocation` is stored in quotes, `MainBinaryName` with its `.exe`), falling back to the
/// template's currentUser default `$LOCALAPPDATA\${PRODUCTNAME}\${MAINBINARYNAME}.exe`.
pub fn installed_exe(
    install_location: Option<&str>,
    main_binary_name: Option<&str>,
    local_app_data: &Path,
) -> PathBuf {
    let dir = install_location
        .map(|l| l.trim().trim_matches('"'))
        .filter(|l| !l.is_empty())
        .map_or_else(|| local_app_data.join("UsageDeck"), PathBuf::from);
    let exe = main_binary_name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("usagedeck.exe");
    dir.join(exe)
}

/// The app's `startAtLogin` setting as written in its settings.json; absent means on (the
/// app's default, store.rs `Settings::default`).
pub fn start_at_login(settings_json: Option<&str>) -> bool {
    settings_json
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.get("startAtLogin").and_then(Value::as_bool))
        .unwrap_or(true)
}

/// The settings.json to write so the app's `startAtLogin` becomes `on`, or `None` when nothing
/// should be written: the setting already says so, or the file is not a JSON object (left for
/// the app, which falls back to its defaults, rather than overwritten).
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

/// The value tauri-plugin-autostart writes under `HKCU\...\Run\UsageDeck`: auto-launch 0.6
/// formats `"{app_path} {args}"` with the unquoted exe path and the app passes no args, so the
/// app's own `enable()` at startup rewrites the same string.
pub fn run_entry(exe: &Path) -> String {
    format!("{} ", exe.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    const V130: Version = Version {
        major: 1,
        minor: 3,
        patch: 0,
    };

    #[test]
    fn updater_flags_pass_through_untouched() {
        let cmd = r#""C:\Users\a b\AppData\Local\Temp\UsageDeck-1.3.0-setup.exe" /P /R /UPDATE"#;
        let args = command_line_args(cmd);
        assert_eq!(args, "/P /R /UPDATE");
        assert_eq!(launch(args, true), Launch::PassThrough("/P /R /UPDATE"));
        assert_eq!(launch(args, false), Launch::PassThrough("/P /R /UPDATE"));
    }

    #[test]
    fn scripted_install_flags_pass_through_verbatim() {
        for (cmd, args) in [
            ("UsageDeck-Setup.exe /S", "/S"),
            ("UsageDeck-Setup.exe\t/S /NS", "/S /NS"),
            (
                r"setup.exe /S /D=C:\Program Files\UsageDeck",
                r"/S /D=C:\Program Files\UsageDeck",
            ),
            (r#""setup.exe"/S"#, "/S"),
            (r#""C:\x\setup.exe"   /ARGS "--a b""#, r#"/ARGS "--a b""#),
        ] {
            assert_eq!(command_line_args(cmd), args, "{cmd}");
            assert_eq!(
                launch(command_line_args(cmd), true),
                Launch::PassThrough(args)
            );
        }
    }

    #[test]
    fn plain_start_opens_the_window_or_falls_back_without_webview2() {
        for cmd in [
            "UsageDeck-Setup.exe",
            "UsageDeck-Setup.exe   ",
            r#""C:\Downloads\UsageDeck-Setup.exe""#,
            r#""C:\Downloads\UsageDeck-Setup.exe" "#,
            r#""unterminated"#,
            "",
        ] {
            assert_eq!(
                launch(command_line_args(cmd), true),
                Launch::CustomUi,
                "{cmd}"
            );
            assert_eq!(
                launch(command_line_args(cmd), false),
                Launch::NsisUi,
                "{cmd}"
            );
        }
    }

    #[test]
    fn webview2_counts_only_a_real_version() {
        assert!(webview2_installed([None, Some("131.0.2903.70"), None]));
        assert!(webview2_installed([Some("0.0.0.0"), None, Some("120.0.1")]));
        assert!(!webview2_installed([None::<&str>, None, None]));
        assert!(!webview2_installed([Some("0.0.0.0"), Some(""), Some("  ")]));
    }

    #[test]
    fn versions_parse_like_display_version() {
        assert_eq!(Version::parse("1.3.0"), Some(V130));
        assert_eq!(Version::parse("v1.3.0"), Some(V130));
        assert_eq!(Version::parse("1.3.0-beta.1"), Some(V130));
        assert_eq!(Version::parse("1.3"), None);
        assert_eq!(Version::parse("1.3.0.4"), None);
        assert_eq!(Version::parse("one"), None);
    }

    #[test]
    fn button_action_follows_the_installed_version() {
        assert_eq!(action(None, V130), Action::Install);
        assert_eq!(action(Some(""), V130), Action::Install);
        assert_eq!(action(Some("1.2.0"), V130), Action::Update);
        assert_eq!(action(Some("1.2.10"), V130), Action::Update);
        assert_eq!(action(Some("garbage"), V130), Action::Update);
        assert_eq!(action(Some("1.3.0"), V130), Action::Reinstall);
        assert_eq!(action(Some("1.10.0"), V130), Action::Downgrade);
    }

    #[test]
    fn installed_exe_comes_from_the_uninstall_key() {
        let local = Path::new(r"C:\Users\a\AppData\Local");
        assert_eq!(
            installed_exe(Some(r#""D:\Apps\UsageDeck""#), Some("usagedeck.exe"), local),
            Path::new(r"D:\Apps\UsageDeck").join("usagedeck.exe")
        );
        assert_eq!(
            installed_exe(None, None, local),
            local.join("UsageDeck").join("usagedeck.exe")
        );
        assert_eq!(
            installed_exe(Some(r#""""#), Some(""), local),
            local.join("UsageDeck").join("usagedeck.exe")
        );
    }

    #[test]
    fn start_at_login_defaults_on() {
        assert!(start_at_login(None));
        assert!(start_at_login(Some("{}")));
        assert!(start_at_login(Some("not json")));
        assert!(!start_at_login(Some(r#"{"startAtLogin": false}"#)));
    }

    #[test]
    fn settings_write_only_changes_start_at_login() {
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
    }

    #[test]
    fn unreadable_settings_are_left_alone() {
        assert_eq!(settings_with_start_at_login(Some("{oops"), false), None);
        assert_eq!(settings_with_start_at_login(Some("[1]"), false), None);
    }

    #[test]
    fn run_entry_matches_the_autostart_plugin() {
        let exe = Path::new(r"C:\Users\a b\AppData\Local\UsageDeck\usagedeck.exe");
        assert_eq!(
            run_entry(exe),
            r"C:\Users\a b\AppData\Local\UsageDeck\usagedeck.exe "
        );
    }
}
