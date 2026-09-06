//! `formulas.gst`: the blueprints a campaign has learned. Unlike every
//! other shared file it is not obfuscated at all — a plaintext
//! Titan-Quest-style key/value stream: `begin_block` and the marker
//! `0xB01DFACE`, `formulasVersion` (3), `numEntries`, `expansionStatus`
//! (one byte), then `itemName` + `formulaRead` per entry, and
//! `end_block` with `0xDEADC0DE`. Every key is a `u32`-length-prefixed
//! ASCII string, as are the record paths. Established 2026-09-06 on the
//! user's own files (`docs/format-references.md`); GD Stash's reader
//! (eyes-only) walks the same keys and also accepts a version 2 without
//! the expansion byte, which no file here has, so only version 3 is
//! typed.
//!
//! Lossless by construction: the two markers are checked rather than
//! carried, the read flag is typed and any other value refuses the
//! file, trailing bytes refuse it too, and keys must match exactly
//! (not case-insensitively), so an accepted file re-encodes
//! byte-for-byte ([`crate::loaded`]). The game appends a newly learned
//! blueprint at the end with the flag at 0 (seen in the diff of a
//! file the game rewrote), which is what [`Formulas::add`] does.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use univault_engine::ids::normalize;
use univault_engine::reader::{ByteReader, Offset, ReadError};
use univault_engine::writer::{encode_windows_1252, write_u32};

use crate::block::length_word;
use crate::crypto::EncodeError;
use crate::gst::Added;
use crate::loaded::SaveCodec;

const BEGIN_BLOCK: (&str, u32) = ("begin_block", 0xB01D_FACE);
const END_BLOCK: (&str, u32) = ("end_block", 0xDEAD_C0DE);
const VERSION_KEY: &str = "formulasVersion";
const COUNT_KEY: &str = "numEntries";
const EXPANSION_KEY: &str = "expansionStatus";
const RECORD_KEY: &str = "itemName";
const READ_KEY: &str = "formulaRead";

/// Why a `formulas.gst` could not be parsed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FormulasError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("expected key {expected:?} at {at}, found {found:?}")]
    Key {
        at: Offset,
        expected: &'static str,
        found: String,
    },
    #[error("{key} marker is {found:#010x}, not {expected:#010x}")]
    Marker {
        key: &'static str,
        expected: u32,
        found: u32,
    },
    #[error("formulasVersion {version} has no known layout")]
    UnsupportedVersion { version: u32 },
    /// The flag is a boolean in every sample; another value would be a
    /// layout this crate does not know, not a third state.
    #[error("blueprint {record:?} has read flag {value}, not 0 or 1")]
    UnknownReadFlag { record: String, value: u32 },
    #[error("{extra} byte(s) follow end_block")]
    TrailingBytes { extra: usize },
}

/// Version of the file. Only [`Self::SUPPORTED`] has a sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FormulasVersion(u32);

impl FormulasVersion {
    /// The version the current game writes.
    pub const SUPPORTED: u32 = 3;

    /// Validates a raw version word.
    ///
    /// # Errors
    /// [`FormulasError::UnsupportedVersion`] for anything but
    /// [`Self::SUPPORTED`].
    pub fn new(raw: u32) -> Result<Self, FormulasError> {
        if raw == Self::SUPPORTED {
            Ok(Self(raw))
        } else {
            Err(FormulasError::UnsupportedVersion { version: raw })
        }
    }

    /// The raw version as stored in the file.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for FormulasVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Whether the player has looked at a blueprint since learning it:
/// the game writes 0 for one just learned (its "new" badge) and 1 once
/// viewed. Serialized as a boolean in this app's interchange documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "bool", from = "bool")]
pub enum FormulaRead {
    Unread,
    Read,
}

impl FormulaRead {
    fn parse(record: &str, value: u32) -> Result<Self, FormulasError> {
        match value {
            0 => Ok(Self::Unread),
            1 => Ok(Self::Read),
            value => Err(FormulasError::UnknownReadFlag {
                record: record.to_owned(),
                value,
            }),
        }
    }

