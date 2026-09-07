//! Item facets: the qualities the game calls out with a symbol on an
//! item's inventory tile — monster infrequent, double rare, and (Fangs
//! of Asterkarn) ascended — plus whether an item is still eligible for
//! ascension. Derived from the item's own record paths and the record
//! database every time they are asked for, never stored. Every facet
//! has an explicit unresolved state: an item whose record the database
//! cannot supply never reads as "not a monster infrequent".
//!
//! The rules, each established on the real database
//! (`docs/format-references.md`, "Item facets"):
//!
//! - **Monster infrequent:** the base record's `itemClassification`
//!   is `Rare`, its `Class` is equipment (armor, jewellery, weapon,
//!   shield, off-hand), and it is not `soulbound`. Faction-vendor
//!   gear shares the classification but is soulbound; components,
//!   augments, and relics share it but are not equipment.
//! - **Double rare:** the prefix and suffix records are both `Rare`.
//! - **Ascension:** an item carrying an ascendant affix
//!   (`ascendant_record` / `ascendant_record_2h`) is ascended;
//!   otherwise it is eligible when
//!   `records/ui/itemascension/itemascension_table.dbr` names a recipe
//!   for its base rarity and that recipe has an affix table for its
//!   equipment category ([`AscensionTable`]).
//!
//! The symbol a tile carries ([`Facets::symbol`]) follows
//! `records/game/gameiteminfo.dbr`'s eleven `*Symbol` variables
//! ([`Symbol`]): the monster-infrequent, double-rare, or combined mark
//! when either facet holds; otherwise an item that is ascended *or
//! eligible for ascension* shows its displayed rarity's symbol
//! (ascended double rares their own variants). The game marks
//! eligible, unascended items — the user's stash tab of plain epics
//! carries the mark in-game (2026-09-06) — while rares keep their
//! monster-infrequent mark.

use std::collections::HashSet;

use univault_engine::ids::RecordId;

use crate::bucket::Bucket;
use crate::gamedata::{AffixInfo, Binding, GameData, ItemInfo, Rarity};
use crate::item::Item;

/// Whether the base record is a monster infrequent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MonsterInfrequent {
    Yes,
    No,
    /// The base record could not be read.
    Unresolved,
}

/// Whether both affixes are rare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DoubleRare {
    Yes,
    No,
    /// An affix record could not be read.
    Unresolved,
}

/// Where the item stands with Fangs of Asterkarn's item ascension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ascension {
    /// Carries an ascendant affix already.
    Ascended,
    /// The ascension table has a recipe for its rarity and category.
    Eligible,
    Ineligible,
    /// The base record or the ascension table could not be read.
    Unresolved,
}

/// The equipment categories the ascension recipes are tabled by —
/// also the set of classes that can be a monster infrequent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AscensionCategory {
    Accessory,
    Armor,
    Offhand,
    OneHandMelee,
    OneHandRanged,
    Shield,
    TwoHandMelee,
    TwoHandRanged,
}

impl AscensionCategory {
    /// Every category.
    pub const ALL: [Self; 8] = [
        Self::Accessory,
        Self::Armor,
        Self::Offhand,
        Self::OneHandMelee,
        Self::OneHandRanged,
        Self::Shield,
        Self::TwoHandMelee,
        Self::TwoHandRanged,
    ];

    /// The category of a type bucket; `None` for anything that is not
    /// equipment. Belts count as accessories, as the game's own
    /// grouping has it.
    #[must_use]
    pub const fn of_bucket(bucket: Bucket) -> Option<Self> {
        match bucket {
            Bucket::OneHanded => Some(Self::OneHandMelee),
            Bucket::TwoHanded => Some(Self::TwoHandMelee),
            Bucket::RangedOneHanded => Some(Self::OneHandRanged),
            Bucket::RangedTwoHanded => Some(Self::TwoHandRanged),
            Bucket::Offhand => Some(Self::Offhand),
            Bucket::Shield => Some(Self::Shield),
            Bucket::Head
            | Bucket::Chest
            | Bucket::Shoulders
            | Bucket::Hands
            | Bucket::Legs
            | Bucket::Feet => Some(Self::Armor),
            Bucket::Waist | Bucket::Amulet | Bucket::Ring | Bucket::Medal => Some(Self::Accessory),
            Bucket::Component
            | Bucket::Material
            | Bucket::Relic
            | Bucket::Augment
            | Bucket::Blueprint
            | Bucket::Transmuter
            | Bucket::Consumable
            | Bucket::Writ
            | Bucket::Quest
            | Bucket::Note
            | Bucket::Misc => None,
        }
    }

