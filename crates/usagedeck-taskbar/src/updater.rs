//! Pure-Rust background auto-updater.
//!
//! Queries GitHub's latest release 15 s after start and every hour thereafter (plus manual
//! checks on demand), verifies the `UsageDeck-Setup.exe` asset size and SHA-256 digest using
//! `usagedeck_core::updater::{release, status}`, downloads it to `%TEMP%`, and launches it
//! silently with `/S /R /UPDATE` before exiting so the installer can replace the executable and
//! relaunch UsageDeck.
//!
//! Safety rules (parity with v1):
//! - Debug and Cargo-target builds never run background checks or auto-install themselves
//!   (manual `check_now` still works).
//! - Auto-install waits while `pause_auto_install` is `true` (e.g., while the Settings dialog is
//!   open) and proceeds once it clears.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{self, UnboundedSender};
use usagedeck_core::updater::release::{
    is_newer, parse_release, verify, Release, UpdateError, Version,
};
use usagedeck_core::updater::status::{decide_auto_install, Machine, Outcome};

pub use usagedeck_core::updater::status::{AutoInstall, Phase, Trigger, UpdateStatus};

pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/zbr79/UsageDeck/releases/latest";

/// GitHub's API rejects requests without a User-Agent.
const USER_AGENT: &str = "UsageDeck-updater";

const FIRST_CHECK: Duration = Duration::from_secs(8);
const CHECK_INTERVAL: Duration = Duration::from_secs(2 * 60);
const API_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const AUTO_INSTALL_POLL: Duration = Duration::from_secs(5);

/// Silent NSIS flags: `/S` silent mode (closes running UsageDeck without prompting), `/R`
/// relaunches the app after install, `/UPDATE` preserves API keys and the Run entry.
pub const INSTALLER_ARGS: [&str; 3] = ["/S", "/R", "/UPDATE"];

/// Callback invoked whenever [`UpdateStatus`] changes.
pub type StatusCallback = Arc<dyn Fn(&UpdateStatus) + Send + Sync>;

/// Callback invoked to exit the application once the verified installer has been spawned.
/// Defaults to `std::process::exit(0)` when not provided.
pub type ExitCallback = Arc<dyn Fn() + Send + Sync>;

/// Configuration for [`start_updater_with_options`].
pub struct UpdaterOptions {
    pub current_version: String,
    pub dev_build: bool,
    /// Shared flag set to `true` while Settings is open so auto-install does not close it.
    pub pause_auto_install: Arc<AtomicBool>,
    pub on_status_change: Option<StatusCallback>,
    pub on_exit_for_update: Option<ExitCallback>,
}

impl Default for UpdaterOptions {
    fn default() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            dev_build: crate::autostart::is_dev_build(),
            pause_auto_install: Arc::new(AtomicBool::new(false)),
            on_status_change: None,
            on_exit_for_update: None,
        }
    }
}

enum UpdaterCmd {
    RunCheck,
    Stop,
}

struct UpdaterInner {
    machine: Mutex<Machine>,
    auto_install: Mutex<Option<AutoInstall>>,
    last_etag: Mutex<Option<String>>,
    pause_auto_install: Arc<AtomicBool>,
    installing: AtomicBool,
    current_version: Version,
    dev_build: bool,
    cmd_tx: UnboundedSender<UpdaterCmd>,
    on_status_change: Option<StatusCallback>,
    on_exit_for_update: Option<ExitCallback>,
}

impl UpdaterInner {
    fn lock_machine(&self) -> MutexGuard<'_, Machine> {
        self.machine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn status(&self) -> UpdateStatus {
        self.lock_machine().status().clone()
    }

    /// Apply `change` to the state machine and notify `on_status_change` (outside the lock)
    /// whenever the serialized [`UpdateStatus`] changed.
    fn update<T>(&self, change: impl FnOnce(&mut Machine) -> T) -> T {
        let (result, changed) = {
            let mut machine = self.lock_machine();
            let before = machine.status().clone();
            let result = change(&mut machine);
            let after = machine.status().clone();
            let changed = (after != before).then_some(after);
            (result, changed)
        };
        if let (Some(status), Some(cb)) = (changed.as_ref(), self.on_status_change.as_ref()) {
            cb(status);
        }
        result
    }

