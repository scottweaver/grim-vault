//! Where Grim Dawn keeps its files on each platform, as path
//! computations only: candidates are derived from environment
//! variables and the shell checks which exist. The config directory
//! itself comes from `univault_engine::platform::config_dir`.

use std::path::{Path, PathBuf};

use univault_engine::platform::config_dir;

/// Steam's app id for Grim Dawn, the folder name under `userdata`.
pub const STEAM_APP_ID: &str = "219990";

/// This app's name as a directory segment.
pub const APP_NAME: &str = "grim-vault";

/// The app's own config directory (`vault-store.json`, settings).
#[must_use]
pub fn app_config_dir() -> Option<PathBuf> {
    config_dir(APP_NAME)
}

/// Files that mark a directory as a save root: the game writes the
/// shared stash beside the per-character `main/` folder.
pub const SAVE_DIR_MARKERS: [&str; 2] = ["transfer.gst", "main"];

/// The file that marks a directory as the game install.
pub const GAME_DIR_MARKER: &str = "database/database.arz";

/// Save roots worth suggesting, most conventional first: the local
/// `Documents/My Games` layout and the Steam-cloud `userdata` layout
/// under each Steam root. Custom locations (network mounts, moved
/// libraries) are not discoverable — the shell offers a picker.
#[must_use]
pub fn save_dir_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(documents) = documents_dir() {
        candidates.push(documents.join("My Games/Grim Dawn/save"));
    }
    for steam in steam_roots() {
        candidates.push(steam.join("userdata"));
    }
    candidates
}

/// Game install roots worth suggesting: `steamapps/common/Grim Dawn`
/// under each Steam root, then the GOG default on Windows.
#[must_use]
pub fn game_dir_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = steam_roots()
        .into_iter()
        .map(|steam| steam.join("steamapps/common/Grim Dawn"))
        .collect();
    if cfg!(target_os = "windows") {
        candidates.push(PathBuf::from(r"C:\GOG Games\Grim Dawn"));
    }
    candidates
}

/// Given a Steam `userdata` directory, the per-account save roots
/// beneath it: `userdata/<account>/219990/remote/save`. The shell
/// lists `<account>` directories; this names the leaf.
#[must_use]
pub fn steam_cloud_save_dir(account_dir: &Path) -> PathBuf {
    account_dir.join(STEAM_APP_ID).join("remote/save")
}

fn documents_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Documents"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn steam_roots() -> Vec<PathBuf> {
    home_dir()
        .map(|home| vec![home.join("Library/Application Support/Steam")])
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn steam_roots() -> Vec<PathBuf> {
    let program_files = std::env::var_os("ProgramFiles(x86)")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    vec![program_files.join("Steam")]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn steam_roots() -> Vec<PathBuf> {
    home_dir()
        .map(|home| {
            vec![
                home.join(".steam/steam"),
                home.join(".local/share/Steam"),
                home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            ]
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_cloud_save_dir_names_the_app_id_leaf() {
        assert_eq!(
            steam_cloud_save_dir(Path::new("/steam/userdata/123")),
            PathBuf::from("/steam/userdata/123/219990/remote/save")
        );
    }

    #[test]
    fn candidates_are_non_empty_on_a_supported_platform() {
        assert!(!save_dir_candidates().is_empty());
        assert!(!game_dir_candidates().is_empty());
    }

    #[test]
    fn app_config_dir_ends_with_the_app_name() {
        let dir = app_config_dir().expect("config dir");
        assert!(dir.ends_with(APP_NAME), "{dir:?}");
    }
}
