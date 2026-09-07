//! Socket edits as the shell performs them: the item the inspector
//! shows gains a component or augment out of the vault store or the
//! component storage, or gives one up to the store. The item itself
//! changes through `grimvault_core::socket`; its container and the
//! part's are the same ends a move has, reached the same way, and
//! every check runs before anything is edited, so a refusal leaves
//! every document as it was.

use grimvault_core::gamedata::GameData;
use grimvault_core::socket::{self, Part, Socket, SocketError};
use grimvault_core::store::{StoredItemId, Timestamp, VaultStore};
use grimvault_core::transfer::{self, ReagentIndex, TransferError};
use thiserror::Error;
use univault_engine::ids::RecordId;

use crate::documents::Doc;
use crate::drag::{self, ApplyError, Containers, DragSource, Ends};

/// Where a part to attach comes from: an entry of the vault store — a
/// stack of one is taken, a larger stack gives one up — or a row of
/// the component storage, which gives one up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartSource {
    Store(StoredItemId),
    Reagent(ReagentIndex),
}

impl PartSource {
    /// The document the part leaves.
    #[must_use]
    pub fn doc(self) -> Doc {
        match self {
            Self::Store(_) => Doc::Store,
            Self::Reagent(_) => Doc::Reagents,
        }
    }
}

/// What the inspector asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Put the part at `part` into `host`'s matching socket.
    Attach { host: DragSource, part: PartSource },
    /// Free the part in `host`'s `socket` into the vault store.
    Detach { host: DragSource, socket: Socket },
}

/// What an edit did, for the shell to mark dirty and order writes by:
/// the host's document always changed; `from` gave the part up, or
/// the store received it as `stored`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    Attached {
        socket: Socket,
        part: RecordId,
        host: Doc,
        from: Doc,
    },
    Detached {
        socket: Socket,
        part: RecordId,
        host: Doc,
        stored: StoredItemId,
    },
}

