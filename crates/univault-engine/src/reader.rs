// Vendored from tq-univault crates/univault-core/src/reader.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Typed little-endian reader for Titan Quest's key/value save format.
//!
//! Save files are a flat stream of `[i32 length][ASCII key]` entries,
//! each followed by a typed value. Strings are length-prefixed:
//! Windows-1252 for record paths ("cstrings"), UTF-16LE for player-visible
//! names. Ported from `TQVaultAE`'s `TQDataService.cs` (MIT).

use std::fmt;

/// Byte position in a save file; carried in every error so a failure can
/// be located in a hex dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset(pub usize);

impl fmt::Display for Offset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "offset 0x{:X}", self.0)
    }
}

/// Errors surfaced while decoding the raw byte stream.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("unexpected end of data at {at}: wanted {wanted} more bytes")]
    UnexpectedEof { at: Offset, wanted: usize },
    #[error("negative length {length} at {at}")]
    NegativeLength { at: Offset, length: i32 },
    #[error("expected key \"{expected}\" at {at}, found \"{found}\"")]
    KeyMismatch {
        at: Offset,
        expected: &'static str,
        found: String,
    },
}

/// Cursor over a save file's bytes. All reads are little-endian and
/// bounds-checked; nothing is validated beyond shape — callers own the
/// meaning of what they read.
pub struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self::at(data, 0)
    }

    /// A reader positioned at `pos` — typically an offset produced by
    /// [`find_key`].
    #[must_use]
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    #[must_use]
    pub fn pos(&self) -> usize {
        self.pos
    }

    fn take(&mut self, wanted: usize) -> Result<&'a [u8], ReadError> {
        let end = self
            .pos
            .checked_add(wanted)
            .filter(|&e| e <= self.data.len());
        match end {
            Some(end) => {
                let bytes = &self.data[self.pos..end];
                self.pos = end;
                Ok(bytes)
            }
            None => Err(ReadError::UnexpectedEof {
                at: Offset(self.pos),
                wanted,
            }),
        }
    }

    /// # Errors
    /// [`ReadError::UnexpectedEof`] at the end of the data.
    pub fn read_u8(&mut self) -> Result<u8, ReadError> {
        let bytes = self.take(1)?;
        Ok(bytes[0])
    }

    /// # Errors
    /// [`ReadError::UnexpectedEof`] if fewer than 4 bytes remain.
    pub fn read_i32(&mut self) -> Result<i32, ReadError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// # Errors
    /// [`ReadError::UnexpectedEof`] if fewer than 4 bytes remain.
    pub fn read_u32(&mut self) -> Result<u32, ReadError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// # Errors
    /// [`ReadError::UnexpectedEof`] if fewer than `length` bytes remain.
    pub fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], ReadError> {
        self.take(length)
    }

    /// # Errors
    /// [`ReadError::UnexpectedEof`] if fewer than 8 bytes remain.
    pub fn read_i64(&mut self) -> Result<i64, ReadError> {
        let bytes = self.take(8)?;
        Ok(i64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// # Errors
    /// [`ReadError::UnexpectedEof`] if fewer than 2 bytes remain.
    pub fn read_i16(&mut self) -> Result<i16, ReadError> {
        let bytes = self.take(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// # Errors
    /// [`ReadError::UnexpectedEof`] if fewer than 4 bytes remain.
    pub fn read_f32(&mut self) -> Result<f32, ReadError> {
        let bytes = self.take(4)?;
        Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_length(&mut self) -> Result<usize, ReadError> {
        let at = Offset(self.pos);
        let length = self.read_i32()?;
        usize::try_from(length).map_err(|_| ReadError::NegativeLength { at, length })
    }

    /// Reads a length-prefixed Windows-1252 string.
    ///
    /// # Errors
    /// [`ReadError::UnexpectedEof`] or [`ReadError::NegativeLength`] on a
    /// malformed prefix.
    pub fn read_cstring(&mut self) -> Result<String, ReadError> {
        let length = self.read_length()?;
        Ok(decode_windows_1252(self.take(length)?))
    }

    /// Reads a length-prefixed UTF-16LE string; the prefix counts 2-byte
    /// units. Invalid surrogates decode to U+FFFD, matching `TQVaultAE`.
    ///
    /// # Errors
    /// [`ReadError::UnexpectedEof`] or [`ReadError::NegativeLength`] on a
    /// malformed prefix.
    pub fn read_utf16_string(&mut self) -> Result<String, ReadError> {
        let unit_count = self.read_length()?;
        let byte_count = unit_count.checked_mul(2).ok_or(ReadError::UnexpectedEof {
            at: Offset(self.pos),
            wanted: usize::MAX,
        })?;
        let bytes = self.take(byte_count)?;
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair));
        Ok(char::decode_utf16(units)
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect())
    }

    /// Consumes the next key and requires it to be `expected`
    /// (ASCII-case-insensitive, matching `TQVaultAE`'s `ValidateNextString`).
    ///
    /// # Errors
    /// [`ReadError::KeyMismatch`] if a different key is present, plus the
    /// `read_cstring` errors.
    pub fn expect_key(&mut self, expected: &'static str) -> Result<(), ReadError> {
        let at = Offset(self.pos);
        let found = self.read_cstring()?;
        if found.eq_ignore_ascii_case(expected) {
            Ok(())
        } else {
            Err(ReadError::KeyMismatch {
                at,
                expected,
                found,
            })
        }
    }

    /// Whether the next key equals `key`, without consuming it. A stream
    /// too short to hold another key reads as `false`.
    #[must_use]
    pub fn next_key_is(&mut self, key: &str) -> bool {
        let saved = self.pos;
        let matched = self
            .read_cstring()
            .is_ok_and(|found| found.eq_ignore_ascii_case(key));
        self.pos = saved;
        matched
    }
}

/// Finds the byte-exact `[i32 length][key]` pattern from `from` and
/// returns the offset just past the key (i.e. of its value), like
/// `TQVaultAE`'s `BinaryFindKey`.
#[must_use]
pub fn find_key(data: &[u8], key: &str, from: usize) -> Option<usize> {
    let length = i32::try_from(key.len()).ok()?;
    let pattern: Vec<u8> = length
        .to_le_bytes()
        .into_iter()
        .chain(key.bytes())
        .collect();
    data.get(from..)?
        .windows(pattern.len())
        .position(|window| window == pattern)
        .map(|hit| from + hit + pattern.len())
}

/// Windows-1252 mappings for 0x80–0x9F; every other byte matches its
/// Unicode code point.
pub(crate) const WINDOWS_1252_C1: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

/// Decodes Windows-1252 bytes; the inverse of
/// [`crate::writer::encode_windows_1252`].
#[must_use]
pub fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| match byte {
            0x80..=0x9F => WINDOWS_1252_C1[usize::from(byte - 0x80)],
            _ => char::from(byte),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(key: &str) -> Vec<u8> {
        let mut buf = i32::try_from(key.len()).unwrap().to_le_bytes().to_vec();
        buf.extend_from_slice(key.as_bytes());
        buf
    }

    #[test]
    fn read_i32_is_little_endian() {
        let mut reader = ByteReader::new(&[0x2A, 0, 0, 0]);
        assert_eq!(reader.read_i32(), Ok(42));
    }

    #[test]
    fn read_u32_keeps_the_high_bit() {
        let mut reader = ByteReader::new(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(reader.read_u32(), Ok(u32::MAX));
    }

    #[test]
    fn read_i32_past_end_reports_position() {
        let mut reader = ByteReader::new(&[1, 2]);
        assert_eq!(
            reader.read_i32(),
            Err(ReadError::UnexpectedEof {
                at: Offset(0),
                wanted: 4
            })
        );
    }

    #[test]
    fn cstring_decodes_windows_1252_high_bytes() {
        let mut data = 2_i32.to_le_bytes().to_vec();
        data.extend_from_slice(&[0x80, 0xE9]);
        let mut reader = ByteReader::new(&data);
        assert_eq!(reader.read_cstring(), Ok("€é".to_string()));
    }

    #[test]
    fn cstring_rejects_negative_length() {
        let data = (-5_i32).to_le_bytes();
        let mut reader = ByteReader::new(&data);
        assert_eq!(
            reader.read_cstring(),
            Err(ReadError::NegativeLength {
                at: Offset(0),
                length: -5
            })
        );
    }

    #[test]
    fn utf16_prefix_counts_units_not_bytes() {
        let mut data = 4_i32.to_le_bytes().to_vec();
        for unit in "Ajax".encode_utf16() {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        let mut reader = ByteReader::new(&data);
        assert_eq!(reader.read_utf16_string(), Ok("Ajax".to_string()));
    }

    #[test]
    fn expect_key_is_case_insensitive() {
        let data = keyed("beGIN_bloCK");
        let mut reader = ByteReader::new(&data);
        assert_eq!(reader.expect_key("begin_block"), Ok(()));
    }

    #[test]
    fn expect_key_mismatch_names_both_keys() {
        let data = keyed("tempBool");
        let mut reader = ByteReader::new(&data);
        assert_eq!(
            reader.expect_key("size"),
            Err(ReadError::KeyMismatch {
                at: Offset(0),
                expected: "size",
                found: "tempBool".to_string()
            })
        );
    }

    #[test]
    fn next_key_is_does_not_consume() {
        let data = keyed("relicName2");
        let mut reader = ByteReader::new(&data);
        assert!(reader.next_key_is("relicName2"));
        assert_eq!(reader.pos(), 0);
    }

    #[test]
    fn find_key_returns_value_offset() {
        let mut data = vec![0xFF; 3];
        data.extend_from_slice(&keyed("money"));
        data.extend_from_slice(&7_i32.to_le_bytes());
        let value_at = find_key(&data, "money", 0).unwrap();
        assert_eq!(ByteReader::at(&data, value_at).read_i32(), Ok(7));
    }

    #[test]
    fn find_key_is_byte_exact_about_the_length_prefix() {
        let data = keyed("moneyx");
        assert_eq!(find_key(&data, "money", 0), None);
    }
}
