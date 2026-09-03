//! Lossless-model gate: the vendored gdlc fixture (see
//! `fixtures/FIXTURES.md`) and randomly generated block sequences must
//! re-encode byte-for-byte, and the single-byte cipher order must be the
//! one that makes the fixture's booleans booleans.

use grimvault_core::block::{OpaqueElement, OpaqueReason};
use grimvault_core::crypto::{BlockId, Decoder, EncodeError, Encoder, KeyTable};
use grimvault_core::gdc::{Block, InventoryState, PlayerFile, Sex};
use grimvault_core::gst::GstFile;

const FIXTURE: &[u8] = include_bytes!("fixtures/v11_player.gdc");

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
    assert_eq!(
        ids,
        vec![1, 2, 3, 4, 5, 6, 7, 17, 8, 12, 13, 14, 15, 16, 10]
    );
    let typed: Vec<bool> = file
        .blocks()
        .iter()
        .map(|b| !matches!(b, Block::Opaque(_)))
        .collect();
    assert_eq!(
        typed,
        [true, false, true, true]
            .into_iter()
            .chain(std::iter::repeat_n(false, 11))
            .collect::<Vec<_>>()
    );

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
fn fixture_opaque_blocks_are_flat_and_verified() {
    let file = PlayerFile::parse(FIXTURE).unwrap();
    for block in file.blocks() {
        if let Block::Opaque(opaque) = block {
            assert_eq!(opaque.reason(), OpaqueReason::Unmodeled);
            assert!(
                matches!(opaque.body(), [OpaqueElement::Bytes(_)]),
                "{} should be one flat byte run",
                opaque.id()
            );
        }
    }
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
