//! The open files as the shell holds them: the transfer stash, the
//! component / crafting-material storage, the vault store, and the
//! read-only characters. Each writable document knows the disk stamp
//! it was read under (the external-change guard's baseline), whether
//! it holds unsaved edits, and whether this load's backup has been
//! taken yet — the backup-first rule is *one backup per load*, so the
//! first write since load takes it and later writes reuse it
//! (ARCHITECTURE.md "Data flow").
//!
//! `player.gdc` is never written by this build — the vault loop edits
//! only `transfer.gst` and `reagents.gst` — so [`CharacterDoc`] has no
//! save path at all.

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use grimvault_core::block::SaveEncodeError;
use grimvault_core::crypto::BlockId;
use grimvault_core::gdc::{GdcError, PlayerFile};
use grimvault_core::gst::{GstError, GstFile, ReagentStorage, TransferStash};
use grimvault_core::loaded::{LoadError, Loaded};
use grimvault_core::store::{StoreError, VaultStore};
use thiserror::Error;
use univault_io::{BackupPolicy, backup_first_write, read_verified, write_synced};

use crate::setup::SaveDir;

/// How this app names and rotates the backups it takes.
pub const BACKUPS: BackupPolicy = BackupPolicy::new("grimvault-bak", 5);

/// A file's size and modification time, as the guard compares them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileStamp {
    size: u64,
    modified: SystemTime,
}

impl FileStamp {
    /// A stamp that never came from a file, for the guard's tests.
    #[cfg(test)]
    pub(crate) const fn synthetic(size: u64, modified: SystemTime) -> Self {
        Self { size, modified }
    }
}

/// The stamp of `path` now; `None` when it cannot be stat'ed (absent,
/// or an unreachable mount).
#[must_use]
pub fn stamp_of(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileStamp {
        size: metadata.len(),
        modified: metadata.modified().ok()?,
    })
}

/// Whether a document differs from what was last read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edits {
    Saved,
    Unsaved,
}

/// Whether this load's backup has been taken yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backup {
    Armed,
    Taken,
}

/// How one save attempt concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Written, verified, and restamped; `backup` names the backup
    /// when this write took one.
    Saved { backup: Option<PathBuf> },
    /// Not written: the file on disk no longer matches the stamp taken
    /// at load or last write.
    Conflict,
}

/// Why a save failed outright.
#[derive(Debug, Error)]
pub enum SaveError {
    #[error("encoding: {0}")]
    Encode(#[from] SaveEncodeError),
    #[error("writing {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
}

/// The writable documents, for labels and the conflict list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doc {
    Stash,
    Store,
    Reagents,
}

impl Doc {
    /// Every writable document.
    pub const ALL: [Doc; 3] = [Doc::Stash, Doc::Store, Doc::Reagents];
}

impl fmt::Display for Doc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stash => "transfer stash",
            Self::Store => "vault store",
            Self::Reagents => "component storage",
        })
    }
}

/// Backup-first on the first write since load, plain synced writes
/// after; the backup state advances only once the write succeeded.
fn write_document(path: &Path, bytes: &[u8], backup: &mut Backup) -> io::Result<Option<PathBuf>> {
    match backup {
        Backup::Armed => {
            let taken = backup_first_write(path, bytes, BACKUPS)?;
            *backup = Backup::Taken;
            Ok(taken)
        }
        Backup::Taken => write_synced(path, bytes).map(|()| None),
    }
}

/// Why a `.gst` could not be opened for editing.
#[derive(Debug, Error)]
pub enum GstOpenError {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Load {
        path: PathBuf,
        source: LoadError<GstError, SaveEncodeError>,
    },
    #[error("{} carries no typed {block}", path.display())]
    NoTypedBlock { path: PathBuf, block: BlockId },
}

/// The typed block a [`GstDoc`] edits, and how to find it in the file.
pub trait EditableBlock: fmt::Debug + Sized {
    /// The block's id, for the error when a file lacks it.
    const ID: BlockId;
    fn of(file: &GstFile) -> Option<&Self>;
    fn of_mut(file: &mut GstFile) -> Option<&mut Self>;
}

