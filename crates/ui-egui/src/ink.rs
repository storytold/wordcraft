//! Drawing on the page with the Draw tab's pens and eraser. While a pen is in use, a drag draws a
//! live stroke and its release commits it with `draw.stroke` (one command, one undo step); with
//! the eraser, a click or drag over a stroke deletes it with `draw.erase` (one undo step per
//! drag). Select (or Esc) goes back to ordinary selection.

use egui::{Color32, Painter, Pos2, Rect, Stroke, Ui};
use serde_json::json;
use wordcraft_doc::freeform::{InkTool, MAX_POINTS};
use wordcraft_engine::cmd::draw::DrawMode;
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::canvas::page_at;

/// A stroke being drawn: its page and points (page points).
pub struct InkDrag {
    page: usize,
    tool: InkTool,
    pts: Vec<[f32; 2]>,
}

/// Smallest move that adds a point to a live stroke, screen points.
const MIN_STEP_PX: f32 = 1.5;

/// Handle the pointer while a pen or the eraser is in use; `true` when it did (no text
/// selection or object dragging then).
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) -> bool {
    let mode = app.session.view.draw.mode;
    if mode == DrawMode::Select {
        app.canvas.ink = None;
        app.canvas.erasing = None;
        return false;
    }
    let (pressed, down, released, at) =
        ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.latest_pos()));
    let starts = pressed && resp.contains_pointer();
    match mode {
        DrawMode::Pen(tool) => {
            if starts && let Some((page, x, y)) = at.and_then(|p| page_at(rects, layout, scale, p)) {
                app.canvas.ink = Some(InkDrag { page, tool, pts: vec![[x, y]] });
            } else if down
                && let Some(p) = at
                && let Some(d) = app.canvas.ink.as_mut()
                && let Some((x, y)) = on_page(rects, layout, scale, d.page, p)
            {
                let far = d.pts.last().is_none_or(|[lx, ly]| ((x - lx).powi(2) + (y - ly).powi(2)).sqrt() * scale >= MIN_STEP_PX);
                if far && d.pts.len() < MAX_POINTS {
                    d.pts.push([x, y]);
                }
            }
            if released && let Some(d) = app.canvas.ink.take() {
                let _ = app.run("draw.stroke", json!({"page": d.page, "points": d.pts, "tool": d.tool.name()}));
            }
        }
        DrawMode::Eraser => {
            if starts {
                app.canvas.erasing = Some(false);
            }
            if (starts || down)
                && let Some(erased) = app.canvas.erasing
                && let Some((page, x, y)) = at.and_then(|p| page_at(rects, layout, scale, p))
                && wordcraft_engine::cmd::draw::ink_at(&mut app.session, page, x, y).is_some()
            {
                // The first stroke a drag erases starts an undo step; the rest join it.
                if erased {
                    app.session.join_next_undo();
                }
                if app.run("draw.erase", json!({"page": page, "x": x, "y": y})).is_ok() {
                    app.canvas.erasing = Some(true);
                }
            }
            if released {
                app.canvas.erasing = None;
            }
        }
        DrawMode::Select => {}
    }
    true
}

/// Page point under screen point `p` on `page` (outside the page too, clamped by the command).
fn on_page(rects: &[Rect], layout: &DocLayout, scale: f32, page: usize, p: Pos2) -> Option<(f32, f32)> {
    let r = rects.get(page)?;
    let s = layout.pages.get(page).map_or(scale, |pg| crate::canvas::page_screen_scale(*r, pg, scale));
    (s > 0.0).then(|| ((p.x - r.min.x) / s, (p.y - r.min.y) / s))
}

/// The live stroke, drawn over the page until it is committed.
pub fn paint(app: &WordApp, painter: &Painter, rects: &[Rect], layout: &DocLayout, scale: f32) {
    let Some(d) = &app.canvas.ink else { return };
    let Some(r) = rects.get(d.page) else { return };
    let s = layout.pages.get(d.page).map_or(scale, |pg| crate::canvas::page_screen_scale(*r, pg, scale));
    let set = app.session.view.draw.settings(d.tool);
    let alpha = (d.tool.alpha() * 255.0).round() as u8;
    let color = Color32::from_rgba_unmultiplied(set.color.0, set.color.1, set.color.2, alpha);
    let pts: Vec<Pos2> = d.pts.iter().map(|[x, y]| egui::pos2(r.min.x + x * s, r.min.y + y * s)).collect();
    let width = (set.width * s).max(1.0);
    if let [only] = pts.as_slice() {
        painter.circle_filled(*only, width / 2.0, color);
    } else {
        painter.add(egui::Shape::line(pts, Stroke::new(width, color)));
    }
}

/// The pointer over the page while a pen or the eraser is in use.
pub fn cursor(app: &WordApp) -> Option<egui::CursorIcon> {
    match app.session.view.draw.mode {
        DrawMode::Select => None,
        DrawMode::Eraser | DrawMode::Pen(_) => Some(egui::CursorIcon::Crosshair),
    }
}
