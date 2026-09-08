//! `player.gdc`: the character save.
//!
//! Header and blocks 1, 3, 4 ported from gdlc (MIT, dandels 2025),
//! `src/player.rs`.
//!
//! Layout: raw seed; `"GDCX"`; header version 2; character name (wide),
//! sex, class tag, level, hardcore flag, expansion status byte; a static
//! zero marker; data version; 16-byte uid; then an ordered sequence of
//! framed blocks to end of file. Every block the game writes is typed —
//! 1 (character info, v5), 3 (inventory) and 4 (per-character stash)
//! here, the rest in [`crate::blocks`] — so the whole file re-encodes
//! from the model and an edit anywhere can be written. A block id this
//! crate does not model, or a modeled id at a version it does not lay
//! out, is carried as an [`OpaqueBlock`]; see `block` for what that
//! promises and what it forbids (no edit before it can be written).
//!
//! [`PlayerFile::encode`] reproduces the loaded bytes exactly when the
//! model is unmodified; `tests/` gates that against the vendored fixture.

use std::borrow::{Borrow, BorrowMut};
use std::fmt;

use serde::{Deserialize, Serialize};

use thiserror::Error;

use crate::block::{
    Dispatch, OpaqueBlock, OpaqueReason, SaveEncodeError, StashTab, length_word, read_block,
};
use crate::blocks::bio::Bio;
use crate::blocks::factions::Factions;
use crate::blocks::markers::Markers;
use crate::blocks::notes::LoreNotes;
use crate::blocks::respawns::Respawns;
use crate::blocks::shrines::Shrines;
use crate::blocks::skills::{Skills, SkillsVersion};
use crate::blocks::stats::{Stats, StatsVersion};
use crate::blocks::teleports::Teleports;
use crate::blocks::tokens::Tokens;
use crate::blocks::tutorials::Tutorials;
use crate::blocks::ui::{Ui, UiVersion};
use crate::blocks::{
    bio, factions, markers, notes, read_array, respawns, shrines, skills, stats, teleports, tokens,
    tutorials, ui,
};
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};
use crate::item::{ContainerVersion, Item, ItemEncodeError, SackItem};

const MAGIC: u32 = u32::from_le_bytes(*b"GDCX");
const HEADER_VERSION: u32 = 2;
const CHARACTER_INFO_VERSION: u32 = 5;
const PLAYER_STASH_MIN_VERSION: u32 = 6;
const EQUIPMENT_SLOTS: usize = 12;
const WEAPON_SLOTS: usize = 2;

/// Why a `player.gdc` could not be parsed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GdcError {
    /// Cipher / framing failure.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// The file does not start with `GDCX`.
    #[error("not a player.gdc: magic {found:#010x}")]
    BadMagic {
        /// The word found where `GDCX` belongs.
        found: u32,
    },
    /// The header version is not the one this crate lays out.
    #[error("unsupported player.gdc header version {version} (expected {HEADER_VERSION})")]
    UnsupportedHeaderVersion {
        /// The version read.
        version: u32,
    },
}

/// Which of the game's two character folders a `player.gdc` lives in:
/// `main/` holds main-campaign characters, `user/` the custom-game
/// (mod) characters. The file itself does not record this — the game
/// lists every `user/` character under every mod — so the realm is
/// part of a character's identity only through its location, and any
/// record that names a character (a store origin) carries it
/// explicitly rather than deriving it from where the file was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Realm {
    /// The main campaign: `main/`.
    Main,
    /// Custom games, that is mods: `user/`.
    Custom,
}

impl Realm {
    /// Both realms in the order the shell lists characters.
    pub const ALL: [Self; 2] = [Self::Main, Self::Custom];

    /// The folder under the save directory holding this realm's
    /// per-character folders.
    #[must_use]
    pub const fn dir_name(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Custom => "user",
        }
    }

    /// The realm whose folder is called `name`.
    #[must_use]
    pub fn parse_dir_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|realm| realm.dir_name() == name)
    }
}

impl fmt::Display for Realm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Main => "main campaign",
            Self::Custom => "custom game",
        })
    }
}

/// Character sex flag. `false`/`true` on the wire; the mapping is
/// inferred from character names in the fixture and user saves and is
/// not verified against the game UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sex {
    /// Wire value 0.
    Female,
    /// Wire value 1.
    Male,
}

impl From<bool> for Sex {
    fn from(flag: bool) -> Self {
        if flag { Self::Male } else { Self::Female }
    }
}

impl From<Sex> for bool {
    fn from(sex: Sex) -> Self {
        matches!(sex, Sex::Male)
    }
}

/// The fixed header preceding the block sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerHeader {
    /// Character name.
    pub name: String,
    /// Sex flag.
    pub sex: Sex,
    /// Class tag such as `tagSkillClassName0506`; empty before a class
    /// is chosen.
    pub class_tag: String,
    /// Character level.
    pub level: u32,
    /// Hardcore flag.
    pub hardcore: bool,
    /// Expansion status byte (observed 7 with both expansions, 3 on an
    /// older character).
    pub expansion_status: u8,
    /// Data version following the zero marker (observed 8).
    pub data_version: u32,
    /// 16-byte uid (observed all zero).
    pub uid: [u8; 16],
}

/// Block 1, version 5. Byte fields whose semantics gdlc only names are
/// kept as bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterInfo {
    /// Whether the character is in the main quest.
    pub is_in_main_quest: bool,
    /// Whether the character has ever entered the game.
    pub has_been_in_game: bool,
    /// Current difficulty.
    pub difficulty: u8,
    /// Highest difficulty reached.
    pub greatest_difficulty: u8,
    /// Iron bits.
    pub money: u32,
    /// Highest survival-mode difficulty reached.
    pub greatest_survival_difficulty: u8,
    /// Current tribute.
    pub current_tribute: u32,
    /// Compass state.
    pub compass_state: u8,
    /// Skill-window help toggle.
    pub skill_window_show_help: u8,
    /// Weapon-swap active flag.
    pub weapon_swap_active: u8,
    /// Weapon-swap enabled flag.
    pub weapon_swap_enabled: u8,
    /// Character texture record, or empty.
    pub texture: String,
    /// Loot filter toggles.
    pub loot_filter: Vec<u8>,
}

