//! Block 17: devotion shrine ids, six lists. Both references read six
//! uid lists and name none of them; three difficulties × two states
//! (restored / discovered, in some order) is the plausible reading and
//! is not asserted here.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/lists.rs` (`ShrineList`);
//! grim-save-parser's `shrine_list.rs` agrees.

use super::{Uid, read_array, read_uids, write_uids};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(17);
/// The only layout version either reference supports.
pub const VERSION: u32 = 2;
/// Number of uid lists in the block.
pub const LISTS: usize = 6;

/// Block 17 at [`VERSION`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shrines {
    /// The six uid lists in file order.
    pub lists: [Vec<Uid>; LISTS],
}

impl Shrines {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            lists: read_array(|| read_uids(dec))?,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// Cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(VERSION);
            self.lists
                .iter()
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
        let shrines = Shrines {
            lists: [
                vec![Uid::new([1; 16])],
                vec![],
                vec![Uid::new([2; 16]), Uid::new([3; 16])],
                vec![],
                vec![],
                vec![Uid::new([4; 16])],
            ],
        };
        round_trip(&shrines, Shrines::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let shrines = Shrines::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            shrines
        });
    }
}
