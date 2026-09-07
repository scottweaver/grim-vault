//! The first usable loop, on a **copy** of a real save directory: vault
//! an item out of `transfer.gst` into a `grimvault-store` document and
//! place it back, with every write backup-first and the written stash
//! re-parsed to prove it.
//!
//! ```text
//! vault_cli [--mod NAME] <game dir> <save dir> <store.json> list
//! vault_cli <game dir> <save dir> <store.json> vault <tab> <index>
//! vault_cli <game dir> <save dir> <store.json> place <id> <tab> [x y]
//! vault_cli <game dir> <save dir> <store.json> reagents
//! vault_cli <game dir> <save dir> <store.json> vault-reagent <index> <count>
//! vault_cli <game dir> <save dir> <store.json> place-reagent <id>
//! vault_cli <game dir> <save dir> <store.json> characters
//! vault_cli <game dir> <save dir> <store.json> vault-sack <character> <sack> <index>
//! vault_cli <game dir> <save dir> <store.json> place-sack <id> <character> <sack> [x y]
//! vault_cli <game dir> <save dir> <store.json> money <character> [<iron bits>]
//! vault_cli <game dir> <save dir> <store.json> import-gds <file.gds>
//! vault_cli <game dir> <save dir> <store.json> respec-attributes <character>
//! vault_cli <game dir> <save dir> <store.json> respec-masteries <character>
//! vault_cli <game dir> <save dir> <store.json> detach <location> (component | augment)
//! vault_cli <game dir> <save dir> <store.json> attach <location> <id> [<seed>]
//! ```
//!
//! `detach` frees the component or augment of the item at
//! `<location>` into the store as an item of its own; `attach` puts
//! stored item `<id>` (a component or augment; a stack gives one up)
//! into the matching socket of that item under `<seed>` (decimal or
//! `0x…`; the clock when omitted). `<location>` is
//! `stash:<tab>:<index>`, `sack:<character>:<sack>:<index>`, or
//! `own:<character>:<tab>:<index>`.
//!
//! The `reagent` commands work the component / crafting-material
//! storage, `reagents.gst`, the same way; the `sack`, `money`, and
//! `respec` commands work a character's `player.gdc`, which is
//! written only when every block of it is typed. `<character>` is
//! `Name` or `main/Name` for a main-campaign character and
//! `user/Name` for a custom-game (mod) character. The `respec`
//! commands are the plain full refunds of `grimvault_core::respec`,
//! under the rules read from the game's own records. `import-gds`
//! adds a GD Stash export's items to the store, skipping entries
//! already imported, and opens no game file.
//!
//! `--mod NAME` works a mod's shared files under `save/<NAME>/`
//! instead of the main campaign's; characters are the same either way.
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
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use grimvault_core::bucket::{Bucket, Group};
use grimvault_core::campaign::Campaign;
use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::{PlayerFile, Realm};
use grimvault_core::gds;
use grimvault_core::gst::{GstFile, ReagentStorage, TransferStash};
use grimvault_core::item::Item;
use grimvault_core::loaded::Loaded;
use grimvault_core::reagents::{ReagentKind, ReagentKinds};
use grimvault_core::respec::{Reset, RespecRules};
use grimvault_core::socket::{self, Part, Socket};
use grimvault_core::store::{ItemOrigin, StoredItem, StoredItemId, Timestamp, VaultStore};
use grimvault_core::transfer::{self, ItemIndex, ReagentIndex, SackIndex, TabIndex};
use univault_engine::ids::{GridPos, RecordId};
use univault_io::{BackupPolicy, backup_first_write, read_verified};

use support::{cli_paths, describe, load_game_data};

const BACKUPS: BackupPolicy = BackupPolicy::new("grimvault-bak", 5);
const USAGE: &str = "usage: vault_cli [--game DIR] [--save DIR] [--store FILE] [--mod NAME] \
                     (list | vault <tab> <index> | place <id> <tab> [x y] \
                     | reagents | vault-reagent <index> <count> | place-reagent <id> \
                     | characters | vault-sack <character> <sack> <index> \
                     | place-sack <id> <character> <sack> [x y] | money <character> [<iron bits>] \
                     | import-gds <file.gds> \
                     | respec-attributes <character> | respec-masteries <character> \
                     | detach <location> (component | augment) | attach <location> <id> [seed]) \
                     — <character> is Name, main/Name or user/Name; <location> is \
                     stash:<tab>:<index>, sack:<character>:<sack>:<index> or \
                     own:<character>:<tab>:<index>; paths not given come from the app's saved \
                     settings";

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
    ImportGds {
        file: PathBuf,
    },
    Socket(SocketCommand),
}