    /// Record the auto-install watcher's latest decision, logging when it changes.
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

    #[cfg(test)]
    fn auto_install_report(&self) -> String {
        let last = self
            .auto_install
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*last {
            Some(decision) => format!("Auto-install: {decision}"),
            None => "Auto-install: not decided yet".into(),
        }
    }

    fn poll_auto_install(&self) {
        let ready = self.lock_machine().ready_version().map(str::to_string);
        let paused = self.pause_auto_install.load(Ordering::Acquire);
        let decision = decide_auto_install(ready.as_deref(), paused, self.dev_build);
        self.decided(&decision);
        if matches!(decision, AutoInstall::Install { .. }) {
            // On success the app exits; a failure is already recorded on the state machine.
            let _ = self.install_now();
        }
    }

    fn install_now(&self) -> Result<(), String> {
        let Some(installer) = self.lock_machine().installer().map(Path::to_path_buf) else {
            return Err("no update is ready to install".into());
        };
        if self
            .installing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        if let Err(e) = std::process::Command::new(&installer)
            .args(INSTALLER_ARGS)
            .spawn()
        {
            self.installing.store(false, Ordering::Release);
            log::warn!("could not start {}: {e}", installer.display());
            let message = format!("Could not start the installer: {e}");
            self.update(|m| m.install_failed(message.clone()));
            return Err(message);
        }
        log::info!("started {}, exiting for the update", installer.display());
        if let Some(exit_cb) = &self.on_exit_for_update {
            exit_cb();
        } else {
            std::process::exit(0);
        }
        Ok(())
    }
}

/// Cloneable handle to the running background updater.
#[derive(Clone)]
pub struct UpdaterHandle {
    inner: Arc<UpdaterInner>,
}

