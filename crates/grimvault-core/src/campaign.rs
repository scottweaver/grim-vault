//! Which set of shared save files is in play. The main campaign keeps
//! its shared files beside the character folders (`save/transfer.gst`,
//! `save/reagents.gst`, ...); every mod keeps its own set in a folder
//! named after it (`save/<Mod>/transfer.gst`, ...), and each `.gst`
//! carries that mod name inside. Nothing names the campaign a
//! *character* belongs to — `player.gdc` has no such field, and the
//! game lists every `user/` character under every mod — so a campaign
//! is a selection a shell makes, never a fact read from a character
//! (ARCHITECTURE.md "Source of truth").

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A mod's name as the game spells it: its folder under `save/` and
/// under the install's `mods/`, and the `mod_name` inside its `.gst`
/// files. Never one of the reserved character folders.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModName(String);

/// Why a string is not a mod name.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ModNameError {
    #[error("a mod name cannot be empty")]
    Empty,
    #[error("{0:?} is a reserved save folder, not a mod")]
    Reserved(String),
    #[error("{0:?} is not a plain folder name")]
    NotAFolderName(String),
}

impl ModName {
    /// Accepts a plain folder name that is not `main` or `user`.
    ///
    /// # Errors
    /// [`ModNameError`].
    pub fn parse(raw: &str) -> Result<Self, ModNameError> {
        if raw.is_empty() {
            return Err(ModNameError::Empty);
        }
        if raw.eq_ignore_ascii_case("main") || raw.eq_ignore_ascii_case("user") {
            return Err(ModNameError::Reserved(raw.to_owned()));
        }
        if raw == "." || raw == ".." || raw.contains(['/', '\\', '\0']) {
            return Err(ModNameError::NotAFolderName(raw.to_owned()));
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The shared save files in play: the main campaign's, or one mod's.
/// Serialized as `"main"` or the mod's name — a mod cannot be called
/// `main`, so the two never collide.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Campaign {
    Main,
    Mod(ModName),
}

impl Campaign {
    const MAIN_TAG: &str = "main";

    /// The folder holding this campaign's shared files: the save
    /// directory itself for the main campaign, `save/<Mod>/` for a mod.
    #[must_use]
    pub fn shared_dir(&self, save_dir: &Path) -> PathBuf {
        match self {
            Self::Main => save_dir.to_path_buf(),
            Self::Mod(name) => save_dir.join(name.as_str()),
        }
    }

    /// The `mod_name` the game writes inside this campaign's `.gst`
    /// files: empty for the main campaign. A file's own value against
    /// its folder is the cross-check that catches a misplaced file.
    #[must_use]
    pub fn wire_name(&self) -> &str {
        match self {
            Self::Main => "",
            Self::Mod(name) => name.as_str(),
        }
    }

    /// Whether a `.gst` claiming `mod_name` belongs here.
    #[must_use]
    pub fn owns_file_naming(&self, mod_name: &str) -> bool {
        self.wire_name().eq_ignore_ascii_case(mod_name)
    }
}

impl fmt::Display for Campaign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Main => f.write_str("main campaign"),
            Self::Mod(name) => f.write_str(name.as_str()),
        }
    }
}

impl From<Campaign> for String {
    fn from(campaign: Campaign) -> Self {
        match campaign {
            Campaign::Main => Campaign::MAIN_TAG.to_owned(),
            Campaign::Mod(name) => name.0,
        }
    }
}

impl TryFrom<String> for Campaign {
    type Error = ModNameError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        if raw == Self::MAIN_TAG {
            Ok(Self::Main)
        } else {
            ModName::parse(&raw).map(Self::Mod)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_names_are_plain_folder_names_other_than_the_character_folders() {
        assert_eq!(
            ModName::parse("LootAscension").unwrap().as_str(),
            "LootAscension"
        );
        assert_eq!(ModName::parse(""), Err(ModNameError::Empty));
        assert_eq!(
            ModName::parse("Main"),
            Err(ModNameError::Reserved("Main".into()))
        );
        assert_eq!(
            ModName::parse("user"),
            Err(ModNameError::Reserved("user".into()))
        );
        for bad in ["..", ".", "a/b", "a\\b"] {
            assert!(matches!(
                ModName::parse(bad),
                Err(ModNameError::NotAFolderName(_))
            ));
        }
    }

    #[test]
    fn a_campaign_names_its_folder_and_its_wire_name() {
        let save = Path::new("/saves");
        assert_eq!(Campaign::Main.shared_dir(save), PathBuf::from("/saves"));
        assert_eq!(Campaign::Main.wire_name(), "");
        let loot = Campaign::Mod(ModName::parse("LootAscension").unwrap());
        assert_eq!(loot.shared_dir(save), PathBuf::from("/saves/LootAscension"));
        assert_eq!(loot.wire_name(), "LootAscension");
        assert!(loot.owns_file_naming("lootascension"));
        assert!(!loot.owns_file_naming(""));
        assert!(Campaign::Main.owns_file_naming(""));
        assert_eq!(loot.to_string(), "LootAscension");
        assert_eq!(Campaign::Main.to_string(), "main campaign");
    }

    #[test]
    fn campaigns_serialize_as_main_or_the_mod_name() {
        let loot = Campaign::Mod(ModName::parse("LootAscension").unwrap());
        assert_eq!(serde_json::to_string(&Campaign::Main).unwrap(), "\"main\"");
        assert_eq!(serde_json::to_string(&loot).unwrap(), "\"LootAscension\"");
        assert_eq!(
            serde_json::from_str::<Campaign>("\"main\"").unwrap(),
            Campaign::Main
        );
        assert_eq!(
            serde_json::from_str::<Campaign>("\"LootAscension\"").unwrap(),
            loot
        );
        assert!(serde_json::from_str::<Campaign>("\"user\"").is_err());
        assert!(serde_json::from_str::<Campaign>("\"\"").is_err());
    }
}
