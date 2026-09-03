//! Block 10: quest trigger tokens held per difficulty (the words the
//! game's quest scripts test, e.g. which bosses fell and which choices
//! were made). yagde calls the block `Crucible`, grim-save-parser
//! `TriggerTokens`; the contents are the latter. Present only when the
//! header's data version is at least 7 (yagde `char.rs`).
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/misc.rs` (`Crucible`), whose
//! reader drops every token it reads (it only pushes tokens already
//! present); the layout is confirmed by grim-save-parser's
//! `trigger_tokens.rs`, which keeps them.

use super::{PerDifficulty, read_strings, write_strings};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(10);
/// The only layout version either reference supports.
pub const VERSION: u32 = 2;

/// Block 10 at [`VERSION`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Tokens held, per difficulty, in file order.
    pub per_difficulty: PerDifficulty<Vec<String>>,
}

impl Tokens {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            per_difficulty: PerDifficulty::read(|| read_strings(dec))?,
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
                .try_for_each(|tokens| write_strings(enc, tokens))?;
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
        let tokens = Tokens {
            per_difficulty: PerDifficulty {
                normal: vec!["TUTORIAL_COMPLETE".into(), "BOSS_WARDEN_DEAD".into()],
                elite: vec!["TUTORIAL_COMPLETE".into()],
                ultimate: vec![],
            },
        };
        round_trip(&tokens, Tokens::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let tokens = Tokens::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            tokens
        });
    }
}
