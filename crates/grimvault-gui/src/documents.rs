//! The open files as the shell holds them: the transfer stash, the
//! component / crafting-material storage, the illusion collection,
//! the vault store, and the characters (the blueprint list, being the
//! one plaintext file, is [`crate::crafting::FormulasDoc`]). Each
//! writable document knows the disk stamp it was read under (the
//! external-change guard's baseline), whether it holds unsaved edits,
//! and whether this load's backup has been taken yet — the
//! backup-first rule is *one backup per load*, so the first write
//! since load takes it and later writes reuse it (ARCHITECTURE.md
//! "Data flow"). That bookkeeping is one type, [`Tracking`], shared by
//! every document; the files the game creates lazily share
//! [`Optional`].
//!
//! A character is writable only while every block of its `player.gdc`
//! is typed: an opaque block cannot be re-keyed, so an edit before it
//! could never be written (ARCHITECTURE.md "Data flow"), and such a
//! file is held read-only rather than refused.

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use grimvault_core::block::SaveEncodeError;
use grimvault_core::crypto::BlockId;
use grimvault_core::gdc::{GdcError, PlayerFile, Realm};
use grimvault_core::gst::{GstError, GstFile, Illusions, ReagentStorage, TransferStash};
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
    /// The character's file carries an opaque block, so no edit to it
    /// can be re-keyed.
    #[error("{} is read-only: block {block} is not typed", path.display())]
    ReadOnly { path: PathBuf, block: BlockId },
}

/// Position of a character in the shell's list: `main/` in folder
/// order, then `user/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CharacterSlot(usize);

impl CharacterSlot {
    #[must_use]
    pub const fn new(slot: usize) -> Self {
        Self(slot)
    }

    #[must_use]
    pub const fn value(self) -> usize {
        self.0
    }
}

/// The writable documents, for labels, the write order, and the
/// conflict list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Doc {
    Stash,
    Store,
    Reagents,
    Blueprints,
    Illusions,
    Character(CharacterSlot),
}

impl fmt::Display for Doc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stash => f.write_str("transfer stash"),
            Self::Store => f.write_str("vault store"),
            Self::Reagents => f.write_str("component storage"),
            Self::Blueprints => f.write_str("blueprint list"),
            Self::Illusions => f.write_str("illusion collection"),
            Self::Character(slot) => write!(f, "character {}", slot.value() + 1),
        }
    }
}

/// The on-disk bookkeeping every writable document shares: where it
/// lives, the stamp the guard compares against, whether it holds
/// unsaved edits, and whether this load's backup has been taken.
#[derive(Debug)]
pub struct Tracking {
    path: PathBuf,
    stamp: Option<FileStamp>,
    edits: Edits,
    backup: Backup,
}

