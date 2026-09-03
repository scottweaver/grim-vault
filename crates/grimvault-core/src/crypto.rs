//! The rolling-XOR obfuscation shared by every Grim Dawn save file
//! (`player.gdc`, `*.gst`).
//!
//! Ported from gdlc (MIT, dandels 2025), `src/decrypt.rs`, with the
//! encoder written here as its exact inverse.
//!
//! The scheme: the file's first `u32` is a raw seed; `seed ^ 0x5555_5555`
//! is the initial key and also generates a 256-entry table. Every
//! *ciphertext* byte read or written then folds `table[byte]` into the
//! key, so the key at any offset is a function of every prior byte and a
//! byte-level splice can never exist (ARCHITECTURE.md "Full re-encode").
//!
//! Two read shapes exist and are **not** interchangeable: a `u32` is
//! masked whole against the current key before the four ciphertext bytes
//! update it, while a byte is masked against the key's low byte and then
//! updates it. Block lengths and block-end checksums are "static" `u32`s
//! — masked against the key without updating it.
//!
//! Single-byte order: this module XORs first and updates second, the
//! order used by gdlc's `read_n_bytes` and by the three other public
//! implementations. gdlc's `read_byte` updates first; that order makes
//! the boolean fields of real saves decode to garbage, so it is a gdlc
//! quirk, not the format (verified against the vendored fixture and the
//! user's saves, see `tests/`).

use std::fmt;

use thiserror::Error;

const SEED_MASK: u32 = 0x5555_5555;
const TABLE_MULTIPLIER: u32 = 39_916_801;

/// Identifier of a framed block (`id`, static length, body, checksum).
///
/// Top-level blocks carry the game's own ids (1 = character info,
/// 3 = inventory, 4 = per-character stash, 18 = transfer stash); nested
/// sacks and stash tabs use id 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(u32);

impl BlockId {
    /// Nested sack / stash-tab block.
    pub const NESTED: Self = Self(0);
    /// `player.gdc` character info.
    pub const CHARACTER_INFO: Self = Self(1);
    /// `player.gdc` inventory (sacks and equipment).
    pub const INVENTORY: Self = Self(3);
    /// `player.gdc` per-character stash.
    pub const PLAYER_STASH: Self = Self(4);
    /// `transfer.gst` shared stash.
    pub const TRANSFER_STASH: Self = Self(18);
    /// `transmutes.gst` unlocked illusions.
    pub const ILLUSIONS: Self = Self(19);
    /// `reagents.gst` component and crafting-material storage.
    pub const REAGENT_STORAGE: Self = Self(20);

    /// Wraps a raw block id read from a file.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw id as stored in the file.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block {}", self.0)
    }
}

/// The 256-entry key-update table derived from a file's seed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyTable {
    entries: [u32; 256],
}

impl KeyTable {
    /// Builds the table for a raw file seed (the first `u32` of the file).
    #[must_use]
    pub fn from_seed(seed: u32) -> Self {
        let mut k = seed ^ SEED_MASK;
        let mut entries = [0u32; 256];
        for entry in &mut entries {
            k = k.rotate_right(1).wrapping_mul(TABLE_MULTIPLIER);
            *entry = k;
        }
        Self { entries }
    }

    fn entry(&self, cipher_byte: u8) -> u32 {
        self.entries[usize::from(cipher_byte)]
    }

    /// The raw table entries, in index order.
    #[must_use]
    pub fn entries(&self) -> &[u32; 256] {
        &self.entries
    }
}

impl fmt::Debug for KeyTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyTable").finish_non_exhaustive()
    }
}

