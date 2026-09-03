//! The first usable loop, on a **copy** of a real save directory: vault
//! an item out of `transfer.gst` into a `grimvault-store` document and
//! place it back, with every write backup-first and the written stash
//! re-parsed to prove it.
//!
//! ```text
//! vault_cli <game dir> <save dir> <store.json> list
//! vault_cli <game dir> <save dir> <store.json> vault <tab> <index>
//! vault_cli <game dir> <save dir> <store.json> place <id> <tab> [x y]
//! vault_cli <game dir> <save dir> <store.json> reagents
//! vault_cli <game dir> <save dir> <store.json> vault-reagent <index> <count>
//! vault_cli <game dir> <save dir> <store.json> place-reagent <id>
//! vault_cli <game dir> <save dir> <store.json> characters
//! vault_cli <game dir> <save dir> <store.json> vault-sack <character> <sack> <index>
//! vault_cli <game dir> <save dir> <store.json> place-sack <id> <character> <sack> [x y]
//! vault_cli <game dir> <save dir> <store.json> money <character> [<iron bits>]
//! ```
//!
//! The `reagent` commands work the component / crafting-material
//! storage, `reagents.gst`, the same way; the `sack` and `money`
//! commands work a character's `main/_<character>/player.gdc`, which
//! is written only when every block of it is typed.
//!
//! Write policy. Each invocation is one load, so every file goes
//! through `backup_first_write` under the `grimvault-bak` policy (five
//! kept): the game files because ARCHITECTURE.md requires backup-first
//! for every game-owned write, and the store — this app's own
//! authoritative vault — because losing it is as bad as losing the
//! stash. The *destination* of a move is always written before its
//! *source* (`vault`: store then stash; `place`: stash then store), so
//! a crash between the two writes leaves the item in both places rather
//! than in neither.

mod support;

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use grimvault_core::bucket::{Bucket, Group};
use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::PlayerFile;
use grimvault_core::gst::{GstFile, ReagentStorage, TransferStash};
use grimvault_core::item::Item;
use grimvault_core::loaded::Loaded;
use grimvault_core::reagents::{ReagentKind, ReagentKinds};
use grimvault_core::store::{ItemOrigin, StoredItem, StoredItemId, Timestamp, VaultStore};
use grimvault_core::transfer::{self, ItemIndex, ReagentIndex, SackIndex, TabIndex};
use univault_engine::ids::{GridPos, RecordId};
use univault_io::{BackupPolicy, backup_first_write, read_verified};

use support::{cli_paths, describe, load_game_data};

const BACKUPS: BackupPolicy = BackupPolicy::new("grimvault-bak", 5);
const USAGE: &str = "usage: vault_cli [--game DIR] [--save DIR] [--store FILE] \
                     (list | vault <tab> <index> | place <id> <tab> [x y] \
                     | reagents | vault-reagent <index> <count> | place-reagent <id> \
                     | characters | vault-sack <character> <sack> <index> \
                     | place-sack <id> <character> <sack> [x y] | money <character> [<iron bits>]) \
                     — paths not given come from the app's saved settings";

enum Command {
    List,
    Vault {
        tab: TabIndex,
        index: ItemIndex,
    },
    Place {
        id: StoredItemId,
        tab: TabIndex,
        pos: Option<GridPos>,
    },
    Reagents,
    VaultReagent {
        index: ReagentIndex,
        count: u32,
    },
    PlaceReagent {
        id: StoredItemId,
    },
    Character(CharacterCommand),
}

/// The commands that work a `player.gdc`.
enum CharacterCommand {
    List,
    VaultSack {
        character: String,
        sack: SackIndex,
        index: ItemIndex,
    },
    PlaceSack {
        id: StoredItemId,
        character: String,
        sack: SackIndex,
        pos: Option<GridPos>,
    },
    Money {
        character: String,
        amount: Option<u32>,
    },
}

struct Invocation {
    game_dir: PathBuf,
    save_dir: PathBuf,
    store_path: PathBuf,
    command: Command,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Invocation {
        game_dir,
        save_dir,
        store_path,
        command,
    } = parse_args(&args)?;
    let game_data = load_game_data(&game_dir)?;
    let stash_path = save_dir.join("transfer.gst");
    let reagents_path = save_dir.join("reagents.gst");
    let mut stash = Loaded::<GstFile>::load(read_verified(&stash_path)?)?;
    let mut store = load_store(&store_path)?;
    println!(
        "loaded {} ({} bytes, lossless) and {} ({} items)",
        stash_path.display(),
        stash.baseline().len(),
        store_path.display(),
        store.len()
    );

