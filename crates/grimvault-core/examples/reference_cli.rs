//! The affix reference card, headless: builds the affix table from the
//! installed database exactly as the window does and prints what a
//! search would show, so the grouping, the ranges and the coverage
//! marks can be read against the real records without the GUI.
//!
//! ```text
//! reference_cli affixes [--game DIR] [--prefix | --suffix] [--rarity NAME]
//!                       [--tiers] [--limit N] [TEXT...]
//! ```
//!
//! Read-only: nothing is written. `TEXT` words are joined into the
//! card's search text (name or stat, case-insensitive); `--tiers`
//! prints every record under its entry. The default limit is 20.

mod support;

use std::error::Error;
use std::time::Instant;

use grimvault_core::gamedata::Rarity;
use grimvault_core::reference::{AffixEntry, AffixQuery, AffixTable, Coverage, Position, Tier};

use support::{cli_paths, load_game_data};

const USAGE: &str = "usage: reference_cli affixes [--game DIR] [--prefix | --suffix] \
                     [--rarity NAME] [--tiers] [--limit N] [TEXT...]";

struct Invocation {
    query: AffixQuery,
    tiers: bool,
    limit: usize,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = cli_paths(&args)?;
    let invocation = parse(&paths.rest)?;
    let game = load_game_data(&paths.game_dir)?;
    let started = Instant::now();
    let table = AffixTable::build(&game);
    let count = |position: Position| {
        table
            .entries()
            .iter()
            .filter(|entry| entry.position == position)
            .count()
    };
    let records: usize = table.entries().iter().map(|entry| entry.tiers.len()).sum();
    println!(
        "affix table: {} names ({} prefixes, {} suffixes) over {records} records, built in {:.2?}",
        table.len(),
        count(Position::Prefix),
        count(Position::Suffix),
        started.elapsed()
    );
    let matches: Vec<&AffixEntry> = table
        .matching(&invocation.query)
        .map(|(_, entry)| entry)
        .collect();
    println!("{} of {} match", matches.len(), table.len());
    for entry in matches.iter().take(invocation.limit) {
        print_entry(entry, invocation.tiers);
    }
    if matches.len() > invocation.limit {
        println!(
            "… and {} more (raise --limit)",
            matches.len() - invocation.limit
        );
    }
    Ok(())
}

fn parse(rest: &[String]) -> Result<Invocation, Box<dyn Error>> {
    let mut words = rest.iter();
    match words.next().map(String::as_str) {
        Some("affixes") => {}
        Some(other) => return Err(format!("unknown card {other}\n{USAGE}").into()),
        None => return Err(USAGE.into()),
    }
    let mut query = AffixQuery::default();
    let mut tiers = false;
    let mut limit = 20;
    let mut text: Vec<&str> = Vec::new();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--prefix" => query.position = Some(Position::Prefix),
            "--suffix" => query.position = Some(Position::Suffix),
            "--tiers" => tiers = true,
            "--rarity" => {
                let name = words.next().ok_or("--rarity needs a value")?;
                query.rarity =
                    Some(Rarity::parse(name).ok_or_else(|| format!("unknown rarity {name}"))?);
            }
            "--limit" => {
                limit = words.next().ok_or("--limit needs a value")?.parse()?;
            }
            flag if flag.starts_with("--") => {
                return Err(format!("unknown flag {flag}\n{USAGE}").into());
            }
            plain => text.push(plain),
        }
    }
    query.text = text.join(" ");
    Ok(Invocation {
        query,
        tiers,
        limit,
    })
}

fn print_entry(entry: &AffixEntry, tiers: bool) {
    let levels = entry.levels.map_or_else(
        || "no level requirement".to_string(),
        |range| format!("Lv {range}"),
    );
    println!(
        "{} — {} {} — {levels} — {} record{}",
        entry.name,
        entry.rarity.map_or("unclassified", Rarity::label),
        entry.position.label(),
        entry.tiers.len(),
        if entry.tiers.len() == 1 { "" } else { "s" }
    );
    for grant in &entry.grants {
        match grant.coverage {
            Coverage::Every => println!("  {}", grant.line.text),
            Coverage::Some { tiers, of } => {
                println!("  {}  ({tiers} of {of} records)", grant.line.text);
            }
        }
    }
    if tiers {
        for tier in &entry.tiers {
            print_tier(tier);
        }
    }
}

fn print_tier(tier: &Tier) {
    let level = tier
        .level
        .map_or_else(|| "  — ".to_string(), |level| format!("Lv{level:>3}"));
    let lines: Vec<&str> = tier
        .stats
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    println!(
        "    {level}  {}: {}",
        tier.record.file_stem(),
        lines.join(" | ")
    );
}