/// Why a decode failed at the cipher / framing layer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    /// The file is shorter than the four-byte seed.
    #[error("file too short to contain a cipher seed")]
    MissingSeed,
    /// A read would run past the end of the file.
    #[error("unexpected end of data at offset {offset} (wanted {wanted} bytes, {available} left)")]
    UnexpectedEof {
        /// Offset the read started at.
        offset: usize,
        /// Bytes the read needed.
        wanted: usize,
        /// Bytes left in the file.
        available: usize,
    },
    /// A read would run past the end of the innermost open block.
    #[error(
        "read past the end of {block} at offset {offset} (wanted {wanted} bytes, {available} left in block)"
    )]
    ReadPastBlockEnd {
        /// The innermost open block.
        block: BlockId,
        /// Offset the read started at.
        offset: usize,
        /// Bytes the read needed.
        wanted: usize,
        /// Bytes left in the block.
        available: usize,
    },
    /// A block start's declared length exceeds the enclosing bounds.
    #[error("{block} declares {len} bytes but only {available} remain")]
    BlockTooLong {
        /// The block being opened.
        block: BlockId,
        /// Declared body length.
        len: u32,
        /// Bytes available for the body.
        available: usize,
    },
    /// `read_block_end` was called at the wrong offset: the parser
    /// consumed a different number of bytes than the block declared.
    #[error("{block} body ended at offset {actual}, expected {expected}")]
    BlockLengthMismatch {
        /// The block being closed.
        block: BlockId,
        /// Where the declared length says the body ends.
        expected: usize,
        /// Where the parser actually stopped.
        actual: usize,
    },
    /// The block-end checksum did not match the running key.
    #[error("{block} checksum mismatch at offset {offset}: key state diverged from the writer's")]
    BlockChecksum {
        /// The block being closed.
        block: BlockId,
        /// Offset of the checksum word.
        offset: usize,
    },
    /// `read_block_end` was called with no block open.
    #[error("read_block_end called outside any block at offset {offset}")]
    NoOpenBlock {
        /// Offset of the call.
        offset: usize,
    },
    /// A block header carried an id other than the one the layout requires.
    #[error("expected {expected} at offset {offset}, found {found}")]
    UnexpectedBlockId {
        /// The id the layout requires.
        expected: BlockId,
        /// The id read.
        found: BlockId,
        /// Offset of the id word.
        offset: usize,
    },
    /// A static zero marker (header separator) decoded to something else.
    #[error("expected a zero marker at offset {offset}, decoded {value:#x}")]
    NonZeroMarker {
        /// Offset of the marker word.
        offset: usize,
        /// What it decoded to.
        value: u32,
    },
    /// A boolean byte was neither 0 nor 1.
    #[error("boolean at offset {offset} decoded to {value}, expected 0 or 1")]
    InvalidBool {
        /// Offset of the byte.
        offset: usize,
        /// What it decoded to.
        value: u8,
    },
    /// A byte string was not valid UTF-8.
    #[error("string at offset {offset} is not valid UTF-8")]
    InvalidUtf8 {
        /// Offset of the string's length prefix.
        offset: usize,
    },
    /// A wide string was not valid UTF-16.
    #[error("wide string at offset {offset} is not valid UTF-16")]
    InvalidUtf16 {
        /// Offset of the string's length prefix.
        offset: usize,
    },
}

/// Why an encode failed at the cipher / framing layer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EncodeError {
    /// A string or byte run is longer than a `u32` length prefix can hold.
    #[error("payload of {len} bytes exceeds the u32 length prefix")]
    PayloadTooLong {
        /// The offending length.
        len: usize,
    },
    /// A block body is longer than its `u32` length field can hold.
    #[error("{block} body of {len} bytes exceeds the u32 length field")]
    BlockTooLong {
        /// The block being closed.
        block: BlockId,
        /// The body length.
        len: usize,
    },
}

/// A saved decoder position; see [`Decoder::checkpoint`].
#[derive(Clone, Debug)]
pub struct Checkpoint {
    pos: usize,
    key: u32,
    block_ends: Vec<(BlockId, usize)>,
}

/// Start of a framed block as read from the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockStart {
    /// The block id.
    pub id: BlockId,
    /// Declared body length in bytes (excludes the id, length, and checksum).
    pub len: u32,
}

/// Streaming decoder over an in-memory save file.
///
/// Reads track the innermost open block (`read_block_start` /
/// `read_block_end`) so an over-consuming parser fails with
/// [`DecodeError::ReadPastBlockEnd`] rather than reading into the next
/// block, and an under-consuming one fails with
/// [`DecodeError::BlockLengthMismatch`] when it tries to close the block.
#[derive(Debug)]
pub struct Decoder<'a> {
    bytes: &'a [u8],
    pos: usize,
    key: u32,
    table: KeyTable,
    seed: u32,
    block_ends: Vec<(BlockId, usize)>,
}

