//! Shell-side file IO shared by tq-univault and grim-vault. Separate
//! from `univault-engine` on purpose: the engine and the game cores
//! are IO-free (they take and return bytes); this crate is where the
//! filesystem is touched, with the network-mount lessons baked in
//! (uncached verified reads, backup-first synced writes).

#![deny(missing_docs)]

pub mod safe_io;

pub use safe_io::{BackupPolicy, backup_first_write, read_verified, write_synced, write_uncached};
