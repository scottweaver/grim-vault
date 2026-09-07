//! Sockets: the component (`ItemRelic`) and augment (`ItemEnchantment`)
//! an item can carry, and the pure edits that put one in or take one
//! out. Which items a part may go on is the part record's own choice:
//! every `ItemRelic` and `ItemEnchantment` record carries one boolean
//! per equipment slot — the `itemrelic.tpl` / `itemenchantment.tpl`
//! variables [`Slot`] names — and a part fits an item whose `Class`
//! maps to a flagged slot.
//!
//! Established on the user's install and saves (2026-09-07,
//! `docs/format-references.md` "Sockets"): every one of the 37
//! socketed items in the saves carries a component whose flags admit
//! the item's class, and every one of the 25 augmented items likewise;
//! components are single-piece and complete at
//! `relic_completion_level` **0** (all 107 records have
//! `completedRelicLevel` 1; every loose and socketed component in the
//! saves reads 0); no socketed item carries a completion bonus
//! (`relic_bonus` is empty on all 37 — the game dropped completion
//! bonuses with Forgotten Gods, and the `bonusTableName` 83 records
//! still name is legacy), so attaching writes none; an augment's level
//! word is 0 throughout.
//!
//! Nothing is destroyed: detaching turns the part into a stand-alone
//! item for the caller to keep, and attaching refuses a filled socket
//! rather than overwriting it. Randomness stays at the edge: the seed a
//! socket or a freed part takes is the caller's.

use std::fmt;

use thiserror::Error;
use univault_engine::arz::DbRecord;
use univault_engine::ids::RecordId;

use crate::gamedata::{GameData, ItemClass};
use crate::item::Item;

/// The value `relic_completion_level` holds on every complete
/// component, loose or socketed, in the current game.
pub const COMPLETE_COMPONENT_LEVEL: u32 = 0;

/// The two sockets every piece of equipment has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Socket {
    Component,
    Augment,
}

impl Socket {
    /// Both sockets, in the game's tooltip order.
    pub const ALL: [Self; 2] = [Self::Component, Self::Augment];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Component => "component",
            Self::Augment => "augment",
        }
    }

    /// The socket a part record fills, from its `Class`; `None` for
    /// anything that is not a part.
    #[must_use]
    pub fn of_class(class: &ItemClass) -> Option<Self> {
        match class.as_str() {
            "ItemRelic" => Some(Self::Component),
            "ItemEnchantment" => Some(Self::Augment),
            _ => None,
        }
    }

    /// The record in the socket, empty when nothing is there.
    #[must_use]
    pub fn record_of(self, item: &Item) -> &str {
        match self {
            Self::Component => &item.relic_name,
            Self::Augment => &item.augment_name,
        }
    }
}

impl fmt::Display for Socket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The equipment slots a part record flags, one boolean each, named
/// as the templates spell them. The `spear` flag (a one-handed spear)
/// is on the templates too, but no shipped class maps to it and no
/// shipped part sets it, so it is not modelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Head,
    Shoulders,
    Chest,
    Hands,
    Legs,
    Feet,
    Waist,
    Amulet,
    Ring,
    Medal,
    Shield,
    Offhand,
    Axe,
    Mace,
    Sword,
    Dagger,
    Scepter,
    Ranged1h,
    Axe2h,
    Mace2h,
    Sword2h,
    Spear2h,
    Staff,
    Ranged2h,
}

impl Slot {
    /// Every slot, armor first, then one-handed and two-handed weapons.
    pub const ALL: [Self; 24] = [
        Self::Head,
        Self::Shoulders,
        Self::Chest,
        Self::Hands,
        Self::Legs,
        Self::Feet,
        Self::Waist,
        Self::Amulet,
        Self::Ring,
        Self::Medal,
        Self::Shield,
        Self::Offhand,
        Self::Axe,
        Self::Mace,
        Self::Sword,
        Self::Dagger,
        Self::Scepter,
        Self::Ranged1h,
        Self::Axe2h,
        Self::Mace2h,
        Self::Sword2h,
        Self::Spear2h,
        Self::Staff,
        Self::Ranged2h,
    ];