    /// The recipe variable listing the category's affix tables.
    #[must_use]
    pub const fn affix_table_variable(self) -> &'static str {
        match self {
            Self::Accessory => "accessoryTablesAffix",
            Self::Armor => "armorTablesAffix",
            Self::Offhand => "offhandTablesAffix",
            Self::OneHandMelee => "oneHandMeleeTablesAffix",
            Self::OneHandRanged => "oneHandRangedTablesAffix",
            Self::Shield => "shieldTablesAffix",
            Self::TwoHandMelee => "twoHandMeleeTablesAffix",
            Self::TwoHandRanged => "twoHandRangedTablesAffix",
        }
    }
}

/// The record naming one ascension recipe per base rarity.
pub const ASCENSION_TABLE: &str = "records/ui/itemascension/itemascension_table.dbr";

/// The rarities the table keys recipes by, with their variables.
const RECIPE_VARIABLES: [(Rarity, &str); 4] = [
    (Rarity::Common, "commonRecipe"),
    (Rarity::Rare, "rareRecipe"),
    (Rarity::Epic, "epicRecipe"),
    (Rarity::Legendary, "legendaryRecipe"),
];

/// Which (base rarity, category) pairs the install can ascend, read
/// once from [`ASCENSION_TABLE`] and its recipe records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AscensionTable {
    /// No layer has the table: an install without Fangs of Asterkarn.
    Absent,
    /// The table or one of its recipes failed to read.
    Unreadable,
    Present(HashSet<(Rarity, AscensionCategory)>),
}

impl AscensionTable {
    /// Reads the table from the layered database.
    #[must_use]
    pub fn read(game: &GameData) -> Self {
        let Some(table) =
            RecordId::parse(ASCENSION_TABLE.to_string()).and_then(|id| game.record(&id))
        else {
            return Self::Absent;
        };
        let Ok(table) = table else {
            return Self::Unreadable;
        };
        let mut eligible = HashSet::new();
        for (rarity, variable) in RECIPE_VARIABLES {
            let Some(recipe) = table
                .string(variable)
                .and_then(|path| RecordId::parse(path.to_string()))
                .and_then(|id| game.record(&id))
            else {
                continue;
            };
            let Ok(recipe) = recipe else {
                return Self::Unreadable;
            };
            for category in AscensionCategory::ALL {
                let tabled = recipe
                    .variable(category.affix_table_variable())
                    .is_some_and(|tables| !tables.is_empty());
                if tabled {
                    eligible.insert((rarity, category));
                }
            }
        }
        Self::Present(eligible)
    }

    fn eligibility(
        &self,
        rarity: Option<Rarity>,
        category: Option<AscensionCategory>,
    ) -> Ascension {
        match self {
            Self::Absent => Ascension::Ineligible,
            Self::Unreadable => Ascension::Unresolved,
            Self::Present(eligible) => match (rarity, category) {
                (Some(rarity), Some(category)) if eligible.contains(&(rarity, category)) => {
                    Ascension::Eligible
                }
                (Some(_) | None, Some(_) | None) => Ascension::Ineligible,
            },
        }
    }
}

/// What the base record contributes to the facets — the shape a shell
/// memoizes per record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseEvidence {
    /// The record is missing or unreadable.
    Unresolved,
    Known {
        rarity: Option<Rarity>,
        bucket: Bucket,
        binding: Binding,
    },
}

