// Vendored from tq-univault crates/univault-gui/src/components/mod.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Self-contained visual components, each painted from a texture the
//! app uploads and a slice layout the app describes, and previewable
//! in isolation over placeholder art:
//! `cargo run -p univault-ui --features dev --bin preview -- <name>`.
//! Call sites hand a component a `Ui` or a `Painter` plus a rect; the
//! component owns nothing but its handle and geometry.

use egui::ColorImage;

mod chevron;
pub mod gilded_border;
pub mod scroll_strip;
pub mod tabbed_panel;

/// The bytes were not a decodable PNG.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct PngError(#[from] image::ImageError);

/// Decodes a PNG into an image ready for `Context::load_texture`.
///
/// # Errors
///
/// [`PngError`] when the bytes do not decode as a PNG.
pub fn load_png(bytes: &[u8]) -> Result<ColorImage, PngError> {
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.into_rgba8();
    #[allow(
        clippy::cast_possible_truncation,
        reason = "u32 image dimensions fit usize on every target the kit builds for"
    )]
    let size = [decoded.width() as usize, decoded.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, decoded.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    #[test]
    fn a_png_round_trips_into_a_color_image() {
        let mut encoded = Vec::new();
        image::RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 128])
            }
        })
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .unwrap();
        let decoded = load_png(&encoded).unwrap();
        assert_eq!(decoded.size, [2, 1]);
        assert_eq!(decoded.pixels[0], Color32::RED);
        assert_eq!(
            decoded.pixels[1],
            Color32::from_rgba_unmultiplied(0, 0, 255, 128)
        );
    }

    #[test]
    fn junk_bytes_are_an_error_not_a_panic() {
        assert!(load_png(b"not a png").is_err());
    }
}
