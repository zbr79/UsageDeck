//! Everything the installer reads from or does to Windows: the command line, the registry, known
//! folders, and the embedded NSIS installer.

use crate::decide::{self, Version};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::GUID;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Environment::GetCommandLineW;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_RoamingAppData, SHGetKnownFolderPath,
    KF_FLAG_DEFAULT,
};
use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

/// Tauri's NSIS installer for this same version, embedded by build.rs (empty in dev builds).
static NSIS_INSTALLER: &[u8] = include_bytes!(env!("USAGEDECK_NSIS_PAYLOAD"));

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The template's `UNINSTKEY` (installer.nsi), under HKCU for the app's currentUser install mode.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\UsageDeck";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
/// tauri-plugin-autostart names its Run value after the product name.
const RUN_VALUE: &str = "UsageDeck";
/// The app's data folder, `%APPDATA%\<identifier>` (src-tauri/tauri.conf.json).
const APP_DATA_FOLDER: &str = "com.usagedeck.app";
const WEBVIEW2_CLIENT: &str =
    r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

pub fn version() -> Version {
    Version::parse(VERSION).expect("CARGO_PKG_VERSION is x.y.z")
}

pub fn command_line() -> String {
    // SAFETY: GetCommandLineW returns a NUL-terminated string owned by the process.
    unsafe { GetCommandLineW().to_string() }.unwrap_or_default()
}

pub fn webview2_installed() -> bool {
    let pv = |root: &windows_registry::Key, path: String| {
        root.open(path).and_then(|k| k.get_string("pv")).ok()
    };
    decide::webview2_installed([
        pv(
            LOCAL_MACHINE,
            format!(r"SOFTWARE\WOW6432Node\{WEBVIEW2_CLIENT}"),
        ),
        pv(LOCAL_MACHINE, format!(r"SOFTWARE\{WEBVIEW2_CLIENT}")),
        pv(CURRENT_USER, format!(r"Software\{WEBVIEW2_CLIENT}")),
    ])
}

/// What the uninstall key says about an installed UsageDeck.
pub struct Installed {
    pub version: Option<String>,
    install_location: Option<String>,
    main_binary_name: Option<String>,
}

pub fn installed() -> Option<Installed> {
    let key = CURRENT_USER.open(UNINSTALL_KEY).ok()?;
    let value = |name: &str| key.get_string(name).ok();
    Some(Installed {
        version: value("DisplayVersion"),
        install_location: value("InstallLocation"),
        main_binary_name: value("MainBinaryName"),
    })
}

pub fn installed_exe() -> PathBuf {
    let found = installed();
    let found = found.as_ref();
    decide::installed_exe(
        found.and_then(|i| i.install_location.as_deref()),
        found.and_then(|i| i.main_binary_name.as_deref()),
        &known_folder(&FOLDERID_LocalAppData).unwrap_or_default(),
    )
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: on success the returned buffer is a NUL-terminated path the caller frees.
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0 as *const _));
        path.map(PathBuf::from)
    }
}

/// The shortcut the template creates (`$DESKTOP\${PRODUCTNAME}.lnk`).
pub fn desktop_shortcut() -> Option<PathBuf> {
    known_folder(&FOLDERID_Desktop).map(|d| d.join("UsageDeck.lnk"))
}

fn settings_file() -> Option<PathBuf> {
    known_folder(&FOLDERID_RoamingAppData).map(|d| d.join(APP_DATA_FOLDER).join("settings.json"))
}

pub fn start_at_login() -> bool {
    let text = settings_file().and_then(|f| std::fs::read_to_string(f).ok());
    decide::start_at_login(text.as_deref())
}

/// Make the app start with Windows or not, now and from its next launch. The app re-applies its
/// `startAtLogin` setting to the Run key at every start (src-tauri/src/lib.rs
/// `set_start_at_login`), so the setting is what lasts; the Run value is written too so the
/// choice holds even if the app is not opened before the next sign-in.
pub fn set_start_with_windows(on: bool, exe: &Path) -> std::io::Result<()> {
    let file = settings_file().ok_or_else(|| std::io::Error::other("no AppData folder"))?;
    let current = std::fs::read_to_string(&file).ok();
    if let Some(next) = decide::settings_with_start_at_login(current.as_deref(), on) {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = file.with_extension("json.tmp");
        std::fs::write(&partial, next)?;
        std::fs::rename(&partial, &file)?;
    }
    if on {
        CURRENT_USER
            .create(RUN_KEY)?
            .set_string(RUN_VALUE, decide::run_entry(exe))?;
    } else {
        for key in [RUN_KEY, STARTUP_APPROVED_KEY] {
            if let Ok(key) = CURRENT_USER.options().write().open(key) {
                let _ = key.remove_value(RUN_VALUE);
            }
        }
    }
    Ok(())
}

/// Write the embedded NSIS installer to %TEMP%, run it with `args` (passed through verbatim) and
/// wait for its exit code.
pub fn run_nsis(args: &str) -> std::io::Result<i32> {
    if NSIS_INSTALLER.is_empty() {
        return Err(std::io::Error::other(
            "this build has no installer inside (built without USAGEDECK_NSIS_PATH)",
        ));
    }
    let path = std::env::temp_dir().join(format!(
        "UsageDeck-{VERSION}-nsis-{}.exe",
        std::process::id()
    ));
    std::fs::write(&path, NSIS_INSTALLER)?;
    let mut command = Command::new(&path);
    if !args.is_empty() {
        command.raw_arg(args);
    }
    let status = command.status();
    let _ = std::fs::remove_file(&path);
    // NSIS exits 0 on success, 1 when cancelled, 2 when the script aborts.
    Ok(status?.code().unwrap_or(1))
}
