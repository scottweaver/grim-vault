//! Lossless-model gate: the vendored gdlc fixture (see
//! `fixtures/FIXTURES.md`) and randomly generated block sequences must
//! re-encode byte-for-byte, the single-byte cipher order must be the
//! one that makes the fixture's booleans booleans, and — the point of
//! typing every block — an edited inventory or stash must survive
//! encode → parse with every later block intact.
//!
//! The same edit checks run over every `main/_*/player.gdc` under
//! `$GRIMVAULT_SAVE_DIR` when that variable names a directory (a
//! **copy** of a save directory, never the live one); the test passes
//! vacuously when it is unset.

use std::path::PathBuf;

use grimvault_core::blocks::skills::SkillsVersion;
use grimvault_core::blocks::stats::StatsVersion;
use grimvault_core::blocks::ui::UiVersion;
use grimvault_core::crypto::{BlockId, Decoder, EncodeError, Encoder, KeyTable};
use grimvault_core::gdc::{InventoryState, PlayerFile, Sex};
use grimvault_core::gst::GstFile;
use grimvault_core::item::StashItem;

const FIXTURE: &[u8] = include_bytes!("fixtures/v11_player.gdc");
const EXPECTED_BLOCK_IDS: [u32; 15] = [1, 2, 3, 4, 5, 6, 7, 17, 8, 12, 13, 14, 15, 16, 10];

#[test]
fn fixture_re_encodes_byte_identically() {
    let file = PlayerFile::parse(FIXTURE).unwrap();
    assert_eq!(file.encode().unwrap(), FIXTURE);
}

#[test]
fn fixture_header_and_typed_blocks_are_read() {
    let file = PlayerFile::parse(FIXTURE).unwrap();
    assert_eq!(file.character_name(), "Laurana");
    assert_eq!(file.level(), 100);
    assert_eq!(file.header().class_tag, "tagSkillClassName0506");
    assert_eq!(file.header().sex, Sex::Female);
    assert!(!file.header().hardcore);
    assert_eq!(file.header().expansion_status, 7);
    assert_eq!(file.header().data_version, 8);

    let ids: Vec<u32> = file.blocks().iter().map(|b| b.id().raw()).collect();
    assert_eq!(ids, EXPECTED_BLOCK_IDS);
    assert!(file.is_fully_typed());

    let info = file.character_info().unwrap();
    assert_eq!(info.money, 1_069_109);
    assert!(info.has_been_in_game);

    let inventory = file.inventory().unwrap();
    assert_eq!(inventory.version.raw(), 11);
    let InventoryState::Entered(contents) = &inventory.state else {
        panic!("fixture character has entered the game");
    };
    assert_eq!(contents.sacks.len(), 6);
    let counts: Vec<usize> = contents.sacks.iter().map(|s| s.items.len()).collect();
    assert_eq!(counts, vec![55, 26, 28, 19, 28, 27]);
    assert_eq!(
        contents.sacks[0].items[0].item.base_name,
        "records/items/gearaccessories/medals/b104e_medal.dbr"
    );
    assert_eq!(inventory.equipped().count(), 14);

    let stash = file.stash().unwrap();
    assert_eq!(stash.version.raw(), 11);
    assert_eq!(stash.tabs.len(), 8);
    assert_eq!(stash.tabs[0].items.len(), 68);
    assert_eq!((stash.tabs[0].width, stash.tabs[0].height), (10, 19));
}

#[test]
fn fixture_later_blocks_carry_their_contents() {
    let file = PlayerFile::parse(FIXTURE).unwrap();
    let bio = file.bio().unwrap();
    assert_eq!(bio.level, 100);
    assert!(bio.experience > 0);
    assert!(bio.physique >= 50.0 && bio.cunning >= 50.0 && bio.spirit >= 50.0);

    let skills = file.skills().unwrap();
    assert_eq!(skills.version(), SkillsVersion::V8);
    assert!(!skills.skills.is_empty());
    assert!(
        skills
            .skills
            .iter()
            .all(|skill| skill.name.starts_with("records/skills/"))
    );

    let stats = file.stats().unwrap();
    assert_eq!(stats.version(), StatsVersion::V12);
    assert_eq!(stats.max_level, 100);
    assert!(stats.playtime > 0);

    let ui = file.ui().unwrap();
    assert_eq!(ui.version(), UiVersion::V7);
    assert!(ui.hotbars.slots().count() > 0);

    assert!(file.respawns().is_some());
    assert!(file.teleports().is_some());
    assert!(file.markers().is_some());
    assert!(file.shrines().is_some());
    assert!(file.lore_notes().is_some());
    assert!(!file.factions().unwrap().factions.is_empty());
    assert!(file.tutorials().is_some());
    assert!(file.tokens().is_some());
}