    /// The part record's boolean that admits this slot.
    #[must_use]
    pub const fn variable(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Shoulders => "shoulders",
            Self::Chest => "chest",
            Self::Hands => "hands",
            Self::Legs => "legs",
            Self::Feet => "feet",
            Self::Waist => "waist",
            Self::Amulet => "amulet",
            Self::Ring => "ring",
            Self::Medal => "medal",
            Self::Shield => "shield",
            Self::Offhand => "offhand",
            Self::Axe => "axe",
            Self::Mace => "mace",
            Self::Sword => "sword",
            Self::Dagger => "dagger",
            Self::Scepter => "scepter",
            Self::Ranged1h => "ranged1h",
            Self::Axe2h => "axe2h",
            Self::Mace2h => "mace2h",
            Self::Sword2h => "sword2h",
            Self::Spear2h => "spear2h",
            Self::Staff => "staff",
            Self::Ranged2h => "ranged2h",
        }
    }

    /// The slot an item of `class` occupies; `None` for anything that
    /// is not equipment, which has no sockets.
    #[must_use]
    pub fn of_class(class: &ItemClass) -> Option<Self> {
        match class.as_str() {
            "ArmorProtective_Head" => Some(Self::Head),
            "ArmorProtective_Shoulders" => Some(Self::Shoulders),
            "ArmorProtective_Chest" => Some(Self::Chest),
            "ArmorProtective_Hands" => Some(Self::Hands),
            "ArmorProtective_Legs" => Some(Self::Legs),
            "ArmorProtective_Feet" => Some(Self::Feet),
            "ArmorProtective_Waist" => Some(Self::Waist),
            "ArmorJewelry_Amulet" => Some(Self::Amulet),
            "ArmorJewelry_Ring" => Some(Self::Ring),
            "ArmorJewelry_Medal" => Some(Self::Medal),
            "WeaponArmor_Shield" => Some(Self::Shield),
            "WeaponArmor_Offhand" => Some(Self::Offhand),
            "WeaponMelee_Axe" => Some(Self::Axe),
            "WeaponMelee_Mace" => Some(Self::Mace),
            "WeaponMelee_Sword" => Some(Self::Sword),
            "WeaponMelee_Dagger" => Some(Self::Dagger),
            "WeaponMelee_Scepter" => Some(Self::Scepter),
            "WeaponHunting_Ranged1h" => Some(Self::Ranged1h),
            "WeaponMelee_Axe2h" => Some(Self::Axe2h),
            "WeaponMelee_Mace2h" => Some(Self::Mace2h),
            "WeaponMelee_Sword2h" => Some(Self::Sword2h),
            "WeaponMelee_Spear2h" => Some(Self::Spear2h),
            "WeaponMagical_Staff" => Some(Self::Staff),
            "WeaponHunting_Ranged2h" => Some(Self::Ranged2h),
            _ => None,
        }
    }

    /// The slot as a sentence names it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Shoulders => "shoulders",
            Self::Chest => "chest",
            Self::Hands => "hands",
            Self::Legs => "legs",
            Self::Feet => "feet",
            Self::Waist => "belt",
            Self::Amulet => "amulet",
            Self::Ring => "ring",
            Self::Medal => "medal",
            Self::Shield => "shield",
            Self::Offhand => "off-hand",
            Self::Axe => "one-handed axe",
            Self::Mace => "one-handed mace",
            Self::Sword => "one-handed sword",
            Self::Dagger => "dagger",
            Self::Scepter => "scepter",
            Self::Ranged1h => "one-handed ranged",
            Self::Axe2h => "two-handed axe",
            Self::Mace2h => "two-handed mace",
            Self::Sword2h => "two-handed sword",
            Self::Spear2h => "two-handed spear",
            Self::Staff => "staff",
            Self::Ranged2h => "two-handed ranged",
        }
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why a socket could not be read or edited.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SocketError {
    #[error("{record:?} is not in the database")]
    UnknownRecord { record: String },
    #[error("{record:?} could not be read from the database")]
    UnreadableRecord { record: String },
    /// The record is neither an `ItemRelic` nor an `ItemEnchantment`.
    #[error("{record:?} is neither a component nor an augment")]
    NotAPart { record: String },
    /// The item's class is not equipment.
    #[error("{record:?} has no sockets")]
    NoSockets { record: String },
    /// The part's record does not flag the host's slot.
    #[error("that {socket} does not go on a {slot}")]
    NotAllowed { socket: Socket, slot: Slot },
    #[error("the item already carries a {0}")]
    Occupied(Socket),
    #[error("the item carries no {0}")]
    Empty(Socket),
}