/// The commands that fill or free an item's sockets.
enum SocketCommand {
    Detach {
        at: Location,
        socket: Socket,
    },
    Attach {
        at: Location,
        id: StoredItemId,
        seed: Option<u32>,
    },
}

impl SocketCommand {
    fn location(&self) -> &Location {
        match self {
            Self::Detach { at, .. } | Self::Attach { at, .. } => at,
        }
    }
}

/// Where an item sits, as the socket commands name it.
enum Location {
    Stash {
        tab: TabIndex,
        index: ItemIndex,
    },
    Sack {
        character: CharacterArg,
        sack: SackIndex,
        index: ItemIndex,
    },
    OwnStash {
        character: CharacterArg,
        tab: TabIndex,
        index: ItemIndex,
    },
}

impl Location {
    /// `stash:<tab>:<index>`, `sack:<character>:<sack>:<index>`, or
    /// `own:<character>:<tab>:<index>`; the character may itself
    /// carry a `/`.
    fn parse(raw: &str) -> Result<Self, Box<dyn Error>> {
        let mut parts = raw.rsplitn(3, ':');
        let index = ItemIndex::new(parts.next().ok_or(USAGE)?.parse()?);
        let slot: u32 = parts.next().ok_or(USAGE)?.parse()?;
        let head = parts.next().ok_or(USAGE)?;
        match head.split_once(':') {
            None if head == "stash" => Ok(Self::Stash {
                tab: TabIndex::new(slot),
                index,
            }),
            Some(("sack", character)) => Ok(Self::Sack {
                character: CharacterArg::parse(character),
                sack: SackIndex::new(slot),
                index,
            }),
            Some(("own", character)) => Ok(Self::OwnStash {
                character: CharacterArg::parse(character),
                tab: TabIndex::new(slot),
                index,
            }),
            Some(_) | None => Err(USAGE.into()),
        }
    }
}

fn parse_socket(raw: &str) -> Result<Socket, Box<dyn Error>> {
    match raw {
        "component" => Ok(Socket::Component),
        "augment" => Ok(Socket::Augment),
        _ => Err(USAGE.into()),
    }
}

fn parse_seed(raw: &str) -> Result<u32, Box<dyn Error>> {
    Ok(match raw.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16)?,
        None => raw.parse()?,
    })
}

/// The commands that work a `player.gdc`.
enum CharacterCommand {
    List,
    VaultSack {
        character: CharacterArg,
        sack: SackIndex,
        index: ItemIndex,
    },
    PlaceSack {
        id: StoredItemId,
        character: CharacterArg,
        sack: SackIndex,
        pos: Option<GridPos>,
    },
    Money {
        character: CharacterArg,
        amount: Option<u32>,
    },
    Respec {
        character: CharacterArg,
        reset: Reset,
    },
}

/// A character named on the command line: `Name` or `main/Name` for
/// the main campaign, `user/Name` for a custom-game (mod) character.
struct CharacterArg {
    realm: Realm,
    name: String,
}

impl CharacterArg {
    fn parse(raw: &str) -> Self {
        raw.split_once('/')
            .and_then(|(dir, name)| Realm::parse_dir_name(dir).map(|realm| (realm, name)))
            .map_or_else(
                || Self {
                    realm: Realm::Main,
                    name: raw.to_owned(),
                },
                |(realm, name)| Self {
                    realm,
                    name: name.to_owned(),
                },
            )
    }

    fn path(&self, save_dir: &Path) -> PathBuf {
        save_dir
            .join(self.realm.dir_name())
            .join(format!("_{}", self.name))
            .join("player.gdc")
    }
}

impl fmt::Display for CharacterArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.realm.dir_name(), self.name)
    }
}

