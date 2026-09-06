// Vendored from tq-univault crates/univault-core/src/arc.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Reader for the game's `.arc` resource archives — localization text
//! (`Text_EN.arc`), textures, and other assets. Read-only reference
//! data per ARCHITECTURE.md. Ported from `TQVaultAE`'s
//! `ArcFileProvider.cs` (MIT).
//!
//! Layout: `"ARC"` magic; entry count at 0x08, part count at 0x0C,
//! table offset at 0x18. At the table offset: 12-byte part entries
//! (offset, compressed size, real size), then the null-terminated
//! ASCII names of active entries, in entry order. The last
//! `44 × entries` bytes of the file are the directory records
//! (storage type, offset, sizes, part range, name info). Storage type
//! 1 is stored raw; everything else concatenates parts inflated with
//! the archive's [`Codec`] (zlib for Titan Quest, LZ4 block for Grim
//! Dawn — the layout is otherwise identical). A 0x03 byte where a
//! name should start marks an inactive ("null file") entry.
//!
//! Two ways in: [`ArcFile`] holds the whole archive in memory and
//! extracts on demand; [`ArcIndex`] is the directory alone, parsed
//! from the header and the table region, and hands back the byte
//! ranges an entry occupies so a shell can read a few entries out of
//! a 240 MB archive without reading the rest.

use std::collections::HashMap;
use std::ops::Range;

use crate::codec::{Codec, CodecError};
use crate::ids::normalize;
use crate::reader::{ByteReader, ReadError};

/// A parsed `.arc` archive: the directory plus the raw bytes, from
/// which contained files extract on demand.
pub struct ArcFile {
    data: Vec<u8>,
    index: ArcIndex,
}

/// The directory of a `.arc` archive without its contents: every
/// active entry's name and where its bytes lie in the file.
pub struct ArcIndex {
    codec: Codec,
    entries: HashMap<String, DirEntry>,
}

/// The counts and table offset from the fixed header — enough to know
/// which byte range the directory tables occupy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArcHeader {
    entry_count: usize,
    part_count: usize,
    toc_offset: usize,
}

#[derive(Clone, Copy)]
struct Part {
    offset: usize,
    compressed_size: usize,
    real_size: usize,
}

#[derive(Clone)]
enum Storage {
    Stored { offset: usize },
    Parts(Vec<Part>),
}

#[derive(Clone)]
struct DirEntry {
    name: String,
    real_size: usize,
    storage: Storage,
}

/// One entry's place in the archive file: the absolute byte ranges to
/// read, which [`ArcIndex::assemble`] turns back into the file.
#[derive(Clone)]
pub struct Located {
    entry: DirEntry,
}

impl Located {
    /// The entry's name as the archive spells it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.entry.name
    }

    /// The byte ranges holding the entry, in assembly order: one for a
    /// stored entry, one per part otherwise.
    #[must_use]
    pub fn ranges(&self) -> Vec<Range<usize>> {
        match &self.entry.storage {
            Storage::Stored { offset } => {
                std::iter::once(*offset..offset.saturating_add(self.entry.real_size)).collect()
            }
            Storage::Parts(parts) => parts
                .iter()
                .map(|part| part.offset..part.offset.saturating_add(part.compressed_size))
                .collect(),
        }
    }
}

