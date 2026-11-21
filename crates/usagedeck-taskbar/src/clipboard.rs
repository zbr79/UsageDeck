//! Pure Win32 UTF-16 clipboard helper (`CF_UNICODETEXT`).
//!
//! Used to copy OpenCode API keys from the flyout or Settings dialog without
//! pulling in Tauri or any external clipboard crate.

use std::time::Duration;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

/// Win32 clipboard format for NUL-terminated UTF-16LE text (`CF_UNICODETEXT`).
const CF_UNICODETEXT: u32 = 13;

/// RAII guard ensuring `CloseClipboard` always runs once `OpenClipboard` succeeds.
struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: `ClipboardGuard` is only constructed after a successful `OpenClipboard`.
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Copy `text` to the Windows clipboard as `CF_UNICODETEXT`.
///
/// Retries `OpenClipboard` briefly in case another process (e.g. a clipboard
/// manager or Remote Desktop) momentarily holds the clipboard lock.
pub fn copy_text(text: &str) -> Result<(), String> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| "clipboard text is too large".to_string())?;

    // SAFETY: standard Win32 clipboard sequence; `hmem` ownership transfers to
    // the system only when `SetClipboardData` succeeds, otherwise we free it.
    unsafe {
        let mut opened = false;
        let mut last_err = String::new();
        for attempt in 0..5 {
            match OpenClipboard(None) {
                Ok(()) => {
                    opened = true;
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    if attempt < 4 {
                        std::thread::sleep(Duration::from_millis(15));
                    }
                }
            }
        }
        if !opened {
            return Err(format!("OpenClipboard failed: {last_err}"));
        }
        let _guard = ClipboardGuard;

        EmptyClipboard().map_err(|e| format!("EmptyClipboard failed: {e}"))?;

        let hmem: HGLOBAL =
            GlobalAlloc(GMEM_MOVEABLE, byte_len).map_err(|e| format!("GlobalAlloc failed: {e}"))?;

        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            let _ = GlobalFree(Some(hmem));
            return Err("GlobalLock returned null".to_string());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr.cast::<u16>(), wide.len());
        let _ = GlobalUnlock(hmem);

        if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0))) {
            let _ = GlobalFree(Some(hmem));
            return Err(format!("SetClipboardData failed: {e}"));
        }
    }

    Ok(())
}
