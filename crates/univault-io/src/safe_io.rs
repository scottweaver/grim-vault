// Vendored from tq-univault crates/univault-gui/src/safe_io.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! The shell's file IO path for game-owned files: backup-first
//! writes (ARCHITECTURE.md — the backup exists and is synced on disk
//! before the original is touched; backups are timestamped siblings
//! named by the app's [`BackupPolicy`], e.g.
//! `Player.chr.univault-bak-1756070000` for tq-univault or
//! `player.gdc.grimvault-bak-1756070000` for grim-vault, rotated to
//! the policy's newest few per file) and cache-bypassing verified
//! reads.
//!
//! Reads and writes here stay out of the local page cache. The save
//! tree lives on an SMB mount that other machines write — the game
//! runs elsewhere — and macOS has served *stale cached pages under a
//! fresh stamp* for such a file, minutes after another client rewrote
//! it. Bytes this app itself cached while writing were exactly what a
//! later launch was fed back, so both directions bypass the cache,
//! and every read is length-checked against the file's own metadata
//! to turn the surviving stale-read shapes into retryable errors
//! instead of clean parses of the wrong bytes.
//!
//! Every write of a target file is verified: once written in place
//! and synced, the file is re-read through [`read_verified`] and
//! compared to the bytes handed in. A disagreement comes back as an
//! error naming where the pre-write contents survive, never as a
//! success over a file that does not hold what the caller believes it
//! saved.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How an app names and rotates the backups this module takes:
/// `suffix` sits between the file name and the timestamp
/// (`Player.chr.<suffix>-<stamp>`, with `-<counter>` appended for
/// further backups within one second), and `max_backups` is how many
/// backups of one file survive a write — counting the one that write
/// takes, so it is never zero. Backups under another suffix are
/// invisible to a policy: two apps sharing a save tree never rotate
/// each other's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupPolicy {
    suffix: &'static str,
    max_backups: usize,
}

impl BackupPolicy {
    /// A policy keeping the newest `max_backups` backups named with
    /// `suffix`.
    ///
    /// # Panics
    ///
    /// When `max_backups` is zero: such a policy would prune the
    /// backup a write has just taken and silently defeat backup-first,
    /// so it is a defect — caught at compile time for a `const`
    /// policy.
    #[must_use]
    pub const fn new(suffix: &'static str, max_backups: usize) -> Self {
        assert!(
            max_backups > 0,
            "a backup policy keeps at least the backup each write takes"
        );
        Self {
            suffix,
            max_backups,
        }
    }

    /// The part of a backup's name between the file name and the
    /// stamp.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        self.suffix
    }

    /// How many backups of one file survive a write.
    #[must_use]
    pub const fn max_backups(self) -> usize {
        self.max_backups
    }
}

/// Reads the whole file, bypassing the local page cache and verifying
/// the byte count against the file's own metadata.
///
/// # Errors
///
/// Any failure opening or reading the file, and
/// [`io::ErrorKind::InvalidData`] when the byte count disagrees with
/// the metadata — the cache or the mount served bytes that are not
/// the file — so callers' existing retry machinery treats it as the
/// failed read it is, rather than parsing stale bytes that look
/// right.
pub fn read_verified(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    set_nocache(&file);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let on_disk = file.metadata()?.len();
    match length_mismatch(bytes.len(), on_disk) {
        None => Ok(bytes),
        Some(mismatch) => Err(io::Error::new(io::ErrorKind::InvalidData, mismatch)),
    }
}

