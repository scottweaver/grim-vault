//! `grimvault-gui --check <game dir> <save dir>`: the Loading phase
//! without a window. Runs the same [`load_world`] the app does and
//! prints what the Ready phase would hold — layers and archives, every
//! stash tab with its items named, the component / crafting-material
//! storage by tab, the store, every character, and — when run from
//! the saved settings — what the standing orders (auto-move tabs,
//! purge tabs, the component sync) would do on this load, as a dry
//! run — exiting non-zero on any error, a character that failed to
//! open included. Nothing is written.

use std::error::Error;
use std::fmt;
use std::path::Path;
use std::process::ExitCode;

use grimvault_core::block::StashTab;
use grimvault_core::blueprint::check_blueprint;
use grimvault_core::bulk::{self, Identities};
use grimvault_core::formulas::FormulaRead;
use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::InventoryState;
use grimvault_core::illusion::{IllusionCategory, audit};
use grimvault_core::item::Item;
use grimvault_core::reagents::ReagentKind;
use grimvault_core::settings::{AutoMoveTab, BlueprintSync, ReagentSync, Settings, StandingOrder};
use grimvault_core::transfer::{SackIndex, TabIndex};
use univault_engine::ids::RecordId;

use crate::automove::{self, AutoMoveTarget};
use crate::crafting::{Blueprints, IllusionCollection};
use crate::documents::{CharacterDoc, CharacterEntry, Optional, Reagents, StoreDoc, Writable};
use crate::facts::FactsCache;
use crate::loader::{LoadStep, LoadedWorld, WorldPaths, load_world};
use crate::panes::character::{EQUIPMENT_SLOTS, WEAPON_SLOTS};
use crate::settings::ConfigDir;
use crate::setup::{GameDir, SaveDir};
use grimvault_core::gdc::Realm;

pub fn run(game: &Path, save: &Path, settings: Option<&Settings>) -> ExitCode {
    match check(game, save, settings) {
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

fn check(game: &Path, save: &Path, settings: Option<&Settings>) -> Result<usize, Box<dyn Error>> {
    let remembered = settings.and_then(|settings| settings.campaign.as_ref());
    let config = ConfigDir::resolve()?;
    println!("config dir: {}", config.path().display());
    let paths = WorldPaths {
        game: GameDir::parse(game)?,
        save: SaveDir::parse(save)?,
        store: settings.map_or_else(|| config.store_file(), |saved| saved.store_file(&config)),
        ui_state: config.ui_state_file(),
    };
    let mut progress = |step: LoadStep| println!("… {step}");
    let world = load_world(&paths, remembered, &mut progress)?;
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

    println!(
        "tile symbols: {} of {} found in UI.arc",
        world.report.symbols,
        grimvault_core::facets::Symbol::ALL.len()
    );
    for (symbol, problem) in world.symbols.problems() {
        println!("  {}: {problem}", symbol.variable());
    }

    let campaigns: Vec<String> = world.campaigns.iter().map(ToString::to_string).collect();
    println!(
        "campaigns: {} — showing the {} ({})",
        campaigns.join(", "),
        world.campaign,
        world.campaign_choice
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

    print_store(&world.store, &mut facts, &world.game);

    problems += print_characters(&world, &paths.save, &mut facts);
    if let Some(settings) = settings {
        print_orders(settings, &world);
    }
    Ok(problems)
}

/// The standing orders and what carrying them out would do to this
/// load — the plan only; nothing moves.
fn print_orders(settings: &Settings, world: &LoadedWorld) {
    let sync = match settings.sync_reagents {
        ReagentSync::On => "on",
        ReagentSync::Off => "off",
    };
    let blueprint_sync = match settings.sync_blueprints {
        BlueprintSync::On => "on",
        BlueprintSync::Off => "off",
    };
    let rule = settings.bulk_duplicates;
    println!(
        "\nstanding orders: {} tab(s) nominated for auto-move, {} for purge; component sync \
         {sync}; blueprint sync {blueprint_sync}; bulk duplicates {rule}",
        settings.auto_move.len(),
        settings.purge_duplicates.len()
    );
    let names = automove::open_names(&world.characters);
    let store = world.store.store();
    for order in StandingOrder::ALL {
        for nomination in settings.nominations(order) {
            let plan = match nominated_tabs(world, nomination, &names) {
                Err(skipped) => skipped.to_string(),
                Ok((tabs, tab)) => match order {
                    StandingOrder::AutoMove => {
                        match bulk::plan_for(tabs, tab, store, &world.game, rule) {
                            Ok(plan) => format!(
                                "would move {} item(s), leaving {} duplicate(s)",
                                plan.moving.len(),
                                plan.duplicates
                            ),
                            Err(error) => error.to_string(),
                        }
                    }
                    StandingOrder::PurgeDuplicates => {
                        match bulk::purge_plan(tabs, tab, store, &world.game) {
                            Ok(doomed) => format!("would delete {} duplicate(s)", doomed.len()),
                            Err(error) => error.to_string(),
                        }
                    }
                },
            };
            println!("  {order} {nomination}: {plan}");
        }
    }
    if settings.sync_reagents == ReagentSync::On
        && let Reagents::Open(doc) = &world.reagents
    {
        let shortfall = bulk::reagent_shortfall(doc.storage(), store);
        let units: u64 = shortfall.iter().map(|short| u64::from(short.units)).sum();
        println!(
            "  component sync would raise {} record(s) by {units} unit(s)",
            shortfall.len()
        );
    }
    if settings.sync_blueprints == BlueprintSync::On
        && let Some(doc) = world.blueprints.doc()
    {
        let missing = bulk::blueprint_shortfall(&doc.formulas().entries, store);
        println!(
            "  blueprint sync would record {} newly learned blueprint(s)",
            missing.len()
        );
    }
}

/// Why a nomination has no tab to plan over on this load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Skipped {
    NotOpen,
    StashNotTyped,
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotOpen => "not open on this load",
            Self::StashNotTyped => "the character's stash is not typed",
        })
    }
}

