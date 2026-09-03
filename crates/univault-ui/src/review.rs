// Vendored from tq-univault crates/univault-gui/src/review.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! In-preview review overlay: drag rectangles, ellipses, and arrows
//! over a component, type a note per shape, and export the annotated
//! frame as PNG + JSON into the directory the harness chose — the PNG
//! for human eyes, the JSON (window- and component-space coordinates
//! per shape) for an agent to act on. Preview-harness only; apps
//! don't wire it in.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{
    Align2, Color32, ColorImage, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2,
    vec2,
};

const INK: Color32 = Color32::from_rgb(255, 64, 48);
const NOTE_INK: Color32 = Color32::from_rgb(255, 235, 230);
const NOTE_BG: Color32 = Color32::from_rgba_premultiplied(40, 8, 4, 230);
const STROKE_W: f32 = 2.0;
const BADGE_R: f32 = 9.0;

/// Drags smaller than this are discarded as slips.
const MIN_DRAG: f32 = 6.0;

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Rect,
    Ellipse,
    Arrow,
}

/// A committed shape, in window points.
#[derive(Clone, Copy)]
enum Geom {
    Rect(Rect),
    Ellipse(Rect),
    Arrow { from: Pos2, to: Pos2 },
}

impl Geom {
    fn from_drag(tool: Tool, from: Pos2, to: Pos2) -> Self {
        match tool {
            Tool::Rect => Self::Rect(Rect::from_two_pos(from, to)),
            Tool::Ellipse => Self::Ellipse(Rect::from_two_pos(from, to)),
            Tool::Arrow => Self::Arrow { from, to },
        }
    }

    /// Where the badge and note hang off the shape.
    fn anchor(self) -> Pos2 {
        match self {
            Self::Rect(rect) | Self::Ellipse(rect) => rect.left_top(),
            Self::Arrow { from, .. } => from,
        }
    }
}

enum Draft {
    Dragging {
        from: Pos2,
        to: Pos2,
    },
    Noting {
        geom: Geom,
        note: String,
        fresh: bool,
    },
    /// Reopened via its badge: editing (or deleting) a committed
    /// annotation.
    Editing {
        index: usize,
        note: String,
        fresh: bool,
    },
}

struct Annotation {
    geom: Geom,
    note: String,
}

/// The overlay's whole state; the harness owns one and calls
/// [`Self::toolbar`] in its control row and [`Self::overlay`] over
/// the component canvas while review mode is on.
pub struct ReviewOverlay {
    export_dir: PathBuf,
    tool: Option<Tool>,
    annotations: Vec<Annotation>,
    draft: Option<Draft>,
    awaiting_screenshot: bool,
    status: Option<String>,
}

impl ReviewOverlay {
    /// Exports land in `export_dir`, created on first export.
    #[must_use]
    pub fn new(export_dir: impl Into<PathBuf>) -> Self {
        Self {
            export_dir: export_dir.into(),
            tool: None,
            annotations: Vec::new(),
            draft: None,
            awaiting_screenshot: false,
            status: None,
        }
    }

    #[must_use]
    pub fn export_dir(&self) -> &Path {
        &self.export_dir
    }

    /// Tool selector, undo/clear, and the export button. A tool is
    /// *armed* by clicking it — only then does the overlay capture
    /// canvas input; clicking the armed tool again releases it, so
    /// the component stays interactive between annotations.
    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        for (label, value) in [
            ("rect", Tool::Rect),
            ("ellipse", Tool::Ellipse),
            ("arrow", Tool::Arrow),
        ] {
            let armed = self.tool == Some(value);
            if ui.selectable_label(armed, label).clicked() {
                self.tool = if armed { None } else { Some(value) };
            }
        }
        if self.tool.is_none() {
            ui.label("arm a tool to draw; click a badge to edit");
        }
        ui.separator();
        if ui.button("undo").clicked() {
            self.annotations.pop();
        }
        if ui.button("clear").clicked() {
            self.annotations.clear();
            self.draft = None;
        }
        if ui.button("export").clicked() && !self.annotations.is_empty() {
            self.awaiting_screenshot = true;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if let Some(status) = &self.status {
            ui.separator();
            ui.label(status.clone());
        }
    }