impl<'a> Decoder<'a> {
    /// Seeds the cipher from the file's first `u32` and positions the
    /// cursor after it.
    ///
    /// # Errors
    /// [`DecodeError::MissingSeed`] if the file is shorter than 4 bytes.
    pub fn new(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        let seed_bytes: [u8; 4] = bytes
            .get(..4)
            .and_then(|s| s.try_into().ok())
            .ok_or(DecodeError::MissingSeed)?;
        let seed = u32::from_le_bytes(seed_bytes);
        Ok(Self {
            bytes,
            pos: 4,
            key: seed ^ SEED_MASK,
            table: KeyTable::from_seed(seed),
            seed,
            block_ends: Vec::new(),
        })
    }

    /// The raw seed word: pass it to [`Encoder::new`] to re-encode a
    /// file byte-identically.
    #[must_use]
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// The current key state. Recorded by opaque-block parsers so a later
    /// encode can detect that the block would be re-keyed.
    #[must_use]
    pub fn key(&self) -> u32 {
        self.key
    }

    /// Byte offset of the cursor from the start of the file.
    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bytes left before the end of the innermost open block, or of the
    /// file when no block is open.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.bound().saturating_sub(self.pos)
    }

    /// Whether the innermost open block (or the file) has been fully consumed.
    #[must_use]
    pub fn is_at_end(&self) -> bool {
        self.remaining() == 0
    }

    fn bound(&self) -> usize {
        self.block_ends
            .last()
            .map_or(self.bytes.len(), |&(_, end)| end)
    }

    /// Captures the cursor, key and open-block stack so a speculative
    /// parse can be undone — including one that failed inside
    /// [`read_block_end`](Self::read_block_end).
    #[must_use]
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            pos: self.pos,
            key: self.key,
            block_ends: self.block_ends.clone(),
        }
    }

    /// Rewinds to a checkpoint taken earlier on this decoder.
    pub fn restore(&mut self, checkpoint: Checkpoint) {
        self.pos = checkpoint.pos;
        self.key = checkpoint.key;
        self.block_ends = checkpoint.block_ends;
    }

    /// Decodes the block header at the cursor without consuming it or
    /// touching the key; `None` if fewer than 8 bytes remain in bounds.
    /// The returned length is unchecked against the bounds.
    #[must_use]
    pub fn peek_block_start(&self) -> Option<BlockStart> {
        let words = self.bytes.get(self.pos..self.pos + 8)?;
        if self.pos + 8 > self.bound() {
            return None;
        }
        let raw_id = u32::from_le_bytes([words[0], words[1], words[2], words[3]]);
        let raw_len = u32::from_le_bytes([words[4], words[5], words[6], words[7]]);
        let key_after_id = raw_id
            .to_le_bytes()
            .iter()
            .fold(self.key, |k, &b| k ^ self.table.entry(b));
        Some(BlockStart {
            id: BlockId::new(raw_id ^ self.key),
            len: raw_len ^ key_after_id,
        })
    }

    fn take(&mut self, wanted: usize) -> Result<&'a [u8], DecodeError> {
        let offset = self.pos;
        let available = self.remaining();
        if wanted > available {
            return Err(match self.block_ends.last() {
                Some(&(block, _)) => DecodeError::ReadPastBlockEnd {
                    block,
                    offset,
                    wanted,
                    available,
                },
                None => DecodeError::UnexpectedEof {
                    offset,
                    wanted,
                    available,
                },
            });
        }
        self.pos += wanted;
        Ok(&self.bytes[offset..offset + wanted])
    }

    fn take_word(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn fold(&mut self, cipher_bytes: &[u8]) {
        for &b in cipher_bytes {
            self.key ^= self.table.entry(b);
        }
    }

    /// Decodes one byte: XOR against the key's low byte, then fold the
    /// ciphertext byte into the key.
    ///
    /// # Errors
    /// Bounds errors per the type docs.
    pub fn read_u8(&mut self) -> Result<u8, DecodeError> {
        let cipher = self.take(1)?[0];
        let plain = cipher ^ self.key.to_le_bytes()[0];
        self.fold(&[cipher]);
        Ok(plain)
    }

    /// Decodes one byte and requires it to be 0 or 1.
    ///
    /// # Errors
    /// [`DecodeError::InvalidBool`] for any other value, plus bounds errors.
    pub fn read_bool(&mut self) -> Result<bool, DecodeError> {
        let offset = self.pos;
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(DecodeError::InvalidBool { offset, value }),
        }
    }

    /// Decodes a `u32`: XOR the whole word against the key, then fold its
    /// four ciphertext bytes into the key.
    ///
    /// # Errors
    /// Bounds errors per the type docs.
    pub fn read_u32(&mut self) -> Result<u32, DecodeError> {
        let cipher = self.take_word()?;
        let plain = cipher ^ self.key;
        self.fold(&cipher.to_le_bytes());
        Ok(plain)
    }

    /// Decodes an `i32` (a `u32` reinterpreted).
    ///
    /// # Errors
    /// Bounds errors per the type docs.
    #[allow(clippy::cast_possible_wrap)] // reinterpretation is the point
    pub fn read_i32(&mut self) -> Result<i32, DecodeError> {
        self.read_u32().map(|v| v as i32)
    }

    /// Decodes an `f32` (a `u32` reinterpreted bit-for-bit).
    ///
    /// # Errors
    /// Bounds errors per the type docs.
    pub fn read_f32(&mut self) -> Result<f32, DecodeError> {
        self.read_u32().map(f32::from_bits)
    }

    /// Decodes a `u32` **without** folding it into the key — the shape of
    /// block lengths and checksums.
    ///
    /// # Errors
    /// Bounds errors per the type docs.
    pub fn read_u32_static(&mut self) -> Result<u32, DecodeError> {
        let cipher = self.take_word()?;
        Ok(cipher ^ self.key)
    }

    /// Reads a static `u32` that must decode to zero (the `player.gdc`
    /// header separator).
    ///
    /// # Errors
    /// [`DecodeError::NonZeroMarker`] otherwise, plus bounds errors.
    pub fn read_zero_marker(&mut self) -> Result<(), DecodeError> {
        let offset = self.pos;
        match self.read_u32_static()? {
            0 => Ok(()),
            value => Err(DecodeError::NonZeroMarker { offset, value }),
        }
    }

    /// Decodes `n` bytes one at a time.
    ///
    /// # Errors
    /// Bounds errors per the type docs.
    pub fn read_bytes(&mut self, n: usize) -> Result<Vec<u8>, DecodeError> {
        let cipher = self.take(n)?;
        let mut key = self.key;
        let plain = cipher
            .iter()
            .map(|&c| {
                let p = c ^ key.to_le_bytes()[0];
                key ^= self.table.entry(c);
                p
            })
            .collect();
        self.key = key;
        Ok(plain)
    }

    /// Decodes a `u32`-length-prefixed UTF-8 string.
    ///
    /// # Errors
    /// [`DecodeError::InvalidUtf8`], plus bounds errors.
    pub fn read_string(&mut self) -> Result<String, DecodeError> {
        let offset = self.pos;
        let len = self.read_u32()?;
        let bytes = self.read_bytes(len as usize)?;
        String::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8 { offset })
    }

    /// Decodes a string whose `u32` prefix counts UTF-16LE code units.
    ///
    /// # Errors
    /// [`DecodeError::InvalidUtf16`], plus bounds errors.
    pub fn read_wstring(&mut self) -> Result<String, DecodeError> {
        let offset = self.pos;
        let units = self.read_u32()? as usize;
        let bytes = self.read_bytes(units * 2)?;
        let code_units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        String::from_utf16(&code_units).map_err(|_| DecodeError::InvalidUtf16 { offset })
    }

    /// Reads a block header (id, then static length) and opens the block
    /// so subsequent reads are bounded by it.
    ///
    /// # Errors
    /// [`DecodeError::BlockTooLong`] if the length overruns the enclosing
    /// bounds, plus bounds errors.
    pub fn read_block_start(&mut self) -> Result<BlockStart, DecodeError> {
        let id = BlockId::new(self.read_u32()?);
        let len = self.read_u32_static()?;
        let available = self.remaining();
        if len as usize > available {
            return Err(DecodeError::BlockTooLong {
                block: id,
                len,
                available,
            });
        }
        self.block_ends.push((id, self.pos + len as usize));
        Ok(BlockStart { id, len })
    }

    /// [`read_block_start`](Self::read_block_start) for a layout that
    /// requires a specific id.
    ///
    /// # Errors
    /// [`DecodeError::UnexpectedBlockId`] on any other id, plus the
    /// errors of `read_block_start`.
    pub fn read_block_start_expecting(
        &mut self,
        expected: BlockId,
    ) -> Result<BlockStart, DecodeError> {
        let offset = self.pos;
        let start = self.read_block_start()?;
        if start.id == expected {
            Ok(start)
        } else {
            Err(DecodeError::UnexpectedBlockId {
                expected,
                found: start.id,
                offset,
            })
        }
    }

    /// Verifies the cursor sits at the declared end of the innermost
    /// block, reads its checksum, and closes it.
    ///
    /// # Errors
    /// [`DecodeError::BlockLengthMismatch`] if the parser consumed a
    /// different number of bytes than declared,
    /// [`DecodeError::BlockChecksum`] if the key state diverged from the
    /// writer's, [`DecodeError::NoOpenBlock`] if nothing is open.
    pub fn read_block_end(&mut self) -> Result<(), DecodeError> {
        let offset = self.pos;
        let Some(&(block, expected)) = self.block_ends.last() else {
            return Err(DecodeError::NoOpenBlock { offset });
        };
        if offset != expected {
            return Err(DecodeError::BlockLengthMismatch {
                block,
                expected,
                actual: offset,
            });
        }
        self.block_ends.pop();
        match self.read_u32_static()? {
            0 => Ok(()),
            _ => Err(DecodeError::BlockChecksum { block, offset }),
        }
    }
}