    match command {
        Command::List => {
            print_stash(&game_data, transfer_stash(stash.model())?);
            print_store(&game_data, &store);
        }
        Command::Vault { tab, index } => {
            let id = transfer::vault_from_stash(
                transfer_stash_mut(stash.model_mut())?,
                tab,
                index,
                &mut store,
                now()?,
            )?;
            println!("vaulted tab {tab} item {index} as stored item {id}");
            write_store(&store_path, &store)?;
            write_stash(&stash_path, &stash)?;
            print_stash(&game_data, transfer_stash(&reparse(&stash_path)?)?);
            print_store(&game_data, &store);
        }
        Command::Place { id, tab, pos } => {
            let target = transfer_stash_mut(stash.model_mut())?;
            let landed = match pos {
                Some(pos) => {
                    transfer::place_in_stash_at(&mut store, id, target, tab, pos, &game_data)?;
                    pos
                }
                None => transfer::place_in_stash(&mut store, id, target, tab, &game_data)?,
            };
            println!(
                "placed stored item {id} in tab {tab} at ({},{})",
                landed.x, landed.y
            );
            write_stash(&stash_path, &stash)?;
            write_store(&store_path, &store)?;
            print_stash(&game_data, transfer_stash(&reparse(&stash_path)?)?);
            print_store(&game_data, &store);
        }
        Command::Reagents => {
            let reagents = load_reagents(&reagents_path)?;
            print_reagents(&game_data, reagent_storage(reagents.model())?);
        }
        Command::VaultReagent { index, count } => {
            let mut reagents = load_reagents(&reagents_path)?;
            let id = transfer::vault_from_reagents(
                reagent_storage_mut(reagents.model_mut())?,
                index,
                count,
                &mut store,
                now()?,
            )?;
            println!("vaulted {count} of storage entry {index} as stored item {id}");
            write_store(&store_path, &store)?;
            write_stash(&reagents_path, &reagents)?;
            print_reagents(&game_data, reagent_storage(&reparse(&reagents_path)?)?);
            print_store(&game_data, &store);
        }
        Command::PlaceReagent { id } => {
            let mut reagents = load_reagents(&reagents_path)?;
            transfer::place_in_reagents(
                &mut store,
                id,
                reagent_storage_mut(reagents.model_mut())?,
                &game_data,
            )?;
            println!("placed stored item {id} in the component / crafting-material storage");
            write_stash(&reagents_path, &reagents)?;
            write_store(&store_path, &store)?;
            print_reagents(&game_data, reagent_storage(&reparse(&reagents_path)?)?);
            print_store(&game_data, &store);
        }
        Command::Character(command) => {
            run_character(command, &save_dir, &store_path, &game_data, &mut store)?;
        }
    }
    Ok(())
}

fn run_character(
    command: CharacterCommand,
    save_dir: &Path,
    store_path: &Path,
    game_data: &GameData,
    store: &mut VaultStore,
) -> Result<(), Box<dyn Error>> {
    match command {
        CharacterCommand::List => {
            for path in character_paths(save_dir)? {
                let player = load_player(&path)?;
                print_character(game_data, player.model());
            }
        }
        CharacterCommand::VaultSack {
            character,
            sack,
            index,
        } => {
            let path = character_path(save_dir, &character);
            let mut player = load_player(&path)?;
            let id = transfer::vault_from_sack(player.model_mut(), sack, index, store, now()?)?;
            println!("vaulted {character}'s sack {sack} item {index} as stored item {id}");
            write_store(store_path, store)?;
            write_player(&path, &player)?;
            print_character(game_data, &reparse_player(&path)?);
            print_store(game_data, store);
        }
        CharacterCommand::PlaceSack {
            id,
            character,
            sack,
            pos,
        } => {
            let path = character_path(save_dir, &character);
            let mut player = load_player(&path)?;
            let landed = match pos {
                Some(pos) => {
                    transfer::place_in_sack_at(
                        store,
                        id,
                        player.model_mut(),
                        sack,
                        pos,
                        game_data,
                    )?;
                    pos
                }
                None => transfer::place_in_sack(store, id, player.model_mut(), sack, game_data)?,
            };
            println!(
                "placed stored item {id} in {character}'s sack {sack} at ({},{})",
                landed.x, landed.y
            );
            write_player(&path, &player)?;
            write_store(store_path, store)?;
            print_character(game_data, &reparse_player(&path)?);
            print_store(game_data, store);
        }
        CharacterCommand::Money { character, amount } => {
            let path = character_path(save_dir, &character);
            let mut player = load_player(&path)?;
            let info = player
                .model_mut()
                .character_info_mut()
                .ok_or("player.gdc carries no typed character info (block 1)")?;
            match amount {
                None => println!("{character} has {} iron bits", info.money),
                Some(amount) => {
                    let before = info.money;
                    info.money = amount;
                    println!("{character}'s iron bits: {before} -> {amount}");
                    write_player(&path, &player)?;
                    let written = reparse_player(&path)?;
                    println!(
                        "re-read iron bits: {}",
                        written.character_info().map_or(0, |info| info.money)
                    );
                }
            }
        }
    }
    Ok(())
}