    /// Input capture, shape painting, the note editor, and export
    /// completion. Call after the component is drawn; `component` is
    /// the rect the component occupies, the JSON's second coordinate
    /// space.
    pub fn overlay(
        &mut self,
        ui: &mut egui::Ui,
        canvas: Rect,
        component: Rect,
        component_name: &str,
    ) {
        if let Some(tool) = self.tool {
            let response = ui
                .interact(
                    canvas,
                    ui.id().with("review-overlay"),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(CursorIcon::Crosshair);
            let drag_pos = response.interact_pointer_pos();
            if response.drag_started()
                && let Some(pos) = drag_pos
                && !matches!(
                    self.draft,
                    Some(Draft::Noting { .. } | Draft::Editing { .. })
                )
            {
                self.draft = Some(Draft::Dragging { from: pos, to: pos });
            }
            if let Some(Draft::Dragging { to, .. }) = &mut self.draft
                && let Some(pos) = drag_pos
            {
                *to = pos;
            }
            if response.drag_stopped()
                && let Some(Draft::Dragging { from, to }) = self.draft
            {
                self.draft = ((to - from).length() >= MIN_DRAG).then(|| Draft::Noting {
                    geom: Geom::from_drag(tool, from, to),
                    note: String::new(),
                    fresh: true,
                });
            }
        } else if matches!(self.draft, Some(Draft::Dragging { .. })) {
            self.draft = None;
        }

        let painter = ui.painter();
        for (index, annotation) in self.annotations.iter().enumerate() {
            paint_geom(painter, annotation.geom);
            paint_badge(painter, annotation.geom.anchor(), index + 1);
            paint_note(painter, annotation.geom.anchor(), &annotation.note);
        }
        match (&self.draft, self.tool) {
            (Some(Draft::Dragging { from, to }), Some(tool)) => {
                paint_geom(painter, Geom::from_drag(tool, *from, *to));
            }
            (Some(Draft::Noting { geom, .. }), _) => {
                paint_geom(painter, *geom);
                paint_badge(painter, geom.anchor(), self.annotations.len() + 1);
            }
            (Some(Draft::Dragging { .. }), None) | (Some(Draft::Editing { .. }) | None, _) => {}
        }
        self.badge_clicks(ui);

        self.note_editor(ui);
        self.finish_export(ui, component, component_name);
    }

    /// A committed annotation's badge reopens it for editing (or
    /// deleting). Badges are registered after everything else, so
    /// they win the hit-test even over an armed tool's capture.
    fn badge_clicks(&mut self, ui: &egui::Ui) {
        if matches!(
            self.draft,
            Some(Draft::Noting { .. } | Draft::Editing { .. })
        ) {
            return;
        }
        for (index, annotation) in self.annotations.iter().enumerate() {
            let hit = Rect::from_center_size(
                annotation.geom.anchor(),
                vec2(2.0 * BADGE_R, 2.0 * BADGE_R),
            );
            let response = ui
                .interact(hit, ui.id().with(("review-badge", index)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            if response.clicked() {
                self.draft = Some(Draft::Editing {
                    index,
                    note: annotation.note.clone(),
                    fresh: true,
                });
                return;
            }
        }
    }

    /// The floating text field of a just-drawn shape or a reopened
    /// badge. Enter commits, Escape discards the change; a reopened
    /// annotation also offers deletion.
    fn note_editor(&mut self, ui: &mut egui::Ui) {
        let (anchor, deletable) = match &self.draft {
            Some(Draft::Noting { geom, .. }) => (geom.anchor(), false),
            Some(Draft::Editing { index, .. }) => (self.annotations[*index].geom.anchor(), true),
            Some(Draft::Dragging { .. }) | None => return,
        };
        let Some(Draft::Noting { note, fresh, .. } | Draft::Editing { note, fresh, .. }) =
            &mut self.draft
        else {
            return;
        };
        let at = anchor + vec2(BADGE_R + 4.0, BADGE_R + 4.0);
        let mut committed = false;
        let mut cancelled = false;
        let mut deleted = false;
        egui::Area::new(ui.id().with("review-note"))
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let edit = ui.add(
                            egui::TextEdit::singleline(note)
                                .hint_text("what's wrong here? (Enter saves, Esc discards)")
                                .desired_width(300.0),
                        );
                        if *fresh {
                            edit.request_focus();
                            *fresh = false;
                        }
                        if deletable {
                            deleted = ui.button("delete").clicked();
                        }
                        cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        committed = !cancelled
                            && !deleted
                            && edit.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    });
                });
            });
        if deleted {
            if let Some(Draft::Editing { index, .. }) = self.draft.take() {
                self.annotations.remove(index);
            }
        } else if committed {
            match self.draft.take() {
                Some(Draft::Noting { geom, note, .. }) => {
                    self.annotations.push(Annotation { geom, note });
                }
                Some(Draft::Editing { index, note, .. }) => {
                    if let Some(annotation) = self.annotations.get_mut(index) {
                        annotation.note = note;
                    }
                }
                Some(Draft::Dragging { .. }) | None => {}
            }
        } else if cancelled {
            self.draft = None;
        }
    }

    /// Writes the pending export once the harness delivers the
    /// screenshot of the annotated frame.
    fn finish_export(&mut self, ui: &egui::Ui, component: Rect, component_name: &str) {
        if !self.awaiting_screenshot {
            return;
        }
        let shot = ui.ctx().input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = shot else {
            return;
        };
        self.awaiting_screenshot = false;
        let pixels_per_point = ui.ctx().pixels_per_point();
        self.status = Some(
            match write_export(
                &self.export_dir,
                &image,
                &self.annotations,
                component,
                component_name,
                pixels_per_point,
            ) {
                Ok(stem) => format!("saved {}.png+.json", self.export_dir.join(stem).display()),
                Err(error) => format!("export failed: {error}"),
            },
        );
    }
}

