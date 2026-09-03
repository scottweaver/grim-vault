// Vendored from tq-univault crates/univault-gui/src/bin/preview.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Component preview harness: shows one kit component in isolation
//! over a checkerboard backdrop that proves transparency, painted
//! from procedurally drawn placeholder art so the art-free kit can be
//! previewed on its own.
//!
//! ```sh
//! cargo run -p univault-ui --features dev --bin preview -- tabbed-panel [--review] [--size WxH] [--review-dir DIR]
//! ```

use std::path::PathBuf;

use eframe::egui::{self, Color32, ColorImage, Margin, Rect, TextureHandle, pos2, vec2};
use univault_ui::chrome::{self, ButtonArt, NameplateArt, TooltipArt};
use univault_ui::components::gilded_border::GildedBorder;
use univault_ui::components::tabbed_panel::{self, LabelInk, TabbedPanel, TabbedPanelArt};
use univault_ui::review::ReviewOverlay;
use univault_ui::slice::{NinePatch, Src, ThreeSlice};
use univault_ui::theme::{Fonts, Palette, Theme};

const DEFAULT_REVIEW_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../review");

#[derive(Clone, Copy)]
enum Subject {
    GildedBorder,
    TabbedPanel,
    Chrome,
}

impl Subject {
    const ALL: [(&'static str, Self); 3] = [
        ("gilded-border", Self::GildedBorder),
        ("tabbed-panel", Self::TabbedPanel),
        ("chrome", Self::Chrome),
    ];

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(known, _)| *known == name)
            .map(|&(_, subject)| subject)
    }

    fn name(self) -> &'static str {
        match self {
            Self::GildedBorder => "gilded-border",
            Self::TabbedPanel => "tabbed-panel",
            Self::Chrome => "chrome",
        }
    }
}

struct Args {
    subject: Subject,
    review_mode: bool,
    size: [f32; 2],
    review_dir: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let review_mode = args.iter().any(|arg| arg == "--review");
    args.retain(|arg| arg != "--review");
    let size = match take_option(&mut args, "--size") {
        None => [900.0, 700.0],
        Some(spec) => parse_size(&spec).ok_or("--size expects WxH, e.g. --size 520x700")?,
    };
    let review_dir = take_option(&mut args, "--review-dir")
        .map_or_else(|| PathBuf::from(DEFAULT_REVIEW_DIR), PathBuf::from);
    let name = args.first().ok_or("no component named")?;
    let subject = Subject::from_name(name).ok_or_else(|| format!("unknown component: {name}"))?;
    Ok(Args {
        subject,
        review_mode,
        size,
        review_dir,
    })
}

/// Removes `flag` and its value from `args`, returning the value.
fn take_option(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let at = args.iter().position(|arg| arg == flag)?;
    let end = (at + 2).min(args.len());
    args.drain(at..end).nth(1)
}

fn parse_size(spec: &str) -> Option<[f32; 2]> {
    let (w, h) = spec.split_once('x')?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

fn main() -> eframe::Result {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "usage: cargo run -p univault-ui --features dev --bin preview -- <component> [--review] [--size WxH] [--review-dir DIR]"
            );
            eprintln!("components:");
            for (name, _) in Subject::ALL {
                eprintln!("  {name}");
            }
            std::process::exit(2);
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(args.size),
        ..Default::default()
    };
    eframe::run_native(
        &format!("univault-ui preview — {}", args.subject.name()),
        options,
        Box::new(move |cc| Ok(Box::new(PreviewApp::new(cc, &args)))),
    )
}

struct PreviewApp {
    subject: Subject,
    theme: Theme,
    gilded_border: GildedBorder,
    tabbed_panel: TabbedPanel,
    button: ButtonArt,
    nameplate: NameplateArt,
    tooltip: TooltipArt,
    selected_tab: usize,
    checkerboard: bool,
    review_mode: bool,
    review: ReviewOverlay,
}

