//! The facet rules against the real record database and the real
//! items: the base records the rules were established on, the symbol
//! bitmaps `gameiteminfo.dbr` names, and every item in the transfer
//! stashes and character files under `$GRIMVAULT_SAVE_DIR` (a **copy**
//! of a save directory, never the live one) classified and counted.
//! Both tests pass vacuously when `$GRIMVAULT_GAME_DIR` is unset or
//! does not name the install.

#[path = "../examples/support/mod.rs"]
#[allow(
    dead_code,
    reason = "shared with the examples, which use the rest of it"
)]
mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use grimvault_core::facets::{
    Ascension, AscensionCategory, AscensionTable, DoubleRare, Facets, MonsterInfrequent, Symbol,
};
use grimvault_core::gamedata::{Binding, GameData, Rarity};
use grimvault_core::gdc::{PlayerFile, Realm};
use grimvault_core::gst::GstFile;
use grimvault_core::item::Item;
use grimvault_core::search::{AscensionFilter, Constraint, Query, Verdict};
use univault_engine::ids::RecordId;

fn game_data() -> Option<GameData> {
    let game_dir = std::env::var_os("GRIMVAULT_GAME_DIR").map(PathBuf::from)?;
    if !game_dir.join("database/database.arz").is_file() {
        return None;
    }
    Some(support::load_game_data(&game_dir).expect("the install's archives parse"))
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
fn the_rules_hold_on_the_records_they_were_established_on() {
    let Some(game) = game_data() else {
        return;
    };
    let table = AscensionTable::read(&game);
    let AscensionTable::Present(pairs) = &table else {
        panic!("the install has Fangs of Asterkarn, so the ascension table is present");
    };
    for rarity in [
        Rarity::Common,
        Rarity::Rare,
        Rarity::Epic,
        Rarity::Legendary,
    ] {
        for category in AscensionCategory::ALL {
            assert!(
                pairs.contains(&(rarity, category)),
                "{rarity:?} {category:?}"
            );
        }
    }
    assert!(!pairs.contains(&(Rarity::Magical, AscensionCategory::Armor)));

    let facets = |base: &str, prefix: &str, suffix: &str| {
        Facets::of(&game, &table, &item(base, prefix, suffix))
    };
    let monster_infrequent = facets("records/items/gearhead/b107a_head.dbr", "", "");
    assert_eq!(
        monster_infrequent.monster_infrequent,
        MonsterInfrequent::Yes
    );
    assert_eq!(monster_infrequent.ascension, Ascension::Eligible);
    assert_eq!(monster_infrequent.symbol(), Some(Symbol::MonsterInfrequent));

    let faction = RecordId::parse("records/items/faction/head/f004c_head.dbr".into()).unwrap();
    let faction_info = game.item_info(&faction).unwrap().unwrap();
    assert_eq!(faction_info.rarity, Some(Rarity::Rare));
    assert_eq!(faction_info.binding, Binding::Soulbound);
    let faction = facets("records/items/faction/head/f004c_head.dbr", "", "");
    assert_eq!(faction.monster_infrequent, MonsterInfrequent::No);
    assert_eq!(faction.ascension, Ascension::Eligible);

    for not_equipment in [
        "records/items/enchants/b46a_enchant.dbr",
        "records/items/materia/compb_markofdreeg.dbr",
        "records/items/faction/booster/boost_ro_b01.dbr",
        "records/items/gearrelic/b001_relic.dbr",
    ] {
        let facets = facets(not_equipment, "", "");
        assert_eq!(
            facets.monster_infrequent,
            MonsterInfrequent::No,
            "{not_equipment}"
        );
        assert_eq!(facets.ascension, Ascension::Ineligible, "{not_equipment}");
    }

    let double = facets(
        "records/items/gearhead/b107a_head.dbr",
        "records/items/lootaffixes/prefix/b_ar002_ar_f.dbr",
        "records/items/lootaffixes/suffix/b_ar033_ar_f.dbr",
    );
    assert_eq!(double.double_rare, DoubleRare::Yes);
    assert_eq!(double.symbol(), Some(Symbol::DoubleRareMonsterInfrequent));
    let single = facets(
        "records/items/gearhead/b107a_head.dbr",
        "records/items/lootaffixes/prefix/b_ar002_ar_f.dbr",
        "records/items/lootaffixes/suffix/a014a_ch_speedattack_03_we.dbr",
    );
    assert_eq!(single.double_rare, DoubleRare::No);

    let unknown = facets("records/items/gearhead/not_a_record.dbr", "", "");
    assert_eq!(unknown.monster_infrequent, MonsterInfrequent::Unresolved);
    assert_eq!(unknown.ascension, Ascension::Unresolved);

    let symbols = game.symbol_bitmaps();
    assert_eq!(symbols.len(), Symbol::ALL.len());
    assert_eq!(
        symbols[&Symbol::MonsterInfrequent].archive_entry(),
        "character/item_monsterinfrequent.tex"
    );
}

#[test]
fn every_real_item_classifies_and_the_counts_are_reported() {
    let (Some(game), Some(save_dir)) = (
        game_data(),
        std::env::var_os("GRIMVAULT_SAVE_DIR").map(PathBuf::from),
    ) else {
        return;
    };
    let table = AscensionTable::read(&game);
    let mut items: Vec<(String, Item)> = Vec::new();
    for stash in ["transfer.gst", "LootAscension/transfer.gst"] {
        let path = save_dir.join(stash);
        if !path.is_file() {
            continue;
        }
        let file = GstFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        for placed in file
            .transfer_stash()
            .unwrap()
            .tabs
            .iter()
            .flat_map(|tab| &tab.items)
        {
            items.push((stash.to_string(), placed.item.clone()));
        }
    }
    for realm in Realm::ALL {
        let Ok(folders) = std::fs::read_dir(save_dir.join(realm.dir_name())) else {
            continue;
        };
        for folder in folders.flatten() {
            let path = folder.path().join("player.gdc");
            if !path.is_file() {
                continue;
            }
            let label = format!(
                "{}/{}",
                realm.dir_name(),
                folder.file_name().to_string_lossy()
            );
            let file = PlayerFile::parse(&std::fs::read(&path).unwrap()).unwrap();
            if let Some(inventory) = file.inventory() {
                for placed in inventory.sacks().iter().flat_map(|sack| &sack.items) {
                    items.push((label.clone(), placed.item.clone()));
                }
                for slot in inventory.equipped() {
                    items.push((label.clone(), slot.item.clone()));
                }
            }
            if let Some(stash) = file.stash() {
                for placed in stash.tabs.iter().flat_map(|tab| &tab.items) {
                    items.push((label.clone(), placed.item.clone()));
                }
            }
        }
    }
    assert!(!items.is_empty(), "no items under the save dir");

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unresolved = Vec::new();
    for (place, item) in &items {
        let facets = Facets::of(&game, &table, item);
        if !facets.is_resolved() {
            unresolved.push(format!("{place}: {}", item.base_name));
        }
        for label in facets.labels() {
            *counts.entry(label).or_default() += 1;
        }
        if let Some(symbol) = facets.symbol() {
            *counts.entry(symbol.variable()).or_default() += 1;
        }
    }
    println!("{} items classified", items.len());
    for (label, count) in &counts {
        println!("  {count:5} {label}");
    }
    assert!(
        unresolved.is_empty(),
        "every record on the user's items resolves once gdx3 and the mods are read: {unresolved:?}"
    );
    assert!(counts.get("Monster Infrequent").copied().unwrap_or(0) > 0);
    assert!(counts.get("Double Rare").copied().unwrap_or(0) > 0);

    let monster_infrequents = Query {
        monster_infrequent: Constraint::Required,
        ..Query::default()
    };
    let matched = items
        .iter()
        .filter(|(_, item)| monster_infrequents.verdict_of(&game, &table, item) == Verdict::Matches)
        .count();
    assert_eq!(matched, counts["Monster Infrequent"]);
    let ascended = Query {
        ascension: AscensionFilter::Ascended,
        ..Query::default()
    };
    let ascended_items = items
        .iter()
        .filter(|(_, item)| ascended.verdict_of(&game, &table, item) == Verdict::Matches)
        .count();
    assert_eq!(ascended_items, counts.get("Ascended").copied().unwrap_or(0));
}
