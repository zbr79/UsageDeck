//! Crash file: a panic hook that writes `crash-<unix-seconds>.txt` into the app log folder, then
//! chains to the default hook. The newest [`KEEP`] crash files are kept.
//!
//! Privacy: the file holds only the panic message, its source location, a backtrace (symbol
//! names and addresses) and the app version. The panic message is free text chosen by whoever
//! wrote the `panic!` / `expect` / `unwrap`, so a panic that formats an account email or key into
//! its message would put it in this file. Never format emails, keys or tokens into a panic
//! message.

use std::fs;
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// How many crash files to keep (newest first).
pub const KEEP: usize = 10;

const PREFIX: &str = "crash-";
const SUFFIX: &str = ".txt";

/// The folder the hook writes to. Set at install time, refined in `setup` once Tauri's path
/// resolver is available.
static DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// `crash-<unix-seconds>.txt`.
pub fn file_name(unix_secs: u64) -> String {
    format!("{PREFIX}{unix_secs}{SUFFIX}")
}

/// The seconds of a crash file name, or `None` for any other file.
pub fn parse_name(name: &str) -> Option<u64> {
    let digits = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The crash file names to delete so only the newest `keep` remain. Other files are ignored.
pub fn to_prune(names: &[String], keep: usize) -> Vec<String> {
    let mut crashes: Vec<(u64, &String)> = names
        .iter()
        .filter_map(|n| parse_name(n).map(|secs| (secs, n)))
        .collect();
    crashes.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(a.1)));
    crashes
        .into_iter()
        .skip(keep)
        .map(|(_, n)| n.clone())
        .collect()
}

/// The text of one crash report.
pub fn render(version: &str, message: &str, location: &str, backtrace: &str) -> String {
    format!(
        "UsageDeck {version} crashed\nmessage: {message}\nlocation: {location}\n\nbacktrace:\n{backtrace}\n"
    )
}

/// Tauri's log folder on Windows before the app exists: `%LOCALAPPDATA%\<identifier>\logs`, the
/// same rule as `PathResolver::app_log_dir` (tauri src/path/desktop.rs, `data_local_dir()`
/// joined with the identifier and `logs`).
pub fn default_log_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("com.usagedeck.app").join("logs"))
}

/// Point the hook at Tauri's resolved log folder.
pub fn set_log_dir(dir: PathBuf) {
    if let Ok(mut slot) = DIR.write() {
        *slot = Some(dir);
    }
}

/// Install the panic hook. Call once, as early as possible.
pub fn install(dir: Option<PathBuf>) {
    if let Some(dir) = dir {
        set_log_dir(dir);
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // never let a failure to write the report hide the panic itself
        let dir = DIR.try_read().ok().and_then(|slot| slot.clone());
        if let Some(dir) = dir {
            let _ = write_report(&dir, info);
        }
        default_hook(info);
    }));
}

fn write_report(dir: &Path, info: &PanicHookInfo<'_>) -> std::io::Result<()> {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(non-text panic payload)");
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "(unknown)".into());
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let text = render(env!("CARGO_PKG_VERSION"), message, &location, &backtrace);

    fs::create_dir_all(dir)?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // append: two panics in the same second share one file instead of overwriting
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file_name(secs)))?;
    file.write_all(text.as_bytes())?;
    drop(file);
    prune(dir);
    Ok(())
}

fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    for name in to_prune(&names, KEEP) {
        let _ = fs::remove_file(dir.join(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        assert_eq!(file_name(1_760_000_000), "crash-1760000000.txt");
        assert_eq!(parse_name("crash-1760000000.txt"), Some(1_760_000_000));
        for other in [
            "UsageDeck.log",
            "crash-.txt",
            "crash-12a.txt",
            "crash-+12.txt",
            "crash-12.log",
            "xcrash-12.txt",
        ] {
            assert_eq!(parse_name(other), None, "{other}");
        }
    }

    #[test]
    fn prune_keeps_newest_and_ignores_other_files() {
        let mut names: Vec<String> = (1..=13u64).map(file_name).collect();
        names.push("UsageDeck.log".into());
        names.push("crash-notes.txt".into());
        names.reverse();
        let mut gone = to_prune(&names, KEEP);
        gone.sort();
        assert_eq!(gone, vec![file_name(1), file_name(2), file_name(3)]);
        // numeric, not text, order: 9 is older than 10
        let names = vec![file_name(9), file_name(10)];
        assert_eq!(to_prune(&names, 1), vec![file_name(9)]);
        assert!(to_prune(&names, KEEP).is_empty());
    }

    #[test]
    fn report_holds_version_message_location() {
        let text = render("1.2.3", "boom", "src/lib.rs:1:2", "frames");
        assert!(text.contains("UsageDeck 1.2.3"));
        assert!(text.contains("message: boom"));
        assert!(text.contains("location: src/lib.rs:1:2"));
        assert!(text.contains("frames"));
    }
}
