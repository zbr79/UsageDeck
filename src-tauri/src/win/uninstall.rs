//! Read-only listing of the installed programs' uninstall entries (per user, per machine, and the
//! 32-bit view), used to find Cursor.exe when it is not in its default folder.

use crate::providers::cursor_signin::UninstallEntry;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
};

const ROOTS: [(HKEY, &str); 3] = [
    (
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
    (
        HKEY_LOCAL_MACHINE,
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
    (
        HKEY_LOCAL_MACHINE,
        r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A string value of `subkey`, expanded if it is REG_EXPAND_SZ.
fn string_value(key: HKEY, subkey: &[u16], name: &str) -> Option<String> {
    let name = wide(name);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut bytes = 0u32;
    // SAFETY: a size query; both strings are NUL-terminated and outlive the call.
    let sized = unsafe {
        RegGetValueW(
            key,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            None,
            Some(&mut bytes),
        )
    };
    if sized != ERROR_SUCCESS || bytes == 0 {
        return None;
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
    // SAFETY: `buf` holds `bytes` bytes, as the size query asked for.
    let read = unsafe {
        RegGetValueW(
            key,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            flags,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if read != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len])).filter(|s| !s.trim().is_empty())
}

fn entries_under(root: HKEY, path: &str) -> Vec<UninstallEntry> {
    let mut key = HKEY::default();
    let path = wide(path);
    // SAFETY: `path` is NUL-terminated; `key` receives the opened handle, closed below.
    if unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key) }
        != ERROR_SUCCESS
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut name = [0u16; 256];
    for index in 0.. {
        let mut len = name.len() as u32;
        // SAFETY: `name` has room for `len` UTF-16 units including the NUL.
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        let subkey: Vec<u16> = name[..len as usize]
            .iter()
            .copied()
            .chain(std::iter::once(0))
            .collect();
        out.push(UninstallEntry {
            display_name: string_value(key, &subkey, "DisplayName"),
            publisher: string_value(key, &subkey, "Publisher"),
            install_location: string_value(key, &subkey, "InstallLocation"),
            display_icon: string_value(key, &subkey, "DisplayIcon"),
        });
    }
    // SAFETY: `key` was opened above and is not used after this.
    let _ = unsafe { RegCloseKey(key) };
    out
}

/// Every uninstall entry the current user can read.
pub fn uninstall_entries() -> Vec<UninstallEntry> {
    ROOTS
        .iter()
        .flat_map(|(root, path)| entries_under(*root, path))
        .collect()
}
