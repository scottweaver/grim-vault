//! Block 15: tutorial pages already shown.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/misc.rs` (`TutorialPages`);
//! grim-save-parser's `tutorial_pages.rs` agrees.

use super::{read_vec, write_vec};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(15);
/// The only layout version either reference supports.
pub const VERSION: u32 = 1;

/// Block 15 at [`VERSION`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tutorials {
    /// Page ids shown, in file order.
    pub pages: Vec<u32>,
}

impl Tutorials {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            pages: read_vec(dec, Decoder::read_u32)?,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// Cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(VERSION);
            write_vec(enc, &self.pages, |enc, &page| {
                enc.write_u32(page);
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
        let tutorials = Tutorials {
            pages: vec![1, 2, 3, 5, 8, 13, 21, 34],
        };
        round_trip(&tutorials, Tutorials::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let tutorials = Tutorials::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            tutorials
        });
    }
}