impl BaseEvidence {
    /// From a resolved record, or its absence.
    #[must_use]
    pub fn of(info: Option<&ItemInfo>) -> Self {
        info.map_or(Self::Unresolved, |info| Self::Known {
            rarity: info.rarity,
            bucket: Bucket::of(info.class.as_ref(), info.reagent),
            binding: info.binding,
        })
    }
}

/// What one affix slot contributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AffixEvidence {
    /// The slot is empty.
    Absent,
    /// The slot names a record the database cannot supply.
    Unresolved,
    Known {
        rarity: Option<Rarity>,
    },
}

impl AffixEvidence {
    /// From the slot's record path and, when the path is not empty,
    /// the resolved record or its absence.
    #[must_use]
    pub fn of(path: &str, info: Option<&AffixInfo>) -> Self {
        if path.is_empty() {
            Self::Absent
        } else {
            info.map_or(Self::Unresolved, |info| Self::Known {
                rarity: info.rarity,
            })
        }
    }

    fn is_rare(self) -> Option<bool> {
        match self {
            Self::Absent => Some(false),
            Self::Unresolved => None,
            Self::Known { rarity } => Some(rarity == Some(Rarity::Rare)),
        }
    }

    fn rarity(self) -> Option<Rarity> {
        match self {
            Self::Absent | Self::Unresolved => None,
            Self::Known { rarity } => rarity,
        }
    }
}

/// The eleven inventory-tile symbols `records/game/gameiteminfo.dbr`
/// names. `Awakened` marks the upgraded items of Fangs of Asterkarn's
/// awakening, a facet this app does not derive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Symbol {
    MonsterInfrequent,
    DoubleRare,
    DoubleRareMonsterInfrequent,
    CommonAscended,
    MagicalAscended,
    RareAscended,
    DoubleRareAscended,
    MonsterDoubleRareAscended,
    EpicAscended,
    LegendaryAscended,
    Awakened,
}

impl Symbol {
    /// Every symbol.
    pub const ALL: [Self; 11] = [
        Self::MonsterInfrequent,
        Self::DoubleRare,
        Self::DoubleRareMonsterInfrequent,
        Self::CommonAscended,
        Self::MagicalAscended,
        Self::RareAscended,
        Self::DoubleRareAscended,
        Self::MonsterDoubleRareAscended,
        Self::EpicAscended,
        Self::LegendaryAscended,
        Self::Awakened,
    ];

    /// The `gameiteminfo.dbr` variable naming the symbol's bitmap.
    #[must_use]
    pub const fn variable(self) -> &'static str {
        match self {
            Self::MonsterInfrequent => "monsterInfrequentSymbol",
            Self::DoubleRare => "doubleRareSymbol",
            Self::DoubleRareMonsterInfrequent => "doubleRareMonsterInfrequentSymbol",
            Self::CommonAscended => "commonAscendedSymbol",
            Self::MagicalAscended => "magicalAscendedSymbol",
            Self::RareAscended => "rareAscendedSymbol",
            Self::DoubleRareAscended => "doubleRareAscendedSymbol",
            Self::MonsterDoubleRareAscended => "monsterDoubleRareAscendedSymbol",
            Self::EpicAscended => "epicAscendedSymbol",
            Self::LegendaryAscended => "legendaryAscendedSymbol",
            Self::Awakened => "awakenedItemSymbol",
        }
    }

    /// What the symbol says, for a tooltip.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MonsterInfrequent => "Monster Infrequent",
            Self::DoubleRare => "Double Rare",
            Self::DoubleRareMonsterInfrequent => "Double Rare Monster Infrequent",
            Self::CommonAscended
            | Self::MagicalAscended
            | Self::RareAscended
            | Self::EpicAscended
            | Self::LegendaryAscended => "Ascended",
            Self::DoubleRareAscended => "Ascended Double Rare",
            Self::MonsterDoubleRareAscended => "Ascended Double Rare Monster Infrequent",
            Self::Awakened => "Awakened",
        }
    }
}

