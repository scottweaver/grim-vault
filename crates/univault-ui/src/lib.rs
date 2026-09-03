//! Art-free egui UI kit shared by tq-univault and grim-vault. Bundles
//! no textures or fonts: every component and the theme take their art
//! from the app (`docs/engine-extraction.md`, decision 4).
//!
//! An app decodes its PNGs with [`components::load_png`], uploads
//! them once with `Context::load_texture`, describes each image's
//! pixel layout as a [`slice::ThreeSlice`] or [`slice::NinePatch`],
//! and hands texture and layout to the component. Fonts arrive as
//! [`theme::FontFace`]s inside a [`theme::Theme`].

pub mod chrome;
pub mod components;
#[cfg(feature = "dev")]
pub mod review;
pub mod slice;
pub mod sort;
pub mod theme;