impl Tracking {
    /// A freshly read file: stamped now, clean, backup armed.
    pub(crate) fn fresh(path: PathBuf) -> Self {
        let stamp = stamp_of(&path);
        Self {
            path,
            stamp,
            edits: Edits::Saved,
            backup: Backup::Armed,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn stamp(&self) -> Option<FileStamp> {
        self.stamp
    }

    #[must_use]
    pub fn edits(&self) -> Edits {
        self.edits
    }

    #[must_use]
    pub fn backup(&self) -> Backup {
        self.backup
    }

    pub fn mark_edited(&mut self) {
        self.edits = Edits::Unsaved;
    }

    /// Writes `bytes` unless the file changed underneath: backup-first
    /// on the first write since load, a plain synced write after; the
    /// backup state advances only once the write succeeded, and the
    /// edits stay unsaved on failure.
    pub(crate) fn save_bytes(&mut self, bytes: &[u8]) -> Result<SaveOutcome, SaveError> {
        if stamp_of(&self.path) != self.stamp {
            return Ok(SaveOutcome::Conflict);
        }
        let written = match self.backup {
            Backup::Armed => backup_first_write(&self.path, bytes, BACKUPS),
            Backup::Taken => write_synced(&self.path, bytes).map(|()| None),
        };
        let backup = written.map_err(|source| SaveError::Write {
            path: self.path.clone(),
            source,
        })?;
        self.backup = Backup::Taken;
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

impl EditableBlock for Illusions {
    const ID: BlockId = BlockId::ILLUSIONS;

    fn of(file: &GstFile) -> Option<&Self> {
        file.illusions()
    }

    fn of_mut(file: &mut GstFile) -> Option<&mut Self> {
        file.illusions_mut()
    }
}

/// A `.gst` editable through its typed block `B`, because it passed
/// the lossless gate and carries that block.
#[derive(Debug)]
pub struct GstDoc<B: EditableBlock> {
    tracking: Tracking,
    loaded: Loaded<GstFile>,
    block: PhantomData<B>,
}

/// The shared stash.
pub type StashDoc = GstDoc<TransferStash>;
/// The component / crafting-material storage.
pub type ReagentDoc = GstDoc<ReagentStorage>;
/// The illusion collection, `transmutes.gst`.
pub type IllusionsDoc = GstDoc<Illusions>;

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
        let tracking = Tracking::fresh(path.clone());
        let loaded = Loaded::<GstFile>::load(bytes).map_err(|source| GstOpenError::Load {
            path: path.clone(),
            source,
        })?;
        if B::of(loaded.model()).is_none() {
            return Err(GstOpenError::NoTypedBlock { path, block: B::ID });
        }
        Ok(Self {
            tracking,
            loaded,
            block: PhantomData,
        })
    }

    #[must_use]
    pub fn tracking(&self) -> &Tracking {
        &self.tracking
    }

    pub fn tracking_mut(&mut self) -> &mut Tracking {
        &mut self.tracking
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.tracking.path()
    }

    /// The typed block. `open` proved it present and `GstFile` cannot
    /// drop a block, so the lookup cannot fail.
    #[must_use]
    pub fn block(&self) -> &B {
        B::of(self.loaded.model())
            .expect("open refused a file without the block and blocks cannot be removed")
    }

    /// The typed block for editing; call [`Tracking::mark_edited`]
    /// after.
    pub fn block_mut(&mut self) -> &mut B {
        B::of_mut(self.loaded.model_mut())
            .expect("open refused a file without the block and blocks cannot be removed")
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
        let bytes = self.loaded.encode()?;
        self.tracking.save_bytes(&bytes)
    }

    /// Re-reads the file, dropping in-memory edits. On failure the
    /// document is untouched, edits included.
    ///
    /// # Errors
    /// [`GstOpenError`].
    pub fn reload(&mut self) -> Result<(), GstOpenError> {
        *self = Self::open(self.tracking.path.clone())?;
        Ok(())
    }
}

impl StashDoc {
    /// Block 18.
    #[must_use]
    pub fn stash(&self) -> &TransferStash {
        self.block()
    }

    /// Block 18 for editing; call [`Tracking::mark_edited`] after.
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

    /// Block 20 for editing; call [`Tracking::mark_edited`] after.
    pub fn storage_mut(&mut self) -> &mut ReagentStorage {
        self.block_mut()
    }
}

impl IllusionsDoc {
    /// Block 19.
    #[must_use]
    pub fn illusions(&self) -> &Illusions {
        self.block()
    }

    /// Block 19 for editing; call [`Tracking::mark_edited`] after.
    pub fn illusions_mut(&mut self) -> &mut Illusions {
        self.block_mut()
    }
}

/// A file document as [`Optional`] drives it: opened from a path,
/// tracked, saved, and re-read.
pub trait Document: fmt::Debug + Sized {
    type OpenError: std::error::Error;

    /// Reads, gates, and stamps the file.
    ///
    /// # Errors
    /// The document's own open error.
    fn open(path: PathBuf) -> Result<Self, Self::OpenError>;
    fn tracking(&self) -> &Tracking;
    fn tracking_mut(&mut self) -> &mut Tracking;
    /// Writes the model unless the file changed underneath.
    ///
    /// # Errors
    /// [`SaveError`]; the edits stay unsaved.
    fn save(&mut self) -> Result<SaveOutcome, SaveError>;
    /// Re-reads the file, dropping in-memory edits; on failure the
    /// document is untouched, edits included.
    ///
    /// # Errors
    /// The document's own open error.
    fn reload(&mut self) -> Result<(), Self::OpenError>;
}

impl<B: EditableBlock> Document for GstDoc<B> {
    type OpenError = GstOpenError;

    fn open(path: PathBuf) -> Result<Self, GstOpenError> {
        GstDoc::open(path)
    }

    fn tracking(&self) -> &Tracking {
        &self.tracking
    }

    fn tracking_mut(&mut self) -> &mut Tracking {
        &mut self.tracking
    }

    fn save(&mut self) -> Result<SaveOutcome, SaveError> {
        GstDoc::save(self)
    }

    fn reload(&mut self) -> Result<(), GstOpenError> {
        GstDoc::reload(self)
    }
}

/// A shared file the game writes only once it has something to put in
/// it — `reagents.gst`, `formulas.gst`, `transmutes.gst` — however it
/// fared: a save directory without it is normal, and a file this build
/// cannot type is shown, not fatal.
#[derive(Debug)]
pub enum Optional<D: Document> {
    Open(D),
    Absent {
        path: PathBuf,
    },
    /// Present but not editable; `stamp` is the file as it was found,
    /// so the guard only reacts when it changes again.
    Failed {
        path: PathBuf,
        stamp: Option<FileStamp>,
        error: D::OpenError,
    },
}

/// The component / crafting-material storage, however it fared.
pub type Reagents = Optional<ReagentDoc>;

impl<D: Document> Optional<D> {
    /// Opens `path` when it exists.
    #[must_use]
    pub fn open(path: PathBuf) -> Self {
        if !path.is_file() {
            return Self::Absent { path };
        }
        let stamp = stamp_of(&path);
        match D::open(path.clone()) {
            Ok(doc) => Self::Open(doc),
            Err(error) => Self::Failed { path, stamp, error },
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Open(doc) => doc.tracking().path(),
            Self::Absent { path } | Self::Failed { path, .. } => path,
        }
    }

    /// The stamp the guard compares against: the document's, or the
    /// file's as it was found unusable; `None` while absent.
    #[must_use]
    pub fn stamp(&self) -> Option<FileStamp> {
        match self {
            Self::Open(doc) => doc.tracking().stamp(),
            Self::Absent { .. } => None,
            Self::Failed { stamp, .. } => *stamp,
        }
    }

    /// The document's edits; an absent or unusable file has none.
    #[must_use]
    pub fn edits(&self) -> Edits {
        self.doc()
            .map_or(Edits::Saved, |doc| doc.tracking().edits())
    }

    #[must_use]
    pub fn doc(&self) -> Option<&D> {
        match self {
            Self::Open(doc) => Some(doc),
            Self::Absent { .. } | Self::Failed { .. } => None,
        }
    }

    pub fn doc_mut(&mut self) -> Option<&mut D> {
        match self {
            Self::Open(doc) => Some(doc),
            Self::Absent { .. } | Self::Failed { .. } => None,
        }
    }

    /// Marks the open document edited; nothing to mark otherwise.
    pub fn mark_edited(&mut self) {
        if let Some(doc) = self.doc_mut() {
            doc.tracking_mut().mark_edited();
        }
    }

    /// See [`Tracking::keep_mine`]; nothing to keep otherwise.
    pub fn keep_mine(&mut self) {
        if let Some(doc) = self.doc_mut() {
            doc.tracking_mut().keep_mine();
        }
    }

    /// Writes the open document; an absent or unusable file has
    /// nothing to write and never has edits to flush.
    ///
    /// # Errors
    /// [`SaveError`].
    pub fn save(&mut self) -> Result<SaveOutcome, SaveError> {
        match self.doc_mut() {
            Some(doc) => doc.save(),
            None => Ok(SaveOutcome::Saved { backup: None }),
        }
    }

    /// Re-reads the file: an open document reloads in place (keeping
    /// its edits on failure), an absent or failed one is opened afresh
    /// — whatever the disk now holds becomes the state, a file the
    /// game has since written included.
    ///
    /// # Errors
    /// The document's open error when an open document could not be
    /// re-read.
    pub fn reload(&mut self) -> Result<(), D::OpenError> {
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
    tracking: Tracking,
    store: VaultStore,
    /// Counts every chance the contents had to change — each mutable
    /// borrow and each reload — so a view caching a derived shape of
    /// the store knows when to rebuild without diffing the items.
    revision: u64,
}

impl StoreDoc {
    /// Reads the store, or starts an empty one when the file is absent.
    ///
    /// # Errors
    /// [`StoreOpenError`] when the file exists but cannot be read or
    /// parsed.
    pub fn open(path: PathBuf) -> Result<Self, StoreOpenError> {
        let store = if path.is_file() {
            let bytes = read_verified(&path).map_err(|source| StoreOpenError::Read {
                path: path.clone(),
                source,
            })?;
            VaultStore::from_json(&bytes).map_err(|source| StoreOpenError::Parse {
                path: path.clone(),
                source,
            })?
        } else {
            VaultStore::new()
        };
        Ok(Self {
            tracking: Tracking::fresh(path),
            store,
            revision: 0,
        })
    }

    /// Changes whenever the store may have changed.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn tracking(&self) -> &Tracking {
        &self.tracking
    }

    pub fn tracking_mut(&mut self) -> &mut Tracking {
        &mut self.tracking
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.tracking.path()
    }

    #[must_use]
    pub fn store(&self) -> &VaultStore {
        &self.store
    }

    /// The store for editing; call [`Tracking::mark_edited`] after.
    pub fn store_mut(&mut self) -> &mut VaultStore {
        self.revision += 1;
        &mut self.store
    }

    /// Writes the store unless the file changed underneath, creating
    /// the config directory on the first save.
    ///
    /// # Errors
    /// [`SaveError::Write`]; the edits stay unsaved.
    pub fn save(&mut self) -> Result<SaveOutcome, SaveError> {
        let path = self.tracking.path();
        path.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .map_err(|source| SaveError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        self.tracking.save_bytes(&self.store.to_json())
    }

    /// See [`GstDoc::reload`].
    ///
    /// # Errors
    /// [`StoreOpenError`].
    pub fn reload(&mut self) -> Result<(), StoreOpenError> {
        let revision = self.revision + 1;
        *self = Self::open(self.tracking.path.clone())?;
        self.revision = revision;
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

/// Whether a character's file can be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writable {
    /// Every block is typed: any edit re-keys correctly.
    Yes,
    /// The first opaque block; nothing before it can be edited, so the
    /// whole file is held read-only.
    OpaqueBlock(BlockId),
}

/// A character's `player.gdc`, gated lossless and stamped; editable
/// only while [`Writable::Yes`].
#[derive(Debug)]
pub struct CharacterDoc {
    realm: Realm,
    tracking: Tracking,
    loaded: Loaded<PlayerFile>,
}

impl CharacterDoc {
    /// Reads and gates a `player.gdc` found under `realm`'s folder.
    ///
    /// # Errors
    /// [`CharacterOpenError`].
    pub fn open(realm: Realm, path: PathBuf) -> Result<Self, CharacterOpenError> {
        let bytes = read_verified(&path)?;
        let tracking = Tracking::fresh(path);
        let loaded = Loaded::<PlayerFile>::load(bytes)?;
        Ok(Self {
            realm,
            tracking,
            loaded,
        })
    }

    /// The folder the file was read from, which the file itself
    /// never names; the store records it with every item vaulted
    /// from this character.
    #[must_use]
    pub fn realm(&self) -> Realm {
        self.realm
    }

    #[must_use]
    pub fn tracking(&self) -> &Tracking {
        &self.tracking
    }

    pub fn tracking_mut(&mut self) -> &mut Tracking {
        &mut self.tracking
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.tracking.path()
    }

    #[must_use]
    pub fn file(&self) -> &PlayerFile {
        self.loaded.model()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        self.file().character_name()
    }

    #[must_use]
    pub fn writable(&self) -> Writable {
        self.file()
            .blocks()
            .iter()
            .find(|block| block.is_opaque())
            .map_or(Writable::Yes, |block| Writable::OpaqueBlock(block.id()))
    }

    /// The file for editing, refused while read-only; call
    /// [`Tracking::mark_edited`] after.
    ///
    /// # Errors
    /// [`SaveError::ReadOnly`] naming the opaque block.
    pub fn file_mut(&mut self) -> Result<&mut PlayerFile, SaveError> {
        match self.writable() {
            Writable::Yes => Ok(self.loaded.model_mut()),
            Writable::OpaqueBlock(block) => Err(SaveError::ReadOnly {
                path: self.tracking.path.clone(),
                block,
            }),
        }
    }

    /// Size of the bytes the model was proven against.
    #[must_use]
    pub fn baseline_len(&self) -> usize {
        self.loaded.baseline().len()
    }

    /// Writes the character unless the file changed underneath.
    ///
    /// # Errors
    /// [`SaveError::ReadOnly`] for a file with an opaque block, or a
    /// failed encode or write; the edits stay unsaved.
    pub fn save(&mut self) -> Result<SaveOutcome, SaveError> {
        if let Writable::OpaqueBlock(block) = self.writable() {
            return Err(SaveError::ReadOnly {
                path: self.tracking.path.clone(),
                block,
            });
        }
        let bytes = self.loaded.encode()?;
        self.tracking.save_bytes(&bytes)
    }

    /// See [`GstDoc::reload`].
    ///
    /// # Errors
    /// [`CharacterOpenError`].
    pub fn reload(&mut self) -> Result<(), CharacterOpenError> {
        *self = Self::open(self.realm, self.tracking.path.clone())?;
        Ok(())
    }
}

/// One `main/*/player.gdc` or `user/*/player.gdc`, however it fared.
#[derive(Debug)]
pub enum CharacterEntry {
    Loaded(CharacterDoc),
    /// Present but unreadable; `stamp` is the file as it was found, so
    /// the guard only reacts when it changes again.
    Failed {
        realm: Realm,
        path: PathBuf,
        stamp: Option<FileStamp>,
        error: CharacterOpenError,
    },
}

impl CharacterEntry {
    fn open(realm: Realm, path: PathBuf) -> Self {
        let stamp = stamp_of(&path);
        match CharacterDoc::open(realm, path.clone()) {
            Ok(doc) => Self::Loaded(doc),
            Err(error) => Self::Failed {
                realm,
                path,
                stamp,
                error,
            },
        }
    }

    #[must_use]
    pub fn realm(&self) -> Realm {
        match self {
            Self::Loaded(doc) => doc.realm(),
            Self::Failed { realm, .. } => *realm,
        }
    }

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

    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Loaded(doc) => doc.path(),
            Self::Failed { path, .. } => path,
        }
    }

    #[must_use]
    pub fn stamp(&self) -> Option<FileStamp> {
        match self {
            Self::Loaded(doc) => doc.tracking().stamp(),
            Self::Failed { stamp, .. } => *stamp,
        }
    }

    /// The document's edits; an unreadable file has none.
    #[must_use]
    pub fn edits(&self) -> Edits {
        self.doc()
            .map_or(Edits::Saved, |doc| doc.tracking().edits())
    }

    #[must_use]
    pub fn doc(&self) -> Option<&CharacterDoc> {
        match self {
            Self::Loaded(doc) => Some(doc),
            Self::Failed { .. } => None,
        }
    }

    pub fn doc_mut(&mut self) -> Option<&mut CharacterDoc> {
        match self {
            Self::Loaded(doc) => Some(doc),
            Self::Failed { .. } => None,
        }
    }

    /// See [`Reagents::reload`].
    ///
    /// # Errors
    /// [`CharacterOpenError`] when a loaded character could not be
    /// re-read.
    pub fn reload(&mut self) -> Result<(), CharacterOpenError> {
        match self {
            Self::Loaded(doc) => doc.reload(),
            Self::Failed { realm, path, .. } => {
                *self = Self::open(*realm, path.clone());
                Ok(())
            }
        }
    }
}

/// Every character under `main/` then `user/`, each in folder order,
/// failures included; a realm whose folder is absent contributes
/// nothing.
#[must_use]
pub fn open_characters(save_dir: &SaveDir) -> Vec<CharacterEntry> {
    Realm::ALL
        .into_iter()
        .flat_map(|realm| {
            character_files(&save_dir.characters_dir(realm))
                .into_iter()
                .map(move |path| CharacterEntry::open(realm, path))
        })
        .collect()
}

/// The `player.gdc` of every character folder under `dir`, in folder
/// order.
fn character_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
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
        .collect()
}

#[cfg(test)]
mod tests {
    use grimvault_core::crypto::{EncodeError, Encoder};
    use grimvault_core::gst::ReagentEntry;

    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../grimvault-core/tests/fixtures/v11_player.gdc");

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
        assert_eq!(doc.tracking().stamp(), None);
        assert_eq!(doc.tracking().backup(), Backup::Armed);