/// Why a socket edit was refused; nothing changed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SocketApplyError {
    #[error(transparent)]
    Socket(#[from] SocketError),
    #[error(transparent)]
    Containers(#[from] ApplyError),
}

impl From<TransferError> for SocketApplyError {
    fn from(error: TransferError) -> Self {
        Self::Containers(ApplyError::Transfer(error))
    }
}

/// Performs `request`; `seed` is what the socket, or the freed part,
/// takes as its own.
///
/// # Errors
/// [`SocketApplyError`]; every container is exactly as it was.
pub fn apply(
    request: Request,
    containers: &mut Containers<'_>,
    game: &GameData,
    seed: u32,
    now: Timestamp,
) -> Result<Applied, SocketApplyError> {
    match request {
        Request::Detach { host, socket } => detach_to_store(host, socket, containers, seed, now),
        Request::Attach { host, part } => attach_from(host, part, containers, game, seed, now),
    }
}

fn detach_to_store(
    host: DragSource,
    socket: Socket,
    containers: &mut Containers<'_>,
    seed: u32,
    now: Timestamp,
) -> Result<Applied, SocketApplyError> {
    let (item, origin) = drag::peek(containers, host)?;
    let freed = socket::detach(&item, socket, seed)?;
    let part = record_of(&freed.part.base_name)?;
    *drag::item_mut(containers, host)? = freed.host;
    let stored = containers.store.add(freed.part, origin, now);
    Ok(Applied::Detached {
        socket,
        part,
        host: host.doc(),
        stored,
    })
}

fn attach_from(
    host: DragSource,
    source: PartSource,
    containers: &mut Containers<'_>,
    game: &GameData,
    seed: u32,
    now: Timestamp,
) -> Result<Applied, SocketApplyError> {
    let (item, _) = drag::peek(containers, host)?;
    let slot = socket::host_slot(game, &item)?;
    let record = offered_record(containers, source)?;
    let part = Part::read(game, &record)?;
    let edited = socket::attach(&item, slot, &part, seed)?;
    let host_before = drag::Snapshot::take(containers, host.doc())?;
    let part_before = drag::Snapshot::take(containers, source.doc())?;
    match take_one(containers, source, now).and_then(|()| place_host(containers, host, edited)) {
        Ok(()) => Ok(Applied::Attached {
            socket: part.socket(),
            part: record,
            host: host.doc(),
            from: source.doc(),
        }),
        Err(error) => {
            part_before.restore(containers);
            host_before.restore(containers);
            Err(error)
        }
    }
}

fn place_host(
    containers: &mut Containers<'_>,
    host: DragSource,
    edited: grimvault_core::item::Item,
) -> Result<(), SocketApplyError> {
    *drag::item_mut(containers, host)? = edited;
    Ok(())
}

/// The record an offered part is, without taking it.
fn offered_record(
    containers: &Containers<'_>,
    source: PartSource,
) -> Result<RecordId, SocketApplyError> {
    match source {
        PartSource::Store(id) => {
            let stored = containers.store.get(id).ok_or(ApplyError::SourceGone)?;
            record_of(&stored.item().base_name)
        }
        PartSource::Reagent(index) => {
            let entry = containers
                .storage()
                .ok_or(ApplyError::NoReagentStorage)?
                .entries
                .get(index.value())
                .ok_or(TransferError::NoSuchReagent(index))?;
            record_of(&entry.record)
        }
    }
}

fn record_of(path: &str) -> Result<RecordId, SocketApplyError> {
    RecordId::parse(path.to_string()).ok_or_else(|| {
        SocketError::UnknownRecord {
            record: path.to_string(),
        }
        .into()
    })
}

/// Takes one part out of `source`: a store stack of one goes, a
/// larger one shrinks, a storage row gives one up.
fn take_one(
    containers: &mut Containers<'_>,
    source: PartSource,
    now: Timestamp,
) -> Result<(), SocketApplyError> {
    match source {
        PartSource::Store(id) => {
            let held = containers
                .store
                .get(id)
                .ok_or(ApplyError::SourceGone)?
                .item()
                .stack_count;
            if held > 1 {
                if let Some(item) = containers.store.item_mut(id) {
                    item.stack_count = held - 1;
                }
            } else {
                containers.store.take(id);
            }
        }
        PartSource::Reagent(index) => {
            let campaign = containers.campaign;
            let mut scratch = VaultStore::new();
            transfer::vault_from_reagents(
                containers.reagents()?,
                campaign,
                index,
                1,
                &mut scratch,
                now,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use grimvault_core::block::{StashTab, TabDecoration};
    use grimvault_core::campaign::Campaign;
    use grimvault_core::gst::{ReagentEntry, ReagentStorage, ReagentStorageVersion, TransferStash};
    use grimvault_core::item::{ContainerVersion, Item, StashItem};
    use grimvault_core::socket::Slot;
    use grimvault_core::store::ItemOrigin;
    use grimvault_core::transfer::{ItemIndex, TabIndex};
    use univault_engine::arz::fixture::{ArzBuilder, Values};
    use univault_engine::arz::{ArzDialect, ArzFile};
    use univault_engine::text::TextDb;

    use super::*;
    use crate::drag::Container;

    const COMPONENT: &str = "records/items/materia/compa_sample.dbr";
    const AUGMENT: &str = "records/items/enchants/a00a_enchant.dbr";
    const HELM: &str = "records/items/gearhead/a00_head.dbr";
    const RING: &str = "records/items/gearaccessories/rings/a00_ring.dbr";
    const NOW: Timestamp = Timestamp::from_unix_seconds(1_756_900_000);
    const SEED: u32 = 0x61ba_ef85;

    fn game() -> GameData {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(
            COMPONENT,
            "ItemRelic",
            &[
                ("Class", Values::Strings(&["ItemRelic"])),
                ("head", Values::Bools(&[true])),
            ],
        );
        builder.record(
            AUGMENT,
            "ItemEnchantment",
            &[
                ("Class", Values::Strings(&["ItemEnchantment"])),
                ("ring", Values::Bools(&[true])),
            ],
        );
        for (id, class) in [(HELM, "ArmorProtective_Head"), (RING, "ArmorJewelry_Ring")] {
            builder.record(id, class, &[("Class", Values::Strings(&[class]))]);
        }
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        GameData::from_parts(vec![database], TextDb::new(), Vec::new())
    }

    fn item(base: &str, stack: u32) -> Item {
        Item {
            base_name: base.into(),
            seed: 7,
            stack_count: stack,
            ..Item::default()
        }
    }

    struct World {
        stash: TransferStash,
        store: VaultStore,
        reagents: ReagentStorage,
    }

    impl World {
        fn new() -> Self {
            let mut store = VaultStore::new();
            store.add(item(COMPONENT, 2), ItemOrigin::Unknown, NOW);
            store.add(item(AUGMENT, 1), ItemOrigin::Unknown, NOW);
            Self {
                stash: TransferStash {
                    version: ContainerVersion::new(11).unwrap(),
                    mod_name: String::new(),
                    expansion_status: 7,
                    tabs: vec![StashTab {
                        width: 10,
                        height: 19,
                        items: vec![
                            StashItem {
                                item: item(HELM, 1),
                                x: 0.0,
                                y: 0.0,
                            },
                            StashItem {
                                item: item(RING, 1),
                                x: 4.0,
                                y: 0.0,
                            },
                        ],
                        decoration: TabDecoration::default(),
                    }],
                },
                store,
                reagents: ReagentStorage {
                    version: ReagentStorageVersion::new(1).unwrap(),
                    mod_name: String::new(),
                    entries: vec![ReagentEntry {
                        record: COMPONENT.into(),
                        count: 3,
                    }],
                },
            }
        }

        fn apply(&mut self, request: Request) -> Result<Applied, SocketApplyError> {
            let mut containers = Containers {
                campaign: &Campaign::Main,
                stash: &mut self.stash,
                store: &mut self.store,
                reagents: Some(&mut self.reagents),
                characters: Vec::new(),
            };
            super::apply(request, &mut containers, &game(), SEED, NOW)
        }

        fn stash_item(&self, index: usize) -> &Item {
            &self.stash.tabs[0].items[index].item
        }

        fn snapshot(&self) -> (TransferStash, VaultStore, ReagentStorage) {
            (
                self.stash.clone(),
                self.store.clone(),
                self.reagents.clone(),
            )
        }
    }

    const TAB0: TabIndex = TabIndex::new(0);

    fn in_stash(index: usize) -> DragSource {
        DragSource::Grid {
            container: Container::TransferStash(TAB0),
            index: ItemIndex::new(index),
        }
    }

    fn component_id() -> StoredItemId {
        StoredItemId::new(1)
    }

    fn augment_id() -> StoredItemId {
        StoredItemId::new(2)
    }

    #[test]
    fn attaching_from_the_store_shrinks_the_stack_then_takes_the_last_one() {
        let mut world = World::new();
        let applied = world
            .apply(Request::Attach {
                host: in_stash(0),
                part: PartSource::Store(component_id()),
            })
            .unwrap();
        assert_eq!(
            applied,
            Applied::Attached {
                socket: Socket::Component,
                part: RecordId::parse(COMPONENT.into()).unwrap(),
                host: Doc::Stash,
                from: Doc::Store,
            }
        );
        assert_eq!(world.stash_item(0).relic_name, COMPONENT);
        assert_eq!(world.stash_item(0).relic_seed, SEED);
        assert_eq!(
            world.store.get(component_id()).unwrap().item().stack_count,
            1
        );

        assert_eq!(
            world.apply(Request::Attach {
                host: in_stash(0),
                part: PartSource::Store(component_id()),
            }),
            Err(SocketError::Occupied(Socket::Component).into())
        );
        assert_eq!(
            world.store.get(component_id()).unwrap().item().stack_count,
            1
        );

        world
            .apply(Request::Detach {
                host: in_stash(0),
                socket: Socket::Component,
            })
            .unwrap();
        world
            .apply(Request::Attach {
                host: in_stash(0),
                part: PartSource::Store(component_id()),
            })
            .unwrap();
        assert!(world.store.get(component_id()).is_none());
        assert_eq!(world.stash_item(0).relic_name, COMPONENT);
    }

    #[test]
    fn detaching_frees_the_part_into_the_store_under_the_hosts_origin() {
        let mut world = World::new();
        world
            .apply(Request::Attach {
                host: in_stash(1),
                part: PartSource::Store(augment_id()),
            })
            .unwrap();
        assert!(world.store.get(augment_id()).is_none());
        assert_eq!(world.stash_item(1).augment_name, AUGMENT);

        let applied = world
            .apply(Request::Detach {
                host: in_stash(1),
                socket: Socket::Augment,
            })
            .unwrap();
        let Applied::Detached {
            socket,
            part,
            host,
            stored,
        } = applied
        else {
            panic!("expected a detach");
        };
        assert_eq!(socket, Socket::Augment);
        assert_eq!(part.as_str(), AUGMENT);
        assert_eq!(host, Doc::Stash);
        assert!(world.stash_item(1).augment_name.is_empty());
        let freed = world.store.get(stored).unwrap();
        assert_eq!(freed.item().base_name, AUGMENT);
        assert_eq!(freed.item().seed, SEED);
        assert_eq!(freed.item().stack_count, 1);
        assert_eq!(
            freed.origin(),
            &ItemOrigin::TransferStash {
                campaign: Campaign::Main,
                tab: TAB0,
            }
        );
        assert_eq!(
            world.apply(Request::Detach {
                host: in_stash(1),
                socket: Socket::Augment,
            }),
            Err(SocketError::Empty(Socket::Augment).into())
        );
    }

    #[test]
    fn attaching_from_the_storage_gives_one_up() {
        let mut world = World::new();
        let applied = world
            .apply(Request::Attach {
                host: in_stash(0),
                part: PartSource::Reagent(ReagentIndex::new(0)),
            })
            .unwrap();
        assert!(matches!(
            applied,
            Applied::Attached {
                from: Doc::Reagents,
                ..
            }
        ));
        assert_eq!(world.reagents.entries[0].count, 2);
        assert_eq!(world.stash_item(0).relic_name, COMPONENT);
    }

    #[test]
    fn a_refused_part_leaves_every_container_untouched() {
        let mut world = World::new();
        let before = world.snapshot();
        assert_eq!(
            world.apply(Request::Attach {
                host: in_stash(1),
                part: PartSource::Store(component_id()),
            }),
            Err(SocketError::NotAllowed {
                socket: Socket::Component,
                slot: Slot::Ring,
            }
            .into())
        );
        assert_eq!(
            world.apply(Request::Attach {
                host: in_stash(0),
                part: PartSource::Store(StoredItemId::new(9)),
            }),
            Err(ApplyError::SourceGone.into())
        );
        assert_eq!(
            world.apply(Request::Attach {
                host: DragSource::Store(component_id()),
                part: PartSource::Store(augment_id()),
            }),
            Err(SocketError::NoSockets {
                record: COMPONENT.into()
            }
            .into())
        );
        assert_eq!(world.snapshot(), before);
    }
}