impl fmt::Debug for UpdaterHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdaterHandle")
            .field("current_version", &self.inner.current_version)
            .field("dev_build", &self.inner.dev_build)
            .field(
                "pause_auto_install",
                &self.inner.pause_auto_install.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl UpdaterHandle {
    /// Trigger an immediate manual update check.
    ///
    /// Transitions to [`Phase::Checking`] (unless an update is already [`Phase::Ready`]) before
    /// returning so UI callers can render immediate feedback without waiting for a callback.
    pub fn check_now(&self) -> UpdateStatus {
        log::info!("manual update check requested");
        if self.inner.update(|m| m.begin(Trigger::Manual)) {
            if self.inner.cmd_tx.send(UpdaterCmd::RunCheck).is_err() {
                log::warn!("updater background thread is not running");
                self.inner.update(|m| {
                    m.finish(
                        Outcome::Failed("updater background task is not running".into()),
                        now_ms(),
                    )
                });
            }
        } else {
            log::info!("update check already in progress: it now reports as this manual check");
        }
        self.inner.status()
    }

    /// Version string of a downloaded, verified update waiting to install, if any.
    pub fn ready_version(&self) -> Option<String> {
        self.inner
            .lock_machine()
            .ready_version()
            .map(str::to_string)
    }

    /// Launch the downloaded verified update installer immediately and exit for restart.
    pub fn install_now(&self) -> Result<(), String> {
        self.inner.install_now()
    }

    /// Pause or resume automatic installation (set `true` while Settings is open).
    pub fn set_pause_auto_install(&self, paused: bool) {
        self.inner
            .pause_auto_install
            .store(paused, Ordering::Release);
    }

    /// Stop the background updater thread cleanly.
    pub fn stop(&self) {
        let _ = self.inner.cmd_tx.send(UpdaterCmd::Stop);
    }
}

/// Start the background updater with custom options (pause flag, callbacks, or explicit version).
pub fn start_updater_with_options(options: UpdaterOptions) -> UpdaterHandle {
    let parsed_version = parse_current_version(&options.current_version).unwrap_or_else(|| {
        log::warn!(
            "could not parse current version {:?}, falling back to package version",
            options.current_version
        );
        package_version()
    });
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<UpdaterCmd>();
    let inner = Arc::new(UpdaterInner {
        machine: Mutex::new(Machine::new(options.current_version)),
        auto_install: Mutex::new(None),
        last_etag: Mutex::new(None),
        pause_auto_install: options.pause_auto_install,
        installing: AtomicBool::new(false),
        current_version: parsed_version,
        dev_build: options.dev_build,
        cmd_tx,
        on_status_change: options.on_status_change,
        on_exit_for_update: options.on_exit_for_update,
    });

    let thread_inner = Arc::clone(&inner);
    let _ = std::thread::Builder::new()
        .name("usagedeck-updater".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    log::error!("failed to build updater tokio runtime: {e}");
                    return;
                }
            };
            rt.block_on(async move {
                let bg_task = if thread_inner.dev_build {
                    log::info!("background update checks off (dev build)");
                    None
                } else {
                    let bg_inner = Arc::clone(&thread_inner);
                    Some(tokio::spawn(async move {
                        tokio::time::sleep(FIRST_CHECK).await;
                        loop {
                            if bg_inner.update(|m| m.begin(Trigger::Background)) {
                                run_check(&bg_inner).await;
                            } else {
                                log::info!("update check already in progress");
                            }
                            tokio::time::sleep(CHECK_INTERVAL).await;
                        }
                    }))
                };

                let auto_inner = Arc::clone(&thread_inner);
                let auto_task = tokio::spawn(async move {
                    loop {
                        auto_inner.poll_auto_install();
                        tokio::time::sleep(AUTO_INSTALL_POLL).await;
                    }
                });

                while let Some(cmd) = cmd_rx.recv().await {
                    match cmd {
                        UpdaterCmd::RunCheck => {
                            let check_inner = Arc::clone(&thread_inner);
                            tokio::spawn(async move {
                                run_check(&check_inner).await;
                            });
                        }
                        UpdaterCmd::Stop => break,
                    }
                }

                if let Some(task) = bg_task {
                    task.abort();
                }
                auto_task.abort();
            });
        });

    UpdaterHandle { inner }
}

/// Parse the running app's version (`"2.0.0"`, `"v2.0.0"`, or `"2.0.0-alpha.1"`) into a
/// numeric [`Version`] by stripping any leading `v` and any `-prerelease` / `+build` suffix.
pub fn parse_current_version(raw: &str) -> Option<Version> {
    let trimmed = raw.trim();
    let bare = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let core = bare.split(['-', '+']).next()?;
    core.parse::<Version>().ok()
}

fn package_version() -> Version {
    Version {
        major: env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(0),
        minor: env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(0),
        patch: env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or(0),
    }
}

/// Destination path in `%TEMP%` for a verified release installer.
pub fn installer_path_for(version: Version) -> PathBuf {
    installer_path_in(&std::env::temp_dir(), version)
}

