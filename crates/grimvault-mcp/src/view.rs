//! JSON views of items, containers, and characters: what every tool
//! that hands back an item says about it, said one way. Pure over
//! loaded data.

use grimvault_core::bucket::Bucket;
use grimvault_core::facets::AscensionTable;
use grimvault_core::gamedata::{GameData, ItemInfo, Rarity};
use grimvault_core::gdc::{Inventory, PlayerStash, WeaponSet};
use grimvault_core::item::Item;
use grimvault_core::search::Resolved;
use grimvault_core::stats::item::{Block, ItemDetails, SetInfo};
use grimvault_core::stats::{Requirement, StatLine};
use grimvault_core::transfer::SackIndex;
use serde_json::{Value, json};
use univault_engine::ids::RecordId;

/// The database's word on an item's base record, when it has one.
#[must_use]
pub fn base_info(game: &GameData, item: &Item) -> Option<ItemInfo> {
    let id = RecordId::parse(item.base_name.clone())?;
    game.item_info(&id)?.ok()
}

/// The localized name of a base record, or its file stem when the
/// database does not know it; `<empty>` for no record at all.
#[must_use]
pub fn record_name(game: &GameData, record: &str) -> String {
    let Some(id) = RecordId::parse(record.to_string()) else {
        return "<empty>".to_string();
    };
    game.item_info(&id)
        .and_then(Result::ok)
        .map_or_else(|| id.file_stem().to_string(), |info| info.name)
}

fn record_or_null(record: &str) -> Value {
    if record.is_empty() {
        Value::Null
    } else {
        json!(record)
    }
}

/// An item as every tool reports it: its display name, every record
/// it carries (absent ones omitted), the seed, the stack, the
/// database's classification, and the facets the game would mark on
/// its tile. `details: true` appends the tooltip.
#[must_use]
pub fn item_json(game: &GameData, table: &AscensionTable, item: &Item, details: bool) -> Value {
    let resolved = Resolved::of(game, table, item);
    let info = base_info(game, item);
    let bucket = info
        .as_ref()
        .map(|info| Bucket::of(info.class.as_ref(), info.reagent));
    let mut out = json!({
        "name": resolved.name(),
        "base_record": item.base_name,
        "seed": item.seed,
        "rarity": info.as_ref().and_then(|info| info.rarity).map(Rarity::label),
        "class": info.as_ref().and_then(|info| info.class.as_ref()).map(|class| class.as_str().to_string()),
        "bucket": bucket.map(Bucket::label),
        "group": bucket.map(|bucket| bucket.group().label()),
        "level_requirement": info.as_ref().and_then(|info| info.level_requirement),
        "facets": resolved.facets().labels(),
    });
    let optional = [
        ("prefix_record", item.prefix_name.as_str()),
        ("suffix_record", item.suffix_name.as_str()),
        ("modifier_record", item.modifier_name.as_str()),
        ("transmute_record", item.transmute_name.as_str()),
        ("component_record", item.relic_name.as_str()),
        ("completion_bonus_record", item.relic_bonus.as_str()),
        ("augment_record", item.augment_name.as_str()),
        ("ascendant_record", item.ascendant_record.as_str()),
    ];
    for (key, record) in optional {
        if !record.is_empty() {
            out[key] = json!(record);
        }
    }
    if item.stack_count > 1 {
        out["stack"] = json!(item.stack_count);
    }
    if item.relic_completion_level > 0 {
        out["component_completion_level"] = json!(item.relic_completion_level);
    }
    if !item.relic_name.is_empty() {
        out["component_name"] = json!(record_name(game, &item.relic_name));
    }
    if !item.augment_name.is_empty() {
        out["augment_name"] = json!(record_name(game, &item.augment_name));
    }
    if details {
        out["details"] = details_json(resolved.details());
    }
    out
}

/// The tooltip: its blocks of stat lines by source, the requirements,
/// and the set the item belongs to.
#[must_use]
pub fn details_json(details: &ItemDetails) -> Value {
    json!({
        "blocks": details.blocks.iter().map(block_json).collect::<Vec<_>>(),
        "requirements": details
            .requirements
            .iter()
            .map(|(requirement, value)| json!({"requirement": requirement_name(*requirement), "value": value}))
            .collect::<Vec<_>>(),
        "set": details.set.as_ref().map(set_json),
        "unrendered": details.unrendered.iter().map(ToString::to_string).collect::<Vec<_>>(),
    })
}