impl PreviewApp {
    fn new(cc: &eframe::CreationContext<'_>, args: &Args) -> Self {
        let theme = Theme {
            palette: Palette::titan_quest(),
            fonts: Fonts::default(),
        };
        theme.apply(&cc.egui_ctx);
        let ctx = &cc.egui_ctx;
        Self {
            subject: args.subject,
            theme,
            gilded_border: placeholder::gilded_border(ctx),
            tabbed_panel: placeholder::tabbed_panel(ctx),
            button: placeholder::button(ctx),
            nameplate: placeholder::nameplate(ctx),
            tooltip: placeholder::tooltip(ctx),
            selected_tab: 0,
            checkerboard: true,
            review_mode: args.review_mode,
            review: ReviewOverlay::new(args.review_dir.clone()),
        }
    }

    fn show_subject(&mut self, ui: &mut egui::Ui, canvas: Rect) {
        let region = canvas.shrink(24.0);
        match self.subject {
            Subject::GildedBorder => {
                self.gilded_border.paint(ui.painter(), region);
                ui.painter().text(
                    region.center(),
                    egui::Align2::CENTER_CENTER,
                    "content area",
                    egui::FontId::proportional(16.0),
                    Color32::from_gray(140),
                );
            }
            Subject::TabbedPanel => {
                let titles = [
                    "Sword (152)",
                    "Axe (98)",
                    "Mace (74)",
                    "Spear (61)",
                    "Bow (88)",
                    "Thrown (37)",
                    "Staff (120)",
                    "Shield (143)",
                ];
                let tabs: Vec<tabbed_panel::Tab> = titles
                    .iter()
                    .map(|title| tabbed_panel::Tab::new(*title))
                    .chain(std::iter::once(tabbed_panel::Tab::disabled(
                        "Relics",
                        "no relics in this vault",
                    )))
                    .collect();
                let selected = self.selected_tab;
                let response = ui.scope_builder(egui::UiBuilder::new().max_rect(region), |ui| {
                    self.tabbed_panel.show(ui, &tabs, selected, |ui| {
                        ui.label(format!("content of \"{}\"", titles[selected]));
                        ui.set_min_size(ui.available_size());
                    })
                });
                if let Some(index) = response.inner.clicked {
                    self.selected_tab = index;
                }
            }
            Subject::Chrome => {
                ui.scope_builder(egui::UiBuilder::new().max_rect(region), |ui| {
                    chrome::nameplate(
                        ui,
                        &self.nameplate,
                        self.theme.heading_font(15.0),
                        "Caravan",
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        chrome::button(ui, &self.button, true, "Import");
                        chrome::button(ui, &self.button, true, "Export");
                        chrome::button(ui, &self.button, false, "Disabled");
                    });
                    ui.add_space(12.0);
                    let (tooltip, _) =
                        ui.allocate_exact_size(vec2(240.0, 120.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(tooltip, 0.0, self.theme.palette.popup);
                    chrome::tooltip_frame(ui.painter(), &self.tooltip, tooltip);
                    ui.painter().text(
                        tooltip.center(),
                        egui::Align2::CENTER_CENTER,
                        "tooltip frame",
                        egui::FontId::proportional(14.0),
                        self.theme.palette.text,
                    );
                });
            }
        }
    }
}

impl eframe::App for PreviewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("preview-controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(self.subject.name());
                ui.separator();
                ui.checkbox(&mut self.checkerboard, "checkerboard backdrop");
                ui.separator();
                ui.checkbox(&mut self.review_mode, "review");
                if self.review_mode {
                    self.review.toolbar(ui);
                } else {
                    ui.separator();
                    ui.label("resize the window to test edge stretch");
                }
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let canvas = ui.max_rect();
                if self.checkerboard {
                    checkerboard(ui.painter(), canvas);
                } else {
                    ui.painter()
                        .rect_filled(canvas, 0.0, Color32::from_gray(28));
                }
                self.show_subject(ui, canvas);
                if self.review_mode {
                    self.review
                        .overlay(ui, canvas, canvas.shrink(24.0), self.subject.name());
                }
            });
    }
}

