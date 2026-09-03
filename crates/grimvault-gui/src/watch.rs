//! The external-change guard's eyes: a thread that stats the open
//! files every [`WATCH_INTERVAL`] and reports what it saw, and the pure
//! [`RefreshTracker`] that decides when a change is *believed* — the
//! same stamp must be observed on two consecutive polls, so a save the
//! game is still writing (over SMB, from another machine) is never
//! acted on half-done.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Duration;

use crate::documents::{FileStamp, stamp_of};

/// Poll cadence.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// What one poll observation means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    /// The file matches what this app last read or wrote.
    Unchanged,
    /// The file could not be stat'ed; nothing is concluded.
    Unreachable,
    /// The file differs, seen once; wait for corroboration.
    Settling,
    /// The file differs and has held still: act on it.
    Settled,
}

/// Two-poll corroboration per path.
#[derive(Debug, Default)]
pub struct RefreshTracker {
    pending: HashMap<PathBuf, FileStamp>,
}

impl RefreshTracker {
    /// Feeds one observation of `path` against `ours`, the stamp this
    /// app holds for it.
    pub fn observe(
        &mut self,
        path: &Path,
        seen: Option<FileStamp>,
        ours: Option<FileStamp>,
    ) -> Observation {
        let Some(seen) = seen else {
            self.pending.remove(path);
            return Observation::Unreachable;
        };
        if Some(seen) == ours {
            self.pending.remove(path);
            return Observation::Unchanged;
        }
        if self.pending.get(path) == Some(&seen) {
            return Observation::Settled;
        }
        self.pending.insert(path.to_path_buf(), seen);
        Observation::Settling
    }

    /// Drops what was seen of `path`, for a file just (re)opened or
    /// deliberately adopted.
    pub fn forget(&mut self, path: &Path) {
        self.pending.remove(path);
    }
}

/// One round of stats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Poll {
    pub stamps: Vec<(PathBuf, Option<FileStamp>)>,
}

/// The poller thread's handles: the paths it should watch go one way,
/// its polls come back the other. Stat calls can hang on an
/// unreachable mount, so they never run on the UI thread.
pub struct Watcher {
    paths: Sender<Vec<PathBuf>>,
    polls: Receiver<Poll>,
}

impl Watcher {
    /// Starts the thread; every poll asks `wake` to repaint so the
    /// shell notices even when nothing else is happening.
    ///
    /// # Errors
    /// The thread could not be spawned — the guard is then off, and
    /// the shell must say so.
    pub fn start(wake: egui::Context) -> std::io::Result<Self> {
        let (paths, path_updates) = channel::<Vec<PathBuf>>();
        let (report, polls) = channel::<Poll>();
        std::thread::Builder::new()
            .name("grimvault-watch".into())
            .spawn(move || poll_loop(&path_updates, &report, &wake))?;
        Ok(Self { paths, polls })
    }

    /// Replaces the watched set.
    pub fn watch(&self, paths: Vec<PathBuf>) {
        let _ = self.paths.send(paths);
    }

    /// Every poll that arrived since the last drain, oldest first.
    pub fn drain(&self) -> Vec<Poll> {
        let mut polls = Vec::new();
        while let Ok(poll) = self.polls.try_recv() {
            polls.push(poll);
        }
        polls
    }
}

fn poll_loop(path_updates: &Receiver<Vec<PathBuf>>, report: &Sender<Poll>, wake: &egui::Context) {
    let mut watched: Vec<PathBuf> = Vec::new();
    loop {
        std::thread::sleep(WATCH_INTERVAL);
        loop {
            match path_updates.try_recv() {
                Ok(paths) => watched = paths,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if watched.is_empty() {
            continue;
        }
        let stamps = watched
            .iter()
            .map(|path| (path.clone(), stamp_of(path)))
            .collect();
        if report.send(Poll { stamps }).is_err() {
            return;
        }
        wake.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn stamp(size: u64, seconds: u64) -> FileStamp {
        FileStamp::synthetic(size, UNIX_EPOCH + Duration::from_secs(seconds))
    }

    #[test]
    fn a_change_is_believed_only_on_the_second_identical_poll() {
        let mut tracker = RefreshTracker::default();
        let path = Path::new("/saves/transfer.gst");
        let ours = Some(stamp(10, 100));
        let theirs = Some(stamp(12, 200));
        assert_eq!(tracker.observe(path, ours, ours), Observation::Unchanged);
        assert_eq!(tracker.observe(path, theirs, ours), Observation::Settling);
        assert_eq!(tracker.observe(path, theirs, ours), Observation::Settled);
        assert_eq!(tracker.observe(path, theirs, ours), Observation::Settled);
    }

    #[test]
    fn a_still_moving_file_keeps_settling() {
        let mut tracker = RefreshTracker::default();
        let path = Path::new("/saves/transfer.gst");
        let ours = Some(stamp(10, 100));
        assert_eq!(
            tracker.observe(path, Some(stamp(12, 200)), ours),
            Observation::Settling
        );
        assert_eq!(
            tracker.observe(path, Some(stamp(14, 201)), ours),
            Observation::Settling
        );
        assert_eq!(
            tracker.observe(path, Some(stamp(14, 201)), ours),
            Observation::Settled
        );
    }

    #[test]
    fn unreachable_and_matching_observations_reset_corroboration() {
        let mut tracker = RefreshTracker::default();
        let path = Path::new("/saves/transfer.gst");
        let ours = Some(stamp(10, 100));
        let theirs = Some(stamp(12, 200));
        assert_eq!(tracker.observe(path, theirs, ours), Observation::Settling);
        assert_eq!(tracker.observe(path, None, ours), Observation::Unreachable);
        assert_eq!(tracker.observe(path, theirs, ours), Observation::Settling);
        assert_eq!(tracker.observe(path, ours, ours), Observation::Unchanged);
        assert_eq!(tracker.observe(path, theirs, ours), Observation::Settling);
        tracker.forget(path);
        assert_eq!(tracker.observe(path, theirs, ours), Observation::Settling);
    }

    #[test]
    fn a_file_that_appears_where_none_was_is_a_change() {
        let mut tracker = RefreshTracker::default();
        let path = Path::new("/cfg/vault-store.json");
        let theirs = Some(stamp(3, 300));
        assert_eq!(tracker.observe(path, None, None), Observation::Unreachable);
        assert_eq!(tracker.observe(path, theirs, None), Observation::Settling);
        assert_eq!(tracker.observe(path, theirs, None), Observation::Settled);
    }
}
