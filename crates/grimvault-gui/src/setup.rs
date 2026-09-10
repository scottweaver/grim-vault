//! First-run setup: the game and save directories, validated into
//! [`GameDir`] and [`SaveDir`] by `grimvault_io` — the only forms the
//! loader accepts — and seeded with the candidates that exist here.

use std::path::{Path, PathBuf};

pub use grimvault_io::dirs::{DirProblem, GameDir, SaveDir};
use grimvault_io::dirs::{existing_game_candidates, existing_save_candidates};

use crate::settings::Settings;

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
    /// The settings this screen was reached from, whose remembered
    /// campaign and standing orders survive a change of directories.
    seed: Option<Settings>,
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
            seed: seed.cloned(),
        }
    }

    pub fn game_dir(&self) -> Result<GameDir, DirProblem> {
        GameDir::parse(Path::new(&self.game_field))
    }

    pub fn save_dir(&self) -> Result<SaveDir, DirProblem> {
        SaveDir::parse(Path::new(&self.save_field))
    }

    /// The raw settings to persist once both directories validate:
    /// the fields' directories under the seed's standing orders.
    #[must_use]
    pub fn settings(&self) -> Settings {
        let game_dir = PathBuf::from(&self.game_field);
        let save_dir = PathBuf::from(&self.save_field);
        self.seed.as_ref().map_or_else(
            || Settings::for_dirs(game_dir.clone(), save_dir.clone()),
            |seed| seed.with_dirs(game_dir.clone(), save_dir.clone()),
        )
    }
}

#[cfg(test)]
mod tests {
    use grimvault_core::campaign::{Campaign, ModName};

    use super::*;

    #[test]
    fn setup_state_seeds_from_settings_and_reports_problems() {
        let mut settings = Settings::for_dirs(
            PathBuf::from("/nowhere/game"),
            PathBuf::from("/nowhere/save"),
        );
        settings.sync_reagents = grimvault_core::settings::ReagentSync::Off;
        settings.campaign = Some(Campaign::Mod(ModName::parse("LootAscension").unwrap()));
        let mut state = SetupState::discover(Some(&settings), Some("note".into()));
        assert_eq!(state.game_field, "/nowhere/game");
        assert_eq!(state.settings(), settings);
        state.save_field = "/elsewhere/save".into();
        assert_eq!(
            state.settings(),
            settings.with_dirs(
                PathBuf::from("/nowhere/game"),
                PathBuf::from("/elsewhere/save")
            )
        );
        assert!(matches!(state.game_dir(), Err(DirProblem::Missing(_))));
        assert!(matches!(state.save_dir(), Err(DirProblem::Missing(_))));
    }
}