fn checkerboard(painter: &egui::Painter, rect: Rect) {
    const SQUARE: f32 = 16.0;
    painter.rect_filled(rect, 0.0, Color32::from_gray(52));
    let light = Color32::from_gray(72);
    let mut row = 0_u32;
    let mut y = rect.min.y;
    while y < rect.max.y {
        let mut col = row % 2;
        let mut x = rect.min.x;
        while x < rect.max.x {
            if col.is_multiple_of(2) {
                let square = Rect::from_min_size(pos2(x, y), vec2(SQUARE, SQUARE)).intersect(rect);
                painter.rect_filled(square, 0.0, light);
            }
            col += 1;
            x += SQUARE;
        }
        row += 1;
        y += SQUARE;
    }
}

/// Procedurally drawn stand-ins for the art an app would ship, laid
/// out the way each component's docs describe, so the slicing is
/// exercised without a single PNG.
mod placeholder {
    use super::{
        ButtonArt, Color32, ColorImage, GildedBorder, LabelInk, Margin, NameplateArt, NinePatch,
        Src, TabbedPanel, TabbedPanelArt, TextureHandle, ThreeSlice, TooltipArt, egui,
    };

    const GOLD: Color32 = Color32::from_rgb(208, 172, 92);
    const GOLD_DIM: Color32 = Color32::from_rgb(128, 103, 54);
    const PLATE: Color32 = Color32::from_rgb(104, 74, 42);
    const PLATE_CAP: Color32 = Color32::from_rgb(84, 58, 32);
    const PLATE_OPEN: Color32 = Color32::from_rgb(140, 102, 56);
    const PLATE_OPEN_CAP: Color32 = Color32::from_rgb(118, 84, 44);
    const RAIL: Color32 = Color32::from_rgb(58, 42, 26);
    const IRON: Color32 = Color32::from_rgb(88, 88, 92);
    const IRON_CAP: Color32 = Color32::from_rgb(64, 64, 68);

    /// A transparent sheet with axis-aligned fills. Coordinates are
    /// `u16` so they convert losslessly both to pixel indices and to
    /// the `f32` slice geometry.
    struct Sheet {
        image: ColorImage,
    }

    impl Sheet {
        fn new(width: u16, height: u16) -> Self {
            Self {
                image: ColorImage::filled(
                    [usize::from(width), usize::from(height)],
                    Color32::TRANSPARENT,
                ),
            }
        }

        fn fill(&mut self, x: u16, y: u16, w: u16, h: u16, color: Color32) {
            for row in y..y + h {
                for col in x..x + w {
                    self.image[(usize::from(col), usize::from(row))] = color;
                }
            }
        }

        fn outline(&mut self, x: u16, y: u16, w: u16, h: u16, thickness: u16, color: Color32) {
            self.fill(x, y, w, thickness, color);
            self.fill(x, y + h - thickness, w, thickness, color);
            self.fill(x, y, thickness, h, color);
            self.fill(x + w - thickness, y, thickness, h, color);
        }

        fn upload(self, ctx: &egui::Context, name: &str) -> TextureHandle {
            ctx.load_texture(
                format!("placeholder:{name}"),
                self.image,
                egui::TextureOptions::LINEAR,
            )
        }
    }

    /// A 128 px square frame: a 2 px outer line, a 1 px inner line,
    /// and a filled bracket in each 40 px corner over an 8 px band.
    pub(super) fn gilded_border(ctx: &egui::Context) -> GildedBorder {
        const SIZE: u16 = 128;
        const CORNER: f32 = 40.0;
        const BAND: f32 = 8.0;
        let mut sheet = Sheet::new(SIZE, SIZE);
        sheet.outline(1, 1, SIZE - 2, SIZE - 2, 2, GOLD);
        sheet.outline(5, 5, SIZE - 10, SIZE - 10, 1, GOLD_DIM);
        for (x, y) in [
            (2, 2),
            (SIZE - 16, 2),
            (2, SIZE - 16),
            (SIZE - 16, SIZE - 16),
        ] {
            sheet.fill(x, y, 14, 14, GOLD);
            sheet.fill(x + 4, y + 4, 6, 6, RAIL);
        }
        let texture = sheet.upload(ctx, "gilded-border");
        let frame = NinePatch::symmetric(Src::full(&texture), CORNER, BAND);
        GildedBorder::new(texture, frame, Margin::same(14))
    }

