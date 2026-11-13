//! Self-updater: finds the newest GitHub release, downloads its `UsageDeck-Setup.exe`, checks it
//! against the size and SHA-256 GitHub reports, and reports what it is doing as an
//! [`UpdateStatus`] the settings window renders (event `update-status`).
//!
//! The release workflow (.github/workflows/release.yml) uploads that installer to every release.
//! Background checks run 15 s after startup and then every hour; "Check for updates" in the tray
//! and the settings window runs one on demand. Installing runs the installer
//! silently and quits; the installer starts the new version. With "Install updates
//! automatically" on, a ready update is installed the same way once the user is away.

mod release;
mod status;

use release::{is_newer, parse_release, verify, Release, UpdateError, Version};
use status::{decide_auto_install, AutoInstall, Machine, Outcome, Trigger};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

pub use status::UpdateStatus;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/zbr79/UsageDeck/releases/latest";
/// GitHub's API rejects requests without a User-Agent.
const USER_AGENT: &str = "UsageDeck-updater";
const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
const API_TIMEOUT: Duration = Duration::from_secs(30);
const AUTO_INSTALL_POLL: Duration = Duration::from_secs(30);
pub const EVENT_STATUS: &str = "update-status";
/// Flags read by Tauri's NSIS template (tauri-bundler 2.10.3): `/S` is NSIS's silent mode, in
/// which a running UsageDeck is closed without asking (utils.nsh CheckIfAppIsRunning:
/// `IfSilent kill_...`) and `/R` relaunches the app (installer.nsi .onInstSuccess runs it for
/// `${Silent}` as well as passive mode). `/UPDATE` keeps the API keys and the Run entry
/// (installer-hooks.nsh). Installers from 1.4.0 on pass these straight to NSIS; the installer
/// window crate turns the `/P` of older apps into `/S` (installer/src/decide.rs).
const INSTALLER_ARGS: [&str; 3] = ["/S", "/R", "/UPDATE"];

/// The updater's shared state, managed by Tauri.
pub struct Updater {
    machine: Mutex<Machine>,
    /// the auto-install watcher's last decision; None before its first look
    auto_install: Mutex<Option<AutoInstall>>,
}

