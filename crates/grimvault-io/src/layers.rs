//! Reading the layered game data: the shipped layers (missing
//! expansion files skipped) and every installed mod under `mods/` as
//! a fill layer, into the [`LayerSet`]s that [`GameData::layered`]
//! composes.
//!
//! The archives — over a gigabyte, mostly off a network mount — are
//! read and parsed [`READERS`] at a time and assembled in layer order,
//! so the composition sees exactly what a one-at-a-time read would
//! have: the same layers in the same order, and the first failure in
//! that order as the load's failure. Each archive is reported as the
//! assembler turns to it, so a progress transcript reads as a
//! one-at-a-time load's would.
//!
//! Single entries of an archive are read by range through the
//! archive's directory ([`open_arc_index`], [`read_arc_entry`]): a
//! `UI.arc` runs to a quarter gigabyte and a shell wanting eleven
//! 4 KB textures out of it need not read the rest.
//!
//! [`GameData::layered`]: grimvault_core::gamedata::GameData::layered

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::thread;

use grimvault_core::gamedata::{ArchiveName, LayerFiles, LayerSet, ModListing};
use thiserror::Error;
use univault_engine::arc::{ArcError, ArcFile, ArcHeader, ArcIndex, Located};
use univault_engine::arz::{ArzDialect, ArzError, ArzFile};
use univault_engine::codec::Codec;

/// How many archives are read at once: enough to keep parsing off
/// the critical path, and no more — the bytes themselves are the
/// bound (a network mount's link, or a local disk), and past four
/// readers neither measured any faster (the commit that set this
/// records the numbers).
pub const READERS: usize = 4;

/// Why an archive could not be read or parsed.
#[derive(Debug, Error)]
pub enum ArchiveFailure {
    /// The file could not be read whole and verified.
    #[error("reading {}: {source}", path.display())]
    Read {
        /// The archive.
        path: PathBuf,
        /// The read failure.
        source: io::Error,
    },
    /// The bytes are not a record database this app reads.
    #[error("record database {}: {source}", path.display())]
    Database {
        /// The archive.
        path: PathBuf,
        /// The parse failure.
        source: ArzError,
    },
    /// The bytes are not a resource archive this app reads.
    #[error("archive {}: {source}", path.display())]
    Archive {
        /// The archive.
        path: PathBuf,
        /// The parse failure.
        source: ArcError,
    },
}

/// Which whole-file archive of a layer a read is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerFile {
    /// The record database (`.arz`).
    Database,
    /// The localization text archive.
    Text,
    /// The item bitmap archive.
    Items,
}

impl LayerFile {
    /// Every archive of a layer, in the order they are read.
    pub const ALL: [Self; 3] = [Self::Database, Self::Text, Self::Items];

    /// The archives a headless shell needs: records and text, not the
    /// item bitmaps (the bulk of the bytes).
    pub const HEADLESS: [Self; 2] = [Self::Database, Self::Text];

    fn relative(self, layer: &LayerFiles) -> &Path {
        match self {
            Self::Database => &layer.database,
            Self::Text => &layer.text,
            Self::Items => &layer.items,
        }
    }
}

/// One archive being taken up by the assembler, in layer order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveRead {
    /// Which archive of its layer.
    pub file: LayerFile,
    /// Its path, relative to the game directory.
    pub relative: PathBuf,
}

/// Whose [`LayerSet`] an archive fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Shipped,
    Mod,
}

/// An archive that is on disk and will be read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlannedRead {
    origin: Origin,
    file: LayerFile,
    relative: PathBuf,
}

impl PlannedRead {
    fn step(&self) -> ArchiveRead {
        ArchiveRead {
            file: self.file,
            relative: self.relative.clone(),
        }
    }
}

/// Every wanted archive of the shipped layers and then of the mod
/// layers, in layer order, skipping what is not on disk (an expansion
/// or mod resource that does not exist is not an error).
fn plan_reads(
    game_dir: &Path,
    shipped: &[LayerFiles],
    mods: &[LayerFiles],
    files: &[LayerFile],
) -> Vec<PlannedRead> {
    let present = |origin: Origin, layers: &[LayerFiles]| -> Vec<PlannedRead> {
        layers
            .iter()
            .flat_map(|layer| {
                files.iter().map(move |file| PlannedRead {
                    origin,
                    file: *file,
                    relative: file.relative(layer).to_path_buf(),
                })
            })
            .filter(|read| game_dir.join(&read.relative).is_file())
            .collect()
    };
    let mut plan = present(Origin::Shipped, shipped);
    plan.extend(present(Origin::Mod, mods));
    plan
}

