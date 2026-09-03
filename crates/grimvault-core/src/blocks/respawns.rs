//! Block 5: respawn points (personal rift / checkpoint ids) known per
//! difficulty, and the active one per difficulty.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/lists.rs` (`RespawnList`);
//! grim-save-parser's `respawn_list.rs` agrees.

use super::{PerDifficulty, Uid, read_uids, write_uids};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(5);
/// The only layout version either reference supports.
pub const VERSION: u32 = 1;

/// Block 5 at [`VERSION`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Respawns {
    /// Respawn points discovered, per difficulty.
    pub discovered: PerDifficulty<Vec<Uid>>,
    /// The active respawn point, per difficulty.
    pub current: PerDifficulty<Uid>,
}

impl Respawns {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let discovered = PerDifficulty::read(|| read_uids(dec))?;
        let current = PerDifficulty::read(|| Uid::read(dec))?;
        Ok(Self {
            discovered,
            current,
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
            self.current.try_for_each(|uid| {
                uid.write(enc);
                Ok(())
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::testing::{open_block, round_trip};

    #[test]
    fn round_trips() {
        let respawns = Respawns {
            discovered: PerDifficulty {
                normal: vec![Uid::new([1; 16]), Uid::new([2; 16])],
                elite: vec![],
                ultimate: vec![Uid::new([3; 16])],
            },
            current: PerDifficulty {
                normal: Uid::new([1; 16]),
                elite: Uid::default(),
                ultimate: Uid::new([3; 16]),
            },
        };
        round_trip(&respawns, Respawns::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let respawns = Respawns::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            respawns
        });
    }
}