impl EditableBlock for TransferStash {
    const ID: BlockId = BlockId::TRANSFER_STASH;

    fn of(file: &GstFile) -> Option<&Self> {
        file.transfer_stash()
    }

    fn of_mut(file: &mut GstFile) -> Option<&mut Self> {
        file.transfer_stash_mut()
    }
}

impl EditableBlock for ReagentStorage {
    const ID: BlockId = BlockId::REAGENT_STORAGE;

    fn of(file: &GstFile) -> Option<&Self> {
        file.reagent_storage()
    }

    fn of_mut(file: &mut GstFile) -> Option<&mut Self> {
        file.reagent_storage_mut()
    }
}

/// A `.gst` editable through its typed block `B`, because it passed
/// the lossless gate and carries that block.
#[derive(Debug)]
pub struct GstDoc<B: EditableBlock> {
    path: PathBuf,
    loaded: Loaded<GstFile>,
    stamp: Option<FileStamp>,
    edits: Edits,
    backup: Backup,
    block: PhantomData<B>,
}

/// The shared stash.
pub type StashDoc = GstDoc<TransferStash>;
/// The component / crafting-material storage.
pub type ReagentDoc = GstDoc<ReagentStorage>;

impl<B: EditableBlock> GstDoc<B> {
    /// Reads, gates, and stamps the file.
    ///
    /// # Errors
    /// [`GstOpenError`], including a `NotLossless` load — never edited
    /// around.
    pub fn open(path: PathBuf) -> Result<Self, GstOpenError> {
        let bytes = read_verified(&path).map_err(|source| GstOpenError::Read {
            path: path.clone(),
            source,
        })?;
        let stamp = stamp_of(&path);
        let loaded = Loaded::<GstFile>::load(bytes).map_err(|source| GstOpenError::Load {
            path: path.clone(),
            source,
        })?;
        if B::of(loaded.model()).is_none() {
            return Err(GstOpenError::NoTypedBlock { path, block: B::ID });
        }
        Ok(Self {
            path,
            loaded,
            stamp,
            edits: Edits::Saved,
            backup: Backup::Armed,
            block: PhantomData,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The typed block. `open` proved it present and `GstFile` cannot
    /// drop a block, so the lookup cannot fail.
    #[must_use]
    pub fn block(&self) -> &B {
        B::of(self.loaded.model())
            .expect("open refused a file without the block and blocks cannot be removed")
    }

    /// The typed block for editing; call [`Self::mark_edited`] after.
    pub fn block_mut(&mut self) -> &mut B {
        B::of_mut(self.loaded.model_mut())
            .expect("open refused a file without the block and blocks cannot be removed")
    }

    pub fn mark_edited(&mut self) {
        self.edits = Edits::Unsaved;
    }

    #[must_use]
    pub fn edits(&self) -> Edits {
        self.edits
    }

    #[must_use]
    pub fn backup(&self) -> Backup {
        self.backup
    }

    #[must_use]
    pub fn stamp(&self) -> Option<FileStamp> {
        self.stamp
    }

    /// Size of the bytes the model was proven against.
    #[must_use]
    pub fn baseline_len(&self) -> usize {
        self.loaded.baseline().len()
    }

    /// Writes the model unless the file changed underneath.
    ///
    /// # Errors
    /// [`SaveError`] when the encode or the write fails; the edits stay
    /// unsaved.
    pub fn save(&mut self) -> Result<SaveOutcome, SaveError> {
        if stamp_of(&self.path) != self.stamp {
            return Ok(SaveOutcome::Conflict);
        }
        let bytes = self.loaded.encode()?;
        let backup = write_document(&self.path, &bytes, &mut self.backup).map_err(|source| {
            SaveError::Write {
                path: self.path.clone(),
                source,
            }
        })?;
        self.stamp = stamp_of(&self.path);
        self.edits = Edits::Saved;
        Ok(SaveOutcome::Saved { backup })
    }

    /// "Keep mine": adopts the external version's stamp and re-arms
    /// backup-first so that version is backed up before the next save
    /// overwrites it.
    pub fn keep_mine(&mut self) {
        self.stamp = stamp_of(&self.path);
        self.backup = Backup::Armed;
    }

    /// Re-reads the file, dropping in-memory edits. On failure the
    /// document is untouched, edits included.
    ///
    /// # Errors
    /// [`GstOpenError`].
    pub fn reload(&mut self) -> Result<(), GstOpenError> {
        *self = Self::open(self.path.clone())?;
        Ok(())
    }
}

impl StashDoc {
    /// Block 18.
    #[must_use]
    pub fn stash(&self) -> &TransferStash {
        self.block()
    }

    /// Block 18 for editing; call [`Self::mark_edited`] after.
    pub fn stash_mut(&mut self) -> &mut TransferStash {
        self.block_mut()
    }
}

impl ReagentDoc {
    /// Block 20.
    #[must_use]
    pub fn storage(&self) -> &ReagentStorage {
        self.block()
    }

    /// Block 20 for editing; call [`Self::mark_edited`] after.
    pub fn storage_mut(&mut self) -> &mut ReagentStorage {
        self.block_mut()
    }
}

/// `reagents.gst` however it fared: the game writes it only once the
/// storage has been used, so a save directory without it is normal,
/// and a file this build cannot type is shown, not fatal.
#[derive(Debug)]
pub enum Reagents {
    Open(ReagentDoc),
    Absent {
        path: PathBuf,
    },
    /// Present but not editable; `stamp` is the file as it was found,
    /// so the guard only reacts when it changes again.
    Failed {
        path: PathBuf,
        stamp: Option<FileStamp>,
        error: GstOpenError,
    },
}

impl Reagents {
    /// Opens `path` when it exists.
    #[must_use]
    pub fn open(path: PathBuf) -> Self {
        if !path.is_file() {
            return Self::Absent { path };
        }
        let stamp = stamp_of(&path);
        match ReagentDoc::open(path.clone()) {
            Ok(doc) => Self::Open(doc),
            Err(error) => Self::Failed { path, stamp, error },
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Open(doc) => doc.path(),
            Self::Absent { path } | Self::Failed { path, .. } => path,
        }
    }

    /// The stamp the guard compares against: the document's, or the
    /// file's as it was found unusable; `None` while absent.
    #[must_use]
    pub fn stamp(&self) -> Option<FileStamp> {
        match self {
            Self::Open(doc) => doc.stamp(),
            Self::Absent { .. } => None,
            Self::Failed { stamp, .. } => *stamp,
        }
    }

    /// The document's edits; an absent or unusable file has none.
    #[must_use]
    pub fn edits(&self) -> Edits {
        self.doc().map_or(Edits::Saved, GstDoc::edits)
    }

    #[must_use]
    pub fn doc(&self) -> Option<&ReagentDoc> {
        match self {
            Self::Open(doc) => Some(doc),
            Self::Absent { .. } | Self::Failed { .. } => None,
        }
    }

    pub fn doc_mut(&mut self) -> Option<&mut ReagentDoc> {
        match self {
            Self::Open(doc) => Some(doc),
            Self::Absent { .. } | Self::Failed { .. } => None,
        }
    }

    /// Re-reads the file: an open document reloads in place (keeping
    /// its edits on failure), an absent or failed one is opened afresh
    /// — whatever the disk now holds becomes the state, a file the
    /// game has since written included.
    ///
    /// # Errors
    /// [`GstOpenError`] when an open document could not be re-read.
    pub fn reload(&mut self) -> Result<(), GstOpenError> {
        match self {
            Self::Open(doc) => doc.reload(),
            Self::Absent { path } | Self::Failed { path, .. } => {
                *self = Self::open(path.clone());
                Ok(())
            }
        }
    }
}

/// Why the store could not be opened.
#[derive(Debug, Error)]
pub enum StoreOpenError {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Parse { path: PathBuf, source: StoreError },
}

/// This app's own vault store. An absent file is an empty store whose
/// first save creates it.
#[derive(Debug)]
pub struct StoreDoc {
    path: PathBuf,
    store: VaultStore,
    stamp: Option<FileStamp>,
    edits: Edits,
    backup: Backup,
}

impl StoreDoc {
    /// Reads the store, or starts an empty one when the file is absent.
    ///
    /// # Errors
    /// [`StoreOpenError`] when the file exists but cannot be read or
    /// parsed.
    pub fn open(path: PathBuf) -> Result<Self, StoreOpenError> {
        let (store, stamp) = if path.is_file() {
            let bytes = read_verified(&path).map_err(|source| StoreOpenError::Read {
                path: path.clone(),
                source,
            })?;
            let stamp = stamp_of(&path);
            let store = VaultStore::from_json(&bytes).map_err(|source| StoreOpenError::Parse {
                path: path.clone(),
                source,
            })?;
            (store, stamp)
        } else {
            (VaultStore::new(), None)
        };
        Ok(Self {
            path,
            store,
            stamp,
            edits: Edits::Saved,
            backup: Backup::Armed,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn store(&self) -> &VaultStore {
        &self.store
    }

    /// The store for editing; call [`Self::mark_edited`] after.
    pub fn store_mut(&mut self) -> &mut VaultStore {
        &mut self.store
    }

    pub fn mark_edited(&mut self) {
        self.edits = Edits::Unsaved;
    }

    #[must_use]
    pub fn edits(&self) -> Edits {
        self.edits
    }

    #[must_use]
    pub fn backup(&self) -> Backup {
        self.backup
    }

    #[must_use]
    pub fn stamp(&self) -> Option<FileStamp> {
        self.stamp
    }

    /// Writes the store unless the file changed underneath, creating
    /// the config directory on the first save.
    ///
    /// # Errors
    /// [`SaveError::Write`]; the edits stay unsaved.
    pub fn save(&mut self) -> Result<SaveOutcome, SaveError> {
        if stamp_of(&self.path) != self.stamp {
            return Ok(SaveOutcome::Conflict);
        }
        let backup = self
            .path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| write_document(&self.path, &self.store.to_json(), &mut self.backup))
            .map_err(|source| SaveError::Write {
                path: self.path.clone(),
                source,
            })?;
        self.stamp = stamp_of(&self.path);
        self.edits = Edits::Saved;
        Ok(SaveOutcome::Saved { backup })
    }

    /// See [`GstDoc::keep_mine`].
    pub fn keep_mine(&mut self) {
        self.stamp = stamp_of(&self.path);
        self.backup = Backup::Armed;
    }

    /// See [`GstDoc::reload`].
    ///
    /// # Errors
    /// [`StoreOpenError`].
    pub fn reload(&mut self) -> Result<(), StoreOpenError> {
        *self = Self::open(self.path.clone())?;
        Ok(())
    }
}

/// Why a character could not be shown.
#[derive(Debug, Error)]
pub enum CharacterOpenError {
    #[error("reading: {0}")]
    Read(#[from] io::Error),
    #[error("{0}")]
    Load(#[from] LoadError<GdcError, SaveEncodeError>),
}

/// A character, read-only: this build has no write path for
/// `player.gdc`.
#[derive(Debug)]
pub struct CharacterDoc {
    path: PathBuf,
    file: PlayerFile,
}

impl CharacterDoc {
    /// Reads and gates a `player.gdc`.
    ///
    /// # Errors
    /// [`CharacterOpenError`].
    pub fn open(path: PathBuf) -> Result<Self, CharacterOpenError> {
        let bytes = read_verified(&path)?;
        let loaded = Loaded::<PlayerFile>::load(bytes)?;
        Ok(Self {
            path,
            file: loaded.model().clone(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn file(&self) -> &PlayerFile {
        &self.file
    }

    #[must_use]
    pub fn name(&self) -> &str {
        self.file.character_name()
    }
}

/// One `main/*/player.gdc`, however it fared.
#[derive(Debug)]
pub enum CharacterEntry {
    Loaded(CharacterDoc),
    Failed {
        path: PathBuf,
        error: CharacterOpenError,
    },
}

impl CharacterEntry {
    /// The character's name, or the folder's when the file is
    /// unreadable.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Loaded(doc) => doc.name().to_string(),
            Self::Failed { path, .. } => path.parent().and_then(Path::file_name).map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().trim_start_matches('_').to_string(),
            ),
        }
    }
}

/// Every character under `main/`, in folder order, failures included.
#[must_use]
pub fn open_characters(save_dir: &SaveDir) -> Vec<CharacterEntry> {
    let Ok(entries) = std::fs::read_dir(save_dir.characters_dir()) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
        .into_iter()
        .map(|folder| folder.join("player.gdc"))
        .filter(|path| path.is_file())
        .map(|path| match CharacterDoc::open(path.clone()) {
            Ok(doc) => CharacterEntry::Loaded(doc),
            Err(error) => CharacterEntry::Failed { path, error },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use grimvault_core::crypto::{EncodeError, Encoder};
    use grimvault_core::gst::ReagentEntry;

    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("grimvault-docs-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn backups_of(path: &Path) -> usize {
        let prefix = format!(
            "{}.grimvault-bak-",
            path.file_name().unwrap().to_string_lossy()
        );
        std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .count()
    }

    fn reagents_bytes() -> Vec<u8> {
        let mut enc = Encoder::new(0x77DF_33C5);
        enc.write_u32(1);
        enc.write_block(BlockId::REAGENT_STORAGE, |enc| {
            enc.write_u32(1);
            enc.write_zero_marker();
            enc.write_u32(0);
            enc.write_u32(1);
            enc.write_block(BlockId::NESTED, |enc| {
                enc.write_string("records/items/materia/compa_moltenskin.dbr")?;
                enc.write_u32(20);
                Ok::<(), EncodeError>(())
            })
        })
        .unwrap();
        enc.finish()
    }

    #[test]
    fn a_store_starts_empty_backs_up_once_per_load_and_restamps() {
        let scratch = Scratch::new("store");
        let path = scratch.0.join("nested").join("vault-store.json");
        let mut doc = StoreDoc::open(path.clone()).unwrap();
        assert!(doc.store().is_empty());
        assert_eq!(doc.stamp(), None);
        assert_eq!(doc.backup(), Backup::Armed);

        doc.mark_edited();
        assert_eq!(doc.edits(), Edits::Unsaved);
        assert!(matches!(
            doc.save().unwrap(),
            SaveOutcome::Saved { backup: None }
        ));
        assert_eq!(doc.edits(), Edits::Saved);
        assert_eq!(doc.backup(), Backup::Taken);
        assert_eq!(doc.stamp(), stamp_of(&path));
        assert_eq!(backups_of(&path), 0);

        doc.mark_edited();
        assert!(matches!(
            doc.save().unwrap(),
            SaveOutcome::Saved { backup: None }
        ));
        assert_eq!(backups_of(&path), 0);

        let mut reopened = StoreDoc::open(path.clone()).unwrap();
        reopened.mark_edited();
        let outcome = reopened.save().unwrap();
        assert!(
            matches!(outcome, SaveOutcome::Saved { backup: Some(_) }),
            "{outcome:?}"
        );
        assert_eq!(backups_of(&path), 1);
    }

    #[test]
    fn an_external_change_turns_a_save_into_a_conflict_until_kept() {
        let scratch = Scratch::new("conflict");
        let path = scratch.0.join("vault-store.json");
        let mut doc = StoreDoc::open(path.clone()).unwrap();
        doc.mark_edited();
        doc.save().unwrap();

        std::fs::write(&path, VaultStore::new().to_json()).unwrap();
        let external = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(external + std::time::Duration::from_secs(5))
            .unwrap();
        doc.mark_edited();
        assert_eq!(doc.save().unwrap(), SaveOutcome::Conflict);
        assert_eq!(doc.edits(), Edits::Unsaved);

        doc.keep_mine();
        assert_eq!(doc.backup(), Backup::Armed);
        let outcome = doc.save().unwrap();
        assert!(
            matches!(outcome, SaveOutcome::Saved { backup: Some(_) }),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_failed_reload_keeps_the_document_and_its_edits() {
        let scratch = Scratch::new("reload");
        let path = scratch.0.join("vault-store.json");
        let mut doc = StoreDoc::open(path.clone()).unwrap();
        doc.mark_edited();
        doc.save().unwrap();
        doc.mark_edited();
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(doc.reload().is_err());
        assert_eq!(doc.edits(), Edits::Unsaved);
    }

    #[test]
    fn a_stash_that_is_not_a_save_is_refused() {
        let scratch = Scratch::new("stash");
        let path = scratch.0.join("transfer.gst");
        std::fs::write(&path, b"\x0b\x00\x00\x00begin_block").unwrap();
        assert!(matches!(
            StashDoc::open(path).unwrap_err(),
            GstOpenError::Load { .. }
        ));
        assert!(matches!(
            StashDoc::open(scratch.0.join("absent.gst")).unwrap_err(),
            GstOpenError::Read { .. }
        ));
    }

    #[test]
    fn a_reagent_file_edits_saves_backup_first_and_reloads() {
        let scratch = Scratch::new("reagents");
        let path = scratch.0.join("reagents.gst");
        let bytes = reagents_bytes();
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            StashDoc::open(path.clone()).unwrap_err(),
            GstOpenError::NoTypedBlock { block, .. } if block == BlockId::TRANSFER_STASH
        ));

        let mut reagents = Reagents::open(path.clone());
        let doc = reagents.doc_mut().expect("typed block 20 opens");
        assert_eq!(doc.baseline_len(), bytes.len());
        assert_eq!(doc.storage().entries[0].count, 20);
        doc.storage_mut().entries.push(ReagentEntry {
            record: "records/items/questitems/scrapmetal.dbr".into(),
            count: 80,
        });
        doc.mark_edited();
        let outcome = doc.save().unwrap();
        assert!(
            matches!(outcome, SaveOutcome::Saved { backup: Some(_) }),
            "{outcome:?}"
        );
        assert_eq!(backups_of(&path), 1);
        assert_eq!(doc.edits(), Edits::Saved);
        let written = GstFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written.reagent_storage().unwrap().entries.len(), 2);

        std::fs::write(&path, &bytes).unwrap();
        reagents.reload().unwrap();
        assert_eq!(reagents.doc().unwrap().storage().entries.len(), 1);
    }

    #[test]
    fn absent_reagents_are_normal_and_appear_on_reload() {
        let scratch = Scratch::new("reagents-absent");
        let path = scratch.0.join("reagents.gst");
        let mut reagents = Reagents::open(path.clone());
        assert!(matches!(reagents, Reagents::Absent { .. }));
        assert!(reagents.doc().is_none());
        assert_eq!(reagents.path(), path);
        reagents.reload().unwrap();
        assert!(matches!(reagents, Reagents::Absent { .. }));

        std::fs::write(&path, reagents_bytes()).unwrap();
        reagents.reload().unwrap();
        assert!(reagents.doc().is_some());

        std::fs::write(&path, b"\x0b\x00\x00\x00begin_block").unwrap();
        let mut failed = Reagents::open(path.clone());
        assert!(matches!(failed, Reagents::Failed { .. }));
        assert_eq!(failed.stamp(), stamp_of(&path));
        assert_eq!(failed.edits(), Edits::Saved);
        failed.reload().unwrap();
        assert!(matches!(failed, Reagents::Failed { .. }));
        std::fs::remove_file(&path).unwrap();
        failed.reload().unwrap();
        assert!(matches!(failed, Reagents::Absent { .. }));
    }

    #[test]
    fn character_labels_fall_back_to_the_folder_name() {
        let entry = CharacterEntry::Failed {
            path: PathBuf::from("/saves/main/_Sif/player.gdc"),
            error: CharacterOpenError::Read(io::Error::other("nope")),
        };
        assert_eq!(entry.label(), "Sif");
    }
}
