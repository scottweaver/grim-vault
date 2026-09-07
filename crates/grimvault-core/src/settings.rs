//! The persisted setup — where the game, the saves, and the vault
//! store live, which tabs empty themselves into the vault, whether the
//! component storage syncs, and whether bulk moves admit seed
//! duplicates — as one self-describing JSON file (`settings.json`,
//! format tag [`FORMAT_TAG`]) under the app's config directory, shared
//! by the desktop app and the command-line examples so a path given
//! once serves every tool. Pure: parsing and serializing only; reading
//! and writing the file is the shell's job. The config directory is
//! the platform's unless [`CONFIG_DIR_ENV`] overrides it, which is how
//! a headless run avoids creating the real one.
//!
//! The settings are machine-local (mount points differ between
//! machines); the vault store need not be. `storeFile` points the
//! store anywhere — a shared drive, say — so two machines each keep
//! their own settings over one vault; absent, the store sits beside
//! the settings as [`STORE_FILE`].

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::campaign::Campaign;
use crate::gdc::Realm;
use crate::platform::app_config_dir;
use crate::transfer::TabIndex;

/// Environment variable that replaces the platform config directory.
pub const CONFIG_DIR_ENV: &str = "GRIMVAULT_CONFIG_DIR";
/// The settings file's name inside the config directory.
pub const SETTINGS_FILE: &str = "settings.json";
/// The vault store's name inside the config directory.
pub const STORE_FILE: &str = "vault-store.json";
/// The desktop shell's persisted view state, beside the store.
pub const UI_STATE_FILE: &str = "ui-state.json";
/// The `format` tag every settings file carries.
pub const FORMAT_TAG: &str = "grimvault-settings";
/// The newest document version this build reads and the one it writes.
pub const FORMAT_VERSION: u32 = 1;

/// The directory holding this app's own files. Existence is not
/// implied; whoever writes into it creates it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigDir(PathBuf);

/// Neither the override variable nor the platform names a config
/// directory.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
#[error("no config directory: set {CONFIG_DIR_ENV} or a home directory")]
pub struct NoConfigDir;

impl ConfigDir {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// [`CONFIG_DIR_ENV`] when set, else the platform's directory.
    ///
    /// # Errors
    /// [`NoConfigDir`] when neither is available.
    pub fn resolve() -> Result<Self, NoConfigDir> {
        std::env::var_os(CONFIG_DIR_ENV)
            .map(PathBuf::from)
            .or_else(app_config_dir)
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

    #[must_use]
    pub fn ui_state_file(&self) -> PathBuf {
        self.0.join(UI_STATE_FILE)
    }
}

/// A tab nominated to empty itself into the vault whenever the app
/// loads or reloads it. Each entry carries the tab's whole identity —
/// the campaign of a transfer-stash tab, the realm and name of a
/// character's own — so a nomination never depends on which files
/// happen to be open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AutoMoveTab {
    TransferStash {
        campaign: Campaign,
        tab: TabIndex,
    },
    CharacterStash {
        realm: Realm,
        name: String,
        tab: TabIndex,
    },
}

impl fmt::Display for AutoMoveTab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransferStash { campaign, tab } => {
                write!(f, "{campaign} transfer stash tab {}", tab.value() + 1)
            }
            Self::CharacterStash { realm, name, tab } => write!(
                f,
                "character {}/{name} stash tab {}",
                realm.dir_name(),
                tab.value() + 1
            ),
        }
    }
}

/// A standing order a tab can be nominated for. Both run at the same
/// moments, auto-move first, so a tab under both ends empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandingOrder {
    /// Empty the tab into the vault store, leaving what the store
    /// already holds.
    AutoMove,
    /// Delete every item the vault store already holds under the same
    /// record and roll seed.
    PurgeDuplicates,
}

impl StandingOrder {
    pub const ALL: [Self; 2] = [Self::AutoMove, Self::PurgeDuplicates];
}

impl fmt::Display for StandingOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AutoMove => "auto-move",
            Self::PurgeDuplicates => "purge",
        })
    }
}

/// Whether the open campaign's component / crafting-material storage
/// raises the vault's counts to its own on every load and reload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReagentSync {
    #[default]
    On,
    Off,
}

