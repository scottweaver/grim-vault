//! Item icons as GPU textures, decoded once per bitmap path from the
//! player's own `Items.arc` archives, and the tile symbols beside them
//! ([`crate::badges`]). An icon the archives lack or a format the
//! decoder does not cover stays a recorded absence, so the panes paint
//! their fallback tile without retrying every frame.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle, TextureOptions};
use grimvault_core::facets::Symbol;
use grimvault_core::gamedata::{BitmapPath, GameData};
use univault_engine::tex::{self, TexError};

use crate::badges::{SymbolCache, SymbolTextures};

/// Why an icon could not be shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconProblem {
    /// The record names no bitmap.
    NoBitmap,
    /// No archive has the entry.
    NotInArchives,
    /// The archive entry could not be read.
    Archive(String),
    /// The texture is a format [`tex::decode`] does not cover.
    Undecodable(TexError),
}

impl std::fmt::Display for IconProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBitmap => f.write_str("the record names no bitmap"),
            Self::NotInArchives => f.write_str("no archive has the bitmap"),
            Self::Archive(error) => write!(f, "archive: {error}"),
            Self::Undecodable(error) => write!(f, "texture: {error}"),
        }
    }
}

/// One bitmap's fate.
#[derive(Clone)]
pub enum Icon {
    Texture(TextureHandle),
    Missing(IconProblem),
}

/// The upload memo: item icons by bitmap path, tile symbols by symbol.
#[derive(Default)]
pub struct IconCache {
    icons: HashMap<BitmapPath, Icon>,
    symbols: SymbolCache,
}

impl IconCache {
    /// A cache that will upload the given symbol textures on first use.
    #[must_use]
    pub fn with_symbols(symbols: SymbolTextures) -> Self {
        Self {
            icons: HashMap::new(),
            symbols: SymbolCache::new(symbols),
        }
    }

    /// The icon for a record's bitmap, decoding and uploading on first
    /// sight.
    pub fn icon(
        &mut self,
        ctx: &egui::Context,
        game: &GameData,
        bitmap: Option<&BitmapPath>,
    ) -> Icon {
        let Some(bitmap) = bitmap else {
            return Icon::Missing(IconProblem::NoBitmap);
        };
        if let Some(icon) = self.icons.get(bitmap) {
            return icon.clone();
        }
        let icon = decode(ctx, game, bitmap);
        self.icons.insert(bitmap.clone(), icon.clone());
        icon
    }

    /// The tile symbol's texture, uploading on first sight.
    pub fn symbol(&mut self, ctx: &egui::Context, symbol: Symbol) -> Icon {
        self.symbols.icon(ctx, symbol)
    }
}

fn decode(ctx: &egui::Context, game: &GameData, bitmap: &BitmapPath) -> Icon {
    match game.bitmap(bitmap) {
        None => Icon::Missing(IconProblem::NotInArchives),
        Some(Err(error)) => Icon::Missing(IconProblem::Archive(error.to_string())),
        Some(Ok(bytes)) => upload(ctx, bitmap.as_str(), &bytes),
    }
}

/// Decodes a `.tex` and uploads it under `name`.
pub fn upload(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Icon {
    match tex::decode(bytes) {
        Ok(image) => {
            let color_image =
                ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.pixels);
            Icon::Texture(ctx.load_texture(name, color_image, TextureOptions::LINEAR))
        }
        Err(error) => Icon::Missing(IconProblem::Undecodable(error)),
    }
}
