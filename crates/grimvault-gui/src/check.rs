//! `grimvault-gui --check <game dir> <save dir>`: the Loading phase
//! without a window. Runs the same [`load_world`] the app does and
//! prints what the Ready phase would hold — layers and archives, every
//! stash tab with its items named, the component / crafting-material
//! storage by tab, the store, and every character — exiting non-zero
//! on any error, a character that failed to open included.

use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::InventoryState;
use grimvault_core::item::Item;
use grimvault_core::reagents::ReagentKind;

use crate::documents::{CharacterEntry, Reagents};
use crate::facts::FactsCache;
use crate::loader::{LoadStep, WorldPaths, load_world};
use crate::panes::character::{EQUIPMENT_SLOTS, EXTRA_SACK, MAIN_SACK, WEAPON_SLOTS};
use crate::settings::ConfigDir;
use crate::setup::{GameDir, SaveDir};

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
        "game data: {} database layers, {} text archives, {} item archives, loaded in {:.1?}",
        world.report.databases,
        world.report.text_archives,
        world.report.item_archives,
        world.report.elapsed
    );

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
        "\ncharacters: {} found under {}",
        world.characters.len(),
        paths.save.characters_dir().display()
    );
    let mut problems = 0;
    for entry in &world.characters {
        match entry {
            CharacterEntry::Loaded(doc) => print_character(doc.file(), &mut facts, &world.game),
            CharacterEntry::Failed { path, error } => {
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

fn print_character(
    file: &grimvault_core::gdc::PlayerFile,
    facts: &mut FactsCache,
    game: &GameData,
) {
    let header = file.header();
    let class = game
        .tag_text(&header.class_tag)
        .map_or_else(|| header.class_tag.clone(), str::to_string);
    println!(
        "  {} — level {}, {}{}",
        header.name,
        header.level,
        if class.is_empty() { "no class" } else { &class },
        if header.hardcore { ", hardcore" } else { "" }
    );
    match file.inventory() {
        None => println!("    inventory: block 3 not typed"),
        Some(inventory) => {
            for (slot, sack) in inventory.sacks().iter().enumerate() {
                let (cols, rows) = if slot == 0 { MAIN_SACK } else { EXTRA_SACK };
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