    /// The tabbed-panel sheet: a closed plate, the open plate cut
    /// through the rail band with wings, and the rail's master corner
    /// and two clean strips.
    pub(super) fn tabbed_panel(ctx: &egui::Context) -> TabbedPanel {
        const PLATE_H: u16 = 35;
        const RAIL_TOP: u16 = 13;
        const RAIL_LEFT: u16 = 10;
        const CORNER: u16 = 24;
        const WING: u16 = 10;
        const INACTIVE: (u16, u16, u16) = (0, 0, 200);
        const ACTIVE: (u16, u16, u16) = (220, 0, 240);
        const CORNER_AT: (u16, u16) = (0, 60);
        const TOP_AT: (u16, u16, u16) = (40, 60, 60);
        const LEFT_AT: (u16, u16, u16) = (0, 100, 60);
        let mut sheet = Sheet::new(600, 200);

        let (x, y, w) = INACTIVE;
        sheet.fill(x, y, w, PLATE_H, PLATE);
        sheet.fill(x, y, 14, PLATE_H, PLATE_CAP);
        sheet.fill(x + w - 14, y, 14, PLATE_H, PLATE_CAP);
        sheet.outline(x, y, w, PLATE_H, 1, GOLD_DIM);

        let (x, y, w) = ACTIVE;
        let open_h = PLATE_H + RAIL_TOP;
        sheet.fill(x, y + PLATE_H, w, RAIL_TOP, RAIL);
        sheet.fill(x, y + PLATE_H, w, 2, GOLD);
        sheet.fill(x + WING, y, w - 2 * WING, open_h, PLATE_OPEN);
        sheet.fill(x + WING, y, 26 - WING, open_h, PLATE_OPEN_CAP);
        sheet.fill(x + w - 26, y, 26 - WING, open_h, PLATE_OPEN_CAP);
        sheet.fill(x + WING, y, 1, open_h, GOLD);
        sheet.fill(x + w - WING - 1, y, 1, open_h, GOLD);
        sheet.fill(x + WING, y, w - 2 * WING, 1, GOLD);

        let (x, y) = CORNER_AT;
        sheet.fill(x, y, CORNER, RAIL_TOP, RAIL);
        sheet.fill(x, y, RAIL_LEFT, CORNER, RAIL);
        sheet.fill(x, y, CORNER, 2, GOLD);
        sheet.fill(x, y, 2, CORNER, GOLD);
        sheet.fill(
            x + RAIL_LEFT,
            y + RAIL_TOP - 1,
            CORNER - RAIL_LEFT,
            1,
            GOLD_DIM,
        );
        sheet.fill(
            x + RAIL_LEFT - 1,
            y + RAIL_TOP,
            1,
            CORNER - RAIL_TOP,
            GOLD_DIM,
        );

        let (x, y, w) = TOP_AT;
        sheet.fill(x, y, w, RAIL_TOP, RAIL);
        sheet.fill(x, y, w, 2, GOLD);
        sheet.fill(x, y + RAIL_TOP - 1, w, 1, GOLD_DIM);

        let (x, y, h) = LEFT_AT;
        sheet.fill(x, y, RAIL_LEFT, h, RAIL);
        sheet.fill(x, y, 2, h, GOLD);
        sheet.fill(x + RAIL_LEFT - 1, y, 1, h, GOLD_DIM);

        let texture = sheet.upload(ctx, "tabbed-panel");
        let px = f32::from;
        let art = TabbedPanelArt {
            inactive: ThreeSlice::new(
                Src::new(px(INACTIVE.0), px(INACTIVE.1), px(INACTIVE.2), px(PLATE_H)),
                14.0,
            ),
            active: ThreeSlice::new(
                Src::new(px(ACTIVE.0), px(ACTIVE.1), px(ACTIVE.2), px(open_h)),
                26.0,
            ),
            active_wing: px(WING),
            rail: NinePatch::mirrored(
                Src::new(px(CORNER_AT.0), px(CORNER_AT.1), px(CORNER), px(CORNER)),
                Src::new(px(TOP_AT.0), px(TOP_AT.1), px(TOP_AT.2), px(RAIL_TOP)),
                Src::new(px(LEFT_AT.0), px(LEFT_AT.1), px(RAIL_LEFT), px(LEFT_AT.2)),
            ),
            interior: Color32::BLACK,
            ink: LabelInk {
                inactive: Color32::from_rgb(186, 166, 118),
                active: Color32::from_rgb(236, 218, 164),
                disabled: Color32::from_gray(115),
            },
            margin: Margin {
                left: 18,
                right: 18,
                top: 56,
                bottom: 21,
            },
        };
        TabbedPanel::new(texture, art)
    }