/// Whether a bulk move or copy into the store — a tab's "Move all" /
/// "Copy all" buttons and the auto-move standing order alike — admits
/// an item the store already holds under the same record and roll
/// seed. Single drags, double-clicks and right-clicks are deliberate
/// acts and always land.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BulkDuplicates {
    #[default]
    Skip,
    Allow,
}

impl fmt::Display for BulkDuplicates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Skip => "skipped",
            Self::Allow => "allowed",
        })
    }
}

/// What setup decided, where the vault store lives when not beside the
/// settings, the campaign the user selected last, and the vault's
/// standing orders. Raw paths: the directories are re-validated
/// against the platform markers every time they are used, because a
/// mount can vanish between sessions, and the store path is resolved
/// by [`Settings::store_file`]. The campaign is a preference, not a
/// fact about the saves: a shell re-checks that it still exists before
/// opening it. A file written before a field existed simply has none:
/// the store beside the settings, no campaign, no tab nominated for
/// either order, the sync on, bulk duplicates skipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub game_dir: PathBuf,
    pub save_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub campaign: Option<Campaign>,
    #[serde(default)]
    pub auto_move: Vec<AutoMoveTab>,
    #[serde(default)]
    pub purge_duplicates: Vec<AutoMoveTab>,
    #[serde(default)]
    pub sync_reagents: ReagentSync,
    #[serde(default)]
    pub bulk_duplicates: BulkDuplicates,
}