struct Invocation {
    game_dir: PathBuf,
    save_dir: PathBuf,
    store_path: PathBuf,
    campaign: Campaign,
    command: Command,
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per command, dispatched top to bottom; splitting would hide the write order"
)]
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Invocation {
        game_dir,
        save_dir,
        store_path,
        campaign,
        command,
    } = parse_args(&args)?;
    let game_data = load_game_data(&game_dir)?;
    if let Command::ImportGds { file } = &command {
        return run_import_gds(file, &store_path, &game_data);
    }
    let shared_dir = campaign.shared_dir(&save_dir);
    let stash_path = shared_dir.join("transfer.gst");
    let reagents_path = shared_dir.join("reagents.gst");
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
                &campaign,
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
                &campaign,
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
        Command::Socket(command) => {
            let ctx = SocketCtx {
                save_dir: &save_dir,
                store_path: &store_path,
                stash_path: &stash_path,
                campaign: &campaign,
                game_data: &game_data,
            };
            run_socket(&command, &ctx, &mut stash, &mut store)?;
        }
        Command::ImportGds { .. } => unreachable!("peeled off before the shared files are opened"),
    }
    Ok(())
}

/// What a socket command works with besides the item's own file.
struct SocketCtx<'a> {
    save_dir: &'a Path,
    store_path: &'a Path,
    stash_path: &'a Path,
    campaign: &'a Campaign,
    game_data: &'a GameData,
}

/// What a socket edit did to the store, which decides the write order:
/// the part's destination goes first.
enum SocketOutcome {
    /// The store gave a part up: the item's file is the destination.
    Attached,
    /// The store received the freed part: it is the destination.
    Detached,
}

/// Fills or frees a socket of the item at the command's location,
/// writing the item's file and the store destination-first, then
/// re-reading the file and printing the item as written.
fn run_socket(
    command: &SocketCommand,
    ctx: &SocketCtx<'_>,
    stash: &mut Loaded<GstFile>,
    store: &mut VaultStore,
) -> Result<(), Box<dyn Error>> {
    let now = now()?;
    match command.location() {
        Location::Stash { tab, index } => {
            let (tab, index) = (*tab, *index);
            let origin = ItemOrigin::TransferStash {
                campaign: ctx.campaign.clone(),
                tab,
            };
            let item =
                stash_item_mut(&mut transfer_stash_mut(stash.model_mut())?.tabs, tab, index)?;
            let outcome = edit_socket(item, command, origin, ctx.game_data, store, now)?;
            match outcome {
                SocketOutcome::Attached => {
                    write_stash(ctx.stash_path, stash)?;
                    write_store(ctx.store_path, store)?;
                }
                SocketOutcome::Detached => {
                    write_store(ctx.store_path, store)?;
                    write_stash(ctx.stash_path, stash)?;
                }
            }
            let written = reparse(ctx.stash_path)?;
            let mut tabs = transfer_stash(&written)?.tabs.clone();
            print_sockets(ctx.game_data, stash_item_mut(&mut tabs, tab, index)?);
        }
        Location::Sack {
            character,
            sack,
            index,
        } => {
            let path = character.path(ctx.save_dir);
            let mut player = load_player(&path)?;
            let origin = ItemOrigin::Character {
                realm: character.realm,
                name: player.model().character_name().to_owned(),
                sack: *sack,
            };
            let item = sack_item_mut(player.model_mut(), *sack, *index)?;
            let outcome = edit_socket(item, command, origin, ctx.game_data, store, now)?;
            write_socket_edit(&outcome, ctx, &path, &player, store)?;
            let mut written = reparse_player(&path)?;
            print_sockets(ctx.game_data, sack_item_mut(&mut written, *sack, *index)?);
        }
        Location::OwnStash {
            character,
            tab,
            index,
        } => {
            let path = character.path(ctx.save_dir);
            let mut player = load_player(&path)?;
            let origin = ItemOrigin::CharacterStash {
                realm: character.realm,
                name: player.model().character_name().to_owned(),
                tab: *tab,
            };
            let item = own_stash_item_mut(player.model_mut(), *tab, *index)?;
            let outcome = edit_socket(item, command, origin, ctx.game_data, store, now)?;
            write_socket_edit(&outcome, ctx, &path, &player, store)?;
            let mut written = reparse_player(&path)?;
            print_sockets(
                ctx.game_data,
                own_stash_item_mut(&mut written, *tab, *index)?,
            );
        }
    }
    Ok(())
}

