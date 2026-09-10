//! The game and save directories, validated by the platform module's
//! markers into [`GameDir`] and [`SaveDir`] — the only forms the
//! loaders accept — and the files each one names. The candidates the
//! platform module derives are filtered here to the ones that exist
//! on this machine.

use std::fmt;
use std::path::{Path, PathBuf};

use grimvault_core::campaign::{Campaign, ModName};
use grimvault_core::gdc::Realm;
use grimvault_core::platform::{
    GAME_DIR_MARKER, SAVE_DIR_MARKERS, game_dir_candidates, save_dir_candidates,
    steam_cloud_save_dir,
};
use thiserror::Error;

/// A directory proven to hold `database/database.arz`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameDir(PathBuf);

/// A directory proven to hold `transfer.gst` beside `main/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveDir(PathBuf);

/// Which directory a problem is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirRole {
    /// The Grim Dawn install.
    Game,
    /// The save directory.
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
    /// No path was given.
    #[error("choose a {0}")]
    Empty(DirRole),
    /// The path is not a directory.
    #[error("{} does not exist", .0.display())]
    Missing(PathBuf),
    /// The directory lacks the file that would prove its role.
    #[error("{} has no {marker}, so it is not a {role}", dir.display())]
    Unmarked {
        /// The directory offered.
        dir: PathBuf,
        /// The missing marker, relative to it.
        marker: &'static str,
        /// The role it was offered for.
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

    /// The directory itself.
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

    /// The directory itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// A campaign's shared stash file.
    #[must_use]
    pub fn transfer_stash(&self, campaign: &Campaign) -> PathBuf {
        campaign.shared_dir(&self.0).join("transfer.gst")
    }

    /// A campaign's component / crafting-material storage file. Not a
    /// marker: a save directory the game has not yet written it into
    /// is still a save directory.
    #[must_use]
    pub fn reagent_storage(&self, campaign: &Campaign) -> PathBuf {
        campaign.shared_dir(&self.0).join("reagents.gst")
    }

    /// A campaign's blueprint list, `formulas.gst`; absent until the
    /// first blueprint is learned there.
    #[must_use]
    pub fn blueprints(&self, campaign: &Campaign) -> PathBuf {
        campaign.shared_dir(&self.0).join("formulas.gst")
    }

    /// A campaign's illusion collection, `transmutes.gst`; absent until
    /// the first illusion is unlocked there.
    #[must_use]
    pub fn illusions(&self, campaign: &Campaign) -> PathBuf {
        campaign.shared_dir(&self.0).join("transmutes.gst")
    }

    /// The campaigns this save directory holds: the main campaign,
    /// then every folder with its own `transfer.gst`, by name — the
    /// game creates such a folder the first time a mod is played.
    #[must_use]
    pub fn campaigns(&self) -> Vec<Campaign> {
        let mut mods: Vec<ModName> = std::fs::read_dir(&self.0)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.path().join("transfer.gst").is_file())
                    .filter_map(|entry| ModName::parse(&entry.file_name().to_string_lossy()).ok())
                    .collect()
            })
            .unwrap_or_default();
        mods.sort();
        std::iter::once(Campaign::Main)
            .chain(mods.into_iter().map(Campaign::Mod))
            .collect()
    }

    /// The per-character folders of `realm`: `main/` for the main
    /// campaign, `user/` for custom games (mods). Only `main/` is a
    /// marker; `user/` appears once a custom-game character exists.
    #[must_use]
    pub fn characters_dir(&self, realm: Realm) -> PathBuf {
        self.0.join(realm.dir_name())
    }

    /// The campaign whose `transfer.gst` the game wrote most recently
    /// — the best witness of what is being played, since neither a
    /// character file nor the game names a character's mod; the main
    /// campaign on a tie or when no stamp can be read.
    #[must_use]
    pub fn campaign_written_last(&self) -> Campaign {
        let mut newest: Option<(Campaign, std::time::SystemTime)> = None;
        for campaign in self.campaigns() {
            let Some(modified) = std::fs::metadata(self.transfer_stash(&campaign))
                .and_then(|meta| meta.modified())
                .ok()
            else {
                continue;
            };
            if newest.as_ref().is_none_or(|(_, held)| modified > *held) {
                newest = Some((campaign, modified));
            }
        }
        newest.map_or(Campaign::Main, |(campaign, _)| campaign)
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
                std::env::temp_dir().join(format!("grimvault-dirs-{name}-{}", std::process::id()));
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
        assert_eq!(
            save.transfer_stash(&Campaign::Main),
            scratch.0.join("transfer.gst")
        );
        assert_eq!(save.characters_dir(Realm::Main), scratch.0.join("main"));
        assert_eq!(save.characters_dir(Realm::Custom), scratch.0.join("user"));
        assert_eq!(save.campaigns(), vec![Campaign::Main]);

        for folder in ["Zeta", "LootAscension", "user", "NoStash"] {
            std::fs::create_dir_all(scratch.0.join(folder)).unwrap();
        }
        for folder in ["Zeta", "LootAscension", "user"] {
            std::fs::write(scratch.0.join(folder).join("transfer.gst"), b"gst").unwrap();
        }
        let loot = Campaign::Mod(ModName::parse("LootAscension").unwrap());
        let zeta = Campaign::Mod(ModName::parse("Zeta").unwrap());
        assert_eq!(save.campaigns(), vec![Campaign::Main, loot.clone(), zeta]);
        assert_eq!(
            save.transfer_stash(&loot),
            scratch.0.join("LootAscension/transfer.gst")
        );
        assert_eq!(
            save.reagent_storage(&loot),
            scratch.0.join("LootAscension/reagents.gst")
        );
        assert_eq!(
            save.blueprints(&loot),
            scratch.0.join("LootAscension/formulas.gst")
        );
        assert_eq!(
            save.illusions(&Campaign::Main),
            scratch.0.join("transmutes.gst")
        );
    }

    #[test]
    fn the_campaign_written_last_is_the_newest_stash_main_on_a_tie() {
        let scratch = Scratch::new("newest");
        std::fs::create_dir_all(scratch.0.join("main")).unwrap();
        std::fs::create_dir_all(scratch.0.join("Loot")).unwrap();
        std::fs::write(scratch.0.join("transfer.gst"), b"gst").unwrap();
        std::fs::write(scratch.0.join("Loot/transfer.gst"), b"gst").unwrap();
        let save = SaveDir::parse(&scratch.0).unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let newer = old + std::time::Duration::from_secs(60);
        let stamp = |path: PathBuf, time| {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };
        stamp(scratch.0.join("transfer.gst"), newer);
        stamp(scratch.0.join("Loot/transfer.gst"), old);
        assert_eq!(save.campaign_written_last(), Campaign::Main);
        stamp(
            scratch.0.join("Loot/transfer.gst"),
            newer + std::time::Duration::from_secs(1),
        );
        assert_eq!(
            save.campaign_written_last(),
            Campaign::Mod(ModName::parse("Loot").unwrap())
        );
        stamp(
            scratch.0.join("transfer.gst"),
            newer + std::time::Duration::from_secs(1),
        );
        assert_eq!(save.campaign_written_last(), Campaign::Main);
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
}