/// One item's facets, with the rarity it displays as (its base's,
/// raised by a rarer affix) for choosing the ascended symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facets {
    pub monster_infrequent: MonsterInfrequent,
    pub double_rare: DoubleRare,
    pub ascension: Ascension,
    displayed: Option<Rarity>,
}

impl Facets {
    /// Classifies from evidence a shell has already gathered.
    #[must_use]
    pub fn classify(
        item: &Item,
        base: BaseEvidence,
        prefix: AffixEvidence,
        suffix: AffixEvidence,
        table: &AscensionTable,
    ) -> Self {
        let monster_infrequent = match base {
            BaseEvidence::Unresolved => MonsterInfrequent::Unresolved,
            BaseEvidence::Known {
                rarity,
                bucket,
                binding,
            } => {
                let rare_equipment = rarity == Some(Rarity::Rare)
                    && AscensionCategory::of_bucket(bucket).is_some()
                    && binding == Binding::Free;
                if rare_equipment {
                    MonsterInfrequent::Yes
                } else {
                    MonsterInfrequent::No
                }
            }
        };
        let double_rare = match (prefix.is_rare(), suffix.is_rare()) {
            (Some(true), Some(true)) => DoubleRare::Yes,
            (Some(false), _) | (_, Some(false)) => DoubleRare::No,
            (None, None | Some(true)) | (Some(true), None) => DoubleRare::Unresolved,
        };
        let ascended = !item.ascendant_record.is_empty() || !item.ascendant_record_2h.is_empty();
        let ascension = match base {
            _ if ascended => Ascension::Ascended,
            BaseEvidence::Unresolved => Ascension::Unresolved,
            BaseEvidence::Known { rarity, bucket, .. } => {
                table.eligibility(rarity, AscensionCategory::of_bucket(bucket))
            }
        };
        let displayed = match base {
            BaseEvidence::Unresolved => None,
            BaseEvidence::Known { rarity, .. } => displayed_rarity(rarity, prefix, suffix),
        };
        Self {
            monster_infrequent,
            double_rare,
            ascension,
            displayed,
        }
    }

    /// Classifies an item straight from the database.
    #[must_use]
    pub fn of(game: &GameData, table: &AscensionTable, item: &Item) -> Self {
        let base = RecordId::parse(item.base_name.clone())
            .and_then(|id| game.item_info(&id))
            .and_then(Result::ok);
        let affix = |path: &str| {
            let info = RecordId::parse(path.to_string())
                .and_then(|id| game.affix_info(&id))
                .and_then(Result::ok);
            AffixEvidence::of(path, info.as_ref())
        };
        Self::classify(
            item,
            BaseEvidence::of(base.as_ref()),
            affix(&item.prefix_name),
            affix(&item.suffix_name),
            table,
        )
    }

    /// The symbol the game draws on the tile, from the facets known to
    /// hold; `None` for an item the game leaves unmarked. The
    /// monster-infrequent and double-rare marks outrank the ascension
    /// mark, which an item carries once ascended or while eligible.
    #[must_use]
    pub fn symbol(&self) -> Option<Symbol> {
        let monster = self.monster_infrequent == MonsterInfrequent::Yes;
        let double = self.double_rare == DoubleRare::Yes;
        let ascended = self.ascension == Ascension::Ascended;
        match (monster, double) {
            (true, true) if ascended => Some(Symbol::MonsterDoubleRareAscended),
            (true, true) => Some(Symbol::DoubleRareMonsterInfrequent),
            (false, true) if ascended => Some(Symbol::DoubleRareAscended),
            (false, true) => Some(Symbol::DoubleRare),
            (true, false) if ascended => Some(rarity_symbol(self.displayed)),
            (true, false) => Some(Symbol::MonsterInfrequent),
            (false, false) => match self.ascension {
                Ascension::Ascended | Ascension::Eligible => Some(rarity_symbol(self.displayed)),
                Ascension::Ineligible | Ascension::Unresolved => None,
            },
        }
    }

