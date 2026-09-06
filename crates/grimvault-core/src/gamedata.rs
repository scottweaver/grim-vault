//! Grim Dawn game-data facade: the layered record database (base,
//! then `gdx1`, then `gdx2`, later layers overriding earlier records
//! by path), the localization tags, and the item bitmap archives,
//! resolved into the facts the app needs about an item base or affix
//! record — display name, rarity, class, level requirement, and grid
//! footprint. Installed mods (`mods/<Mod>/`) are **fill** layers:
//! consulted only for what no shipped layer has, so a mod's override
//! of a shipped record never changes how a base-game item shows
//! ([`GameData::layered`]). Read-only reference data per
//! ARCHITECTURE.md; nothing here writes, and nothing here touches the
//! filesystem — [`shipped_layers`] and [`mod_layers`] name the files
//! and a shell reads them.
//!
//! Variable names follow the game's templates as observed in the real
//! database and in gdlc (MIT): `itemNameTag` (or `description` for
//! components and relics) names an item, `lootRandomizerName` names an
//! affix, `itemClassification` is the rarity, `levelRequirement` the
//! level gate, and the icon whose pixel size defines the footprint is
//! `bitmap` for gear and materials, `relicBitmap` for components
//! (`ItemRelic`, whose `shardBitmap` is the partial piece older game
//! versions dropped), `artifactBitmap` for relics, `emptyBitmap` for
//! transmuters (see [`BITMAP_VARIABLES`]).

use std::fmt;
use std::path::{Path, PathBuf};

use univault_engine::arc::{ArcError, ArcFile};
use univault_engine::arz::{ArzError, ArzFile, DbRecord};
use univault_engine::ids::{RecordId, normalize};
use univault_engine::tex::{self, TexError};
use univault_engine::text::TextDb;

use crate::reagents::ReagentKind;

/// Item quality as the game's `itemClassification` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rarity {
    Common,
    Magical,
    Rare,
    Epic,
    Legendary,
    Quest,
}

impl Rarity {
    /// `None` for a classification this app does not know, so an
    /// unrecognised quality never masquerades as a known one.
    #[must_use]
    pub fn parse(classification: &str) -> Option<Self> {
        match classification.to_ascii_lowercase().as_str() {
            "common" => Some(Self::Common),
            "magical" => Some(Self::Magical),
            "rare" => Some(Self::Rare),
            "epic" => Some(Self::Epic),
            "legendary" => Some(Self::Legendary),
            "quest" => Some(Self::Quest),
            _ => None,
        }
    }
}

/// The record's `Class` variable (`WeaponHunting_Ranged2h`,
/// `ArmorProtective_Head`, `ItemRelic`, ...). An open set the game
/// defines, kept verbatim until this app designs its own categories.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemClass(String);

impl ItemClass {
    /// Wraps a `Class` value verbatim; the game's spelling is the identity.
    #[must_use]
    pub fn new(class: String) -> Self {
        Self(class)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Path of an item bitmap inside the `Items.arc` archives, as the
/// `bitmap` variable spells it (`items/gearweapons/.../x.tex`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BitmapPath(String);

impl BitmapPath {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The archive entry name: `bitmap` values carry an `items/`
    /// prefix naming the archive, which the entries inside it omit.
    fn archive_entry(&self) -> &str {
        let normalized_prefix_len = "items/".len();
        if self.0.len() > normalized_prefix_len
            && self.0[..normalized_prefix_len].eq_ignore_ascii_case("items/")
        {
            &self.0[normalized_prefix_len..]
        } else {
            &self.0
        }
    }
}

/// The variables an item record may name its icon by, in lookup
/// order: gear and crafting materials use `bitmap`, components
/// (`ItemRelic`) `relicBitmap`, relics (`ItemArtifact`)
/// `artifactBitmap`, transmuters `emptyBitmap`.
pub const BITMAP_VARIABLES: [&str; 4] = ["bitmap", "relicBitmap", "artifactBitmap", "emptyBitmap"];

/// Grid footprint of an item in inventory cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Footprint {
    pub width: i32,
    pub height: i32,
}

/// What the database says about one item base record. `name` is the
/// localized name when the text tables have it, else the record's
/// `FileDescription` (the developers' working name), else the file
/// stem — never empty. `reagent` is the record's place in the
/// component / crafting-material storage, from its `Class` and
/// `craftingMaterial` flag ([`ReagentKind::of`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInfo {
    pub name: String,
    pub class: Option<ItemClass>,
    pub rarity: Option<Rarity>,
    pub level_requirement: Option<u32>,
    pub bitmap: Option<BitmapPath>,
    pub reagent: Option<ReagentKind>,
}

