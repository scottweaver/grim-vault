//! The store search, headless: every flag is one typed constraint of
//! `grimvault_core::search::Query`, so the whole matcher and every sort
//! key can be proven against a real vault store without the window.
//!
//! ```text
//! search_cli [--game DIR] [--save DIR] [--store FILE]
//!            [--name TEXT] [--affix TEXT]
//!            [--stat TEXT [--min N] [--max N]]... [--affix-stat TEXT [--min N] [--max N]]...
//!            [--level N] [--physique N] [--cunning N] [--spirit N]
//!            [--set [TEXT]] [--rarity NAME] [--bucket NAME] [--group NAME]
//!            [--mi] [--dr] [--ascended | --upgradeable] [--socketed yes|no]
//!            [--sort name|rarity|level|type] [--desc] [--limit N]
//! ```
//!
//! Read-only: the store is parsed, never written. `--min` / `--max`
//! bound the most recent `--stat` / `--affix-stat`; `--set` alone
//! means any set. Prints how many items match, how many could not be
//! decided, and the first `--limit` (default 20) matches in the sort
//! order: id, name, rarity, level requirement, type.

mod support;

use std::error::Error;

use grimvault_core::bucket::{Bucket, Group};
use grimvault_core::facets::AscensionTable;
use grimvault_core::gamedata::Rarity;
use grimvault_core::search::{
    AscensionFilter, CategoryFilter, Constraint, Criterion, Query, Resolved, SetFilter,
    SocketFilter, SortKey, SortRank, ValueBounds, Verdict,
};
use grimvault_core::store::VaultStore;
use univault_io::read_verified;

use support::{cli_paths, load_game_data};

const USAGE: &str = "usage: search_cli [--game DIR] [--save DIR] [--store FILE] [--name TEXT] \
                     [--affix TEXT] [--stat TEXT [--min N] [--max N]]... \
                     [--affix-stat TEXT [--min N] [--max N]]... [--level N] [--physique N] \
                     [--cunning N] [--spirit N] [--set [TEXT]] [--rarity NAME] [--bucket NAME] \
                     [--group NAME] [--mi] [--dr] [--ascended | --upgradeable] \
                     [--socketed yes|no] [--sort name|rarity|level|type] [--desc] [--limit N]";

struct Invocation {
    query: Query,
    sort: SortKey,
    descending: bool,
    limit: usize,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = cli_paths(&args)?;
    let invocation = parse(&paths.rest)?;
    let game = load_game_data(&paths.game_dir)?;
    let store = VaultStore::from_json(&read_verified(&paths.store_path)?)?;
    let table = AscensionTable::read(&game);

