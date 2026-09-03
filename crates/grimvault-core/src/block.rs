//! Block-level building blocks shared by `player.gdc` and `*.gst`:
//! the top-level block walker, opaque (unmodeled) blocks, and stash tabs.
//!
//! Stash-tab layout ported from gdlc (MIT, dandels 2025), `src/stash.rs`.
//!
//! # Opaque blocks and the lossless model
//!
//! An unmodeled block is carried as its byte-decoded body so the file
//! re-encodes byte-for-byte. Two facts make this more than a flat byte
//! copy:
//!
//! 1. Nested blocks inside the body carry static length and checksum
//!    words that never feed the key, so a flat byte decode diverges
//!    from the writer's key sequence and fails the enclosing checksum.
//!    The reader therefore detects nested blocks structurally: a
//!    candidate header whose declared length fits and whose checksum
//!    verifies against the running key is a block (a 32-bit
//!    coincidence otherwise), everything else is bytes. A static zero
//!    marker directly after a block's version word (the shape of
//!    blocks 18, 19 and 20) likewise never feeds the key; a body whose
//!    flat decode fails its checksum is retried with that shape.
//! 2. A byte decoded against key `K` is only *semantically* stable
//!    while re-encoded against the same `K`: the game XORs a `u32`'s
//!    upper three bytes against the key's upper bytes, a byte-wise
//!    decode against the low byte of a key that has already advanced.
//!    Without the block's field layout the two cannot be told apart, so
//!    an opaque body re-keyed by an edit to an earlier block would
//!    decode differently in-game while looking valid. Each opaque block
//!    records the key it was read under and refuses to encode under any
//!    other ([`SaveEncodeError::OpaqueRekeyed`]) — a loud failure in
//!    place of silent corruption. Editing a typed block that precedes
//!    an opaque one is therefore not yet writable; that requires typing
//!    every block after the edit point.

use thiserror::Error;

use crate::blocks::skills::{SkillField, SkillsVersion};
use crate::crypto::{BlockId, DecodeError, Decoder, EncodeError, Encoder};
use crate::item::{ContainerVersion, ItemEncodeError, StashItem};

/// Why a save file could not be written.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SaveEncodeError {
    /// Cipher-level failure.
    #[error(transparent)]
    Cipher(#[from] EncodeError),
    /// An item refused the container version.
    #[error(transparent)]
    Item(#[from] ItemEncodeError),
    /// A decorated stash tab was written at a version without decoration.
    #[error("stash tab decoration is set but container {version} does not carry it")]
    DecorationNotInVersion {
        /// The version being written.
        version: ContainerVersion,
    },
    /// An opaque block would be encoded under a different key than it
    /// was read under (an earlier block changed); see the module docs.
    #[error(
        "opaque {block} was read under key {read_under:#010x} but would be written under {would_write_under:#010x}"
    )]
    OpaqueRekeyed {
        /// The block that cannot be re-keyed.
        block: BlockId,
        /// Key state when the block was read.
        read_under: u32,
        /// Key state at the point the block was about to be written.
        would_write_under: u32,
    },
    /// A skill carries a field its block version has no slot for
    /// (`blocks::skills`).
    #[error("skill field {field} is set but block 8 {version} does not carry it")]
    SkillFieldNotInVersion {
        /// The field that would be lost.
        field: SkillField,
        /// The version being written.
        version: SkillsVersion,
    },
    /// A v7 UI hotbar set holds a different number of slots than the
    /// block declares per set (`blocks::ui`).
    #[error("hotbar set {set} holds {found} slots but the block declares {expected} per set")]
    HotbarSetSize {
        /// Index of the offending set.
        set: usize,
        /// Slots the block declares per set.
        expected: u32,
        /// Slots the set holds.
        found: usize,
    },
}

/// Why a block was carried opaquely rather than typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpaqueReason {
    /// This crate models no block with this id.
    Unmodeled,
    /// The id is modeled but the block's version is not.
    UnsupportedVersion {
        /// The version read from the block.
        version: u32,
    },
}

/// One structural element of an opaque body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpaqueElement {
    /// A run of byte-decoded plaintext.
    Bytes(Vec<u8>),
    /// A nested framed block.
    Block {
        /// The nested block's id.
        id: BlockId,
        /// The nested block's body.
        body: Vec<OpaqueElement>,
    },
    /// A static zero word that does not feed the key
    /// ([`Decoder::read_zero_marker`]).
    ZeroMarker,
}

