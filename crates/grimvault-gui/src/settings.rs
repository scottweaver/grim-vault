//! The persisted setup — where the game and the saves live — as one
//! self-describing JSON file (`settings.json`, format tag
//! [`FORMAT_TAG`]) under the app's config directory. The config
//! directory is the platform's unless [`CONFIG_DIR_ENV`] overrides it,
//! which is how a headless check runs without creating the real one.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Environment variable that replaces the platform config directory.
pub const CONFIG_DIR_ENV: &str = "GRIMVAULT_CONFIG_DIR";
/// The settings file's name inside the config directory.
pub const SETTINGS_FILE: &str = "settings.json";
/// The vault store's name inside the config directory.
pub const STORE_FILE: &str = "vault-store.json";
/// The `format` tag every settings file carries.
pub const FORMAT_TAG: &str = "grimvault-settings";
/// The newest document version this build reads and the one it writes.
pub const FORMAT_VERSION: u32 = 1;

/// The directory holding this app's own files. Existence is not
/// implied: [`save`] creates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigDir(PathBuf);

/// Neither the override variable nor the platform names a config
/// directory.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("no config directory: set {CONFIG_DIR_ENV} or a home directory")]
pub struct NoConfigDir;

impl ConfigDir {
    /// [`CONFIG_DIR_ENV`] when set, else the platform's directory.
    ///
    /// # Errors
    /// [`NoConfigDir`] when neither is available.
    pub fn resolve() -> Result<Self, NoConfigDir> {
        std::env::var_os(CONFIG_DIR_ENV)
            .map(PathBuf::from)
            .or_else(grimvault_core::platform::app_config_dir)
            .map(Self)
            .ok_or(NoConfigDir)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    #[must_use]
    pub fn settings_file(&self) -> PathBuf {
        self.0.join(SETTINGS_FILE)
    }

    #[must_use]
    pub fn store_file(&self) -> PathBuf {
        self.0.join(STORE_FILE)
    }
}

/// What setup decided. Raw paths: the directories are re-validated
/// against the platform markers every time they are used, because a
/// mount can vanish between sessions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub game_dir: PathBuf,
    pub save_dir: PathBuf,
}

/// Why a settings file was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsProblem {
    #[error("not a {FORMAT_TAG} document: format tag {found:?}")]
    WrongFormat { found: String },
    #[error("settings version {version} is newer than this app's {FORMAT_VERSION}")]
    Newer { version: u32 },
    #[error("settings JSON: {0}")]
    Json(String),
}

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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    format: String,
    version: u32,
    #[serde(flatten)]
    settings: Settings,
}

impl Settings {
    /// Parses a settings document.
    ///
    /// # Errors
    /// [`SettingsProblem`] for a foreign tag, a newer version, or
    /// malformed JSON.
    pub fn parse(bytes: &[u8]) -> Result<Self, SettingsProblem> {
        let file: SettingsFile = serde_json::from_slice(bytes)
            .map_err(|error| SettingsProblem::Json(error.to_string()))?;
        if file.format != FORMAT_TAG {
            return Err(SettingsProblem::WrongFormat { found: file.format });
        }
        if file.version > FORMAT_VERSION {
            return Err(SettingsProblem::Newer {
                version: file.version,
            });
        }
        Ok(file.settings)
    }

    /// The document, pretty-printed with a trailing newline.
    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        let file = SettingsFile {
            format: FORMAT_TAG.to_string(),
            version: FORMAT_VERSION,
            settings: self.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&file)
            .expect("settings serialize infallibly: string keys, no fallible Serialize");
        bytes.push(b'\n');
        bytes
    }
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
    fn settings_round_trip_with_the_documented_envelope() {
        let bytes = sample().to_json();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.contains("\"format\": \"grimvault-settings\""),
            "{text}"
        );
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(text.contains("\"gameDir\""), "{text}");
        assert!(text.ends_with('\n'));
        assert_eq!(Settings::parse(&bytes).unwrap(), sample());
    }

    #[test]
    fn foreign_newer_and_malformed_documents_are_refused() {
        assert_eq!(
            Settings::parse(
                br#"{"format":"grimvault-store","version":1,"gameDir":"a","saveDir":"b"}"#
            ),
            Err(SettingsProblem::WrongFormat {
                found: "grimvault-store".into()
            })
        );
        assert_eq!(
            Settings::parse(
                br#"{"format":"grimvault-settings","version":2,"gameDir":"a","saveDir":"b"}"#
            ),
            Err(SettingsProblem::Newer { version: 2 })
        );
        assert!(matches!(
            Settings::parse(b"{"),
            Err(SettingsProblem::Json(_))
        ));
    }

    #[test]
    fn config_dir_names_its_files() {
        let dir = ConfigDir(PathBuf::from("/cfg/grim-vault"));
        assert_eq!(
            dir.settings_file(),
            PathBuf::from("/cfg/grim-vault/settings.json")
        );
        assert_eq!(
            dir.store_file(),
            PathBuf::from("/cfg/grim-vault/vault-store.json")
        );
    }

    #[test]
    fn load_and_save_round_trip_through_a_scratch_directory() {
        let scratch =
            std::env::temp_dir().join(format!("grimvault-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let dir = ConfigDir(scratch.clone());
        assert!(load(&dir).unwrap().is_none());
        save(&dir, &sample()).unwrap();
        assert_eq!(load(&dir).unwrap(), Some(sample()));
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
