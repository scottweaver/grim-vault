// Ported in shape from tq-univault crates/univault-gui/src/ui_state.rs @ 36e7774 (MIT OR Apache-2.0, same author).
//! View state that survives a restart: the store pane's mode, the
//! bucket in view, the search bar and the table's sort. One small
//! self-describing JSON file (`ui-state.json`, format tag
//! [`FORMAT_TAG`]) under the config directory, beside the store. It is
//! a convenience, never data: an unreadable, foreign, or newer file is
//! ignored and the pane starts from defaults; unknown top-level fields
//! ride along untouched so a newer build's additions survive an older
//! one.
//!
//! Writes are debounced — the state is snapshotted every frame and
//! written once it has held still for [`QUIET`], so typing into the
//! search bar does not hit the disk per keystroke — and flushed on
//! exit.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::panes::store::StoreView;

/// The `format` tag every view-state file carries.
pub const FORMAT_TAG: &str = "grimvault-ui-state";
/// The newest document version this build reads and the one it writes.
pub const FORMAT_VERSION: u32 = 1;

/// How long the state must hold still before it is written.
const QUIET: Duration = Duration::from_secs(1);

/// Everything the file holds. Fields default individually, so a file
/// from an older build that lacks one still restores the rest.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    pub store: StoreView,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl UiState {
    /// The state a pane's view stands for, carrying forward whatever
    /// unknown fields the file on disk held.
    #[must_use]
    pub fn of(store: &StoreView, on_disk: &Self) -> Self {
        Self {
            store: store.clone(),
            extra: on_disk.extra.clone(),
        }
    }
}

/// The on-disk shape: the identity tags around the state.
#[derive(Serialize, Deserialize)]
struct UiStateFile {
    format: String,
    version: u32,
    #[serde(flatten)]
    state: UiState,
}

impl UiState {
    fn from_json(bytes: &[u8]) -> Option<Self> {
        let file: UiStateFile = serde_json::from_slice(bytes).ok()?;
        (file.format == FORMAT_TAG && file.version <= FORMAT_VERSION).then_some(file.state)
    }

    fn to_json(&self) -> Vec<u8> {
        let file = UiStateFile {
            format: FORMAT_TAG.to_string(),
            version: FORMAT_VERSION,
            state: self.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&file).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }
}

/// The file and what it holds, plus the pending-change clock the
/// debounce runs on.
pub struct PersistedUiState {
    path: PathBuf,
    on_disk: UiState,
    /// When the live state first diverged from `on_disk`; cleared by a
    /// write or by the state returning to what is on disk.
    pending_since: Option<Instant>,
}

impl PersistedUiState {
    /// Opens the file, restoring whatever it validly holds.
    #[must_use]
    pub fn load(path: PathBuf) -> Self {
        let on_disk = univault_io::read_verified(&path)
            .ok()
            .and_then(|bytes| UiState::from_json(&bytes))
            .unwrap_or_default();
        Self {
            path,
            on_disk,
            pending_since: None,
        }
    }

    /// The state restored at load (or last written).
    #[must_use]
    pub fn on_disk(&self) -> &UiState {
        &self.on_disk
    }

    /// Feeds this frame's live state. A change is written once it has
    /// held still for [`QUIET`]; until then the wait remaining is
    /// returned so the caller can schedule the next look.
    pub fn observe(&mut self, current: UiState, now: Instant) -> Option<Duration> {
        if current == self.on_disk {
            self.pending_since = None;
            return None;
        }
        let since = *self.pending_since.get_or_insert(now);
        let elapsed = now.saturating_duration_since(since);
        if elapsed < QUIET {
            return Some(QUIET.saturating_sub(elapsed));
        }
        self.write(current);
        None
    }

    /// Writes any pending change now — the exit path.
    pub fn flush(&mut self, current: UiState) {
        if current != self.on_disk {
            self.write(current);
        }
    }