    /// The facets in words: each that holds, then a note when any
    /// could not be resolved.
    #[must_use]
    pub fn labels(&self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        if self.monster_infrequent == MonsterInfrequent::Yes {
            labels.push("Monster Infrequent");
        }
        if self.double_rare == DoubleRare::Yes {
            labels.push("Double Rare");
        }
        match self.ascension {
            Ascension::Ascended => labels.push("Ascended"),
            Ascension::Eligible => labels.push("Upgradeable (Ascendant)"),
            Ascension::Ineligible | Ascension::Unresolved => {}
        }
        if !self.is_resolved() {
            labels.push("some facets unresolved");
        }
        labels
    }

    /// The rarity the item shows as — its base's, raised by a rarer
    /// affix, a quest item always quest — `None` when the base record
    /// could not be read.
    #[must_use]
    pub fn displayed_rarity(&self) -> Option<Rarity> {
        self.displayed
    }

    /// Whether every facet could be decided.
    #[must_use]
    pub fn is_resolved(&self) -> bool {
        self.monster_infrequent != MonsterInfrequent::Unresolved
            && self.double_rare != DoubleRare::Unresolved
            && self.ascension != Ascension::Unresolved
    }
}

/// The ascension symbol for the rarity an item displays as; an
/// unresolved rarity takes the common one, the game's plainest mark.
fn rarity_symbol(displayed: Option<Rarity>) -> Symbol {
    match displayed {
        Some(Rarity::Common) | None => Symbol::CommonAscended,
        Some(Rarity::Magical) => Symbol::MagicalAscended,
        Some(Rarity::Rare | Rarity::Quest) => Symbol::RareAscended,
        Some(Rarity::Epic) => Symbol::EpicAscended,
        Some(Rarity::Legendary) => Symbol::LegendaryAscended,
    }
}

/// The rarity an item shows as: a quest item is always a quest item;
/// otherwise the highest tier among its base and affixes.
fn displayed_rarity(
    base: Option<Rarity>,
    prefix: AffixEvidence,
    suffix: AffixEvidence,
) -> Option<Rarity> {
    if base == Some(Rarity::Quest) {
        return base;
    }
    [base, prefix.rarity(), suffix.rarity()]
        .into_iter()
        .flatten()
        .max_by_key(|rarity| tier(*rarity))
}

/// Quality order for display, quest items outside it.
const fn tier(rarity: Rarity) -> u8 {
    match rarity {
        Rarity::Common => 0,
        Rarity::Magical => 1,
        Rarity::Rare => 2,
        Rarity::Epic => 3,
        Rarity::Legendary => 4,
        Rarity::Quest => 5,
    }
}

#[cfg(test)]
mod tests {
    use univault_engine::arz::ArzDialect;
    use univault_engine::arz::ArzFile;
    use univault_engine::arz::fixture::{ArzBuilder, Values};
    use univault_engine::text::TextDb;

    use super::*;

    const MI: &str = "records/items/gearhead/b107a_head.dbr";
    const FACTION: &str = "records/items/faction/head/f004c_head.dbr";
    const AUGMENT: &str = "records/items/enchants/b46a_enchant.dbr";
    const COMMON: &str = "records/items/gearhead/a01_head.dbr";
    const RELIC: &str = "records/items/gearrelic/b001_relic.dbr";
    const RARE_PREFIX: &str = "records/items/lootaffixes/prefix/b_ar002_ar_f.dbr";
    const RARE_SUFFIX: &str = "records/items/lootaffixes/suffix/b_ar033_ar_f.dbr";
    const MAGIC_SUFFIX: &str = "records/items/lootaffixes/suffix/a014a.dbr";
    const ASCENDANT: &str = "records/items/lootaffixes/ascended/ao303c.dbr";
    const RARE_RECIPE: &str = "records/items/crafting/blueprints/ascension/craft_ascended_rare.dbr";
    const COMMON_RECIPE: &str =
        "records/items/crafting/blueprints/ascension/craft_ascended_common.dbr";

