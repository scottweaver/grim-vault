// Vendored from tq-univault crates/univault-core/src/arz.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Reader for the game's `database.arz` — the compressed record
//! database holding item stats, classifications, and the localization
//! tags that name things — plus [`compose`], which serializes records
//! into **new** database images for an app's own mod bundles. The
//! game's databases remain read-only reference data per
//! ARCHITECTURE.md: nothing here ever rewrites them. Reader ported
//! from `TQVaultAE`'s `ArzFileProvider.cs` / `RecordInfoProvider.cs`
//! (MIT); writer layout from the MIT `TQArchive-Wrapper` reference.
//!
//! Layout: a 24-byte header (six dwords: format tag, record-table
//! start / size / count, string-table start / size), a string table
//! (`i32` count, then length-prefixed strings), and a record table
//! whose variable-length entries point at compressed record payloads.
//! Stored payload offsets are relative to the header end, so
//! [`HEADER_SIZE`] is added on read. Payloads decompress lazily, one
//! record at a time.
//!
//! Titan Quest and Grim Dawn share the header, string table, and
//! decompressed payload byte for byte; an [`ArzDialect`] captures the
//! three things that differ (codec, one extra record-table field, the
//! header's leading dword).

use std::collections::HashMap;

use crate::codec::{Codec, CodecError};
use crate::ids::{RecordId, normalize};
use crate::reader::{ByteReader, ReadError};
use crate::writer::{write_cstring, write_f32, write_i32, write_i64, write_u32};

/// Size of the ARZ header; stored record offsets are relative to it.
pub const HEADER_SIZE: usize = 24;

/// The per-game shape of an ARZ file. Values are verified against
/// real installs: Titan Quest AE's `database.arz` opens with
/// `04 00 03 00`, Grim Dawn's with `02 00 03 00` (a `u16` tag then
/// `u16` version 3 in both).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArzDialect {
    codec: Codec,
    entry_has_decompressed_size: bool,
    first_dword: u32,
}

impl ArzDialect {
    /// zlib records; record-table entries carry `i32 id; type; i32
    /// offset; i32 compressed size; i64 timestamp`.
    #[must_use]
    pub const fn titan_quest() -> Self {
        Self {
            codec: Codec::Zlib,
            entry_has_decompressed_size: false,
            first_dword: 0x0003_0004,
        }
    }

    /// Raw LZ4 block records; entries add a `u32 decompressed size`
    /// between the compressed size and the timestamp. Records are
    /// always compressed: about 0.1% of a real database's blocks are
    /// exactly as long as their payload, and every one of them is a
    /// genuine LZ4 block, so unlike ARC parts there is no stored-raw
    /// case here.
    #[must_use]
    pub const fn grim_dawn() -> Self {
        Self {
            codec: Codec::Lz4Block,
            entry_has_decompressed_size: true,
            first_dword: 0x0003_0002,
        }
    }

    #[must_use]
    pub const fn codec(self) -> Codec {
        self.codec
    }

    /// The header's leading dword, checked on parse and written by
    /// [`compose`].
    #[must_use]
    pub const fn first_dword(self) -> u32 {
        self.first_dword
    }
}

/// A parsed `database.arz`: the record index plus the raw bytes, from
/// which individual records decompress on demand.
pub struct ArzFile {
    data: Vec<u8>,
    dialect: ArzDialect,
    strings: Vec<String>,
    entries: HashMap<String, RecordEntry>,
    /// Normalized ids in record-table order, so re-serialization and
    /// iteration are deterministic.
    order: Vec<String>,
}

struct RecordEntry {
    id: RecordId,
    record_type: String,
    payload_offset: usize,
    payload_size: usize,
    /// Present exactly when the dialect records it.
    decompressed_size: Option<usize>,
    timestamp: i64,
}

/// One decompressed database record: a set of named, typed variables
/// in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct DbRecord {
    pub id: RecordId,
    /// The record's class string, e.g. `ArmorProtective_Head`.
    pub record_type: String,
    variables: Vec<DbVariable>,
}

impl DbRecord {
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&DbVariable> {
        self.variables.iter().find(|variable| variable.name == name)
    }

    pub fn variables(&self) -> impl Iterator<Item = &DbVariable> {
        self.variables.iter()
    }

    /// Replaces (or appends) a variable — the mod-patching edit.
    pub fn set_variable(&mut self, variable: DbVariable) {
        match self
            .variables
            .iter_mut()
            .find(|existing| existing.name == variable.name)
        {
            Some(existing) => *existing = variable,
            None => self.variables.push(variable),
        }
    }

    /// First value of a string variable — the common case for tag
    /// lookups like `description` or `itemNameTag`.
    #[must_use]
    pub fn string(&self, name: &str) -> Option<&str> {
        match &self.variable(name)?.values {
            DbValues::Strings(values) => values.first().map(String::as_str),
            DbValues::Integers(_) | DbValues::Floats(_) | DbValues::Booleans(_) => None,
        }
    }

