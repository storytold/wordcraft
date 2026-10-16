//! Canvas object interaction: the shape tools (drag to draw a shape on the page).

use egui::{Color32, Pos2, Rect, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_doc::para::ShapeKind;
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::theme::{Tokens, regular};

/// Below this the drawing gesture inserts a default-sized shape instead.
const DRAG_SLOP: f32 = 3.0;

/// A shape drawing stroke in progress: its page and starting/current page coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeDrawing {
    pub page: usize,
    pub from: (f32, f32),
    pub to: (f32, f32),
}

fn fmt_in(w: f32, h: f32) -> String {
    format!("{:.2}\" × {:.2}\"", w / 72.0, h / 72.0)
}

/// Turn the tool off (Esc).
pub fn stop(app: &mut WordApp) -> bool {
    let was = app.canvas.shape_tool.is_some();
    app.canvas.shape_tool = None;
    app.canvas.shape_drawing = None;
    was
}

/// Status message hint when a shape tool is armed.
pub fn tool_hint(kind: ShapeKind) -> String {
    format!("{} — {}", tl!(kind.label()), tl!("Drag on the page to draw it; Esc to cancel"))
}

/// Set crosshair cursor while the shape tool is armed.
pub fn cursor(app: &WordApp, ui: &Ui, resp: &egui::Response) {
    if app.canvas.shape_tool.is_some() && (resp.hovered() || app.canvas.shape_drawing.is_some()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
}

/// Mouse and keyboard interaction for the shape tool.
/// Returns true when this tool handled the event so caret/text selection stands down.
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) -> bool {
    let Some(kind) = app.canvas.shape_tool else { return false };
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        stop(app);
        return true;
    }
    let latest = ui.input(|i| i.pointer.latest_pos());
    if ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() {
        if let Some(p) = latest
            && let Some((page, x, y)) = crate::canvas::page_at(rects, layout, scale, p)
        {
            app.canvas.shape_drawing = Some(ShapeDrawing { page, from: (x, y), to: (x, y) });
        }
        return true;
    }
    let Some(mut st) = app.canvas.shape_drawing else {
        // Shape tool is armed, hovering waiting for press
        return false;
    };
    if let Some(p) = latest
        && let Some(r) = rects.get(st.page)
    {
        let sc = layout.pages.get(st.page).filter(|pg| pg.w > 0.0).map_or(scale, |pg| r.width() / pg.w);
        if sc > 0.0 {
            st.to = ((p.x - r.min.x) / sc, (p.y - r.min.y) / sc);
            app.canvas.shape_drawing = Some(st);
        }
    }
    if ui.input(|i| i.pointer.primary_down()) {
        return true;
    }
    // Primary released: finish drawing and insert shape
    app.canvas.shape_drawing = None;
    app.canvas.shape_tool = None;

    let (dw, dh) = ((st.to.0 - st.from.0).abs(), (st.to.1 - st.from.1).abs());
    let (w, h, ox, oy) = if dw < DRAG_SLOP && dh < DRAG_SLOP {
        let (def_w, def_h) = if kind.is_line_like() { (108.0, 1.0) } else { (108.0, 72.0) };
        (def_w, def_h, st.from.0.max(0.0), st.from.1.max(0.0))
    } else {
        let w = dw.clamp(4.0, 2000.0);
        let h = dh.clamp(if kind.is_line_like() { 1.0 } else { 4.0 }, 2000.0);
        let ox = st.from.0.min(st.to.0).max(0.0);
        let oy = st.from.1.min(st.to.1).max(0.0);
        (w, h, ox, oy)
    };
    let _ = app.run(
        "insert.shape",
        json!({
            "kind": kind.ooxml_prst(),
            "width": w,
            "height": h,
            "x": ox,
            "y": oy,
            "anchor": "page",
            "wrap": "square",
        }),
    );
    true
}

/// Draw the ghost preview of the shape being drawn and its dimension badge.
pub fn paint(app: &WordApp, painter: &egui::Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    let Some(kind) = app.canvas.shape_tool else { return };
    let Some(st) = app.canvas.shape_drawing else { return };
    let Some(pr) = pages.get(st.page) else { return };
    let sc = layout.pages.get(st.page).filter(|pg| pg.w > 0.0).map_or(scale, |pg| pr.width() / pg.w);
    if sc <= 0.0 {
        return;
    }
    let a = pos2(pr.min.x + st.from.0 * sc, pr.min.y + st.from.1 * sc);
    let b = pos2(pr.min.x + st.to.0 * sc, pr.min.y + st.to.1 * sc);
    let r = Rect::from_two_pos(a, b);
    let (dw, dh) = ((st.to.0 - st.from.0).abs(), (st.to.1 - st.from.1).abs());
    let big = dw > DRAG_SLOP || dh > DRAG_SLOP;

    // Preview contours inside the rectangle
    let inner = r.shrink(1.0);
    if big && inner.width() > 2.0 && inner.height() > 1.0 {
        for (closed, pts) in wordcraft_render::shape_preview_contours(kind, inner.width(), inner.height()) {
            let pts: Vec<Pos2> = pts.into_iter().map(|(x, y)| pos2(inner.min.x + x, inner.min.y + y)).collect();
            let stroke = Stroke::new(1.2, t.accent);
            if closed && !kind.is_line_like() {
                painter.add(egui::Shape::closed_line(pts, stroke));
            } else {
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
    }
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);

    if big {
        let label = fmt_in(dw, dh);
        let fs = 11.0;
        let g = painter.layout(label, regular(fs), Color32::WHITE, 200.0);
        let box_r = Rect::from_center_size(pos2(r.center().x, r.min.y - 12.0), vec2(g.size().x + 10.0, g.size().y + 4.0));
        painter.rect_filled(box_r, 2.0, t.accent);
        painter.galley(pos2(box_r.center().x - g.size().x / 2.0, box_r.center().y - g.size().y / 2.0), g, Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::Document;

    #[test]
    fn shape_tool_arm_and_stop() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        assert_eq!(app.canvas.shape_tool, None);

        // Arm the tool
        app.canvas.shape_tool = Some(ShapeKind::RoundedRectangle);
        assert_eq!(app.canvas.shape_tool, Some(ShapeKind::RoundedRectangle));

        // Stop disarms the tool
        assert!(stop(&mut app));
        assert_eq!(app.canvas.shape_tool, None);
        assert_eq!(app.canvas.shape_drawing, None);
    }

    #[test]
    fn shape_tool_hint_text() {
        let hint = tool_hint(ShapeKind::Rectangle);
        assert!(hint.contains("Rectangle"));
        assert!(hint.contains("Drag"));
    }

    #[test]
    fn shape_insert_with_coordinates() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        let res = app.run(
            "insert.shape",
            json!({
                "kind": "ellipse",
                "width": 150.0,
                "height": 100.0,
                "x": 50.0,
                "y": 60.0,
                "anchor": "page"
            }),
        );
        assert!(res.is_ok());

        // Verify object was inserted
        let sel = wordcraft_engine::cmd::objects::selected(&app.session);
        assert!(sel.is_some());
        let (_, obj) = sel.unwrap();
        match obj {
            wordcraft_doc::para::InlineObject::Shape { kind, w, h, float, .. } => {
                assert_eq!(kind, ShapeKind::Ellipse);
                assert_eq!(w, 150.0);
                assert_eq!(h, 100.0);
                assert_eq!(float.x, 50.0);
                assert_eq!(float.y, 60.0);
                assert_eq!(float.h_rel, wordcraft_doc::para::Anchor::Page);
            }
            _ => panic!("expected shape"),
        }
    }
}
