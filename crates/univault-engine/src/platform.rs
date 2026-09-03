// Vendored from tq-univault crates/univault-core/src/platform.rs @ 36e7774 (config_dir only); adapted per docs/engine-extraction.md.
//! Per-OS path conventions — the one place `cfg(target_os)` is
//! allowed (each consumer's ARCHITECTURE.md platform-confinement
//! rule). Functions here compute paths from environment variables;
//! reading or writing the filesystem stays in the shell. Game-specific
//! save-tree layouts live in each game's core crate.

use std::path::PathBuf;

/// The app's configuration directory for this platform (not created
/// here): `~/Library/Application Support/<app>` on macOS,
/// `%APPDATA%\<app>` on Windows, XDG config on Linux. `app_name` is
/// the final path segment, so it must be a plain directory name.
#[must_use]
pub fn config_dir(app_name: &str) -> Option<PathBuf> {
    config_base().map(|base| base.join(app_name))
}

#[cfg(target_os = "macos")]
fn config_base() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
}

#[cfg(target_os = "windows")]
fn config_base() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn config_base() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_ends_with_the_app_name() {
        let dir = config_dir("grim-vault").expect("a config dir on a supported platform");
        assert!(dir.ends_with("grim-vault"), "{dir:?}");
    }

    #[test]
    fn different_apps_get_sibling_directories() {
        let a = config_dir("tq-univault").expect("config dir");
        let b = config_dir("grim-vault").expect("config dir");
        assert_eq!(a.parent(), b.parent());
        assert_ne!(a, b);
    }
}
