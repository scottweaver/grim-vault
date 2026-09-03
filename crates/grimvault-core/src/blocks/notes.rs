//! Block 12: lore notes collected, as record paths.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/misc.rs` (`NoteList`);
//! grim-save-parser's `lore_notes.rs` agrees.

use super::{read_strings, write_strings};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(12);
/// The only layout version either reference supports.
pub const VERSION: u32 = 1;

/// Block 12 at [`VERSION`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoreNotes {
    /// Record paths of the notes collected, in file order.
    pub notes: Vec<String>,
}

impl LoreNotes {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            notes: read_strings(dec)?,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// Cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(VERSION);
            write_strings(enc, &self.notes)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::testing::{open_block, round_trip};

    #[test]
    fn round_trips() {
        let notes = LoreNotes {
            notes: vec![
                "records/items/lore/act1/lore_a01_001.dbr".into(),
                "records/items/lore/act1/lore_a01_002.dbr".into(),
            ],
        };
        round_trip(&notes, LoreNotes::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let notes = LoreNotes::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            notes
        });
    }
}
