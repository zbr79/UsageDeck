//! User activity for the polling gate: is the workstation locked, and how long since the last
//! keyboard or mouse input. Both are cheap Win32 queries with no side effects.

use std::time::Duration;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_CONTROL_FLAGS,
    DESKTOP_READOBJECTS, UOI_NAME,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

/// True when the input desktop is not "Default" (lock screen, UAC prompt) or cannot be opened
/// at all, which is what happens while the workstation is locked.
pub fn workstation_locked() -> bool {
    // SAFETY: plain Win32 calls; the buffer outlives the call and its size is passed along, and
    // the desktop handle is closed before returning.
    unsafe {
        let Ok(desk) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS)
        else {
            return true;
        };
        let mut name = [0u16; 64];
        let mut needed = 0u32;
        let read = GetUserObjectInformationW(
            HANDLE(desk.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            (name.len() * 2) as u32,
            Some(&mut needed),
        );
        let _ = CloseDesktop(desk);
        if read.is_err() {
            return true;
        }
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        !String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case("Default")
    }
}

/// Time since the last keyboard or mouse input in this session; zero when unknown.
pub fn idle_time() -> Duration {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a properly sized LASTINPUTINFO owned by this frame.
    let ok = unsafe { GetLastInputInfo(&mut info) }.as_bool();
    if !ok {
        return Duration::ZERO;
    }
    // dwTime is a 32-bit tick count, so compare in wrapping 32-bit arithmetic.
    let now = unsafe { GetTickCount64() } as u32;
    Duration::from_millis(u64::from(now.wrapping_sub(info.dwTime)))
}