/// Destination path in `dir` for a verified release installer.
pub fn installer_path_in(dir: &Path, version: Version) -> PathBuf {
    dir.join(format!("UsageDeck-{version}-setup.exe"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

enum CheckReply {
    NotModified,
    UpToDate {
        etag: Option<String>,
    },
    Ready {
        version: Version,
        installer: PathBuf,
        etag: Option<String>,
    },
}

async fn run_check(inner: &UpdaterInner) {
    let current = inner.current_version;
    let cached_etag = inner
        .last_etag
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let on_release = |version: Version| inner.update(|m| m.saw_release(version.to_string()));
    let on_download = |version: Version| inner.update(|m| m.downloading(version.to_string()));
    let mut ready = false;
    let outcome = match check_latest(current, cached_etag.as_deref(), on_release, on_download).await
    {
        Ok(CheckReply::Ready {
            version,
            installer,
            etag,
        }) => {
            if let Some(tag) = etag {
                *inner
                    .last_etag
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tag);
            }
            log::info!("update {version} downloaded to {}", installer.display());
            ready = true;
            Outcome::Ready {
                version: version.to_string(),
                installer,
            }
        }
        Ok(CheckReply::UpToDate { etag }) => {
            if let Some(tag) = etag {
                *inner
                    .last_etag
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tag);
            }
            log::info!("update check: {current} is the latest version");
            Outcome::UpToDate
        }
        Ok(CheckReply::NotModified) => {
            log::debug!("update check: 304 Not Modified ({current} is up to date)");
            Outcome::UpToDate
        }
        Err(e) => {
            log::warn!("update check failed: {e}");
            Outcome::Failed(e.to_string())
        }
    };
    inner.update(|m| m.finish(outcome, now_ms()));
    if ready {
        inner.poll_auto_install();
    }
}

async fn check_latest(
    current: Version,
    cached_etag: Option<&str>,
    on_release: impl FnOnce(Version),
    on_download: impl FnOnce(Version),
) -> Result<CheckReply, UpdateError> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .build()?;
    let mut req = client
        .get(LATEST_RELEASE_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        // Dedicated timeout for the metadata call; the client's 10 min limit is for the download.
        .timeout(API_TIMEOUT);
    if let Some(etag) = cached_etag {
        req = req.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let resp = req.send().await?;
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(CheckReply::NotModified);
    }
    let resp = resp.error_for_status()?;
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = resp.text().await?;
    let release = parse_release(&body)?;
    on_release(release.version);
    if !is_newer(current, release.version) {
        return Ok(CheckReply::UpToDate { etag });
    }
    let path = installer_path_for(release.version);
    // Reuse an already-downloaded installer only if it still passes size + SHA-256 verification.
    let cached = has_verified_cache(&path, &release);
    if !cached {
        on_download(release.version);
        download(&client, &release, &path).await?;
    }
    Ok(CheckReply::Ready {
        version: release.version,
        installer: path,
        etag,
    })
}

fn has_verified_cache(path: &Path, release: &Release) -> bool {
    std::fs::read(path).is_ok_and(|bytes| verify(&bytes, release).is_ok())
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
    write_verified_installer(&bytes, release, path)
}

/// Verify `bytes` against `release` (exact size + SHA-256) and write atomically via `.part`.
fn write_verified_installer(
    bytes: &[u8],
    release: &Release,
    path: &Path,
) -> Result<(), UpdateError> {
    verify(bytes, release)?;
    let partial = path.with_extension("part");
    if let Err(e) = std::fs::write(&partial, bytes) {
        let _ = std::fs::remove_file(&partial);
        return Err(UpdateError::Io(e));
    }
    if let Err(e) = std::fs::rename(&partial, path) {
        let _ = std::fs::remove_file(&partial);
        return Err(UpdateError::Io(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-updater-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_inner(
        dev_build: bool,
        pause_auto_install: Arc<AtomicBool>,
        on_status_change: Option<StatusCallback>,
    ) -> (UpdaterInner, mpsc::UnboundedReceiver<UpdaterCmd>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let inner = UpdaterInner {
            machine: Mutex::new(Machine::new("2.0.0".into())),
            auto_install: Mutex::new(None),
            last_etag: Mutex::new(None),
            pause_auto_install,
            installing: AtomicBool::new(false),
            current_version: v(2, 0, 0),
            dev_build,
            cmd_tx,
            on_status_change,
            on_exit_for_update: Some(Arc::new(|| {})),
        };
        (inner, cmd_rx)
    }

    #[test]
    fn parses_release_and_prerelease_current_versions() {
        assert_eq!(parse_current_version("2.0.0"), Some(v(2, 0, 0)));
        assert_eq!(parse_current_version("v1.5.15"), Some(v(1, 5, 15)));
        assert_eq!(parse_current_version("2.0.0-alpha.1"), Some(v(2, 0, 0)));
        assert_eq!(parse_current_version("v2.1.3+build.9"), Some(v(2, 1, 3)));
        assert_eq!(parse_current_version("not-a-version"), None);
        assert_eq!(parse_current_version("1.2"), None);
    }

    #[test]
    fn installer_path_uses_versioned_setup_name() {
        let p = installer_path_in(Path::new("/tmp"), v(2, 1, 0));
        assert_eq!(p, PathBuf::from("/tmp/UsageDeck-2.1.0-setup.exe"));
    }

    #[test]
    fn write_verified_installer_validates_digest_and_populates_cache() {
        let dir = temp_dir("verify");
        let bytes = b"fake-installer-payload-bytes";
        let release = Release {
            version: v(2, 1, 0),
            asset_url: "https://example.invalid/UsageDeck-Setup.exe".into(),
            size: bytes.len() as u64,
            sha256: Sha256::digest(bytes).into(),
        };
        let path = installer_path_in(&dir, release.version);

        // Corrupted payload must fail and leave no file behind.
        assert!(
            write_verified_installer(b"corrupted-installer-payload!", &release, &path).is_err()
        );
        assert!(!path.exists());
        assert!(!has_verified_cache(&path, &release));

        // Valid payload writes atomically and passes subsequent cache verification.
        write_verified_installer(bytes, &release, &path).unwrap();
        assert!(path.is_file());
        assert!(!path.with_extension("part").exists());
        assert!(has_verified_cache(&path, &release));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_callback_fires_only_on_actual_change_and_allows_reentrant_reads() {
        let events: Arc<Mutex<Vec<Phase>>> = Arc::new(Mutex::new(Vec::new()));
        let events_cb = Arc::clone(&events);
        let pause = Arc::new(AtomicBool::new(false));
        let (inner, _rx) = test_inner(
            true,
            pause,
            Some(Arc::new(move |status| {
                events_cb.lock().unwrap().push(status.phase.clone());
            })),
        );

        assert!(inner.update(|m| m.begin(Trigger::Background)));
        // Background begin stays Idle -> no callback fired.
        assert!(events.lock().unwrap().is_empty());

        // Manual begin while background check is running upgrades phase to Checking.
        assert!(!inner.update(|m| m.begin(Trigger::Manual)));
        assert_eq!(*events.lock().unwrap(), vec![Phase::Checking]);

        inner.update(|m| m.finish(Outcome::UpToDate, 100));
        assert_eq!(
            *events.lock().unwrap(),
            vec![Phase::Checking, Phase::UpToDate]
        );
    }

    #[test]
    fn pause_auto_install_blocks_auto_install_and_records_decision() {
        let pause = Arc::new(AtomicBool::new(true));
        let (inner, _rx) = test_inner(false, Arc::clone(&pause), None);
        assert!(inner.update(|m| m.begin(Trigger::Background)));
        inner.update(|m| {
            m.finish(
                Outcome::Ready {
                    version: "2.1.0".into(),
                    installer: PathBuf::from("nonexistent-installer.exe"),
                },
                500,
            )
        });

        inner.poll_auto_install();
        assert_eq!(
            inner.auto_install_report(),
            "Auto-install: waiting for the settings window to close to install 2.1.0"
        );
        // Installer was NOT invoked while paused; phase remains Ready.
        assert!(matches!(inner.status().phase, Phase::Ready { .. }));
    }

    #[test]
    fn dev_build_never_auto_installs() {
        let pause = Arc::new(AtomicBool::new(false));
        let (inner, _rx) = test_inner(true, pause, None);
        assert!(inner.update(|m| m.begin(Trigger::Manual)));
        inner.update(|m| {
            m.finish(
                Outcome::Ready {
                    version: "2.1.0".into(),
                    installer: PathBuf::from("nonexistent-installer.exe"),
                },
                500,
            )
        });

        inner.poll_auto_install();
        assert_eq!(
            inner.auto_install_report(),
            "Auto-install: never in a development build"
        );
        assert!(matches!(inner.status().phase, Phase::Ready { .. }));
    }

    #[test]
    fn install_now_fails_cleanly_when_nothing_is_ready() {
        let pause = Arc::new(AtomicBool::new(false));
        let (inner, _rx) = test_inner(false, pause, None);
        assert!(inner.install_now().is_err());
    }
}