/// A record the database vouches for as a component or augment: the
/// socket it fills and the slots it admits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    id: RecordId,
    socket: Socket,
    slots: Vec<Slot>,
}

impl Part {
    /// Reads `id`'s record from the layered database.
    ///
    /// # Errors
    /// [`SocketError::UnknownRecord`], [`SocketError::UnreadableRecord`],
    /// or [`SocketError::NotAPart`].
    pub fn read(game: &GameData, id: &RecordId) -> Result<Self, SocketError> {
        Self::of_record(&record(game, id)?)
    }

    /// From a record already in hand.
    ///
    /// # Errors
    /// [`SocketError::NotAPart`] when its `Class` fills no socket.
    pub fn of_record(record: &DbRecord) -> Result<Self, SocketError> {
        let socket = record
            .string("Class")
            .map(|class| ItemClass::new(class.to_string()))
            .as_ref()
            .and_then(Socket::of_class)
            .ok_or_else(|| SocketError::NotAPart {
                record: record.id.as_str().to_string(),
            })?;
        let slots = Slot::ALL
            .into_iter()
            .filter(|slot| record.boolean(slot.variable()).unwrap_or(false))
            .collect();
        Ok(Self {
            id: record.id.clone(),
            socket,
            slots,
        })
    }

    #[must_use]
    pub fn id(&self) -> &RecordId {
        &self.id
    }

    #[must_use]
    pub fn socket(&self) -> Socket {
        self.socket
    }

    /// The slots the record flags, in [`Slot::ALL`] order.
    #[must_use]
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    #[must_use]
    pub fn fits(&self, slot: Slot) -> bool {
        self.slots.contains(&slot)
    }
}

/// The slot `item`'s base record occupies.
///
/// # Errors
/// [`SocketError::UnknownRecord`], [`SocketError::UnreadableRecord`],
/// or [`SocketError::NoSockets`] for a record that is not equipment.
pub fn host_slot(game: &GameData, item: &Item) -> Result<Slot, SocketError> {
    let id = RecordId::parse(item.base_name.clone()).ok_or_else(|| SocketError::UnknownRecord {
        record: item.base_name.clone(),
    })?;
    let record = record(game, &id)?;
    record
        .string("Class")
        .map(|class| ItemClass::new(class.to_string()))
        .as_ref()
        .and_then(Slot::of_class)
        .ok_or_else(|| SocketError::NoSockets {
            record: item.base_name.clone(),
        })
}

fn record(game: &GameData, id: &RecordId) -> Result<DbRecord, SocketError> {
    match game.record(id) {
        Some(Ok(record)) => Ok(record),
        Some(Err(_)) => Err(SocketError::UnreadableRecord {
            record: id.as_str().to_string(),
        }),
        None => Err(SocketError::UnknownRecord {
            record: id.as_str().to_string(),
        }),
    }
}

/// `host` with `part` in its socket under `seed`; `slot` is the host's
/// own ([`host_slot`]).
///
/// # Errors
/// [`SocketError::Occupied`] when the socket is filled — nothing is
/// ever overwritten — or [`SocketError::NotAllowed`] when the part's
/// record does not flag `slot`.
pub fn attach(host: &Item, slot: Slot, part: &Part, seed: u32) -> Result<Item, SocketError> {
    let socket = part.socket();
    if !socket.record_of(host).is_empty() {
        return Err(SocketError::Occupied(socket));
    }
    if !part.fits(slot) {
        return Err(SocketError::NotAllowed { socket, slot });
    }
    let record = part.id().as_str().to_string();
    let mut host = host.clone();
    match socket {
        Socket::Component => {
            host.relic_name = record;
            host.relic_bonus.clear();
            host.relic_seed = seed;
            host.relic_completion_level = COMPLETE_COMPONENT_LEVEL;
        }
        Socket::Augment => {
            host.augment_name = record;
            host.augment_seed = seed;
            host.unknown = 0;
        }
    }
    Ok(host)
}

/// What [`detach`] yields: the host without the part, and the part as
/// an item of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detached {
    pub host: Item,
    pub part: Item,
}