/// The character's file and the store, the part's destination first.
fn write_socket_edit(
    outcome: &SocketOutcome,
    ctx: &SocketCtx<'_>,
    path: &Path,
    player: &Loaded<PlayerFile>,
    store: &VaultStore,
) -> Result<(), Box<dyn Error>> {
    match outcome {
        SocketOutcome::Attached => {
            write_player(path, player)?;
            write_store(ctx.store_path, store)
        }
        SocketOutcome::Detached => {
            write_store(ctx.store_path, store)?;
            write_player(path, player)
        }
    }
}

/// The edit itself, on the item in place: every check runs before
/// anything changes, so a refusal leaves the item and the store as
/// they were.
fn edit_socket(
    item: &mut Item,
    command: &SocketCommand,
    origin: ItemOrigin,
    game_data: &GameData,
    store: &mut VaultStore,
    now: Timestamp,
) -> Result<SocketOutcome, Box<dyn Error>> {
    match command {
        SocketCommand::Detach { socket, .. } => {
            let freed = socket::detach(item, *socket, clock_seed(now))?;
            *item = freed.host;
            let name = describe(game_data, &freed.part);
            let stored = store.add(freed.part, origin, now);
            println!("freed the {socket} {name} into the store as stored item {stored}");
            Ok(SocketOutcome::Detached)
        }
        SocketCommand::Attach { id, seed, .. } => {
            let slot = socket::host_slot(game_data, item)?;
            let offered = store
                .get(*id)
                .ok_or_else(|| format!("the store has no item {id}"))?
                .item()
                .clone();
            let record = RecordId::parse(offered.base_name.clone())
                .ok_or_else(|| format!("stored item {id} names no record"))?;
            let part = Part::read(game_data, &record)?;
            let seed = seed.unwrap_or_else(|| clock_seed(now));
            let edited = socket::attach(item, slot, &part, seed)?;
            if offered.stack_count > 1 {
                if let Some(stack) = store.item_mut(*id) {
                    stack.stack_count = offered.stack_count - 1;
                }
            } else {
                store.take(*id);
            }
            *item = edited;
            println!(
                "put {} in as the {} of the item (a {slot}) under seed {seed:#x}",
                describe(game_data, &offered),
                part.socket()
            );
            Ok(SocketOutcome::Attached)
        }
    }
}

/// The seed a freed part or a socket takes when none is given: the
/// clock, which is as arbitrary as the game's own.
fn clock_seed(now: Timestamp) -> u32 {
    u32::try_from(now.unix_seconds() & u64::from(u32::MAX)).unwrap_or(0)
}

fn print_sockets(game_data: &GameData, item: &Item) {
    println!("item as written: {}", describe(game_data, item));
    for socket in Socket::ALL {
        let record = socket.record_of(item);
        if record.is_empty() {
            println!("  {}: none", socket.title());
        } else {
            let part = Item {
                base_name: record.to_string(),
                ..Item::default()
            };
            println!("  {}: {}", socket.title(), describe(game_data, &part));
        }
    }
    println!(
        "  relic seed {:#x}, completion level {}, bonus {:?}; augment seed {:#x}, level {}",
        item.relic_seed,
        item.relic_completion_level,
        item.relic_bonus,
        item.augment_seed,
        item.unknown
    );
}

fn stash_item_mut(
    tabs: &mut [grimvault_core::block::StashTab],
    tab: TabIndex,
    index: ItemIndex,
) -> Result<&mut Item, Box<dyn Error>> {
    tabs.get_mut(usize::try_from(tab.value())?)
        .ok_or_else(|| format!("no tab {tab}"))?
        .items
        .get_mut(index.value())
        .map(|placed| &mut placed.item)
        .ok_or_else(|| format!("tab {tab} has no item {index}").into())
}