fn block_json(block: &Block) -> Value {
    json!({
        "source": block.source.label(),
        "record": record_or_null(block.record.as_str()),
        "title": block.title,
        "flavor": block.flavor,
        "lines": line_texts(&block.lines),
    })
}

/// The lines' texts, as the game shows them.
#[must_use]
pub fn line_texts(lines: &[StatLine]) -> Vec<String> {
    lines.iter().map(|line| line.text.clone()).collect()
}

fn set_json(set: &SetInfo) -> Value {
    json!({
        "record": set.record.as_str(),
        "name": set.name,
        "members": set.members,
        "bonuses": set
            .tiers
            .iter()
            .map(|tier| json!({"pieces": tier.pieces, "lines": line_texts(&tier.lines)}))
            .collect::<Vec<_>>(),
    })
}

/// The requirement as a lowercase word.
#[must_use]
pub fn requirement_name(requirement: Requirement) -> &'static str {
    match requirement {
        Requirement::Level => "level",
        Requirement::Physique => "physique",
        Requirement::Cunning => "cunning",
        Requirement::Spirit => "spirit",
    }
}

/// A character's worn gear: every slot, both weapon sets, the set in
/// hand marked; an empty slot is `null`.
#[must_use]
pub fn equipment_json(game: &GameData, table: &AscensionTable, inventory: &Inventory) -> Value {
    let Some(contents) = inventory.contents() else {
        return json!({"note": "the character has never entered the game; no inventory yet"});
    };
    let in_hand = contents.active_weapon_set();
    let slots: Vec<Value> = contents
        .slots()
        .map(|(slot, worn)| {
            json!({
                "slot": slot.to_string(),
                "weapon_set": slot.weapon_set().map(set_number),
                "in_hand": slot.weapon_set().is_none_or(|set| set == in_hand),
                "item": (!worn.item.is_empty()).then(|| item_json(game, table, &worn.item, false)),
            })
        })
        .collect();
    json!({
        "active_weapon_set": set_number(in_hand),
        "slots": slots,
    })
}

fn set_number(set: WeaponSet) -> u8 {
    set.number()
}

/// A character's sacks with their grid sizes and placed items.
#[must_use]
pub fn sacks_json(game: &GameData, table: &AscensionTable, inventory: &Inventory) -> Vec<Value> {
    inventory
        .sacks()
        .iter()
        .enumerate()
        .map(|(index, sack)| {
            let dims = SackIndex::new(u32::try_from(index).unwrap_or(u32::MAX)).dimensions();
            json!({
                "sack": index,
                "width": dims.width,
                "height": dims.height,
                "items": sack
                    .items
                    .iter()
                    .map(|placed| {
                        let mut item = item_json(game, table, &placed.item, false);
                        item["x"] = json!(placed.x);
                        item["y"] = json!(placed.y);
                        item
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// Stash tabs — a character's own or the transfer stash's — with
/// their sizes and placed items.
#[must_use]
pub fn tabs_json(
    game: &GameData,
    table: &AscensionTable,
    tabs: &[grimvault_core::block::StashTab],
    only: Option<usize>,
) -> Vec<Value> {
    tabs.iter()
        .enumerate()
        .filter(|(index, _)| only.is_none_or(|wanted| wanted == *index))
        .map(|(index, tab)| {
            json!({
                "tab": index,
                "label": tab.decoration.button_name,
                "width": tab.width,
                "height": tab.height,
                "items": tab
                    .items
                    .iter()
                    .map(|placed| {
                        let mut item = item_json(game, table, &placed.item, false);
                        item["x"] = json!(placed.x);
                        item["y"] = json!(placed.y);
                        item
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// How many items a character's own stash holds, tab by tab, without
/// listing them.
#[must_use]
pub fn stash_summary(stash: &PlayerStash) -> Vec<Value> {
    stash
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| json!({"tab": index, "items": tab.items.len()}))
        .collect()
}
