//! Block 2: level, experience, unspent points and base attributes.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/info.rs` (`Bio`);
//! grim-save-parser's `character_bio.rs` agrees on every field.

use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(2);
/// The only layout version either reference supports.
pub const VERSION: u32 = 8;

/// Block 2 at [`VERSION`].
#[derive(Clone, Debug, PartialEq)]
pub struct Bio {
    /// Character level (also in the file header).
    pub level: u32,
    /// Total experience.
    pub experience: u32,
    /// Attribute points not yet spent.
    pub attribute_points_unspent: u32,
    /// Skill points not yet spent.
    pub skill_points_unspent: u32,
    /// Devotion points not yet spent.
    pub devotion_points_unspent: u32,
    /// Devotion points unlocked in total.
    pub total_devotion_unlocked: u32,
    /// Base physique.
    pub physique: f32,
    /// Base cunning.
    pub cunning: f32,
    /// Base spirit.
    pub spirit: f32,
    /// Current health.
    pub health: f32,
    /// Current energy.
    pub energy: f32,
}

impl Bio {
    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            level: dec.read_u32()?,
            experience: dec.read_u32()?,
            attribute_points_unspent: dec.read_u32()?,
            skill_points_unspent: dec.read_u32()?,
            devotion_points_unspent: dec.read_u32()?,
            total_devotion_unlocked: dec.read_u32()?,
            physique: dec.read_f32()?,
            cunning: dec.read_f32()?,
            spirit: dec.read_f32()?,
            health: dec.read_f32()?,
            energy: dec.read_f32()?,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// Cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(VERSION);
            enc.write_u32(self.level);
            enc.write_u32(self.experience);
            enc.write_u32(self.attribute_points_unspent);
            enc.write_u32(self.skill_points_unspent);
            enc.write_u32(self.devotion_points_unspent);
            enc.write_u32(self.total_devotion_unlocked);
            enc.write_f32(self.physique);
            enc.write_f32(self.cunning);
            enc.write_f32(self.spirit);
            enc.write_f32(self.health);
            enc.write_f32(self.energy);
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
        let bio = Bio {
            level: 47,
            experience: 1_234_567,
            attribute_points_unspent: 3,
            skill_points_unspent: 5,
            devotion_points_unspent: 1,
            total_devotion_unlocked: 40,
            physique: 458.0,
            cunning: 50.0,
            spirit: 178.0,
            health: 6_120.5,
            energy: 1_010.25,
        };
        round_trip(&bio, Bio::write, |dec| {
            assert_eq!(open_block(dec, BLOCK_ID), VERSION);
            let bio = Bio::read_body(dec).unwrap();
            dec.read_block_end().unwrap();
            bio
        });
    }
}