        doc.tracking_mut().mark_edited();
        assert_eq!(doc.tracking().edits(), Edits::Unsaved);
        assert!(matches!(
            doc.save().unwrap(),
            SaveOutcome::Saved { backup: None }
        ));
        assert_eq!(doc.tracking().edits(), Edits::Saved);
        assert_eq!(doc.tracking().backup(), Backup::Taken);
        assert_eq!(doc.tracking().stamp(), stamp_of(&path));
        assert_eq!(backups_of(&path), 0);

        doc.tracking_mut().mark_edited();
        assert!(matches!(
            doc.save().unwrap(),
            SaveOutcome::Saved { backup: None }
        ));
        assert_eq!(backups_of(&path), 0);

        let mut reopened = StoreDoc::open(path.clone()).unwrap();
        reopened.tracking_mut().mark_edited();
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
        doc.tracking_mut().mark_edited();
        doc.save().unwrap();

        std::fs::write(&path, VaultStore::new().to_json()).unwrap();
        let external = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(external + std::time::Duration::from_secs(5))
            .unwrap();
        doc.tracking_mut().mark_edited();
        assert_eq!(doc.save().unwrap(), SaveOutcome::Conflict);
        assert_eq!(doc.tracking().edits(), Edits::Unsaved);

        doc.tracking_mut().keep_mine();
        assert_eq!(doc.tracking().backup(), Backup::Armed);
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
        doc.tracking_mut().mark_edited();
        doc.save().unwrap();
        doc.tracking_mut().mark_edited();
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(doc.reload().is_err());
        assert_eq!(doc.tracking().edits(), Edits::Unsaved);
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
        doc.tracking_mut().mark_edited();
        let outcome = doc.save().unwrap();
        assert!(
            matches!(outcome, SaveOutcome::Saved { backup: Some(_) }),
            "{outcome:?}"
        );
        assert_eq!(backups_of(&path), 1);
        assert_eq!(doc.tracking().edits(), Edits::Saved);
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
    fn a_character_edits_saves_backup_first_and_re_reads_typed() {
        let scratch = Scratch::new("character");
        let folder = scratch.0.join("main").join("_Laurana");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("player.gdc");
        std::fs::write(&path, FIXTURE).unwrap();

        let mut doc = CharacterDoc::open(Realm::Main, path.clone()).unwrap();
        assert_eq!(doc.name(), "Laurana");
        assert_eq!(doc.realm(), Realm::Main);
        assert_eq!(doc.writable(), Writable::Yes);
        assert_eq!(doc.baseline_len(), FIXTURE.len());
        assert_eq!(doc.tracking().backup(), Backup::Armed);

        let before = doc.file().character_info().unwrap().money;
        doc.file_mut().unwrap().character_info_mut().unwrap().money = before + 1;
        doc.tracking_mut().mark_edited();
        let outcome = doc.save().unwrap();
        assert!(
            matches!(outcome, SaveOutcome::Saved { backup: Some(_) }),
            "{outcome:?}"
        );
        assert_eq!(backups_of(&path), 1);
        assert_eq!(doc.tracking().edits(), Edits::Saved);

        let written = PlayerFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert!(written.is_fully_typed());
        assert_eq!(written.character_info().unwrap().money, before + 1);
        assert_eq!(written.blocks().len(), doc.file().blocks().len());

        std::fs::write(&path, FIXTURE).unwrap();
        doc.reload().unwrap();
        assert_eq!(doc.file().character_info().unwrap().money, before);
    }