/// An archive read and parsed on the thread that read it.
enum Parsed {
    Database(ArzFile),
    Text(ArcFile),
    Items(ArcFile),
}

/// Why an archive could not be read, without the path — the reader
/// works from the plan's index, and the assembler, which knows the
/// plan, names the file ([`ReadProblem::at`]).
#[derive(Debug)]
enum ReadProblem {
    Read(io::Error),
    Database(ArzError),
    Archive(ArcError),
    /// Every reader stopped without delivering this archive's result.
    NoResult,
}

impl ReadProblem {
    fn at(self, path: PathBuf) -> ArchiveFailure {
        match self {
            Self::Read(source) => ArchiveFailure::Read { path, source },
            Self::Database(source) => ArchiveFailure::Database { path, source },
            Self::Archive(source) => ArchiveFailure::Archive { path, source },
            Self::NoResult => ArchiveFailure::Read {
                path,
                source: io::Error::other("no reader delivered it"),
            },
        }
    }
}

/// One archive's bytes and parse. Archives are read-only reference
/// data on a possibly stale mount: the verified read turns a short
/// read into an error instead of a corrupt parse.
fn fetch(game_dir: &Path, read: &PlannedRead) -> Result<Parsed, ReadProblem> {
    let bytes =
        univault_io::read_verified(&game_dir.join(&read.relative)).map_err(ReadProblem::Read)?;
    let archive = |bytes| ArcFile::parse(bytes, Codec::Lz4Block).map_err(ReadProblem::Archive);
    match read.file {
        LayerFile::Database => ArzFile::parse(bytes, ArzDialect::grim_dawn())
            .map(Parsed::Database)
            .map_err(ReadProblem::Database),
        LayerFile::Text => archive(bytes).map(Parsed::Text),
        LayerFile::Items => archive(bytes).map(Parsed::Items),
    }
}

/// A reader's result for the archive at that index of the plan.
type Arrival = (usize, Result<Parsed, ReadProblem>);

/// Reads the wanted `files` of the `shipped` and then the `mods`
/// layers that exist under `game_dir`, [`READERS`] at a time, and
/// returns the two sets in layer order, reporting each archive to
/// `progress` as the assembler takes it up.
///
/// # Errors
/// [`ArchiveFailure`] for the first archive in layer order that could
/// not be read or parsed.
pub fn read_layers(
    game_dir: &Path,
    shipped: &[LayerFiles],
    mods: &[LayerFiles],
    files: &[LayerFile],
    progress: &mut dyn FnMut(ArchiveRead),
) -> Result<(LayerSet, LayerSet), ArchiveFailure> {
    let plan = plan_reads(game_dir, shipped, mods, files);
    read_planned(game_dir, &plan, READERS, progress)
}