impl CharacterInfo {
    fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let is_in_main_quest = dec.read_bool()?;
        let has_been_in_game = dec.read_bool()?;
        let difficulty = dec.read_u8()?;
        let greatest_difficulty = dec.read_u8()?;
        let money = dec.read_u32()?;
        let greatest_survival_difficulty = dec.read_u8()?;
        let current_tribute = dec.read_u32()?;
        let compass_state = dec.read_u8()?;
        let skill_window_show_help = dec.read_u8()?;
        let weapon_swap_active = dec.read_u8()?;
        let weapon_swap_enabled = dec.read_u8()?;
        let texture = dec.read_string()?;
        let loot_filter_len = dec.read_u32()?;
        let loot_filter = dec.read_bytes(loot_filter_len as usize)?;
        Ok(Self {
            is_in_main_quest,
            has_been_in_game,
            difficulty,
            greatest_difficulty,
            money,
            greatest_survival_difficulty,
            current_tribute,
            compass_state,
            skill_window_show_help,
            weapon_swap_active,
            weapon_swap_enabled,
            texture,
            loot_filter,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::CHARACTER_INFO, |enc| {
            enc.write_u32(CHARACTER_INFO_VERSION);
            enc.write_bool(self.is_in_main_quest);
            enc.write_bool(self.has_been_in_game);
            enc.write_u8(self.difficulty);
            enc.write_u8(self.greatest_difficulty);
            enc.write_u32(self.money);
            enc.write_u8(self.greatest_survival_difficulty);
            enc.write_u32(self.current_tribute);
            enc.write_u8(self.compass_state);
            enc.write_u8(self.skill_window_show_help);
            enc.write_u8(self.weapon_swap_active);
            enc.write_u8(self.weapon_swap_enabled);
            enc.write_string(&self.texture)?;
            enc.write_u32(length_word(self.loot_filter.len())?);
            enc.write_bytes(&self.loot_filter);
            Ok(())
        })
    }
}

/// An equipment slot: the item (empty base name for an empty slot) and
/// gdlc's `attached` byte.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EquippedItem {
    /// The item, or an empty item for an empty slot.
    pub item: Item,
    /// Attached flag byte.
    pub attached: u8,
}

impl EquippedItem {
    /// An empty slot as the game writes one it never filled: no item,
    /// not attached, and a stack count of 1 — the shape of every empty
    /// slot in the gdlc fixture and in five real characters
    /// (2026-09-08). A slot the game itself emptied keeps stale fields
    /// of its last occupant besides; this app writes the clean form.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            item: Item {
                stack_count: 1,
                ..Item::default()
            },
            attached: 0,
        }
    }

    fn read(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        let item = Item::read(dec, version)?;
        let attached = dec.read_u8()?;
        Ok(Self { item, attached })
    }

    fn write(&self, enc: &mut Encoder, version: ContainerVersion) -> Result<(), ItemEncodeError> {
        self.item.write(enc, version)?;
        enc.write_u8(self.attached);
        Ok(())
    }
}

/// A character's two weapon sets; block 3's `useAlternate` byte says
/// which is in hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeaponSet {
    First,
    Second,
}

impl WeaponSet {
    /// Both sets, in file order.
    pub const ALL: [Self; 2] = [Self::First, Self::Second];

    /// The set's number as the game shows it.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::First => 1,
            Self::Second => 2,
        }
    }
}

impl fmt::Display for WeaponSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "weapon set {}", self.number())
    }
}

/// One of a character's equipment slots: the twelve worn slots in
/// block 3's order (GD Stash's order, checked against the classes of
/// the items the user's characters wear), then each weapon set's two
/// hands. Serializes by name, so a store origin naming a slot never
/// depends on this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EquipSlot {
    Head,
    Amulet,
    Chest,
    Legs,
    Feet,
    Hands,
    Ring1,
    Ring2,
    Belt,
    Shoulders,
    Medal,
    Relic,
    MainHand1,
    OffHand1,
    MainHand2,
    OffHand2,
}

/// Where a slot sits in [`InventoryContents`].
enum SlotPlace {
    Worn(usize),
    Hand(WeaponSet, usize),
}

impl EquipSlot {
    /// Every slot, in file order.
    pub const ALL: [Self; 16] = [
        Self::Head,
        Self::Amulet,
        Self::Chest,
        Self::Legs,
        Self::Feet,
        Self::Hands,
        Self::Ring1,
        Self::Ring2,
        Self::Belt,
        Self::Shoulders,
        Self::Medal,
        Self::Relic,
        Self::MainHand1,
        Self::OffHand1,
        Self::MainHand2,
        Self::OffHand2,
    ];

    /// The twelve worn slots, in file order.
    pub const WORN: [Self; 12] = [
        Self::Head,
        Self::Amulet,
        Self::Chest,
        Self::Legs,
        Self::Feet,
        Self::Hands,
        Self::Ring1,
        Self::Ring2,
        Self::Belt,
        Self::Shoulders,
        Self::Medal,
        Self::Relic,
    ];

    /// A weapon set's main and off hand.
    #[must_use]
    pub const fn hands(set: WeaponSet) -> [Self; 2] {
        match set {
            WeaponSet::First => [Self::MainHand1, Self::OffHand1],
            WeaponSet::Second => [Self::MainHand2, Self::OffHand2],
        }
    }