/// Takes the part out of `socket`; the freed part is a single
/// stand-alone item under `seed`.
///
/// # Errors
/// [`SocketError::Empty`] when the socket holds nothing.
pub fn detach(host: &Item, socket: Socket, seed: u32) -> Result<Detached, SocketError> {
    let record = socket.record_of(host);
    if record.is_empty() {
        return Err(SocketError::Empty(socket));
    }
    let part = Item {
        base_name: record.to_string(),
        seed,
        stack_count: 1,
        ..Item::default()
    };
    let mut host = host.clone();
    match socket {
        Socket::Component => {
            host.relic_name.clear();
            host.relic_bonus.clear();
            host.relic_seed = 0;
            host.relic_completion_level = COMPLETE_COMPONENT_LEVEL;
        }
        Socket::Augment => {
            host.augment_name.clear();
            host.augment_seed = 0;
            host.unknown = 0;
        }
    }
    Ok(Detached { host, part })
}

#[cfg(test)]
mod tests {
    use univault_engine::arz::fixture::{ArzBuilder, Values};
    use univault_engine::arz::{ArzDialect, ArzFile};
    use univault_engine::text::TextDb;

    use super::*;

    const COMPONENT: &str = "records/items/materia/compa_sample.dbr";
    const AUGMENT: &str = "records/items/enchants/a00a_enchant.dbr";
    const HELM: &str = "records/items/gearhead/a00_head.dbr";
    const RING: &str = "records/items/gearaccessories/rings/a00_ring.dbr";
    const SWORD: &str = "records/items/gearweapons/swords1h/a00_sword.dbr";
    const POTION: &str = "records/items/potions/healthpotion.dbr";

    fn game() -> GameData {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(
            COMPONENT,
            "ItemRelic",
            &[
                ("Class", Values::Strings(&["ItemRelic"])),
                ("completedRelicLevel", Values::Ints(&[1])),
                ("head", Values::Bools(&[true])),
                ("sword", Values::Bools(&[true])),
                ("ring", Values::Bools(&[false])),
            ],
        );
        builder.record(
            AUGMENT,
            "ItemEnchantment",
            &[
                ("Class", Values::Strings(&["ItemEnchantment"])),
                ("ring", Values::Bools(&[true])),
                ("amulet", Values::Bools(&[true])),
            ],
        );
        for (id, class) in [
            (HELM, "ArmorProtective_Head"),
            (RING, "ArmorJewelry_Ring"),
            (SWORD, "WeaponMelee_Sword"),
            (POTION, "OneShot_PotionHealth"),
        ] {
            builder.record(id, class, &[("Class", Values::Strings(&[class]))]);
        }
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        GameData::from_parts(vec![database], TextDb::new(), Vec::new())
    }

    fn id(path: &str) -> RecordId {
        RecordId::parse(path.to_string()).unwrap()
    }

    fn item(base: &str) -> Item {
        Item {
            base_name: base.into(),
            seed: 0x1234,
            stack_count: 1,
            ..Item::default()
        }
    }

    #[test]
    fn parts_read_their_socket_and_the_slots_their_record_flags() {
        let game = game();
        let component = Part::read(&game, &id(COMPONENT)).unwrap();
        assert_eq!(component.socket(), Socket::Component);
        assert_eq!(component.slots(), [Slot::Head, Slot::Sword]);
        assert!(component.fits(Slot::Head));
        assert!(!component.fits(Slot::Ring));
        let augment = Part::read(&game, &id(AUGMENT)).unwrap();
        assert_eq!(augment.socket(), Socket::Augment);
        assert_eq!(augment.slots(), [Slot::Amulet, Slot::Ring]);
    }

    #[test]
    fn records_that_are_no_part_are_refused_by_name() {
        let game = game();
        assert_eq!(
            Part::read(&game, &id(HELM)),
            Err(SocketError::NotAPart {
                record: HELM.into()
            })
        );
        assert_eq!(
            Part::read(&game, &id("records/items/nothing.dbr")),
            Err(SocketError::UnknownRecord {
                record: "records/items/nothing.dbr".into()
            })
        );
    }

    #[test]
    fn hosts_take_their_slot_from_their_class() {
        let game = game();
        assert_eq!(host_slot(&game, &item(HELM)), Ok(Slot::Head));
        assert_eq!(host_slot(&game, &item(RING)), Ok(Slot::Ring));
        assert_eq!(host_slot(&game, &item(SWORD)), Ok(Slot::Sword));
        assert_eq!(
            host_slot(&game, &item(POTION)),
            Err(SocketError::NoSockets {
                record: POTION.into()
            })
        );
        assert_eq!(
            host_slot(&game, &item(COMPONENT)),
            Err(SocketError::NoSockets {
                record: COMPONENT.into()
            })
        );
        assert_eq!(
            host_slot(&game, &Item::default()),
            Err(SocketError::UnknownRecord {
                record: String::new()
            })
        );
    }

