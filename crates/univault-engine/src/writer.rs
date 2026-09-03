// Vendored from tq-univault crates/univault-core/src/writer.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Little-endian encoders for the key/value save format — the write
//! side of [`crate::reader`]. Mirrors `TQVaultAE`'s `WriteCString`
//! (MIT): strings are length-prefixed Windows-1252.

use crate::reader::WINDOWS_1252_C1;

pub fn write_i32(buf: &mut Vec<u8>, value: i32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

pub fn write_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

pub fn write_f32(buf: &mut Vec<u8>, value: f32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

pub fn write_i64(buf: &mut Vec<u8>, value: i64) {
    buf.extend_from_slice(&value.to_le_bytes());
}

/// Writes a length-prefixed Windows-1252 string (keys and record
/// paths alike).
pub fn write_cstring(buf: &mut Vec<u8>, value: &str) {
    let encoded = encode_windows_1252(value);
    write_i32(buf, i32::try_from(encoded.len()).unwrap_or(0));
    buf.extend_from_slice(&encoded);
}

/// Writes `key` followed by an `i32` value — the format's most common
/// pairing.
pub fn write_keyed_i32(buf: &mut Vec<u8>, key: &str, value: i32) {
    write_cstring(buf, key);
    write_i32(buf, value);
}

/// Inverse of [`crate::reader::decode_windows_1252`]: characters
/// outside the codepage become `?`, like .NET's best-fit-off
/// encoder.
#[must_use]
pub fn encode_windows_1252(text: &str) -> Vec<u8> {
    text.chars()
        .map(|character| {
            WINDOWS_1252_C1
                .iter()
                .zip(0x80_u8..)
                .find(|&(&mapped, _)| mapped == character)
                .map_or_else(
                    || u8::try_from(u32::from(character)).unwrap_or(b'?'),
                    |(_, byte)| byte,
                )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::ByteReader;

    #[test]
    fn cstring_round_trips_through_the_reader() {
        let mut buf = Vec::new();
        write_cstring(&mut buf, "baseName");
        write_cstring(&mut buf, "€é — em");
        let mut reader = ByteReader::new(&buf);
        assert_eq!(reader.read_cstring().unwrap(), "baseName");
        assert_eq!(reader.read_cstring().unwrap(), "€é — em");
    }

    #[test]
    fn unencodable_characters_become_question_marks() {
        assert_eq!(encode_windows_1252("Χαλκός"), b"??????".to_vec());
    }

    #[test]
    fn keyed_i32_matches_the_wire_shape() {
        let mut buf = Vec::new();
        write_keyed_i32(&mut buf, "seed", 42);
        let mut reader = ByteReader::new(&buf);
        reader.expect_key("seed").unwrap();
        assert_eq!(reader.read_i32(), Ok(42));
    }

    #[test]
    fn u32_round_trips_through_the_reader() {
        let mut buf = Vec::new();
        write_u32(&mut buf, 0x8000_0001);
        assert_eq!(ByteReader::new(&buf).read_u32(), Ok(0x8000_0001));
    }
}