/// Reads and parses the planned archives, `readers` at a time in plan
/// order, and assembles the shipped and the mod sets from them.
fn read_planned(
    game_dir: &Path,
    plan: &[PlannedRead],
    readers: usize,
    progress: &mut dyn FnMut(ArchiveRead),
) -> Result<(LayerSet, LayerSet), ArchiveFailure> {
    let next = AtomicUsize::new(0);
    let (sender, arrivals) = channel::<Arrival>();
    thread::scope(|scope| {
        for _ in 0..readers.min(plan.len()) {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(read) = plan.get(index) else { break };
                    if sender.send((index, fetch(game_dir, read))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        assemble(game_dir, plan, arrivals, progress)
    })
}

/// Files the arrivals into their sets in plan order, whatever order
/// they came in: each archive is reported as it is taken up, and the
/// first failure in plan order is the load's — later archives'
/// outcomes are never looked at.
fn assemble(
    game_dir: &Path,
    plan: &[PlannedRead],
    arrivals: impl IntoIterator<Item = Arrival>,
    progress: &mut dyn FnMut(ArchiveRead),
) -> Result<(LayerSet, LayerSet), ArchiveFailure> {
    let mut arrivals = arrivals.into_iter();
    let mut early = BTreeMap::new();
    let mut shipped = LayerSet::default();
    let mut mods = LayerSet::default();
    for (index, read) in plan.iter().enumerate() {
        progress(read.step());
        let parsed = take(index, &mut early, &mut arrivals)
            .unwrap_or(Err(ReadProblem::NoResult))
            .map_err(|problem| problem.at(game_dir.join(&read.relative)))?;
        let set = match read.origin {
            Origin::Shipped => &mut shipped,
            Origin::Mod => &mut mods,
        };
        match parsed {
            Parsed::Database(database) => set.databases.push(database),
            Parsed::Text(archive) => set.text_archives.push(archive),
            Parsed::Items(archive) => set.item_archives.push(archive),
        }
    }
    Ok((shipped, mods))
}

/// The result for `wanted`, holding any other archive's result that
/// arrives first for its own turn; `None` once the arrivals end
/// without it.
fn take(
    wanted: usize,
    early: &mut BTreeMap<usize, Result<Parsed, ReadProblem>>,
    arrivals: &mut impl Iterator<Item = Arrival>,
) -> Option<Result<Parsed, ReadProblem>> {
    loop {
        if let Some(result) = early.remove(&wanted) {
            return Some(result);
        }
        let (index, result) = arrivals.next()?;
        early.insert(index, result);
    }
}

/// Every folder under `mods/` with the names its `database/` and
/// `resources/` hold; an absent `mods/` is simply no mods.
#[must_use]
pub fn list_mods(game_dir: &Path) -> Vec<ModListing> {
    let Ok(folders) = std::fs::read_dir(game_dir.join("mods")) else {
        return Vec::new();
    };
    folders
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| ModListing {
            folder: entry.file_name().to_string_lossy().into_owned(),
            database_files: file_names(&entry.path().join("database")),
            resource_files: file_names(&entry.path().join("resources")),
        })
        .collect()
}

/// The names of the files in `dir`; none when it cannot be listed.
#[must_use]
pub fn file_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The path, relative to `game_dir`, of the archive `archive` names
/// in a layer's `resources` folder, matched case-insensitively; `None`
/// when the folder holds no such file.
#[must_use]
pub fn find_archive(game_dir: &Path, resources: &Path, archive: &ArchiveName) -> Option<PathBuf> {
    file_names(&game_dir.join(resources))
        .into_iter()
        .find(|name| archive.names_file(name))
        .map(|name| resources.join(name))
}

/// The directory of an archive, from its header and table region
/// alone.
///
/// # Errors
/// [`ArchiveFailure`] when the file cannot be read or is not an
/// archive.
pub fn open_arc_index(path: &Path) -> Result<ArcIndex, ArchiveFailure> {
    let archive = |source| ArchiveFailure::Archive {
        path: path.to_path_buf(),
        source,
    };
    let file_len = std::fs::metadata(path)
        .map_err(|source| ArchiveFailure::Read {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let header = read_ranges(path, std::slice::from_ref(&(0..ArcHeader::LEN)))?;
    let header = ArcHeader::parse(&header[0]).map_err(archive)?;
    let tables = header
        .tables_range(usize::try_from(file_len).unwrap_or(usize::MAX))
        .map_err(archive)?;
    let tables = read_ranges(path, std::slice::from_ref(&tables))?;
    ArcIndex::parse(header, &tables[0], Codec::Lz4Block).map_err(archive)
}

/// One entry's bytes, read as the ranges the directory names.
///
/// # Errors
/// [`ArchiveFailure`] when a range cannot be read or the parts do not
/// assemble.
pub fn read_arc_entry(
    path: &Path,
    index: &ArcIndex,
    located: &Located,
) -> Result<Vec<u8>, ArchiveFailure> {
    let parts = read_ranges(path, &located.ranges())?;
    let slices: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    index
        .assemble(located, &slices)
        .map_err(|source| ArchiveFailure::Archive {
            path: path.to_path_buf(),
            source,
        })
}

fn read_ranges(
    path: &Path,
    ranges: &[std::ops::Range<usize>],
) -> Result<Vec<Vec<u8>>, ArchiveFailure> {
    let ranges: Vec<std::ops::Range<u64>> = ranges
        .iter()
        .map(|range| {
            u64::try_from(range.start).unwrap_or(u64::MAX)
                ..u64::try_from(range.end).unwrap_or(u64::MAX)
        })
        .collect();
    univault_io::read_ranges(path, &ranges).map_err(|source| ArchiveFailure::Read {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use grimvault_core::gamedata::{mod_layers, shipped_layers};
    use univault_engine::arz::fixture::ArzBuilder;

    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("grimvault-layers-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, relative: &Path, bytes: &[u8]) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn database_with(record: &str) -> Vec<u8> {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(record, "ItemRelic", &[]);
        builder.build()
    }

    fn parsed_database(record: &str) -> Parsed {
        Parsed::Database(ArzFile::parse(database_with(record), ArzDialect::grim_dawn()).unwrap())
    }

    fn records(set: &LayerSet) -> Vec<String> {
        set.databases
            .iter()
            .flat_map(|database| database.record_ids().map(ToString::to_string))
            .collect()
    }

    fn database_read(origin: Origin, relative: &str) -> PlannedRead {
        PlannedRead {
            origin,
            file: LayerFile::Database,
            relative: PathBuf::from(relative),
        }
    }

    fn steps_of(plan: &[PlannedRead]) -> Vec<ArchiveRead> {
        plan.iter().map(PlannedRead::step).collect()
    }

    #[test]
    fn plan_reads_lists_what_is_on_disk_shipped_first_in_layer_order() {
        let scratch = Scratch::new("plan");
        let shipped = shipped_layers();
        let mods = mod_layers([ModListing {
            folder: "m".into(),
            database_files: vec!["m.arz".into()],
            resource_files: vec!["Items.arc".into()],
        }]);
        for relative in [
            &shipped[0].database,
            &shipped[0].text,
            &shipped[1].database,
            &mods[0].database,
            &mods[0].items,
        ] {
            scratch.write(relative, b"");
        }

        let plan = plan_reads(&scratch.0, &shipped, &mods, &LayerFile::ALL);

        let listed: Vec<(Origin, LayerFile, &Path)> = plan
            .iter()
            .map(|read| (read.origin, read.file, read.relative.as_path()))
            .collect();
        assert_eq!(
            listed,
            vec![
                (
                    Origin::Shipped,
                    LayerFile::Database,
                    shipped[0].database.as_path()
                ),
                (Origin::Shipped, LayerFile::Text, shipped[0].text.as_path()),
                (
                    Origin::Shipped,
                    LayerFile::Database,
                    shipped[1].database.as_path()
                ),
                (Origin::Mod, LayerFile::Database, mods[0].database.as_path()),
                (Origin::Mod, LayerFile::Items, mods[0].items.as_path()),
            ]
        );

        let headless = plan_reads(&scratch.0, &shipped, &mods, &LayerFile::HEADLESS);
        assert!(headless.iter().all(|read| read.file != LayerFile::Items));
        assert_eq!(headless.len(), 4);
    }

    #[test]
    fn assemble_files_arrivals_in_plan_order_whatever_order_they_came() {
        let plan = [
            database_read(Origin::Shipped, "a.arz"),
            database_read(Origin::Shipped, "b.arz"),
            database_read(Origin::Mod, "c.arz"),
        ];
        let arrivals = [
            (2, Ok(parsed_database("records/c.dbr"))),
            (0, Ok(parsed_database("records/a.dbr"))),
            (1, Ok(parsed_database("records/b.dbr"))),
        ];
        let mut reported = Vec::new();

        let (shipped, mods) = assemble(Path::new("/game"), &plan, arrivals, &mut |step| {
            reported.push(step);
        })
        .unwrap();

        assert_eq!(reported, steps_of(&plan));
        assert_eq!(records(&shipped), ["records/a.dbr", "records/b.dbr"]);
        assert_eq!(records(&mods), ["records/c.dbr"]);
    }

    #[test]
    fn the_first_failure_in_plan_order_is_the_loads_whichever_arrived_first() {
        let plan = [
            database_read(Origin::Shipped, "a.arz"),
            database_read(Origin::Shipped, "b.arz"),
            database_read(Origin::Shipped, "c.arz"),
        ];
        let arrivals = [
            (2, Err(ReadProblem::Archive(ArcError::NotArc))),
            (0, Ok(parsed_database("records/a.dbr"))),
            (
                1,
                Err(ReadProblem::Database(ArzError::DialectMismatch {
                    found: 1,
                    expected: 2,
                })),
            ),
        ];
        let mut reported = Vec::new();

        let failure = assemble(Path::new("/game"), &plan, arrivals, &mut |step| {
            reported.push(step);
        })
        .err()
        .unwrap();

        assert!(
            matches!(&failure, ArchiveFailure::Database { path, .. } if path == Path::new("/game/b.arz")),
            "{failure}"
        );
        assert_eq!(reported, steps_of(&plan[..2]));
    }

    #[test]
    fn an_archive_no_reader_delivered_is_a_failure_not_a_wait() {
        let plan = [
            database_read(Origin::Shipped, "a.arz"),
            database_read(Origin::Shipped, "b.arz"),
        ];
        let arrivals = [(0, Ok(parsed_database("records/a.dbr")))];

        let failure = assemble(Path::new("/game"), &plan, arrivals, &mut |_| {})
            .err()
            .unwrap();

        assert!(
            matches!(&failure, ArchiveFailure::Read { path, .. } if path == Path::new("/game/b.arz")),
            "{failure}"
        );
    }

    #[test]
    fn read_planned_reads_on_threads_and_keeps_layer_order() {
        let scratch = Scratch::new("threads");
        let shipped = shipped_layers();
        scratch.write(&shipped[0].database, &database_with("records/base.dbr"));
        scratch.write(&shipped[1].database, &database_with("records/x1.dbr"));
        scratch.write(&shipped[2].database, &database_with("records/x2.dbr"));
        let plan = plan_reads(&scratch.0, &shipped, &[], &LayerFile::ALL);
        let mut reported = Vec::new();

        let (loaded, mods) =
            read_planned(&scratch.0, &plan, 2, &mut |step| reported.push(step)).unwrap();

        assert_eq!(reported, steps_of(&plan));
        assert_eq!(
            records(&loaded),
            ["records/base.dbr", "records/x1.dbr", "records/x2.dbr"]
        );
        assert!(mods.databases.is_empty());
    }

    #[test]
    fn read_planned_fails_on_the_first_bad_archive_in_layer_order() {
        let scratch = Scratch::new("bad");
        let shipped = shipped_layers();
        scratch.write(&shipped[0].database, &database_with("records/base.dbr"));
        scratch.write(&shipped[1].database, b"not a database");
        scratch.write(&shipped[2].database, b"");
        let plan = plan_reads(&scratch.0, &shipped, &[], &LayerFile::ALL);
        let mut reported = Vec::new();

        let failure = read_planned(&scratch.0, &plan, 3, &mut |step| reported.push(step))
            .err()
            .unwrap();

        assert!(
            matches!(&failure, ArchiveFailure::Database { path, .. } if *path == scratch.0.join(&shipped[1].database)),
            "{failure}"
        );
        assert_eq!(reported, steps_of(&plan[..2]));
    }

    #[test]
    fn mods_are_listed_with_their_database_and_resource_files() {
        let scratch = Scratch::new("mods");
        scratch.write(Path::new("mods/Loot/database/loot.arz"), b"");
        scratch.write(Path::new("mods/Loot/resources/Text_EN.arc"), b"");
        scratch.write(Path::new("mods/stray-file"), b"");
        let mut listed = list_mods(&scratch.0);
        listed.sort_by(|a, b| a.folder.cmp(&b.folder));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].folder, "Loot");
        assert_eq!(listed[0].database_files, vec!["loot.arz".to_string()]);
        assert_eq!(listed[0].resource_files, vec!["Text_EN.arc".to_string()]);
        assert!(list_mods(&scratch.0.join("nowhere")).is_empty());
    }
}