/// Errors from parsing an ARC archive or extracting a file from it.
#[derive(Debug, thiserror::Error)]
pub enum ArcError {
    #[error("not an ARC archive (bad magic or too short)")]
    NotArc,
    #[error("invalid {what} count {count}")]
    InvalidCount { what: &'static str, count: i32 },
    #[error("invalid table offset {offset}")]
    InvalidOffset { offset: i32 },
    #[error("directory record table larger than the file")]
    TruncatedDirectory,
    #[error("entry {name}: data range outside the file")]
    DataOutOfRange { name: String },
    #[error("entry {name}: given {actual} byte ranges, the directory lists {expected}")]
    RangeCount {
        name: String,
        expected: usize,
        actual: usize,
    },
    #[error("entry {name}: part {part}: decompression failed: {source}")]
    Decompress {
        name: String,
        part: usize,
        source: CodecError,
    },
    #[error("entry {name}: extracted {actual} bytes, directory says {expected}")]
    SizeMismatch {
        name: String,
        actual: usize,
        expected: usize,
    },
    #[error(transparent)]
    Read(#[from] ReadError),
}

impl ArcHeader {
    /// Bytes the header occupies at the start of the file.
    pub const LEN: usize = 0x1C;

    /// Parses the header from the first [`Self::LEN`] bytes of the
    /// file (a longer slice is fine).
    ///
    /// # Errors
    /// [`ArcError::NotArc`] for a short or unmarked slice, an invalid
    /// count or offset otherwise.
    pub fn parse(bytes: &[u8]) -> Result<Self, ArcError> {
        if bytes.len() < Self::LEN || &bytes[..3] != b"ARC" {
            return Err(ArcError::NotArc);
        }
        let mut counts = ByteReader::at(bytes, 0x08);
        let entry_count = count_field(counts.read_i32()?, "entry")?;
        let part_count = count_field(counts.read_i32()?, "part")?;
        let offset = ByteReader::at(bytes, 0x18).read_i32()?;
        let toc_offset = usize::try_from(offset).map_err(|_| ArcError::InvalidOffset { offset })?;
        Ok(Self {
            entry_count,
            part_count,
            toc_offset,
        })
    }

    /// The byte range of the directory tables — the part table, the
    /// names, and the directory records — which run from the table
    /// offset to the end of a `file_len`-byte file.
    ///
    /// # Errors
    /// [`ArcError::InvalidOffset`] when the table offset lies past the
    /// end of the file.
    pub fn tables_range(&self, file_len: usize) -> Result<Range<usize>, ArcError> {
        if self.toc_offset > file_len {
            return Err(ArcError::InvalidOffset {
                offset: i32::try_from(self.toc_offset).unwrap_or(i32::MAX),
            });
        }
        Ok(self.toc_offset..file_len)
    }
}

impl ArcIndex {
    /// Parses the directory from the bytes of
    /// [`ArcHeader::tables_range`]. Contents are located by
    /// [`ArcIndex::locate`] and rebuilt by [`ArcIndex::assemble`] with
    /// `codec`.
    ///
    /// # Errors
    /// Any structural error in the directory tables.
    pub fn parse(header: ArcHeader, tables: &[u8], codec: Codec) -> Result<Self, ArcError> {
        let mut reader = ByteReader::at(tables, 0);
        let parts = (0..header.part_count)
            .map(|_| {
                let offset = count_field(reader.read_i32()?, "part offset")?;
                let compressed_size = count_field(reader.read_i32()?, "part size")?;
                let real_size = count_field(reader.read_i32()?, "part real size")?;
                Ok(Part {
                    offset,
                    compressed_size,
                    real_size,
                })
            })
            .collect::<Result<Vec<_>, ArcError>>()?;
        let names_offset = reader.pos();

        let directory_bytes = header
            .entry_count
            .checked_mul(44)
            .ok_or(ArcError::TruncatedDirectory)?;
        let directory_start = tables
            .len()
            .checked_sub(directory_bytes)
            .ok_or(ArcError::TruncatedDirectory)?;

        let raw_records = read_directory(tables, directory_start, header.entry_count)?;
        let mut names = ByteReader::at(tables, names_offset);
        let mut entries = HashMap::new();
        for raw in raw_records {
            let Some(storage) = raw.storage(&parts) else {
                continue;
            };
            let Some(name) = read_entry_name(&mut names, directory_start)? else {
                continue;
            };
            entries.insert(
                normalize(&name),
                DirEntry {
                    name,
                    real_size: raw.real_size,
                    storage,
                },
            );
        }
        Ok(Self { codec, entries })
    }

    /// The codec compressed parts inflate with.
    #[must_use]
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Where an entry lies, by its internal path (matched
    /// case-insensitively with `/` and `\` interchangeable). `None`
    /// when the archive has no such entry.
    #[must_use]
    pub fn locate(&self, name: &str) -> Option<Located> {
        self.entries.get(&normalize(name)).map(|entry| Located {
            entry: entry.clone(),
        })
    }

    /// Every active entry's name, in no particular order.
    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        self.entries.values().map(|entry| entry.name.as_str())
    }

    /// Rebuilds an entry from the bytes of each of its
    /// [`Located::ranges`], in order: a stored entry is copied, parts
    /// are inflated and concatenated, and the result is checked
    /// against the directory's size.
    ///
    /// # Errors
    /// [`ArcError::RangeCount`] when the slices do not pair up with the
    /// ranges, [`ArcError::DataOutOfRange`] when a slice is not the
    /// length of its range, then the decompression and size errors of
    /// extraction.
    pub fn assemble(&self, located: &Located, ranges: &[&[u8]]) -> Result<Vec<u8>, ArcError> {
        let entry = &located.entry;
        let out_of_range = || ArcError::DataOutOfRange {
            name: entry.name.clone(),
        };
        let expected = match &entry.storage {
            Storage::Stored { .. } => 1,
            Storage::Parts(parts) => parts.len(),
        };
        if ranges.len() != expected {
            return Err(ArcError::RangeCount {
                name: entry.name.clone(),
                expected,
                actual: ranges.len(),
            });
        }
        match &entry.storage {
            Storage::Stored { .. } => {
                if ranges[0].len() == entry.real_size {
                    Ok(ranges[0].to_vec())
                } else {
                    Err(out_of_range())
                }
            }
            Storage::Parts(parts) => {
                let mut out = Vec::with_capacity(entry.real_size);
                for (index, (part, compressed)) in parts.iter().zip(ranges).enumerate() {
                    if compressed.len() != part.compressed_size {
                        return Err(out_of_range());
                    }
                    let inflated = self.inflate_part(part, compressed).map_err(|source| {
                        ArcError::Decompress {
                            name: entry.name.clone(),
                            part: index,
                            source,
                        }
                    })?;
                    out.extend_from_slice(&inflated);
                }
                if out.len() == entry.real_size {
                    Ok(out)
                } else {
                    Err(ArcError::SizeMismatch {
                        name: entry.name.clone(),
                        actual: out.len(),
                        expected: entry.real_size,
                    })
                }
            }
        }
    }

    /// Grim Dawn's archiver stores an LZ4 part raw when compression
    /// would not shrink it (a real `Items.arc` holds 94 such parts,
    /// none of them a valid LZ4 block), and IAGD reads them the same
    /// way. Titan Quest's zlib parts are always inflated, as
    /// `TQVaultAE` does.
    fn inflate_part(&self, part: &Part, compressed: &[u8]) -> Result<Vec<u8>, CodecError> {
        match self.codec {
            Codec::Lz4Block if compressed.len() == part.real_size => Ok(compressed.to_vec()),
            Codec::Zlib | Codec::Lz4Block => self.codec.decompress(compressed, part.real_size),
        }
    }
}

impl ArcFile {
    /// Parses the archive directory. File contents stay in place until
    /// [`ArcFile::file`] extracts them with `codec`.
    ///
    /// # Errors
    /// Any structural error in the header or directory tables.
    pub fn parse(data: Vec<u8>, codec: Codec) -> Result<Self, ArcError> {
        let header = ArcHeader::parse(&data)?;
        let tables = header.tables_range(data.len())?;
        let index = ArcIndex::parse(header, &data[tables], codec)?;
        Ok(Self { data, index })
    }

    /// The codec compressed parts inflate with.
    #[must_use]
    pub fn codec(&self) -> Codec {
        self.index.codec()
    }

    /// Extracts one contained file by its internal path (matched
    /// case-insensitively with `/` and `\` interchangeable). `None`
    /// when the archive has no such entry.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<Result<Vec<u8>, ArcError>> {
        let located = self.index.locate(name)?;
        let slices: Option<Vec<&[u8]>> = located
            .ranges()
            .into_iter()
            .map(|range| self.data.get(range))
            .collect();
        Some(match slices {
            Some(slices) => self.index.assemble(&located, &slices),
            None => Err(ArcError::DataOutOfRange {
                name: located.name().to_string(),
            }),
        })
    }

    /// Every active entry's name, in no particular order.
    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        self.index.file_names()
    }
}

