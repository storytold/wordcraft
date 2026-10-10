//! Draw Table and Eraser as canvas modes (Insert › Tables, Table Layout › Draw). The buttons
//! toggle a pen or an eraser; while one is on, presses on the page draw (or erase) instead of
//! moving the caret, a preview follows the pointer, and each finished stroke or click runs
//! `table.draw` / `table.eraser` with page coordinates (one undo step each). Esc or the button
//! again turns it off. The geometry decisions are the engine's (`cmd::table_draw`).

use egui::{Color32, Pos2, Rect, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_engine::cmd::table_draw::{self, Stroke as Kind};
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::theme::Tokens;

/// Which table tool is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableTool {
    Draw,
    Erase,
}

impl TableTool {
    fn id(self) -> &'static str {
        match self {
            TableTool::Draw => "table.draw",
            TableTool::Erase => "table.eraser",
        }
    }
}

/// A stroke in progress: its page and its start and current end (page points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PenStroke {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
}

/// The ribbon buttons (no page coordinates): turn the tool on, or off when it already is.
/// `{"on": bool}` sets it. With coordinates the command goes to the engine instead (`None`).
pub fn toggle(app: &mut WordApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let tool = match id {
        "table.draw" => TableTool::Draw,
        "table.eraser" => TableTool::Erase,
        _ => return None,
    };
    if p.get("page").is_some() {
        return None;
    }
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(app.canvas.table_tool != Some(tool));
    app.canvas.table_tool = on.then_some(tool);
    app.canvas.table_stroke = None;
    if on {
        app.canvas.want_focus = true;
        app.status(match tool {
            TableTool::Draw => tl!("Drag a rectangle to draw a table, or a line across cells to split them. Esc stops."),
            TableTool::Erase => tl!("Click a border between two cells to merge them. Esc stops."),
        });
    }
    Some(Ok(json!({"tool": app.canvas.table_tool.map(TableTool::id)})))
}

/// Turn the tool off (Esc).
pub fn stop(app: &mut WordApp) -> bool {
    let was = app.canvas.table_tool.is_some();
    app.canvas.table_tool = None;
    app.canvas.table_stroke = None;
    was
}

/// The page point under screen point `p` on page `page` (its screen rect from `rects`).
fn on_page(rects: &[Rect], layout: &DocLayout, scale: f32, page: usize, p: Pos2) -> Option<(f32, f32)> {
    let r = rects.get(page)?;
    let scale = layout.pages.get(page).filter(|pg| pg.w > 0.0).map_or(scale, |pg| r.width() / pg.w);
    (scale > 0.0).then(|| ((p.x - r.min.x) / scale, (p.y - r.min.y) / scale))
}

/// Mouse input while a tool is on; `false` (nothing done) when none is, so the caret logic runs.
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) -> bool {
    let Some(tool) = app.canvas.table_tool else { return false };
    if !app.canvas.focused && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        stop(app);
        return true;
    }
    let latest = ui.input(|i| i.pointer.latest_pos());
    if ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer() {
        if let Some(p) = latest
            && let Some((page, x, y)) = crate::canvas::page_at(rects, layout, scale, p)
        {
            app.canvas.table_stroke = Some(PenStroke { page, from: (x, y), to: (x, y) });
        }
        return true;
    }
    let Some(mut st) = app.canvas.table_stroke else { return true };
    if let Some(p) = latest
        && let Some(to) = on_page(rects, layout, scale, st.page, p)
    {
        st.to = to;
        app.canvas.table_stroke = Some(st);
    }
    if ui.input(|i| i.pointer.primary_down()) {
        return true;
    }
    app.canvas.table_stroke = None;
    let params = match tool {
        TableTool::Draw => {
            // A click without a drag draws nothing.
            if (st.to.0 - st.from.0).abs() < 2.0 && (st.to.1 - st.from.1).abs() < 2.0 {
                return true;
            }
            json!({"page": st.page, "x0": st.from.0, "y0": st.from.1, "x1": st.to.0, "y1": st.to.1})
        }
        TableTool::Erase => json!({"page": st.page, "x": st.to.0, "y": st.to.1}),
    };
    if let Err(e) = app.run(tool.id(), params) {
        app.status(e);
    }
    true
}