#[derive(Debug, thiserror::Error)]
pub enum GameDataError {
    #[error("record database: {0}")]
    Arz(#[from] ArzError),
    #[error("resource archive: {0}")]
    Arc(#[from] ArcError),
    #[error("item bitmap: {0}")]
    Tex(#[from] TexError),
}

/// The files of one game-data layer, relative to the game directory.
/// Any may be absent on disk (an expansion not installed, a mod
/// without text or icons); a loader skips what is not there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerFiles {
    pub database: PathBuf,
    pub text: PathBuf,
    pub items: PathBuf,
}

/// The shipped layers in overlay order: the base game, then each
/// expansion (`gdx1`, `gdx2`, and `gdx3` once installed).
#[must_use]
pub fn shipped_layers() -> Vec<LayerFiles> {
    let base = LayerFiles {
        database: PathBuf::from("database/database.arz"),
        text: PathBuf::from("resources/Text_EN.arc"),
        items: PathBuf::from("resources/Items.arc"),
    };
    let expansions = ["gdx1", "gdx2", "gdx3"].into_iter().map(|root| LayerFiles {
        database: Path::new(root)
            .join("database")
            .join(format!("{}.arz", root.to_uppercase())),
        text: Path::new(root).join("resources/Text_EN.arc"),
        items: Path::new(root).join("resources/Items.arc"),
    });
    std::iter::once(base).chain(expansions).collect()
}

/// What a shell found in one folder under `mods/`: its name and the
/// file names inside its `database/` and `resources/` sub-folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModListing {
    pub folder: String,
    pub database_files: Vec<String>,
    pub resource_files: Vec<String>,
}

/// The layer of every installed mod that ships a database, in
/// folder-name order so two mods defining the same record resolve the
/// same way every launch. File names match case-insensitively: a
/// mod's archives need not follow the shipped spelling
/// (`mods/survivalmode/database/SurvivalMode.arz`), and a mod with
/// several databases contributes the first by name.
#[must_use]
pub fn mod_layers(mods: impl IntoIterator<Item = ModListing>) -> Vec<LayerFiles> {
    let mut mods: Vec<ModListing> = mods.into_iter().collect();
    mods.sort_by(|a, b| a.folder.cmp(&b.folder));
    mods.iter().filter_map(mod_layer).collect()
}

fn mod_layer(listing: &ModListing) -> Option<LayerFiles> {
    let database = listing
        .database_files
        .iter()
        .filter(|name| has_extension(name, "arz"))
        .min()?;
    let root = Path::new("mods").join(&listing.folder);
    let resource = |canonical: &str| {
        let found = listing
            .resource_files
            .iter()
            .find(|name| name.eq_ignore_ascii_case(canonical))
            .map_or(canonical, String::as_str);
        root.join("resources").join(found)
    };
    Some(LayerFiles {
        database: root.join("database").join(database),
        text: resource("Text_EN.arc"),
        items: resource("Items.arc"),
    })
}

fn has_extension(name: &str, extension: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|found| found.eq_ignore_ascii_case(extension))
}

/// The archives of one origin — the shipped game, or the installed
/// mods — each list in overlay order (base first).
#[derive(Default)]
pub struct LayerSet {
    pub databases: Vec<ArzFile>,
    pub text_archives: Vec<ArcFile>,
    pub item_archives: Vec<ArcFile>,
}

/// The layered game data. Layers are in overlay order (base first);
/// every lookup walks them from the last to the first so an expansion
/// record or tag overrides the base game's.
pub struct GameData {
    databases: Vec<ArzFile>,
    text: TextDb,
    item_archives: Vec<ArcFile>,
}

impl GameData {
    #[must_use]
    pub fn from_parts(databases: Vec<ArzFile>, text: TextDb, item_archives: Vec<ArcFile>) -> Self {
        Self {
            databases,
            text,
            item_archives,
        }
    }