    fn plate(
        ctx: &egui::Context,
        name: &str,
        w: u16,
        h: u16,
        caps: u16,
        fill: Color32,
        cap: Color32,
    ) -> TextureHandle {
        let mut sheet = Sheet::new(w, h);
        sheet.fill(0, 0, w, h, fill);
        sheet.fill(0, 0, caps, h, cap);
        sheet.fill(w - caps, 0, caps, h, cap);
        sheet.outline(0, 0, w, h, 1, GOLD_DIM);
        sheet.upload(ctx, name)
    }

    pub(super) fn button(ctx: &egui::Context) -> ButtonArt {
        ButtonArt {
            up: plate(ctx, "button-up", 64, 26, 8, PLATE, PLATE_CAP),
            over: plate(ctx, "button-over", 64, 26, 8, PLATE_OPEN, PLATE_OPEN_CAP),
            down: plate(ctx, "button-down", 64, 26, 8, RAIL, PLATE_CAP),
            caps: 8.0,
            height: 26.0,
            ink: Color32::from_rgb(24, 18, 8),
            disabled_ink: Color32::from_rgb(52, 44, 28),
        }
    }

    pub(super) fn nameplate(ctx: &egui::Context) -> NameplateArt {
        NameplateArt {
            plate: plate(ctx, "nameplate", 120, 30, 24, IRON, IRON_CAP),
            caps: 24.0,
            height: 30.0,
            ink: Color32::from_rgb(24, 18, 8),
        }
    }

    /// Eight 4 px tiles: corners solid gold, edges a gold line over
    /// dim gold.
    pub(super) fn tooltip(ctx: &egui::Context) -> TooltipArt {
        let corner = |name: &str| {
            let mut sheet = Sheet::new(4, 4);
            sheet.fill(0, 0, 4, 4, GOLD);
            sheet.upload(ctx, name)
        };
        let edge = |name: &str, horizontal: bool| {
            let mut sheet = Sheet::new(4, 4);
            sheet.fill(0, 0, 4, 4, GOLD_DIM);
            if horizontal {
                sheet.fill(0, 1, 4, 2, GOLD);
            } else {
                sheet.fill(1, 0, 2, 4, GOLD);
            }
            sheet.upload(ctx, name)
        };
        TooltipArt {
            top_left: corner("tooltip-tl"),
            top: edge("tooltip-t", true),
            top_right: corner("tooltip-tr"),
            left: edge("tooltip-l", false),
            right: edge("tooltip-r", false),
            bottom_left: corner("tooltip-bl"),
            bottom: edge("tooltip-b", true),
            bottom_right: corner("tooltip-br"),
            band: 4.0,
        }
    }
}
