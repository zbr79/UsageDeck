//! Pure-Win32 HKCU `Run` registry helper for "Start at login".
//!
//! Writes or deletes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` -> `"UsageDeck"`
//! using the `windows = "0.62"` Win32 registry APIs directly (zero Tauri plugins).
//!
//! Safety guard: debug builds and executables launched from a Cargo `target` directory never
//! modify the Windows Run key, so development and test runs cannot clobber an installed copy.

use std::path::Path;
use windows::core::w;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_SZ,
};

/// Registry path under `HKEY_CURRENT_USER` holding per-user startup commands.
pub const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Value name under `RUN_KEY_PATH`; matches the NSIS uninstaller cleanup in `installer-hooks.nsh`.
pub const VALUE_NAME: &str = "UsageDeck";

/// RAII wrapper ensuring an opened Win32 `HKEY` is always closed.
struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        // SAFETY: `self.0` was opened via `RegOpenKeyExW` and is closed once here.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

/// True when the current executable sits inside a Cargo target directory: any `target` path
/// component, or an ancestor holding Cargo's `CACHEDIR.TAG` / `.rustc_info.json` markers
/// (covers a custom `CARGO_TARGET_DIR`).
pub fn running_from_cargo_target() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return true;
    };
    is_cargo_target_path(&exe)
}

/// Pure path check behind [`running_from_cargo_target`].
pub fn is_cargo_target_path(exe: &Path) -> bool {
    exe.ancestors().skip(1).any(|dir| {
        dir.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("target"))
            || dir.join("CACHEDIR.TAG").is_file()
            || dir.join(".rustc_info.json").is_file()
    })
}

/// True for debug builds or any binary running out of a Cargo target directory.
pub fn is_dev_build() -> bool {
    cfg!(debug_assertions) || running_from_cargo_target()
}

/// Quote the executable path for the Windows `Run` value so paths with spaces launch cleanly.
pub fn format_run_command(exe: &Path) -> String {
    let raw = exe.to_string_lossy();
    let trimmed = raw.trim_matches('"');
    format!("\"{trimmed}\"")
}

/// Encode a Rust string as a NUL-terminated UTF-16LE byte buffer suitable for `REG_SZ`.
pub fn encode_reg_sz_bytes(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Register or remove the `HKCU\...\Run\UsageDeck` entry.
///
/// Debug builds and binaries running from a Cargo target directory leave the registry untouched
/// and only log what was requested.
pub fn set_start_at_login(enabled: bool) {
    if is_dev_build() {
        log::info!("start at login left unchanged (dev build), wanted {enabled}");
        return;
    }
    match try_set_start_at_login(enabled) {
        Ok(()) => log::info!("start at login set to {enabled}"),
        Err(e) => log::warn!("could not set start at login to {enabled}: {e}"),
    }
}

/// Like [`set_start_at_login`], returning any Win32 or I/O error instead of only logging it.
///
/// Still refuses to write in debug or Cargo-target builds (returning `Ok(())` as a no-op).
pub fn try_set_start_at_login(enabled: bool) -> Result<(), String> {
    if is_dev_build() {
        log::info!("start at login left unchanged (dev build), wanted {enabled}");
        return Ok(());
    }

    let mut raw_key = HKEY::default();
    // SAFETY: static NUL-terminated UTF-16 key path; `raw_key` receives the opened handle and is
    // immediately wrapped in `RegKey` for guaranteed cleanup.
    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            None,
            KEY_SET_VALUE,
            &mut raw_key,
        )
    };
    if open_status != ERROR_SUCCESS {
        return Err(format!(
            "RegOpenKeyExW({RUN_KEY_PATH}) failed with Win32 error {}",
            open_status.0
        ));
    }
    let key = RegKey(raw_key);

    if enabled {
        let exe = std::env::current_exe()
            .map_err(|e| format!("could not resolve current executable path: {e}"))?;
        let command = format_run_command(&exe);
        let data = encode_reg_sz_bytes(&command);
        // SAFETY: `key.0` is open with `KEY_SET_VALUE`, value name is NUL-terminated, and `data`
        // is a NUL-terminated UTF-16LE buffer.
        let status = unsafe { RegSetValueExW(key.0, w!("UsageDeck"), None, REG_SZ, Some(&data)) };
        if status != ERROR_SUCCESS {
            return Err(format!(
                "RegSetValueExW({VALUE_NAME}) failed with Win32 error {}",
                status.0
            ));
        }
        Ok(())
    } else {
        // SAFETY: `key.0` is open with `KEY_SET_VALUE` and value name is NUL-terminated.
        let status = unsafe { RegDeleteValueW(key.0, w!("UsageDeck")) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!(
                "RegDeleteValueW({VALUE_NAME}) failed with Win32 error {}",
                status.0
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "usagedeck-autostart-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_cargo_target_component_case_insensitively() {
        assert!(is_cargo_target_path(Path::new(
            r"C:\repo\target\release\usagedeck-taskbar.exe"
        )));
        assert!(is_cargo_target_path(Path::new(
            r"C:\repo\TARGET\debug\usagedeck-taskbar.exe"
        )));
        assert!(is_cargo_target_path(Path::new(
            "/home/dev/UsageDeck/target/debug/usagedeck-taskbar"
        )));
    }

    #[test]
    fn detects_custom_cargo_target_dir_by_marker_files() {
        let dir = temp_dir("cachedir");
        std::fs::write(
            dir.join("CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55",
        )
        .unwrap();
        let exe = dir.join("release").join("usagedeck-taskbar.exe");
        assert!(is_cargo_target_path(&exe));
        let _ = std::fs::remove_dir_all(&dir);

        let dir = temp_dir("rustc-info");
        std::fs::write(dir.join(".rustc_info.json"), "{}").unwrap();
        let exe = dir.join("debug").join("usagedeck-taskbar.exe");
        assert!(is_cargo_target_path(&exe));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installed_path_is_not_treated_as_cargo_target() {
        let dir = temp_dir("installed");
        let exe = dir.join("UsageDeck").join("usagedeck-taskbar.exe");
        assert!(!is_cargo_target_path(&exe));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_run_command_quotes_paths_with_spaces_once() {
        assert_eq!(
            format_run_command(Path::new(r"C:\Program Files\UsageDeck\usagedeck.exe")),
            r#""C:\Program Files\UsageDeck\usagedeck.exe""#
        );
        assert_eq!(
            format_run_command(Path::new(r#""C:\UsageDeck\usagedeck.exe""#)),
            r#""C:\UsageDeck\usagedeck.exe""#
        );
    }

    #[test]
    fn encode_reg_sz_bytes_is_nul_terminated_utf16le() {
        let bytes = encode_reg_sz_bytes("AB");
        assert_eq!(bytes, vec![b'A', 0, b'B', 0, 0, 0]);
    }
}