    /// Composes the game data: `shipped` layers override each other
    /// base-first, and `mods` are fill layers below them all, so a
    /// record, tag, or bitmap resolves from a mod only when no
    /// shipped layer has it (ARCHITECTURE.md "Source of truth").
    ///
    /// # Errors
    /// The first text archive entry that fails to inflate.
    pub fn layered(shipped: LayerSet, mods: LayerSet) -> Result<Self, ArcError> {
        let text_archives: Vec<ArcFile> = mods
            .text_archives
            .into_iter()
            .chain(shipped.text_archives)
            .collect();
        let text = text_from_archives(&text_archives)?;
        Ok(Self::from_parts(
            mods.databases
                .into_iter()
                .chain(shipped.databases)
                .collect(),
            text,
            mods.item_archives
                .into_iter()
                .chain(shipped.item_archives)
                .collect(),
        ))
    }

    /// The record from the topmost layer that has it.
    #[must_use]
    pub fn record(&self, id: &RecordId) -> Option<Result<DbRecord, ArzError>> {
        self.databases.iter().rev().find_map(|db| db.record(id))
    }

    /// Every record id across all layers, deduplicated by normalized
    /// path (an id shadowed by a later layer appears once).
    pub fn record_ids(&self) -> impl Iterator<Item = &RecordId> {
        let mut seen = std::collections::HashSet::new();
        self.databases
            .iter()
            .rev()
            .flat_map(ArzFile::record_ids)
            .filter(move |id| seen.insert(normalize(id.as_str())))
    }

    #[must_use]
    pub fn tag_text(&self, tag: &str) -> Option<&str> {
        self.text.get(tag)
    }

    /// `None` when no layer has the record.
    #[must_use]
    pub fn item_info(&self, id: &RecordId) -> Option<Result<ItemInfo, GameDataError>> {
        let record = match self.record(id)? {
            Ok(record) => record,
            Err(error) => return Some(Err(error.into())),
        };
        let name = self
            .localized_name(&record, &["itemNameTag", "description"])
            .or_else(|| record.string("FileDescription").map(str::to_string))
            .unwrap_or_else(|| id.file_stem().to_string());
        let class = record
            .string("Class")
            .map(|class| ItemClass(class.to_string()));
        let reagent = ReagentKind::of(
            class.as_ref(),
            record.boolean("craftingMaterial").unwrap_or(false),
        );
        Some(Ok(ItemInfo {
            name,
            class,
            rarity: record.string("itemClassification").and_then(Rarity::parse),
            level_requirement: record
                .integer("levelRequirement")
                .and_then(|level| u32::try_from(level).ok()),
            bitmap: BITMAP_VARIABLES
                .iter()
                .find_map(|variable| record.string(variable))
                .map(|bitmap| BitmapPath(bitmap.to_string())),
            reagent,
        }))
    }

    /// The localized name of an affix record (`lootRandomizerName`),
    /// `None` when the record is missing or has no localized name.
    #[must_use]
    pub fn affix_name(&self, id: &RecordId) -> Option<String> {
        let record = self.record(id)?.ok()?;
        self.localized_name(&record, &["lootRandomizerName"])
    }

    /// `prefix base suffix`, each part localized, parts the database
    /// cannot name omitted. `None` only when the base record itself is
    /// unknown.
    #[must_use]
    pub fn display_name(
        &self,
        base: &RecordId,
        prefix: Option<&RecordId>,
        suffix: Option<&RecordId>,
    ) -> Option<String> {
        let base_name = self.item_info(base)?.ok()?.name;
        let parts = [
            prefix.and_then(|id| self.affix_name(id)),
            Some(base_name),
            suffix.and_then(|id| self.affix_name(id)),
        ];
        Some(parts.into_iter().flatten().collect::<Vec<_>>().join(" "))
    }

    /// The `.tex` image of a bitmap, from the topmost archive that has
    /// it — the same bytes [`Self::footprint`] measures, for a shell
    /// that decodes the icon itself. `None` when no archive has the
    /// entry.
    #[must_use]
    pub fn bitmap(&self, bitmap: &BitmapPath) -> Option<Result<Vec<u8>, GameDataError>> {
        let entry = bitmap.archive_entry();
        self.item_archives
            .iter()
            .rev()
            .find_map(|archive| archive.file(entry))
            .map(|bytes| bytes.map_err(GameDataError::from))
    }