    const fn raw(self) -> u32 {
        match self {
            Self::Unread => 0,
            Self::Read => 1,
        }
    }
}

impl From<FormulaRead> for bool {
    fn from(read: FormulaRead) -> Self {
        matches!(read, FormulaRead::Read)
    }
}

impl From<bool> for FormulaRead {
    fn from(read: bool) -> Self {
        if read { Self::Read } else { Self::Unread }
    }
}

/// One learned blueprint: the record path of its `ItemArtifactFormula`
/// record and the read flag. The file keeps nothing else per entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlueprintEntry {
    pub record: String,
    pub read: FormulaRead,
}

/// The parsed file: the version, the expansion-status byte (7 in the
/// user's `.gst`, 3 in the Forgotten-Gods-level `.dst`, the same
/// values block 18 carries), and the entries in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formulas {
    pub version: FormulasVersion,
    pub expansion_status: u8,
    pub entries: Vec<BlueprintEntry>,
}

impl Formulas {
    /// Parses a whole file image.
    ///
    /// # Errors
    /// [`FormulasError`] for any deviation from the layout above.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormulasError> {
        let mut reader = ByteReader::new(bytes);
        expect_marker(&mut reader, BEGIN_BLOCK)?;
        expect_key(&mut reader, VERSION_KEY)?;
        let version = FormulasVersion::new(reader.read_u32()?)?;
        expect_key(&mut reader, COUNT_KEY)?;
        let count = reader.read_u32()?;
        expect_key(&mut reader, EXPANSION_KEY)?;
        let expansion_status = reader.read_u8()?;
        let entries = (0..count)
            .map(|_| read_entry(&mut reader))
            .collect::<Result<Vec<_>, _>>()?;
        expect_marker(&mut reader, END_BLOCK)?;
        let extra = bytes.len() - reader.pos();
        if extra > 0 {
            return Err(FormulasError::TrailingBytes { extra });
        }
        Ok(Self {
            version,
            expansion_status,
            entries,
        })
    }

    /// Re-encodes the file; byte-identical to the input when unmodified.
    ///
    /// # Errors
    /// [`EncodeError::PayloadTooLong`] when a record path or the entry
    /// count exceeds a `u32` length prefix.
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut out = Vec::new();
        write_marker(&mut out, BEGIN_BLOCK)?;
        write_key(&mut out, VERSION_KEY)?;
        write_u32(&mut out, self.version.raw());
        write_key(&mut out, COUNT_KEY)?;
        write_u32(&mut out, length_word(self.entries.len())?);
        write_key(&mut out, EXPANSION_KEY)?;
        out.push(self.expansion_status);
        for entry in &self.entries {
            write_key(&mut out, RECORD_KEY)?;
            write_text(&mut out, &entry.record)?;
            write_key(&mut out, READ_KEY)?;
            write_u32(&mut out, entry.read.raw());
        }
        write_marker(&mut out, END_BLOCK)?;
        Ok(out)
    }

    /// The position of a record's entry; record paths compare the way
    /// the game's database keys do (case-insensitive, either slash).
    #[must_use]
    pub fn position_of(&self, record: &str) -> Option<usize> {
        let wanted = normalize(record);
        self.entries
            .iter()
            .position(|entry| normalize(&entry.record) == wanted)
    }

    #[must_use]
    pub fn contains(&self, record: &str) -> bool {
        self.position_of(record).is_some()
    }

    /// Appends `entry` unless its record is already listed — the file
    /// is a set the game only ever grows, and it appends too.
    pub fn add(&mut self, entry: BlueprintEntry) -> Added {
        if self.contains(&entry.record) {
            return Added::AlreadyKnown;
        }
        self.entries.push(entry);
        Added::Added
    }
}

impl SaveCodec for Formulas {
    type ParseError = FormulasError;
    type EncodeError = EncodeError;

    fn parse(bytes: &[u8]) -> Result<Self, FormulasError> {
        Formulas::parse(bytes)
    }

    fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        Formulas::encode(self)
    }
}

fn read_entry(reader: &mut ByteReader<'_>) -> Result<BlueprintEntry, FormulasError> {
    expect_key(reader, RECORD_KEY)?;
    let record = reader.read_cstring()?;
    expect_key(reader, READ_KEY)?;
    let read = FormulaRead::parse(&record, reader.read_u32()?)?;
    Ok(BlueprintEntry { record, read })
}