struct RawRecord {
    storage_type: i32,
    offset: usize,
    real_size: usize,
    part_count: i32,
    first_part: i32,
}

impl RawRecord {
    /// `None` marks an inactive entry (no parts and not stored raw, or
    /// an out-of-range part window) — skipped like `TQVaultAE` does.
    fn storage(&self, parts: &[Part]) -> Option<Storage> {
        if self.storage_type == 1 {
            return Some(Storage::Stored {
                offset: self.offset,
            });
        }
        let first = usize::try_from(self.first_part).ok()?;
        let count = usize::try_from(self.part_count).ok().filter(|&c| c >= 1)?;
        let window = parts.get(first..first.checked_add(count)?)?;
        Some(Storage::Parts(window.to_vec()))
    }
}

fn read_directory(data: &[u8], start: usize, count: usize) -> Result<Vec<RawRecord>, ArcError> {
    let mut reader = ByteReader::at(data, start);
    (0..count)
        .map(|_| {
            let storage_type = reader.read_i32()?;
            let offset = count_field(reader.read_i32()?, "entry offset")?;
            reader.read_i32()?;
            let real_size = count_field(reader.read_i32()?, "entry size")?;
            reader.read_i32()?;
            reader.read_i32()?;
            reader.read_i32()?;
            let part_count = reader.read_i32()?;
            let first_part = reader.read_i32()?;
            reader.read_i32()?;
            reader.read_i32()?;
            Ok(RawRecord {
                storage_type,
                offset,
                real_size,
                part_count,
                first_part,
            })
        })
        .collect()
}

