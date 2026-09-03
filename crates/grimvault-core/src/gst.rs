//! `*.gst` files: `transfer.gst` (the shared stash, block 18) and the
//! same block walker over `transmutes.gst` and `reagents.gst`.
//!
//! Block 18 ported from gdlc (MIT, dandels 2025), `src/stash.rs`.
//!
//! Layout: raw seed; one `u32` (observed 2 in `transfer.gst`, 1 in
//! `transmutes.gst` / `reagents.gst`); then framed blocks to end of
//! file. Block 18 is typed; block 19 (`transmutes.gst`, v2) and block 20
//! (`reagents.gst`, v1) are carried opaquely. `formulas.gst` is **not**
//! obfuscated at all — it is a plaintext `begin_block` / `end_block`
//! key-value format — and is refused with
//! [`GstError::PlaintextKeyValueFormat`].

use thiserror::Error;

use crate::block::{
    Dispatch, OpaqueBlock, OpaqueReason, SaveEncodeError, StashTab, length_word, read_block,
};
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};
use crate::item::ContainerVersion;

const TRANSFER_STASH_MIN_VERSION: u32 = 5;
const PLAINTEXT_PREAMBLE: &[u8] = b"\x0b\x00\x00\x00begin_block";

/// Why a `.gst` could not be parsed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GstError {
    /// Cipher / framing failure.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// The file is the plaintext key-value format used by
    /// `formulas.gst`, which this walker does not read.
    #[error("plaintext begin_block/end_block key-value format, not a rolling-XOR save")]
    PlaintextKeyValueFormat,
}

/// Block 18: the shared stash.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferStash {
    /// Layout version.
    pub version: ContainerVersion,
    /// Mod name the stash belongs to; empty for the base game.
    pub mod_name: String,
    /// Expansion status byte following the mod name (observed 7, the
    /// same value as the `player.gdc` header's).
    pub expansion_status: u8,
    /// The tabs, in order.
    pub tabs: Vec<StashTab>,
}

impl TransferStash {
    fn read_body(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        dec.read_zero_marker()?;
        let mod_name = dec.read_string()?;
        let expansion_status = dec.read_u8()?;
        let count = dec.read_u32()?;
        let tabs = (0..count)
            .map(|_| StashTab::read(dec, version))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            version,
            mod_name,
            expansion_status,
            tabs,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::TRANSFER_STASH, |enc| {
            enc.write_u32(self.version.raw());
            enc.write_zero_marker();
            enc.write_string(&self.mod_name)?;
            enc.write_u8(self.expansion_status);
            enc.write_u32(length_word(self.tabs.len())?);
            self.tabs
                .iter()
                .try_for_each(|tab| tab.write(enc, self.version))
        })
    }
}

/// One top-level block of a `.gst`, in file order.
#[derive(Clone, Debug, PartialEq)]
pub enum GstBlock {
    /// Block 18 at a supported version.
    TransferStash(TransferStash),
    /// Anything else, preserved verbatim.
    Opaque(OpaqueBlock),
}

impl GstBlock {
    /// The block id.
    #[must_use]
    pub fn id(&self) -> BlockId {
        match self {
            Self::TransferStash(_) => BlockId::TRANSFER_STASH,
            Self::Opaque(block) => block.id(),
        }
    }

    fn read(dec: &mut Decoder<'_>) -> Result<Self, GstError> {
        read_block(
            dec,
            |dec, header| {
                Ok::<_, GstError>(match header.id {
                    BlockId::TRANSFER_STASH => match ContainerVersion::new(header.version) {
                        Ok(version) if version.raw() >= TRANSFER_STASH_MIN_VERSION => {
                            Dispatch::Typed(Self::TransferStash(TransferStash::read_body(
                                dec, version,
                            )?))
                        }
                        Ok(_) | Err(_) => Dispatch::Opaque(OpaqueReason::UnsupportedVersion {
                            version: header.version,
                        }),
                    },
                    _ => Dispatch::Opaque(OpaqueReason::Unmodeled),
                })
            },
            Self::Opaque,
        )
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        match self {
            Self::TransferStash(stash) => stash.write(enc),
            Self::Opaque(block) => block.write(enc),
        }
    }
}

