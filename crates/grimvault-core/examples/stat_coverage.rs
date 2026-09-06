//! Coverage proof for the stat renderer: renders every item in the
//! transfer stash of every campaign, the component storage, every
//! character's sacks, own stash and equipment, and the vault store,
//! then prints how many lines came out, every attribute the renderer
//! could not turn into text with a count, and a few tooltips in full.
//!
//! ```text
//! stat_coverage [--game DIR] [--save DIR] [--store FILE] [--samples N] [--item RECORD]...
//! ```
//!
//! Read-only: nothing is written. Paths not given come from the app's
//! saved settings, like `vault_cli`. `--item` renders a bare record as
//! an item (no affixes) and prints its tooltip.

mod support;

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::{PlayerFile, Realm};
use grimvault_core::gst::GstFile;
use grimvault_core::item::Item;
use grimvault_core::stats::{self, BlockSource, ItemDetails, Section, UnrenderedReason};
use grimvault_core::store::VaultStore;
use univault_io::read_verified;

use support::{cli_paths, describe, load_game_data};

struct Tally {
    items: usize,
    lines: usize,
    unrendered: BTreeMap<String, usize>,
    missing_records: BTreeMap<String, usize>,
    missing_tags: BTreeMap<String, usize>,
    samples: Vec<(String, ItemDetails)>,
    wanted_samples: usize,
}

impl Tally {
    fn add(&mut self, game: &GameData, item: &Item) {
        if item.is_empty() {
            return;
        }
        let details = stats::item_details(game, item);
        self.items += 1;
        self.lines += details
            .blocks
            .iter()
            .map(|block| block.lines.len())
            .sum::<usize>()
            + details.requirement_lines.len();
        for gap in &details.unrendered {
            match &gap.reason {
                UnrenderedReason::UnknownAttribute => {
                    *self.unrendered.entry(gap.variable.clone()).or_default() += 1;
                }
                UnrenderedReason::MissingTag(tag) => {
                    *self
                        .missing_tags
                        .entry(format!("{} ({tag})", gap.variable))
                        .or_default() += 1;
                }
                UnrenderedReason::MissingRecord(id) => {
                    *self
                        .missing_records
                        .entry(format!("{} ({id})", gap.variable))
                        .or_default() += 1;
                }
            }
        }
        if self.samples.len() < self.wanted_samples && wants_sample(item, &details) {
            self.samples.push((describe(game, item), details));
        }
    }
}

/// Prefer items that exercise several blocks: an affix or a component
/// or a granted skill.
fn wants_sample(item: &Item, details: &ItemDetails) -> bool {
    let blocks = details
        .blocks
        .iter()
        .filter(|block| !block.lines.is_empty())
        .count();
    let granted = details.blocks.iter().any(|block| {
        block
            .lines
            .iter()
            .any(|line| line.section == Section::GrantedSkill)
    });
    blocks >= 2 || granted || !item.prefix_name.is_empty()
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = cli_paths(&args)?;
    let mut wanted_samples = 3;
    let mut bare_records: Vec<String> = Vec::new();
    let mut rest = paths.rest.iter();
    while let Some(word) = rest.next() {
        match word.as_str() {
            "--samples" => {
                wanted_samples = rest.next().ok_or("--samples needs a number")?.parse()?;
            }
            "--item" => bare_records.push(rest.next().ok_or("--item needs a record")?.clone()),
            other => return Err(format!("unexpected argument {other}").into()),
        }
    }
    let game = load_game_data(&paths.game_dir)?;
    let mut tally = Tally {
        items: 0,
        lines: 0,
        unrendered: BTreeMap::new(),
        missing_records: BTreeMap::new(),
        missing_tags: BTreeMap::new(),
        samples: Vec::new(),
        wanted_samples,
    };

    for record in &bare_records {
        let item = Item {
            base_name: record.clone(),
            ..Item::default()
        };
        let details = stats::item_details(&game, &item);
        print_tooltip(&describe(&game, &item), &details);
    }

    for stash in stash_files(&paths.save_dir)? {
        let file = GstFile::parse(&read_verified(&stash)?)?;
        if let Some(transfer) = file.transfer_stash() {
            for tab in &transfer.tabs {
                for placed in &tab.items {
                    tally.add(&game, &placed.item);
                }
            }
        }
        if let Some(storage) = file.reagent_storage() {
            for entry in &storage.entries {
                let item = Item {
                    base_name: entry.record.clone(),
                    stack_count: entry.count,
                    ..Item::default()
                };
                tally.add(&game, &item);
            }
        }
        println!("read {}", stash.display());
    }

    for (realm, path) in character_paths(&paths.save_dir)? {
        let player = PlayerFile::parse(&read_verified(&path)?)?;
        if let Some(inventory) = player.inventory() {
            for sack in inventory.sacks() {
                for placed in &sack.items {
                    tally.add(&game, &placed.item);
                }
            }
            for equipped in inventory.equipped() {
                tally.add(&game, &equipped.item);
            }
        }
        if let Some(stash) = player.stash() {
            for tab in &stash.tabs {
                for placed in &tab.items {
                    tally.add(&game, &placed.item);
                }
            }
        }
        println!("read {} [{realm}]", path.display());
    }

    if paths.store_path.is_file() {
        let store = VaultStore::from_json(&read_verified(&paths.store_path)?)?;
        for stored in store.items() {
            tally.add(&game, stored.item());
        }
        println!(
            "read {} ({} items)",
            paths.store_path.display(),
            store.len()
        );
    }

    println!("\n{} items rendered, {} lines", tally.items, tally.lines);
    print_counts("unknown attributes", &tally.unrendered);
    print_counts("missing tags", &tally.missing_tags);
    print_counts("missing records", &tally.missing_records);
    for (name, details) in &tally.samples {
        print_tooltip(name, details);
    }
    Ok(())
}