    /// The slot's name as the game's character sheet labels it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Head => "Head",
            Self::Amulet => "Amulet",
            Self::Chest => "Chest",
            Self::Legs => "Legs",
            Self::Feet => "Feet",
            Self::Hands => "Hands",
            Self::Ring1 => "Ring 1",
            Self::Ring2 => "Ring 2",
            Self::Belt => "Belt",
            Self::Shoulders => "Shoulders",
            Self::Medal => "Medal",
            Self::Relic => "Relic",
            Self::MainHand1 | Self::MainHand2 => "Main hand",
            Self::OffHand1 | Self::OffHand2 => "Off hand",
        }
    }

    /// The weapon set a hand slot belongs to; `None` for a worn slot.
    #[must_use]
    pub const fn weapon_set(self) -> Option<WeaponSet> {
        match self.place() {
            SlotPlace::Worn(_) => None,
            SlotPlace::Hand(set, _) => Some(set),
        }
    }

    const fn place(self) -> SlotPlace {
        match self {
            Self::Head => SlotPlace::Worn(0),
            Self::Amulet => SlotPlace::Worn(1),
            Self::Chest => SlotPlace::Worn(2),
            Self::Legs => SlotPlace::Worn(3),
            Self::Feet => SlotPlace::Worn(4),
            Self::Hands => SlotPlace::Worn(5),
            Self::Ring1 => SlotPlace::Worn(6),
            Self::Ring2 => SlotPlace::Worn(7),
            Self::Belt => SlotPlace::Worn(8),
            Self::Shoulders => SlotPlace::Worn(9),
            Self::Medal => SlotPlace::Worn(10),
            Self::Relic => SlotPlace::Worn(11),
            Self::MainHand1 => SlotPlace::Hand(WeaponSet::First, 0),
            Self::OffHand1 => SlotPlace::Hand(WeaponSet::First, 1),
            Self::MainHand2 => SlotPlace::Hand(WeaponSet::Second, 0),
            Self::OffHand2 => SlotPlace::Hand(WeaponSet::Second, 1),
        }
    }
}

impl fmt::Display for EquipSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.weapon_set() {
            None => f.write_str(self.label()),
            Some(set) => write!(f, "{} ({set})", self.label()),
        }
    }
}

/// One inventory sack (bag).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sack {
    /// Leading flag byte (gdlc reads it as a boolean it calls `is_ok`).
    pub flag: u8,
    /// Items with their cell positions.
    pub items: Vec<SackItem>,
}

impl Sack {
    fn read(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        dec.read_block_start_expecting(BlockId::NESTED)?;
        let flag = dec.read_u8()?;
        let count = dec.read_u32()?;
        let items = (0..count)
            .map(|_| SackItem::read(dec, version))
            .collect::<Result<Vec<_>, _>>()?;
        dec.read_block_end()?;
        Ok(Self { flag, items })
    }

    fn write(&self, enc: &mut Encoder, version: ContainerVersion) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::NESTED, |enc| {
            enc.write_u8(self.flag);
            enc.write_u32(length_word(self.items.len())?);
            self.items
                .iter()
                .try_for_each(|item| item.write(enc, version))?;
            Ok(())
        })
    }
}

/// Sacks and equipment of a character that has entered the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryContents {
    /// Index of the focused sack.
    pub focused_sack: u32,
    /// Index of the selected sack.
    pub selected_sack: u32,
    /// The sacks, in order.
    pub sacks: Vec<Sack>,
    /// Weapon-swap byte preceding the equipment.
    pub use_alternate: u8,
    /// The twelve equipment slots.
    pub equipment: [EquippedItem; EQUIPMENT_SLOTS],
    /// Byte preceding weapon set 1.
    pub alternate_1: u8,
    /// Weapon set 1.
    pub weapon_set_1: [EquippedItem; WEAPON_SLOTS],
    /// Byte preceding weapon set 2.
    pub alternate_2: u8,
    /// Weapon set 2.
    pub weapon_set_2: [EquippedItem; WEAPON_SLOTS],
}

impl InventoryContents {
    fn read(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        let sack_count = dec.read_u32()?;
        let focused_sack = dec.read_u32()?;
        let selected_sack = dec.read_u32()?;
        let sacks = (0..sack_count)
            .map(|_| Sack::read(dec, version))
            .collect::<Result<Vec<_>, _>>()?;
        let use_alternate = dec.read_u8()?;
        let equipment = read_array(|| EquippedItem::read(dec, version))?;
        let alternate_1 = dec.read_u8()?;
        let weapon_set_1 = read_array(|| EquippedItem::read(dec, version))?;
        let alternate_2 = dec.read_u8()?;
        let weapon_set_2 = read_array(|| EquippedItem::read(dec, version))?;
        Ok(Self {
            focused_sack,
            selected_sack,
            sacks,
            use_alternate,
            equipment,
            alternate_1,
            weapon_set_1,
            alternate_2,
            weapon_set_2,
        })
    }

    fn write(&self, enc: &mut Encoder, version: ContainerVersion) -> Result<(), SaveEncodeError> {
        enc.write_u32(length_word(self.sacks.len())?);
        enc.write_u32(self.focused_sack);
        enc.write_u32(self.selected_sack);
        self.sacks
            .iter()
            .try_for_each(|sack| sack.write(enc, version))?;
        enc.write_u8(self.use_alternate);
        write_slots(enc, version, &self.equipment)?;
        enc.write_u8(self.alternate_1);
        write_slots(enc, version, &self.weapon_set_1)?;
        enc.write_u8(self.alternate_2);
        write_slots(enc, version, &self.weapon_set_2)?;
        Ok(())
    }

    /// Every occupied slot across equipment and both weapon sets.
    pub fn equipped(&self) -> impl Iterator<Item = &EquippedItem> {
        self.slots()
            .map(|(_, worn)| worn)
            .filter(|worn| !worn.item.is_empty())
    }

