//! Block 13: faction standings.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/misc.rs` (`FactionList`,
//! `Faction`); grim-save-parser's `faction_pack.rs` / `faction_data.rs`
//! agree on every field. Neither explains the leading word beyond
//! calling it `faction`.

use super::{read_vec, write_vec};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(13);
/// The only layout version either reference supports.
pub const VERSION: u32 = 5;

/// One faction's standing. `modified` and `unlocked` are bytes both
/// references read without interpreting; kept as read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Faction {
    /// Byte both references call `modified`.
    pub modified: u8,
    /// Byte both references call `unlocked`.
    pub unlocked: u8,
    /// Reputation value (negative for hostile factions).
    pub value: f32,
    /// Positive reputation boost.
    pub positive_boost: f32,
    /// Negative reputation boost.
    pub negative_boost: f32,
}

impl Faction {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            modified: dec.read_u8()?,
            unlocked: dec.read_u8()?,
            value: dec.read_f32()?,
            positive_boost: dec.read_f32()?,
            negative_boost: dec.read_f32()?,
        })
    }

    fn write(&self, enc: &mut Encoder) {
        enc.write_u8(self.modified);
        enc.write_u8(self.unlocked);
        enc.write_f32(self.value);
        enc.write_f32(self.positive_boost);
        enc.write_f32(self.negative_boost);
    }
}

/// Block 13 at [`VERSION`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Factions {
    /// Word preceding the list; both references name it `faction` and
    /// neither interprets it.
    pub faction: u32,
    /// Standings in file order (the index is the game's faction index).
    pub factions: Vec<Faction>,
}

impl Factions {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let faction = dec.read_u32()?;
        let factions = read_vec(dec, Faction::read)?;
        Ok(Self { faction, factions })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// Cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(VERSION);
            enc.write_u32(self.faction);
            write_vec(enc, &self.factions, |enc, faction| {
                faction.write(enc);
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
        let factions = Factions {
            faction: 1,
            factions: vec![
                Faction {
                    modified: 1,
                    unlocked: 1,
                    value: 15_000.0,
                    positive_boost: 0.1,
                    negative_boost: 0.0,
                },
                Faction {
                    modified: 0,
                    unlocked: 1,
                    value: -20_000.0,
                    positive_boost: 0.0,
                    negative_boost: 0.25,
                },
            ],
        };
        round_trip(&factions, Factions::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let factions = Factions::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            factions
        });
    }
}
