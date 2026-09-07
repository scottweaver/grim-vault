//! Type buckets: the computed view that groups store entries by what
//! they are. A bucket is derived from the item's base record — its
//! `Class`, and for the crafting materials the `craftingMaterial` flag
//! the game files them under (they are `QuestItem`s by class,
//! [`crate::reagents`]) — every time it is displayed and is never
//! persisted (ARCHITECTURE.md "Source of truth"), so nothing can be
//! misfiled and copying an entry's bytes cannot change where it shows
//! up.
//!
//! The `Class` strings are the game's, observed in the shipped record
//! database; every observed value maps explicitly and anything else
//! lands in [`Bucket::Misc`] rather than being guessed at.

use serde::{Deserialize, Serialize};

use crate::gamedata::ItemClass;
use crate::reagents::ReagentKind;

/// The top level of the view, in display order: what is worn (relics
/// are equipped, so they are accessories), what goes into worn items
/// (components and augments), what the Inventor takes (materials,
/// blueprints, transmuters), what is used up (potions, oils, writs,
/// merits), and the rest (user grouping 2026-09-07).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    Weapons,
    Armor,
    Accessories,
    Upgrades,
    Crafting,
    Consumables,
    Other,
}

impl Group {
    /// Every group in display order.
    pub const ALL: [Group; 7] = [
        Group::Weapons,
        Group::Armor,
        Group::Accessories,
        Group::Upgrades,
        Group::Crafting,
        Group::Consumables,
        Group::Other,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Group::Weapons => "Weapons",
            Group::Armor => "Armor",
            Group::Accessories => "Accessories",
            Group::Upgrades => "Item Upgrades",
            Group::Crafting => "Crafting",
            Group::Consumables => "Consumables",
            Group::Other => "Other",
        }
    }
}

/// One bucket of the view, in display order within its [`Group`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Bucket {
    OneHanded,
    TwoHanded,
    RangedOneHanded,
    RangedTwoHanded,
    Offhand,
    Shield,
    Head,
    Chest,
    Shoulders,
    Hands,
    Legs,
    Feet,
    Waist,
    Amulet,
    Ring,
    Medal,
    Relic,
    Component,
    Augment,
    Material,
    Blueprint,
    Transmuter,
    Consumable,
    Writ,
    Quest,
    Note,
    Misc,
}

impl Bucket {
    /// Every bucket in display order (grouped, groups in [`Group::ALL`]
    /// order).
    pub const ALL: [Bucket; 27] = [
        Bucket::OneHanded,
        Bucket::TwoHanded,
        Bucket::RangedOneHanded,
        Bucket::RangedTwoHanded,
        Bucket::Offhand,
        Bucket::Shield,
        Bucket::Head,
        Bucket::Chest,
        Bucket::Shoulders,
        Bucket::Hands,
        Bucket::Legs,
        Bucket::Feet,
        Bucket::Waist,
        Bucket::Amulet,
        Bucket::Ring,
        Bucket::Medal,
        Bucket::Relic,
        Bucket::Component,
        Bucket::Augment,
        Bucket::Material,
        Bucket::Blueprint,
        Bucket::Transmuter,
        Bucket::Consumable,
        Bucket::Writ,
        Bucket::Quest,
        Bucket::Note,
        Bucket::Misc,
    ];

    /// The bucket for a record: a crafting material by the game's own
    /// flag first (its class is `QuestItem`, which is not what it is),
    /// else by `Class`; [`Bucket::Misc`] for no class or one this app
    /// has not mapped.
    #[must_use]
    pub fn of(class: Option<&ItemClass>, reagent: Option<ReagentKind>) -> Bucket {
        match reagent {
            Some(ReagentKind::CraftingMaterial) => Bucket::Material,
            Some(ReagentKind::Component) | None => class.map_or(Bucket::Misc, Self::of_class),
        }
    }

    fn of_class(class: &ItemClass) -> Bucket {
        let class = class.as_str();
        if class.starts_with("OneShot_") {
            return Bucket::Consumable;
        }
        match class {
            "WeaponMelee_Axe"
            | "WeaponMelee_Dagger"
            | "WeaponMelee_Mace"
            | "WeaponMelee_Scepter"
            | "WeaponMelee_Sword" => Bucket::OneHanded,
            "WeaponMelee_Axe2h"
            | "WeaponMelee_Mace2h"
            | "WeaponMelee_Spear2h"
            | "WeaponMelee_Sword2h" => Bucket::TwoHanded,
            "WeaponHunting_Ranged1h" => Bucket::RangedOneHanded,
            "WeaponHunting_Ranged2h" => Bucket::RangedTwoHanded,
            "WeaponArmor_Offhand" => Bucket::Offhand,
            "WeaponArmor_Shield" => Bucket::Shield,
            "ArmorProtective_Head" => Bucket::Head,
            "ArmorProtective_Chest" => Bucket::Chest,
            "ArmorProtective_Shoulders" => Bucket::Shoulders,
            "ArmorProtective_Hands" => Bucket::Hands,
            "ArmorProtective_Legs" => Bucket::Legs,
            "ArmorProtective_Feet" => Bucket::Feet,
            "ArmorProtective_Waist" => Bucket::Waist,
            "ArmorJewelry_Amulet" => Bucket::Amulet,
            "ArmorJewelry_Ring" => Bucket::Ring,
            "ArmorJewelry_Medal" => Bucket::Medal,
            "ItemRelic" => Bucket::Component,
            "ItemArtifact" => Bucket::Relic,
            "ItemEnchantment" => Bucket::Augment,
            "ItemArtifactFormula" | "ItemSetFormula" | "ItemRandomSetFormula" => Bucket::Blueprint,
            "ItemTransmuter" | "ItemTransmuterSet" => Bucket::Transmuter,
            "ItemUsableSkill" | "ItemAttributeReset" | "ItemDevotionReset" => Bucket::Consumable,
            "ItemFactionBooster" | "ItemFactionWarrant" | "ItemDifficultyUnlock" => Bucket::Writ,
            "QuestItem" => Bucket::Quest,
            "ItemNote" => Bucket::Note,
            _ => Bucket::Misc,
        }
    }