/// Streaming encoder producing the exact inverse of [`Decoder`].
#[derive(Debug)]
pub struct Encoder {
    out: Vec<u8>,
    key: u32,
    table: KeyTable,
}

impl Encoder {
    /// Starts a file with the given raw seed word (written verbatim).
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self {
            out: seed.to_le_bytes().to_vec(),
            key: seed ^ SEED_MASK,
            table: KeyTable::from_seed(seed),
        }
    }

    /// The current key state.
    #[must_use]
    pub fn key(&self) -> u32 {
        self.key
    }

    /// Byte offset of the cursor from the start of the output.
    #[must_use]
    pub fn position(&self) -> usize {
        self.out.len()
    }

    /// Consumes the encoder, yielding the file bytes.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.out
    }

    /// Encodes one byte: XOR against the key's low byte, then fold the
    /// resulting ciphertext byte into the key.
    pub fn write_u8(&mut self, plain: u8) {
        let cipher = plain ^ self.key.to_le_bytes()[0];
        self.key ^= self.table.entry(cipher);
        self.out.push(cipher);
    }

    /// Encodes a boolean as a 0/1 byte.
    pub fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    /// Encodes a `u32`: XOR the whole word against the key, then fold its
    /// four ciphertext bytes into the key.
    pub fn write_u32(&mut self, plain: u32) {
        let cipher = plain ^ self.key;
        let bytes = cipher.to_le_bytes();
        for b in bytes {
            self.key ^= self.table.entry(b);
        }
        self.out.extend_from_slice(&bytes);
    }

    /// Encodes an `i32` (reinterpreted as `u32`).
    #[allow(clippy::cast_sign_loss)] // reinterpretation is the point
    pub fn write_i32(&mut self, plain: i32) {
        self.write_u32(plain as u32);
    }

    /// Encodes an `f32` bit-for-bit.
    pub fn write_f32(&mut self, plain: f32) {
        self.write_u32(plain.to_bits());
    }

    /// Encodes a `u32` **without** folding it into the key.
    pub fn write_u32_static(&mut self, plain: u32) {
        self.out
            .extend_from_slice(&(plain ^ self.key).to_le_bytes());
    }

    /// Writes the static zero marker (the `player.gdc` header separator).
    pub fn write_zero_marker(&mut self) {
        self.write_u32_static(0);
    }

    /// Encodes bytes one at a time.
    pub fn write_bytes(&mut self, plain: &[u8]) {
        self.out.reserve(plain.len());
        for &p in plain {
            self.write_u8(p);
        }
    }

    /// Encodes a `u32`-length-prefixed UTF-8 string.
    ///
    /// # Errors
    /// [`EncodeError::PayloadTooLong`] if the byte length exceeds `u32`.
    pub fn write_string(&mut self, s: &str) -> Result<(), EncodeError> {
        let len = length_prefix(s.len())?;
        self.write_u32(len);
        self.write_bytes(s.as_bytes());
        Ok(())
    }

    /// Encodes a string as UTF-16LE with a `u32` code-unit count prefix.
    ///
    /// # Errors
    /// [`EncodeError::PayloadTooLong`] if the unit count exceeds `u32`.
    pub fn write_wstring(&mut self, s: &str) -> Result<(), EncodeError> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let len = length_prefix(units.len())?;
        self.write_u32(len);
        for unit in units {
            self.write_bytes(&unit.to_le_bytes());
        }
        Ok(())
    }

    /// Writes a framed block: id, static length, the body produced by
    /// `body`, then the checksum. The length is patched in after the body
    /// is written; it never feeds the key, so the patch is exact.
    ///
    /// # Errors
    /// Whatever `body` returns, or [`EncodeError::BlockTooLong`] (via
    /// `From`) if the body exceeds `u32::MAX` bytes.
    pub fn write_block<E>(
        &mut self,
        id: BlockId,
        body: impl FnOnce(&mut Self) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<EncodeError>,
    {
        self.write_u32(id.raw());
        let len_key = self.key;
        let len_at = self.out.len();
        self.out.extend_from_slice(&[0; 4]);
        let body_start = self.out.len();
        body(self)?;
        let body_len = self.out.len() - body_start;
        let len = u32::try_from(body_len).map_err(|_| EncodeError::BlockTooLong {
            block: id,
            len: body_len,
        })?;
        self.out[len_at..len_at + 4].copy_from_slice(&(len ^ len_key).to_le_bytes());
        self.write_u32_static(0);
        Ok(())
    }
}

