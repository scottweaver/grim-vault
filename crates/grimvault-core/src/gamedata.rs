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
//! versions dropped), `artifactBitmap` for relics,
//! `artifactFormulaBitmapName` for blueprints, `emptyBitmap` for
//! transmuters, `noteBitmap` for notes, `fullBitmap` for illusion
//! sets (see [`BITMAP_VARIABLES`]). `soulbound` marks gear the
//! game binds to its owner (faction-vendor items), and
//! `records/game/gameiteminfo.dbr` names the inventory-tile symbols
//! ([`GameData::symbol_bitmaps`]) that live in `UI.arc` — an archive
//! the shell reads by entry rather than whole, so it is not part of
//! [`GameData`].

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use univault_engine::arc::{ArcError, ArcFile};
use univault_engine::arz::{ArzError, ArzFile, DbRecord};
use univault_engine::ids::{RecordId, normalize};
use univault_engine::tex::{self, TexError};
use univault_engine::text::TextDb;

use crate::facets::Symbol;
use crate::reagents::ReagentKind;
use crate::stats::{RecordStats, Scale, SkillLevel, StatCache};

/// Item quality as the game's `itemClassification` spells it. Orders
/// by tier, common to legendary; quest items sit after the ladder, as
/// the game gives them no place on it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum Rarity {
    Common,
    Magical,
    Rare,
    Epic,
    Legendary,
    Quest,
}

impl Rarity {
    /// Every rarity, in tier order.
    pub const ALL: [Self; 6] = [
        Self::Common,
        Self::Magical,
        Self::Rare,
        Self::Epic,
        Self::Legendary,
        Self::Quest,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Common => "Common",
            Self::Magical => "Magical",
            Self::Rare => "Rare",
            Self::Epic => "Epic",
            Self::Legendary => "Legendary",
            Self::Quest => "Quest",
        }
    }

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

/// Whether the game binds the record's items to the character that
/// acquires them (`soulbound`): faction-vendor gear and its augments
/// are, drops are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    Soulbound,
    Free,
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
        archive_entry_of(&self.0, "items/")
    }

    /// The archive the path's first segment names — `items/…` is in
    /// `Items.arc`, `ui/…` in `UI.arc`, `level art/…` in
    /// `Level Art.arc` — and the entry name inside it. `None` for a
    /// bare entry name with no segment to name one.
    #[must_use]
    pub fn archive(&self) -> Option<(ArchiveName, &str)> {
        let (archive, entry) = self.0.split_once('/')?;
        (!archive.is_empty() && !entry.is_empty())
            .then(|| (ArchiveName(archive.to_ascii_lowercase()), entry))
    }

    /// Whether no `Items.arc` can hold the bitmap: its path names
    /// another archive. The four Lokarr set pieces hide their icons in
    /// `gdx1`'s `Level Art.arc`, the potion formulas theirs in
    /// `UI.arc` (`docs/format-references.md`).
    #[must_use]
    pub fn is_foreign(&self) -> bool {
        self.archive()
            .is_some_and(|(archive, _)| !archive.is_items())
    }
}

/// A resource archive as a bitmap path names it: the first path
/// segment, lower-cased (`items`, `ui`, `level art`). The file on
/// disk is `<Name>.arc` in a layer's `resources/` folder, spelled in
/// whatever case the game or mod chose.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArchiveName(String);

impl ArchiveName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The archive every item bitmap is expected in, and the one
    /// [`GameData`] reads whole.
    #[must_use]
    pub fn is_items(&self) -> bool {
        self.0 == "items"
    }

    /// Whether `file_name` is this archive's file: `<name>.arc`,
    /// case-insensitively.
    #[must_use]
    pub fn names_file(&self, file_name: &str) -> bool {
        Path::new(file_name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("arc"))
            && Path::new(file_name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(&self.0))
    }
}

impl fmt::Display for ArchiveName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Path of a user-interface bitmap inside the `UI.arc` archives, as a
/// record spells it (`ui/character/item_doublerare.tex`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UiBitmapPath(String);