/// A block preserved without interpretation; see the module docs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpaqueBlock {
    id: BlockId,
    reason: OpaqueReason,
    read_under: u32,
    body: Vec<OpaqueElement>,
}

impl OpaqueBlock {
    /// The block id.
    #[must_use]
    pub fn id(&self) -> BlockId {
        self.id
    }

    /// Why the block is opaque.
    #[must_use]
    pub fn reason(&self) -> OpaqueReason {
        self.reason
    }

    /// The structural body.
    #[must_use]
    pub fn body(&self) -> &[OpaqueElement] {
        &self.body
    }

    /// Ids of the nested blocks directly inside the body, in order.
    pub fn nested_ids(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.body.iter().filter_map(|e| match e {
            OpaqueElement::Block { id, .. } => Some(*id),
            OpaqueElement::Bytes(_) | OpaqueElement::ZeroMarker => None,
        })
    }

    /// Reads a whole block (header through checksum) starting at the
    /// cursor, which must sit on the block's id word.
    ///
    /// # Errors
    /// Cipher / framing errors, including a checksum failure when the
    /// body's structure could not be recovered.
    pub fn read(dec: &mut Decoder<'_>, reason: OpaqueReason) -> Result<Self, DecodeError> {
        let read_under = dec.key();
        let start = dec.read_block_start()?;
        let body = read_framed_body(dec)?;
        Ok(Self {
            id: start.id,
            reason,
            read_under,
            body,
        })
    }

    /// Writes the block, refusing if the key differs from the one it
    /// was read under.
    ///
    /// # Errors
    /// [`SaveEncodeError::OpaqueRekeyed`], or cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        let would_write_under = enc.key();
        if would_write_under != self.read_under {
            return Err(SaveEncodeError::OpaqueRekeyed {
                block: self.id,
                read_under: self.read_under,
                would_write_under,
            });
        }
        write_block(enc, self.id, &self.body)
    }
}

/// Reads an open block's body through its checksum: flat first, then
/// retried with a zero marker after the first word; the flat attempt's
/// error stands if neither shape verifies.
fn read_framed_body(dec: &mut Decoder<'_>) -> Result<Vec<OpaqueElement>, DecodeError> {
    let checkpoint = dec.checkpoint();
    let flat_error = match read_body(dec).and_then(|body| dec.read_block_end().map(|()| body)) {
        Ok(body) => return Ok(body),
        Err(error) => error,
    };
    dec.restore(checkpoint);
    read_body_with_marker(dec).map_err(|_| flat_error)
}

fn read_body_with_marker(dec: &mut Decoder<'_>) -> Result<Vec<OpaqueElement>, DecodeError> {
    let first_word = dec.read_bytes(4)?;
    dec.read_zero_marker()?;
    let mut body = vec![OpaqueElement::Bytes(first_word), OpaqueElement::ZeroMarker];
    body.extend(read_body(dec)?);
    dec.read_block_end()?;
    Ok(body)
}

fn read_body(dec: &mut Decoder<'_>) -> Result<Vec<OpaqueElement>, DecodeError> {
    let mut elements = Vec::new();
    let mut run = Vec::new();
    while !dec.is_at_end() {
        if let Some(block) = try_read_nested_block(dec)? {
            flush_run(&mut run, &mut elements);
            elements.push(block);
        } else {
            run.push(dec.read_u8()?);
        }
    }
    flush_run(&mut run, &mut elements);
    Ok(elements)
}

fn flush_run(run: &mut Vec<u8>, elements: &mut Vec<OpaqueElement>) {
    if !run.is_empty() {
        elements.push(OpaqueElement::Bytes(std::mem::take(run)));
    }
}

const BLOCK_FRAMING_LEN: usize = 4 + 4 + 4;

fn try_read_nested_block(dec: &mut Decoder<'_>) -> Result<Option<OpaqueElement>, DecodeError> {
    let Some(candidate) = dec.peek_block_start() else {
        return Ok(None);
    };
    if candidate.len as usize + BLOCK_FRAMING_LEN > dec.remaining() {
        return Ok(None);
    }
    let checkpoint = dec.checkpoint();
    dec.read_block_start()?;
    if let Ok(body) = read_framed_body(dec) {
        Ok(Some(OpaqueElement::Block {
            id: candidate.id,
            body,
        }))
    } else {
        dec.restore(checkpoint);
        Ok(None)
    }
}