    #[test]
    fn a_character_with_an_opaque_block_is_read_only() {
        let scratch = Scratch::new("character-opaque");
        let path = scratch.0.join("player.gdc");
        let mut enc = Encoder::new(0x0BAD_F00D);
        enc.write_u32(u32::from_le_bytes(*b"GDCX"));
        enc.write_u32(2);
        enc.write_wstring("Sif").unwrap();
        enc.write_bool(false);
        enc.write_string("").unwrap();
        enc.write_u32(1);
        enc.write_bool(false);
        enc.write_u8(7);
        enc.write_zero_marker();
        enc.write_u32(8);
        enc.write_bytes(&[0; 16]);
        enc.write_block(BlockId::new(99), |enc| {
            enc.write_u32(1);
            enc.write_string("records/x")?;
            Ok::<(), EncodeError>(())
        })
        .unwrap();
        std::fs::write(&path, enc.finish()).unwrap();

        let mut doc = CharacterDoc::open(Realm::Custom, path).unwrap();
        assert_eq!(doc.name(), "Sif");
        assert_eq!(doc.writable(), Writable::OpaqueBlock(BlockId::new(99)));
        assert!(matches!(
            doc.file_mut(),
            Err(SaveError::ReadOnly { block, .. }) if block == BlockId::new(99)
        ));
        assert!(matches!(doc.save(), Err(SaveError::ReadOnly { .. })));
        assert_eq!(backups_of(doc.path()), 0);
    }