impl UiBitmapPath {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The archive entry name: the `ui/` prefix names the archive,
    /// which the entries inside it omit.
    #[must_use]
    pub fn archive_entry(&self) -> &str {
        archive_entry_of(&self.0, "ui/")
    }
}

/// Strips the archive-naming prefix a record's bitmap path carries,
/// case-insensitively; a path without it is already an entry name.
fn archive_entry_of<'a>(path: &'a str, archive: &str) -> &'a str {
    match path.get(..archive.len()) {
        Some(head) if path.len() > archive.len() && head.eq_ignore_ascii_case(archive) => {
            &path[archive.len()..]
        }
        Some(_) | None => path,
    }
}

/// The record naming the inventory-tile symbols, loot beams, and
/// rarity colours.
const GAME_ITEM_INFO: &str = "records/game/gameiteminfo.dbr";

/// The variables an item record may name its icon by, in lookup
/// order: gear and crafting materials use `bitmap`, components
/// (`ItemRelic`) `relicBitmap`, relics (`ItemArtifact`)
/// `artifactBitmap`, blueprints (`ItemArtifactFormula`)
/// `artifactFormulaBitmapName`, transmuters (`ItemTransmuter`)
/// `emptyBitmap`, notes (`ItemNote`: quest notes and lore objects)
/// `noteBitmap` — the template calls it the bitmap "to show in the
/// UI", and it is the 32 × 32 inventory icon, not the parchment
/// the note is read on — and illusion sets (`ItemTransmuterSet`,
/// which carry no `emptyBitmap`) `fullBitmap`. GD Stash and GD Item
/// Assistant read the same seven (`docs/format-references.md`).
pub const BITMAP_VARIABLES: [&str; 7] = [
    "bitmap",
    "relicBitmap",
    "artifactBitmap",
    "artifactFormulaBitmapName",
    "emptyBitmap",
    "noteBitmap",
    "fullBitmap",
];

/// The table classes whose records are items a character can hold,
/// by the prefixes the game's templates give them; every record with
/// an item bitmap in the shipped databases is one of these (survey of
/// 2026-09-07, `docs/format-references.md`).
const ITEM_CLASS_PREFIXES: [&str; 5] = ["Armor", "Weapon", "Item", "OneShot", "QuestItem"];

/// Whether a record's table class is an item table.
#[must_use]
pub fn is_item_class(record_type: &str) -> bool {
    ITEM_CLASS_PREFIXES
        .iter()
        .any(|prefix| record_type.starts_with(prefix))
}

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
/// `craftingMaterial` flag ([`ReagentKind::of`]). `max_stack_size` is
/// the record's own `maxStackSize` when set above zero — the
/// template's override of the engine's per-class stacking default.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInfo {
    pub name: String,
    pub class: Option<ItemClass>,
    pub rarity: Option<Rarity>,
    pub level_requirement: Option<u32>,
    pub bitmap: Option<BitmapPath>,
    pub reagent: Option<ReagentKind>,
    pub binding: Binding,
    pub max_stack_size: Option<u32>,
}

/// What the database says about one affix record: its localized name
/// (`lootRandomizerName` — the ascendant affixes have none) and its
/// `itemClassification`.
#[derive(Debug, Clone, PartialEq)]
pub struct AffixInfo {
    pub name: Option<String>,
    pub rarity: Option<Rarity>,
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
/// `resources` is the folder the archives live in, for the few
/// bitmaps that name an archive other than `Items.arc`
/// ([`GameData::foreign_bitmaps`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerFiles {
    pub database: PathBuf,
    pub text: PathBuf,
    pub items: PathBuf,
    pub ui: PathBuf,
    pub resources: PathBuf,
}

