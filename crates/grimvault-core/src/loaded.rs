//! The lossless gate applied at load (ARCHITECTURE.md "Data flow"): a
//! save file becomes editable only once its parsed model has
//! reproduced the file byte-for-byte. A [`Loaded`] value is the proof —
//! it holds the model and the bytes it was proven against, and there is
//! no other way to obtain one. A model that cannot reproduce its file
//! is refused up front, so it can never be edited and written back
//! over a file it did not understand.

use thiserror::Error;

use crate::block::SaveEncodeError;
use crate::gdc::{GdcError, PlayerFile};
use crate::gst::{GstError, GstFile};

/// A save format with a typed parse and a full re-encode.
pub trait SaveCodec: Sized {
    type ParseError;
    type EncodeError;

    /// Parses a whole file image.
    ///
    /// # Errors
    /// The format's own parse error.
    fn parse(bytes: &[u8]) -> Result<Self, Self::ParseError>;

    /// Re-encodes the model; byte-identical to the input when unmodified.
    ///
    /// # Errors
    /// The format's own encode error.
    fn encode(&self) -> Result<Vec<u8>, Self::EncodeError>;
}

impl SaveCodec for GstFile {
    type ParseError = GstError;
    type EncodeError = SaveEncodeError;

    fn parse(bytes: &[u8]) -> Result<Self, GstError> {
        GstFile::parse(bytes)
    }

    fn encode(&self) -> Result<Vec<u8>, SaveEncodeError> {
        GstFile::encode(self)
    }
}

impl SaveCodec for PlayerFile {
    type ParseError = GdcError;
    type EncodeError = SaveEncodeError;

    fn parse(bytes: &[u8]) -> Result<Self, GdcError> {
        PlayerFile::parse(bytes)
    }

    fn encode(&self) -> Result<Vec<u8>, SaveEncodeError> {
        PlayerFile::encode(self)
    }
}

/// Why a file could not be loaded for editing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LoadError<P, E> {
    #[error("parse: {0}")]
    Parse(P),
    /// The model parsed but refused to re-encode unmodified.
    #[error("re-encode of the unmodified model: {0}")]
    Encode(E),
    /// The unmodified model re-encoded to different bytes; the offset
    /// is the first byte that differs (the shorter length when one
    /// image is a prefix of the other).
    #[error(
        "model is not lossless: re-encode first differs from the file at byte {first_difference}"
    )]
    NotLossless { first_difference: usize },
}

/// A model proven to reproduce `baseline` exactly at load time.
#[derive(Clone, Debug)]
pub struct Loaded<T: SaveCodec> {
    model: T,
    baseline: Vec<u8>,
}

impl<T: SaveCodec> Loaded<T> {
    /// Parses `bytes` and admits the model only if its unmodified
    /// re-encode is `bytes`.
    ///
    /// # Errors
    /// [`LoadError::Parse`], [`LoadError::Encode`], or
    /// [`LoadError::NotLossless`].
    pub fn load(bytes: Vec<u8>) -> Result<Self, LoadError<T::ParseError, T::EncodeError>> {
        let model = T::parse(&bytes).map_err(LoadError::Parse)?;
        let reencoded = model.encode().map_err(LoadError::Encode)?;
        match first_difference(&bytes, &reencoded) {
            None => Ok(Self {
                model,
                baseline: bytes,
            }),
            Some(first_difference) => Err(LoadError::NotLossless { first_difference }),
        }
    }

    #[must_use]
    pub fn model(&self) -> &T {
        &self.model
    }

    pub fn model_mut(&mut self) -> &mut T {
        &mut self.model
    }

    /// The bytes the model was loaded from and proven against.
    #[must_use]
    pub fn baseline(&self) -> &[u8] {
        &self.baseline
    }

    /// The current model's bytes — the baseline until something was
    /// edited.
    ///
    /// # Errors
    /// The codec's encode error (for a stash, an edit that would re-key
    /// an opaque block).
    pub fn encode(&self) -> Result<Vec<u8>, T::EncodeError> {
        self.model.encode()
    }
}

fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
    left.iter()
        .zip(right)
        .position(|(l, r)| l != r)
        .or_else(|| (left.len() != right.len()).then(|| left.len().min(right.len())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{StashTab, TabDecoration};
    use crate::crypto::{BlockId, EncodeError, Encoder};
    use crate::item::{ContainerVersion, Item, StashItem};

    /// Accepts any bytes and re-encodes them with the last byte
    /// flipped — a codec that understands nothing but claims to.
    #[derive(Debug)]
    struct Lossy(Vec<u8>);

    #[derive(Debug, PartialEq, Eq)]
    struct Never;

    impl SaveCodec for Lossy {
        type ParseError = Never;
        type EncodeError = Never;

        fn parse(bytes: &[u8]) -> Result<Self, Never> {
            Ok(Self(bytes.to_vec()))
        }

        fn encode(&self) -> Result<Vec<u8>, Never> {
            let mut bytes = self.0.clone();
            if let Some(last) = bytes.last_mut() {
                *last ^= 0xFF;
            }
            Ok(bytes)
        }
    }

    /// Re-encodes one byte short.
    #[derive(Debug)]
    struct Truncating(Vec<u8>);

    impl SaveCodec for Truncating {
        type ParseError = Never;
        type EncodeError = Never;

        fn parse(bytes: &[u8]) -> Result<Self, Never> {
            Ok(Self(bytes.to_vec()))
        }

        fn encode(&self) -> Result<Vec<u8>, Never> {
            Ok(self.0[..self.0.len() - 1].to_vec())
        }
    }

    fn stash_bytes() -> Vec<u8> {
        let mut enc = Encoder::new(0x0BAD_F00D);
        enc.write_u32(2);
        enc.write_block(BlockId::TRANSFER_STASH, |enc| {
            enc.write_u32(11);
            enc.write_zero_marker();
            enc.write_string("")?;
            enc.write_u8(7);
            enc.write_u32(1);
            StashTab {
                width: 10,
                height: 19,
                items: vec![StashItem {
                    item: Item {
                        base_name: "records/items/materia/a.dbr".into(),
                        stack_count: 15,
                        ..Item::default()
                    },
                    x: 1.0,
                    y: 6.0,
                }],
                decoration: TabDecoration::default(),
            }
            .write(enc, ContainerVersion::new(11).unwrap())
            .map_err(|_| EncodeError::PayloadTooLong { len: 0 })
        })
        .unwrap();
        enc.finish()
    }

    #[test]
    fn a_lossless_model_is_admitted_with_its_baseline() {
        let bytes = stash_bytes();
        let loaded = Loaded::<GstFile>::load(bytes.clone()).unwrap();
        assert_eq!(loaded.baseline(), &bytes[..]);
        assert_eq!(loaded.encode().unwrap(), bytes);
        assert_eq!(loaded.model().transfer_stash().unwrap().tabs.len(), 1);
    }

    #[test]
    fn edits_change_the_encoding_but_not_the_baseline() {
        let bytes = stash_bytes();
        let mut loaded = Loaded::<GstFile>::load(bytes.clone()).unwrap();
        loaded.model_mut().transfer_stash_mut().unwrap().tabs[0]
            .items
            .clear();
        let edited = loaded.encode().unwrap();
        assert_ne!(edited, bytes);
        assert_eq!(loaded.baseline(), &bytes[..]);
        assert!(
            GstFile::parse(&edited)
                .unwrap()
                .transfer_stash()
                .unwrap()
                .tabs[0]
                .items
                .is_empty()
        );
    }

    #[test]
    fn a_model_that_cannot_reproduce_its_file_is_refused() {
        assert_eq!(
            Loaded::<Lossy>::load(vec![1, 2, 3, 4]).unwrap_err(),
            LoadError::NotLossless {
                first_difference: 3
            }
        );
        assert_eq!(
            Loaded::<Truncating>::load(vec![1, 2, 3, 4]).unwrap_err(),
            LoadError::NotLossless {
                first_difference: 3
            }
        );
    }

    #[test]
    fn parse_failures_are_reported_as_such() {
        assert!(matches!(
            Loaded::<PlayerFile>::load(b"not a player file".to_vec()).unwrap_err(),
            LoadError::Parse(_)
        ));
    }
}
