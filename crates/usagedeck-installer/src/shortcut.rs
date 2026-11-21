//! Win32 COM helper for creating `.lnk` shortcuts via `IShellLinkW` and `IPersistFile`.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::{IUnknown, Interface, Result, PCWSTR};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

/// RAII guard balancing `CoInitializeEx` with `CoUninitialize` on this thread.
struct ComGuard {
    uninit: bool,
}

impl ComGuard {
    fn init() -> Result<Self> {
        // SAFETY: standard STA initialization for Shell COM objects on the calling thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_ok() {
            Ok(Self { uninit: true })
        } else if hr == RPC_E_CHANGED_MODE {
            // COM is already initialized in MTA on this thread; ShellLink still works and we
            // must not call CoUninitialize for an apartment we did not initialize.
            Ok(Self { uninit: false })
        } else {
            hr.ok()?;
            Ok(Self { uninit: false })
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninit {
            // SAFETY: balances the successful `CoInitializeEx` call in `ComGuard::init`.
            unsafe {
                CoUninitialize();
            }
        }
    }
}

fn wide_os(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Create or overwrite a Windows `.lnk` shortcut pointing at `target_exe`.
pub fn create_shortcut(
    lnk_path: &Path,
    target_exe: &Path,
    working_dir: &Path,
    description: &str,
) -> Result<()> {
    if let Some(parent) = lnk_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let _com = ComGuard::init()?;

    let target_w = wide_os(target_exe.as_os_str());
    let workdir_w = wide_os(working_dir.as_os_str());
    let desc_w = wide_str(description);
    let lnk_w = wide_os(lnk_path.as_os_str());

    let no_outer: Option<&IUnknown> = None;

    // SAFETY: `ShellLink` COM object created in-process; all wide strings are NUL-terminated
    // and live through each COM method call.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, no_outer, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(target_w.as_ptr()))?;
        link.SetWorkingDirectory(PCWSTR(workdir_w.as_ptr()))?;
        link.SetDescription(PCWSTR(desc_w.as_ptr()))?;
        link.SetIconLocation(PCWSTR(target_w.as_ptr()), 0)?;

        let persist: IPersistFile = link.cast()?;
        persist.Save(PCWSTR(lnk_w.as_ptr()), true)?;
    }

    Ok(())
}
