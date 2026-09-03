// Vendored from tq-univault crates/univault-gui/src/theme.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! The kit's look, pinned to the dark theme: a [`Palette`] over egui's
//! dark visuals, squared-off chrome, and whatever faces the app
//! supplies as [`Fonts`]. [`Theme::apply`] installs all of it once at
//! startup; the palette is also the colour source for custom-painted
//! surfaces (grids, item tiles, tooltips). [`Palette::titan_quest`] is
//! the bronze-and-gold set tq-univault ships; grim-vault builds its
//! own.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontFamily, FontId, Stroke, TextStyle};

/// The colour roles the kit paints with. Widget visuals derive from
/// the `surface*`, `accent*`, and `text*` roles; `grid_*` and
/// `tile_*` are for custom-painted grids and item tiles; the rest
/// fill egui's remaining visuals slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub accent: Color32,
    pub accent_dim: Color32,
    pub accent_faint: Color32,
    pub heading: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_weak: Color32,
    pub surface: Color32,
    pub surface_raised: Color32,
    pub surface_deep: Color32,
    pub popup: Color32,
    pub grid_bg: Color32,
    pub grid_line: Color32,
    pub tile_bg: Color32,
    pub tile_edge: Color32,
    pub faint_bg: Color32,
    pub selection_bg: Color32,
    pub selection_stroke: Color32,
    pub hover_bg: Color32,
    pub active_bg: Color32,
    pub warn: Color32,
    pub error: Color32,
}

impl Palette {
    /// Titan Quest AE: bronze and gold over dark leather-and-olive
    /// surfaces.
    #[must_use]
    pub const fn titan_quest() -> Self {
        Self {
            accent: Color32::from_rgb(208, 172, 92),
            accent_dim: Color32::from_rgb(128, 103, 54),
            accent_faint: Color32::from_rgb(94, 77, 44),
            heading: Color32::from_rgb(219, 185, 110),
            text: Color32::from_rgb(225, 210, 177),
            text_strong: Color32::from_rgb(243, 233, 210),
            text_weak: Color32::from_rgb(160, 145, 115),
            surface: Color32::from_rgb(31, 27, 18),
            surface_raised: Color32::from_rgb(48, 40, 25),
            surface_deep: Color32::from_rgb(18, 15, 10),
            popup: Color32::from_rgb(13, 11, 8),
            grid_bg: Color32::from_rgb(33, 36, 24),
            grid_line: Color32::from_rgb(47, 51, 34),
            tile_bg: Color32::from_rgb(43, 47, 31),
            tile_edge: Color32::from_rgb(66, 70, 48),
            faint_bg: Color32::from_rgb(39, 34, 22),
            selection_bg: Color32::from_rgb(110, 88, 42),
            selection_stroke: Color32::from_rgb(232, 200, 120),
            hover_bg: Color32::from_rgb(60, 50, 31),
            active_bg: Color32::from_rgb(72, 60, 37),
            warn: Color32::from_rgb(235, 180, 76),
            error: Color32::from_rgb(230, 105, 85),
        }
    }
}

/// One font file the app supplies, under the name egui will know it
/// by.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFace {
    pub name: String,
    pub data: Arc<FontData>,
}

impl FontFace {
    #[must_use]
    pub fn new(name: impl Into<String>, data: FontData) -> Self {
        Self {
            name: name.into(),
            data: Arc::new(data),
        }
    }

    /// A face compiled into the app (`include_bytes!`).
    #[must_use]
    pub fn from_static(name: impl Into<String>, bytes: &'static [u8]) -> Self {
        Self::new(name, FontData::from_static(bytes))
    }

    /// A face read at runtime.
    #[must_use]
    pub fn from_owned(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self::new(name, FontData::from_owned(bytes))
    }
}