    /// Footprint of a bitmap from its pixel size, from the topmost
    /// archive that has it. `None` when no archive has the entry.
    #[must_use]
    pub fn footprint(&self, bitmap: &BitmapPath) -> Option<Result<Footprint, GameDataError>> {
        Some(self.bitmap(bitmap)?.and_then(|bytes| {
            let (width_px, height_px) = tex::dimensions(&bytes)?;
            let (width, height) = tex::cells(width_px, height_px);
            Ok(Footprint { width, height })
        }))
    }

    fn localized_name(&self, record: &DbRecord, tag_variables: &[&str]) -> Option<String> {
        tag_variables
            .iter()
            .find_map(|variable| record.string(variable))
            .and_then(|tag| self.text.get(tag))
            .map(strip_caret_codes)
    }
}

/// Builds the tag table from `Text_XX.arc` archives in overlay order:
/// every text file of every archive, later archives overriding
/// earlier tags.
///
/// # Errors
/// The first archive entry that fails to inflate.
pub fn text_from_archives(archives: &[ArcFile]) -> Result<TextDb, ArcError> {
    let mut text = TextDb::new();
    for archive in archives {
        let names: Vec<&str> = archive.file_names().collect();
        for name in names {
            if let Some(bytes) = archive.file(name) {
                text.add_file(&bytes?);
            }
        }
    }
    Ok(text)
}

