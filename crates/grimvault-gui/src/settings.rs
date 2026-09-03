//! Reading and writing the app's settings file. The document model
//! (`Settings`, `ConfigDir`, the format tag) lives in
//! `grimvault_core::settings` so the command-line examples share it;
//! this module is the shell side: the filesystem.

use std::io;
use std::path::PathBuf;

pub use grimvault_core::settings::{ConfigDir, Settings, SettingsProblem};
use thiserror::Error;

/// Why the settings could not be read from disk.
#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {problem}", path.display())]
    Refused {
        path: PathBuf,
        problem: SettingsProblem,
    },
}

/// The settings under `dir`, `None` when no file exists yet.
///
/// # Errors
/// [`SettingsError`] when the file exists but cannot be read or parsed.
pub fn load(dir: &ConfigDir) -> Result<Option<Settings>, SettingsError> {
    let path = dir.settings_file();
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = univault_io::read_verified(&path).map_err(|source| SettingsError::Read {
        path: path.clone(),
        source,
    })?;
    Settings::parse(&bytes)
        .map(Some)
        .map_err(|problem| SettingsError::Refused { path, problem })
}

/// Writes the settings, creating the config directory first.
///
/// # Errors
/// Any failure creating the directory or writing the file.
pub fn save(dir: &ConfigDir, settings: &Settings) -> io::Result<()> {
    std::fs::create_dir_all(dir.path())?;
    univault_io::write_synced(&dir.settings_file(), &settings.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Settings {
        Settings {
            game_dir: PathBuf::from("/games/Grim Dawn"),
            save_dir: PathBuf::from("/saves/save"),
        }
    }

    #[test]
    fn load_and_save_round_trip_through_a_scratch_directory() {
        let scratch =
            std::env::temp_dir().join(format!("grimvault-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let dir = ConfigDir::new(scratch.clone());
        assert!(load(&dir).unwrap().is_none());
        save(&dir, &sample()).unwrap();
        assert_eq!(load(&dir).unwrap(), Some(sample()));
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
