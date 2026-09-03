//! Grim Dawn file formats, item model, and vault logic. GUI-agnostic:
//! no egui, eframe, winit, rmcp, or tokio anywhere in the dependency
//! tree (ARCHITECTURE.md "Crate layering").
//!
//! Save codec: [`crypto`] is the rolling-XOR cipher and block framing,
//! [`item`] the item record, [`block`] the shared block walker, opaque
//! blocks and stash tabs, [`gdc`] `player.gdc`, and [`gst`] the `.gst`
//! family. Every parser is lossless: an unmodified model re-encodes to
//! the loaded bytes exactly.
//!
//! [`gamedata`] is the layered record database, localization, and
//! item bitmaps, resolved into names, rarity, and footprints.

pub mod block;
pub mod crypto;
pub mod gamedata;
pub mod gdc;
pub mod gst;
pub mod item;