    /// Every slot with what it holds, in file order.
    pub fn slots(&self) -> impl Iterator<Item = (EquipSlot, &EquippedItem)> {
        EquipSlot::ALL
            .into_iter()
            .map(|slot| (slot, self.slot(slot)))
    }

    /// What `slot` holds — an empty slot for nothing.
    #[must_use]
    pub fn slot(&self, slot: EquipSlot) -> &EquippedItem {
        match slot.place() {
            SlotPlace::Worn(index) => &self.equipment[index],
            SlotPlace::Hand(WeaponSet::First, index) => &self.weapon_set_1[index],
            SlotPlace::Hand(WeaponSet::Second, index) => &self.weapon_set_2[index],
        }
    }

    /// What `slot` holds, for editing.
    pub fn slot_mut(&mut self, slot: EquipSlot) -> &mut EquippedItem {
        match slot.place() {
            SlotPlace::Worn(index) => &mut self.equipment[index],
            SlotPlace::Hand(WeaponSet::First, index) => &mut self.weapon_set_1[index],
            SlotPlace::Hand(WeaponSet::Second, index) => &mut self.weapon_set_2[index],
        }
    }

    /// The weapon set in the character's hands: `useAlternate` zero is
    /// the first set (every real file read so far), anything else the
    /// second.
    #[must_use]
    pub const fn active_weapon_set(&self) -> WeaponSet {
        if self.use_alternate == 0 {
            WeaponSet::First
        } else {
            WeaponSet::Second
        }
    }
}

fn write_slots(
    enc: &mut Encoder,
    version: ContainerVersion,
    slots: &[EquippedItem],
) -> Result<(), ItemEncodeError> {
    slots.iter().try_for_each(|slot| slot.write(enc, version))
}

/// What block 3 holds beyond its version and flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InventoryState {
    /// The character has never entered the game: the block ends after
    /// the flag byte.
    NeverEntered,
    /// Sacks and equipment.
    Entered(Box<InventoryContents>),
}

/// Block 3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    /// Layout version.
    pub version: ContainerVersion,
    /// Flag byte following the version (gdlc expects 0).
    pub flag: u8,
    /// Contents.
    pub state: InventoryState,
}

impl Inventory {
    fn read_body(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        let flag = dec.read_u8()?;
        let state = if dec.is_at_end() {
            InventoryState::NeverEntered
        } else {
            InventoryState::Entered(Box::new(InventoryContents::read(dec, version)?))
        };
        Ok(Self {
            version,
            flag,
            state,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::INVENTORY, |enc| {
            enc.write_u32(self.version.raw());
            enc.write_u8(self.flag);
            match &self.state {
                InventoryState::NeverEntered => Ok(()),
                InventoryState::Entered(contents) => contents.write(enc, self.version),
            }
        })
    }

    /// The sacks; empty for a character that never entered the game.
    #[must_use]
    pub fn sacks(&self) -> &[Sack] {
        match &self.state {
            InventoryState::NeverEntered => &[],
            InventoryState::Entered(contents) => &contents.sacks,
        }
    }

    /// The sacks, mutably; empty for a character that never entered
    /// the game.
    pub fn sacks_mut(&mut self) -> &mut [Sack] {
        match &mut self.state {
            InventoryState::NeverEntered => &mut [],
            InventoryState::Entered(contents) => &mut contents.sacks,
        }
    }

    /// Every occupied equipment slot.
    pub fn equipped(&self) -> impl Iterator<Item = &EquippedItem> {
        self.contents()
            .into_iter()
            .flat_map(InventoryContents::equipped)
    }

    /// The sacks and equipment; `None` for a character that never
    /// entered the game and so has neither.
    #[must_use]
    pub fn contents(&self) -> Option<&InventoryContents> {
        match &self.state {
            InventoryState::NeverEntered => None,
            InventoryState::Entered(contents) => Some(contents),
        }
    }

    /// The sacks and equipment, for editing; `None` as [`Self::contents`].
    pub fn contents_mut(&mut self) -> Option<&mut InventoryContents> {
        match &mut self.state {
            InventoryState::NeverEntered => None,
            InventoryState::Entered(contents) => Some(contents),
        }
    }
}

/// Block 4: the per-character stash.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerStash {
    /// Layout version.
    pub version: ContainerVersion,
    /// The tabs, in order.
    pub tabs: Vec<StashTab>,
}

impl PlayerStash {
    fn read_body(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        let count = dec.read_u32()?;
        let tabs = (0..count)
            .map(|_| StashTab::read(dec, version))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { version, tabs })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::PLAYER_STASH, |enc| {
            enc.write_u32(self.version.raw());
            enc.write_u32(length_word(self.tabs.len())?);
            self.tabs
                .iter()
                .try_for_each(|tab| tab.write(enc, self.version))
        })
    }
}

/// One top-level block of the file, in file order. The game writes them
/// as `1 2 3 4 5 6 7 17 8 12 13 14 15 16 10`; the model keeps whatever
/// order and set it finds.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// Block 1 at version 5.
    CharacterInfo(CharacterInfo),
    /// Block 2 at version 8.
    Bio(Bio),
    /// Block 3 at a supported version.
    Inventory(Inventory),
    /// Block 4 at a supported version.
    Stash(PlayerStash),
    /// Block 5 at version 1.
    Respawns(Respawns),
    /// Block 6 at version 1.
    Teleports(Teleports),
    /// Block 7 at version 1.
    Markers(Markers),
    /// Block 17 at version 2.
    Shrines(Shrines),
    /// Block 8 at a supported version.
    Skills(Skills),
    /// Block 12 at version 1.
    LoreNotes(LoreNotes),
    /// Block 13 at version 5.
    Factions(Factions),
    /// Block 14 at a supported version.
    Ui(Box<Ui>),
    /// Block 15 at version 1.
    Tutorials(Tutorials),
    /// Block 16 at a supported version.
    Stats(Box<Stats>),
    /// Block 10 at version 2.
    Tokens(Tokens),
    /// Anything else, preserved verbatim.
    Opaque(OpaqueBlock),
}