/// A parsed `.gst`: leading word plus the ordered block sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct GstFile {
    seed: u32,
    file_version: u32,
    blocks: Vec<GstBlock>,
}

impl GstFile {
    /// Parses a whole file image.
    ///
    /// # Errors
    /// [`GstError::PlaintextKeyValueFormat`] for `formulas.gst`-style
    /// files, or any cipher / framing failure.
    pub fn parse(bytes: &[u8]) -> Result<Self, GstError> {
        if bytes.starts_with(PLAINTEXT_PREAMBLE) {
            return Err(GstError::PlaintextKeyValueFormat);
        }
        let mut dec = Decoder::new(bytes)?;
        let file_version = dec.read_u32()?;
        let mut blocks = Vec::new();
        while !dec.is_at_end() {
            blocks.push(GstBlock::read(&mut dec)?);
        }
        Ok(Self {
            seed: dec.seed(),
            file_version,
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
        enc.write_u32(self.file_version);
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

    /// The word between the seed and the first block.
    #[must_use]
    pub fn file_version(&self) -> u32 {
        self.file_version
    }

    /// The blocks in file order.
    #[must_use]
    pub fn blocks(&self) -> &[GstBlock] {
        &self.blocks
    }

    /// Block 18, when typed.
    #[must_use]
    pub fn transfer_stash(&self) -> Option<&TransferStash> {
        self.blocks.iter().find_map(|block| match block {
            GstBlock::TransferStash(stash) => Some(stash),
            GstBlock::Opaque(_) => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::TabDecoration;
    use crate::item::{Item, StashItem};

    fn sample() -> GstFile {
        GstFile {
            seed: 0xC533_DF77,
            file_version: 2,
            blocks: vec![GstBlock::TransferStash(TransferStash {
                version: ContainerVersion::new(11).unwrap(),
                mod_name: String::new(),
                expansion_status: 7,
                tabs: vec![
                    StashTab {
                        width: 10,
                        height: 18,
                        items: vec![StashItem {
                            item: Item {
                                base_name: "records/items/gearrelic/r.dbr".into(),
                                seed: 99,
                                stack_count: 1,
                                ..Item::default()
                            },
                            x: 4.0,
                            y: 6.0,
                        }],
                        decoration: TabDecoration {
                            button_name: "Relics".into(),
                            ..TabDecoration::default()
                        },
                    },
                    StashTab {
                        width: 10,
                        height: 18,
                        items: vec![],
                        decoration: TabDecoration::default(),
                    },
                ],
            })],
        }
    }

    #[test]
    fn transfer_stash_round_trips_through_bytes() {
        let file = sample();
        let bytes = file.encode().unwrap();
        let parsed = GstFile::parse(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(parsed.encode().unwrap(), bytes);
        let stash = parsed.transfer_stash().unwrap();
        assert_eq!(stash.tabs.len(), 2);
        assert_eq!(stash.tabs[0].items.len(), 1);
    }

    #[test]
    fn unknown_blocks_are_opaque_and_round_trip() {
        let mut enc = Encoder::new(1);
        enc.write_u32(1);
        enc.write_block(BlockId::new(19), |enc| {
            enc.write_u32(2);
            enc.write_block(BlockId::NESTED, |enc| {
                enc.write_string("records/items/crafting/x.dbr")?;
                enc.write_u32(0);
                Ok::<(), crate::crypto::EncodeError>(())
            })?;
            Ok::<(), crate::crypto::EncodeError>(())
        })
        .unwrap();
        let bytes = enc.finish();
        let file = GstFile::parse(&bytes).unwrap();
        assert!(file.transfer_stash().is_none());
        assert_eq!(file.blocks()[0].id(), BlockId::new(19));
        assert_eq!(file.encode().unwrap(), bytes);
    }

    #[test]
    fn plaintext_formulas_are_refused() {
        let mut bytes = PLAINTEXT_PREAMBLE.to_vec();
        bytes.extend_from_slice(&0xB01D_FACE_u32.to_le_bytes());
        assert_eq!(
            GstFile::parse(&bytes),
            Err(GstError::PlaintextKeyValueFormat)
        );
    }
}