fn length_prefix(len: usize) -> Result<u32, EncodeError> {
    u32::try_from(len).map_err(|_| EncodeError::PayloadTooLong { len })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gdlc_table(seed: u32) -> [u32; 256] {
        let mut k = seed ^ 0x5555_5555;
        let mut table = [0u32; 256];
        for entry in &mut table {
            k = k.rotate_right(1).wrapping_mul(39_916_801);
            *entry = k;
        }
        table
    }

    #[test]
    fn key_table_matches_gdlc_construction() {
        for seed in [0, 1, 0xDEAD_BEEF, u32::MAX, 0x1234_5678] {
            assert_eq!(*KeyTable::from_seed(seed).entries(), gdlc_table(seed));
        }
    }

    #[test]
    fn scalar_round_trip() {
        let mut enc = Encoder::new(0xCAFE_F00D);
        enc.write_u8(7);
        enc.write_bool(true);
        enc.write_u32(0xDEAD_BEEF);
        enc.write_i32(-42);
        enc.write_f32(3.5);
        enc.write_u32_static(99);
        enc.write_zero_marker();
        enc.write_bytes(&[1, 2, 3, 250]);
        enc.write_string("records/items/gearweapons/swords/x.dbr")
            .unwrap();
        enc.write_wstring("Mighty Mallory \u{1F600}").unwrap();
        let bytes = enc.finish();

        let mut dec = Decoder::new(&bytes).unwrap();
        assert_eq!(dec.seed(), 0xCAFE_F00D);
        assert_eq!(dec.read_u8().unwrap(), 7);
        assert!(dec.read_bool().unwrap());
        assert_eq!(dec.read_u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(dec.read_i32().unwrap(), -42);
        assert_eq!(dec.read_f32().unwrap().to_bits(), 3.5_f32.to_bits());
        assert_eq!(dec.read_u32_static().unwrap(), 99);
        dec.read_zero_marker().unwrap();
        assert_eq!(dec.read_bytes(4).unwrap(), vec![1, 2, 3, 250]);
        assert_eq!(
            dec.read_string().unwrap(),
            "records/items/gearweapons/swords/x.dbr"
        );
        assert_eq!(dec.read_wstring().unwrap(), "Mighty Mallory \u{1F600}");
        assert!(dec.is_at_end());
    }

    #[test]
    fn nested_blocks_round_trip_and_verify() {
        let mut enc = Encoder::new(0x0102_0304);
        enc.write_u32(2);
        enc.write_block(BlockId::new(18), |enc| {
            enc.write_u32(11);
            enc.write_string("").unwrap();
            enc.write_block(BlockId::NESTED, |enc| {
                enc.write_u32(10);
                enc.write_u32(18);
                enc.write_wstring("tab").unwrap();
                Ok::<(), EncodeError>(())
            })?;
            enc.write_u8(1);
            Ok::<(), EncodeError>(())
        })
        .unwrap();
        let bytes = enc.finish();

        let mut dec = Decoder::new(&bytes).unwrap();
        assert_eq!(dec.read_u32().unwrap(), 2);
        let outer = dec.read_block_start().unwrap();
        assert_eq!(outer.id, BlockId::new(18));
        assert_eq!(dec.read_u32().unwrap(), 11);
        assert_eq!(dec.read_string().unwrap(), "");
        let inner = dec.read_block_start().unwrap();
        assert_eq!(inner.id, BlockId::NESTED);
        assert_eq!(inner.len, 4 + 4 + 4 + 6);
        assert_eq!(dec.read_u32().unwrap(), 10);
        assert_eq!(dec.read_u32().unwrap(), 18);
        assert_eq!(dec.read_wstring().unwrap(), "tab");
        assert!(dec.is_at_end());
        dec.read_block_end().unwrap();
        assert_eq!(dec.read_u8().unwrap(), 1);
        assert_eq!(outer.len as usize, dec.position() - 16);
        dec.read_block_end().unwrap();
        assert!(dec.is_at_end());
    }

    #[test]
    fn block_bounds_are_enforced() {
        let mut enc = Encoder::new(9);
        enc.write_block(BlockId::new(5), |enc| {
            enc.write_u32(1);
            Ok::<(), EncodeError>(())
        })
        .unwrap();
        enc.write_u32(0xFFFF);
        let bytes = enc.finish();

        let mut dec = Decoder::new(&bytes).unwrap();
        dec.read_block_start().unwrap();
        assert_eq!(
            dec.read_block_end(),
            Err(DecodeError::BlockLengthMismatch {
                block: BlockId::new(5),
                expected: 16,
                actual: 12
            })
        );
        dec.read_u32().unwrap();
        assert!(matches!(
            dec.read_u32(),
            Err(DecodeError::ReadPastBlockEnd { .. })
        ));
        dec.read_block_end().unwrap();
        assert_eq!(dec.read_u32().unwrap(), 0xFFFF);
        assert!(matches!(
            dec.read_u32(),
            Err(DecodeError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn corrupted_body_fails_checksum() {
        let mut enc = Encoder::new(77);
        enc.write_block(BlockId::new(3), |enc| {
            enc.write_string("abc").unwrap();
            Ok::<(), EncodeError>(())
        })
        .unwrap();
        let mut bytes = enc.finish();
        bytes[14] ^= 0x40;

        let mut dec = Decoder::new(&bytes).unwrap();
        dec.read_block_start().unwrap();
        dec.read_bytes(7).unwrap();
        assert!(matches!(
            dec.read_block_end(),
            Err(DecodeError::BlockChecksum { .. })
        ));
    }

    #[test]
    fn invalid_bool_is_rejected() {
        let mut enc = Encoder::new(1);
        enc.write_u8(2);
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes).unwrap();
        assert_eq!(
            dec.read_bool(),
            Err(DecodeError::InvalidBool {
                offset: 4,
                value: 2
            })
        );
    }

    #[test]
    fn short_input_has_no_seed() {
        assert_eq!(
            Decoder::new(&[1, 2, 3]).err(),
            Some(DecodeError::MissingSeed)
        );
    }
}