impl Updater {
    /// Built before any window exists, so a page's first `get_update_status` always finds it.
    pub fn new(current_version: String) -> Self {
        Self {
            machine: Mutex::new(Machine::new(current_version)),
            auto_install: Mutex::new(None),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.lock().status().clone()
    }

    /// The auto-install line of the diagnostics report.
    pub fn auto_install_report(&self) -> String {
        let last = self
            .auto_install
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*last {
            Some(decision) => format!("Auto-install: {decision}"),
            None => "Auto-install: not decided yet".into(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Machine> {
        self.machine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Record the watcher's decision, logging it when it changed.
    fn decided(&self, decision: &AutoInstall) {
        let mut last = self
            .auto_install
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.as_ref() != Some(decision) {
            log::info!("auto-install: {decision}");
            *last = Some(decision.clone());
        }
    }
}

/// Apply `change` and tell every window when the status changed. Returns what `change` returned.
/// The event goes out under the lock, so windows get the changes in the order they happened.
fn update<T>(app: &AppHandle, change: impl FnOnce(&mut Machine) -> T) -> T {
    let updater = app.state::<Updater>();
    let mut machine = updater.lock();
    let before = machine.status().clone();
    let result = change(&mut machine);
    if *machine.status() != before {
        if let Err(e) = app.emit(EVENT_STATUS, machine.status()) {
            log::warn!("could not send the update status: {e}");
        }
    }
    result
}

/// Background checks for the life of the app.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if update(&app, |m| m.begin(Trigger::Background)) {
                run(&app).await;
            } else {
                log::info!("update check already in progress");
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

/// Every 30 s for the life of the app: install the ready update when "Install updates
/// automatically" is on and the settings window is closed. A failed start leaves the Ready phase,
/// so it is not retried until a check finds the update again.
pub fn watch_auto_install(app: AppHandle, dev_build: bool) {
    tauri::async_runtime::spawn(async move {
        loop {
            let setting_on = app
                .try_state::<crate::Core>()
                .is_some_and(|core| core.store.get_settings().auto_install_updates);
            let ready = app
                .state::<Updater>()
                .lock()
                .ready_version()
                .map(str::to_string);
            let decision = decide_auto_install(
                setting_on,
                ready.as_deref(),
                app.get_webview_window(crate::SETTINGS).is_some(),
                dev_build,
            );
            app.state::<Updater>().decided(&decision);
            if matches!(decision, AutoInstall::Install { .. }) {
                // on success the app exits; a failure is already recorded as the status
                let _ = install_update(app.clone());
            }
            tokio::time::sleep(AUTO_INSTALL_POLL).await;
        }
    });
}

/// "Check for updates" from the tray or the settings window. The status changes before this
/// returns and is handed back, so the caller can show "Checking…" without waiting for the event.
pub fn check_now(app: &AppHandle) -> UpdateStatus {
    log::info!("manual update check requested");
    if update(app, |m| m.begin(Trigger::Manual)) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { run(&app).await });
    } else {
        log::info!("update check already in progress: it now reports as this manual check");
    }
    app.state::<Updater>().status()
}

/// The check `begin` allowed: ask GitHub, download when newer, record how it ended.
async fn run(app: &AppHandle) {
    let v = &app.package_info().version;
    let current = Version {
        major: v.major,
        minor: v.minor,
        patch: v.patch,
    };
    let on_release = |version: Version| update(app, |m| m.saw_release(version.to_string()));
    let on_download = |version: Version| update(app, |m| m.downloading(version.to_string()));
    let outcome = match check(current, on_release, on_download).await {
        Ok(Some((version, installer))) => {
            log::info!("update {version} downloaded to {}", installer.display());
            Outcome::Ready {
                version: version.to_string(),
                installer,
            }
        }
        Ok(None) => {
            log::info!("update check: {current} is the latest version");
            Outcome::UpToDate
        }
        Err(e) => {
            log::warn!("update check failed: {e}");
            Outcome::Failed(e.to_string())
        }
    };
    update(app, |m| m.finish(outcome, now_ms()));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The newest release's verified installer, when it is newer than `current`.
async fn check(
    current: Version,
    on_release: impl FnOnce(Version),
    on_download: impl FnOnce(Version),
) -> Result<Option<(Version, PathBuf)>, UpdateError> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(10 * 60))
        .build()?;
    let body = client
        .get(LATEST_RELEASE_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        // its own limit: the client's 10 minutes are for the download
        .timeout(API_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let release = parse_release(&body)?;
    on_release(release.version);
    if !is_newer(current, release.version) {
        return Ok(None);
    }
    let path = std::env::temp_dir().join(format!("UsageDeck-{}-setup.exe", release.version));
    // A copy left by an earlier check the user postponed is reused once it verifies again.
    let cached = std::fs::read(&path).is_ok_and(|bytes| verify(&bytes, &release).is_ok());
    if !cached {
        on_download(release.version);
        download(&client, &release, &path).await?;
    }
    Ok(Some((release.version, path)))
}

async fn download(
    client: &reqwest::Client,
    release: &Release,
    path: &Path,
) -> Result<(), UpdateError> {
    let bytes = client
        .get(&release.asset_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    verify(&bytes, release)?;
    let partial = path.with_extension("part");
    std::fs::write(&partial, &bytes)?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

// ---- commands -------------------------------------------------------------------------------

#[tauri::command]
pub fn get_update_status(updater: tauri::State<'_, Updater>) -> UpdateStatus {
    updater.status()
}

/// Returns the status the check starts with ("checking", or the ready update shown again).
#[tauri::command]
pub fn check_for_updates(app: AppHandle) -> UpdateStatus {
    check_now(&app)
}

/// Start the ready installer silently, then quit so it can replace the exe; it relaunches the
/// app (`/R`).
#[tauri::command]
pub fn install_update(app: AppHandle) -> Result<(), String> {
    let Some(installer) = app
        .state::<Updater>()
        .lock()
        .installer()
        .map(Path::to_path_buf)
    else {
        return Err("no update is ready to install".into());
    };
    if let Err(e) = std::process::Command::new(&installer)
        .args(INSTALLER_ARGS)
        .spawn()
    {
        log::warn!("could not start {}: {e}", installer.display());
        let message = format!("Could not start the installer: {e}");
        update(&app, |m| m.install_failed(message.clone()));
        return Err(message);
    }
    log::info!("started {}, exiting for the update", installer.display());
    app.exit(0);
    Ok(())
}