    fn item_record(builder: &mut ArzBuilder, id: &str, class: &str, rarity: &str, soulbound: bool) {
        builder.record(
            id,
            class,
            &[
                ("Class", Values::Strings(&[class])),
                ("itemClassification", Values::Strings(&[rarity])),
                ("soulbound", Values::Bools(&[soulbound])),
            ],
        );
    }

    fn affix_record(builder: &mut ArzBuilder, id: &str, rarity: Option<&str>) {
        match rarity {
            Some(rarity) => builder.record(
                id,
                "LootRandomizer",
                &[("itemClassification", Values::Strings(&[rarity]))],
            ),
            None => builder.record(id, "LootRandomizer", &[]),
        }
    }

    fn database(with_ascension: bool) -> GameData {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        item_record(&mut builder, MI, "ArmorProtective_Head", "Rare", false);
        item_record(&mut builder, FACTION, "ArmorProtective_Head", "Rare", true);
        item_record(&mut builder, AUGMENT, "ItemEnchantment", "Rare", true);
        item_record(
            &mut builder,
            COMMON,
            "ArmorProtective_Head",
            "Common",
            false,
        );
        item_record(&mut builder, RELIC, "ItemArtifact", "Rare", false);
        affix_record(&mut builder, RARE_PREFIX, Some("Rare"));
        affix_record(&mut builder, RARE_SUFFIX, Some("Rare"));
        affix_record(&mut builder, MAGIC_SUFFIX, Some("Magical"));
        affix_record(&mut builder, ASCENDANT, None);
        if with_ascension {
            builder.record(
                ASCENSION_TABLE,
                "",
                &[
                    ("commonRecipe", Values::Strings(&[COMMON_RECIPE])),
                    ("rareRecipe", Values::Strings(&[RARE_RECIPE])),
                ],
            );
            builder.record(
                COMMON_RECIPE,
                "ItemAscensionFormula",
                &[("armorTablesAffix", Values::Strings(&["a.dbr", "b.dbr"]))],
            );
            builder.record(
                RARE_RECIPE,
                "ItemAscensionFormula",
                &[("armorTablesAffix", Values::Strings(&["a.dbr"]))],
            );
        }
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        GameData::from_parts(vec![database], TextDb::new(), Vec::new())
    }

    fn item(base: &str, prefix: &str, suffix: &str) -> Item {
        Item {
            base_name: base.into(),
            prefix_name: prefix.into(),
            suffix_name: suffix.into(),
            ..Item::default()
        }
    }

    #[test]
    fn a_rare_unbound_equipment_record_is_a_monster_infrequent() {
        let game = database(true);
        let table = AscensionTable::read(&game);
        let facets = |base: &str| Facets::of(&game, &table, &item(base, "", ""));
        assert_eq!(facets(MI).monster_infrequent, MonsterInfrequent::Yes);
        assert_eq!(facets(FACTION).monster_infrequent, MonsterInfrequent::No);
        assert_eq!(facets(AUGMENT).monster_infrequent, MonsterInfrequent::No);
        assert_eq!(facets(RELIC).monster_infrequent, MonsterInfrequent::No);
        assert_eq!(facets(COMMON).monster_infrequent, MonsterInfrequent::No);
        assert_eq!(
            facets("records/items/gearhead/missing.dbr").monster_infrequent,
            MonsterInfrequent::Unresolved
        );
        assert_eq!(facets("").monster_infrequent, MonsterInfrequent::Unresolved);
    }

    #[test]
    fn double_rare_needs_both_affixes_rare_and_stays_open_on_an_unknown_one() {
        let game = database(true);
        let table = AscensionTable::read(&game);
        let facets = |prefix: &str, suffix: &str| {
            Facets::of(&game, &table, &item(COMMON, prefix, suffix)).double_rare
        };
        assert_eq!(facets(RARE_PREFIX, RARE_SUFFIX), DoubleRare::Yes);
        assert_eq!(facets(RARE_PREFIX, MAGIC_SUFFIX), DoubleRare::No);
        assert_eq!(facets(RARE_PREFIX, ""), DoubleRare::No);
        assert_eq!(facets("", ""), DoubleRare::No);
        assert_eq!(facets(RARE_PREFIX, "records/x.dbr"), DoubleRare::Unresolved);
        assert_eq!(facets("records/x.dbr", MAGIC_SUFFIX), DoubleRare::No);
        assert_eq!(facets(RARE_PREFIX, ASCENDANT), DoubleRare::No);
    }