    #[test]
    fn attaching_fills_the_socket_and_detaching_frees_the_part() {
        let game = game();
        let helm = item(HELM);
        let component = Part::read(&game, &id(COMPONENT)).unwrap();
        let socketed = attach(&helm, Slot::Head, &component, 0x61ba_ef85).unwrap();
        assert_eq!(socketed.relic_name, COMPONENT);
        assert_eq!(socketed.relic_seed, 0x61ba_ef85);
        assert_eq!(socketed.relic_bonus, "");
        assert_eq!(socketed.relic_completion_level, COMPLETE_COMPONENT_LEVEL);
        assert_eq!(socketed.base_name, helm.base_name);
        assert_eq!(socketed.seed, helm.seed);

        let Detached { host, part } = detach(&socketed, Socket::Component, 77).unwrap();
        assert_eq!(host, helm);
        assert_eq!(
            part,
            Item {
                base_name: COMPONENT.into(),
                seed: 77,
                stack_count: 1,
                ..Item::default()
            }
        );
    }

    #[test]
    fn augments_use_their_own_fields() {
        let game = game();
        let ring = item(RING);
        let augment = Part::read(&game, &id(AUGMENT)).unwrap();
        let augmented = attach(&ring, Slot::Ring, &augment, 0x3a9f_8374).unwrap();
        assert_eq!(augmented.augment_name, AUGMENT);
        assert_eq!(augmented.augment_seed, 0x3a9f_8374);
        assert_eq!(augmented.unknown, 0);
        assert!(augmented.relic_name.is_empty());
        assert_eq!(detach(&augmented, Socket::Augment, 5).unwrap().host, ring);
        assert_eq!(
            detach(&augmented, Socket::Component, 5),
            Err(SocketError::Empty(Socket::Component))
        );
    }

    #[test]
    fn a_filled_socket_is_never_overwritten() {
        let game = game();
        let component = Part::read(&game, &id(COMPONENT)).unwrap();
        let socketed = attach(&item(HELM), Slot::Head, &component, 1).unwrap();
        assert_eq!(
            attach(&socketed, Slot::Head, &component, 2),
            Err(SocketError::Occupied(Socket::Component))
        );
        assert_eq!(
            detach(&item(HELM), Socket::Component, 1),
            Err(SocketError::Empty(Socket::Component))
        );
    }

    #[test]
    fn a_part_refuses_a_slot_its_record_does_not_flag() {
        let game = game();
        let component = Part::read(&game, &id(COMPONENT)).unwrap();
        assert_eq!(
            attach(&item(RING), Slot::Ring, &component, 1),
            Err(SocketError::NotAllowed {
                socket: Socket::Component,
                slot: Slot::Ring
            })
        );
        let augment = Part::read(&game, &id(AUGMENT)).unwrap();
        assert_eq!(
            attach(&item(SWORD), Slot::Sword, &augment, 1),
            Err(SocketError::NotAllowed {
                socket: Socket::Augment,
                slot: Slot::Sword
            })
        );
    }

    #[test]
    fn slot_variables_are_distinct_and_every_equipment_class_maps() {
        let variables: std::collections::HashSet<&str> =
            Slot::ALL.iter().map(|slot| slot.variable()).collect();
        assert_eq!(variables.len(), Slot::ALL.len());
        for class in [
            "ArmorProtective_Waist",
            "ArmorJewelry_Medal",
            "WeaponArmor_Offhand",
            "WeaponMelee_Spear2h",
            "WeaponMagical_Staff",
            "WeaponHunting_Ranged2h",
        ] {
            assert!(
                Slot::of_class(&ItemClass::new(class.into())).is_some(),
                "{class}"
            );
        }
        assert_eq!(Slot::of_class(&ItemClass::new("ItemArtifact".into())), None);
        assert_eq!(Slot::Waist.label(), "belt");
        assert_eq!(
            SocketError::NotAllowed {
                socket: Socket::Augment,
                slot: Slot::Waist
            }
            .to_string(),
            "that augment does not go on a belt"
        );
    }
}