fn sack_item_mut(
    player: &mut PlayerFile,
    sack: SackIndex,
    index: ItemIndex,
) -> Result<&mut Item, Box<dyn Error>> {
    player
        .inventory_mut()
        .ok_or("player.gdc carries no typed inventory (block 3)")?
        .sacks_mut()
        .get_mut(usize::try_from(sack.value())?)
        .ok_or_else(|| format!("no sack {sack}"))?
        .items
        .get_mut(index.value())
        .map(|placed| &mut placed.item)
        .ok_or_else(|| format!("sack {sack} has no item {index}").into())
}

fn own_stash_item_mut(
    player: &mut PlayerFile,
    tab: TabIndex,
    index: ItemIndex,
) -> Result<&mut Item, Box<dyn Error>> {
    let stash = player
        .stash_mut()
        .ok_or("player.gdc carries no typed stash (block 4)")?;
    stash_item_mut(&mut stash.tabs, tab, index)
}

/// Imports a GD Stash export into the store, backup-first as every
/// store write is; no game file is opened.
fn run_import_gds(
    file: &Path,
    store_path: &Path,
    game_data: &GameData,
) -> Result<(), Box<dyn Error>> {
    let mut store = load_store(store_path)?;
    let export = gds::parse(&read_verified(file)?)?;
    println!(
        "parsed {} ({}, {} entries)",
        file.display(),
        export.version(),
        export.len()
    );
    let report = gds::import(&mut store, &export, file, game_data, now()?);
    println!("import: {report}");
    for (record, entries) in &report.unknown_records {
        println!("  unknown record {record} ({entries} entries, imported anyway)");
    }
    if report.added.is_empty() {
        println!("nothing new; {} left untouched", store_path.display());
    } else {
        write_store(store_path, &store)?;
    }
    print_bucket_counts(game_data, &store);
    Ok(())
}

fn print_bucket_counts(game_data: &GameData, store: &VaultStore) {
    println!("\nstore: {} items", store.len());
    let mut counts: BTreeMap<Bucket, usize> = BTreeMap::new();
    for stored in store.items() {
        *counts
            .entry(bucket_of(game_data, stored.item()))
            .or_default() += 1;
    }
    for group in Group::ALL {
        let buckets: Vec<(Bucket, usize)> = Bucket::ALL
            .into_iter()
            .filter(|bucket| bucket.group() == group)
            .filter_map(|bucket| counts.get(&bucket).map(|count| (bucket, *count)))
            .collect();
        if buckets.is_empty() {
            continue;
        }
        println!(
            "  {} ({})",
            group.label(),
            buckets.iter().map(|(_, count)| count).sum::<usize>()
        );
        for (bucket, count) in buckets {
            println!("    {} {count}", bucket.label());
        }
    }
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
            for (realm, path) in character_paths(save_dir)? {
                let player = load_player(&path)?;
                print_character(game_data, realm, player.model());
            }
        }
        CharacterCommand::VaultSack {
            character,
            sack,
            index,
        } => {
            let path = character.path(save_dir);
            let mut player = load_player(&path)?;
            let id = transfer::vault_from_sack(
                player.model_mut(),
                character.realm,
                sack,
                index,
                store,
                now()?,
            )?;
            println!("vaulted {character}'s sack {sack} item {index} as stored item {id}");
            write_store(store_path, store)?;
            write_player(&path, &player)?;
            print_character(game_data, character.realm, &reparse_player(&path)?);
            print_store(game_data, store);
        }
        CharacterCommand::PlaceSack {
            id,
            character,
            sack,
            pos,
        } => {
            let path = character.path(save_dir);
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
            print_character(game_data, character.realm, &reparse_player(&path)?);
            print_store(game_data, store);
        }
        CharacterCommand::Money { character, amount } => {
            let path = character.path(save_dir);
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
        CharacterCommand::Respec { character, reset } => {
            let path = character.path(save_dir);
            let mut player = load_player(&path)?;
            let rules = RespecRules::load(game_data)?;
            print_progress(character.realm, player.model());
            let report = reset.apply(player.model_mut(), &rules)?;
            println!("reset {reset} on {character}: {report}");
            if report.is_noop() {
                println!("nothing to write");
            } else {
                write_player(&path, &player)?;
                print_progress(character.realm, &reparse_player(&path)?);
            }
        }
    }
    Ok(())
}

