//! The app's settings file: the document model lives in
//! `grimvault_core::settings`, the filesystem side in
//! `grimvault_io::settings`; this module names both for the shell.

pub use grimvault_core::settings::{ConfigDir, Settings};
pub use grimvault_io::settings::{load, save};