    /// View state is a convenience: a write that fails is dropped
    /// silently and retried on the next change.
    fn write(&mut self, current: UiState) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = univault_io::write_synced(&self.path, &current.to_json());
        self.on_disk = current;
        self.pending_since = None;
    }
}

#[cfg(test)]
mod tests {
    use grimvault_core::bucket::{Bucket, Group};
    use grimvault_core::search::{Query, SortKey};
    use univault_ui::sort::SortDirection;

    use super::*;
    use crate::panes::store::StoreMode;
    use crate::search::{SearchView, Sort};

    fn changed() -> UiState {
        UiState {
            store: StoreView {
                group: Group::Armor,
                bucket: Bucket::Head,
                query: Query {
                    name: "cowl".into(),
                    ..Query::default()
                },
                mode: StoreMode::Search,
                search: SearchView {
                    query: Query {
                        name: "wendigo".into(),
                        ..Query::default()
                    },
                    sort: Sort {
                        key: SortKey::Level,
                        direction: SortDirection::Descending,
                    },
                },
            },
            extra: Map::new(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("grimvault-ui-state-{name}-{}", std::process::id()))
            .join("ui-state.json")
    }

    #[test]
    fn state_round_trips_through_its_file_shape() {
        let state = changed();
        let bytes = state.to_json();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.contains("\"format\": \"grimvault-ui-state\""),
            "{text}"
        );
        assert!(text.contains("\"version\": 1"), "{text}");
        assert!(text.ends_with('\n'));
        assert_eq!(UiState::from_json(&bytes), Some(state));
    }

    #[test]
    fn foreign_newer_and_malformed_files_are_ignored() {
        assert_eq!(
            UiState::from_json(br#"{"format":"grimvault-store","version":1}"#),
            None
        );
        assert_eq!(
            UiState::from_json(br#"{"format":"grimvault-ui-state","version":2}"#),
            None
        );
        assert_eq!(UiState::from_json(b"not json"), None);
    }

    #[test]
    fn missing_fields_restore_the_rest_and_unknown_ones_survive() {
        let partial = br#"{"format":"grimvault-ui-state","version":1,"store":{"mode":"search"},"later":{"a":1}}"#;
        let state = UiState::from_json(partial).expect("parses");
        assert_eq!(state.store.mode, StoreMode::Search);
        assert_eq!(state.store.bucket, StoreView::default().bucket);
        let carried = UiState::of(&changed().store, &state);
        let value: Value = serde_json::from_slice(&carried.to_json()).unwrap();
        assert_eq!(value["later"], serde_json::json!({ "a": 1 }));
        assert_eq!(value["store"]["mode"], serde_json::json!("search"));
    }

    #[test]
    fn a_change_is_written_only_after_it_holds_still() {
        let path = scratch("quiet");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let mut file = PersistedUiState::load(path.clone());
        let start = Instant::now();
        assert_eq!(file.observe(UiState::default(), start), None);
        assert!(!path.exists());

        assert_eq!(file.observe(changed(), start), Some(QUIET));
        assert!(!path.exists());
        assert_eq!(file.observe(changed(), start + QUIET / 2), Some(QUIET / 2));
        assert_eq!(file.observe(changed(), start + QUIET), None);
        assert_eq!(*PersistedUiState::load(path.clone()).on_disk(), changed());

        let mut reverted = changed();
        reverted.store.mode = StoreMode::Buckets;
        assert!(file.observe(reverted, start + QUIET).is_some());
        assert_eq!(file.observe(changed(), start + QUIET * 3), None);
        assert_eq!(*PersistedUiState::load(path.clone()).on_disk(), changed());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn flush_writes_without_waiting() {
        let path = scratch("flush");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let mut file = PersistedUiState::load(path.clone());
        file.flush(changed());
        assert_eq!(*PersistedUiState::load(path.clone()).on_disk(), changed());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
