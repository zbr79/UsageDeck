//! Lightweight rotating file logger implementing `log::Log`.
//!
//! Writes formatted log lines to `%LOCALAPPDATA%\com.usagedeck.app\logs\usagedeck.log` with size
//! rotation (~1 MB per file, 5 files kept: the active `usagedeck.log` plus 4 rotated archives
//! `usagedeck.log.1` .. `usagedeck.log.4`). Zero external dependencies beyond `log` and `chrono`.

use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

/// Active log file name inside `%LOCALAPPDATA%\com.usagedeck.app\logs`.
pub const LOG_FILE_NAME: &str = "usagedeck.log";

/// Maximum active file size in bytes before rotating (~1 MB).
pub const MAX_FILE_SIZE: u64 = 1_000_000;

/// Total number of log files kept on disk (1 active + 4 rotated backups).
pub const MAX_FILES: usize = 5;

/// Default log level filter.
pub const DEFAULT_LEVEL: LevelFilter = LevelFilter::Info;

/// Initialize the global logger writing to `%LOCALAPPDATA%\com.usagedeck.app\logs\usagedeck.log`
/// at `LevelFilter::Info`. Returns the active log file path on success, or `None` if `LOCALAPPDATA`
/// is unset or a global logger was already installed.
pub fn init() -> Option<PathBuf> {
    let dir = usagedeck_core::default_log_dir()?;
    init_in_dir(dir, DEFAULT_LEVEL).ok()
}

/// Initialize the global rotating file logger inside `dir` with the given `level`.
pub fn init_in_dir(dir: PathBuf, level: LevelFilter) -> Result<PathBuf, SetLoggerError> {
    let logger = RotatingLogger::new(dir, MAX_FILE_SIZE, MAX_FILES, level);
    let path = logger.log_path();
    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(level);
    Ok(path)
}

/// Thread-safe rotating file logger implementing [`log::Log`].
pub struct RotatingLogger {
    level: LevelFilter,
    writer: Mutex<RotatingWriter>,
}

impl RotatingLogger {
    pub fn new(dir: PathBuf, max_bytes: u64, max_files: usize, level: LevelFilter) -> Self {
        Self {
            level,
            writer: Mutex::new(RotatingWriter::new(
                dir,
                LOG_FILE_NAME.to_string(),
                max_bytes,
                max_files,
            )),
        }
    }

    /// Path to the active log file (e.g. `...\logs\usagedeck.log`).
    pub fn log_path(&self) -> PathBuf {
        let writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        writer.active_path()
    }
}

impl Log for RotatingLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string();
        let line = format_record(&timestamp, record.level(), record.target(), *record.args());
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = writer.write_line(&line);
    }

    fn flush(&self) {
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = writer.flush();
    }
}

/// Format a single log line with timestamp, level, target, and message.
pub fn format_record(
    timestamp: &str,
    level: Level,
    target: &str,
    args: fmt::Arguments<'_>,
) -> String {
    format!("[{timestamp}] [{level}] [{target}] {args}\n")
}

/// Internal size-rotating file sink.
struct RotatingWriter {
    dir: PathBuf,
    base_name: String,
    max_bytes: u64,
    max_files: usize,
    file: Option<File>,
    written: u64,
}

impl RotatingWriter {
    fn new(dir: PathBuf, base_name: String, max_bytes: u64, max_files: usize) -> Self {
        Self {
            dir,
            base_name,
            max_bytes: max_bytes.max(1),
            max_files: max_files.max(1),
            file: None,
            written: 0,
        }
    }

    fn active_path(&self) -> PathBuf {
        self.dir.join(&self.base_name)
    }

    fn archive_path(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{}.{index}", self.base_name))
    }

    fn ensure_open(&mut self) -> io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        fs::create_dir_all(&self.dir)?;
        let path = self.active_path();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.written = file.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(file);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Close the active file handle first so Windows allows renaming it.
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
        let backups = self.max_files.saturating_sub(1);
        let active = self.active_path();

