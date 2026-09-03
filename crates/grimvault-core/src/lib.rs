//! Grim Dawn file formats, item model, and vault logic. GUI-agnostic:
//! no egui, eframe, winit, rmcp, or tokio anywhere in the dependency
//! tree, and no filesystem access (ARCHITECTURE.md "Crate layering").
//!
//! Save codec: [`crypto`] is the rolling-XOR cipher and block framing,
//! [`item`] the item record, [`block`] the shared block walker, opaque
//! blocks and stash tabs, [`gdc`] `player.gdc`, and [`gst`] the `.gst`
//! family. Every parser is lossless: an unmodified model re-encodes to
//! the loaded bytes exactly, and [`loaded`] makes that a precondition
//! of editing.
//!
//! [`gamedata`] is the layered record database, localization, and
//! item bitmaps, resolved into names, rarity, and footprints.
//!
//! Vault: [`store`] is this app's own item store, [`bucket`] the
//! computed type view over it, [`reagents`] the rule for what belongs
//! in the game's component / crafting-material storage, and
//! [`transfer`] the pure moves between the transfer stash, that
//! storage, and the store.

pub mod block;
pub mod blocks;
pub mod bucket;
pub mod crypto;
pub mod gamedata;
pub mod gdc;
pub mod gst;
pub mod item;
pub mod loaded;
pub mod platform;
pub mod reagents;
pub mod settings;
pub mod store;
pub mod transfer;