fn character_path(save_dir: &Path, character: &str) -> PathBuf {
    save_dir
        .join("main")
        .join(format!("_{character}"))
        .join("player.gdc")
}

fn character_paths(save_dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(save_dir.join("main"))?
        .flatten()
        .map(|entry| entry.path().join("player.gdc"))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    Ok(paths)
}

fn load_player(path: &Path) -> Result<Loaded<PlayerFile>, Box<dyn Error>> {
    let player = Loaded::<PlayerFile>::load(read_verified(path)?)?;
    let typing = if player.model().is_fully_typed() {
        "every block typed"
    } else {
        "an opaque block remains: read-only"
    };
    println!(
        "loaded {} ({} bytes, lossless; {typing})",
        path.display(),
        player.baseline().len()
    );
    Ok(player)
}

/// A character is written only when every block is typed: `encode`
/// would refuse a re-key of an opaque block anyway, but the refusal
/// here comes before the backup is taken.
fn write_player(path: &Path, player: &Loaded<PlayerFile>) -> Result<(), Box<dyn Error>> {
    if !player.model().is_fully_typed() {
        return Err(format!(
            "{} carries an opaque block, so it cannot be written",
            path.display()
        )
        .into());
    }
    let bytes = player.encode()?;
    let backup = backup_first_write(path, &bytes, BACKUPS)?;
    report_write(path, bytes.len(), backup.as_deref());
    Ok(())
}

fn reparse_player(path: &Path) -> Result<PlayerFile, Box<dyn Error>> {
    let bytes = read_verified(path)?;
    let file = PlayerFile::parse(&bytes)?;
    let round_trip = if file.encode()? == bytes {
        "byte-identical"
    } else {
        "NOT byte-identical"
    };
    println!("re-parsed {} after write: {round_trip}", path.display());
    Ok(file)
}

fn print_character(game_data: &GameData, player: &PlayerFile) {
    let header = player.header();
    println!(
        "\ncharacter {} (level {}): {} iron bits",
        header.name,
        header.level,
        player.character_info().map_or(0, |info| info.money)
    );
    let sacks = player
        .inventory()
        .map_or(&[][..], |inventory| inventory.sacks());
    for (slot, sack) in sacks.iter().enumerate() {
        let dims = SackIndex::new(u32::try_from(slot).unwrap_or(u32::MAX)).dimensions();
        println!(
            "  sack {slot} ({}x{}): {} items",
            dims.width,
            dims.height,
            sack.items.len()
        );
        for (index, placed) in sack.items.iter().enumerate() {
            println!(
                "    [{index}] ({:>2},{:>2}) {}",
                placed.x,
                placed.y,
                describe(game_data, &placed.item)
            );
        }
    }
    let tabs = player.stash().map_or(&[][..], |stash| &stash.tabs[..]);
    for (slot, tab) in tabs.iter().enumerate() {
        println!(
            "  own stash tab {slot} ({}x{}): {} items",
            tab.width,
            tab.height,
            tab.items.len()
        );
    }
}