/// Reads only the given byte ranges of a file, bypassing the local
/// page cache, each checked against the file's own metadata before it
/// is read — the way to take a few entries out of a very large
/// archive without reading the rest. Returns the ranges' bytes in the
/// order given.
///
/// # Errors
///
/// Any failure opening, seeking, or reading the file, and
/// [`io::ErrorKind::InvalidData`] when a range reaches past the length
/// the metadata reports — a stale or truncated view of the file — so
/// the caller sees a failed read rather than a short one.
pub fn read_ranges(path: &Path, ranges: &[Range<u64>]) -> io::Result<Vec<Vec<u8>>> {
    let mut file = File::open(path)?;
    set_nocache(&file);
    let on_disk = file.metadata()?.len();
    ranges
        .iter()
        .map(|range| {
            if range.end > on_disk || range.start > range.end {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "range {}..{} lies outside a file whose metadata says {on_disk} bytes",
                        range.start, range.end
                    ),
                ));
            }
            let len = usize::try_from(range.end - range.start)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "range too large"))?;
            file.seek(SeekFrom::Start(range.start))?;
            let mut bytes = vec![0; len];
            file.read_exact(&mut bytes)?;
            Ok(bytes)
        })
        .collect()
}

/// Backs `path` up beside itself, syncs the backup, prunes backups
/// beyond the policy, then overwrites `path` in place with `bytes`,
/// syncs, and re-reads it to prove the write landed. Returns the
/// backup's path. When `path` does not exist yet (a new vault), it is
/// simply created. The backup is taken through [`read_verified`] — a
/// save whose baseline cannot be faithfully backed up must fail
/// before the original is touched.
///
/// # Errors
///
/// Each step's failure, named: reading for backup, copying and
/// syncing the backup, writing and syncing the file, re-reading it,
/// and finally [`io::ErrorKind::InvalidData`] when the re-read file
/// is not `bytes` — that error names the backup still holding the
/// previous contents.
pub fn backup_first_write(
    path: &Path,
    bytes: &[u8],
    policy: BackupPolicy,
) -> io::Result<Option<PathBuf>> {
    let backup = if path.exists() {
        Some(take_backup(path, policy)?)
    } else {
        None
    };
    let prior = backup
        .as_deref()
        .map_or(PriorContents::NewFile, PriorContents::Backup);
    write_verified(path, bytes, prior)?;
    Ok(backup)
}

/// Overwrites `path` without taking a fresh backup, still syncing and
/// re-reading to prove the write landed — the autosave path for files
/// already backed up since they were last loaded.
///
/// # Errors
///
/// Writing, syncing, or re-reading failures, and
/// [`io::ErrorKind::InvalidData`] when the re-read file is not
/// `bytes`; that error points at the backup the first write since
/// load took.
pub fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_verified(path, bytes, PriorContents::EarlierBackup)
}

/// Creates or truncates `path` and writes `bytes` without leaving the
/// pages in the local cache — pages cached by our own writes are what
/// a stale-cache read serves back later.
///
/// # Errors
///
/// Any failure creating or writing the file.
pub fn write_uncached(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    set_nocache(&file);
    file.write_all(bytes)
}

/// Copies `path` through [`read_verified`] to a fresh sibling, syncs
/// it, and rotates away the older backups the policy no longer keeps.
fn take_backup(path: &Path, policy: BackupPolicy) -> io::Result<PathBuf> {
    let current = read_verified(path).map_err(|error| step("reading for backup", &error))?;
    let backup = fresh_backup_path(path, policy);
    write_uncached(&backup, &current).map_err(|error| step("copying backup", &error))?;
    copy_permissions(path, &backup);
    best_effort_sync(&backup).map_err(|error| step("syncing backup", &error))?;
    prune_backups(path, policy);
    Ok(backup)
}

/// Writes `bytes` over `path` in place, syncs, and proves the write
/// landed by re-reading the file through [`read_verified`]. `prior`
/// says where the previous contents survive, for the error a failed
/// proof carries.
fn write_verified(path: &Path, bytes: &[u8], prior: PriorContents<'_>) -> io::Result<()> {
    write_uncached(path, bytes).map_err(|error| step("writing file", &error))?;
    best_effort_sync(path).map_err(|error| step("syncing file", &error))?;
    let on_disk = read_verified(path).map_err(|error| step("re-reading written file", &error))?;
    match written_mismatch(&on_disk, bytes) {
        None => Ok(()),
        Some(mismatch) => Err(verification_failure(&mismatch, prior)),
    }
}

