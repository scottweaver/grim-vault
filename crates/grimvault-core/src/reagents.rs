//! Which items belong in the game's component and crafting-material
//! storage (`reagents.gst`, [`crate::gst::ReagentStorage`]), and which
//! of its two in-game tabs an entry shows under.
//!
//! The rule is the record database's own (surveyed 2026-09-03 across
//! all three layers): exactly the records whose `craftingMaterial`
//! flag is set are storable — every `ItemRelic` (107 components under
//! `records/items/materia/`) and 18 `QuestItem`s (the 15 crafting
//! materials under `records/items/crafting/materials/`, Scrap and
//! Dynamite under `records/items/questitems/`, one more under
//! `materia/`). Components are the `ItemRelic`s; everything else with
//! the flag is a crafting material.

use crate::gamedata::{GameData, ItemClass};
use crate::item::Item;
use univault_engine::ids::RecordId;

/// The two in-game tabs of the storage, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReagentKind {
    Component,
    CraftingMaterial,
}

impl ReagentKind {
    /// Both kinds in display order.
    pub const ALL: [ReagentKind; 2] = [ReagentKind::Component, ReagentKind::CraftingMaterial];

    /// The tab's in-game name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            ReagentKind::Component => "Components",
            ReagentKind::CraftingMaterial => "Crafting materials",
        }
    }

    /// The kind of a record from its `Class` and `craftingMaterial`
    /// flag: `ItemRelic` is a component, any other flagged record a
    /// crafting material, an unflagged record is not storable at all.
    #[must_use]
    pub fn of(class: Option<&ItemClass>, crafting_material: bool) -> Option<Self> {
        if class.is_some_and(|class| class.as_str() == "ItemRelic") {
            Some(Self::Component)
        } else if crafting_material {
            Some(Self::CraftingMaterial)
        } else {
            None
        }
    }

    /// The tab an entry *already in storage* shows under when its
    /// record cannot be classified (unknown to the database, or a
    /// record the game let in under a rule this app does not know):
    /// the crafting-materials tab, the game's own catch-all, so the
    /// entry stays visible and vaultable.
    #[must_use]
    pub const fn in_storage(kind: Option<Self>) -> Self {
        match kind {
            Some(kind) => kind,
            None => Self::CraftingMaterial,
        }
    }
}

/// Supplies the storage kind of an item's base record, `None` when
/// the record is unknown or not storable.
pub trait ReagentKinds {
    fn reagent_kind(&self, item: &Item) -> Option<ReagentKind>;
}

impl ReagentKinds for GameData {
    fn reagent_kind(&self, item: &Item) -> Option<ReagentKind> {
        let base = RecordId::parse(item.base_name.clone())?;
        self.item_info(&base)?.ok()?.reagent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str) -> ItemClass {
        ItemClass::new(name.to_string())
    }

    #[test]
    fn relics_are_components_and_flagged_records_materials() {
        assert_eq!(
            ReagentKind::of(Some(&class("ItemRelic")), true),
            Some(ReagentKind::Component)
        );
        assert_eq!(
            ReagentKind::of(Some(&class("ItemRelic")), false),
            Some(ReagentKind::Component)
        );
        assert_eq!(
            ReagentKind::of(Some(&class("QuestItem")), true),
            Some(ReagentKind::CraftingMaterial)
        );
        assert_eq!(
            ReagentKind::of(None, true),
            Some(ReagentKind::CraftingMaterial)
        );
    }

    #[test]
    fn unflagged_records_are_not_storable() {
        assert_eq!(ReagentKind::of(Some(&class("QuestItem")), false), None);
        assert_eq!(
            ReagentKind::of(Some(&class("ArmorProtective_Head")), false),
            None
        );
        assert_eq!(ReagentKind::of(None, false), None);
    }

    #[test]
    fn unclassified_entries_in_storage_show_as_materials() {
        assert_eq!(ReagentKind::in_storage(None), ReagentKind::CraftingMaterial);
        assert_eq!(
            ReagentKind::in_storage(Some(ReagentKind::Component)),
            ReagentKind::Component
        );
    }

    #[test]
    fn all_lists_both_tabs_with_distinct_labels() {
        assert_eq!(ReagentKind::ALL.len(), 2);
        assert_ne!(
            ReagentKind::Component.label(),
            ReagentKind::CraftingMaterial.label()
        );
    }
}
