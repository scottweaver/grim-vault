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
//! [`formulas`] is `formulas.gst`, the one shared file that is plain
//! key/value text rather than a rolling-XOR save; [`blueprint`] and
//! [`illusion`] are the rules for what the database admits into it
//! and into `transmutes.gst`, with the add / export / import ops, and
//! [`interchange`] the envelope of this app's own JSON documents.
//!
//! [`gamedata`] is the layered record database, localization, and
//! item bitmaps, resolved into names, rarity, and footprints;
//! [`facets`] derives from them what the game marks on an item's tile
//! (monster infrequent, double rare, ascended or upgradeable),
//! [`search`] is the typed query over those facets and names, and
//! [`stats`] renders a record's variables into the game's own stat
//! lines and assembles an item's tooltip from them.
//!
//! Vault: [`store`] is this app's own item store, [`bucket`] the
//! computed type view over it, [`reagents`] the rule for what belongs
//! in the game's component / crafting-material storage, [`transfer`]
//! the pure moves between the transfer stash, that storage, and the
//! store, and [`gds`] the read-only import of GD Stash's exports.
//! [`respec`] is the full attribute and mastery refund on a
//! character, under the same rules the game reads from its own
//! records.

pub mod block;
pub mod blocks;
pub mod blueprint;
pub mod bucket;
pub mod campaign;
pub mod crypto;
pub mod facets;
pub mod formulas;
pub mod gamedata;
pub mod gdc;
pub mod gds;
pub mod gst;
pub mod illusion;
pub mod interchange;
pub mod item;
pub mod loaded;
pub mod platform;
pub mod reagents;
pub mod respec;
pub mod search;
pub mod settings;
pub mod stats;
pub mod store;
pub mod transfer;