    #[test]
    fn a_rare_base_with_two_rare_affixes_is_both_facets_at_once() {
        let game = database(true);
        let table = AscensionTable::read(&game);
        let facets = Facets::of(&game, &table, &item(MI, RARE_PREFIX, RARE_SUFFIX));
        assert_eq!(facets.monster_infrequent, MonsterInfrequent::Yes);
        assert_eq!(facets.double_rare, DoubleRare::Yes);
        assert_eq!(facets.symbol(), Some(Symbol::DoubleRareMonsterInfrequent));
        assert_eq!(
            facets.labels(),
            vec![
                "Monster Infrequent",
                "Double Rare",
                "Upgradeable (Ascendant)"
            ]
        );
    }

    #[test]
    fn ascension_follows_the_table_and_an_ascendant_affix_wins() {
        let game = database(true);
        let table = AscensionTable::read(&game);
        let facets = |base: &str| Facets::of(&game, &table, &item(base, "", "")).ascension;
        assert_eq!(facets(MI), Ascension::Eligible);
        assert_eq!(facets(COMMON), Ascension::Eligible);
        assert_eq!(facets(FACTION), Ascension::Eligible);
        assert_eq!(facets(RELIC), Ascension::Ineligible);
        assert_eq!(facets(AUGMENT), Ascension::Ineligible);
        assert_eq!(facets("records/missing.dbr"), Ascension::Unresolved);
        let ascended = Item {
            ascendant_record: ASCENDANT.into(),
            ..item("records/missing.dbr", "", "")
        };
        assert_eq!(
            Facets::of(&game, &table, &ascended).ascension,
            Ascension::Ascended
        );
        let two_handed = Item {
            ascendant_record_2h: ASCENDANT.into(),
            ..item(MI, "", "")
        };
        assert_eq!(
            Facets::of(&game, &table, &two_handed).ascension,
            Ascension::Ascended
        );
    }

    #[test]
    fn a_recipe_without_a_shield_table_makes_shields_ineligible_for_that_rarity() {
        let game = database(true);
        let table = AscensionTable::read(&game);
        let AscensionTable::Present(pairs) = &table else {
            panic!("table present");
        };
        assert!(pairs.contains(&(Rarity::Rare, AscensionCategory::Armor)));
        assert!(!pairs.contains(&(Rarity::Rare, AscensionCategory::Shield)));
        assert!(!pairs.contains(&(Rarity::Epic, AscensionCategory::Armor)));
        assert!(pairs.contains(&(Rarity::Common, AscensionCategory::Armor)));
    }

    #[test]
    fn without_the_table_nothing_is_eligible_but_ascended_items_still_are() {
        let game = database(false);
        let table = AscensionTable::read(&game);
        assert_eq!(table, AscensionTable::Absent);
        assert_eq!(
            Facets::of(&game, &table, &item(MI, "", "")).ascension,
            Ascension::Ineligible
        );
        let ascended = Item {
            ascendant_record: ASCENDANT.into(),
            ..item(MI, "", "")
        };
        assert_eq!(
            Facets::of(&game, &table, &ascended).ascension,
            Ascension::Ascended
        );
    }