/// The app's faces. `body` faces go ahead of egui's proportional
/// stack, so the first is the body face and egui's own remain
/// fallbacks for glyphs it lacks. `heading` faces lead the family
/// named `heading_family`, followed by the whole body stack, so
/// symbol glyphs a display face lacks (arrows, ⌘) still resolve. With
/// no faces at all the theme still installs over egui's defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Fonts {
    pub heading_family: Arc<str>,
    pub heading: Vec<FontFace>,
    pub body: Vec<FontFace>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            heading_family: Arc::from("univault-heading"),
            heading: Vec::new(),
            body: Vec::new(),
        }
    }
}

/// The whole look: colours and faces.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub palette: Palette,
    pub fonts: Fonts,
}

impl Theme {
    /// Installs the look on the context — fonts, text styles, and
    /// widget visuals — pinned to the dark theme regardless of the OS
    /// preference.
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(egui::Theme::Dark);
        ctx.set_fonts(self.font_definitions());
        let text_styles = text_styles(self.heading_family());
        let visuals = self.visuals();
        ctx.all_styles_mut(|style| {
            style.text_styles.clone_from(&text_styles);
            style.visuals = visuals.clone();
        });
    }

    /// The family headings are set in.
    #[must_use]
    pub fn heading_family(&self) -> FontFamily {
        FontFamily::Name(Arc::clone(&self.fonts.heading_family))
    }

    /// The heading family at an explicit size, for painted text
    /// (nameplates, plate labels).
    #[must_use]
    pub fn heading_font(&self, size: f32) -> FontId {
        FontId::new(size, self.heading_family())
    }

    /// Heading text in the heading face and colour — pane and dialog
    /// titles.
    #[must_use]
    pub fn heading(&self, text: impl Into<String>) -> egui::RichText {
        egui::RichText::new(text.into())
            .text_style(TextStyle::Heading)
            .color(self.palette.heading)
    }

    /// A file path under a pane heading: small, dim monospace, so it
    /// informs without competing with the panes.
    #[must_use]
    pub fn path_text(&self, text: impl Into<String>) -> egui::RichText {
        egui::RichText::new(text.into())
            .monospace()
            .size(10.5)
            .color(self.palette.text_weak)
    }

    /// A section title inside a pane: the heading face at body scale
    /// in the plain accent — a rank below [`Self::heading`].
    #[must_use]
    pub fn section(&self, text: impl Into<String>) -> egui::RichText {
        egui::RichText::new(text.into())
            .font(self.heading_font(15.0))
            .color(self.palette.accent)
    }

    /// egui's default faces with the app's inserted per [`Fonts`].
    #[must_use]
    pub fn font_definitions(&self) -> egui::FontDefinitions {
        let mut fonts = egui::FontDefinitions::default();
        for face in self.fonts.body.iter().chain(&self.fonts.heading) {
            fonts
                .font_data
                .insert(face.name.clone(), Arc::clone(&face.data));
        }
        let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
        for (index, face) in self.fonts.body.iter().enumerate() {
            proportional.insert(index, face.name.clone());
        }
        let headings = self
            .fonts
            .heading
            .iter()
            .map(|face| face.name.clone())
            .chain(proportional.iter().cloned())
            .collect();
        fonts.families.insert(self.heading_family(), headings);
        fonts
    }

    fn visuals(&self) -> egui::Visuals {
        let palette = self.palette;
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = palette.surface;
        visuals.window_fill = palette.surface;
        visuals.window_stroke = Stroke::new(1.5, palette.accent_dim);
        visuals.window_corner_radius = CornerRadius::same(4);
        visuals.menu_corner_radius = CornerRadius::same(3);
        visuals.extreme_bg_color = palette.surface_deep;
        visuals.code_bg_color = palette.surface_raised;
        visuals.faint_bg_color = palette.faint_bg;
        visuals.selection.bg_fill = palette.selection_bg;
        visuals.selection.stroke = Stroke::new(1.0, palette.selection_stroke);
        visuals.hyperlink_color = palette.accent;
        visuals.warn_fg_color = palette.warn;
        visuals.error_fg_color = palette.error;
        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_fill = palette.surface;
        widgets.noninteractive.weak_bg_fill = palette.surface;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.accent_faint);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
        widgets.inactive.bg_fill = palette.surface_raised;
        widgets.inactive.weak_bg_fill = palette.surface_raised;
        widgets.inactive.bg_stroke = Stroke::new(1.0, palette.accent_dim);
        widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
        widgets.hovered.bg_fill = palette.hover_bg;
        widgets.hovered.weak_bg_fill = palette.hover_bg;
        widgets.hovered.bg_stroke = Stroke::new(1.5, palette.accent);
        widgets.hovered.fg_stroke = Stroke::new(1.5, palette.text_strong);
        widgets.active.bg_fill = palette.active_bg;
        widgets.active.weak_bg_fill = palette.active_bg;
        widgets.active.bg_stroke = Stroke::new(1.5, palette.accent);
        widgets.active.fg_stroke = Stroke::new(2.0, palette.text_strong);
        widgets.open.bg_fill = palette.surface_raised;
        widgets.open.weak_bg_fill = palette.surface_raised;
        widgets.open.bg_stroke = Stroke::new(1.0, palette.accent);
        widgets.open.fg_stroke = Stroke::new(1.0, palette.text_strong);
        for widget in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            widget.corner_radius = CornerRadius::same(2);
        }
        visuals
    }
}

