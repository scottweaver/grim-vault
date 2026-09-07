//! The socket rules against the real record database and the real
//! items: every part record the game ships, and every socketed or
//! augmented item in the transfer stashes and character files under
//! `$GRIMVAULT_SAVE_DIR` (a **copy** of a save directory, never the
//! live one) — its part must admit its host's slot, and freeing the
//! part then putting it back under the original seed must reproduce
//! the item exactly. Passes vacuously when `$GRIMVAULT_GAME_DIR` is
//! unset or does not name the install.

#[path = "../examples/support/mod.rs"]
#[allow(
    dead_code,
    reason = "shared with the examples, which use the rest of it"
)]
mod support;

use std::path::PathBuf;

use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::{PlayerFile, Realm};
use grimvault_core::gst::GstFile;
use grimvault_core::item::Item;
use grimvault_core::socket::{
    COMPLETE_COMPONENT_LEVEL, Part, Slot, Socket, attach, detach, host_slot,
};
use univault_engine::ids::RecordId;

fn game_data() -> Option<GameData> {
    let game_dir = std::env::var_os("GRIMVAULT_GAME_DIR").map(PathBuf::from)?;
    if !game_dir.join("database/database.arz").is_file() {
        return None;
    }
    Some(support::load_game_data(&game_dir).expect("the install's archives parse"))
}

fn save_items() -> Vec<(String, Item)> {
    let Some(save_dir) = std::env::var_os("GRIMVAULT_SAVE_DIR").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for realm in Realm::ALL {
        let Ok(folders) = std::fs::read_dir(save_dir.join(realm.dir_name())) else {
            continue;
        };
        for folder in folders.flatten() {
            let path = folder.path().join("player.gdc");
            if !path.is_file() {
                continue;
            }
            let player = PlayerFile::parse(&std::fs::read(&path).unwrap()).unwrap();
            let label = format!("{}/{}", realm.dir_name(), player.character_name());
            if let Some(inventory) = player.inventory() {
                for sack in inventory.sacks() {
                    items.extend(
                        sack.items
                            .iter()
                            .map(|placed| (label.clone(), placed.item.clone())),
                    );
                }
                items.extend(
                    inventory
                        .equipped()
                        .map(|slot| (label.clone(), slot.item.clone())),
                );
            }
            if let Some(stash) = player.stash() {
                for tab in &stash.tabs {
                    items.extend(
                        tab.items
                            .iter()
                            .map(|placed| (label.clone(), placed.item.clone())),
                    );
                }
            }
        }
    }
    let mut stashes = vec![save_dir.join("transfer.gst")];
    if let Ok(folders) = std::fs::read_dir(&save_dir) {
        stashes.extend(
            folders
                .flatten()
                .map(|entry| entry.path().join("transfer.gst"))
                .filter(|path| path.is_file()),
        );
    }
    for path in stashes {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let file = GstFile::parse(&bytes).unwrap();
        if let Some(stash) = file.transfer_stash() {
            for tab in &stash.tabs {
                items.extend(
                    tab.items
                        .iter()
                        .map(|placed| (path.display().to_string(), placed.item.clone())),
                );
            }
        }
    }
    items
}

#[test]
fn every_shipped_part_reads_and_components_are_single_piece() {
    let Some(game) = game_data() else {
        return;
    };
    let mut components = 0;
    let mut augments = 0;
    for (class, socket) in [
        ("ItemRelic", Socket::Component),
        ("ItemEnchantment", Socket::Augment),
    ] {
        for id in game.record_ids_of_type(class) {
            let part = Part::read(&game, id).unwrap_or_else(|error| panic!("{id:?}: {error}"));
            assert_eq!(part.socket(), socket, "{id:?}");
            let record = game.record(id).unwrap().unwrap();
            match socket {
                Socket::Component => {
                    components += 1;
                    assert_eq!(
                        record.integer("completedRelicLevel"),
                        Some(1),
                        "{id:?}: components are single-piece"
                    );
                    assert!(
                        !part.slots().is_empty(),
                        "{id:?}: a component fits somewhere"
                    );
                }
                Socket::Augment => augments += 1,
            }
            assert!(
                !part.fits(Slot::Staff),
                "{id:?}: no shipped part flags the staff slot"
            );
        }
    }
    assert!(components >= 107, "{components} components");
    assert!(augments >= 386, "{augments} augments");
}

#[test]
fn every_socketed_item_in_the_saves_obeys_the_rules_and_round_trips() {
    let Some(game) = game_data() else {
        return;
    };
    let items = save_items();
    if items.is_empty() {
        return;
    }
    let mut socketed = 0;
    let mut augmented = 0;
    for (place, item) in &items {
        if item.is_empty() {
            continue;
        }
        let base = RecordId::parse(item.base_name.clone()).unwrap();
        let class = game
            .record(&base)
            .and_then(Result::ok)
            .and_then(|record| record.string("Class").map(str::to_string));
        if class.as_deref() == Some("ItemRelic") {
            assert_eq!(
                item.relic_completion_level, COMPLETE_COMPONENT_LEVEL,
                "{place}: loose component {}",
                item.base_name
            );
        }
        for socket in Socket::ALL {
            let record = socket.record_of(item);
            if record.is_empty() {
                continue;
            }
            let slot = host_slot(&game, item)
                .unwrap_or_else(|error| panic!("{place}: {} — {error}", item.base_name));
            let part = Part::read(&game, &RecordId::parse(record.to_string()).unwrap())
                .unwrap_or_else(|error| panic!("{place}: {record} — {error}"));
            assert_eq!(part.socket(), socket, "{place}: {record}");
            assert!(
                part.fits(slot),
                "{place}: {record} on {} ({slot}); flags {:?}",
                item.base_name,
                part.slots()
            );
            let seed = match socket {
                Socket::Component => {
                    socketed += 1;
                    assert_eq!(
                        item.relic_completion_level, COMPLETE_COMPONENT_LEVEL,
                        "{place}"
                    );
                    assert!(item.relic_bonus.is_empty(), "{place}: {}", item.relic_bonus);
                    item.relic_seed
                }
                Socket::Augment => {
                    augmented += 1;
                    assert_eq!(item.unknown, 0, "{place}");
                    item.augment_seed
                }
            };
            let freed = detach(item, socket, 1).unwrap();
            assert!(socket.record_of(&freed.host).is_empty());
            assert_eq!(freed.part.base_name, record);
            let back = attach(&freed.host, slot, &part, seed).unwrap();
            assert_eq!(&back, item, "{place}: {record} round trip");
        }
    }
    assert!(socketed > 0, "no socketed item in the saves");
    assert!(augmented > 0, "no augmented item in the saves");
    eprintln!(
        "socketed {socketed}, augmented {augmented}, of {} items",
        items.len()
    );
}