fn print_counts(title: &str, counts: &BTreeMap<String, usize>) {
    let total: usize = counts.values().sum();
    println!("\n{title}: {} distinct, {total} occurrences", counts.len());
    let mut sorted: Vec<(&String, &usize)> = counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    for (name, count) in sorted {
        println!("  {count:6} {name}");
    }
}

fn print_tooltip(name: &str, details: &ItemDetails) {
    println!("\n=== {name}");
    for block in &details.blocks {
        if block.lines.is_empty() && block.flavor.is_none() {
            continue;
        }
        match (&block.source, &block.title) {
            (BlockSource::Base, _) => {}
            (source, Some(title)) => println!("  -- {} ({title})", source.label()),
            (source, None) => println!("  -- {}", source.label()),
        }
        let mut section = None;
        for line in &block.lines {
            if section.is_some_and(|last| last != line.section) {
                println!();
            }
            section = Some(line.section);
            println!(
                "  {:<10} {}",
                format!("{:?}", line.emphasis).to_lowercase(),
                line.text
            );
        }
        if let Some(flavor) = &block.flavor {
            println!("  {:<10} {flavor}", "flavor");
        }
    }
    if let Some(set) = &details.set {
        println!("  -- Set: {}", set.name);
        for member in &set.members {
            println!("  {:<10}     {member}", "member");
        }
        for tier in &set.tiers {
            for line in &tier.lines {
                println!("  {:<10} ({}) {}", "set", tier.pieces, line.text);
            }
        }
    }
    for line in &details.requirement_lines {
        println!("  {:<10} {}", "require", line.text);
    }
    for gap in &details.unrendered {
        println!("  {:<10} {gap}", "GAP");
    }
}

/// `transfer.gst` and `reagents.gst` of the main campaign and every
/// mod folder that has a stash.
fn stash_files(save_dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut folders = vec![save_dir.to_path_buf()];
    for entry in fs::read_dir(save_dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("transfer.gst").is_file() {
            folders.push(path);
        }
    }
    Ok(folders
        .into_iter()
        .flat_map(|folder| [folder.join("transfer.gst"), folder.join("reagents.gst")])
        .filter(|path| path.is_file())
        .collect())
}

fn character_paths(save_dir: &Path) -> Result<Vec<(Realm, PathBuf)>, Box<dyn Error>> {
    let mut found = Vec::new();
    for realm in Realm::ALL {
        let dir = save_dir.join(realm.dir_name());
        if !dir.is_dir() {
            continue;
        }
        let mut paths: Vec<PathBuf> = fs::read_dir(dir)?
            .flatten()
            .map(|entry| entry.path().join("player.gdc"))
            .filter(|path| path.is_file())
            .collect();
        paths.sort();
        found.extend(paths.into_iter().map(|path| (realm, path)));
    }
    Ok(found)
}
