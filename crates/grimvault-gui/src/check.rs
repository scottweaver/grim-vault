//! `grimvault-gui --check <game dir> <save dir>`: the Loading phase
//! without a window. Runs the same [`load_world`] the app does and
//! prints what the Ready phase would hold — layers and archives, every
//! stash tab with its items named, the component / crafting-material
//! storage by tab, the store, and every character — exiting non-zero
//! on any error, a character that failed to open included.

use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use grimvault_core::blueprint::check_blueprint;
use grimvault_core::formulas::FormulaRead;
use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::InventoryState;
use grimvault_core::illusion::{IllusionCategory, audit};
use grimvault_core::item::Item;
use grimvault_core::reagents::ReagentKind;
use grimvault_core::transfer::SackIndex;
use univault_engine::ids::RecordId;

use crate::crafting::{Blueprints, IllusionCollection};
use crate::documents::{CharacterDoc, CharacterEntry, Optional, Reagents, Writable};
use crate::facts::FactsCache;
use crate::loader::{LoadStep, WorldPaths, load_world};
use crate::panes::character::{EQUIPMENT_SLOTS, WEAPON_SLOTS};
use crate::settings::ConfigDir;
use crate::setup::{GameDir, SaveDir};
use grimvault_core::gdc::Realm;

pub fn run(game: &Path, save: &Path) -> ExitCode {
    match check(game, save) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(problems) => {
            eprintln!("{problems} problem(s) found");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn check(game: &Path, save: &Path) -> Result<usize, Box<dyn Error>> {
    let config = ConfigDir::resolve()?;
    println!("config dir: {}", config.path().display());
    let paths = WorldPaths {
        game: GameDir::parse(game)?,
        save: SaveDir::parse(save)?,
        store: config.store_file(),
    };
    let mut progress = |step: LoadStep| println!("… {step}");
    let world = load_world(&paths, &mut progress)?;
    let mut facts = FactsCache::default();
    println!(
        "game data: {} database layers, {} text archives, {} item archives, {} mod layers, \
         loaded in {:.1?}",
        world.report.databases,
        world.report.text_archives,
        world.report.item_archives,
        world.report.mods,
        world.report.elapsed
    );

    let campaigns: Vec<String> = world.campaigns.iter().map(ToString::to_string).collect();
    println!(
        "campaigns: {} — showing the {} (its transfer.gst was written last)",
        campaigns.join(", "),
        world.campaign
    );
    for warning in &world.warnings {
        println!("  WARNING {warning}");
    }

    let stash = world.stash.stash();
    println!(
        "\ntransfer stash: {} ({} bytes, lossless; block 18 {}; {} tabs)",
        world.stash.path().display(),
        world.stash.baseline_len(),
        stash.version,
        stash.tabs.len()
    );
    for (slot, tab) in stash.tabs.iter().enumerate() {
        println!(
            "  tab {slot} {:?} ({}x{}): {} items",
            tab.decoration.button_name,
            tab.width,
            tab.height,
            tab.items.len()
        );
        for (index, placed) in tab.items.iter().enumerate() {
            println!(
                "    [{index}] ({:>2.0},{:>2.0}) {}",
                placed.x,
                placed.y,
                describe(&mut facts, &world.game, &placed.item)
            );
        }
    }

    print_reagents(&world.reagents, &mut facts, &world.game);
    let mut problems = print_blueprints(&world.blueprints, &mut facts, &world.game);
    problems += print_illusions(&world.illusions, &mut facts, &world.game);

    let store = world.store.store();
    let status = if world.store.path().is_file() {
        String::new()
    } else {
        " (absent; created by the first save)".to_string()
    };
    println!(
        "\nstore: {}{status}: {} items",
        world.store.path().display(),
        store.len()
    );
    for stored in store.items() {
        let bucket = facts.base(&world.game, stored.item()).bucket;
        println!(
            "  {} {} [{}]",
            stored.id(),
            describe(&mut facts, &world.game, stored.item()),
            bucket.label()
        );
    }

    println!(
        "\ncharacters: {} found under {} and {}",
        world.characters.len(),
        paths.save.characters_dir(Realm::Main).display(),
        paths.save.characters_dir(Realm::Custom).display()
    );
    for entry in &world.characters {
        match entry {
            CharacterEntry::Loaded(doc) => print_character(doc, &mut facts, &world.game),
            CharacterEntry::Failed { path, error, .. } => {
                println!("  FAILED {}: {error}", path.display());
                problems += 1;
            }
        }
    }
    Ok(problems)
}

fn print_reagents(reagents: &Reagents, facts: &mut FactsCache, game: &GameData) {
    match reagents {
        Reagents::Open(doc) => {
            let storage = doc.storage();
            println!(
                "\ncomponent storage: {} ({} bytes, lossless; block 20 {}; {} entries, {} items)",
                doc.path().display(),
                doc.baseline_len(),
                storage.version,
                storage.entries.len(),
                storage.total_count()
            );
            for kind in ReagentKind::ALL {
                let mut rows: Vec<(String, usize, u32)> = storage
                    .entries
                    .iter()
                    .enumerate()
                    .filter_map(|(index, entry)| {
                        let item = Item {
                            base_name: entry.record.clone(),
                            ..Item::default()
                        };
                        let base = facts.base(game, &item);
                        (ReagentKind::in_storage(base.reagent) == kind)
                            .then(|| (base.name.clone(), index, entry.count))
                    })
                    .collect();
                rows.sort();
                println!("  {} ({})", kind.label(), rows.len());
                for (name, index, count) in rows {
                    println!("    [{index:>2}] {name} x{count}");
                }
            }
        }
        Reagents::Absent { path } => {
            println!(
                "\ncomponent storage: {} is absent (the game writes it once the storage is used)",
                path.display()
            );
        }
        Reagents::Failed { path, error, .. } => {
            println!(
                "\ncomponent storage: {} cannot be edited: {error}",
                path.display()
            );
        }
    }
}

/// The blueprint list with every entry named and checked against the
/// database; the number of entries the database refuses.
fn print_blueprints(blueprints: &Blueprints, facts: &mut FactsCache, game: &GameData) -> usize {
    let doc = match blueprints {
        Optional::Open(doc) => doc,
        Optional::Absent { path } => {
            println!(
                "\nblueprints: {} is absent (the game writes it once a blueprint is learned)",
                path.display()
            );
            return 0;
        }
        Optional::Failed { path, error, .. } => {
            println!("\nblueprints: {} cannot be edited: {error}", path.display());
            return 0;
        }
    };
    let formulas = doc.formulas();
    let unread = formulas
        .entries
        .iter()
        .filter(|entry| entry.read == FormulaRead::Unread)
        .count();
    println!(
        "\nblueprints: {} ({} bytes, lossless; {}; {} entries, {unread} new)",
        doc.path().display(),
        doc.baseline_len(),
        formulas.version,
        formulas.entries.len()
    );
    let mut problems = 0;
    let mut rows: Vec<(String, usize, &'static str, Option<String>)> = formulas
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let name = facts.base(game, &record_item(&entry.record)).name.clone();
            let badge = match entry.read {
                FormulaRead::Read => "",
                FormulaRead::Unread => " NEW",
            };
            let problem = RecordId::parse(entry.record.clone()).map_or(
                Some("empty record".to_string()),
                |id| {
                    check_blueprint(game, &id)
                        .err()
                        .map(|error| error.to_string())
                },
            );
            (name, index, badge, problem)
        })
        .collect();
    rows.sort();
    for (name, index, badge, problem) in rows {
        match problem {
            None => println!("  [{index:>3}] {name}{badge}"),
            Some(problem) => {
                problems += 1;
                println!("  [{index:>3}] {name}{badge} — PROBLEM {problem}");
            }
        }
    }
    problems
}

/// The illusion collection by category with every entry named, then
/// the database's disagreements; how many there were.
fn print_illusions(
    illusions: &IllusionCollection,
    facts: &mut FactsCache,
    game: &GameData,
) -> usize {
    let doc = match illusions {
        Optional::Open(doc) => doc,
        Optional::Absent { path } => {
            println!(
                "\nillusions: {} is absent (the game writes it once an illusion is unlocked)",
                path.display()
            );
            return 0;
        }
        Optional::Failed { path, error, .. } => {
            println!("\nillusions: {} cannot be edited: {error}", path.display());
            return 0;
        }
    };
    let collection = doc.illusions();
    println!(
        "\nillusions: {} ({} bytes, lossless; block 19 {}; {} unlocked in {} slots)",
        doc.path().display(),
        doc.baseline_len(),
        collection.version,
        collection.total_count(),
        collection.slots.len()
    );
    for slot in &collection.slots {
        let category = IllusionCategory::from_slot_id(slot.slot).map_or_else(
            || format!("slot {} (unknown)", slot.slot),
            |category| category.to_string(),
        );
        println!("  {category}: {} records", slot.records.len());
        let mut names: Vec<String> = slot
            .records
            .iter()
            .map(|record| facts.base(game, &record_item(record)).name.clone())
            .collect();
        names.sort();
        for name in names {
            println!("    {name}");
        }
    }
    let problems = audit(collection, game);
    for (record, error) in &problems {
        println!("  PROBLEM {record}: {error}");
    }
    problems.len()
}

fn record_item(record: &str) -> Item {
    Item {
        base_name: record.to_string(),
        ..Item::default()
    }
}

fn print_character(doc: &CharacterDoc, facts: &mut FactsCache, game: &GameData) {
    let file = doc.file();
    let header = file.header();
    let class = game
        .tag_text(&header.class_tag)
        .map_or_else(|| header.class_tag.clone(), str::to_string);
    let access = match doc.writable() {
        Writable::Yes => "editable".to_string(),
        Writable::OpaqueBlock(block) => format!("read-only: block {block} not typed"),
    };
    println!(
        "  {} [{}] — level {}, {}{}; {} iron bits; {} bytes, lossless; {access}",
        header.name,
        doc.realm(),
        header.level,
        if class.is_empty() { "no class" } else { &class },
        if header.hardcore { ", hardcore" } else { "" },
        file.character_info().map_or(0, |info| info.money),
        doc.baseline_len()
    );
    match file.inventory() {
        None => println!("    inventory: block 3 not typed"),
        Some(inventory) => {
            for (slot, sack) in inventory.sacks().iter().enumerate() {
                let dims = SackIndex::new(u32::try_from(slot).unwrap_or(u32::MAX)).dimensions();
                let cols = i32::try_from(dims.width).unwrap_or(0);
                let rows = i32::try_from(dims.height).unwrap_or(0);
                let (width, height) = sack.items.iter().fold((0, 0), |(width, height), placed| {
                    let footprint = facts.base(game, &placed.item).footprint;
                    let (w, h) = footprint.map_or((1, 1), |f| (f.width, f.height));
                    let x = i32::try_from(placed.x).unwrap_or(0);
                    let y = i32::try_from(placed.y).unwrap_or(0);
                    (width.max(x + w), height.max(y + h))
                });
                println!(
                    "    sack {slot}: {} items, extent {width}x{height} (rendered {}x{})",
                    sack.items.len(),
                    cols.max(width),
                    rows.max(height)
                );
            }
            if let InventoryState::Entered(contents) = &inventory.state {
                for (label, slot) in EQUIPMENT_SLOTS.iter().zip(&contents.equipment) {
                    if !slot.item.is_empty() {
                        let class = facts.base(game, &slot.item).class.clone();
                        println!(
                            "    equipped {label}: {} — {}",
                            class.map_or_else(|| "?".to_string(), |class| class.to_string()),
                            describe(facts, game, &slot.item)
                        );
                    }
                }
                for (set, slots) in [(1, &contents.weapon_set_1), (2, &contents.weapon_set_2)] {
                    for (label, slot) in WEAPON_SLOTS.iter().zip(slots) {
                        if !slot.item.is_empty() {
                            let class = facts.base(game, &slot.item).class.clone();
                            println!(
                                "    weapon set {set} {label}: {} — {}",
                                class.map_or_else(|| "?".to_string(), |class| class.to_string()),
                                describe(facts, game, &slot.item)
                            );
                        }
                    }
                }
            }
        }
    }
    match file.stash() {
        None => println!("    personal stash: block 4 not typed"),
        Some(stash) => {
            for (slot, tab) in stash.tabs.iter().enumerate() {
                println!(
                    "    stash tab {slot} ({}x{}): {} items",
                    tab.width,
                    tab.height,
                    tab.items.len()
                );
            }
        }
    }
}

/// `Prefix Base Suffix xN [Rarity WxH]`, `?` for what the database
/// cannot resolve.
fn describe(facts: &mut FactsCache, game: &GameData, item: &Item) -> String {
    let view = facts.facts(game, item);
    let rarity = view
        .base
        .rarity
        .map_or_else(|| "?".to_string(), |rarity| format!("{rarity:?}"));
    let footprint = view
        .base
        .footprint
        .map_or_else(|| "?".to_string(), |f| format!("{}x{}", f.width, f.height));
    let stack = if item.stack_count > 1 {
        format!(" x{}", item.stack_count)
    } else {
        String::new()
    };
    format!("{}{stack} [{rarity} {footprint}]", view.display_name())
}
