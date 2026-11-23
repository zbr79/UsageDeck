//! Background Tokio polling engine that refreshes all enabled providers and
//! Claude Code spend without blocking the Win32 UI thread.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use usagedeck_core::claude_logs;
use usagedeck_core::store::{CredentialVault, Store, ThemeMode};
use usagedeck_core::view::{demo_snapshot, poll_snapshot, ProviderKind, TaskbarSnapshot};
use usagedeck_core::{default_data_dir, Core};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Posted to the taskbar strip HWND whenever a fresh `TaskbarSnapshot` is ready.
pub const WM_APP_SNAPSHOT: u32 = WM_APP + 1;

const POLL_INTERVAL: Duration = Duration::from_secs(60);
const CURSOR_BURST_INTERVAL: Duration = Duration::from_secs(10);
const CURSOR_BURST_WINDOW: Duration = Duration::from_secs(120);

pub enum PollerCmd {
    RefreshNow,
    /// Poll every 10s for up to 2 minutes after opening Cursor to pick up renewed sign-in.
    WatchCursorSignIn,
    Stop,
}

pub struct SharedSnapshot {
    inner: Mutex<TaskbarSnapshot>,
    loading: Mutex<bool>,
}

impl SharedSnapshot {
    pub fn new(demo: bool) -> Arc<Self> {
        let initial = if demo {
            demo_snapshot(claude_logs::now_ms())
        } else {
            TaskbarSnapshot {
                items: Vec::new(),
                updated_at_ms: 0,
                hide_in_fullscreen: true,
                is_demo: false,
                theme_mode: ThemeMode::Auto,
                snoozed_count: 0,
                taskbar_meter_style: usagedeck_core::store::TaskbarMeterStyle::default(),
            }
        };
        Arc::new(Self {
            inner: Mutex::new(initial),
            loading: Mutex::new(!demo),
        })
    }

    pub fn get(&self) -> (TaskbarSnapshot, bool) {
        let snap = self.inner.lock().unwrap().clone();
        let loading = *self.loading.lock().unwrap();
        (snap, loading)
    }

    pub fn set(&self, snap: TaskbarSnapshot) {
        *self.inner.lock().unwrap() = snap;
        *self.loading.lock().unwrap() = false;
    }
}

/// Spawn the background Tokio runtime thread that polls providers and posts
/// `WM_APP_SNAPSHOT` to `target_hwnd` (passed as raw `isize` for `Send` safety).
pub fn start_poller(
    target_hwnd: isize,
    shared: Arc<SharedSnapshot>,
    demo: bool,
) -> Sender<PollerCmd> {
    let (tx, rx) = mpsc::channel::<PollerCmd>();

    let _ = std::thread::Builder::new()
        .name("usagedeck-poller".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    log::error!("failed to build tokio runtime: {e}");
                    return;
                }
            };

            if demo {
                post_snapshot_ready(target_hwnd);
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        PollerCmd::RefreshNow | PollerCmd::WatchCursorSignIn => {
                            shared.set(demo_snapshot(claude_logs::now_ms()));
                            post_snapshot_ready(target_hwnd);
                        }
                        PollerCmd::Stop => break,
                    }
                }
                return;
            }

            let Some(data_dir) = default_data_dir() else {
                log::error!("APPDATA is not set; cannot load UsageDeck store");
                return;
            };
            let core = Core::new(Store::new(data_dir, Box::new(CredentialVault)));
            claude_logs::start_scanner(core.spend.clone());

            // Initial fetch immediately on launch.
            let snap = rt.block_on(poll_snapshot(&core));
            shared.set(snap);
            post_snapshot_ready(target_hwnd);

            let mut cursor_burst_until: Option<Instant> = None;

            loop {
                let wait = match cursor_burst_until {
                    Some(deadline) if Instant::now() < deadline => CURSOR_BURST_INTERVAL,
                    Some(_) => {
                        cursor_burst_until = None;
                        POLL_INTERVAL
                    }
                    None => POLL_INTERVAL,
                };
                match rx.recv_timeout(wait) {
                    Ok(PollerCmd::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(PollerCmd::WatchCursorSignIn) => {
                        cursor_burst_until = Some(Instant::now() + CURSOR_BURST_WINDOW);
                        let snap = rt.block_on(poll_snapshot(&core));
                        if cursor_signed_in(&snap) {
                            cursor_burst_until = None;
                        }
                        shared.set(snap);
                        post_snapshot_ready(target_hwnd);
                    }
                    Ok(PollerCmd::RefreshNow) | Err(mpsc::RecvTimeoutError::Timeout) => {
                        let snap = rt.block_on(poll_snapshot(&core));
                        if cursor_burst_until.is_some() && cursor_signed_in(&snap) {
                            cursor_burst_until = None;
                        }
                        shared.set(snap);
                        post_snapshot_ready(target_hwnd);
                    }
                }
            }
        });

    tx
}

fn cursor_signed_in(snap: &TaskbarSnapshot) -> bool {
    snap.items
        .iter()
        .any(|i| i.provider == ProviderKind::Cursor && !i.auth_expired)
}

fn post_snapshot_ready(raw_hwnd: isize) {
    let hwnd = HWND(raw_hwnd as *mut std::ffi::c_void);
    // SAFETY: `PostMessageW` is thread-safe for posting custom `WM_APP` notifications.
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_SNAPSHOT, WPARAM(0), LPARAM(0));
    }
}
