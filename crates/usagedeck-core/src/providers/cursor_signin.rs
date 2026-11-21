//! Pure rules for getting a rejected Cursor sign-in back. UsageDeck never refreshes Cursor's token
//! itself (a refresh could cost Cursor its own sign-in); Cursor renews it when it runs. So the
//! widget offers to start Cursor, and re-reads `state.vscdb` on every poll after a failed fetch,
//! to pick up the renewed token as soon as Cursor writes it.
//!
//! No Windows or app dependencies, so the unit tests also run on Linux.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Whether a cached read of `state.vscdb` can be served instead of reading the file again. After
/// any failed fetch the file is read again on the next poll: Cursor may have renewed the token.
pub fn reuse_state(age: Duration, ttl: Duration, last_fetch_failed: bool) -> bool {
    age < ttl && !last_fetch_failed
}

/// The values of one `...\CurrentVersion\Uninstall\<key>` registry entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UninstallEntry {
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub install_location: Option<String>,
    pub display_icon: Option<String>,
}

/// By text, not `Path::file_name`, so a Windows path reads the same in the Linux test run.
fn is_cursor_exe(path: &str) -> bool {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.eq_ignore_ascii_case("cursor.exe")
}

/// Cursor.exe of an uninstall entry that is Cursor's (named "Cursor" or "Cursor (User)", from
/// Anysphere when a publisher is given). `InstallLocation` wins; `DisplayIcon` (`"C:\…\Cursor.exe",0`)
/// is the fallback. Whether the file exists is the caller's check.
pub fn exe_from_uninstall(entry: &UninstallEntry) -> Option<PathBuf> {
    let name = entry.display_name.as_deref()?.trim();
    let named = name.eq_ignore_ascii_case("cursor")
        || name
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("cursor ("));
    let from_anysphere = entry
        .publisher
        .as_deref()
        .is_none_or(|p| p.to_ascii_lowercase().contains("anysphere"));
    if !named || !from_anysphere {
        return None;
    }
    let installed = entry
        .install_location
        .as_deref()
        .map(|l| l.trim().trim_matches('"'))
        .filter(|l| !l.is_empty())
        .map(|l| PathBuf::from(l).join("Cursor.exe"));
    let icon = entry.display_icon.as_deref().map(|icon| {
        let icon = icon.trim();
        let path = match icon.rsplit_once(',') {
            Some((path, index)) if index.trim().parse::<i32>().is_ok() => path,
            _ => icon,
        };
        path.trim().trim_matches('"')
    });
    installed.or(icon.filter(|p| is_cursor_exe(p)).map(PathBuf::from))
}

/// Where to look for Cursor.exe, in order: the per-user install (Cursor's default), the
/// per-machine install, then whatever the uninstall entries name.
pub fn exe_candidates(
    local_app_data: Option<&Path>,
    program_files: Option<&Path>,
    entries: &[UninstallEntry],
) -> Vec<PathBuf> {
    let default = |base: Option<&Path>| base.map(|b| b.join("Programs").join("cursor"));
    [
        default(local_app_data),
        program_files.map(|b| b.join("cursor")),
    ]
    .into_iter()
    .flatten()
    .map(|dir| dir.join("Cursor.exe"))
    .chain(entries.iter().filter_map(exe_from_uninstall))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(60);

    #[test]
    fn a_fresh_read_is_reused_until_a_fetch_fails() {
        let young = Duration::from_secs(5);
        assert!(reuse_state(young, TTL, false));
        assert!(!reuse_state(young, TTL, true));
        assert!(!reuse_state(TTL, TTL, false));
    }

    fn entry(
        name: &str,
        publisher: Option<&str>,
        location: Option<&str>,
        icon: Option<&str>,
    ) -> UninstallEntry {
        UninstallEntry {
            display_name: Some(name.into()),
            publisher: publisher.map(Into::into),
            install_location: location.map(Into::into),
            display_icon: icon.map(Into::into),
        }
    }

    #[test]
    fn cursor_uninstall_entries_give_cursor_exe() {
        let user = entry(
            "Cursor (User)",
            Some("Anysphere"),
            Some(r"C:\Users\me\AppData\Local\Programs\cursor\"),
            None,
        );
        assert_eq!(
            exe_from_uninstall(&user),
            Some(PathBuf::from(r"C:\Users\me\AppData\Local\Programs\cursor\").join("Cursor.exe"))
        );
        let icon_only = entry(
            "Cursor",
            None,
            Some("  "),
            Some(r#""D:\Apps\Cursor\Cursor.exe",0"#),
        );
        assert_eq!(
            exe_from_uninstall(&icon_only),
            Some(PathBuf::from(r"D:\Apps\Cursor\Cursor.exe"))
        );
        let bare_icon = entry(
            "cursor",
            Some("Anysphere, Inc."),
            None,
            Some(r"D:\c\Cursor.exe"),
        );
        assert_eq!(
            exe_from_uninstall(&bare_icon),
            Some(PathBuf::from(r"D:\c\Cursor.exe"))
        );
    }

    #[test]
    fn other_programs_and_icons_are_ignored() {
        let lookalike = entry("Cursor Pro Tools", None, Some(r"C:\x"), None);
        assert_eq!(exe_from_uninstall(&lookalike), None);
        let other_publisher = entry("Cursor", Some("Someone Else"), Some(r"C:\x"), None);
        assert_eq!(exe_from_uninstall(&other_publisher), None);
        let icon_is_an_ico = entry("Cursor", None, None, Some(r"C:\x\cursor.ico,0"));
        assert_eq!(exe_from_uninstall(&icon_is_an_ico), None);
        assert_eq!(exe_from_uninstall(&UninstallEntry::default()), None);
    }

    #[test]
    fn default_installs_come_before_registry_entries() {
        let reg = entry("Cursor", None, Some(r"E:\Cursor"), None);
        let found = exe_candidates(
            Some(Path::new(r"C:\Users\me\AppData\Local")),
            Some(Path::new(r"C:\Program Files")),
            &[reg, entry("Notepad++", None, Some(r"C:\npp"), None)],
        );
        assert_eq!(
            found,
            vec![
                Path::new(r"C:\Users\me\AppData\Local")
                    .join("Programs")
                    .join("cursor")
                    .join("Cursor.exe"),
                Path::new(r"C:\Program Files")
                    .join("cursor")
                    .join("Cursor.exe"),
                Path::new(r"E:\Cursor").join("Cursor.exe"),
            ]
        );
        assert!(exe_candidates(None, None, &[]).is_empty());
    }
}