/// The tabs a nomination names among the loaded documents, with the
/// nominated tab's index.
fn nominated_tabs<'a>(
    world: &'a LoadedWorld,
    nomination: &AutoMoveTab,
    names: &[automove::OpenName<'_>],
) -> Result<(&'a [StashTab], TabIndex), Skipped> {
    match automove::resolve(nomination, &world.campaign, names) {
        None => Err(Skipped::NotOpen),
        Some(AutoMoveTarget::TransferStash(tab)) => Ok((&world.stash.stash().tabs, tab)),
        Some(AutoMoveTarget::CharacterStash { character, tab }) => world
            .characters
            .get(character.value())
            .and_then(CharacterEntry::doc)
            .and_then(|doc| doc.file().stash())
            .map(|stash| (stash.tabs.as_slice(), tab))
            .ok_or(Skipped::StashNotTyped),
    }
}

/// Every character, the one the picker opens on first; the count of
/// those that failed to open.
fn print_characters(world: &LoadedWorld, save: &SaveDir, facts: &mut FactsCache) -> usize {
    println!(
        "\ncharacters: {} found under {} and {}",
        world.characters.len(),
        save.characters_dir(Realm::Main).display(),
        save.characters_dir(Realm::Custom).display()
    );
    match world
        .newest_character
        .and_then(|slot| world.characters.get(slot.value()))
    {
        Some(entry) => println!(
            "  opening on {} ({}; its player.gdc was written last)",
            entry.label(),
            entry.path().display()
        ),
        None => println!("  opening on none"),
    }
    let mut failed = 0;
    for entry in &world.characters {
        match entry {
            CharacterEntry::Loaded(doc) => print_character(doc, facts, &world.game),
            CharacterEntry::Failed { path, error, .. } => {
                println!("  FAILED {}: {error}", path.display());
                failed += 1;
            }
        }
    }
    failed
}

fn print_store(doc: &StoreDoc, facts: &mut FactsCache, game: &GameData) {
    let store = doc.store();
    let status = if doc.path().is_file() {
        String::new()
    } else {
        " (absent; created by the first save)".to_string()
    };
    println!(
        "\nstore: {}{status}: {} items, {} blueprints known",
        doc.path().display(),
        store.len(),
        store.blueprints().len()
    );
    let folded = store.clone().consolidate_stacks(|item| game.is_stack(item));
    if folded.entries > 0 {
        println!("  the window would consolidate its stacks: {folded}");
    }
    for stored in store.items() {
        let bucket = facts.base(game, stored.item()).bucket;
        println!(
            "  {} {} [{}]",
            stored.id(),
            describe(facts, game, stored.item()),
            bucket.label()
        );
    }
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

/// `Prefix Base Suffix xN [Rarity WxH · facets]`, `?` for what the
/// database cannot resolve.
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
    let marks = view.facets.labels();
    let marks = if marks.is_empty() {
        String::new()
    } else {
        format!(" · {}", marks.join(" · "))
    };
    format!(
        "{}{stack} [{rarity} {footprint}{marks}]",
        view.display_name()
    )
}