        if backups > 0 {
            let oldest = self.archive_path(backups);
            let _ = fs::remove_file(&oldest);

            for i in (1..backups).rev() {
                let src = self.archive_path(i);
                if src.exists() {
                    let dst = self.archive_path(i + 1);
                    let _ = fs::remove_file(&dst);
                    let _ = fs::rename(&src, &dst);
                }
            }

            if active.exists() {
                let first = self.archive_path(1);
                let _ = fs::remove_file(&first);
                let _ = fs::rename(&active, &first);
            }
        }

        fs::create_dir_all(&self.dir)?;
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&active)?;
        self.written = 0;
        self.file = Some(file);
        Ok(())
    }

    fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.ensure_open()?;
        let len = line.len() as u64;
        if self.written > 0 && self.written.saturating_add(len) > self.max_bytes {
            self.rotate()?;
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        file.write_all(line.as_bytes())?;
        self.written = self.written.saturating_add(len);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.flush()?;
        }
        Ok(())
    }
}

/// Return existing active and rotated log files in `dir`, newest first.
#[cfg(test)]
pub fn list_log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let active = dir.join(LOG_FILE_NAME);
    if active.is_file() {
        files.push(active);
    }
    for i in 1..MAX_FILES {
        let archive = dir.join(format!("{LOG_FILE_NAME}.{i}"));
        if archive.is_file() {
            files.push(archive);
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-logging-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn formats_log_records_consistently() {
        let line = format_record(
            "2026-11-21 06:45:00.123 -08:00",
            Level::Warn,
            "usagedeck_taskbar::updater",
            format_args!("update check failed: {}", "offline"),
        );
        assert_eq!(
            line,
            "[2026-11-21 06:45:00.123 -08:00] [WARN] [usagedeck_taskbar::updater] update check failed: offline\n"
        );
    }

    #[test]
    fn rotates_and_keeps_at_most_five_files() {
        let dir = temp_dir("rotate5");
        let mut writer = RotatingWriter::new(dir.clone(), LOG_FILE_NAME.into(), 30, 5);

        // Each line is 20 bytes; every 2nd line forces a rotation (20 + 20 > 30).
        for i in 0..10 {
            let line = format!("entry-{i:02}-0123456789\n");
            assert_eq!(line.len(), 20);
            writer.write_line(&line).unwrap();
        }
        writer.flush().unwrap();

        assert!(dir.join("usagedeck.log").is_file());
        assert!(dir.join("usagedeck.log.1").is_file());
        assert!(dir.join("usagedeck.log.2").is_file());
        assert!(dir.join("usagedeck.log.3").is_file());
        assert!(dir.join("usagedeck.log.4").is_file());
        assert!(!dir.join("usagedeck.log.5").exists());

        assert_eq!(
            fs::read_to_string(dir.join("usagedeck.log")).unwrap(),
            "entry-09-0123456789\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("usagedeck.log.1")).unwrap(),
            "entry-08-0123456789\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("usagedeck.log.4")).unwrap(),
            "entry-05-0123456789\n"
        );
        assert_eq!(list_log_files(&dir).len(), 5);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn max_files_one_truncates_active_file_without_backups() {
        let dir = temp_dir("rotate1");
        let mut writer = RotatingWriter::new(dir.clone(), LOG_FILE_NAME.into(), 16, 1);

        writer.write_line("first-line\n").unwrap();
        writer.write_line("second-line\n").unwrap();
        writer.flush().unwrap();

        assert_eq!(
            fs::read_to_string(dir.join("usagedeck.log")).unwrap(),
            "second-line\n"
        );
        assert!(!dir.join("usagedeck.log.1").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopening_preserves_existing_size_and_rotates_when_full() {
        let dir = temp_dir("reopen");
        {
            let mut writer = RotatingWriter::new(dir.clone(), LOG_FILE_NAME.into(), 25, 3);
            writer.write_line("01234567890123456789\n").unwrap(); // 21 bytes
            writer.flush().unwrap();
        }
        {
            let mut writer = RotatingWriter::new(dir.clone(), LOG_FILE_NAME.into(), 25, 3);
            writer.write_line("next-line\n").unwrap(); // 21 + 10 > 25 -> rotate
            writer.flush().unwrap();
        }

        assert_eq!(
            fs::read_to_string(dir.join("usagedeck.log")).unwrap(),
            "next-line\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("usagedeck.log.1")).unwrap(),
            "01234567890123456789\n"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