/// The pointer while a tool is on (a crosshair; the tool's icon is drawn beside it).
pub fn cursor(app: &WordApp, ui: &Ui, resp: &egui::Response) {
    if app.canvas.table_tool.is_some() && (resp.hovered() || app.canvas.table_stroke.is_some()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
}

fn dashed_rect(painter: &egui::Painter, r: Rect, s: Stroke) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&pts, s, 5.0, 3.0));
}

/// The live preview: the rectangle or line being drawn, or the border the eraser would remove;
/// and the tool's icon beside the pointer.
pub fn paint(app: &WordApp, painter: &egui::Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    let Some(tool) = app.canvas.table_tool else { return };
    let to_screen = |page: usize, x: f32, y: f32| -> Option<Pos2> {
        let r = pages.get(page)?;
        let sc = layout.pages.get(page).filter(|pg| pg.w > 0.0).map_or(scale, |pg| r.width() / pg.w);
        Some(pos2(r.min.x + x * sc, r.min.y + y * sc))
    };
    let ink = Stroke::new(1.5, t.accent);
    let hover = painter.ctx().input(|i| i.pointer.hover_pos());
    match (tool, app.canvas.table_stroke) {
        (TableTool::Draw, Some(st)) => match table_draw::stroke(layout, st.page, st.from.0, st.from.1, st.to.0, st.to.1) {
            Some(Kind::Rect(r)) => {
                if let (Some(a), Some(b)) = (to_screen(st.page, r.x, r.y), to_screen(st.page, r.right(), r.bottom())) {
                    painter.rect_stroke(Rect::from_two_pos(a, b), 0.0, ink, egui::StrokeKind::Middle);
                }
            }
            Some(Kind::Vertical { x, y0, y1 }) => {
                if let (Some(a), Some(b)) = (to_screen(st.page, x, y0), to_screen(st.page, x, y1)) {
                    painter.line_segment([a, b], ink);
                }
            }
            Some(Kind::Horizontal { y, x0, x1 }) => {
                if let (Some(a), Some(b)) = (to_screen(st.page, x0, y), to_screen(st.page, x1, y)) {
                    painter.line_segment([a, b], ink);
                }
            }
            // Not a table or a line yet: the drag's box, dashed and dim.
            None => {
                if let (Some(a), Some(b)) = (to_screen(st.page, st.from.0, st.from.1), to_screen(st.page, st.to.0, st.to.1)) {
                    dashed_rect(painter, Rect::from_two_pos(a, b), Stroke::new(1.0, t.text_dim));
                }
            }
        },
        (TableTool::Erase, _) => {
            if let Some(p) = hover
                && let Some((page, x, y)) = crate::canvas::page_at(pages, layout, scale, p)
                && let Some(e) = table_draw::erase_target(layout, page, x, y)
                && let (Some(a), Some(b)) = (to_screen(page, e.from.0, e.from.1), to_screen(page, e.to.0, e.to.1))
            {
                painter.line_segment([a, b], Stroke::new(3.0, Color32::from_rgb(0xD1, 0x3B, 0x3B)));
            }
        }
        _ => {}
    }
    if let Some(p) = hover.filter(|p| pages.iter().any(|r| r.contains(*p))) {
        let r = Rect::from_min_size(p + vec2(8.0, 6.0), vec2(16.0, 16.0));
        let icon = if tool == TableTool::Draw { "pencil" } else { "eraser" };
        crate::icons::paint(painter, r, icon, t.icon, t.accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The buttons toggle the tools (one at a time); coordinates go to the engine.
    #[test]
    fn buttons_toggle_the_pen_and_the_eraser() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        assert_eq!(a.run("table.draw", json!({})).unwrap()["tool"], "table.draw");
        assert_eq!(a.canvas.table_tool, Some(TableTool::Draw));
        assert_eq!(a.run("table.eraser", json!({})).unwrap()["tool"], "table.eraser");
        assert_eq!(a.run("table.eraser", json!({})).unwrap()["tool"], Value::Null);
        assert_eq!(a.canvas.table_tool, None);
        a.run("table.draw", json!({"on": true})).unwrap();
        assert!(stop(&mut a) && a.canvas.table_tool.is_none());
        // With coordinates it is the engine's command (an error here: nothing to split).
        assert!(a.run("table.draw", json!({"page": 0, "x0": 1, "y0": 1, "x1": 2, "y1": 2})).is_err());
        assert!(a.canvas.table_tool.is_none());
    }
}