fn write_block(
    enc: &mut Encoder,
    id: BlockId,
    body: &[OpaqueElement],
) -> Result<(), SaveEncodeError> {
    enc.write_block(id, |enc| write_elements(enc, body))
}

fn write_elements(enc: &mut Encoder, body: &[OpaqueElement]) -> Result<(), SaveEncodeError> {
    body.iter().try_for_each(|element| match element {
        OpaqueElement::Bytes(bytes) => {
            enc.write_bytes(bytes);
            Ok(())
        }
        OpaqueElement::Block { id, body } => write_block(enc, *id, body),
        OpaqueElement::ZeroMarker => {
            enc.write_zero_marker();
            Ok(())
        }
    })
}

/// The header of a top-level block as seen by a [`read_block`]
/// dispatcher: id, declared body length, and the version word every
/// top-level block opens with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockHeader {
    /// The block id.
    pub id: BlockId,
    /// Declared body length in bytes, including the version word.
    pub len: u32,
    /// The block's version (its first body word).
    pub version: u32,
}

/// A dispatcher's verdict on a block: a typed body it has fully read,
/// or an instruction to re-read the block opaquely.
#[derive(Debug)]
pub enum Dispatch<T> {
    /// The dispatcher consumed the body (after the version word) and
    /// produced this value; the walker closes the block.
    Typed(T),
    /// Re-read the whole block opaquely for this reason.
    Opaque(OpaqueReason),
}

/// Reads one top-level block. `dispatch` sees the header (version
/// already consumed) and either reads the rest of the body or asks for
/// an opaque read; `wrap_opaque` lifts the opaque block into `T`. A
/// block too short to hold a version word is always opaque.
///
/// # Errors
/// Whatever `dispatch` returns, or cipher / framing errors.
pub fn read_block<T, E>(
    dec: &mut Decoder<'_>,
    dispatch: impl FnOnce(&mut Decoder<'_>, BlockHeader) -> Result<Dispatch<T>, E>,
    wrap_opaque: impl FnOnce(OpaqueBlock) -> T,
) -> Result<T, E>
where
    E: From<DecodeError>,
{
    let checkpoint = dec.checkpoint();
    let start = dec.read_block_start()?;
    let verdict = if start.len >= 4 {
        let header = BlockHeader {
            id: start.id,
            len: start.len,
            version: dec.read_u32()?,
        };
        dispatch(dec, header)?
    } else {
        Dispatch::Opaque(OpaqueReason::Unmodeled)
    };
    match verdict {
        Dispatch::Typed(value) => {
            dec.read_block_end()?;
            Ok(value)
        }
        Dispatch::Opaque(reason) => {
            dec.restore(checkpoint);
            Ok(wrap_opaque(OpaqueBlock::read(dec, reason)?))
        }
    }
}

/// Tab decoration carried from [`ContainerVersion::has_tab_decoration`]
/// on; all-default before that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabDecoration {
    /// Border style index.
    pub border_index: u32,
    /// Border colour index.
    pub border_color_index: u32,
    /// Symbol index.
    pub symbol_index: u32,
    /// Symbol colour index.
    pub symbol_color_index: u32,
    /// The tab's button label.
    pub button_name: String,
}

impl TabDecoration {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// One stash tab: a grid of `width` × `height` cells and its items.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StashTab {
    /// Grid width in cells.
    pub width: u32,
    /// Grid height in cells.
    pub height: u32,
    /// Items with their cell positions.
    pub items: Vec<StashItem>,
    /// Version-gated decoration.
    pub decoration: TabDecoration,
}

impl StashTab {
    /// Reads a nested tab block (id 0) at the cursor.
    ///
    /// # Errors
    /// Cipher / framing errors.
    pub fn read(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        dec.read_block_start_expecting(BlockId::NESTED)?;
        let width = dec.read_u32()?;
        let height = dec.read_u32()?;
        let count = dec.read_u32()?;
        let items = (0..count)
            .map(|_| StashItem::read(dec, version))
            .collect::<Result<Vec<_>, _>>()?;
        let decoration = if version.has_tab_decoration() {
            TabDecoration {
                border_index: dec.read_u32()?,
                border_color_index: dec.read_u32()?,
                symbol_index: dec.read_u32()?,
                symbol_color_index: dec.read_u32()?,
                button_name: dec.read_wstring()?,
            }
        } else {
            TabDecoration::default()
        };
        dec.read_block_end()?;
        Ok(Self {
            width,
            height,
            items,
            decoration,
        })
    }

