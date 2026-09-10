//! Where a Grim Dawn install and its saves are, and how to read them:
//! the shell-side IO every Grim Vault shell shares. The desktop shell,
//! the MCP server, and the command-line examples all open the same
//! directories, list the same mods, read the same archive layers, and
//! find the same character files, so that code lives here once, on
//! top of `univault-io`'s verified reads and beneath every shell
//! (ARCHITECTURE.md "Crate layering"). All format knowledge stays in
//! `grimvault-core`: this crate decides which files to read and reads
//! them, and hands the bytes to the core's parsers.
//!
//! [`dirs`] validates the two directories and names the files under
//! them, [`settings`] reads and writes `settings.json`, [`layers`]
//! reads the record databases and resource archives of every layer —
//! shipped and mod — in parallel, and [`characters`] finds the
//! `player.gdc` files.

#![deny(missing_docs)]

pub mod characters;
pub mod dirs;
pub mod layers;
pub mod settings;

pub use dirs::{DirProblem, DirRole, GameDir, SaveDir};
pub use layers::{ArchiveFailure, ArchiveRead, LayerFile, list_mods, read_layers};
