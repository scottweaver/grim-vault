//! Block 7: map markers per difficulty.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/lists.rs` (`MarkerList`);
//! grim-save-parser's `marker_list.rs` agrees.

use super::{PerDifficulty, Uid, read_uids, write_uids};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(7);
/// The only layout version either reference supports.
pub const VERSION: u32 = 1;

/// Block 7 at [`VERSION`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Markers {
    /// Marker ids, per difficulty.
    pub per_difficulty: PerDifficulty<Vec<Uid>>,
}

impl Markers {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            per_difficulty: PerDifficulty::read(|| read_uids(dec))?,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// Cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(VERSION);
            self.per_difficulty
                .try_for_each(|uids| write_uids(enc, uids))?;
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
        let markers = Markers {
            per_difficulty: PerDifficulty {
                normal: vec![],
                elite: vec![Uid::new([0xAB; 16])],
                ultimate: vec![Uid::new([0xCD; 16]), Uid::new([0xEF; 16])],
            },
        };
        round_trip(&markers, Markers::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let markers = Markers::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            markers
        });
    }
}
