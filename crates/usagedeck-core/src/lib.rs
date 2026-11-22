//! UsageDeck core library: providers, local storage, Claude Code spend engine,
//! and the pure-Rust taskbar view model. Zero Tauri or webview dependencies.

pub mod claude_logs;
pub mod crash;
pub mod history;
pub mod pricing;
pub mod providers;
pub mod store;
pub mod updater;
pub mod view;
pub mod win;

use providers::{SeenAccounts, UsageCache};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use store::{Store, SEEN_ACCOUNTS_FILE, USAGE_CACHE_FILE};

/// Application folder identifier under `%APPDATA%` / `%LOCALAPPDATA%`,
/// preserving full compatibility with existing UsageDeck settings and caches.
pub const APP_IDENTIFIER: &str = "com.usagedeck.app";

/// Default data directory: `%APPDATA%\com.usagedeck.app`.
pub fn default_data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join(APP_IDENTIFIER))
}

/// Default log directory: `%LOCALAPPDATA%\com.usagedeck.app\logs`.
pub fn default_log_dir() -> Option<PathBuf> {
    crash::default_log_dir()
}

/// Shared engine state passed to every provider.
pub struct Core {
    pub store: Store,
    pub cache: UsageCache,
    pub seen: SeenAccounts,
    pub http: reqwest::Client,
    /// Claude Code spend from the local transcripts.
    pub spend: Arc<claude_logs::SpendEngine>,
}

impl Core {
    pub fn new(store: Store) -> Self {
        providers::cursor::sweep_stale_temp_dirs();
        let cache = UsageCache::new(store.file(USAGE_CACHE_FILE));
        let seen = SeenAccounts::new(store.file(SEEN_ACCOUNTS_FILE));
        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .timeout(Duration::from_secs(20))
            .build()
            .expect("failed to build hardened HTTPS client");
        let spend = Arc::new(claude_logs::SpendEngine::new(
            store.dir().to_path_buf(),
            claude_logs::default_root(),
        ));
        Self {
            store,
            cache,
            seen,
            http,
            spend,
        }
    }
}