fn parse_args(args: &[String]) -> Result<Invocation, Box<dyn Error>> {
    let paths = cli_paths(args)?;
    let [command, rest @ ..] = paths.rest.as_slice() else {
        return Err(USAGE.into());
    };
    let command = match (command.as_str(), rest) {
        ("list", []) => Command::List,
        ("vault", [tab, index]) => Command::Vault {
            tab: TabIndex::new(tab.parse()?),
            index: ItemIndex::new(index.parse()?),
        },
        ("place", [id, tab]) => Command::Place {
            id: StoredItemId::new(id.parse()?),
            tab: TabIndex::new(tab.parse()?),
            pos: None,
        },
        ("place", [id, tab, x, y]) => Command::Place {
            id: StoredItemId::new(id.parse()?),
            tab: TabIndex::new(tab.parse()?),
            pos: Some(GridPos {
                x: x.parse()?,
                y: y.parse()?,
            }),
        },
        ("reagents", []) => Command::Reagents,
        ("vault-reagent", [index, count]) => Command::VaultReagent {
            index: ReagentIndex::new(index.parse()?),
            count: count.parse()?,
        },
        ("place-reagent", [id]) => Command::PlaceReagent {
            id: StoredItemId::new(id.parse()?),
        },
        ("characters", []) => Command::Character(CharacterCommand::List),
        ("vault-sack", [character, sack, index]) => {
            Command::Character(CharacterCommand::VaultSack {
                character: character.clone(),
                sack: SackIndex::new(sack.parse()?),
                index: ItemIndex::new(index.parse()?),
            })
        }
        ("place-sack", [id, character, sack]) => Command::Character(CharacterCommand::PlaceSack {
            id: StoredItemId::new(id.parse()?),
            character: character.clone(),
            sack: SackIndex::new(sack.parse()?),
            pos: None,
        }),
        ("place-sack", [id, character, sack, x, y]) => {
            Command::Character(CharacterCommand::PlaceSack {
                id: StoredItemId::new(id.parse()?),
                character: character.clone(),
                sack: SackIndex::new(sack.parse()?),
                pos: Some(GridPos {
                    x: x.parse()?,
                    y: y.parse()?,
                }),
            })
        }
        ("money", [character]) => Command::Character(CharacterCommand::Money {
            character: character.clone(),
            amount: None,
        }),
        ("money", [character, amount]) => Command::Character(CharacterCommand::Money {
            character: character.clone(),
            amount: Some(amount.parse()?),
        }),
        _ => return Err(USAGE.into()),
    };
    Ok(Invocation {
        game_dir: paths.game_dir,
        save_dir: paths.save_dir,
        store_path: paths.store_path,
        command,
    })
}

fn now() -> Result<Timestamp, Box<dyn Error>> {
    Ok(Timestamp::from_unix_seconds(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
    ))
}

fn transfer_stash(file: &GstFile) -> Result<&TransferStash, Box<dyn Error>> {
    file.transfer_stash()
        .ok_or_else(|| "transfer.gst carries no typed transfer stash (block 18)".into())
}

fn transfer_stash_mut(file: &mut GstFile) -> Result<&mut TransferStash, Box<dyn Error>> {
    file.transfer_stash_mut()
        .ok_or_else(|| "transfer.gst carries no typed transfer stash (block 18)".into())
}

fn reagent_storage(file: &GstFile) -> Result<&ReagentStorage, Box<dyn Error>> {
    file.reagent_storage()
        .ok_or_else(|| "reagents.gst carries no typed reagent storage (block 20)".into())
}

fn reagent_storage_mut(file: &mut GstFile) -> Result<&mut ReagentStorage, Box<dyn Error>> {
    file.reagent_storage_mut()
        .ok_or_else(|| "reagents.gst carries no typed reagent storage (block 20)".into())
}

fn load_reagents(path: &Path) -> Result<Loaded<GstFile>, Box<dyn Error>> {
    let reagents = Loaded::<GstFile>::load(read_verified(path)?)?;
    println!(
        "loaded {} ({} bytes, lossless)",
        path.display(),
        reagents.baseline().len()
    );
    Ok(reagents)
}

fn load_store(path: &Path) -> Result<VaultStore, Box<dyn Error>> {
    if path.is_file() {
        Ok(VaultStore::from_json(&read_verified(path)?)?)
    } else {
        println!(
            "{} does not exist yet; starting an empty store",
            path.display()
        );
        Ok(VaultStore::new())
    }
}

fn write_stash(path: &Path, stash: &Loaded<GstFile>) -> Result<(), Box<dyn Error>> {
    let bytes = stash.encode()?;
    let backup = backup_first_write(path, &bytes, BACKUPS)?;
    report_write(path, bytes.len(), backup.as_deref());
    Ok(())
}