impl Block {
    /// The block id.
    #[must_use]
    pub fn id(&self) -> BlockId {
        match self {
            Self::CharacterInfo(_) => BlockId::CHARACTER_INFO,
            Self::Bio(_) => bio::BLOCK_ID,
            Self::Inventory(_) => BlockId::INVENTORY,
            Self::Stash(_) => BlockId::PLAYER_STASH,
            Self::Respawns(_) => respawns::BLOCK_ID,
            Self::Teleports(_) => teleports::BLOCK_ID,
            Self::Markers(_) => markers::BLOCK_ID,
            Self::Shrines(_) => shrines::BLOCK_ID,
            Self::Skills(_) => skills::BLOCK_ID,
            Self::LoreNotes(_) => notes::BLOCK_ID,
            Self::Factions(_) => factions::BLOCK_ID,
            Self::Ui(_) => ui::BLOCK_ID,
            Self::Tutorials(_) => tutorials::BLOCK_ID,
            Self::Stats(_) => stats::BLOCK_ID,
            Self::Tokens(_) => tokens::BLOCK_ID,
            Self::Opaque(block) => block.id(),
        }
    }

    /// Whether the block is carried opaquely.
    #[must_use]
    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque(_))
    }

    fn read(dec: &mut Decoder<'_>) -> Result<Self, GdcError> {
        read_block(
            dec,
            |dec, header| {
                let unsupported = OpaqueReason::UnsupportedVersion {
                    version: header.version,
                };
                let version = header.version;
                Ok::<_, GdcError>(match header.id {
                    BlockId::CHARACTER_INFO if version == CHARACTER_INFO_VERSION => {
                        Dispatch::Typed(Self::CharacterInfo(CharacterInfo::read_body(dec)?))
                    }
                    bio::BLOCK_ID if version == bio::VERSION => {
                        Dispatch::Typed(Self::Bio(Bio::read_body(dec)?))
                    }
                    BlockId::INVENTORY => match ContainerVersion::new(version) {
                        Ok(version) => {
                            Dispatch::Typed(Self::Inventory(Inventory::read_body(dec, version)?))
                        }
                        Err(_) => Dispatch::Opaque(unsupported),
                    },
                    BlockId::PLAYER_STASH => match ContainerVersion::new(version) {
                        Ok(version) if version.raw() >= PLAYER_STASH_MIN_VERSION => {
                            Dispatch::Typed(Self::Stash(PlayerStash::read_body(dec, version)?))
                        }
                        Ok(_) | Err(_) => Dispatch::Opaque(unsupported),
                    },
                    respawns::BLOCK_ID if version == respawns::VERSION => {
                        Dispatch::Typed(Self::Respawns(Respawns::read_body(dec)?))
                    }
                    teleports::BLOCK_ID if version == teleports::VERSION => {
                        Dispatch::Typed(Self::Teleports(Teleports::read_body(dec)?))
                    }
                    markers::BLOCK_ID if version == markers::VERSION => {
                        Dispatch::Typed(Self::Markers(Markers::read_body(dec)?))
                    }
                    shrines::BLOCK_ID if version == shrines::VERSION => {
                        Dispatch::Typed(Self::Shrines(Shrines::read_body(dec)?))
                    }
                    skills::BLOCK_ID => match SkillsVersion::new(version) {
                        Some(version) => {
                            Dispatch::Typed(Self::Skills(Skills::read_body(dec, version)?))
                        }
                        None => Dispatch::Opaque(unsupported),
                    },
                    notes::BLOCK_ID if version == notes::VERSION => {
                        Dispatch::Typed(Self::LoreNotes(LoreNotes::read_body(dec)?))
                    }
                    factions::BLOCK_ID if version == factions::VERSION => {
                        Dispatch::Typed(Self::Factions(Factions::read_body(dec)?))
                    }
                    ui::BLOCK_ID => match UiVersion::new(version) {
                        Some(version) => {
                            Dispatch::Typed(Self::Ui(Box::new(Ui::read_body(dec, version)?)))
                        }
                        None => Dispatch::Opaque(unsupported),
                    },
                    tutorials::BLOCK_ID if version == tutorials::VERSION => {
                        Dispatch::Typed(Self::Tutorials(Tutorials::read_body(dec)?))
                    }
                    stats::BLOCK_ID => match StatsVersion::new(version) {
                        Some(version) => {
                            Dispatch::Typed(Self::Stats(Box::new(Stats::read_body(dec, version)?)))
                        }
                        None => Dispatch::Opaque(unsupported),
                    },
                    tokens::BLOCK_ID if version == tokens::VERSION => {
                        Dispatch::Typed(Self::Tokens(Tokens::read_body(dec)?))
                    }
                    BlockId::CHARACTER_INFO
                    | bio::BLOCK_ID
                    | respawns::BLOCK_ID
                    | teleports::BLOCK_ID
                    | markers::BLOCK_ID
                    | shrines::BLOCK_ID
                    | notes::BLOCK_ID
                    | factions::BLOCK_ID
                    | tutorials::BLOCK_ID
                    | tokens::BLOCK_ID => Dispatch::Opaque(unsupported),
                    _ => Dispatch::Opaque(OpaqueReason::Unmodeled),
                })
            },
            Self::Opaque,
        )
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        match self {
            Self::CharacterInfo(info) => info.write(enc),
            Self::Bio(bio) => bio.write(enc),
            Self::Inventory(inventory) => inventory.write(enc),
            Self::Stash(stash) => stash.write(enc),
            Self::Respawns(respawns) => respawns.write(enc),
            Self::Teleports(teleports) => teleports.write(enc),
            Self::Markers(markers) => markers.write(enc),
            Self::Shrines(shrines) => shrines.write(enc),
            Self::Skills(skills) => skills.write(enc),
            Self::LoreNotes(notes) => notes.write(enc),
            Self::Factions(factions) => factions.write(enc),
            Self::Ui(ui) => ui.write(enc),
            Self::Tutorials(tutorials) => tutorials.write(enc),
            Self::Stats(stats) => stats.write(enc),
            Self::Tokens(tokens) => tokens.write(enc),
            Self::Opaque(block) => block.write(enc),
        }
    }
}