/// A key, matched exactly: the game writes them in one spelling and a
/// re-encode must reproduce it.
fn expect_key(reader: &mut ByteReader<'_>, expected: &'static str) -> Result<(), FormulasError> {
    let at = Offset(reader.pos());
    let found = reader.read_cstring()?;
    if found == expected {
        Ok(())
    } else {
        Err(FormulasError::Key {
            at,
            expected,
            found,
        })
    }
}

fn expect_marker(
    reader: &mut ByteReader<'_>,
    (key, expected): (&'static str, u32),
) -> Result<(), FormulasError> {
    expect_key(reader, key)?;
    let found = reader.read_u32()?;
    if found == expected {
        Ok(())
    } else {
        Err(FormulasError::Marker {
            key,
            expected,
            found,
        })
    }
}

fn write_key(out: &mut Vec<u8>, key: &str) -> Result<(), EncodeError> {
    write_text(out, key)
}

fn write_marker(out: &mut Vec<u8>, (key, marker): (&str, u32)) -> Result<(), EncodeError> {
    write_key(out, key)?;
    write_u32(out, marker);
    Ok(())
}

/// The inverse of `read_cstring`: a `u32` length and Windows-1252
/// bytes, refusing rather than truncating an overlong string.
fn write_text(out: &mut Vec<u8>, text: &str) -> Result<(), EncodeError> {
    let bytes = encode_windows_1252(text);
    write_u32(out, length_word(bytes.len())?);
    out.extend_from_slice(&bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUIRES_BOOTS: &str =
        "records/items/crafting/blueprints/armor/craft_feet_squiresboots02.dbr";
    const SEAL: &str = "records/items/crafting/blueprints/relics/craft_relic_sealnight.dbr";

    fn keyed(key: &str) -> Vec<u8> {
        let mut out = Vec::new();
        write_key(&mut out, key).unwrap();
        out
    }

    /// The bytes the game writes, assembled independently of `encode`.
    fn game_image(entries: &[(&str, u32)]) -> Vec<u8> {
        let mut out = keyed("begin_block");
        out.extend_from_slice(&[0xCE, 0xFA, 0x1D, 0xB0]);
        out.extend(keyed("formulasVersion"));
        out.extend_from_slice(&3_u32.to_le_bytes());
        out.extend(keyed("numEntries"));
        out.extend_from_slice(&u32::try_from(entries.len()).unwrap().to_le_bytes());
        out.extend(keyed("expansionStatus"));
        out.push(7);
        for (record, read) in entries {
            out.extend(keyed("itemName"));
            out.extend(keyed(record));
            out.extend(keyed("formulaRead"));
            out.extend_from_slice(&read.to_le_bytes());
        }
        out.extend(keyed("end_block"));
        out.extend_from_slice(&[0xDE, 0xC0, 0xAD, 0xDE]);
        out
    }

    #[test]
    fn the_games_layout_parses_and_re_encodes_byte_for_byte() {
        let bytes = game_image(&[(SQUIRES_BOOTS, 1), (SEAL, 0)]);
        let formulas = Formulas::parse(&bytes).unwrap();
        assert_eq!(formulas.version, FormulasVersion::new(3).unwrap());
        assert_eq!(formulas.expansion_status, 7);
        assert_eq!(
            formulas.entries,
            vec![
                BlueprintEntry {
                    record: SQUIRES_BOOTS.into(),
                    read: FormulaRead::Read
                },
                BlueprintEntry {
                    record: SEAL.into(),
                    read: FormulaRead::Unread
                },
            ]
        );
        assert_eq!(formulas.encode().unwrap(), bytes);
        assert_eq!(
            crate::loaded::Loaded::<Formulas>::load(bytes.clone())
                .unwrap()
                .baseline(),
            &bytes[..]
        );
    }

    #[test]
    fn an_empty_list_round_trips() {
        let bytes = game_image(&[]);
        let formulas = Formulas::parse(&bytes).unwrap();
        assert!(formulas.entries.is_empty());
        assert_eq!(formulas.encode().unwrap(), bytes);
    }

    #[test]
    fn add_appends_unread_once_and_compares_records_like_the_database() {
        let mut formulas = Formulas::parse(&game_image(&[(SQUIRES_BOOTS, 1)])).unwrap();
        let entry = |record: &str| BlueprintEntry {
            record: record.into(),
            read: FormulaRead::Unread,
        };
        assert_eq!(formulas.add(entry(SEAL)), Added::Added);
        assert_eq!(formulas.add(entry(SEAL)), Added::AlreadyKnown);
        assert_eq!(
            formulas.add(entry(&SQUIRES_BOOTS.to_uppercase().replace('/', "\\"))),
            Added::AlreadyKnown
        );
        assert_eq!(formulas.entries.len(), 2);
        assert_eq!(formulas.position_of(SEAL), Some(1));
        assert!(!formulas.contains("records/items/other.dbr"));
        let reparsed = Formulas::parse(&formulas.encode().unwrap()).unwrap();
        assert_eq!(reparsed, formulas);
    }

    #[test]
    fn every_deviation_from_the_layout_is_refused() {
        let good = game_image(&[(SEAL, 1)]);

        let mut bad_marker = good.clone();
        bad_marker[15] ^= 0x01;
        assert_eq!(
            Formulas::parse(&bad_marker),
            Err(FormulasError::Marker {
                key: "begin_block",
                expected: 0xB01D_FACE,
                found: 0xB01D_FACF,
            })
        );

        let mut wrong_case = good.clone();
        let key_at = keyed("begin_block").len() + 4 + 4;
        assert_eq!(&wrong_case[key_at..key_at + 15], b"formulasVersion");
        wrong_case[key_at] = b'F';
        assert!(matches!(
            Formulas::parse(&wrong_case),
            Err(FormulasError::Key {
                expected: "formulasVersion",
                ..
            })
        ));

        let mut version_2 = good.clone();
        let version_at = key_at + 15;
        assert_eq!(version_2[version_at], 3);
        version_2[version_at] = 2;
        assert_eq!(
            Formulas::parse(&version_2),
            Err(FormulasError::UnsupportedVersion { version: 2 })
        );

        let flag_2 = game_image(&[(SEAL, 2)]);
        assert_eq!(
            Formulas::parse(&flag_2),
            Err(FormulasError::UnknownReadFlag {
                record: SEAL.into(),
                value: 2
            })
        );

        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            Formulas::parse(&trailing),
            Err(FormulasError::TrailingBytes { extra: 1 })
        );

        assert!(matches!(
            Formulas::parse(&good[..good.len() - 2]),
            Err(FormulasError::Read(ReadError::UnexpectedEof { .. }))
        ));
        assert!(matches!(
            Formulas::parse(b"\x02\x00\x00\x00GD"),
            Err(FormulasError::Key {
                expected: "begin_block",
                ..
            })
        ));
    }

    #[test]
    fn the_read_flag_is_a_boolean_on_the_wire_and_in_json() {
        assert_eq!(serde_json::to_string(&FormulaRead::Read).unwrap(), "true");
        assert_eq!(
            serde_json::from_str::<FormulaRead>("false").unwrap(),
            FormulaRead::Unread
        );
        assert_eq!(FormulaRead::Read.raw(), 1);
        assert_eq!(FormulaRead::Unread.raw(), 0);
    }

    #[test]
    fn non_ascii_record_bytes_survive_a_round_trip() {
        let mut bytes = game_image(&[]);
        let end = keyed("end_block").len() + 4;
        let body_end = bytes.len() - end;
        let count_at = keyed("begin_block").len()
            + 4
            + keyed("formulasVersion").len()
            + 4
            + keyed("numEntries").len();
        bytes[count_at] = 1;
        let mut entry = keyed("itemName");
        entry.extend_from_slice(&3_u32.to_le_bytes());
        entry.extend_from_slice(&[b'a', 0xE9, 0x80]);
        entry.extend(keyed("formulaRead"));
        entry.extend_from_slice(&1_u32.to_le_bytes());
        bytes.splice(body_end..body_end, entry);
        let formulas = Formulas::parse(&bytes).unwrap();
        assert_eq!(formulas.entries[0].record, "aé€");
        assert_eq!(formulas.encode().unwrap(), bytes);
    }
}