    #[test]
    fn character_labels_fall_back_to_the_folder_name() {
        let entry = CharacterEntry::Failed {
            realm: Realm::Custom,
            path: PathBuf::from("/saves/user/_Sif/player.gdc"),
            stamp: None,
            error: CharacterOpenError::Read(io::Error::other("nope")),
        };
        assert_eq!(entry.label(), "Sif");
        assert_eq!(entry.realm(), Realm::Custom);
        assert_eq!(entry.edits(), Edits::Saved);
        assert!(entry.doc().is_none());
    }

    #[test]
    fn characters_are_listed_main_campaign_first_then_custom_games() {
        let scratch = Scratch::new("realms");
        std::fs::write(scratch.0.join("transfer.gst"), b"").unwrap();
        for folder in ["user/_Zark", "main/_Sif", "main/_Aria"] {
            let folder = scratch.0.join(folder);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("player.gdc"), FIXTURE).unwrap();
        }
        std::fs::create_dir_all(scratch.0.join("main/_NoFile")).unwrap();

        let listed: Vec<(Realm, PathBuf)> = open_characters(&SaveDir::parse(&scratch.0).unwrap())
            .iter()
            .map(|entry| {
                assert!(entry.doc().is_some(), "{}", entry.path().display());
                (entry.realm(), entry.path().to_path_buf())
            })
            .collect();
        assert_eq!(
            listed,
            vec![
                (Realm::Main, scratch.0.join("main/_Aria/player.gdc")),
                (Realm::Main, scratch.0.join("main/_Sif/player.gdc")),
                (Realm::Custom, scratch.0.join("user/_Zark/player.gdc")),
            ]
        );
    }
}