fn paint_geom(painter: &egui::Painter, geom: Geom) {
    let stroke = Stroke::new(STROKE_W, INK);
    match geom {
        Geom::Rect(rect) => {
            painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Middle);
        }
        Geom::Ellipse(rect) => {
            painter.add(egui::Shape::from(egui::epaint::EllipseShape::stroke(
                rect.center(),
                rect.size() / 2.0,
                stroke,
            )));
        }
        Geom::Arrow { from, to } => painter.arrow(from, to - from, stroke),
    }
}

fn paint_badge(painter: &egui::Painter, anchor: Pos2, number: usize) {
    painter.circle_filled(anchor, BADGE_R, INK);
    painter.text(
        anchor,
        Align2::CENTER_CENTER,
        number.to_string(),
        FontId::proportional(12.0),
        Color32::WHITE,
    );
}

fn paint_note(painter: &egui::Painter, anchor: Pos2, note: &str) {
    if note.is_empty() {
        return;
    }
    let at = anchor + vec2(BADGE_R + 4.0, -BADGE_R);
    let galley = painter.layout_no_wrap(note.to_string(), FontId::proportional(13.0), NOTE_INK);
    let bg = Rect::from_min_size(at, galley.size() + vec2(8.0, 4.0));
    painter.rect_filled(bg, 3.0, NOTE_BG);
    painter.galley(at + vec2(4.0, 2.0), galley, NOTE_INK);
}

/// Writes `review-<unix-secs>.png` (the annotated frame) and its
/// `.json` twin into `dir`; returns the file stem.
fn write_export(
    dir: &Path,
    image: &ColorImage,
    annotations: &[Annotation],
    component: Rect,
    component_name: &str,
    pixels_per_point: f32,
) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let stem = format!("review-{secs}");

    let [width, height] = image.size;
    let mut bytes = Vec::with_capacity(width * height * 4);
    for pixel in &image.pixels {
        bytes.extend_from_slice(&pixel.to_array());
    }
    let dims = [width, height].map(u32::try_from);
    let [width, height] = [
        dims[0].map_err(|error| error.to_string())?,
        dims[1].map_err(|error| error.to_string())?,
    ];
    image::RgbaImage::from_raw(width, height, bytes)
        .ok_or_else(|| "screenshot pixel buffer does not match its dimensions".to_owned())?
        .save_with_format(dir.join(format!("{stem}.png")), image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;

    let annotations: Vec<serde_json::Value> = annotations
        .iter()
        .enumerate()
        .map(|(index, annotation)| annotation_json(index + 1, annotation, component))
        .collect();
    let json = serde_json::json!({
        "component": component_name,
        "exported_at_unix": secs,
        "pixels_per_point": pixels_per_point,
        "component_rect_window": rect_json(component),
        "annotations": annotations,
    });
    std::fs::write(
        dir.join(format!("{stem}.json")),
        serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(stem)
}

fn rect_json(rect: Rect) -> serde_json::Value {
    serde_json::json!([rect.min.x, rect.min.y, rect.width(), rect.height()])
}

fn pos_json(pos: Pos2) -> serde_json::Value {
    serde_json::json!([pos.x, pos.y])
}

fn annotation_json(number: usize, annotation: &Annotation, component: Rect) -> serde_json::Value {
    let relative = |pos: Pos2| pos2(pos.x - component.min.x, pos.y - component.min.y);
    let mut value = match annotation.geom {
        Geom::Rect(rect) | Geom::Ellipse(rect) => serde_json::json!({
            "kind": if matches!(annotation.geom, Geom::Rect(_)) { "rect" } else { "ellipse" },
            "window": rect_json(rect),
            "component": rect_json(Rect::from_min_size(relative(rect.min), rect.size())),
        }),
        Geom::Arrow { from, to } => serde_json::json!({
            "kind": "arrow",
            "window_from": pos_json(from),
            "window_to": pos_json(to),
            "component_from": pos_json(relative(from)),
            "component_to": pos_json(relative(to)),
        }),
    };
    let object = value
        .as_object_mut()
        .expect("annotation_json builds an object");
    object.insert("n".into(), serde_json::json!(number));
    object.insert("note".into(), serde_json::json!(annotation.note));
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_lands_in_the_configured_directory() {
        let dir = std::env::temp_dir().join(format!(
            "univault-ui-review-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let image = ColorImage::filled([4, 3], Color32::RED);
        let annotations = [Annotation {
            geom: Geom::Rect(Rect::from_min_size(pos2(10.0, 20.0), vec2(30.0, 40.0))),
            note: "off by one".to_owned(),
        }];
        let component = Rect::from_min_size(pos2(5.0, 5.0), vec2(100.0, 100.0));

        let stem =
            write_export(&dir, &image, &annotations, component, "gilded-border", 2.0).unwrap();

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(format!("{stem}.json"))).unwrap())
                .unwrap();
        assert_eq!(json["component"], "gilded-border");
        assert_eq!(
            json["annotations"][0]["component"],
            serde_json::json!([5.0, 15.0, 30.0, 40.0])
        );
        assert_eq!(json["annotations"][0]["note"], "off by one");
        let png = std::fs::read(dir.join(format!("{stem}.png"))).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