/// Why a settings document was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsProblem {
    #[error("not a {FORMAT_TAG} document: format tag {found:?}")]
    WrongFormat { found: String },
    #[error("settings version {version} is newer than this app's {FORMAT_VERSION}")]
    Newer { version: u32 },
    #[error("settings JSON: {0}")]
    Json(String),
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
    /// The settings for two directories: the store beside the
    /// settings, no campaign remembered, no tab nominated, the sync
    /// on, bulk duplicates skipped.
    #[must_use]
    pub fn for_dirs(game_dir: PathBuf, save_dir: PathBuf) -> Self {
        Self {
            game_dir,
            save_dir,
            store_file: None,
            campaign: None,
            auto_move: Vec::new(),
            purge_duplicates: Vec::new(),
            sync_reagents: ReagentSync::default(),
            bulk_duplicates: BulkDuplicates::default(),
        }
    }

    /// The same store, remembered campaign, and standing orders over
    /// other directories.
    #[must_use]
    pub fn with_dirs(&self, game_dir: PathBuf, save_dir: PathBuf) -> Self {
        Self {
            game_dir,
            save_dir,
            ..self.clone()
        }
    }

    /// Where the vault store lives: `storeFile` when set — an absolute
    /// path as given, a relative one under the config directory — else
    /// [`STORE_FILE`] beside the settings.
    #[must_use]
    pub fn store_file(&self, config: &ConfigDir) -> PathBuf {
        self.store_file
            .as_deref()
            .map_or_else(|| config.store_file(), |file| config.path().join(file))
    }

    /// The tabs nominated for `order`, in nomination order.
    #[must_use]
    pub fn nominations(&self, order: StandingOrder) -> &[AutoMoveTab] {
        match order {
            StandingOrder::AutoMove => &self.auto_move,
            StandingOrder::PurgeDuplicates => &self.purge_duplicates,
        }
    }

    fn nominations_mut(&mut self, order: StandingOrder) -> &mut Vec<AutoMoveTab> {
        match order {
            StandingOrder::AutoMove => &mut self.auto_move,
            StandingOrder::PurgeDuplicates => &mut self.purge_duplicates,
        }
    }

    #[must_use]
    pub fn is_nominated(&self, order: StandingOrder, tab: &AutoMoveTab) -> bool {
        self.nominations(order).contains(tab)
    }

    /// Adds `tab` to `order`'s nominations; adding it twice is once.
    pub fn nominate(&mut self, order: StandingOrder, tab: AutoMoveTab) {
        if !self.is_nominated(order, &tab) {
            self.nominations_mut(order).push(tab);
        }
    }

    pub fn withdraw(&mut self, order: StandingOrder, tab: &AutoMoveTab) {
        self.nominations_mut(order)
            .retain(|nominated| nominated != tab);
    }

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
    #[expect(
        clippy::missing_panics_doc,
        reason = "serializing paths, enums and plain lists cannot fail; the expect states the invariant"
    )]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign::ModName;

    fn sample() -> Settings {
        Settings::for_dirs(
            PathBuf::from("/games/Grim Dawn"),
            PathBuf::from("/saves/save"),
        )
    }

    fn loot_ascension() -> Campaign {
        Campaign::Mod(ModName::parse("LootAscension").unwrap())
    }

    fn mod_tab() -> AutoMoveTab {
        AutoMoveTab::TransferStash {
            campaign: loot_ascension(),
            tab: TabIndex::new(3),
        }
    }

    fn own_tab() -> AutoMoveTab {
        AutoMoveTab::CharacterStash {
            realm: Realm::Custom,
            name: "Zark".into(),
            tab: TabIndex::new(0),
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
        assert!(text.contains("\"syncReagents\": \"on\""), "{text}");
        assert!(text.contains("\"bulkDuplicates\": \"skip\""), "{text}");
        assert!(text.ends_with('\n'));
        assert!(!text.contains("campaign"), "{text}");
        assert_eq!(Settings::parse(&bytes).unwrap(), sample());
    }

    #[test]
    fn the_remembered_campaign_round_trips_and_an_older_file_has_none() {
        let remembered = Settings {
            campaign: Some(loot_ascension()),
            ..sample()
        };
        let bytes = remembered.to_json();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("\"campaign\": \"LootAscension\""), "{text}");
        assert_eq!(Settings::parse(&bytes).unwrap(), remembered);
        assert_eq!(
            Settings::parse(
                br#"{"format":"grimvault-settings","version":1,"gameDir":"a","saveDir":"b","campaign":"main"}"#
            )
            .unwrap()
            .campaign,
            Some(Campaign::Main)
        );
        assert_eq!(
            Settings::parse(
                br#"{"format":"grimvault-settings","version":1,"gameDir":"a","saveDir":"b"}"#
            )
            .unwrap()
            .campaign,
            None
        );
        assert!(matches!(
            Settings::parse(
                br#"{"format":"grimvault-settings","version":1,"gameDir":"a","saveDir":"b","campaign":"user"}"#
            ),
            Err(SettingsProblem::Json(_))
        ));
    }

    #[test]
    fn nominations_carry_their_whole_identity_and_survive_the_round_trip() {
        let mut settings = sample();
        settings.nominate(StandingOrder::AutoMove, mod_tab());
        settings.nominate(StandingOrder::AutoMove, own_tab());
        settings.nominate(StandingOrder::AutoMove, mod_tab());
        settings.nominate(StandingOrder::PurgeDuplicates, own_tab());
        settings.sync_reagents = ReagentSync::Off;
        settings.bulk_duplicates = BulkDuplicates::Allow;
        assert_eq!(settings.auto_move, vec![mod_tab(), own_tab()]);
        assert_eq!(settings.purge_duplicates, vec![own_tab()]);
        assert_eq!(
            settings.nominations(StandingOrder::AutoMove),
            &[mod_tab(), own_tab()]
        );
        assert_eq!(
            settings.nominations(StandingOrder::PurgeDuplicates),
            &[own_tab()]
        );
        let bytes = settings.to_json();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.contains("\"kind\": \"transferStash\"")
                && text.contains("\"campaign\": \"LootAscension\""),
            "{text}"
        );
        assert!(
            text.contains("\"kind\": \"characterStash\"") && text.contains("\"realm\": \"custom\""),
            "{text}"
        );
        assert!(text.contains("\"purgeDuplicates\": ["), "{text}");
        assert!(text.contains("\"syncReagents\": \"off\""), "{text}");
        assert!(text.contains("\"bulkDuplicates\": \"allow\""), "{text}");
        assert_eq!(Settings::parse(&bytes).unwrap(), settings);
        settings.withdraw(StandingOrder::AutoMove, &mod_tab());
        assert_eq!(settings.auto_move, vec![own_tab()]);
        assert!(!settings.is_nominated(StandingOrder::AutoMove, &mod_tab()));
        assert!(settings.is_nominated(StandingOrder::PurgeDuplicates, &own_tab()));
        assert!(!settings.is_nominated(StandingOrder::PurgeDuplicates, &mod_tab()));
        settings.withdraw(StandingOrder::PurgeDuplicates, &own_tab());
        assert!(settings.purge_duplicates.is_empty());
        assert_eq!(settings.auto_move, vec![own_tab()]);
        settings.campaign = Some(loot_ascension());
        settings.nominate(StandingOrder::PurgeDuplicates, mod_tab());
        let moved = settings.with_dirs("/g".into(), "/s".into());
        assert_eq!(moved.auto_move, vec![own_tab()]);
        assert_eq!(moved.purge_duplicates, vec![mod_tab()]);
        assert_eq!(moved.campaign, Some(loot_ascension()));
        assert_eq!(moved.sync_reagents, ReagentSync::Off);
        assert_eq!(moved.bulk_duplicates, BulkDuplicates::Allow);
    }

    #[test]
    fn a_file_from_before_the_standing_orders_reads_with_none_and_the_sync_on() {
        let settings = Settings::parse(
            br#"{"format":"grimvault-settings","version":1,"gameDir":"/g","saveDir":"/s"}"#,
        )
        .unwrap();
        assert!(settings.auto_move.is_empty());
        assert!(settings.purge_duplicates.is_empty());
        assert_eq!(settings.sync_reagents, ReagentSync::On);
        assert_eq!(settings.bulk_duplicates, BulkDuplicates::Skip);
        let named = Settings::parse(
            br#"{"format":"grimvault-settings","version":1,"gameDir":"/g","saveDir":"/s","purgeDuplicates":[{"kind":"transferStash","campaign":"main","tab":2}]}"#,
        )
        .unwrap();
        assert_eq!(
            named.purge_duplicates,
            vec![AutoMoveTab::TransferStash {
                campaign: Campaign::Main,
                tab: TabIndex::new(2)
            }]
        );
        assert!(named.auto_move.is_empty());
        assert_eq!(StandingOrder::AutoMove.to_string(), "auto-move");
        assert_eq!(StandingOrder::PurgeDuplicates.to_string(), "purge");
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
    fn the_store_sits_beside_the_settings_unless_a_store_file_says_otherwise() {
        let config = ConfigDir::new(PathBuf::from("/cfg/grim-vault"));
        let beside = sample();
        assert_eq!(beside.store_file(&config), config.store_file());
        let text = String::from_utf8(beside.to_json()).unwrap();
        assert!(!text.contains("storeFile"), "{text}");

        let shared = Settings {
            store_file: Some(PathBuf::from("/nas/grim-vault/vault-store.json")),
            ..sample()
        };
        assert_eq!(
            shared.store_file(&config),
            PathBuf::from("/nas/grim-vault/vault-store.json")
        );
        let bytes = shared.to_json();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.contains("\"storeFile\": \"/nas/grim-vault/vault-store.json\""),
            "{text}"
        );
        assert_eq!(Settings::parse(&bytes).unwrap(), shared);
        assert_eq!(
            shared.with_dirs("/g".into(), "/s".into()).store_file,
            shared.store_file
        );

        let relative = Settings {
            store_file: Some(PathBuf::from("vaults/mine.json")),
            ..sample()
        };
        assert_eq!(
            relative.store_file(&config),
            PathBuf::from("/cfg/grim-vault/vaults/mine.json")
        );
        assert_eq!(
            Settings::parse(
                br#"{"format":"grimvault-settings","version":1,"gameDir":"a","saveDir":"b"}"#
            )
            .unwrap()
            .store_file,
            None
        );
    }

    #[test]
    fn config_dir_names_its_files() {
        let dir = ConfigDir::new(PathBuf::from("/cfg/grim-vault"));
        assert_eq!(
            dir.settings_file(),
            PathBuf::from("/cfg/grim-vault/settings.json")
        );
        assert_eq!(
            dir.store_file(),
            PathBuf::from("/cfg/grim-vault/vault-store.json")
        );
        assert_eq!(
            dir.ui_state_file(),
            PathBuf::from("/cfg/grim-vault/ui-state.json")
        );
    }
}