/// The three edits that change the key for every block after the
/// inventory; each returns `false` when the file has nothing to edit.
type Edit = fn(&mut PlayerFile) -> bool;

const EDITS: [(&str, Edit); 3] = [
    ("remove one sack item", remove_sack_item),
    ("add one sack item", add_sack_item),
    ("move one sack item to stash tab 0", move_sack_item_to_stash),
];

fn remove_sack_item(file: &mut PlayerFile) -> bool {
    file.inventory_mut()
        .and_then(|inventory| {
            inventory
                .sacks_mut()
                .iter_mut()
                .find(|sack| !sack.items.is_empty())
                .map(|sack| sack.items.remove(0))
        })
        .is_some()
}

fn add_sack_item(file: &mut PlayerFile) -> bool {
    let Some(sack) = file.inventory_mut().and_then(|inventory| {
        inventory
            .sacks_mut()
            .iter_mut()
            .find(|sack| !sack.items.is_empty())
    }) else {
        return false;
    };
    let mut copy = sack.items[0].clone();
    copy.x += 1;
    sack.items.push(copy);
    true
}

fn move_sack_item_to_stash(file: &mut PlayerFile) -> bool {
    let Some(taken) = file.inventory_mut().and_then(|inventory| {
        inventory
            .sacks_mut()
            .iter_mut()
            .find(|sack| !sack.items.is_empty())
            .map(|sack| sack.items.remove(0))
    }) else {
        return false;
    };
    let Some(tab) = file.stash_mut().and_then(|stash| stash.tabs.first_mut()) else {
        return false;
    };
    tab.items.push(StashItem {
        item: taken.item,
        x: 0.0,
        y: 0.0,
    });
    true
}

/// Applies every applicable edit to `file` and asserts encode → parse
/// reproduces the edited model exactly; returns how many applied.
fn assert_edits_survive_re_encode(file: &PlayerFile, original: &[u8], label: &str) -> usize {
    let mut applied = 0;
    for (edit_name, edit) in EDITS {
        let mut edited = file.clone();
        if !edit(&mut edited) {
            continue;
        }
        applied += 1;
        assert_ne!(&edited, file, "{label}: `{edit_name}` changed nothing");
        let bytes = edited
            .encode()
            .unwrap_or_else(|error| panic!("{label}: `{edit_name}`: encode: {error}"));
        assert_ne!(bytes, original, "{label}: `{edit_name}` bytes unchanged");
        let parsed = PlayerFile::parse(&bytes)
            .unwrap_or_else(|error| panic!("{label}: `{edit_name}`: re-parse: {error}"));
        assert_eq!(parsed, edited, "{label}: `{edit_name}` did not survive");
        assert_eq!(parsed.blocks().len(), file.blocks().len());
        assert!(parsed.is_fully_typed());
    }
    applied
}

#[test]
fn fixture_edits_survive_re_encode() {
    let file = PlayerFile::parse(FIXTURE).unwrap();
    assert_eq!(
        assert_edits_survive_re_encode(&file, FIXTURE, "fixture"),
        EDITS.len()
    );
}