fn write_store(path: &Path, store: &VaultStore) -> Result<(), Box<dyn Error>> {
    let bytes = store.to_json();
    let backup = backup_first_write(path, &bytes, BACKUPS)?;
    report_write(path, bytes.len(), backup.as_deref());
    Ok(())
}

fn report_write(path: &Path, len: usize, backup: Option<&Path>) {
    match backup {
        Some(backup) => println!(
            "wrote {} ({len} bytes, verified); backup at {}",
            path.display(),
            backup.display()
        ),
        None => println!("wrote {} ({len} bytes, verified); new file", path.display()),
    }
}

fn reparse(path: &Path) -> Result<GstFile, Box<dyn Error>> {
    let bytes = read_verified(path)?;
    let file = GstFile::parse(&bytes)?;
    let round_trip = if file.encode()? == bytes {
        "byte-identical"
    } else {
        "NOT byte-identical"
    };
    println!("re-parsed {} after write: {round_trip}", path.display());
    Ok(file)
}

fn print_stash(game_data: &GameData, stash: &TransferStash) {
    println!("\ntransfer stash: {} tabs", stash.tabs.len());
    for (tab, contents) in stash.tabs.iter().enumerate() {
        println!(
            "  tab {tab} ({}x{}): {} items",
            contents.width,
            contents.height,
            contents.items.len()
        );
        for (index, placed) in contents.items.iter().enumerate() {
            println!(
                "    [{index}] ({:>2.0},{:>2.0}) {}",
                placed.x,
                placed.y,
                describe(game_data, &placed.item)
            );
        }
    }
}

fn print_reagents(game_data: &GameData, storage: &ReagentStorage) {
    println!(
        "\ncomponent / crafting-material storage: {} entries, {} items",
        storage.entries.len(),
        storage.total_count()
    );
    for kind in ReagentKind::ALL {
        let mut rows: Vec<(String, usize, u32, &str)> = storage
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let item = Item {
                    base_name: entry.record.clone(),
                    ..Item::default()
                };
                (ReagentKind::in_storage(game_data.reagent_kind(&item)) == kind).then(|| {
                    (
                        describe(game_data, &item),
                        index,
                        entry.count,
                        entry.record.as_str(),
                    )
                })
            })
            .collect();
        rows.sort();
        println!("  {} ({})", kind.label(), rows.len());
        for (name, index, count, record) in rows {
            println!("    [{index:>2}] {name} x{count}  {record}");
        }
    }
}

fn print_store(game_data: &GameData, store: &VaultStore) {
    println!("\nstore: {} items", store.len());
    let mut by_bucket: BTreeMap<Bucket, Vec<&StoredItem>> = BTreeMap::new();
    for stored in store.items() {
        by_bucket
            .entry(bucket_of(game_data, stored.item()))
            .or_default()
            .push(stored);
    }
    for group in Group::ALL {
        let buckets: Vec<Bucket> = Bucket::ALL
            .into_iter()
            .filter(|bucket| bucket.group() == group && by_bucket.contains_key(bucket))
            .collect();
        if buckets.is_empty() {
            continue;
        }
        println!("  {}", group.label());
        for bucket in buckets {
            println!("    {}", bucket.label());
            for stored in &by_bucket[&bucket] {
                println!(
                    "      {} {} — from {}, stored at {}",
                    stored.id(),
                    describe(game_data, stored.item()),
                    origin_label(stored.origin()),
                    stored.stored_at().unix_seconds()
                );
            }
        }
    }
}

fn bucket_of(game_data: &GameData, item: &Item) -> Bucket {
    RecordId::parse(item.base_name.clone())
        .and_then(|base| game_data.item_info(&base))
        .and_then(Result::ok)
        .and_then(|info| info.class)
        .map_or(Bucket::Misc, |class| Bucket::of(&class))
}

fn origin_label(origin: &ItemOrigin) -> String {
    match origin {
        ItemOrigin::TransferStash { tab } => format!("transfer stash tab {tab}"),
        ItemOrigin::Character { name, sack } => format!("character {name} sack {sack}"),
        ItemOrigin::CharacterStash { name, tab } => format!("character {name} stash tab {tab}"),
        ItemOrigin::ReagentStorage => "component / crafting-material storage".to_string(),
        ItemOrigin::Unknown => "unknown".to_string(),
    }
}