/// Projects the first block of one variant. A variant added later is
/// correctly *not* that block, so the `else` branch is the
/// specification rather than a sink.
macro_rules! block_accessor {
    ($(#[$doc:meta])* $name:ident: $variant:ident($ty:ty)) => {
        $(#[$doc])*
        #[must_use]
        pub fn $name(&self) -> Option<&$ty> {
            self.blocks.iter().find_map(|block| {
                if let Block::$variant(value) = block {
                    Some(Borrow::<$ty>::borrow(value))
                } else {
                    None
                }
            })
        }
    };
    ($(#[$doc:meta])* $name:ident, $name_mut:ident: $variant:ident($ty:ty)) => {
        block_accessor!($(#[$doc])* $name: $variant($ty));

        $(#[$doc])*
        pub fn $name_mut(&mut self) -> Option<&mut $ty> {
            self.blocks.iter_mut().find_map(|block| {
                if let Block::$variant(value) = block {
                    Some(BorrowMut::<$ty>::borrow_mut(value))
                } else {
                    None
                }
            })
        }
    };
}

/// A parsed `player.gdc`: header plus the ordered block sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerFile {
    seed: u32,
    header: PlayerHeader,
    blocks: Vec<Block>,
}

impl PlayerFile {
    /// Assembles a file from its parts; [`encode`](Self::encode) writes
    /// exactly what it is given. Parsing bytes is [`parse`](Self::parse).
    #[must_use]
    pub fn from_parts(seed: u32, header: PlayerHeader, blocks: Vec<Block>) -> Self {
        Self {
            seed,
            header,
            blocks,
        }
    }

    /// Parses a whole file image.
    ///
    /// # Errors
    /// [`GdcError`] for a bad magic or header version, or any cipher /
    /// framing failure — including a typed block whose layout did not
    /// consume exactly its declared length.
    pub fn parse(bytes: &[u8]) -> Result<Self, GdcError> {
        let mut dec = Decoder::new(bytes)?;
        let magic = dec.read_u32()?;
        if magic != MAGIC {
            return Err(GdcError::BadMagic { found: magic });
        }
        let header_version = dec.read_u32()?;
        if header_version != HEADER_VERSION {
            return Err(GdcError::UnsupportedHeaderVersion {
                version: header_version,
            });
        }
        let name = dec.read_wstring()?;
        let sex = Sex::from(dec.read_bool()?);
        let class_tag = dec.read_string()?;
        let level = dec.read_u32()?;
        let hardcore = dec.read_bool()?;
        let expansion_status = dec.read_u8()?;
        dec.read_zero_marker()?;
        let data_version = dec.read_u32()?;
        let uid = read_array(|| dec.read_u8())?;
        let header = PlayerHeader {
            name,
            sex,
            class_tag,
            level,
            hardcore,
            expansion_status,
            data_version,
            uid,
        };
        let mut blocks = Vec::new();
        while !dec.is_at_end() {
            blocks.push(Block::read(&mut dec)?);
        }
        Ok(Self {
            seed: dec.seed(),
            header,
            blocks,
        })
    }

    /// Re-encodes the file; byte-identical to the input when unmodified.
    ///
    /// # Errors
    /// [`SaveEncodeError`], notably `OpaqueRekeyed` when a block before
    /// an opaque one changed.
    pub fn encode(&self) -> Result<Vec<u8>, SaveEncodeError> {
        let mut enc = Encoder::new(self.seed);
        enc.write_u32(MAGIC);
        enc.write_u32(HEADER_VERSION);
        enc.write_wstring(&self.header.name)?;
        enc.write_bool(self.header.sex.into());
        enc.write_string(&self.header.class_tag)?;
        enc.write_u32(self.header.level);
        enc.write_bool(self.header.hardcore);
        enc.write_u8(self.header.expansion_status);
        enc.write_zero_marker();
        enc.write_u32(self.header.data_version);
        enc.write_bytes(&self.header.uid);
        self.blocks
            .iter()
            .try_for_each(|block| block.write(&mut enc))?;
        Ok(enc.finish())
    }

    /// The raw cipher seed the file was written with.
    #[must_use]
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// The fixed header.
    #[must_use]
    pub fn header(&self) -> &PlayerHeader {
        &self.header
    }

    /// The fixed header for editing; `level` duplicates block 2's and
    /// `class_tag` is derived from block 8's masteries, so an edit
    /// here keeps them in step.
    pub fn header_mut(&mut self) -> &mut PlayerHeader {
        &mut self.header
    }

    /// The character's name.
    #[must_use]
    pub fn character_name(&self) -> &str {
        &self.header.name
    }

    /// The character's level.
    #[must_use]
    pub fn level(&self) -> u32 {
        self.header.level
    }

    /// The blocks in file order.
    #[must_use]
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Whether every block is typed, so any edit can be written.
    #[must_use]
    pub fn is_fully_typed(&self) -> bool {
        !self.blocks.iter().any(Block::is_opaque)
    }

    block_accessor!(
        /// Block 1, when typed.
        character_info, character_info_mut: CharacterInfo(CharacterInfo)
    );
    block_accessor!(
        /// Block 2, when typed.
        bio, bio_mut: Bio(Bio)
    );
    block_accessor!(
        /// Block 3, when typed.
        inventory, inventory_mut: Inventory(Inventory)
    );
    block_accessor!(
        /// Block 4, when typed.
        stash, stash_mut: Stash(PlayerStash)
    );
    block_accessor!(
        /// Block 5, when typed.
        respawns: Respawns(Respawns)
    );
    block_accessor!(
        /// Block 6, when typed.
        teleports: Teleports(Teleports)
    );
    block_accessor!(
        /// Block 7, when typed.
        markers: Markers(Markers)
    );
    block_accessor!(
        /// Block 17, when typed.
        shrines: Shrines(Shrines)
    );
    block_accessor!(
        /// Block 8, when typed.
        skills, skills_mut: Skills(Skills)
    );
    block_accessor!(
        /// Block 12, when typed.
        lore_notes: LoreNotes(LoreNotes)
    );
    block_accessor!(
        /// Block 13, when typed.
        factions: Factions(Factions)
    );
    block_accessor!(
        /// Block 14, when typed.
        ui: Ui(Ui)
    );
    block_accessor!(
        /// Block 15, when typed.
        tutorials: Tutorials(Tutorials)
    );
    block_accessor!(
        /// Block 16, when typed.
        stats: Stats(Stats)
    );
    block_accessor!(
        /// Block 10, when typed.
        tokens: Tokens(Tokens)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::TabDecoration;
    use crate::item::StashItem;

    fn item(name: &str) -> Item {
        Item {
            base_name: name.into(),
            stack_count: 1,
            ..Item::default()
        }
    }

    fn sample() -> PlayerFile {
        let version = ContainerVersion::new(11).unwrap();
        let mut equipment: [EquippedItem; EQUIPMENT_SLOTS] = Default::default();
        equipment[0].item = item("records/items/gearhead/h.dbr");
        PlayerFile {
            seed: 0x0BAD_F00D,
            header: PlayerHeader {
                name: "Sif".into(),
                sex: Sex::Female,
                class_tag: "tagSkillClassName0306".into(),
                level: 6,
                hardcore: false,
                expansion_status: 7,
                data_version: 8,
                uid: [0; 16],
            },
            blocks: vec![
                Block::CharacterInfo(CharacterInfo {
                    is_in_main_quest: true,
                    has_been_in_game: true,
                    difficulty: 0,
                    greatest_difficulty: 0,
                    money: 1234,
                    greatest_survival_difficulty: 0,
                    current_tribute: 0,
                    compass_state: 3,
                    skill_window_show_help: 1,
                    weapon_swap_active: 0,
                    weapon_swap_enabled: 1,
                    texture: "creatures/pc/hero02.tex".into(),
                    loot_filter: vec![1; 42],
                }),
                Block::Inventory(Inventory {
                    version,
                    flag: 0,
                    state: InventoryState::Entered(Box::new(InventoryContents {
                        focused_sack: 0,
                        selected_sack: 0,
                        sacks: vec![Sack {
                            flag: 1,
                            items: vec![SackItem {
                                item: item("records/items/gearweapons/w.dbr"),
                                x: 1,
                                y: 2,
                            }],
                        }],
                        use_alternate: 0,
                        equipment,
                        alternate_1: 0,
                        weapon_set_1: Default::default(),
                        alternate_2: 0,
                        weapon_set_2: Default::default(),
                    })),
                }),
                Block::Stash(PlayerStash {
                    version,
                    tabs: vec![StashTab {
                        width: 8,
                        height: 16,
                        items: vec![StashItem {
                            item: item("records/items/materia/m.dbr"),
                            x: 0.0,
                            y: 1.0,
                        }],
                        decoration: TabDecoration::default(),
                    }],
                }),
            ],
        }
    }

    #[test]
    fn typed_blocks_round_trip_through_bytes() {
        let file = sample();
        let bytes = file.encode().unwrap();
        let parsed = PlayerFile::parse(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(parsed.encode().unwrap(), bytes);
        assert_eq!(parsed.character_name(), "Sif");
        assert_eq!(parsed.level(), 6);
        assert_eq!(parsed.inventory().unwrap().sacks()[0].items.len(), 1);
        assert_eq!(parsed.inventory().unwrap().equipped().count(), 1);
        assert_eq!(parsed.stash().unwrap().tabs.len(), 1);
        assert_eq!(parsed.character_info().unwrap().money, 1234);
    }

    #[test]
    fn every_slot_names_one_place_in_the_inventory() {
        let mut file = sample();
        let contents = file.inventory_mut().unwrap().contents_mut().unwrap();
        for (index, slot) in EquipSlot::ALL.into_iter().enumerate() {
            contents.slot_mut(slot).item = item(&format!("records/items/{index}.dbr"));
        }
        assert_eq!(
            contents
                .equipment
                .iter()
                .map(|worn| worn.item.base_name.as_str())
                .collect::<Vec<_>>(),
            (0..12)
                .map(|index| format!("records/items/{index}.dbr"))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            contents.weapon_set_1[0].item.base_name,
            "records/items/12.dbr"
        );
        assert_eq!(
            contents.weapon_set_1[1].item.base_name,
            "records/items/13.dbr"
        );
        assert_eq!(
            contents.weapon_set_2[0].item.base_name,
            "records/items/14.dbr"
        );
        assert_eq!(
            contents.weapon_set_2[1].item.base_name,
            "records/items/15.dbr"
        );
        assert_eq!(contents.equipped().count(), 16);
        assert_eq!(
            contents.slots().map(|(slot, _)| slot).collect::<Vec<_>>(),
            EquipSlot::ALL
        );
        assert_eq!(
            EquipSlot::hands(WeaponSet::Second),
            [EquipSlot::MainHand2, EquipSlot::OffHand2]
        );
        assert_eq!(EquipSlot::Belt.weapon_set(), None);
        assert_eq!(EquipSlot::OffHand1.weapon_set(), Some(WeaponSet::First));
        assert_eq!(EquipSlot::OffHand1.to_string(), "Off hand (weapon set 1)");
        assert_eq!(EquipSlot::Ring2.to_string(), "Ring 2");
        assert_eq!(contents.active_weapon_set(), WeaponSet::First);
        contents.use_alternate = 1;
        assert_eq!(contents.active_weapon_set(), WeaponSet::Second);
    }

    #[test]
    fn an_emptied_slot_round_trips_as_the_games_empty_slot() {
        let mut file = sample();
        let contents = file.inventory_mut().unwrap().contents_mut().unwrap();
        assert!(!contents.slot(EquipSlot::Head).item.is_empty());
        *contents.slot_mut(EquipSlot::Head) = EquippedItem::empty();
        assert!(contents.slot(EquipSlot::Head).item.is_empty());
        assert_eq!(contents.slot(EquipSlot::Head).item.stack_count, 1);
        assert_eq!(contents.slot(EquipSlot::Head).attached, 0);
        assert_eq!(contents.equipped().count(), 0);
        let bytes = file.encode().unwrap();
        let parsed = PlayerFile::parse(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(
            parsed
                .inventory()
                .unwrap()
                .contents()
                .unwrap()
                .slot(EquipSlot::Head),
            &EquippedItem::empty()
        );
    }

    #[test]
    fn never_entered_inventory_round_trips() {
        let mut file = sample();
        file.blocks[1] = Block::Inventory(Inventory {
            version: ContainerVersion::new(4).unwrap(),
            flag: 0,
            state: InventoryState::NeverEntered,
        });
        let bytes = file.encode().unwrap();
        let parsed = PlayerFile::parse(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert!(parsed.inventory().unwrap().sacks().is_empty());
    }

    #[test]
    fn unsupported_versions_fall_back_to_opaque() {
        let mut enc = Encoder::new(5);
        enc.write_u32(MAGIC);
        enc.write_u32(HEADER_VERSION);
        enc.write_wstring("X").unwrap();
        enc.write_bool(false);
        enc.write_string("").unwrap();
        enc.write_u32(1);
        enc.write_bool(false);
        enc.write_u8(7);
        enc.write_zero_marker();
        enc.write_u32(8);
        enc.write_bytes(&[0; 16]);
        enc.write_block(BlockId::CHARACTER_INFO, |enc| {
            enc.write_u32(6);
            enc.write_bytes(&[1, 2, 3]);
            Ok::<(), crate::crypto::EncodeError>(())
        })
        .unwrap();
        enc.write_block(BlockId::INVENTORY, |enc| {
            enc.write_u32(12);
            enc.write_u8(0);
            Ok::<(), crate::crypto::EncodeError>(())
        })
        .unwrap();
        enc.write_block(BlockId::new(16), |enc| {
            enc.write_u32(1);
            enc.write_string("records/x").unwrap();
            Ok::<(), crate::crypto::EncodeError>(())
        })
        .unwrap();
        enc.write_block(BlockId::new(99), |enc| {
            enc.write_u32(1);
            enc.write_string("records/y").unwrap();
            Ok::<(), crate::crypto::EncodeError>(())
        })
        .unwrap();
        let bytes = enc.finish();

        let file = PlayerFile::parse(&bytes).unwrap();
        let reasons: Vec<_> = file
            .blocks()
            .iter()
            .map(|block| {
                if let Block::Opaque(opaque) = block {
                    Some(opaque.reason())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            reasons,
            vec![
                Some(OpaqueReason::UnsupportedVersion { version: 6 }),
                Some(OpaqueReason::UnsupportedVersion { version: 12 }),
                Some(OpaqueReason::UnsupportedVersion { version: 1 }),
                Some(OpaqueReason::Unmodeled),
            ]
        );
        assert!(!file.is_fully_typed());
        assert!(file.inventory().is_none());
        assert!(file.stats().is_none());
        assert_eq!(file.encode().unwrap(), bytes);
    }

    #[test]
    fn bad_magic_is_reported() {
        let mut enc = Encoder::new(1);
        enc.write_u32(0x1234);
        assert_eq!(
            PlayerFile::parse(&enc.finish()),
            Err(GdcError::BadMagic { found: 0x1234 })
        );
    }

    #[test]
    fn editing_before_an_opaque_block_is_refused() {
        let mut file = sample();
        let mut enc = Encoder::new(1);
        enc.write_block(BlockId::new(9), |enc| {
            enc.write_u32(1);
            Ok::<(), crate::crypto::EncodeError>(())
        })
        .unwrap();
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes).unwrap();
        file.blocks.push(Block::Opaque(
            OpaqueBlock::read(&mut dec, OpaqueReason::Unmodeled).unwrap(),
        ));
        assert!(matches!(
            file.encode(),
            Err(SaveEncodeError::OpaqueRekeyed { block, .. }) if block == BlockId::new(9)
        ));
    }

    #[test]
    fn realm_dir_names_round_trip() {
        for realm in Realm::ALL {
            assert_eq!(Realm::parse_dir_name(realm.dir_name()), Some(realm));
        }
        assert_eq!(Realm::Main.dir_name(), "main");
        assert_eq!(Realm::Custom.dir_name(), "user");
        assert_eq!(Realm::parse_dir_name("mods"), None);
    }

    #[test]
    fn realms_serialize_as_camel_case_tags() {
        assert_eq!(serde_json::to_string(&Realm::Main).unwrap(), "\"main\"");
        assert_eq!(
            serde_json::from_str::<Realm>("\"custom\"").unwrap(),
            Realm::Custom
        );
    }
}
