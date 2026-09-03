//! Typed layouts of every `player.gdc` block beyond 1, 3 and 4 — one
//! module per block id — so the whole file is a typed model and an edit
//! to the inventory or stash, which re-keys every block after it, can
//! be written (`block` docs, "Opaque blocks and the lossless model").
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/{info,lists,skills,misc,
//! stats}.rs`, cross-checked field by field against grim-save-parser
//! (MIT, nbak), `parser/src/parser/model/*.rs`. Field names follow the
//! references where they agree; a field neither reference names keeps
//! a neutral `unknown_<type>_<n>` name and is preserved as read.
//! Supported versions are yagde's; a block at any other version is
//! carried as an [`OpaqueBlock`](crate::block::OpaqueBlock).
//!
//! Every module exposes `BLOCK_ID`, a `VERSION` constant or a version
//! enum, a `read_body` that starts after the version word (the walker
//! in `block::read_block` has consumed it) and a `write` that emits the
//! whole framed block; the two mirror each other field for field.
//! None of these blocks nests id-0 blocks or zero markers.

pub mod bio;
pub mod factions;
pub mod markers;
pub mod notes;
pub mod respawns;
pub mod shrines;
pub mod skills;
pub mod stats;
pub mod teleports;
pub mod tokens;
pub mod tutorials;
pub mod ui;

use crate::block::length_word;
use crate::crypto::{DecodeError, Decoder, EncodeError, Encoder};

/// A 16-byte object id as the game stores world objects (respawn
/// points, rift gates, markers, shrines) — opaque to this crate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Uid([u8; 16]);

impl Uid {
    /// Wraps raw id bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// The raw id bytes.
    #[must_use]
    pub const fn bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Reads sixteen bytes.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        read_array(|| dec.read_u8()).map(Self)
    }

    /// Writes the sixteen bytes.
    pub fn write(&self, enc: &mut Encoder) {
        enc.write_bytes(&self.0);
    }
}

/// One value per game difficulty, in the order the file stores them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PerDifficulty<T> {
    pub normal: T,
    pub elite: T,
    pub ultimate: T,
}

impl<T> PerDifficulty<T> {
    /// Reads the three values in file order.
    ///
    /// # Errors
    /// Whatever `read` returns.
    pub fn read(mut read: impl FnMut() -> Result<T, DecodeError>) -> Result<Self, DecodeError> {
        Ok(Self {
            normal: read()?,
            elite: read()?,
            ultimate: read()?,
        })
    }

    /// The three values in file order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        [&self.normal, &self.elite, &self.ultimate].into_iter()
    }

    /// Applies `write` to each value in file order, stopping at the
    /// first error.
    ///
    /// # Errors
    /// Whatever `write` returns.
    pub fn try_for_each<E>(&self, write: impl FnMut(&T) -> Result<(), E>) -> Result<(), E> {
        self.iter().try_for_each(write)
    }
}

/// Reads exactly `N` values.
///
/// # Errors
/// Whatever `read` returns.
pub(crate) fn read_array<T, const N: usize>(
    mut read: impl FnMut() -> Result<T, DecodeError>,
) -> Result<[T; N], DecodeError> {
    let items = (0..N).map(|_| read()).collect::<Result<Vec<_>, _>>()?;
    Ok(items
        .try_into()
        .unwrap_or_else(|_| unreachable!("collected exactly N elements")))
}

/// Reads a `u32` count followed by that many values.
fn read_vec<'a, T>(
    dec: &mut Decoder<'a>,
    mut read: impl FnMut(&mut Decoder<'a>) -> Result<T, DecodeError>,
) -> Result<Vec<T>, DecodeError> {
    let count = dec.read_u32()?;
    (0..count).map(|_| read(dec)).collect()
}

/// Writes a `u32` count followed by the values.
fn write_vec<T, E>(
    enc: &mut Encoder,
    items: &[T],
    mut write: impl FnMut(&mut Encoder, &T) -> Result<(), E>,
) -> Result<(), E>
where
    E: From<EncodeError>,
{
    enc.write_u32(length_word(items.len())?);
    items.iter().try_for_each(|item| write(enc, item))
}

fn read_uids(dec: &mut Decoder<'_>) -> Result<Vec<Uid>, DecodeError> {
    read_vec(dec, Uid::read)
}

fn write_uids(enc: &mut Encoder, uids: &[Uid]) -> Result<(), EncodeError> {
    write_vec(enc, uids, |enc, uid| {
        uid.write(enc);
        Ok(())
    })
}

fn read_strings(dec: &mut Decoder<'_>) -> Result<Vec<String>, DecodeError> {
    read_vec(dec, Decoder::read_string)
}

fn write_strings(enc: &mut Encoder, strings: &[String]) -> Result<(), EncodeError> {
    write_vec(enc, strings, |enc, s| enc.write_string(s))
}

#[cfg(test)]
pub(crate) mod testing {
    use crate::block::SaveEncodeError;
    use crate::crypto::{Decoder, Encoder};

    /// Writes a block, re-reads it through `read` (which gets the
    /// decoder positioned on the block's id word), and asserts the
    /// decoder consumed everything.
    pub(crate) fn round_trip<T: PartialEq + std::fmt::Debug>(
        value: &T,
        write: impl FnOnce(&T, &mut Encoder) -> Result<(), SaveEncodeError>,
        read: impl FnOnce(&mut Decoder<'_>) -> T,
    ) {
        let mut enc = Encoder::new(0x5EED_0001);
        write(value, &mut enc).unwrap();
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes).unwrap();
        let back = read(&mut dec);
        assert!(dec.is_at_end());
        assert_eq!(&back, value);
    }

    /// Opens a block at the cursor and returns its version word.
    pub(crate) fn open_block(dec: &mut Decoder<'_>, expected: crate::crypto::BlockId) -> u32 {
        dec.read_block_start_expecting(expected).unwrap();
        dec.read_u32().unwrap()
    }
}