    let mut matches: Vec<(SortRank, String)> = Vec::new();
    let mut unresolved = 0;
    for stored in store.items() {
        let resolved = Resolved::of(&game, &table, stored.item());
        let subject = resolved.subject(stored.item());
        match invocation.query.verdict(&subject) {
            Verdict::Matches => {
                let rank = SortRank::of(&subject);
                let level = rank
                    .level()
                    .map_or_else(|| "–".to_string(), |level| format!("Lv {level}"));
                let rarity = subject.facets.displayed_rarity().map_or("?", Rarity::label);
                let bucket = match subject.base {
                    grimvault_core::facets::BaseEvidence::Unresolved => "unknown record",
                    grimvault_core::facets::BaseEvidence::Known { bucket, .. } => bucket.label(),
                };
                matches.push((
                    rank,
                    format!(
                        "{:>6}  {}  [{rarity}]  {level}  {bucket}",
                        stored.id(),
                        resolved.name()
                    ),
                ));
            }
            Verdict::Excluded => {}
            Verdict::Unresolved => unresolved += 1,
        }
    }
    matches.sort_by(|(a, _), (b, _)| {
        let ordering = a.compare(b, invocation.sort);
        if invocation.descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    println!(
        "{} of {} match, {unresolved} unresolved; sorted by {}{}",
        matches.len(),
        store.len(),
        invocation.sort.label().to_lowercase(),
        if invocation.descending {
            " descending"
        } else {
            ""
        }
    );
    for (_, line) in matches.iter().take(invocation.limit) {
        println!("{line}");
    }
    if matches.len() > invocation.limit {
        println!("… and {} more", matches.len() - invocation.limit);
    }
    Ok(())
}

fn parse(args: &[String]) -> Result<Invocation, Box<dyn Error>> {
    let mut query = Query::default();
    let mut sort = SortKey::Name;
    let mut descending = false;
    let mut limit = 20;
    let mut words = args.iter().peekable();
    while let Some(word) = words.next() {
        let mut value = || -> Result<&String, Box<dyn Error>> {
            words
                .next()
                .ok_or_else(|| format!("{word} needs a value\n{USAGE}").into())
        };
        match word.as_str() {
            "--name" => query.name.clone_from(value()?),
            "--affix" => query.criteria.push(Criterion::HasAffix {
                text: value()?.clone(),
            }),
            "--stat" => query.criteria.push(Criterion::StatContains {
                text: value()?.clone(),
                bounds: ValueBounds::ANY,
            }),
            "--affix-stat" => query.criteria.push(Criterion::AffixStat {
                text: value()?.clone(),
                bounds: ValueBounds::ANY,
            }),
            "--min" | "--max" => bound(&mut query, word, value()?.parse()?)?,
            "--level" => query.requirements.level = Some(value()?.parse()?),
            "--physique" => query.requirements.physique = Some(value()?.parse()?),
            "--cunning" => query.requirements.cunning = Some(value()?.parse()?),
            "--spirit" => query.requirements.spirit = Some(value()?.parse()?),
            "--set" => {
                query.set = match words.peek() {
                    Some(next) if !next.starts_with("--") => SetFilter::Named {
                        text: words.next().cloned().unwrap_or_default(),
                    },
                    Some(_) | None => SetFilter::Member,
                }
            }
            "--rarity" => query.rarity = Some(rarity(value()?)?),
            "--bucket" => {
                query.category = CategoryFilter::Bucket {
                    bucket: bucket(value()?)?,
                }
            }
            "--group" => {
                query.category = CategoryFilter::Group {
                    group: group(value()?)?,
                }
            }
            "--mi" => query.monster_infrequent = Constraint::Required,
            "--dr" => query.double_rare = Constraint::Required,
            "--ascended" => query.ascension = AscensionFilter::Ascended,
            "--upgradeable" => query.ascension = AscensionFilter::Upgradeable,
            "--socketed" => query.socket = socket(value()?)?,
            "--sort" => sort = sort_key(value()?)?,
            "--desc" => descending = true,
            "--limit" => limit = value()?.parse()?,
            _ => return Err(format!("unexpected argument {word:?}\n{USAGE}").into()),
        }
    }
    Ok(Invocation {
        query,
        sort,
        descending,
        limit,
    })
}

/// `--min` / `--max` bound the most recent `--stat` / `--affix-stat`.
fn bound(query: &mut Query, word: &str, number: f32) -> Result<(), Box<dyn Error>> {
    let bounds = query
        .criteria
        .last_mut()
        .and_then(Criterion::bounds_mut)
        .ok_or(format!(
            "{word} must follow --stat or --affix-stat\n{USAGE}"
        ))?;
    if word == "--min" {
        bounds.min = Some(number);
    } else {
        bounds.max = Some(number);
    }
    Ok(())
}

fn rarity(name: &str) -> Result<Rarity, Box<dyn Error>> {
    Rarity::parse(name).ok_or_else(|| format!("unknown rarity {name:?}").into())
}

fn bucket(name: &str) -> Result<Bucket, Box<dyn Error>> {
    Bucket::ALL
        .into_iter()
        .find(|bucket| named(bucket.label(), name) || named(&format!("{bucket:?}"), name))
        .ok_or_else(|| format!("unknown bucket {name:?}").into())
}

fn group(name: &str) -> Result<Group, Box<dyn Error>> {
    Group::ALL
        .into_iter()
        .find(|group| named(group.label(), name))
        .ok_or_else(|| format!("unknown group {name:?}").into())
}

fn socket(word: &str) -> Result<SocketFilter, Box<dyn Error>> {
    match word {
        "yes" | "true" => Ok(SocketFilter::Socketed),
        "no" | "false" => Ok(SocketFilter::Unsocketed),
        other => Err(format!("--socketed takes yes or no, not {other:?}").into()),
    }
}

fn sort_key(word: &str) -> Result<SortKey, Box<dyn Error>> {
    match word {
        "name" => Ok(SortKey::Name),
        "rarity" => Ok(SortKey::Rarity),
        "level" => Ok(SortKey::Level),
        "type" | "category" => Ok(SortKey::Category),
        other => Err(format!("unknown sort key {other:?}\n{USAGE}").into()),
    }
}

/// Case-insensitive, ignoring spaces, hyphens and parentheses, so
/// `ranged1h`, `Ranged (One-Handed)` and `RangedOneHanded` all name
/// the same bucket.
fn named(label: &str, wanted: &str) -> bool {
    let fold = |text: &str| {
        text.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
    };
    fold(label) == fold(wanted)
}