#[test]
fn real_saves_round_trip_and_survive_edits() {
    let Some(save_dir) = std::env::var_os("GRIMVAULT_SAVE_DIR").map(PathBuf::from) else {
        return;
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(save_dir.join("main"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path().join("player.gdc"))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "no main/_*/player.gdc under the save dir"
    );
    let mut applied = 0;
    for path in paths {
        let label = path.display().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let file = PlayerFile::parse(&bytes).unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(
            file.encode().unwrap(),
            bytes,
            "{label}: unmodified round trip"
        );
        let ids: Vec<u32> = file.blocks().iter().map(|b| b.id().raw()).collect();
        assert_eq!(ids, EXPECTED_BLOCK_IDS, "{label}: block sequence");
        assert!(file.is_fully_typed(), "{label}: an opaque block remains");
        applied += assert_edits_survive_re_encode(&file, &bytes, &label);
    }
    assert!(applied > 0, "no save had a sack item to edit");
}

fn read_header_bools_update_first(bytes: &[u8]) -> (u8, u8) {
    let seed = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let table = KeyTable::from_seed(seed);
    let mut key = seed ^ 0x5555_5555;
    let mut pos = 4;
    let read_u32 = |key: &mut u32, pos: &mut usize| {
        let raw = u32::from_le_bytes([
            bytes[*pos],
            bytes[*pos + 1],
            bytes[*pos + 2],
            bytes[*pos + 3],
        ]);
        *pos += 4;
        let plain = raw ^ *key;
        for b in raw.to_le_bytes() {
            *key ^= table.entries()[usize::from(b)];
        }
        plain
    };
    let read_u8_gdlc = |key: &mut u32, pos: &mut usize| {
        let raw = bytes[*pos];
        *pos += 1;
        *key ^= table.entries()[usize::from(raw)];
        raw ^ key.to_le_bytes()[0]
    };
    read_u32(&mut key, &mut pos);
    read_u32(&mut key, &mut pos);
    let units = read_u32(&mut key, &mut pos);
    for _ in 0..units * 2 {
        read_u8_gdlc(&mut key, &mut pos);
    }
    let sex = read_u8_gdlc(&mut key, &mut pos);
    let class_len = read_u32(&mut key, &mut pos);
    for _ in 0..class_len {
        read_u8_gdlc(&mut key, &mut pos);
    }
    read_u32(&mut key, &mut pos);
    let hardcore = read_u8_gdlc(&mut key, &mut pos);
    (sex, hardcore)
}

#[test]
fn single_byte_order_is_xor_then_update() {
    let mut dec = Decoder::new(FIXTURE).unwrap();
    dec.read_u32().unwrap();
    dec.read_u32().unwrap();
    dec.read_wstring().unwrap();
    let sex = dec.read_u8().unwrap();
    dec.read_string().unwrap();
    dec.read_u32().unwrap();
    let hardcore = dec.read_u8().unwrap();
    assert!(
        sex <= 1 && hardcore <= 1,
        "documented order: {sex} {hardcore}"
    );

    let (sex, hardcore) = read_header_bools_update_first(FIXTURE);
    assert!(
        sex > 1 && hardcore > 1,
        "gdlc's update-first order must not yield booleans: {sex} {hardcore}"
    );
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        let [_, _, b2, b3, b4, b5, _, _] = self.0.to_le_bytes();
        u32::from_le_bytes([b2, b3, b4, b5])
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }

    fn byte(&mut self) -> u8 {
        self.next().to_le_bytes()[0]
    }
}

fn write_random_body(enc: &mut Encoder, rng: &mut Rng, depth: u32) -> Result<(), EncodeError> {
    for _ in 0..rng.below(12) {
        match rng.below(6) {
            0 => enc.write_u32(0),
            1 => enc.write_u32(rng.next()),
            2 => {
                let run: Vec<u8> = (0..rng.below(24)).map(|_| rng.byte()).collect();
                enc.write_bytes(&run);
            }
            3 => enc.write_string("records/items/lootaffixes/prefix/x.dbr")?,
            4 if depth < 2 => {
                let id = BlockId::new(rng.below(3));
                enc.write_block(id, |enc| write_random_body(enc, rng, depth + 1))?;
            }
            _ => enc.write_u8(rng.byte()),
        }
    }
    Ok(())
}

fn random_gst_image(rng: &mut Rng) -> Vec<u8> {
    let mut enc = Encoder::new(rng.next());
    enc.write_u32(1 + rng.below(2));
    for _ in 0..=rng.below(4) {
        let id = BlockId::new(19 + rng.below(20));
        enc.write_block(id, |enc| {
            enc.write_u32(1 + rng.below(3));
            if rng.below(2) == 0 {
                enc.write_zero_marker();
            }
            write_random_body(enc, rng, 0)
        })
        .unwrap();
    }
    enc.finish()
}

#[test]
fn random_block_sequences_round_trip_opaquely() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..200 {
        let bytes = random_gst_image(&mut rng);
        let file = GstFile::parse(&bytes).unwrap_or_else(|e| panic!("{e}: {bytes:02x?}"));
        assert!(file.transfer_stash().is_none());
        assert_eq!(file.encode().unwrap(), bytes);
    }
}

#[test]
fn fixture_passes_the_lossless_gate_at_load() {
    let loaded = grimvault_core::loaded::Loaded::<PlayerFile>::load(FIXTURE.to_vec()).unwrap();
    assert_eq!(loaded.baseline(), FIXTURE);
    assert_eq!(loaded.model().character_name(), "Laurana");
    assert_eq!(loaded.encode().unwrap(), FIXTURE);
}