/// Reads the next null-terminated ASCII name, stopping at the
/// directory records. A 0x03 byte in first position is `TQVaultAE`'s
/// "null file" marker: the entry is skipped and the byte is left for
/// the record data that follows.
fn read_entry_name(
    names: &mut ByteReader<'_>,
    directory_start: usize,
) -> Result<Option<String>, ArcError> {
    let mut bytes = Vec::new();
    loop {
        if names.pos() >= directory_start {
            return Ok(none_if_empty(&bytes));
        }
        let byte = names.read_u8()?;
        match byte {
            0x00 => return Ok(none_if_empty(&bytes)),
            0x03 if bytes.is_empty() => return Ok(None),
            _ => bytes.push(byte),
        }
    }
}

fn none_if_empty(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn count_field(value: i32, what: &'static str) -> Result<usize, ArcError> {
    usize::try_from(value).map_err(|_| ArcError::InvalidCount { what, count: value })
}

/// Builds synthetic ARC images — stored entries and multi-part
/// compressed entries under either codec — for tests here and in
/// consumer crates (behind the `test-util` feature).
#[cfg(any(test, feature = "test-util"))]
pub mod fixture {
    // Fixture builders panic on sizes that cannot occur in a test image.
    #![allow(clippy::missing_panics_doc)]

    use crate::codec::Codec;
    use crate::writer::write_i32;

    /// Size of the real 28-byte header: magic, version, entry count,
    /// part count, part-table size, string-table size, table offset.
    const HEADER_SIZE: usize = 28;

    enum Contents {
        Stored(Vec<u8>),
        Parts(Vec<Vec<u8>>),
    }

    struct Entry {
        name: String,
        contents: Contents,
    }

    pub struct ArcBuilder {
        codec: Codec,
        entries: Vec<Entry>,
    }

    impl ArcBuilder {
        #[must_use]
        pub fn new(codec: Codec) -> Self {
            Self {
                codec,
                entries: Vec::new(),
            }
        }

        /// Adds an entry stored raw (storage type 1).
        pub fn stored(&mut self, name: &str, bytes: &[u8]) {
            self.entries.push(Entry {
                name: name.to_string(),
                contents: Contents::Stored(bytes.to_vec()),
            });
        }

        /// Adds a compressed entry (storage type 3) whose plaintext is
        /// the concatenation of `parts`, one archive part per slice.
        pub fn compressed(&mut self, name: &str, parts: &[&[u8]]) {
            self.entries.push(Entry {
                name: name.to_string(),
                contents: Contents::Parts(parts.iter().map(|part| part.to_vec()).collect()),
            });
        }

        #[must_use]
        pub fn build(self) -> Vec<u8> {
            let mut blobs = Vec::new();
            let mut part_table = Vec::new();
            let mut part_count = 0_i32;
            let mut names = Vec::new();
            let mut records = Vec::new();
            for entry in &self.entries {
                names.extend_from_slice(entry.name.as_bytes());
                names.push(0);
                let offset = HEADER_SIZE + blobs.len();
                match &entry.contents {
                    Contents::Stored(bytes) => {
                        blobs.extend_from_slice(bytes);
                        push_record(&mut records, 1, offset, bytes.len(), bytes.len(), 0, 0);
                    }
                    Contents::Parts(parts) => {
                        let first_part = part_count;
                        let mut compressed_total = 0;
                        let mut real_total = 0;
                        for plain in parts {
                            let packed = pack_part(self.codec, plain);
                            push_usize(&mut part_table, HEADER_SIZE + blobs.len());
                            push_usize(&mut part_table, packed.len());
                            push_usize(&mut part_table, plain.len());
                            compressed_total += packed.len();
                            real_total += plain.len();
                            blobs.extend_from_slice(&packed);
                            part_count += 1;
                        }
                        push_record(
                            &mut records,
                            3,
                            offset,
                            compressed_total,
                            real_total,
                            part_count - first_part,
                            first_part,
                        );
                    }
                }
            }

            let toc_offset = HEADER_SIZE + blobs.len();
            let mut file = Vec::new();
            file.extend_from_slice(b"ARC\0");
            write_i32(&mut file, 3);
            push_usize(&mut file, self.entries.len());
            write_i32(&mut file, part_count);
            push_usize(&mut file, part_table.len());
            push_usize(&mut file, names.len());
            push_usize(&mut file, toc_offset);
            assert_eq!(file.len(), HEADER_SIZE);
            file.extend_from_slice(&blobs);
            file.extend_from_slice(&part_table);
            file.extend_from_slice(&names);
            file.extend_from_slice(&records);
            file
        }
    }

    /// Builds an ARC image whose entries are all stored raw — enough
    /// for texture and text lookups that never touch a codec.
    #[must_use]
    pub fn build_arc(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = ArcBuilder::new(Codec::Zlib);
        for (name, bytes) in entries {
            builder.stored(name, bytes);
        }
        builder.build()
    }

    /// Mirrors the game archivers: a zlib part is always compressed,
    /// an LZ4 part that would not shrink is stored raw.
    fn pack_part(codec: Codec, plain: &[u8]) -> Vec<u8> {
        let block = codec.compress(plain);
        match codec {
            Codec::Lz4Block if block.len() >= plain.len() => plain.to_vec(),
            Codec::Zlib | Codec::Lz4Block => block,
        }
    }

    fn push_usize(buf: &mut Vec<u8>, value: usize) {
        write_i32(buf, i32::try_from(value).unwrap());
    }

    fn push_record(
        buf: &mut Vec<u8>,
        storage_type: i32,
        offset: usize,
        compressed_size: usize,
        real_size: usize,
        part_count: i32,
        first_part: i32,
    ) {
        write_i32(buf, storage_type);
        push_usize(buf, offset);
        push_usize(buf, compressed_size);
        push_usize(buf, real_size);
        buf.extend_from_slice(&[0; 12]);
        write_i32(buf, part_count);
        write_i32(buf, first_part);
        write_i32(buf, 0);
        write_i32(buf, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{ArcBuilder, build_arc};
    use super::*;

    const CODECS: [Codec; 2] = [Codec::Zlib, Codec::Lz4Block];

    const STORED: &[u8] = b"stored contents";
    const SPLIT_A: &[u8] = b"first half / first half / first half / ";
    const SPLIT_B: &[u8] = b"second half second half second half";

    /// Two files: "text\\stored.txt" stored raw, "text\\split.txt"
    /// compressed in two parts.
    fn sample_arc(codec: Codec) -> Vec<u8> {
        let mut builder = ArcBuilder::new(codec);
        builder.stored("text\\stored.txt", STORED);
        builder.compressed("text\\split.txt", &[SPLIT_A, SPLIT_B]);
        builder.build()
    }

    fn split_full() -> Vec<u8> {
        [SPLIT_A, SPLIT_B].concat()
    }

    /// The ranged path a shell takes: header, tables, then only the
    /// entry's own bytes.
    fn index_of(file: &[u8], codec: Codec) -> ArcIndex {
        let header = ArcHeader::parse(&file[..ArcHeader::LEN]).unwrap();
        let tables = header.tables_range(file.len()).unwrap();
        ArcIndex::parse(header, &file[tables], codec).unwrap()
    }

    fn read_through_index(file: &[u8], codec: Codec, name: &str) -> Result<Vec<u8>, ArcError> {
        let index = index_of(file, codec);
        let located = index.locate(name).unwrap();
        let slices: Vec<&[u8]> = located
            .ranges()
            .into_iter()
            .map(|range| &file[range])
            .collect();
        index.assemble(&located, &slices)
    }

    #[test]
    fn extracts_stored_entries() {
        for codec in CODECS {
            let arc = ArcFile::parse(sample_arc(codec), codec).unwrap();
            assert_eq!(arc.file("text\\stored.txt").unwrap().unwrap(), STORED);
        }
    }

    #[test]
    fn extracts_and_concatenates_compressed_parts() {
        for codec in CODECS {
            let arc = ArcFile::parse(sample_arc(codec), codec).unwrap();
            assert_eq!(
                arc.file("text\\split.txt").unwrap().unwrap(),
                split_full(),
                "{codec:?}"
            );
        }
    }

    #[test]
    fn the_index_locates_and_assembles_exactly_what_the_file_extracts() {
        for codec in CODECS {
            let file = sample_arc(codec);
            assert_eq!(
                read_through_index(&file, codec, "text/stored.txt").unwrap(),
                STORED
            );
            assert_eq!(
                read_through_index(&file, codec, "TEXT\\SPLIT.TXT").unwrap(),
                split_full(),
                "{codec:?}"
            );
            let index = index_of(&file, codec);
            assert_eq!(index.locate("text\\split.txt").unwrap().ranges().len(), 2);
            assert!(index.locate("text\\missing.txt").is_none());
            let mut names: Vec<&str> = index.file_names().collect();
            names.sort_unstable();
            assert_eq!(names, vec!["text\\split.txt", "text\\stored.txt"]);
        }
    }

    #[test]
    fn assembling_from_the_wrong_slices_is_refused() {
        let file = sample_arc(Codec::Lz4Block);
        let index = index_of(&file, Codec::Lz4Block);
        let split = index.locate("text\\split.txt").unwrap();
        assert!(matches!(
            index.assemble(&split, &[SPLIT_A]),
            Err(ArcError::RangeCount {
                expected: 2,
                actual: 1,
                ..
            })
        ));
        let ranges = split.ranges();
        assert!(matches!(
            index.assemble(&split, &[&file[ranges[0].clone()], b"short"]),
            Err(ArcError::DataOutOfRange { .. })
        ));
        let stored = index.locate("text\\stored.txt").unwrap();
        assert!(matches!(
            index.assemble(&stored, &[b"wrong length"]),
            Err(ArcError::DataOutOfRange { .. })
        ));
    }

    #[test]
    fn the_header_needs_its_full_length_and_a_table_inside_the_file() {
        let file = sample_arc(Codec::Zlib);
        assert!(matches!(
            ArcHeader::parse(&file[..ArcHeader::LEN - 1]),
            Err(ArcError::NotArc)
        ));
        let header = ArcHeader::parse(&file).unwrap();
        assert!(matches!(
            header.tables_range(ArcHeader::LEN),
            Err(ArcError::InvalidOffset { .. })
        ));
    }

    #[test]
    fn lz4_parts_stored_raw_copy_through() {
        let raw: Vec<u8> = (0..=255_u8).collect();
        assert!(Codec::Lz4Block.compress(&raw).len() > raw.len());
        let mut builder = ArcBuilder::new(Codec::Lz4Block);
        builder.compressed("bin\\noise.bin", &[&raw, SPLIT_A]);
        let arc = ArcFile::parse(builder.build(), Codec::Lz4Block).unwrap();
        assert_eq!(
            arc.file("bin\\noise.bin").unwrap().unwrap(),
            [raw.as_slice(), SPLIT_A].concat()
        );
    }

    #[test]
    fn wrong_codec_fails_to_inflate() {
        let arc = ArcFile::parse(sample_arc(Codec::Zlib), Codec::Lz4Block).unwrap();
        assert!(matches!(
            arc.file("text\\split.txt").unwrap(),
            Err(ArcError::Decompress { part: 0, .. })
        ));
    }

    #[test]
    fn lookup_ignores_case_and_slash_direction() {
        let arc = ArcFile::parse(sample_arc(Codec::Zlib), Codec::Zlib).unwrap();
        assert_eq!(arc.file("TEXT/STORED.TXT").unwrap().unwrap(), STORED);
    }

    #[test]
    fn unknown_entry_is_none() {
        let arc = ArcFile::parse(sample_arc(Codec::Zlib), Codec::Zlib).unwrap();
        assert!(arc.file("text\\missing.txt").is_none());
    }

    #[test]
    fn lists_entry_names() {
        let arc = ArcFile::parse(sample_arc(Codec::Lz4Block), Codec::Lz4Block).unwrap();
        let mut names: Vec<&str> = arc.file_names().collect();
        names.sort_unstable();
        assert_eq!(names, vec!["text\\split.txt", "text\\stored.txt"]);
    }

    #[test]
    fn build_arc_stores_everything_raw() {
        let file = build_arc(&[("a.tex", b"aaaa"), ("dir\\b.txt", b"bb")]);
        let arc = ArcFile::parse(file, Codec::Lz4Block).unwrap();
        assert_eq!(arc.file("A.TEX").unwrap().unwrap(), b"aaaa");
        assert_eq!(arc.file("dir/b.txt").unwrap().unwrap(), b"bb");
    }

    #[test]
    fn rejects_non_arc_data() {
        assert!(matches!(
            ArcFile::parse(b"definitely not an archive".to_vec(), Codec::Zlib),
            Err(ArcError::NotArc)
        ));
    }

    #[test]
    fn size_mismatch_is_reported() {
        for codec in CODECS {
            let mut file = sample_arc(codec);
            // Shrink split.txt's recorded real size (second record, 4th i32).
            let records_start = file.len() - 88;
            let real_size_at = records_start + 44 + 12;
            file[real_size_at..real_size_at + 4].copy_from_slice(&5_i32.to_le_bytes());
            let arc = ArcFile::parse(file, codec).unwrap();
            assert!(matches!(
                arc.file("text\\split.txt").unwrap(),
                Err(ArcError::SizeMismatch { expected: 5, .. })
            ));
        }
    }
}