/// The level, the pools a respec touches, and the class.
fn print_progress(realm: Realm, player: &PlayerFile) {
    let header = player.header();
    let class = if header.class_tag.is_empty() {
        "no class"
    } else {
        header.class_tag.as_str()
    };
    println!(
        "\ncharacter {} [{realm}] (level {}): {class}",
        header.name, header.level
    );
    if let Some(bio) = player.bio() {
        println!(
            "  unspent: {} attribute points, {} skill points; physique {} cunning {} spirit {}; \
             health {} energy {}",
            bio.attribute_points_unspent,
            bio.skill_points_unspent,
            bio.physique,
            bio.cunning,
            bio.spirit,
            bio.health,
            bio.energy
        );
    }
    if let Some(skills) = player.skills() {
        let mastery_skills = skills
            .skills
            .iter()
            .filter(|skill| skill.name.starts_with("records/skills/playerclass"))
            .count();
        println!(
            "  skills: {} in block 8, {mastery_skills} under records/skills/playerclass*; \
             masteries allowed {}",
            skills.skills.len(),
            skills.masteries_allowed
        );
    }
}

/// Every character's file, `main/` then `user/`, each in folder
/// order; an absent `user/` contributes nothing.
fn character_paths(save_dir: &Path) -> Result<Vec<(Realm, PathBuf)>, Box<dyn Error>> {
    let mut found = Vec::new();
    for realm in Realm::ALL {
        let dir = save_dir.join(realm.dir_name());
        if !dir.is_dir() {
            continue;
        }
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .flatten()
            .map(|entry| entry.path().join("player.gdc"))
            .filter(|path| path.is_file())
            .collect();
        paths.sort();
        found.extend(paths.into_iter().map(|path| (realm, path)));
    }
    Ok(found)
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

fn print_character(game_data: &GameData, realm: Realm, player: &PlayerFile) {
    let header = player.header();
    println!(
        "\ncharacter {} [{realm}] (level {}): {} iron bits",
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
                character: CharacterArg::parse(character),
                sack: SackIndex::new(sack.parse()?),
                index: ItemIndex::new(index.parse()?),
            })
        }
        ("place-sack", [id, character, sack]) => Command::Character(CharacterCommand::PlaceSack {
            id: StoredItemId::new(id.parse()?),
            character: CharacterArg::parse(character),
            sack: SackIndex::new(sack.parse()?),
            pos: None,
        }),
        ("place-sack", [id, character, sack, x, y]) => {
            Command::Character(CharacterCommand::PlaceSack {
                id: StoredItemId::new(id.parse()?),
                character: CharacterArg::parse(character),
                sack: SackIndex::new(sack.parse()?),
                pos: Some(GridPos {
                    x: x.parse()?,
                    y: y.parse()?,
                }),
            })
        }
        ("money", [character]) => Command::Character(CharacterCommand::Money {
            character: CharacterArg::parse(character),
            amount: None,
        }),
        ("money", [character, amount]) => Command::Character(CharacterCommand::Money {
            character: CharacterArg::parse(character),
            amount: Some(amount.parse()?),
        }),
        ("import-gds", [file]) => Command::ImportGds {
            file: PathBuf::from(file),
        },
        ("respec-attributes", [character]) => Command::Character(CharacterCommand::Respec {
            character: CharacterArg::parse(character),
            reset: Reset::Attributes,
        }),
        ("respec-masteries", [character]) => Command::Character(CharacterCommand::Respec {
            character: CharacterArg::parse(character),
            reset: Reset::Masteries,
        }),
        ("detach", [at, socket]) => Command::Socket(SocketCommand::Detach {
            at: Location::parse(at)?,
            socket: parse_socket(socket)?,
        }),
        ("attach", [at, id]) => Command::Socket(SocketCommand::Attach {
            at: Location::parse(at)?,
            id: StoredItemId::new(id.parse()?),
            seed: None,
        }),
        ("attach", [at, id, seed]) => Command::Socket(SocketCommand::Attach {
            at: Location::parse(at)?,
            id: StoredItemId::new(id.parse()?),
            seed: Some(parse_seed(seed)?),
        }),
        _ => return Err(USAGE.into()),
    };
    Ok(Invocation {
        game_dir: paths.game_dir,
        save_dir: paths.save_dir,
        store_path: paths.store_path,
        campaign: paths.campaign,
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
                    stored.origin(),
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