/// Where a target's pre-write contents survive, named by the error a
/// failed post-write verification returns.
#[derive(Clone, Copy, Debug)]
enum PriorContents<'a> {
    Backup(&'a Path),
    NewFile,
    EarlierBackup,
}

impl fmt::Display for PriorContents<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backup(backup) => write!(
                f,
                "the previous contents are backed up at {}",
                backup.display()
            ),
            Self::NewFile => f.write_str("the file was new, so there is no backup to restore"),
            Self::EarlierBackup => f.write_str(
                "the previous contents were backed up by the first write since the file was loaded",
            ),
        }
    }
}

fn verification_failure(mismatch: &str, prior: PriorContents<'_>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("verifying written file: {mismatch}; {prior}"),
    )
}

/// `None` when the re-read file holds exactly the bytes written,
/// otherwise the error text locating the disagreement.
fn written_mismatch(on_disk: &[u8], written: &[u8]) -> Option<String> {
    if on_disk == written {
        return None;
    }
    let detail = match on_disk.iter().zip(written).position(|(a, b)| a != b) {
        Some(offset) => format!("first differing byte at offset {offset}"),
        None => format!(
            "{} bytes on disk against {} written",
            on_disk.len(),
            written.len()
        ),
    };
    Some(format!(
        "re-read after write does not match what was written: {detail}"
    ))
}

/// `None` when the bytes read match the file's metadata, otherwise
/// the error text naming both counts.
fn length_mismatch(bytes_read: usize, on_disk: u64) -> Option<String> {
    (u64::try_from(bytes_read) != Ok(on_disk)).then(|| {
        format!(
            "read {bytes_read} bytes of a file whose metadata says {on_disk} — \
             a stale or mid-write network read; retrying usually sees the real file"
        )
    })
}

/// Asks the OS to keep this file's bytes out of the page cache.
/// Best-effort: a file that refuses the flag still reads and writes
/// correctly, merely through the cache again.
#[cfg(target_os = "macos")]
fn set_nocache(file: &File) {
    use std::os::fd::AsRawFd;
    // SAFETY: fcntl on a File's own fd, which is open for its
    // lifetime; F_NOCACHE takes a plain integer argument.
    let _ = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1) };
}

#[cfg(not(target_os = "macos"))]
fn set_nocache(_file: &File) {}

/// Best-effort permission mirroring onto a fresh backup, matching
/// what `fs::copy` used to preserve; a failure never fails the save
/// the backup protects.
fn copy_permissions(from: &Path, to: &Path) {
    if let Ok(metadata) = fs::metadata(from) {
        let _ = fs::set_permissions(to, metadata.permissions());
    }
}

/// Deletes the oldest backups of `path` beyond the policy's limit.
/// Best-effort: a failed prune never fails the save that a fresh,
/// synced backup already protects.
fn prune_backups(path: &Path, policy: BackupPolicy) {
    let mut backups = existing_backups(path, policy);
    backups.sort_unstable();
    let excess = backups.len().saturating_sub(policy.max_backups);
    for (_, _, old_backup) in backups.into_iter().take(excess) {
        let _ = fs::remove_file(old_backup);
    }
}

/// Every backup of `path` under the policy's suffix beside it, as
/// `(stamp, slot, path)` — the tuple orders oldest-first.
fn existing_backups(path: &Path, policy: BackupPolicy) -> Vec<(u64, u32, PathBuf)> {
    let Some(directory) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Vec::new();
    };
    let Some(file_name) = path.file_name().map(|name| name.to_string_lossy()) else {
        return Vec::new();
    };
    let suffix = policy.suffix;
    let prefix = format!("{file_name}.{suffix}-");
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let age = backup_age(name.strip_prefix(prefix.as_str())?)?;
            Some((age.0, age.1, entry.path()))
        })
        .collect()
}

