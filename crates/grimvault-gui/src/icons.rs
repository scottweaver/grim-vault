//! Item icons as GPU textures, decoded once per bitmap path from the
//! player's own `Items.arc` archives. An icon the archives lack or a
//! format the decoder does not cover stays a recorded absence, so the
//! panes paint their fallback tile without retrying every frame.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle, TextureOptions};
use grimvault_core::gamedata::{BitmapPath, GameData};
use univault_engine::tex::{self, TexError};

/// Why an icon could not be shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconProblem {
    /// The record names no bitmap.
    NoBitmap,
    /// No item archive has the entry.
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
            Self::NotInArchives => f.write_str("no item archive has the bitmap"),
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

/// The upload memo, keyed by bitmap path.
#[derive(Default)]
pub struct IconCache {
    icons: HashMap<BitmapPath, Icon>,
}

impl IconCache {
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
}

fn decode(ctx: &egui::Context, game: &GameData, bitmap: &BitmapPath) -> Icon {
    let bytes = match game.bitmap(bitmap) {
        None => return Icon::Missing(IconProblem::NotInArchives),
        Some(Err(error)) => return Icon::Missing(IconProblem::Archive(error.to_string())),
        Some(Ok(bytes)) => bytes,
    };
    match tex::decode(&bytes) {
        Ok(image) => {
            let color_image =
                ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.pixels);
            Icon::Texture(ctx.load_texture(bitmap.as_str(), color_image, TextureOptions::LINEAR))
        }
        Err(error) => Icon::Missing(IconProblem::Undecodable(error)),
    }
}