/// Grim Dawn colours some labels with bare `^k`-style codes (a caret
/// and one letter) that the braces-only stripper in `TextDb` leaves
/// in place.
fn strip_caret_codes(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c == '^' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_layers_run_base_then_each_expansion() {
        let layers = shipped_layers();
        let databases: Vec<&Path> = layers
            .iter()
            .map(|layer| layer.database.as_path())
            .collect();
        assert_eq!(
            databases,
            [
                Path::new("database/database.arz"),
                Path::new("gdx1/database/GDX1.arz"),
                Path::new("gdx2/database/GDX2.arz"),
                Path::new("gdx3/database/GDX3.arz"),
            ]
        );
        assert_eq!(layers[0].text, Path::new("resources/Text_EN.arc"));
        assert_eq!(layers[2].items, Path::new("gdx2/resources/Items.arc"));
    }

    #[test]
    fn mod_layers_take_each_mods_database_by_name_in_folder_order() {
        let listed = |folder: &str, databases: &[&str], resources: &[&str]| ModListing {
            folder: folder.into(),
            database_files: databases.iter().map(|s| (*s).to_string()).collect(),
            resource_files: resources.iter().map(|s| (*s).to_string()).collect(),
        };
        let layers = mod_layers([
            listed(
                "survivalmode",
                &["SurvivalMode.arz"],
                &["Items.arc", "text_en.arc"],
            ),
            listed(
                "LootAscension",
                &["readme.txt", "LootAscension.arz"],
                &["Quests.arc"],
            ),
            listed("Empty", &[], &["Items.arc"]),
        ]);
        assert_eq!(
            layers,
            vec![
                LayerFiles {
                    database: PathBuf::from("mods/LootAscension/database/LootAscension.arz"),
                    text: PathBuf::from("mods/LootAscension/resources/Text_EN.arc"),
                    items: PathBuf::from("mods/LootAscension/resources/Items.arc"),
                },
                LayerFiles {
                    database: PathBuf::from("mods/survivalmode/database/SurvivalMode.arz"),
                    text: PathBuf::from("mods/survivalmode/resources/text_en.arc"),
                    items: PathBuf::from("mods/survivalmode/resources/Items.arc"),
                },
            ]
        );
    }

    #[test]
    fn a_mod_with_several_databases_contributes_the_first_by_name() {
        let layers = mod_layers([ModListing {
            folder: "twin".into(),
            database_files: vec!["b.arz".into(), "A.ARZ".into()],
            resource_files: vec![],
        }]);
        assert_eq!(
            layers[0].database,
            PathBuf::from("mods/twin/database/A.ARZ")
        );
    }

    #[test]
    fn mods_fill_only_what_no_shipped_layer_defines() {
        use univault_engine::arc::fixture::ArcBuilder;
        use univault_engine::arz::ArzDialect;
        use univault_engine::arz::fixture::{ArzBuilder, Values};
        use univault_engine::codec::Codec;
        use univault_engine::tex::fixture::tex;

        const SHARED: &str = "records/items/gear/shared.dbr";
        const MOD_ONLY: &str = "records/items/gear/modonly.dbr";
        let database = |records: &[(&str, &str, &str)]| {
            let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
            for (id, name_tag, bitmap) in records {
                builder.record(
                    id,
                    "ArmorProtective_Head",
                    &[
                        ("itemNameTag", Values::Strings(&[name_tag])),
                        ("bitmap", Values::Strings(&[bitmap])),
                    ],
                );
            }
            ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap()
        };
        let archive = |entries: &[(&str, &[u8])]| {
            let mut builder = ArcBuilder::new(Codec::Lz4Block);
            for (name, bytes) in entries {
                builder.stored(name, bytes);
            }
            ArcFile::parse(builder.build(), Codec::Lz4Block).unwrap()
        };
        let shipped = LayerSet {
            databases: vec![database(&[(SHARED, "tagShared", "items/gear/shared.tex")])],
            text_archives: vec![archive(&[("tags.txt", b"tagShared=Shipped Name\n")])],
            item_archives: vec![archive(&[("gear/shared.tex", &tex(64, 64))])],
        };
        let mods = LayerSet {
            databases: vec![database(&[
                (SHARED, "tagModded", "items/gear/shared.tex"),
                (MOD_ONLY, "tagModOnly", "items/gear/modonly.tex"),
            ])],
            text_archives: vec![archive(&[(
                "tags.txt",
                b"tagShared=Mod Override\ntagModded=Modded\ntagModOnly=Mod Only\n",
            )])],
            item_archives: vec![archive(&[
                ("gear/shared.tex", &tex(32, 32)),
                ("gear/modonly.tex", &tex(32, 96)),
            ])],
        };
        let game = GameData::layered(shipped, mods).unwrap();
        let info = |id: &str| {
            game.item_info(&RecordId::parse(id.to_string()).unwrap())
                .unwrap()
                .unwrap()
        };
        let footprint = |info: &ItemInfo| {
            game.footprint(info.bitmap.as_ref().unwrap())
                .unwrap()
                .unwrap()
        };

        let shared = info(SHARED);
        assert_eq!(shared.name, "Shipped Name");
        assert_eq!(
            footprint(&shared),
            Footprint {
                width: 2,
                height: 2
            }
        );
        let mod_only = info(MOD_ONLY);
        assert_eq!(mod_only.name, "Mod Only");
        assert_eq!(
            footprint(&mod_only),
            Footprint {
                width: 1,
                height: 3
            }
        );
    }

    #[test]
    fn rarity_parses_case_insensitively_and_rejects_unknown() {
        assert_eq!(Rarity::parse("Legendary"), Some(Rarity::Legendary));
        assert_eq!(Rarity::parse("EPIC"), Some(Rarity::Epic));
        assert_eq!(Rarity::parse("Mythical"), None);
    }

    #[test]
    fn bitmap_path_strips_the_archive_prefix_only() {
        assert_eq!(
            BitmapPath("items/gear/x.tex".into()).archive_entry(),
            "gear/x.tex"
        );
        assert_eq!(
            BitmapPath("Items/gear/x.tex".into()).archive_entry(),
            "gear/x.tex"
        );
        assert_eq!(
            BitmapPath("gear/x.tex".into()).archive_entry(),
            "gear/x.tex"
        );
    }

    #[test]
    fn bitmap_lookup_order_starts_with_gear_and_ends_with_transmuters() {
        assert_eq!(BITMAP_VARIABLES.first(), Some(&"bitmap"));
        assert!(BITMAP_VARIABLES.contains(&"relicBitmap"));
        assert_eq!(BITMAP_VARIABLES.last(), Some(&"emptyBitmap"));
    }

    #[test]
    fn caret_codes_are_stripped() {
        assert_eq!(strip_caret_codes("^kAether Soul"), "Aether Soul");
        assert_eq!(strip_caret_codes("plain"), "plain");
        assert_eq!(strip_caret_codes("trailing^"), "trailing");
    }
}