fn text_styles(heading_family: FontFamily) -> BTreeMap<TextStyle, FontId> {
    [
        (TextStyle::Heading, FontId::new(19.0, heading_family)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One of egui's own bundled faces, re-registered under a new
    /// name, so the insertion path is exercised without the kit
    /// bundling a font.
    fn egui_face(bundled: &str, name: &str) -> FontFace {
        let defaults = egui::FontDefinitions::default();
        FontFace {
            name: name.to_owned(),
            data: Arc::clone(&defaults.font_data[bundled]),
        }
    }

    fn themed(fonts: Fonts) -> Theme {
        Theme {
            palette: Palette::titan_quest(),
            fonts,
        }
    }

    #[test]
    fn fonts_parse_and_the_dark_look_installs() {
        let theme = themed(Fonts {
            heading_family: Arc::from("test-heading"),
            heading: vec![egui_face("Hack", "display")],
            body: vec![egui_face("Ubuntu-Light", "body")],
        });
        let ctx = egui::Context::default();
        theme.apply(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label(theme.heading("Vault"));
            ui.label(theme.section("Equipment"));
            ui.label("body text");
        });
        output.textures_delta.clear();
        let style = ctx.style_of(egui::Theme::Dark);
        assert!(style.visuals.dark_mode);
        assert_eq!(style.visuals.panel_fill, Palette::titan_quest().surface);
        assert_eq!(
            style.text_styles[&TextStyle::Heading].family,
            theme.heading_family()
        );
        assert_eq!(ctx.theme(), egui::Theme::Dark);
    }

    #[test]
    fn heading_family_leads_with_the_heading_face_and_keeps_fallbacks() {
        let theme = themed(Fonts {
            heading_family: Arc::from("test-heading"),
            heading: vec![egui_face("Hack", "display")],
            body: vec![egui_face("Ubuntu-Light", "body")],
        });
        let fonts = theme.font_definitions();
        let stack = &fonts.families[&theme.heading_family()];
        assert_eq!(stack[0], "display");
        assert_eq!(stack[1], "body");
        assert!(stack.len() > 2);
        assert_eq!(fonts.families[&FontFamily::Proportional][0], "body");
    }

    #[test]
    fn without_faces_the_heading_family_is_the_body_stack() {
        let theme = themed(Fonts::default());
        let fonts = theme.font_definitions();
        assert_eq!(
            fonts.families[&theme.heading_family()],
            fonts.families[&FontFamily::Proportional]
        );
        assert_eq!(
            fonts.font_data.len(),
            egui::FontDefinitions::default().font_data.len()
        );
    }
}