    #[test]
    fn the_symbol_follows_the_games_variable_set() {
        let game = database(true);
        let table = AscensionTable::read(&game);
        let symbol = |base: &str, prefix: &str, suffix: &str, ascendant: &str| {
            let candidate = Item {
                ascendant_record: ascendant.into(),
                ..item(base, prefix, suffix)
            };
            Facets::of(&game, &table, &candidate).symbol()
        };
        assert_eq!(symbol(COMMON, "", "", ""), Some(Symbol::CommonAscended));
        assert_eq!(
            symbol(COMMON, "", MAGIC_SUFFIX, ""),
            Some(Symbol::MagicalAscended)
        );
        assert_eq!(symbol(MI, "", "", ""), Some(Symbol::MonsterInfrequent));
        assert_eq!(
            symbol(COMMON, RARE_PREFIX, RARE_SUFFIX, ""),
            Some(Symbol::DoubleRare)
        );
        assert_eq!(
            symbol(FACTION, RARE_PREFIX, RARE_SUFFIX, ""),
            Some(Symbol::DoubleRare)
        );
        assert_eq!(
            symbol(COMMON, "", "", ASCENDANT),
            Some(Symbol::CommonAscended)
        );
        assert_eq!(
            symbol(COMMON, "", MAGIC_SUFFIX, ASCENDANT),
            Some(Symbol::MagicalAscended)
        );
        assert_eq!(
            symbol(COMMON, "", RARE_SUFFIX, ASCENDANT),
            Some(Symbol::RareAscended)
        );
        assert_eq!(symbol(MI, "", "", ASCENDANT), Some(Symbol::RareAscended));
        assert_eq!(
            symbol(COMMON, RARE_PREFIX, RARE_SUFFIX, ASCENDANT),
            Some(Symbol::DoubleRareAscended)
        );
        assert_eq!(
            symbol(MI, RARE_PREFIX, RARE_SUFFIX, ASCENDANT),
            Some(Symbol::MonsterDoubleRareAscended)
        );
        assert_eq!(
            symbol("records/missing.dbr", RARE_PREFIX, RARE_SUFFIX, ""),
            Some(Symbol::DoubleRare)
        );
        assert_eq!(symbol("records/missing.dbr", "", "", ""), None);
    }

    #[test]
    fn without_an_ascension_table_an_unmarked_item_stays_unmarked() {
        let game = database(false);
        let table = AscensionTable::read(&game);
        let facets = Facets::of(&game, &table, &item(COMMON, "", ""));
        assert_eq!(facets.ascension, Ascension::Ineligible);
        assert_eq!(facets.symbol(), None);
        assert_eq!(
            Facets::of(&game, &table, &item(MI, "", "")).symbol(),
            Some(Symbol::MonsterInfrequent)
        );
    }

    #[test]
    fn every_symbol_has_a_distinct_variable_and_a_label() {
        let variables: HashSet<&str> = Symbol::ALL.iter().map(|s| s.variable()).collect();
        assert_eq!(variables.len(), Symbol::ALL.len());
        assert!(Symbol::ALL.iter().all(|s| !s.label().is_empty()));
    }

    #[test]
    fn every_affixed_equipment_bucket_has_a_category_and_nothing_else_does() {
        for bucket in Bucket::ALL {
            let is_equipment = bucket != Bucket::Relic
                && matches!(
                    bucket.group(),
                    crate::bucket::Group::Weapons
                        | crate::bucket::Group::Armor
                        | crate::bucket::Group::Accessories
                );
            assert_eq!(
                AscensionCategory::of_bucket(bucket).is_some(),
                is_equipment,
                "{bucket:?}"
            );
        }
        assert_eq!(
            AscensionCategory::of_bucket(Bucket::Waist),
            Some(AscensionCategory::Accessory)
        );
    }

    #[test]
    fn unresolved_facets_are_named_in_the_labels() {
        let facets = Facets::classify(
            &item("records/missing.dbr", RARE_PREFIX, ""),
            BaseEvidence::Unresolved,
            AffixEvidence::Unresolved,
            AffixEvidence::Absent,
            &AscensionTable::Absent,
        );
        assert!(!facets.is_resolved());
        assert_eq!(facets.labels(), vec!["some facets unresolved"]);
        assert_eq!(facets.double_rare, DoubleRare::No);
    }
}
