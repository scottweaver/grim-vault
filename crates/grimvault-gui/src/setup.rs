//! First-run setup: the game and save directories, validated by the
//! platform module's markers into [`GameDir`] and [`SaveDir`] — the
//! only forms the loader accepts — and seeded with the candidates the
//! platform module derives, filtered to the ones that exist here.

use std::fmt;
use std::path::{Path, PathBuf};

use grimvault_core::platform::{
    GAME_DIR_MARKER, SAVE_DIR_MARKERS, game_dir_candidates, save_dir_candidates,
    steam_cloud_save_dir,
};
use thiserror::Error;

use crate::settings::Settings;

/// A directory proven to hold `database/database.arz`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameDir(PathBuf);

/// A directory proven to hold `transfer.gst` beside `main/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveDir(PathBuf);

/// Which directory a problem is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirRole {
    Game,
    Save,
}

impl fmt::Display for DirRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Game => "Grim Dawn install",
            Self::Save => "save directory",
        })
    }
}

/// Why a directory was not accepted.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DirProblem {
    #[error("choose a {0}")]
    Empty(DirRole),
    #[error("{} does not exist", .0.display())]
    Missing(PathBuf),
    #[error("{} has no {marker}, so it is not a {role}", dir.display())]
    Unmarked {
        dir: PathBuf,
        marker: &'static str,
        role: DirRole,
    },
}

