//! The one point where the two games' container formats differ:
//! Titan Quest compresses ARC parts and ARZ records with zlib, Grim
//! Dawn with raw LZ4 blocks. Exactly two codecs exist, so this is an
//! enum rather than a trait; [`crate::arc`] and [`crate::arz`] take a
//! [`Codec`] (directly, or inside an `ArzDialect`) and never name a
//! compression library themselves.

use std::io::Read;

/// Block compression scheme of a container's payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    /// zlib-wrapped deflate, self-delimiting. `TQVaultAE` skips the
    /// two-byte zlib header and inflates the rest as raw deflate; a
    /// zlib-aware decoder over the whole part is the same operation.
    Zlib,
    /// A raw LZ4 block with no frame header, so the decompressed
    /// length has to come from the container. Whether a payload may
    /// be stored raw instead is the container's rule, not the
    /// codec's: Grim Dawn's ARC parts are, its ARZ records never are
    /// (see [`crate::arc`] and [`crate::arz`]).
    Lz4Block,
}

/// Errors from inflating one compressed payload.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("zlib inflate failed: {0}")]
    Zlib(#[source] std::io::Error),
    #[error("lz4 block decode failed: {0}")]
    Lz4(#[source] lz4_flex::block::DecompressError),
    #[error("lz4 block inflated to {actual} bytes, container recorded {expected}")]
    LengthMismatch { expected: usize, actual: usize },
    #[error("lz4 block needs the decompressed length, which this container does not record")]
    LengthRequired,
}

impl Codec {
    /// Inflates `src`, whose decompressed size the container recorded
    /// as `decompressed_len`. `Zlib` streams are self-delimiting and
    /// use the length only as a capacity hint; `Lz4Block` requires it
    /// exactly — a block carries no length of its own, so the
    /// recorded one is the only integrity check.
    ///
    /// # Errors
    /// [`CodecError::Zlib`] or [`CodecError::Lz4`] when the bytes are
    /// not a valid stream for this codec; [`CodecError::LengthMismatch`]
    /// when an LZ4 block inflates to a different size than recorded.
    pub fn decompress(self, src: &[u8], decompressed_len: usize) -> Result<Vec<u8>, CodecError> {
        match self {
            Codec::Zlib => inflate_zlib(src, decompressed_len),
            Codec::Lz4Block => {
                let out =
                    lz4_flex::block::decompress(src, decompressed_len).map_err(CodecError::Lz4)?;
                if out.len() == decompressed_len {
                    Ok(out)
                } else {
                    Err(CodecError::LengthMismatch {
                        expected: decompressed_len,
                        actual: out.len(),
                    })
                }
            }
        }
    }

    /// Inflates `src` when the container records no decompressed size
    /// — Titan Quest's ARZ record table. Only a self-delimiting codec
    /// can do this.
    ///
    /// # Errors
    /// [`CodecError::LengthRequired`] for `Lz4Block`; [`CodecError::Zlib`]
    /// on a bad stream.
    pub fn decompress_unsized(self, src: &[u8]) -> Result<Vec<u8>, CodecError> {
        match self {
            Codec::Zlib => inflate_zlib(src, 0),
            Codec::Lz4Block => Err(CodecError::LengthRequired),
        }
    }

    /// Compresses `src` into a stream that [`Codec::decompress`] with
    /// `src.len()` as the length reproduces. Always a real stream,
    /// even when it is no smaller than the input.
    #[must_use]
    pub fn compress(self, src: &[u8]) -> Vec<u8> {
        match self {
            Codec::Zlib => deflate_zlib(src),
            Codec::Lz4Block => lz4_flex::block::compress(src),
        }
    }
}

fn inflate_zlib(src: &[u8], capacity: usize) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::with_capacity(capacity);
    flate2::read::ZlibDecoder::new(src)
        .read_to_end(&mut out)
        .map_err(CodecError::Zlib)?;
    Ok(out)
}

fn deflate_zlib(src: &[u8]) -> Vec<u8> {
    use std::io::Write as _;

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to a Vec cannot fail; the encoder API just returns io::Result.
    let _ = encoder.write_all(src);
    encoder.finish().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPRESSIBLE: &[u8] = b"abcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabc";

    fn incompressible() -> Vec<u8> {
        (0..=255_u8).collect()
    }

    #[test]
    fn zlib_round_trips_and_shrinks() {
        let packed = Codec::Zlib.compress(COMPRESSIBLE);
        assert!(packed.len() < COMPRESSIBLE.len());
        assert_eq!(
            Codec::Zlib.decompress(&packed, COMPRESSIBLE.len()).unwrap(),
            COMPRESSIBLE
        );
        assert_eq!(
            Codec::Zlib.decompress_unsized(&packed).unwrap(),
            COMPRESSIBLE
        );
    }

    #[test]
    fn lz4_round_trips_and_shrinks() {
        let packed = Codec::Lz4Block.compress(COMPRESSIBLE);
        assert!(packed.len() < COMPRESSIBLE.len());
        assert_eq!(
            Codec::Lz4Block
                .decompress(&packed, COMPRESSIBLE.len())
                .unwrap(),
            COMPRESSIBLE
        );
    }

    #[test]
    fn lz4_never_substitutes_raw_bytes_for_a_block() {
        let raw = incompressible();
        let packed = Codec::Lz4Block.compress(&raw);
        assert!(packed.len() > raw.len());
        assert_eq!(Codec::Lz4Block.decompress(&packed, raw.len()).unwrap(), raw);
        assert!(matches!(
            Codec::Lz4Block.decompress(&raw, raw.len()),
            Err(CodecError::Lz4(_) | CodecError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn lz4_refuses_to_inflate_without_a_length() {
        let packed = Codec::Lz4Block.compress(COMPRESSIBLE);
        assert!(matches!(
            Codec::Lz4Block.decompress_unsized(&packed),
            Err(CodecError::LengthRequired)
        ));
    }

    #[test]
    fn lz4_reports_a_length_larger_than_the_block_inflates_to() {
        let packed = Codec::Lz4Block.compress(COMPRESSIBLE);
        assert!(matches!(
            Codec::Lz4Block.decompress(&packed, COMPRESSIBLE.len() + 7),
            Err(CodecError::LengthMismatch { expected, actual })
                if expected == COMPRESSIBLE.len() + 7 && actual == COMPRESSIBLE.len()
        ));
    }

    #[test]
    fn lz4_reports_a_length_smaller_than_the_block_inflates_to() {
        let packed = Codec::Lz4Block.compress(COMPRESSIBLE);
        assert!(matches!(
            Codec::Lz4Block.decompress(&packed, COMPRESSIBLE.len() - 7),
            Err(CodecError::Lz4(_))
        ));
    }

    #[test]
    fn zlib_reports_garbage() {
        assert!(matches!(
            Codec::Zlib.decompress(b"not zlib", 8),
            Err(CodecError::Zlib(_))
        ));
    }
}