    /// First value of an integer variable.
    #[must_use]
    pub fn integer(&self, name: &str) -> Option<i32> {
        match &self.variable(name)?.values {
            DbValues::Integers(values) => values.first().copied(),
            DbValues::Strings(_) | DbValues::Floats(_) | DbValues::Booleans(_) => None,
        }
    }

    /// First value of a boolean variable.
    #[must_use]
    pub fn boolean(&self, name: &str) -> Option<bool> {
        match &self.variable(name)?.values {
            DbValues::Booleans(values) => values.first().copied(),
            DbValues::Strings(_) | DbValues::Floats(_) | DbValues::Integers(_) => None,
        }
    }

    /// First value of a float variable.
    #[must_use]
    pub fn float(&self, name: &str) -> Option<f32> {
        match &self.variable(name)?.values {
            DbValues::Floats(values) => values.first().copied(),
            DbValues::Strings(_) | DbValues::Integers(_) | DbValues::Booleans(_) => None,
        }
    }
}

/// A record variable: its name and homogeneous typed values (arrays
/// are common; single values are one-element arrays).
#[derive(Debug, Clone, PartialEq)]
pub struct DbVariable {
    pub name: String,
    pub values: DbValues,
}

impl DbVariable {
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.values {
            DbValues::Integers(values) => values.len(),
            DbValues::Floats(values) => values.len(),
            DbValues::Strings(values) => values.len(),
            DbValues::Booleans(values) => values.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The four value types the format defines (0=int, 1=float,
/// 2=string-table index, 3=bool-as-i32).
#[derive(Debug, Clone, PartialEq)]
pub enum DbValues {
    Integers(Vec<i32>),
    Floats(Vec<f32>),
    Strings(Vec<String>),
    Booleans(Vec<bool>),
}

/// Errors from parsing an ARZ file or one of its records.
#[derive(Debug, thiserror::Error)]
pub enum ArzError {
    #[error("header starts with {found:#010x}, expected {expected:#010x} for this dialect")]
    DialectMismatch { found: u32, expected: u32 },
    #[error("invalid header field {field}: {value}")]
    InvalidHeader { field: &'static str, value: i32 },
    #[error("invalid {what} count {count}")]
    InvalidCount { what: &'static str, count: i32 },
    #[error("string index {index} out of range ({len} strings)")]
    StringIndex { index: i32, len: usize },
    #[error("record table entry {index} has an empty record path")]
    EmptyRecordPath { index: usize },
    #[error("record {id}: payload range outside the file")]
    PayloadOutOfRange { id: String },
    #[error("record {id}: decompression failed: {source}")]
    Decompress { id: String, source: CodecError },
    #[error("record {id}: payload length {len} is not a multiple of 4")]
    UnalignedPayload { id: String, len: usize },
    #[error("record {id}: variable {name}: invalid data type {data_type}")]
    InvalidDataType {
        id: String,
        name: String,
        data_type: i16,
    },
    #[error("record {id}: variable {name}: invalid value count {count}")]
    InvalidValueCount {
        id: String,
        name: String,
        count: i16,
    },
    #[error(transparent)]
    Read(#[from] ReadError),
}

impl ArzFile {
    /// Parses the header, string table, and record index. Record
    /// payloads stay compressed until [`ArzFile::record`] asks for
    /// them.
    ///
    /// # Errors
    /// [`ArzError::DialectMismatch`] when the leading dword is not the
    /// dialect's, plus any structural error in the header or tables.
    pub fn parse(data: Vec<u8>, dialect: ArzDialect) -> Result<Self, ArzError> {
        let mut header = ByteReader::new(&data);
        let first_dword = header.read_u32()?;
        if first_dword != dialect.first_dword {
            return Err(ArzError::DialectMismatch {
                found: first_dword,
                expected: dialect.first_dword,
            });
        }
        let record_table_start = offset_field(header.read_i32()?, "record table start")?;
        header.read_i32()?;
        let record_count = header.read_i32()?;
        let record_count = usize::try_from(record_count).map_err(|_| ArzError::InvalidCount {
            what: "record",
            count: record_count,
        })?;
        let string_table_start = offset_field(header.read_i32()?, "string table start")?;

        let strings = read_string_table(&data, string_table_start)?;
        let (entries, order) =
            read_record_table(&data, record_table_start, record_count, &strings, dialect)?;
        Ok(Self {
            data,
            dialect,
            strings,
            entries,
            order,
        })
    }

    #[must_use]
    pub fn dialect(&self) -> ArzDialect {
        self.dialect
    }

    /// Looks up and decompresses a record. `None` when the id is not
    /// in the database (ids match case-insensitively with `/` and `\`
    /// interchangeable, like `TQVaultAE`'s `NormalizeRecordPath`).
    #[must_use]
    pub fn record(&self, id: &RecordId) -> Option<Result<DbRecord, ArzError>> {
        let entry = self.entries.get(&normalize(id.as_str()))?;
        Some(self.decompress(entry))
    }

    /// Record ids in record-table order.
    pub fn record_ids(&self) -> impl Iterator<Item = &RecordId> {
        self.order.iter().map(|key| &self.entries[key].id)
    }

    /// Every record's id with its class string, in record-table order,
    /// straight from the table — a whole-database survey by class
    /// without inflating a single record.
    pub fn record_types(&self) -> impl Iterator<Item = (&RecordId, &str)> {
        self.order.iter().map(|key| {
            let entry = &self.entries[key];
            (&entry.id, entry.record_type.as_str())
        })
    }

    /// Number of records in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The record's stored build timestamp, preserved when composing
    /// mod databases.
    #[must_use]
    pub fn record_timestamp(&self, id: &RecordId) -> Option<i64> {
        Some(self.entries.get(&normalize(id.as_str()))?.timestamp)
    }

    fn decompress(&self, entry: &RecordEntry) -> Result<DbRecord, ArzError> {
        let compressed = entry
            .payload_offset
            .checked_add(entry.payload_size)
            .and_then(|end| self.data.get(entry.payload_offset..end))
            .ok_or_else(|| ArzError::PayloadOutOfRange {
                id: entry.id.as_str().to_string(),
            })?;
        let codec = self.dialect.codec;
        let payload = match entry.decompressed_size {
            Some(len) => codec.decompress(compressed, len),
            None => codec.decompress_unsized(compressed),
        }
        .map_err(|source| ArzError::Decompress {
            id: entry.id.as_str().to_string(),
            source,
        })?;
        let variables = parse_variables(&payload, &entry.id, &self.strings)?;
        Ok(DbRecord {
            id: entry.id.clone(),
            record_type: entry.record_type.clone(),
            variables,
        })
    }
}

fn offset_field(value: i32, field: &'static str) -> Result<usize, ArzError> {
    usize::try_from(value).map_err(|_| ArzError::InvalidHeader { field, value })
}

fn read_string_table(data: &[u8], start: usize) -> Result<Vec<String>, ArzError> {
    let mut reader = ByteReader::at(data, start);
    let count = reader.read_i32()?;
    let count = usize::try_from(count).map_err(|_| ArzError::InvalidCount {
        what: "string",
        count,
    })?;
    (0..count).map(|_| Ok(reader.read_cstring()?)).collect()
}

fn read_record_table(
    data: &[u8],
    start: usize,
    count: usize,
    strings: &[String],
    dialect: ArzDialect,
) -> Result<(HashMap<String, RecordEntry>, Vec<String>), ArzError> {
    let mut reader = ByteReader::at(data, start);
    let mut entries = HashMap::with_capacity(count);
    let mut order = Vec::with_capacity(count);
    for index in 0..count {
        let id_index = reader.read_i32()?;
        let record_type = reader.read_cstring()?;
        let payload_offset = offset_field(reader.read_i32()?, "record payload offset")?;
        let payload_size = offset_field(reader.read_i32()?, "record payload size")?;
        let decompressed_size = if dialect.entry_has_decompressed_size {
            Some(offset_field(
                reader.read_i32()?,
                "record decompressed size",
            )?)
        } else {
            None
        };
        let timestamp = reader.read_i64()?;

        let raw_id = usize::try_from(id_index)
            .ok()
            .and_then(|id_index| strings.get(id_index))
            .ok_or(ArzError::StringIndex {
                index: id_index,
                len: strings.len(),
            })?;
        let id = RecordId::parse(raw_id.clone()).ok_or(ArzError::EmptyRecordPath { index })?;
        let key = normalize(id.as_str());
        order.push(key.clone());
        entries.insert(
            key,
            RecordEntry {
                id,
                record_type,
                payload_offset: HEADER_SIZE + payload_offset,
                payload_size,
                decompressed_size,
                timestamp,
            },
        );
    }
    Ok((entries, order))
}

fn parse_variables(
    payload: &[u8],
    record_id: &RecordId,
    strings: &[String],
) -> Result<Vec<DbVariable>, ArzError> {
    let id = || record_id.as_str().to_string();
    if !payload.len().is_multiple_of(4) {
        return Err(ArzError::UnalignedPayload {
            id: id(),
            len: payload.len(),
        });
    }
    let mut reader = ByteReader::new(payload);
    let mut variables = Vec::new();
    while reader.pos() < payload.len() {
        let data_type = reader.read_i16()?;
        let count = reader.read_i16()?;
        let name_index = reader.read_i32()?;
        let name = usize::try_from(name_index)
            .ok()
            .and_then(|name_index| strings.get(name_index))
            .ok_or(ArzError::StringIndex {
                index: name_index,
                len: strings.len(),
            })?
            .clone();
        let value_count = usize::try_from(count)
            .ok()
            .filter(|&value_count| value_count >= 1)
            .ok_or_else(|| ArzError::InvalidValueCount {
                id: id(),
                name: name.clone(),
                count,
            })?;

        let values = match data_type {
            0 => DbValues::Integers(read_values(&mut reader, value_count, ByteReader::read_i32)?),
            1 => DbValues::Floats(read_values(&mut reader, value_count, ByteReader::read_f32)?),
            2 => DbValues::Strings(
                read_values(&mut reader, value_count, ByteReader::read_i32)?
                    .into_iter()
                    .map(|index| {
                        usize::try_from(index)
                            .ok()
                            .and_then(|index| strings.get(index))
                            .map(|value| value.trim().to_string())
                            .ok_or(ArzError::StringIndex {
                                index,
                                len: strings.len(),
                            })
                    })
                    .collect::<Result<_, _>>()?,
            ),
            3 => DbValues::Booleans(
                read_values(&mut reader, value_count, ByteReader::read_i32)?
                    .into_iter()
                    .map(|value| value != 0)
                    .collect(),
            ),
            _ => {
                return Err(ArzError::InvalidDataType {
                    id: id(),
                    name,
                    data_type,
                });
            }
        };
        variables.push(DbVariable { name, values });
    }
    Ok(variables)
}

fn read_values<'data, T>(
    reader: &mut ByteReader<'data>,
    count: usize,
    mut read: impl FnMut(&mut ByteReader<'data>) -> Result<T, ReadError>,
) -> Result<Vec<T>, ArzError> {
    (0..count).map(|_| Ok(read(reader)?)).collect()
}

/// Serializes records into a new database image in `dialect`'s shape
/// — the write half of this format, used to build an app's **own**
/// mod archives. The game's databases stay read-only
/// (ARCHITECTURE.md); a composed image is always a new file. Layout
/// mirrors `ArtManager`'s output (via the MIT `TQArchive-Wrapper`
/// reference): 24-byte header, compressed record payloads, record
/// table, string table.
#[must_use]
pub fn compose(records: &[(DbRecord, i64)], dialect: ArzDialect) -> Vec<u8> {
    let mut interner = Interner::default();
    let mut payloads = Vec::new();
    let mut table = Vec::new();
    for (record, timestamp) in records {
        let name_index = interner.intern(record.id.as_str());
        let raw = encode_variables(record, &mut interner);
        let compressed = dialect.codec.compress(&raw);
        table.push(TableRow {
            name_index,
            record_type: record.record_type.clone(),
            offset: payloads.len(),
            compressed_size: compressed.len(),
            decompressed_size: raw.len(),
            timestamp: *timestamp,
        });
        payloads.extend_from_slice(&compressed);
    }

    let mut record_table = Vec::new();
    for row in &table {
        write_i32(&mut record_table, row.name_index);
        write_cstring(&mut record_table, &row.record_type);
        write_i32(&mut record_table, i32::try_from(row.offset).unwrap_or(0));
        write_i32(
            &mut record_table,
            i32::try_from(row.compressed_size).unwrap_or(0),
        );
        if dialect.entry_has_decompressed_size {
            write_i32(
                &mut record_table,
                i32::try_from(row.decompressed_size).unwrap_or(0),
            );
        }
        write_i64(&mut record_table, row.timestamp);
    }
    let mut string_table = Vec::new();
    write_i32(
        &mut string_table,
        i32::try_from(interner.strings.len()).unwrap_or(0),
    );
    for value in &interner.strings {
        write_cstring(&mut string_table, value);
    }

    let record_table_start = HEADER_SIZE + payloads.len();
    let string_table_start = record_table_start + record_table.len();
    let mut out = Vec::with_capacity(string_table_start + string_table.len());
    write_u32(&mut out, dialect.first_dword);
    write_i32(&mut out, i32::try_from(record_table_start).unwrap_or(0));
    write_i32(&mut out, i32::try_from(record_table.len()).unwrap_or(0));
    write_i32(&mut out, i32::try_from(table.len()).unwrap_or(0));
    write_i32(&mut out, i32::try_from(string_table_start).unwrap_or(0));
    write_i32(&mut out, i32::try_from(string_table.len()).unwrap_or(0));
    out.extend_from_slice(&payloads);
    out.extend_from_slice(&record_table);
    out.extend_from_slice(&string_table);
    out
}

struct TableRow {
    name_index: i32,
    record_type: String,
    offset: usize,
    compressed_size: usize,
    decompressed_size: usize,
    timestamp: i64,
}

fn encode_variables(record: &DbRecord, interner: &mut Interner) -> Vec<u8> {
    let mut raw = Vec::new();
    for variable in record.variables() {
        let (type_code, count) = match &variable.values {
            DbValues::Integers(values) => (0_i16, values.len()),
            DbValues::Floats(values) => (1_i16, values.len()),
            DbValues::Strings(values) => (2_i16, values.len()),
            DbValues::Booleans(values) => (3_i16, values.len()),
        };
        raw.extend_from_slice(&type_code.to_le_bytes());
        raw.extend_from_slice(&i16::try_from(count).unwrap_or(i16::MAX).to_le_bytes());
        write_i32(&mut raw, interner.intern(&variable.name));
        match &variable.values {
            DbValues::Integers(values) => {
                for value in values {
                    write_i32(&mut raw, *value);
                }
            }
            DbValues::Floats(values) => {
                for value in values {
                    write_f32(&mut raw, *value);
                }
            }
            DbValues::Strings(values) => {
                for value in values {
                    write_i32(&mut raw, interner.intern(value));
                }
            }
            DbValues::Booleans(values) => {
                for value in values {
                    write_i32(&mut raw, i32::from(*value));
                }
            }
        }
    }
    raw
}

#[derive(Default)]
struct Interner {
    strings: Vec<String>,
    indexes: HashMap<String, i32>,
}

impl Interner {
    fn intern(&mut self, value: &str) -> i32 {
        if let Some(index) = self.indexes.get(value) {
            return *index;
        }
        let index = i32::try_from(self.strings.len()).unwrap_or(0);
        self.strings.push(value.to_string());
        self.indexes.insert(value.to_string(), index);
        index
    }
}

/// Builds synthetic ARZ byte images in either dialect so tests (here
/// and in consumer crates, behind the `test-util` feature) can
/// assemble records with chosen types and variables.
#[cfg(any(test, feature = "test-util"))]
pub mod fixture {
    // Fixture builders panic on sizes that cannot occur in a test image.
    #![allow(clippy::missing_panics_doc)]

    use super::{ArzDialect, HEADER_SIZE};
    use crate::writer::{write_cstring, write_i32, write_u32};

    pub struct ArzBuilder {
        dialect: ArzDialect,
        strings: Vec<String>,
        records: Vec<(i32, String, Payload)>,
    }

    enum Payload {
        Plain(Vec<u8>),
        Encoded {
            block: Vec<u8>,
            decompressed_len: usize,
        },
    }

    pub enum Values<'a> {
        Ints(&'a [i32]),
        Floats(&'a [f32]),
        Strings(&'a [&'a str]),
        Bools(&'a [bool]),
    }

    impl ArzBuilder {
        #[must_use]
        pub fn new(dialect: ArzDialect) -> Self {
            Self {
                dialect,
                strings: Vec::new(),
                records: Vec::new(),
            }
        }

        pub fn intern(&mut self, value: &str) -> i32 {
            let index = self
                .strings
                .iter()
                .position(|existing| existing == value)
                .unwrap_or_else(|| {
                    self.strings.push(value.to_string());
                    self.strings.len() - 1
                });
            i32::try_from(index).unwrap()
        }

        pub fn record(&mut self, id: &str, record_type: &str, variables: &[(&str, Values<'_>)]) {
            let mut payload = Vec::new();
            for (name, values) in variables {
                let name_index = self.intern(name);
                let (type_code, count): (i16, usize) = match values {
                    Values::Ints(v) => (0, v.len()),
                    Values::Floats(v) => (1, v.len()),
                    Values::Strings(v) => (2, v.len()),
                    Values::Bools(v) => (3, v.len()),
                };
                payload.extend_from_slice(&type_code.to_le_bytes());
                payload.extend_from_slice(&i16::try_from(count).unwrap().to_le_bytes());
                payload.extend_from_slice(&name_index.to_le_bytes());
                match values {
                    Values::Ints(v) => {
                        for value in *v {
                            payload.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    Values::Floats(v) => {
                        for value in *v {
                            payload.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                    Values::Strings(v) => {
                        for value in *v {
                            let index = self.intern(value);
                            payload.extend_from_slice(&index.to_le_bytes());
                        }
                    }
                    Values::Bools(v) => {
                        for value in *v {
                            payload.extend_from_slice(&i32::from(*value).to_le_bytes());
                        }
                    }
                }
            }
            self.record_raw(id, record_type, payload);
        }

        /// Adds a record with an arbitrary (possibly corrupt) payload.
        pub fn record_raw(&mut self, id: &str, record_type: &str, payload: Vec<u8>) {
            let id_index = self.intern(id);
            self.records
                .push((id_index, record_type.to_string(), Payload::Plain(payload)));
        }

        /// Adds a record from an already-encoded `block`, stored
        /// verbatim with `decompressed_len` in the table — for streams
        /// a real packer emits that the codec's own encoder would not.
        pub fn record_block(
            &mut self,
            id: &str,
            record_type: &str,
            block: Vec<u8>,
            decompressed_len: usize,
        ) {
            let id_index = self.intern(id);
            self.records.push((
                id_index,
                record_type.to_string(),
                Payload::Encoded {
                    block,
                    decompressed_len,
                },
            ));
        }

        #[must_use]
        pub fn build(self) -> Vec<u8> {
            self.build_with_layout().0
        }

        /// Also returns the record-table start offset, for tests that
        /// corrupt table bytes.
        #[must_use]
        pub fn build_with_layout(self) -> (Vec<u8>, usize) {
            fn push_usize(buf: &mut Vec<u8>, value: usize) {
                write_i32(buf, i32::try_from(value).unwrap());
            }

            let mut payloads = Vec::new();
            let mut record_table = Vec::new();
            for (id_index, record_type, payload) in &self.records {
                let (compressed, decompressed_len) = match payload {
                    Payload::Plain(plain) => (self.dialect.codec.compress(plain), plain.len()),
                    Payload::Encoded {
                        block,
                        decompressed_len,
                    } => (block.clone(), *decompressed_len),
                };
                write_i32(&mut record_table, *id_index);
                write_cstring(&mut record_table, record_type);
                push_usize(&mut record_table, payloads.len());
                push_usize(&mut record_table, compressed.len());
                if self.dialect.entry_has_decompressed_size {
                    push_usize(&mut record_table, decompressed_len);
                }
                write_i32(&mut record_table, 0);
                write_i32(&mut record_table, 0);
                payloads.extend_from_slice(&compressed);
            }

            let mut string_table = Vec::new();
            push_usize(&mut string_table, self.strings.len());
            for string in &self.strings {
                write_cstring(&mut string_table, string);
            }

            let record_table_start = HEADER_SIZE + payloads.len();
            let string_table_start = record_table_start + record_table.len();
            let mut file = Vec::new();
            write_u32(&mut file, self.dialect.first_dword);
            push_usize(&mut file, record_table_start);
            push_usize(&mut file, record_table.len());
            push_usize(&mut file, self.records.len());
            push_usize(&mut file, string_table_start);
            push_usize(&mut file, string_table.len());
            file.extend_from_slice(&payloads);
            file.extend_from_slice(&record_table);
            file.extend_from_slice(&string_table);
            (file, record_table_start)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{ArzBuilder, Values};
    use super::*;

    const DIALECTS: [ArzDialect; 2] = [ArzDialect::titan_quest(), ArzDialect::grim_dawn()];

    const SWORD_ID: &str = "records\\item\\equipmentweapon\\sword_01.dbr";

    fn sword_arz(dialect: ArzDialect) -> Vec<u8> {
        let mut builder = ArzBuilder::new(dialect);
        builder.record(
            SWORD_ID,
            "WeaponMelee_Sword",
            &[
                ("itemLevel", Values::Ints(&[12])),
                ("attackSpeed", Values::Floats(&[1.5, 2.0])),
                ("description", Values::Strings(&["  tagSwordName01 "])),
                ("active", Values::Bools(&[true])),
            ],
        );
        builder.build()
    }

    fn record_id(raw: &str) -> RecordId {
        RecordId::parse(raw.to_string()).unwrap()
    }

    #[test]
    fn real_header_dwords_are_encoded_in_the_dialects() {
        assert_eq!(
            ArzDialect::titan_quest().first_dword().to_le_bytes(),
            [0x04, 0x00, 0x03, 0x00]
        );
        assert_eq!(
            ArzDialect::grim_dawn().first_dword().to_le_bytes(),
            [0x02, 0x00, 0x03, 0x00]
        );
    }

    #[test]
    fn lookup_ignores_case_and_slash_direction() {
        for dialect in DIALECTS {
            let arz = ArzFile::parse(sword_arz(dialect), dialect).unwrap();
            let record = arz
                .record(&record_id("RECORDS/ITEM/EQUIPMENTWEAPON/SWORD_01.DBR"))
                .unwrap()
                .unwrap();
            assert_eq!(record.id.as_str(), SWORD_ID);
            assert_eq!(record.record_type, "WeaponMelee_Sword");
            assert_eq!(arz.dialect(), dialect);
        }
    }

    #[test]
    fn decodes_all_four_value_types() {
        for dialect in DIALECTS {
            let arz = ArzFile::parse(sword_arz(dialect), dialect).unwrap();
            let record = arz.record(&record_id(SWORD_ID)).unwrap().unwrap();
            assert_eq!(record.integer("itemLevel"), Some(12));
            assert_eq!(
                record.variable("attackSpeed").unwrap().values,
                DbValues::Floats(vec![1.5, 2.0])
            );
            assert_eq!(record.string("description"), Some("tagSwordName01"));
            assert_eq!(
                record.variable("active").unwrap().values,
                DbValues::Booleans(vec![true])
            );
        }
    }

    #[test]
    fn wrong_dialect_is_rejected_at_the_header() {
        let grim = sword_arz(ArzDialect::grim_dawn());
        assert!(matches!(
            ArzFile::parse(grim, ArzDialect::titan_quest()),
            Err(ArzError::DialectMismatch {
                found: 0x0003_0002,
                expected: 0x0003_0004
            })
        ));
        let titan = sword_arz(ArzDialect::titan_quest());
        assert!(matches!(
            ArzFile::parse(titan, ArzDialect::grim_dawn()),
            Err(ArzError::DialectMismatch {
                found: 0x0003_0004,
                expected: 0x0003_0002
            })
        ));
    }

    #[test]
    fn grim_dawn_record_is_inflated_even_when_block_and_payload_sizes_match() {
        // Hand-built LZ4 block: 8 literals (the variable header), a
        // minimum-length match copying the first 4 bytes, 8 trailing
        // literals — 20 bytes encoding a 20-byte payload, the shape
        // that real databases hold for ~0.1% of their records.
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        let name_index = builder.intern("noise").to_le_bytes();
        let mut block = vec![0x80, 0x00, 0x00, 0x03, 0x00];
        block.extend_from_slice(&name_index);
        block.extend_from_slice(&[0x08, 0x00, 0x80]);
        block.extend_from_slice(&[0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55]);
        assert_eq!(block.len(), 20);
        builder.record_block(SWORD_ID, "Noise", block, 20);

        let arz = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        let record = arz.record(&record_id(SWORD_ID)).unwrap().unwrap();
        assert_eq!(
            record.variable("noise").unwrap().values,
            DbValues::Integers(vec![0x0003_0000, 0x1122_3344, 0x5566_7788])
        );
    }

    #[test]
    fn typed_accessors_refuse_other_types() {
        let dialect = ArzDialect::titan_quest();
        let arz = ArzFile::parse(sword_arz(dialect), dialect).unwrap();
        let record = arz.record(&record_id(SWORD_ID)).unwrap().unwrap();
        assert_eq!(record.string("itemLevel"), None);
        assert_eq!(record.integer("description"), None);
    }

    #[test]
    fn unknown_record_is_none() {
        let dialect = ArzDialect::grim_dawn();
        let arz = ArzFile::parse(sword_arz(dialect), dialect).unwrap();
        assert!(arz.record(&record_id("records\\nothing.dbr")).is_none());
    }

    #[test]
    fn compose_round_trips_records_order_and_timestamps() {
        for dialect in DIALECTS {
            let mut builder = ArzBuilder::new(dialect);
            builder.record(
                "records\\item\\zeta.dbr",
                "WeaponMelee_Sword",
                &[
                    ("itemNameTag", Values::Strings(&["tagZeta"])),
                    ("offensivePhysicalMin", Values::Floats(&[12.5, 14.0])),
                    ("levelRequirement", Values::Ints(&[4])),
                    ("cannotPickUpMultiple", Values::Bools(&[true])),
                ],
            );
            builder.record(
                "records\\item\\alpha.dbr",
                "LootRandomizer",
                &[("lootRandomizerName", Values::Strings(&["tagAlpha"]))],
            );
            let original = ArzFile::parse(builder.build(), dialect).unwrap();

            let records: Vec<(DbRecord, i64)> = original
                .record_ids()
                .map(|id| {
                    (
                        original.record(id).unwrap().unwrap(),
                        original.record_timestamp(id).unwrap(),
                    )
                })
                .collect();
            let image = compose(&records, dialect);
            assert_eq!(image[..4], dialect.first_dword().to_le_bytes());
            let composed = ArzFile::parse(image, dialect).unwrap();

            let original_ids: Vec<&str> = original.record_ids().map(RecordId::as_str).collect();
            let composed_ids: Vec<&str> = composed.record_ids().map(RecordId::as_str).collect();
            assert_eq!(original_ids, composed_ids);
            assert_eq!(composed.len(), 2);
            for id in original.record_ids() {
                assert_eq!(
                    original.record(id).unwrap().unwrap(),
                    composed.record(id).unwrap().unwrap(),
                    "{id:?} under {dialect:?}"
                );
                assert_eq!(original.record_timestamp(id), composed.record_timestamp(id));
            }
        }
    }

    #[test]
    fn composed_image_only_parses_under_its_own_dialect() {
        let records = vec![(
            DbRecord {
                id: record_id(SWORD_ID),
                record_type: "WeaponMelee_Sword".to_string(),
                variables: vec![DbVariable {
                    name: "itemLevel".to_string(),
                    values: DbValues::Integers(vec![3]),
                }],
            },
            0,
        )];
        let image = compose(&records, ArzDialect::grim_dawn());
        assert!(matches!(
            ArzFile::parse(image, ArzDialect::titan_quest()),
            Err(ArzError::DialectMismatch { .. })
        ));
    }

    #[test]
    fn set_variable_replaces_in_place_and_appends() {
        let dialect = ArzDialect::titan_quest();
        let mut builder = ArzBuilder::new(dialect);
        builder.record(
            "records\\skills\\thing.dbr",
            "Skill_Attack",
            &[
                ("skillTargetNumber", Values::Ints(&[4])),
                ("skillManaCost", Values::Floats(&[10.0])),
            ],
        );
        let arz = ArzFile::parse(builder.build(), dialect).unwrap();
        let mut record = arz
            .record(&RecordId::parse("records\\skills\\thing.dbr".into()).unwrap())
            .unwrap()
            .unwrap();
        record.set_variable(DbVariable {
            name: "skillTargetNumber".to_string(),
            values: DbValues::Integers(vec![12]),
        });
        record.set_variable(DbVariable {
            name: "skillTargetRadius".to_string(),
            values: DbValues::Floats(vec![18.0]),
        });
        assert_eq!(record.integer("skillTargetNumber"), Some(12));
        assert_eq!(record.float("skillTargetRadius"), Some(18.0));
        let names: Vec<&str> = record.variables().map(|v| v.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["skillTargetNumber", "skillManaCost", "skillTargetRadius"]
        );
    }

    #[test]
    fn record_ids_yields_raw_ids() {
        let dialect = ArzDialect::grim_dawn();
        let arz = ArzFile::parse(sword_arz(dialect), dialect).unwrap();
        let ids: Vec<&str> = arz.record_ids().map(RecordId::as_str).collect();
        assert_eq!(ids, vec![SWORD_ID]);
        assert_eq!(arz.len(), 1);
        assert!(!arz.is_empty());
    }

    #[test]
    fn bad_data_type_is_reported_with_variable_name() {
        for dialect in DIALECTS {
            let mut builder = ArzBuilder::new(dialect);
            let name_index = builder.intern("brokenVar");
            let mut payload = Vec::new();
            payload.extend_from_slice(&7_i16.to_le_bytes());
            payload.extend_from_slice(&1_i16.to_le_bytes());
            payload.extend_from_slice(&name_index.to_le_bytes());
            payload.extend_from_slice(&0_i32.to_le_bytes());
            builder.record_raw(SWORD_ID, "WeaponMelee_Sword", payload);
            let arz = ArzFile::parse(builder.build(), dialect).unwrap();
            assert!(matches!(
                arz.record(&record_id(SWORD_ID)).unwrap(),
                Err(ArzError::InvalidDataType { data_type: 7, .. })
            ));
        }
    }

    #[test]
    fn unaligned_payload_is_rejected() {
        for dialect in DIALECTS {
            let mut builder = ArzBuilder::new(dialect);
            builder.record_raw(SWORD_ID, "WeaponMelee_Sword", vec![0, 1, 2]);
            let arz = ArzFile::parse(builder.build(), dialect).unwrap();
            assert!(matches!(
                arz.record(&record_id(SWORD_ID)).unwrap(),
                Err(ArzError::UnalignedPayload { len: 3, .. })
            ));
        }
    }

    #[test]
    fn corrupt_payload_is_a_decompress_error() {
        for dialect in DIALECTS {
            let mut builder = ArzBuilder::new(dialect);
            builder.record(
                SWORD_ID,
                "WeaponMelee_Sword",
                &[("itemLevel", Values::Ints(&[1, 1, 1, 1, 1, 1, 1, 1]))],
            );
            let mut file = builder.build();
            for byte in &mut file[HEADER_SIZE..HEADER_SIZE + 4] {
                *byte = 0xFF;
            }
            let arz = ArzFile::parse(file, dialect).unwrap();
            assert!(
                matches!(
                    arz.record(&record_id(SWORD_ID)).unwrap(),
                    Err(ArzError::Decompress { .. })
                ),
                "{dialect:?}"
            );
        }
    }

    #[test]
    fn string_index_out_of_range_fails_at_parse() {
        for dialect in DIALECTS {
            let mut builder = ArzBuilder::new(dialect);
            builder.record(
                SWORD_ID,
                "WeaponMelee_Sword",
                &[("itemLevel", Values::Ints(&[1]))],
            );
            let (mut file, record_table_start) = builder.build_with_layout();
            file[record_table_start..record_table_start + 4].copy_from_slice(&99_i32.to_le_bytes());
            assert!(matches!(
                ArzFile::parse(file, dialect),
                Err(ArzError::StringIndex { index: 99, .. })
            ));
        }
    }
}