    /// Writes the tab as a nested block (id 0).
    ///
    /// # Errors
    /// [`SaveEncodeError::DecorationNotInVersion`] if the decoration is
    /// set but `version` predates it; item and cipher errors otherwise.
    pub fn write(
        &self,
        enc: &mut Encoder,
        version: ContainerVersion,
    ) -> Result<(), SaveEncodeError> {
        if !version.has_tab_decoration() && !self.decoration.is_default() {
            return Err(SaveEncodeError::DecorationNotInVersion { version });
        }
        enc.write_block(BlockId::NESTED, |enc| {
            enc.write_u32(self.width);
            enc.write_u32(self.height);
            enc.write_u32(length_word(self.items.len())?);
            self.items
                .iter()
                .try_for_each(|item| item.write(enc, version))?;
            if version.has_tab_decoration() {
                enc.write_u32(self.decoration.border_index);
                enc.write_u32(self.decoration.border_color_index);
                enc.write_u32(self.decoration.symbol_index);
                enc.write_u32(self.decoration.symbol_color_index);
                enc.write_wstring(&self.decoration.button_name)?;
            }
            Ok(())
        })
    }
}

/// Converts a collection length to its on-disk `u32` count.
///
/// # Errors
/// [`EncodeError::PayloadTooLong`] beyond `u32::MAX` elements.
pub fn length_word(len: usize) -> Result<u32, EncodeError> {
    u32::try_from(len).map_err(|_| EncodeError::PayloadTooLong { len })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;

    fn encode_sample() -> Vec<u8> {
        let mut enc = Encoder::new(0xA5A5_1234);
        enc.write_u32(1);
        enc.write_block(BlockId::new(19), |enc| {
            enc.write_u32(2);
            enc.write_u32(0);
            enc.write_string("records/items/x.dbr")?;
            enc.write_block(BlockId::NESTED, |enc| {
                enc.write_u32(0);
                enc.write_u32(7);
                enc.write_block(BlockId::new(5), |enc| {
                    enc.write_u8(0);
                    enc.write_u8(0);
                    Ok::<(), EncodeError>(())
                })?;
                enc.write_u32(0);
                Ok::<(), EncodeError>(())
            })?;
            enc.write_u32(0);
            enc.write_u8(0);
            Ok::<(), EncodeError>(())
        })
        .unwrap();
        enc.finish()
    }

    #[test]
    fn opaque_block_recovers_nested_structure_and_round_trips() {
        let bytes = encode_sample();
        let mut dec = Decoder::new(&bytes).unwrap();
        assert_eq!(dec.read_u32().unwrap(), 1);
        let block = OpaqueBlock::read(&mut dec, OpaqueReason::Unmodeled).unwrap();
        assert!(dec.is_at_end());
        assert_eq!(block.id(), BlockId::new(19));
        assert_eq!(
            block.nested_ids().collect::<Vec<_>>(),
            vec![BlockId::NESTED]
        );
        let OpaqueElement::Block { body, .. } = &block.body()[1] else {
            panic!("second element should be the nested block");
        };
        assert!(matches!(body[1], OpaqueElement::Block { id, .. } if id == BlockId::new(5)));

        let mut enc = Encoder::new(dec.seed());
        enc.write_u32(1);
        block.write(&mut enc).unwrap();
        assert_eq!(enc.finish(), bytes);
    }

    #[test]
    fn opaque_block_recovers_zero_marker_after_version() {
        let mut enc = Encoder::new(0x0C0F_FEE0);
        enc.write_u32(1);
        enc.write_block(BlockId::new(20), |enc| {
            enc.write_u32(1);
            enc.write_zero_marker();
            enc.write_u32(0);
            enc.write_string("records/items/crafting/materials/m.dbr")?;
            enc.write_block(BlockId::NESTED, |enc| {
                enc.write_u32(0);
                Ok::<(), EncodeError>(())
            })?;
            Ok::<(), EncodeError>(())
        })
        .unwrap();
        let bytes = enc.finish();

        let mut dec = Decoder::new(&bytes).unwrap();
        dec.read_u32().unwrap();
        let block = OpaqueBlock::read(&mut dec, OpaqueReason::Unmodeled).unwrap();
        assert!(dec.is_at_end());
        assert_eq!(block.body()[1], OpaqueElement::ZeroMarker);
        assert_eq!(
            block.nested_ids().collect::<Vec<_>>(),
            vec![BlockId::NESTED]
        );

        let mut enc = Encoder::new(dec.seed());
        enc.write_u32(1);
        block.write(&mut enc).unwrap();
        assert_eq!(enc.finish(), bytes);
    }

    #[test]
    fn opaque_block_refuses_rekeying() {
        let bytes = encode_sample();
        let mut dec = Decoder::new(&bytes).unwrap();
        dec.read_u32().unwrap();
        let block = OpaqueBlock::read(&mut dec, OpaqueReason::Unmodeled).unwrap();
        let mut enc = Encoder::new(dec.seed());
        enc.write_u32(2);
        assert!(matches!(
            block.write(&mut enc),
            Err(SaveEncodeError::OpaqueRekeyed { block, .. }) if block == BlockId::new(19)
        ));
    }

    #[test]
    fn walker_types_known_blocks_and_falls_back_to_opaque() {
        let bytes = encode_sample();
        let mut dec = Decoder::new(&bytes).unwrap();
        dec.read_u32().unwrap();
        let typed = read_block(
            &mut dec,
            |dec, header| {
                assert_eq!(header.id, BlockId::new(19));
                assert_eq!(header.version, 2);
                dec.read_u32()?;
                let name = dec.read_string()?;
                dec.read_block_start()?;
                dec.read_u32()?;
                dec.read_u32()?;
                dec.read_block_start()?;
                dec.read_u8()?;
                dec.read_u8()?;
                dec.read_block_end()?;
                dec.read_u32()?;
                dec.read_block_end()?;
                dec.read_u32()?;
                dec.read_u8()?;
                Ok::<_, DecodeError>(Dispatch::Typed(Some(name)))
            },
            |_| None,
        )
        .unwrap();
        assert_eq!(typed.as_deref(), Some("records/items/x.dbr"));
        assert!(dec.is_at_end());

        let mut dec = Decoder::new(&bytes).unwrap();
        dec.read_u32().unwrap();
        let opaque = read_block(
            &mut dec,
            |_, header| {
                Ok::<_, DecodeError>(Dispatch::Opaque(OpaqueReason::UnsupportedVersion {
                    version: header.version,
                }))
            },
            Some,
        )
        .unwrap()
        .expect("fell back to opaque");
        assert_eq!(
            opaque.reason(),
            OpaqueReason::UnsupportedVersion { version: 2 }
        );
        assert!(dec.is_at_end());
    }

    fn sample_tab() -> StashTab {
        StashTab {
            width: 10,
            height: 18,
            items: vec![StashItem {
                item: Item {
                    base_name: "records/items/a.dbr".into(),
                    stack_count: 1,
                    ..Item::default()
                },
                x: 2.0,
                y: 3.0,
            }],
            decoration: TabDecoration {
                border_index: 1,
                border_color_index: 2,
                symbol_index: 3,
                symbol_color_index: 4,
                button_name: "Sets".into(),
            },
        }
    }

    #[test]
    fn stash_tab_round_trips_with_and_without_decoration() {
        for (raw, decorated) in [(11, true), (8, false)] {
            let version = ContainerVersion::new(raw).unwrap();
            let tab = if decorated {
                sample_tab()
            } else {
                StashTab {
                    decoration: TabDecoration::default(),
                    ..sample_tab()
                }
            };
            let mut enc = Encoder::new(3);
            tab.write(&mut enc, version).unwrap();
            let bytes = enc.finish();
            let mut dec = Decoder::new(&bytes).unwrap();
            assert_eq!(StashTab::read(&mut dec, version).unwrap(), tab);
            assert!(dec.is_at_end());
        }
    }

    #[test]
    fn decorated_tab_refuses_old_version() {
        let version = ContainerVersion::new(8).unwrap();
        let mut enc = Encoder::new(3);
        assert_eq!(
            sample_tab().write(&mut enc, version),
            Err(SaveEncodeError::DecorationNotInVersion { version })
        );
    }
}
