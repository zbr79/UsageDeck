//! Self-updater: finds the newest GitHub release, downloads its `UsageDeck-Setup.exe`, checks it
//! against the size and SHA-256 GitHub reports, and offers to restart into the installer.
//!
//! The release workflow (.github/workflows/release.yml) uploads that installer to every release.
//! Background checks run 15 s after startup and then every 6 hours; the tray's "Check for
//! updates" runs one on demand. Only a manual check reports "up to date" or errors in a box;
//! background checks only log.

mod release;

use release::{is_newer, parse_release, verify, Release, UpdateError, Version};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::AppHandle;
use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    MB_YESNO, MESSAGEBOX_RESULT, MESSAGEBOX_STYLE,
};

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/zbr79/UsageDeck/releases/latest";
/// GitHub's API rejects requests without a User-Agent.
const USER_AGENT: &str = "UsageDeck-updater";
const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Flags read by Tauri's NSIS template (tauri-bundler 2.10.3, installer.nsi): `/P` is passive
/// mode (progress bar only; the Restart Manager closes a running UsageDeck without asking),
/// `/R` relaunches the app from `.onInstSuccess`, and `/UPDATE` skips the reinstall page's
/// uninstall step, so installer-hooks.nsh keeps the API keys and the Run entry.
const INSTALLER_ARGS: [&str; 3] = ["/P", "/R", "/UPDATE"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Trigger {
    Background,
    Manual,
}

/// A verified installer on disk, newer than the running app.
struct Ready {
    version: Version,
    path: PathBuf,
}

/// Set while a check, download or "restart now?" box is in progress, so a tray click during a
/// background check does not download or ask twice.
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Background checks for the life of the app.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            run(&app, Trigger::Background).await;
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

/// The tray's "Check for updates".
pub fn check_now(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { run(&app, Trigger::Manual).await });
}

async fn run(app: &AppHandle, trigger: Trigger) {
    if IN_FLIGHT.swap(true, Ordering::SeqCst) {
        log::info!("update check already in progress");
        return;
    }
    let v = &app.package_info().version;
    let current = Version {
        major: v.major,
        minor: v.minor,
        patch: v.patch,
    };
    match check(current).await {
        Ok(Some(ready)) => {
            log::info!(
                "update {} downloaded to {}",
                ready.version,
                ready.path.display()
            );
            let question = format!(
                "UsageDeck {} is ready to install. Restart now?",
                ready.version
            );
            if message_box(question, MB_YESNO | MB_ICONINFORMATION).await == IDYES {
                install(app, &ready.path).await;
            }
        }
        Ok(None) => {
            log::info!("update check: {current} is the latest version");
            if trigger == Trigger::Manual {
                let text = format!("You're on the latest version (UsageDeck {current}).");
                message_box(text, MB_OK | MB_ICONINFORMATION).await;
            }
        }
        Err(e) => {
            log::warn!("update check failed: {e}");
            if trigger == Trigger::Manual {
                let text = format!("Could not check for updates.\n\n{e}");
                message_box(text, MB_OK | MB_ICONWARNING).await;
            }
        }
    }
    IN_FLIGHT.store(false, Ordering::SeqCst);
}

async fn check(current: Version) -> Result<Option<Ready>, UpdateError> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(10 * 60))
        .build()?;
    let body = client
        .get(LATEST_RELEASE_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let release = parse_release(&body)?;
    if !is_newer(current, release.version) {
        return Ok(None);
    }
    let path = std::env::temp_dir().join(format!("UsageDeck-{}-setup.exe", release.version));
    // A copy left by an earlier check the user postponed is reused once it verifies again.
    let cached = std::fs::read(&path).is_ok_and(|bytes| verify(&bytes, &release).is_ok());
    if !cached {
        download(&client, &release, &path).await?;
    }
    Ok(Some(Ready {
        version: release.version,
        path,
    }))
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

/// Start the installer, then quit so it can replace the exe; it relaunches the app (`/R`).
async fn install(app: &AppHandle, installer: &Path) {
    if let Err(e) = std::process::Command::new(installer)
        .args(INSTALLER_ARGS)
        .spawn()
    {
        log::warn!("could not start {}: {e}", installer.display());
        let text = format!("Could not start the UsageDeck installer.\n\n{e}");
        message_box(text, MB_OK | MB_ICONWARNING).await;
        return;
    }
    log::info!("started {}, exiting for the update", installer.display());
    app.exit(0);
}

/// A native message box, shown on a blocking thread so the async runtime keeps running.
async fn message_box(text: String, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let shown = tauri::async_runtime::spawn_blocking(move || {
        let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let title: Vec<u16> = "UsageDeck".encode_utf16().chain([0]).collect();
        // SAFETY: both buffers are NUL-terminated UTF-16 and outlive the call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                style | MB_SETFOREGROUND | MB_TOPMOST,
            )
        }
    })
    .await;
    shown.unwrap_or_default()
}