/// Parses a backup's tail, `<stamp>` or `<stamp>-<counter>`, into an
/// ordering key; anything else is not one of our backups.
fn backup_age(tail: &str) -> Option<(u64, u32)> {
    match tail.split_once('-') {
        Some((stamp, counter)) => Some((
            stamp.parse().ok()?,
            counter.parse::<u32>().ok()?.checked_add(1)?,
        )),
        None => Some((tail.parse().ok()?, 0)),
    }
}

fn step(what: &str, error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

/// Flush-to-disk, degrading gracefully: macOS `sync_all` issues
/// `F_FULLFSYNC`, which network filesystems (SMB — "os error 45")
/// reject. There the close-flush is the strongest guarantee
/// available, so "unsupported" is not an error. The handle must be
/// writable: Windows' `FlushFileBuffers` denies read-only handles.
fn best_effort_sync(path: &Path) -> io::Result<()> {
    match File::options().write(true).open(path)?.sync_all() {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == io::ErrorKind::Unsupported || error.raw_os_error() == Some(45) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn fresh_backup_path(path: &Path, policy: BackupPolicy) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    // Rotation ages backups by name, so a new backup must never
    // reuse a pruned slot below the newest existing one — it would
    // be mistaken for the oldest and pruned on the next save.
    let newest = existing_backups(path, policy)
        .into_iter()
        .map(|(stamp, slot, _)| (stamp, slot))
        .max();
    let (stamp, mut counter) = match newest {
        Some((stamp, slot)) if stamp >= now => (stamp, slot),
        _ => (now, 0),
    };
    loop {
        let candidate = append_extension(path, &backup_tail(policy, stamp, counter));
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

/// `<suffix>-<stamp>` for the first backup taken in a second, then
/// `<suffix>-<stamp>-<counter>` for the ones after it.
fn backup_tail(policy: BackupPolicy, stamp: u64, counter: u32) -> String {
    let suffix = policy.suffix;
    if counter == 0 {
        format!("{suffix}-{stamp}")
    } else {
        format!("{suffix}-{stamp}-{counter}")
    }
}

fn append_extension(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map_or_else(
        || std::ffi::OsString::from("file"),
        std::ffi::OsStr::to_os_string,
    );
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TQ: BackupPolicy = BackupPolicy::new("univault-bak", 5);
    const GD: BackupPolicy = BackupPolicy::new("grimvault-bak", 5);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("univault-io-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self, file: &str) -> PathBuf {
            self.0.join(file)
        }

        fn names_with_prefix(&self, prefix: &str) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(prefix))
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn backup_exists_with_original_content_after_write() {
        let scratch = Scratch::new("backup");
        let target = scratch.path("Player.chr");
        fs::write(&target, b"original bytes").unwrap();

        let backup = backup_first_write(&target, b"new bytes", TQ)
            .unwrap()
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new bytes");
        assert_eq!(fs::read(&backup).unwrap(), b"original bytes");

        let second = backup_first_write(&target, b"third", TQ).unwrap().unwrap();
        assert_ne!(backup, second);
        assert_eq!(fs::read(&second).unwrap(), b"new bytes");
    }

    #[test]
    fn backups_rotate_keeping_the_newest_five() {
        let scratch = Scratch::new("rotate");
        let target = scratch.path("Player.chr");
        let decoy = scratch.path("Other.chr");
        fs::write(&target, b"v0").unwrap();
        fs::write(&decoy, b"other").unwrap();
        backup_first_write(&decoy, b"other2", TQ).unwrap();

        for round in 1..=8_u8 {
            backup_first_write(&target, &[round], TQ).unwrap();
        }
        let backups = scratch.names_with_prefix("Player.chr.univault-bak-");
        assert_eq!(backups.len(), TQ.max_backups(), "{backups:?}");
        let newest_content = fs::read(scratch.path(backups.last().unwrap())).unwrap();
        assert_eq!(newest_content, vec![7]);
        assert_eq!(
            scratch.names_with_prefix("Other.chr.univault-bak-").len(),
            1
        );
    }

    #[test]
    fn backup_suffixes_order_by_stamp_then_counter() {
        assert_eq!(backup_age("100"), Some((100, 0)));
        assert_eq!(backup_age("100-1"), Some((100, 2)));
        assert!(backup_age("100-1") > backup_age("100"));
        assert!(backup_age("101") > backup_age("100-9"));
        assert_eq!(backup_age("not-a-stamp"), None);
        assert_eq!(backup_age(""), None);
    }

    #[test]
    fn read_verified_returns_the_file_and_errors_on_a_missing_one() {
        let scratch = Scratch::new("readv");
        let target = scratch.path("winsys.dxb");
        fs::write(&target, b"stash bytes").unwrap();
        assert_eq!(read_verified(&target).unwrap(), b"stash bytes");
        assert!(read_verified(&scratch.path("absent.dxb")).is_err());
    }

    #[test]
    fn read_ranges_returns_each_range_and_refuses_one_past_the_end() {
        let scratch = Scratch::new("ranges");
        let target = scratch.path("UI.arc");
        fs::write(&target, b"0123456789abcdef").unwrap();
        assert_eq!(
            read_ranges(&target, &[0..4, 10..16, 5..5]).unwrap(),
            vec![b"0123".to_vec(), b"abcdef".to_vec(), Vec::new()]
        );
        let past_the_end = read_ranges(&target, &[0..4, 12..17]).unwrap_err();
        assert_eq!(past_the_end.kind(), io::ErrorKind::InvalidData);
        assert!(read_ranges(&scratch.path("absent.arc"), &[0..1, 2..3]).is_err());
    }

    #[test]
    fn length_mismatch_names_both_counts_and_passes_agreement() {
        assert_eq!(length_mismatch(19468, 19468), None);
        assert_eq!(length_mismatch(0, 0), None);
        let mismatch = length_mismatch(15234, 19468).unwrap();
        assert!(mismatch.contains("15234"), "{mismatch}");
        assert!(mismatch.contains("19468"), "{mismatch}");
    }

    #[test]
    fn creating_a_new_file_needs_no_backup() {
        let scratch = Scratch::new("new");
        let target = scratch.path("fresh.json");
        let backup = backup_first_write(&target, b"{}", GD).unwrap();
        assert!(backup.is_none());
        assert_eq!(fs::read(&target).unwrap(), b"{}");
    }

    #[test]
    fn backup_name_carries_the_policy_suffix() {
        let scratch = Scratch::new("suffix");
        let target = scratch.path("player.gdc");
        fs::write(&target, b"v0").unwrap();
        let backup = backup_first_write(&target, b"v1", GD).unwrap().unwrap();
        let name = backup.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("player.gdc.grimvault-bak-"), "{name}");
        assert_eq!(backup.parent(), target.parent());
        assert_eq!(GD.suffix(), "grimvault-bak");
    }

    #[test]
    fn policies_with_different_suffixes_rotate_independently() {
        let scratch = Scratch::new("independent");
        let target = scratch.path("player.gdc");
        fs::write(&target, b"v0").unwrap();
        backup_first_write(&target, b"tq", TQ).unwrap();
        for round in 0..8_u8 {
            backup_first_write(&target, &[round], GD).unwrap();
        }
        assert_eq!(
            scratch.names_with_prefix("player.gdc.univault-bak-").len(),
            1
        );
        assert_eq!(
            scratch.names_with_prefix("player.gdc.grimvault-bak-").len(),
            GD.max_backups()
        );
    }

    #[test]
    fn prune_limit_follows_the_policy() {
        let policy = BackupPolicy::new("grimvault-bak", 2);
        let scratch = Scratch::new("prune");
        let target = scratch.path("transfer.gst");
        fs::write(&target, [0]).unwrap();
        for round in 1..=6_u8 {
            backup_first_write(&target, &[round], policy).unwrap();
        }
        let backups = scratch.names_with_prefix("transfer.gst.grimvault-bak-");
        assert_eq!(backups.len(), 2, "{backups:?}");
        let newest_content = fs::read(scratch.path(backups.last().unwrap())).unwrap();
        assert_eq!(newest_content, vec![5]);
    }

    #[test]
    fn a_single_backup_policy_keeps_the_one_just_taken() {
        let policy = BackupPolicy::new("grimvault-bak", 1);
        let scratch = Scratch::new("single");
        let target = scratch.path("transfer.gst");
        fs::write(&target, b"v0").unwrap();
        for round in 1..=3_u8 {
            backup_first_write(&target, &[round], policy).unwrap();
        }
        let backups = scratch.names_with_prefix("transfer.gst.grimvault-bak-");
        assert_eq!(backups.len(), 1, "{backups:?}");
        assert_eq!(fs::read(scratch.path(&backups[0])).unwrap(), vec![2]);
    }

    #[test]
    #[should_panic(expected = "at least the backup")]
    fn a_policy_keeping_no_backups_is_a_defect() {
        let _ = BackupPolicy::new("grimvault-bak", 0);
    }

    #[test]
    fn a_fresh_backup_never_reuses_a_slot_below_the_newest() {
        let scratch = Scratch::new("monotonic");
        let target = scratch.path("Player.chr");
        fs::write(&target, b"v0").unwrap();
        fs::write(
            scratch.path("Player.chr.univault-bak-9999999999"),
            b"from a fast clock",
        )
        .unwrap();

        let next = fresh_backup_path(&target, TQ);
        assert_eq!(
            next.file_name().unwrap(),
            "Player.chr.univault-bak-9999999999-1"
        );
        fs::write(&next, b"still ahead").unwrap();
        let after = fresh_backup_path(&target, TQ);
        assert_eq!(
            after.file_name().unwrap(),
            "Player.chr.univault-bak-9999999999-2"
        );
    }

    #[test]
    fn writes_are_verified_by_re_reading_the_target() {
        let scratch = Scratch::new("verify");
        let target = scratch.path("transfer.gst");
        write_synced(&target, b"first").unwrap();
        assert_eq!(read_verified(&target).unwrap(), b"first");

        let backup = backup_first_write(&target, b"second", GD).unwrap();
        assert!(backup.is_some());
        assert_eq!(read_verified(&target).unwrap(), b"second");

        write_synced(&target, b"").unwrap();
        assert_eq!(read_verified(&target).unwrap(), b"");
    }

    #[test]
    fn written_mismatch_locates_the_disagreement_and_passes_agreement() {
        assert_eq!(written_mismatch(b"same", b"same"), None);
        assert_eq!(written_mismatch(b"", b""), None);
        let differing = written_mismatch(b"abXd", b"abcd").unwrap();
        assert!(differing.contains("offset 2"), "{differing}");
        let truncated = written_mismatch(b"abc", b"abcdef").unwrap();
        assert!(truncated.contains("3 bytes on disk"), "{truncated}");
        assert!(truncated.contains("6 written"), "{truncated}");
    }

    #[test]
    fn a_verification_failure_is_invalid_data_naming_the_backup() {
        let backup = Path::new("saves").join("player.gdc.grimvault-bak-1756070000");
        let with_backup = verification_failure("differs", PriorContents::Backup(&backup));
        assert_eq!(with_backup.kind(), io::ErrorKind::InvalidData);
        let message = with_backup.to_string();
        assert!(
            message.contains("player.gdc.grimvault-bak-1756070000"),
            "{message}"
        );
        assert!(message.contains("differs"), "{message}");

        let new_file = verification_failure("differs", PriorContents::NewFile).to_string();
        assert!(new_file.contains("no backup"), "{new_file}");

        let autosave = verification_failure("differs", PriorContents::EarlierBackup).to_string();
        assert!(autosave.contains("first write"), "{autosave}");
    }
}
