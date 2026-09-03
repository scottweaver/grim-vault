//! The first usable loop, on a **copy** of a real save directory: vault
//! an item out of `transfer.gst` into a `grimvault-store` document and
//! place it back, with every write backup-first and the written stash
//! re-parsed to prove it.
//!
//! ```text
//! vault_cli <game dir> <save dir> <store.json> list
//! vault_cli <game dir> <save dir> <store.json> vault <tab> <index>
//! vault_cli <game dir> <save dir> <store.json> place <id> <tab> [x y]
//! ```
//!
//! Write policy. Each invocation is one load, so both files go through
//! `backup_first_write` under the `grimvault-bak` policy (five kept):
//! the stash because ARCHITECTURE.md requires backup-first for every
//! game-owned write, and the store — this app's own authoritative
//! vault — because losing it is as bad as losing the stash. The
//! *destination* of a move is always written before its *source*
//! (`vault`: store then stash; `place`: stash then store), so a crash
//! between the two writes leaves the item in both places rather than in
//! neither.

mod support;

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use grimvault_core::bucket::{Bucket, Group};
use grimvault_core::gamedata::GameData;
use grimvault_core::gst::{GstFile, TransferStash};
use grimvault_core::item::Item;
use grimvault_core::loaded::Loaded;
use grimvault_core::store::{ItemOrigin, StoredItem, StoredItemId, Timestamp, VaultStore};
use grimvault_core::transfer::{self, ItemIndex, TabIndex};
use univault_engine::ids::{GridPos, RecordId};
use univault_io::{BackupPolicy, backup_first_write, read_verified};

use support::{describe, load_game_data};

const BACKUPS: BackupPolicy = BackupPolicy::new("grimvault-bak", 5);
const USAGE: &str = "usage: vault_cli <game dir> <save dir> <store.json> \
                     (list | vault <tab> <index> | place <id> <tab> [x y])";

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
}

struct Invocation {
    game_dir: PathBuf,
    save_dir: PathBuf,
    store_path: PathBuf,
    command: Command,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let invocation = parse_args(&args)?;
    let game_data = load_game_data(&invocation.game_dir)?;
    let stash_path = invocation.save_dir.join("transfer.gst");
    let mut stash = Loaded::<GstFile>::load(read_verified(&stash_path)?)?;
    let mut store = load_store(&invocation.store_path)?;
    println!(
        "loaded {} ({} bytes, lossless) and {} ({} items)",
        stash_path.display(),
        stash.baseline().len(),
        invocation.store_path.display(),
        store.len()
    );

    match invocation.command {
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
            write_store(&invocation.store_path, &store)?;
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
            write_store(&invocation.store_path, &store)?;
            print_stash(&game_data, transfer_stash(&reparse(&stash_path)?)?);
            print_store(&game_data, &store);
        }
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Invocation, Box<dyn Error>> {
    let [game_dir, save_dir, store_path, command, rest @ ..] = args else {
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
        _ => return Err(USAGE.into()),
    };
    Ok(Invocation {
        game_dir: PathBuf::from(game_dir),
        save_dir: PathBuf::from(save_dir),
        store_path: PathBuf::from(store_path),
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
        ItemOrigin::Character { name } => format!("character {name}"),
        ItemOrigin::Unknown => "unknown".to_string(),
    }
}
