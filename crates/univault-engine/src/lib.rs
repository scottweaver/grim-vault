//! Game-agnostic engine formats shared by tq-univault and grim-vault.
//!
//! Lives inside the grim-vault workspace for now; the crate boundary is
//! what makes a later move to its own repository a move rather than a
//! rewrite (see `docs/engine-extraction.md`). This crate is pure: it
//! takes bytes and returns bytes or typed models, and never touches the
//! filesystem outside tests.

pub mod arc;
pub mod arz;
pub mod codec;
pub mod format;
pub mod grid;
pub mod ids;
pub mod platform;
pub mod reader;
pub mod tex;
pub mod text;
pub mod writer;