impl GameDir {
    /// Accepts a directory that carries [`GAME_DIR_MARKER`].
    ///
    /// # Errors
    /// [`DirProblem`] naming what is missing.
    pub fn parse(raw: &Path) -> Result<Self, DirProblem> {
        check_dir(raw, DirRole::Game)?;
        check_marker(raw, GAME_DIR_MARKER, DirRole::Game)?;
        Ok(Self(raw.to_path_buf()))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl SaveDir {
    /// Accepts a directory that carries every [`SAVE_DIR_MARKERS`]
    /// entry.
    ///
    /// # Errors
    /// [`DirProblem`] naming what is missing.
    pub fn parse(raw: &Path) -> Result<Self, DirProblem> {
        check_dir(raw, DirRole::Save)?;
        SAVE_DIR_MARKERS
            .iter()
            .try_for_each(|marker| check_marker(raw, marker, DirRole::Save))?;
        Ok(Self(raw.to_path_buf()))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The shared stash file.
    #[must_use]
    pub fn transfer_stash(&self) -> PathBuf {
        self.0.join("transfer.gst")
    }

    /// The directory of per-character folders.
    #[must_use]
    pub fn characters_dir(&self) -> PathBuf {
        self.0.join("main")
    }
}

fn check_dir(raw: &Path, role: DirRole) -> Result<(), DirProblem> {
    if raw.as_os_str().is_empty() {
        return Err(DirProblem::Empty(role));
    }
    if !raw.is_dir() {
        return Err(DirProblem::Missing(raw.to_path_buf()));
    }
    Ok(())
}

fn check_marker(raw: &Path, marker: &'static str, role: DirRole) -> Result<(), DirProblem> {
    if raw.join(marker).exists() {
        Ok(())
    } else {
        Err(DirProblem::Unmarked {
            dir: raw.to_path_buf(),
            marker,
            role,
        })
    }
}

/// The setup screen's working state. The fields are re-validated from
/// their text every frame, so a verdict is never stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupState {
    pub game_field: String,
    pub save_field: String,
    pub game_candidates: Vec<PathBuf>,
    pub save_candidates: Vec<PathBuf>,
    /// Why the app is on this screen when it is not the first run.
    pub note: Option<String>,
}

impl SetupState {
    /// Seeds the fields from earlier settings (when any) and discovers
    /// the candidates that exist on this machine.
    #[must_use]
    pub fn discover(seed: Option<&Settings>, note: Option<String>) -> Self {
        let game_candidates = existing_game_candidates();
        let save_candidates = existing_save_candidates();
        let game_field = seed
            .map(|settings| settings.game_dir.clone())
            .or_else(|| game_candidates.first().cloned())
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let save_field = seed
            .map(|settings| settings.save_dir.clone())
            .or_else(|| save_candidates.first().cloned())
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        Self {
            game_field,
            save_field,
            game_candidates,
            save_candidates,
            note,
        }
    }

    pub fn game_dir(&self) -> Result<GameDir, DirProblem> {
        GameDir::parse(Path::new(&self.game_field))
    }

    pub fn save_dir(&self) -> Result<SaveDir, DirProblem> {
        SaveDir::parse(Path::new(&self.save_field))
    }

    /// The raw settings to persist once both directories validate.
    #[must_use]
    pub fn settings(&self) -> Settings {
        Settings {
            game_dir: PathBuf::from(&self.game_field),
            save_dir: PathBuf::from(&self.save_field),
        }
    }
}

/// Game install candidates that exist.
#[must_use]
pub fn existing_game_candidates() -> Vec<PathBuf> {
    game_dir_candidates()
        .into_iter()
        .filter(|path| path.is_dir())
        .collect()
}

/// Save root candidates that exist; a Steam `userdata` candidate is
/// expanded into the per-account cloud save roots beneath it.
#[must_use]
pub fn existing_save_candidates() -> Vec<PathBuf> {
    save_dir_candidates()
        .into_iter()
        .flat_map(|candidate| {
            if candidate.file_name().is_some_and(|name| name == "userdata") {
                steam_account_save_dirs(&candidate)
            } else {
                vec![candidate]
            }
        })
        .filter(|path| path.is_dir())
        .collect()
}

fn steam_account_save_dirs(userdata: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(userdata) else {
        return Vec::new();
    };
    let mut accounts: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|account| steam_cloud_save_dir(&account))
        .collect();
    accounts.sort();
    accounts
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("grimvault-setup-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn game_dir_needs_the_database_marker() {
        let scratch = Scratch::new("game");
        assert_eq!(
            GameDir::parse(Path::new("")),
            Err(DirProblem::Empty(DirRole::Game))
        );
        assert!(matches!(
            GameDir::parse(&scratch.0.join("nope")),
            Err(DirProblem::Missing(_))
        ));
        assert!(matches!(
            GameDir::parse(&scratch.0),
            Err(DirProblem::Unmarked {
                marker: GAME_DIR_MARKER,
                role: DirRole::Game,
                ..
            })
        ));
        std::fs::create_dir_all(scratch.0.join("database")).unwrap();
        std::fs::write(scratch.0.join(GAME_DIR_MARKER), b"arz").unwrap();
        assert_eq!(GameDir::parse(&scratch.0).unwrap().path(), scratch.0);
    }

    #[test]
    fn save_dir_needs_every_marker_and_names_its_files() {
        let scratch = Scratch::new("save");
        std::fs::write(scratch.0.join("transfer.gst"), b"gst").unwrap();
        assert!(matches!(
            SaveDir::parse(&scratch.0),
            Err(DirProblem::Unmarked {
                marker: "main",
                role: DirRole::Save,
                ..
            })
        ));
        std::fs::create_dir_all(scratch.0.join("main")).unwrap();
        let save = SaveDir::parse(&scratch.0).unwrap();
        assert_eq!(save.transfer_stash(), scratch.0.join("transfer.gst"));
        assert_eq!(save.characters_dir(), scratch.0.join("main"));
    }

    #[test]
    fn steam_accounts_expand_to_cloud_save_roots_in_order() {
        let scratch = Scratch::new("steam");
        std::fs::create_dir_all(scratch.0.join("222")).unwrap();
        std::fs::create_dir_all(scratch.0.join("111")).unwrap();
        std::fs::write(scratch.0.join("not-a-dir"), b"").unwrap();
        assert_eq!(
            steam_account_save_dirs(&scratch.0),
            vec![
                steam_cloud_save_dir(&scratch.0.join("111")),
                steam_cloud_save_dir(&scratch.0.join("222")),
            ]
        );
    }

    #[test]
    fn setup_state_seeds_from_settings_and_reports_problems() {
        let settings = Settings {
            game_dir: PathBuf::from("/nowhere/game"),
            save_dir: PathBuf::from("/nowhere/save"),
        };
        let state = SetupState::discover(Some(&settings), Some("note".into()));
        assert_eq!(state.game_field, "/nowhere/game");
        assert_eq!(state.settings(), settings);
        assert!(matches!(state.game_dir(), Err(DirProblem::Missing(_))));
        assert!(matches!(state.save_dir(), Err(DirProblem::Missing(_))));
    }
}
