//! Windows-only helpers: Chromium safeStorage decryption, idle/lock detection,
//! installed program uninstall registry entries, provider app launching, and full-screen detection.

pub mod fullscreen;
pub mod idle;
pub mod launch;
pub mod os_crypt;
pub mod uninstall;
