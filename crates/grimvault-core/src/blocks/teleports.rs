//! Block 6: rift gates (teleport points) discovered per difficulty.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/lists.rs` (`TeleportList`);
//! grim-save-parser's `teleport_list.rs` agrees.

use super::{PerDifficulty, Uid, read_uids, write_uids};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(6);
/// The only layout version either reference supports.
pub const VERSION: u32 = 1;

/// Block 6 at [`VERSION`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Teleports {
    /// Rift gates discovered, per difficulty.
    pub discovered: PerDifficulty<Vec<Uid>>,
}

impl Teleports {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            discovered: PerDifficulty::read(|| read_uids(dec))?,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// Cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(VERSION);
            self.discovered.try_for_each(|uids| write_uids(enc, uids))?;
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
        let teleports = Teleports {
            discovered: PerDifficulty {
                normal: vec![Uid::new([9; 16])],
                elite: vec![Uid::new([8; 16]), Uid::new([7; 16])],
                ultimate: vec![],
            },
        };
        round_trip(&teleports, Teleports::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let teleports = Teleports::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            teleports
        });
    }
}