/// The shipped layers in overlay order: the base game, then each
/// expansion (`gdx1`, `gdx2`, and `gdx3` once installed).
#[must_use]
pub fn shipped_layers() -> Vec<LayerFiles> {
    let base = LayerFiles {
        database: PathBuf::from("database/database.arz"),
        text: PathBuf::from("resources/Text_EN.arc"),
        items: PathBuf::from("resources/Items.arc"),
        ui: PathBuf::from("resources/UI.arc"),
        resources: PathBuf::from("resources"),
    };
    let expansions = ["gdx1", "gdx2", "gdx3"].into_iter().map(|root| LayerFiles {
        database: Path::new(root)
            .join("database")
            .join(format!("{}.arz", root.to_uppercase())),
        text: Path::new(root).join("resources/Text_EN.arc"),
        items: Path::new(root).join("resources/Items.arc"),
        ui: Path::new(root).join("resources/UI.arc"),
        resources: Path::new(root).join("resources"),
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
        ui: resource("UI.arc"),
        resources: root.join("resources"),
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
    /// The bytes of the bitmaps outside `Items.arc`, keyed by
    /// normalized path, handed in by a shell that read them by entry
    /// ([`Self::with_bitmaps`]).
    loose_bitmaps: HashMap<String, Vec<u8>>,
    stats: StatCache,
}

impl GameData {
    #[must_use]
    pub fn from_parts(databases: Vec<ArzFile>, text: TextDb, item_archives: Vec<ArcFile>) -> Self {
        Self {
            databases,
            text,
            item_archives,
            loose_bitmaps: HashMap::new(),
            stats: StatCache::default(),
        }
    }

    /// Adds the bytes of bitmaps no item archive holds — the ones
    /// [`Self::foreign_bitmaps`] names, read from the archives their
    /// paths name — so [`Self::bitmap`] and [`Self::footprint`] answer
    /// for them too. A path given twice keeps the last bytes.
    #[must_use]
    pub fn with_bitmaps(
        mut self,
        bitmaps: impl IntoIterator<Item = (BitmapPath, Vec<u8>)>,
    ) -> Self {
        self.loose_bitmaps.extend(
            bitmaps
                .into_iter()
                .map(|(path, bytes)| (normalize(path.as_str()), bytes)),
        );
        self
    }

    /// Every distinct bitmap of an item record that names an archive
    /// other than `Items.arc` ([`BitmapPath::is_foreign`]), in path
    /// order. Only the item tables are inflated
    /// ([`is_item_class`]): the other 80,000 records carry no item
    /// bitmap.
    #[must_use]
    pub fn foreign_bitmaps(&self) -> Vec<BitmapPath> {
        let mut found: Vec<BitmapPath> = self
            .record_types()
            .filter(|(_, record_type)| is_item_class(record_type))
            .filter_map(|(id, _)| self.item_info(id)?.ok()?.bitmap)
            .filter(BitmapPath::is_foreign)
            .collect();
        found.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        found.dedup();
        found
    }

    /// The stat lines of a record at `level` under `scale`, rendered
    /// once and shared ([`crate::stats`]). `None` when no layer has the
    /// record.
    #[must_use]
    pub fn record_stats(
        &self,
        id: &RecordId,
        level: SkillLevel,
        scale: Scale,
    ) -> Option<Arc<RecordStats>> {
        self.stats.record_stats(self, id, level, scale)
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

    /// The positions, topmost first, of every layer that defines the
    /// record — in the order the layers were composed (mods, then the
    /// shipped layers base-first), so a shell that knows what it
    /// composed can name them. Empty when no layer has it.
    pub fn defining_layers<'a>(&'a self, id: &'a RecordId) -> impl Iterator<Item = usize> + 'a {
        self.databases
            .iter()
            .enumerate()
            .rev()
            .filter(move |(_, db)| db.record(id).is_some())
            .map(|(index, _)| index)
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

    /// Every record id with its table class, from the topmost layer
    /// that defines each id, without inflating any record. The table's
    /// class is the record's `Class` variable (checked on the user's
    /// blueprint and illusion entries, 2026-09-06).
    pub fn record_types(&self) -> impl Iterator<Item = (&RecordId, &str)> {
        let mut seen = std::collections::HashSet::new();
        self.databases
            .iter()
            .rev()
            .flat_map(ArzFile::record_types)
            .filter(move |(id, _)| seen.insert(normalize(id.as_str())))
    }

    /// The records of one table class, see [`Self::record_types`].
    pub fn record_ids_of_type<'a>(
        &'a self,
        record_type: &'a str,
    ) -> impl Iterator<Item = &'a RecordId> + 'a {
        self.record_types()
            .filter(move |(_, found)| *found == record_type)
            .map(|(id, _)| id)
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
        let binding = if record.boolean("soulbound").unwrap_or(false) {
            Binding::Soulbound
        } else {
            Binding::Free
        };
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
            binding,
            max_stack_size: record
                .integer("maxStackSize")
                .and_then(|size| u32::try_from(size).ok())
                .filter(|size| *size > 0),
        }))
    }

    /// `None` when no layer has the affix record.
    #[must_use]
    pub fn affix_info(&self, id: &RecordId) -> Option<Result<AffixInfo, GameDataError>> {
        Some(
            self.record(id)?
                .map_err(GameDataError::from)
                .map(|record| AffixInfo {
                    name: self.localized_name(&record, &["lootRandomizerName"]),
                    rarity: record.string("itemClassification").and_then(Rarity::parse),
                }),
        )
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
        let affix_name = |id: &RecordId| self.affix_info(id)?.ok()?.name;
        let parts = [
            prefix.and_then(affix_name),
            Some(base_name),
            suffix.and_then(affix_name),
        ];
        Some(parts.into_iter().flatten().collect::<Vec<_>>().join(" "))
    }

    /// The inventory-tile symbols `records/game/gameiteminfo.dbr`
    /// names, as `UI.arc` entry paths; a symbol the record leaves
    /// unnamed — or every symbol, when no layer has the record — is
    /// absent.
    #[must_use]
    pub fn symbol_bitmaps(&self) -> HashMap<Symbol, UiBitmapPath> {
        let Some(Ok(record)) =
            RecordId::parse(GAME_ITEM_INFO.to_string()).and_then(|id| self.record(&id))
        else {
            return HashMap::new();
        };
        Symbol::ALL
            .into_iter()
            .filter_map(|symbol| {
                let path = record.string(symbol.variable())?;
                Some((symbol, UiBitmapPath(path.to_string())))
            })
            .collect()
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
            .or_else(|| {
                self.loose_bitmaps
                    .get(&normalize(bitmap.as_str()))
                    .map(|bytes| Ok(bytes.clone()))
            })
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

/// A one-layer database for the rule modules' tests: records by path,
/// table class, and string variables; no text, no bitmaps, so a name
/// resolves to its tag.
#[cfg(test)]
pub(crate) mod fixture {
    use univault_engine::arz::fixture::{ArzBuilder, Values};
    use univault_engine::arz::{ArzDialect, ArzFile};
    use univault_engine::text::TextDb;

    use super::GameData;

    /// A record's path, its table class, and its string variables.
    pub(crate) type Record<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

    pub(crate) fn game_with(records: &[Record<'_>]) -> GameData {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        for (id, class, strings) in records {
            let mut variables: Vec<(&str, Values<'_>)> =
                vec![("Class", Values::Strings(std::slice::from_ref(class)))];
            variables.extend(
                strings
                    .iter()
                    .map(|(name, value)| (*name, Values::Strings(std::slice::from_ref(value)))),
            );
            builder.record(id, class, &variables);
        }
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        GameData::from_parts(vec![database], TextDb::new(), Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_ids_of_type_reads_the_table_and_the_class_agrees() {
        let game = fixture::game_with(&[
            ("records/a.dbr", "ItemArtifactFormula", &[]),
            ("records/b.dbr", "ArmorProtective_Head", &[]),
            ("records/c.dbr", "ItemArtifactFormula", &[]),
        ]);
        let found: Vec<&str> = game
            .record_ids_of_type("ItemArtifactFormula")
            .map(RecordId::as_str)
            .collect();
        assert_eq!(found, ["records/a.dbr", "records/c.dbr"]);
        let info = game
            .item_info(&RecordId::parse("records/c.dbr".into()).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(info.class.unwrap().as_str(), "ItemArtifactFormula");
        assert_eq!(info.name, "c");
    }

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
        assert_eq!(layers[3].ui, Path::new("gdx3/resources/UI.arc"));
        assert_eq!(layers[1].resources, Path::new("gdx1/resources"));
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
                &["Items.arc", "text_en.arc", "ui.arc"],
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
                    ui: PathBuf::from("mods/LootAscension/resources/UI.arc"),
                    resources: PathBuf::from("mods/LootAscension/resources"),
                },
                LayerFiles {
                    database: PathBuf::from("mods/survivalmode/database/SurvivalMode.arz"),
                    text: PathBuf::from("mods/survivalmode/resources/text_en.arc"),
                    items: PathBuf::from("mods/survivalmode/resources/Items.arc"),
                    ui: PathBuf::from("mods/survivalmode/resources/ui.arc"),
                    resources: PathBuf::from("mods/survivalmode/resources"),
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
    fn bitmap_paths_name_their_archive_by_first_segment() {
        let archive = |path: &str| {
            BitmapPath(path.into())
                .archive()
                .map(|(archive, entry)| (archive.as_str().to_string(), entry.to_string()))
        };
        assert_eq!(
            archive("items/gearhead/x.tex"),
            Some(("items".into(), "gearhead/x.tex".into()))
        );
        assert_eq!(
            archive("Level Art/buildings/signs/sign_h01a_dif.tex"),
            Some((
                "level art".into(),
                "buildings/signs/sign_h01a_dif.tex".into()
            ))
        );
        assert_eq!(archive("bare.tex"), None);
        assert_eq!(archive("/x.tex"), None);
        assert!(!BitmapPath("items/gearhead/x.tex".into()).is_foreign());
        assert!(!BitmapPath("bare.tex".into()).is_foreign());
        assert!(BitmapPath("ui/cauldron/x.tex".into()).is_foreign());
        assert!(BitmapPath("level art/x.tex".into()).is_foreign());
    }

    #[test]
    fn archive_names_match_their_file_case_insensitively() {
        let (level_art, _) = BitmapPath("level art/x.tex".into()).archive().unwrap();
        assert!(level_art.names_file("Level Art.arc"));
        assert!(level_art.names_file("level art.ARC"));
        assert!(!level_art.names_file("Level Art.txt"));
        assert!(!level_art.names_file("Items.arc"));
        assert!(!level_art.is_items());
        let (items, _) = BitmapPath("Items/x.tex".into()).archive().unwrap();
        assert!(items.is_items());
    }

    #[test]
    fn item_classes_are_the_armor_weapon_item_oneshot_and_quest_tables() {
        for class in [
            "ArmorProtective_Head",
            "WeaponMelee_Axe2h",
            "ItemArtifactFormula",
            "OneShot_SkillUnlock",
            "QuestItem",
        ] {
            assert!(is_item_class(class), "{class}");
        }
        for class in [
            "Sign",
            "Monster",
            "Skill_AttackRadius",
            "LootItemTable_DynWeighted",
        ] {
            assert!(!is_item_class(class), "{class}");
        }
    }

    #[test]
    fn foreign_bitmaps_are_the_item_records_icons_outside_items_arc() {
        use univault_engine::arc::fixture::ArcBuilder;
        use univault_engine::arz::ArzDialect;
        use univault_engine::arz::fixture::{ArzBuilder, Values};
        use univault_engine::codec::Codec;
        use univault_engine::tex::fixture::tex;

        const LOKARR: &str = "level art/buildings/signs/sign_h01a_dif.tex";
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(
            "records/items/gearhead/plain.dbr",
            "ArmorProtective_Head",
            &[("bitmap", Values::Strings(&["items/gearhead/plain.tex"]))],
        );
        builder.record(
            "records/storyelements/signs/signh.dbr",
            "ArmorProtective_Head",
            &[("bitmap", Values::Strings(&[LOKARR]))],
        );
        builder.record(
            "records/storyelements/signs/signh2.dbr",
            "ArmorProtective_Chest",
            &[(
                "bitmap",
                Values::Strings(&["Level Art/buildings/signs/SIGN_H01A_DIF.tex"]),
            )],
        );
        builder.record(
            "records/levelart/signs/post.dbr",
            "Sign",
            &[(
                "bitmap",
                Values::Strings(&["level art/buildings/signs/post.tex"]),
            )],
        );
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        let mut items = ArcBuilder::new(Codec::Lz4Block);
        items.stored("gearhead/plain.tex", &tex(32, 32));
        let items = ArcFile::parse(items.build(), Codec::Lz4Block).unwrap();
        let game = GameData::from_parts(vec![database], TextDb::new(), vec![items]);

        let foreign: Vec<String> = game
            .foreign_bitmaps()
            .iter()
            .map(|path| path.as_str().to_string())
            .collect();
        assert_eq!(
            foreign,
            ["Level Art/buildings/signs/SIGN_H01A_DIF.tex", LOKARR]
        );

        let lokarr = BitmapPath(LOKARR.into());
        assert!(game.bitmap(&lokarr).is_none());
        assert!(game.footprint(&lokarr).is_none());
        let game = game.with_bitmaps([(lokarr.clone(), tex(64, 96))]);
        assert_eq!(game.bitmap(&lokarr).unwrap().unwrap(), tex(64, 96));
        assert_eq!(
            game.footprint(&lokarr).unwrap().unwrap(),
            Footprint {
                width: 2,
                height: 3
            }
        );
        let spelled_otherwise = BitmapPath("Level Art/buildings/signs/SIGN_H01A_DIF.tex".into());
        assert_eq!(
            game.bitmap(&spelled_otherwise).unwrap().unwrap(),
            tex(64, 96)
        );
        let plain = BitmapPath("items/gearhead/plain.tex".into());
        assert_eq!(game.bitmap(&plain).unwrap().unwrap(), tex(32, 32));
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
        assert_eq!(
            UiBitmapPath("ui/character/item_doublerare.tex".into()).archive_entry(),
            "character/item_doublerare.tex"
        );
        assert_eq!(UiBitmapPath("ui/".into()).archive_entry(), "ui/");
        assert_eq!(UiBitmapPath("u".into()).archive_entry(), "u");
    }

    #[test]
    fn symbol_bitmaps_come_from_gameiteminfo_and_are_absent_without_it() {
        use univault_engine::arz::ArzDialect;
        use univault_engine::arz::fixture::{ArzBuilder, Values};

        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(
            GAME_ITEM_INFO,
            "",
            &[
                (
                    "monsterInfrequentSymbol",
                    Values::Strings(&["ui/character/item_monsterinfrequent.tex"]),
                ),
                (
                    "doubleRareSymbol",
                    Values::Strings(&["ui/character/item_doublerare.tex"]),
                ),
            ],
        );
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        let game = GameData::from_parts(vec![database], TextDb::new(), Vec::new());
        let symbols = game.symbol_bitmaps();
        assert_eq!(symbols.len(), 2);
        assert_eq!(
            symbols[&Symbol::DoubleRare].archive_entry(),
            "character/item_doublerare.tex"
        );
        assert!(!symbols.contains_key(&Symbol::Awakened));

        let empty = GameData::from_parts(Vec::new(), TextDb::new(), Vec::new());
        assert!(empty.symbol_bitmaps().is_empty());
    }

    #[test]
    fn bitmap_lookup_order_starts_with_gear_and_ends_with_illusion_sets() {
        assert_eq!(BITMAP_VARIABLES.first(), Some(&"bitmap"));
        assert!(BITMAP_VARIABLES.contains(&"relicBitmap"));
        let empty = BITMAP_VARIABLES
            .iter()
            .position(|variable| *variable == "emptyBitmap");
        let full = BITMAP_VARIABLES
            .iter()
            .position(|variable| *variable == "fullBitmap");
        assert!(empty < full, "a transmuter keeps its empty scroll");
        assert_eq!(BITMAP_VARIABLES.last(), Some(&"fullBitmap"));
    }

    #[test]
    fn notes_and_illusion_sets_have_icons_and_footprints() {
        use univault_engine::arc::fixture::ArcBuilder;
        use univault_engine::arz::ArzDialect;
        use univault_engine::arz::fixture::{ArzBuilder, Values};
        use univault_engine::codec::Codec;
        use univault_engine::tex::fixture::tex;

        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(
            "records/storyelements/questitems/cultistdirections.dbr",
            "ItemNote",
            &[
                (
                    "noteBitmap",
                    Values::Strings(&["items/misc/parchment01.tex"]),
                ),
                ("noteWidth", Values::Ints(&[400])),
            ],
        );
        builder.record(
            "records/items/transmutes/transmute_powderedwig.dbr",
            "ItemTransmuterSet",
            &[(
                "fullBitmap",
                Values::Strings(&["items/transmutes/transmute_powderedwig.tex"]),
            )],
        );
        builder.record(
            "records/items/transmutes/knight/transmute_silverknight_shoulders.dbr",
            "ItemTransmuter",
            &[
                (
                    "emptyBitmap",
                    Values::Strings(&["items/transmutes/transmute_empty02.tex"]),
                ),
                (
                    "fullBitmap",
                    Values::Strings(&[
                        "items/transmutes/knight/transmute_silverknight_shoulders.tex",
                    ]),
                ),
            ],
        );
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        let mut items = ArcBuilder::new(Codec::Lz4Block);
        items.stored("misc/parchment01.tex", &tex(32, 32));
        items.stored("transmutes/transmute_powderedwig.tex", &tex(64, 64));
        items.stored("transmutes/transmute_empty02.tex", &tex(64, 64));
        items.stored(
            "transmutes/knight/transmute_silverknight_shoulders.tex",
            &tex(64, 64),
        );
        let items = ArcFile::parse(items.build(), Codec::Lz4Block).unwrap();
        let game = GameData::from_parts(vec![database], TextDb::new(), vec![items]);
        let icon = |id: &str| {
            let info = game
                .item_info(&RecordId::parse(id.to_string()).unwrap())
                .unwrap()
                .unwrap();
            let bitmap = info.bitmap.expect("an icon");
            let footprint = game.footprint(&bitmap).unwrap().unwrap();
            (bitmap.as_str().to_string(), footprint)
        };

        assert_eq!(
            icon("records/storyelements/questitems/cultistdirections.dbr"),
            (
                "items/misc/parchment01.tex".to_string(),
                Footprint {
                    width: 1,
                    height: 1
                }
            )
        );
        assert_eq!(
            icon("records/items/transmutes/transmute_powderedwig.dbr"),
            (
                "items/transmutes/transmute_powderedwig.tex".to_string(),
                Footprint {
                    width: 2,
                    height: 2
                }
            )
        );
        assert_eq!(
            icon("records/items/transmutes/knight/transmute_silverknight_shoulders.dbr").0,
            "items/transmutes/transmute_empty02.tex"
        );
        assert!(game.foreign_bitmaps().is_empty());
    }

    #[test]
    fn caret_codes_are_stripped() {
        assert_eq!(strip_caret_codes("^kAether Soul"), "Aether Soul");
        assert_eq!(strip_caret_codes("plain"), "plain");
        assert_eq!(strip_caret_codes("trailing^"), "trailing");
    }
}