    #[must_use]
    pub const fn group(self) -> Group {
        match self {
            Bucket::OneHanded
            | Bucket::TwoHanded
            | Bucket::RangedOneHanded
            | Bucket::RangedTwoHanded
            | Bucket::Offhand
            | Bucket::Shield => Group::Weapons,
            Bucket::Head
            | Bucket::Chest
            | Bucket::Shoulders
            | Bucket::Hands
            | Bucket::Legs
            | Bucket::Feet
            | Bucket::Waist => Group::Armor,
            Bucket::Amulet | Bucket::Ring | Bucket::Medal | Bucket::Relic => Group::Accessories,
            Bucket::Component | Bucket::Augment => Group::Upgrades,
            Bucket::Material | Bucket::Blueprint | Bucket::Transmuter => Group::Crafting,
            Bucket::Consumable | Bucket::Writ => Group::Consumables,
            Bucket::Quest | Bucket::Note | Bucket::Misc => Group::Other,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Bucket::OneHanded => "One-Handed",
            Bucket::TwoHanded => "Two-Handed",
            Bucket::RangedOneHanded => "Ranged (One-Handed)",
            Bucket::RangedTwoHanded => "Ranged (Two-Handed)",
            Bucket::Offhand => "Off-Hand",
            Bucket::Shield => "Shields",
            Bucket::Head => "Head",
            Bucket::Chest => "Chest",
            Bucket::Shoulders => "Shoulders",
            Bucket::Hands => "Hands",
            Bucket::Legs => "Legs",
            Bucket::Feet => "Feet",
            Bucket::Waist => "Belts",
            Bucket::Amulet => "Amulets",
            Bucket::Ring => "Rings",
            Bucket::Medal => "Medals",
            Bucket::Relic => "Relics",
            Bucket::Component => "Components",
            Bucket::Augment => "Augments",
            Bucket::Material => "Crafting Materials",
            Bucket::Blueprint => "Blueprints",
            Bucket::Transmuter => "Transmuters",
            Bucket::Consumable => "Potions & Oils",
            Bucket::Writ => "Writs & Merits",
            Bucket::Quest => "Quest Items",
            Bucket::Note => "Notes",
            Bucket::Misc => "Miscellaneous",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn class(name: &str) -> ItemClass {
        ItemClass::new(name.to_string())
    }

    const MAPPING: &[(&str, Bucket)] = &[
        ("WeaponMelee_Axe", Bucket::OneHanded),
        ("WeaponMelee_Dagger", Bucket::OneHanded),
        ("WeaponMelee_Mace", Bucket::OneHanded),
        ("WeaponMelee_Scepter", Bucket::OneHanded),
        ("WeaponMelee_Sword", Bucket::OneHanded),
        ("WeaponMelee_Axe2h", Bucket::TwoHanded),
        ("WeaponMelee_Mace2h", Bucket::TwoHanded),
        ("WeaponMelee_Spear2h", Bucket::TwoHanded),
        ("WeaponMelee_Sword2h", Bucket::TwoHanded),
        ("WeaponHunting_Ranged1h", Bucket::RangedOneHanded),
        ("WeaponHunting_Ranged2h", Bucket::RangedTwoHanded),
        ("WeaponArmor_Offhand", Bucket::Offhand),
        ("WeaponArmor_Shield", Bucket::Shield),
        ("ArmorProtective_Head", Bucket::Head),
        ("ArmorProtective_Chest", Bucket::Chest),
        ("ArmorProtective_Shoulders", Bucket::Shoulders),
        ("ArmorProtective_Hands", Bucket::Hands),
        ("ArmorProtective_Legs", Bucket::Legs),
        ("ArmorProtective_Feet", Bucket::Feet),
        ("ArmorProtective_Waist", Bucket::Waist),
        ("ArmorJewelry_Amulet", Bucket::Amulet),
        ("ArmorJewelry_Ring", Bucket::Ring),
        ("ArmorJewelry_Medal", Bucket::Medal),
        ("ItemRelic", Bucket::Component),
        ("ItemArtifact", Bucket::Relic),
        ("ItemEnchantment", Bucket::Augment),
        ("ItemArtifactFormula", Bucket::Blueprint),
        ("ItemSetFormula", Bucket::Blueprint),
        ("ItemRandomSetFormula", Bucket::Blueprint),
        ("ItemTransmuter", Bucket::Transmuter),
        ("ItemTransmuterSet", Bucket::Transmuter),
        ("OneShot_Potion", Bucket::Consumable),
        ("OneShot_Scroll", Bucket::Consumable),
        ("OneShot_", Bucket::Consumable),
        ("ItemUsableSkill", Bucket::Consumable),
        ("ItemAttributeReset", Bucket::Consumable),
        ("ItemDevotionReset", Bucket::Consumable),
        ("ItemFactionBooster", Bucket::Writ),
        ("ItemFactionWarrant", Bucket::Writ),
        ("ItemDifficultyUnlock", Bucket::Writ),
        ("QuestItem", Bucket::Quest),
        ("ItemNote", Bucket::Note),
    ];

    #[test]
    fn every_observed_class_maps_to_its_bucket() {
        for (name, expected) in MAPPING {
            assert_eq!(Bucket::of(Some(&class(name)), None), *expected, "{name}");
        }
    }

    #[test]
    fn the_crafting_material_flag_outranks_the_quest_item_class() {
        assert_eq!(
            Bucket::of(
                Some(&class("QuestItem")),
                Some(ReagentKind::CraftingMaterial)
            ),
            Bucket::Material
        );
        assert_eq!(
            Bucket::of(Some(&class("ItemRelic")), Some(ReagentKind::Component)),
            Bucket::Component
        );
        assert_eq!(
            Bucket::of(None, Some(ReagentKind::CraftingMaterial)),
            Bucket::Material
        );
        assert_eq!(Bucket::of(None, None), Bucket::Misc);
        assert_eq!(Bucket::Material.group(), Group::Crafting);
        assert_eq!(Bucket::Material.label(), "Crafting Materials");
    }

    #[test]
    fn groups_say_what_their_buckets_are_for() {
        assert_eq!(Bucket::Relic.group(), Group::Accessories);
        assert_eq!(Bucket::Component.group(), Group::Upgrades);
        assert_eq!(Bucket::Augment.group(), Group::Upgrades);
        assert_eq!(Bucket::Blueprint.group(), Group::Crafting);
        assert_eq!(Bucket::Consumable.group(), Group::Consumables);
        assert_eq!(Bucket::Writ.group(), Group::Consumables);
        assert_eq!(Bucket::Quest.group(), Group::Other);
        assert_eq!(Bucket::Misc.group(), Group::Other);
        assert_eq!(Group::Upgrades.label(), "Item Upgrades");
        assert_eq!(Bucket::Writ.label(), "Writs & Merits");
        assert_eq!(Bucket::Consumable.label(), "Potions & Oils");
        let older_view: Group = serde_json::from_str("\"other\"").unwrap();
        assert_eq!(older_view, Group::Other);
        assert_eq!(
            serde_json::to_string(&Group::Upgrades).unwrap(),
            "\"upgrades\""
        );
    }

    #[test]
    fn unmapped_classes_fall_back_to_misc() {
        assert_eq!(
            Bucket::of(Some(&class("ItemFromTheFuture")), None),
            Bucket::Misc
        );
        assert_eq!(Bucket::of(Some(&class("")), None), Bucket::Misc);
        assert_eq!(
            Bucket::of(Some(&class("weaponmelee_axe")), None),
            Bucket::Misc
        );
        assert_eq!(Bucket::of(Some(&class("OneShot")), None), Bucket::Misc);
    }

    #[test]
    fn all_lists_every_bucket_once_grouped_in_group_order() {
        let unique: HashSet<Bucket> = Bucket::ALL.iter().copied().collect();
        assert_eq!(unique.len(), Bucket::ALL.len());
        assert!(MAPPING.iter().all(|(_, bucket)| unique.contains(bucket)));
        assert!(unique.contains(&Bucket::Material));
        let group_order: Vec<usize> = Bucket::ALL
            .iter()
            .map(|bucket| {
                Group::ALL
                    .iter()
                    .position(|group| *group == bucket.group())
                    .unwrap()
            })
            .collect();
        assert!(group_order.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(
            Group::ALL
                .iter()
                .all(|group| { Bucket::ALL.iter().any(|bucket| bucket.group() == *group) })
        );
    }

    #[test]
    fn labels_are_distinct_and_non_empty() {
        let labels: HashSet<&str> = Bucket::ALL.iter().map(|bucket| bucket.label()).collect();
        assert_eq!(labels.len(), Bucket::ALL.len());
        assert!(labels.iter().all(|label| !label.is_empty()));
        let groups: HashSet<&str> = Group::ALL.iter().map(|group| group.label()).collect();
        assert_eq!(groups.len(), Group::ALL.len());
    }
}
