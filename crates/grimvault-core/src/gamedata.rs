//! Grim Dawn game-data facade: the layered record database (base,
//! then `gdx1`, then `gdx2`, later layers overriding earlier records
//! by path), the localization tags, and the item bitmap archives,
//! resolved into the facts the app needs about an item base or affix
//! record — display name, rarity, class, level requirement, and grid
//! footprint. Read-only reference data per ARCHITECTURE.md; nothing
//! here writes.
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
