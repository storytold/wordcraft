//! Draw Table and Eraser as canvas modes (Insert › Tables, Table Layout › Draw). The buttons
//! toggle a pen or an eraser; while one is on, presses on the page draw (or erase) instead of
//! moving the caret, a preview follows the pointer, and each finished stroke or click runs
//! `table.draw` / `table.eraser` with page coordinates (one undo step each). Esc or the button
//! again turns it off. The geometry decisions are the engine's (`cmd::table_draw`).

use egui::{Pos2, Rect, Stroke, Ui, pos2, vec2};
use serde_json::{Value, json};
use wordcraft_engine::cmd::table_draw::{self, Stroke as Kind};
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::theme::Tokens;

/// How near (screen points) the eraser has to be to a border, or a pen rectangle to a table's
/// bottom to add a row: the same at every zoom, as in Word.
const REACH_PX: f32 = 6.0;
/// An eraser press that moves less than this (screen points) both ways is a click.
const ERASER_CLICK_PX: f32 = 3.0;
/// A pen stroke shorter than this (screen points) both ways draws nothing (Word: about 23
/// pixels at any zoom).
const PEN_MIN_PX: f32 = 23.0;

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
    // View Gridlines while a tool is on is the user's choice: it works at once, and stopping
    // the tool doesn't put back the setting from before (the engine runs the command).
    if id == "table.viewGridlines" {
        app.canvas.table_gridlines = None;
        return None;
    }
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
    show_gridlines(app);
    if on {
        app.canvas.want_focus = true;
        app.status(match tool {
            TableTool::Draw => tl!("Drag a rectangle to draw a table, or a line across cells to split them. Esc stops."),
            TableTool::Erase => tl!("Click a border, or drag across borders, to erase them. Esc stops."),
        });
    }
    Some(Ok(json!({"tool": app.canvas.table_tool.map(TableTool::id)})))
}

/// Turn the tool off (Esc).
pub fn stop(app: &mut WordApp) -> bool {
    let was = app.canvas.table_tool.is_some();
    app.canvas.table_tool = None;
    app.canvas.table_stroke = None;
    show_gridlines(app);
    was
}

/// Esc: during a stroke it drops just the stroke and the tool stays on, as in Word; else it
/// turns the tool off.
pub fn escape(app: &mut WordApp) -> bool {
    app.canvas.table_stroke.take().is_some() || stop(app)
}

/// While a tool is on, the table gridlines show (so hidden borders do), as in Word; when it's
/// off again, View Gridlines is back as it was.
fn show_gridlines(app: &mut WordApp) {
    let view = &mut app.session.view;
    match (app.canvas.table_tool.is_some(), app.canvas.table_gridlines) {
        (true, None) => {
            app.canvas.table_gridlines = Some(view.table_gridlines);
            view.table_gridlines = true;
        }
        (false, Some(was)) => {
            view.table_gridlines = was;
            app.canvas.table_gridlines = None;
        }
        _ => {}
    }
}

/// Page points per screen point on page `page`.
fn page_points(rects: &[Rect], layout: &DocLayout, scale: f32, page: usize) -> f32 {
    let scale = rects.get(page).zip(layout.pages.get(page)).filter(|(_, pg)| pg.w > 0.0).map_or(scale, |(r, pg)| r.width() / pg.w);
    if scale > 0.0 { 1.0 / scale } else { 1.0 }
}

/// Whether an eraser stroke is a click (it works at its start): it barely moved.
fn erase_click(st: &PenStroke, points: f32) -> bool {
    let near = ERASER_CLICK_PX * points;
    (st.to.0 - st.from.0).abs() < near && (st.to.1 - st.from.1).abs() < near
}

/// Whether a pen stroke is too short to draw anything (a click): under about 23 screen points,
/// and never shorter than the shortest line the command takes (at 400 % 23 px is under 6 pt).
fn pen_short(st: &PenStroke, points: f32) -> bool {
    (st.to.0 - st.from.0).abs().max((st.to.1 - st.from.1).abs()) < (PEN_MIN_PX * points).max(table_draw::MIN_LINE)
}

/// The pen preview's thin grey: dark on white paper, light on the dark paper of Switch Modes
/// (whatever the interface theme), so it shows on the page it is drawn on.
fn preview_grey(dark_paper: bool) -> egui::Color32 {
    if dark_paper { egui::Color32::from_gray(0xB0) } else { egui::Color32::from_gray(0x60) }
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
        escape(app);
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
            let points = page_points(rects, layout, scale, st.page);
            // A click (or a stroke too short to count) draws nothing; off every table it puts the
            // pen down, as in Word.
            if pen_short(&st, points) {
                if layout.cell_at(st.page, st.from.0, st.from.1).is_none() {
                    stop(app);
                }
                return true;
            }
            json!({"page": st.page, "x0": st.from.0, "y0": st.from.1, "x1": st.to.0, "y1": st.to.1, "reach": REACH_PX * points})
        }
        TableTool::Erase => {
            let points = page_points(rects, layout, scale, st.page);
            let reach = REACH_PX * points;
            let (x, y) = st.from;
            if !erase_click(&st, points) {
                json!({"page": st.page, "x0": x, "y0": y, "x1": st.to.0, "y1": st.to.1, "reach": reach})
            } else if layout.cell_at(st.page, x, y).is_none() && table_draw::erase_target(&app.session.doc, layout, st.page, x, y, reach).is_none() {
                // A click off every table puts the eraser down, as in Word.
                stop(app);
                return true;
            } else {
                json!({"page": st.page, "x": x, "y": y, "reach": reach})
            }
        }
    };
    if let Err(e) = app.run(tool.id(), params) {
        app.status(e);
    }
    true
}

/// The pointer while a tool is on: our drawn pencil or eraser (the system pointer is hidden).
pub fn cursor(app: &WordApp, ui: &Ui, resp: &egui::Response) {
    let Some(tool) = app.canvas.table_tool else { return };
    if resp.hovered() || app.canvas.table_stroke.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
            match tool {
                TableTool::Draw => pencil_pointer(ui.painter(), p, Tokens::get(ui.ctx()).accent),
                TableTool::Erase => eraser_pointer(ui.painter(), p),
            }
        }
    }
}

/// The pen's pointer, our own drawing in the eraser's style: a white pencil with a grey side and
/// ferrule and an `end` in the theme's accent, as on the Draw Table button, tilted up and to the
/// right from its lead's tip, the point it draws at.
fn pencil_pointer(painter: &egui::Painter, at: Pos2, end: egui::Color32) {
    let (sin, cos) = 55f32.to_radians().sin_cos();
    let (along, across) = (vec2(cos, -sin), vec2(sin, cos));
    let p = |s: f32, w: f32| at + along * s + across * w;
    // The part from `s0` to `s1` along the pencil and from `w0` to `w1` across it (screen points).
    let part = |s0: f32, s1: f32, w0: f32, w1: f32| vec![p(s0, w0), p(s1, w0), p(s1, w1), p(s0, w1)];
    let (ink, white) = (Stroke::new(1.0, egui::Color32::from_gray(0x14)), egui::Color32::WHITE);
    // The top face is 6.8 points wide and its side shows 1.8 below it; the point is 7 long, the
    // lead 2.6; the body ends at 19, the ferrule at 21 and the pencil at 24.
    let (w, side) = (3.4, 5.2);
    let outline = vec![p(0.0, 0.0), p(7.0, -w), p(24.0, -w), p(24.0, side), p(7.0, side)];
    painter.add(egui::Shape::convex_polygon(outline.clone(), white, Stroke::new(2.6, white)));
    painter.add(egui::Shape::convex_polygon(part(7.0, 21.0, w, side), egui::Color32::from_gray(0x78), Stroke::NONE));
    let lead = 2.6 / 7.0;
    painter.add(egui::Shape::convex_polygon(vec![p(0.0, 0.0), p(2.6, -w * lead), p(2.6, side * lead)], egui::Color32::from_gray(0x3C), Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(part(19.0, 21.0, -w, side), egui::Color32::from_gray(0xCD), Stroke::NONE));
    // The end's side is a quarter darker.
    let [r, g, b, _] = end.to_array();
    let shade = |c: u8| u8::try_from(u16::from(c) * 3 / 4).unwrap_or(c);
    painter.add(egui::Shape::convex_polygon(part(21.0, 24.0, -w, w), end, Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(part(21.0, 24.0, w, side), egui::Color32::from_rgb(shade(r), shade(g), shade(b)), Stroke::NONE));
    for s in [7.0, 19.0, 21.0] {
        painter.line_segment([p(s, -w), p(s, side)], ink);
    }
    painter.line_segment([p(7.0, w), p(24.0, w)], ink);
    painter.add(egui::Shape::closed_line(outline, ink));
}

/// The side of the eraser's pink end, darker than its top.
const ERASER_PINK_SIDE: egui::Color32 = egui::Color32::from_rgb(0xC2, 0x45, 0x7A);

/// The eraser's pointer, our own drawing (egui has no cursor images; the system pointer is
/// hidden): an eraser with a pink end and a white sleeve, as on its button, seen a little from
/// the side, tilted up and to the right from the point it works at (the middle of its lower
/// end); outlined in black with a white edge, so it shows on dark paper too.
fn eraser_pointer(painter: &egui::Painter, at: Pos2) {
    let (sin, cos) = 40f32.to_radians().sin_cos();
    let (along, across) = (vec2(cos, -sin), vec2(sin, cos));
    // The part from `s0` to `s1` along the eraser and from `w0` to `w1` across it (screen points).
    let part = |s0: f32, s1: f32, w0: f32, w1: f32| {
        vec![at + along * s0 + across * w0, at + along * s1 + across * w0, at + along * s1 + across * w1, at + along * s0 + across * w1]
    };
    let (ink, white) = (Stroke::new(1.0, egui::Color32::from_gray(0x14)), egui::Color32::WHITE);
    // The top face is 9.6 points wide; its side shows 2.4 points below it; the sleeve starts 9 points up.
    let (top, whole) = (part(0.0, 21.0, -4.8, 4.8), part(0.0, 21.0, -4.8, 7.2));
    painter.add(egui::Shape::convex_polygon(whole.clone(), white, Stroke::new(2.6, white)));
    painter.add(egui::Shape::convex_polygon(part(0.0, 9.0, -4.8, 4.8), crate::icons::ERASER_PINK, Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(part(0.0, 9.0, 4.8, 7.2), ERASER_PINK_SIDE, Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(part(9.0, 21.0, 4.8, 7.2), egui::Color32::from_gray(0xB4), Stroke::NONE));
    painter.line_segment([at + along * 9.0 - across * 4.8, at + along * 9.0 + across * 7.2], ink);
    painter.add(egui::Shape::closed_line(top, ink));
    painter.add(egui::Shape::closed_line(whole, ink));
}

fn dashed_rect(painter: &egui::Painter, r: Rect, s: Stroke) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&pts, s, 5.0, 3.0));
}

/// The live preview: the rectangle or line being drawn, or the borders the eraser would remove.
pub fn paint(app: &WordApp, painter: &egui::Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    let Some(tool) = app.canvas.table_tool else { return };
    let to_screen = |page: usize, x: f32, y: f32| -> Option<Pos2> {
        let r = pages.get(page)?;
        let sc = layout.pages.get(page).filter(|pg| pg.w > 0.0).map_or(scale, |pg| r.width() / pg.w);
        Some(pos2(r.min.x + x * sc, r.min.y + y * sc))
    };
    // The pen previews thin and grey, whatever the pen's style.
    let ink = Stroke::new(1.0, preview_grey(app.session.view.dark_mode));
    match (tool, app.canvas.table_stroke) {
        // Nothing until the stroke is long enough to count; a line across the whole of the cells
        // it splits.
        (TableTool::Draw, Some(st)) if !pen_short(&st, page_points(pages, layout, scale, st.page)) => {
            let reach = REACH_PX * page_points(pages, layout, scale, st.page);
            let segment = |a: (f32, f32), b: (f32, f32)| to_screen(st.page, a.0, a.1).zip(to_screen(st.page, b.0, b.1));
            match table_draw::stroke(layout, st.page, st.from.0, st.from.1, st.to.0, st.to.1, reach) {
                Some(Kind::Rect(r)) => {
                    if let Some((a, b)) = segment((r.x, r.y), (r.right(), r.bottom())) {
                        painter.rect_stroke(Rect::from_two_pos(a, b), 0.0, ink, egui::StrokeKind::Middle);
                    }
                }
                Some(line) => {
                    if let Some((a, b)) = table_draw::line_extent(layout, st.page, line).and_then(|(a, b)| segment(a, b)) {
                        painter.line_segment([a, b], ink);
                    }
                }
                // Not a table or a line: the stroke as drawn, dashed.
                None => {
                    if let Some((a, b)) = segment(st.from, st.to) {
                        dashed_rect(painter, Rect::from_two_pos(a, b), ink);
                    }
                }
            }
        }
        // While pressed (only then, as in Word): the drag's thin red rectangle and, thick, every
        // border it would erase; for a click, the border at the press.
        (TableTool::Erase, Some(st)) => {
            let (doc, points) = (&app.session.doc, page_points(pages, layout, scale, st.page));
            let found = if erase_click(&st, points) {
                table_draw::erase_target(doc, layout, st.page, st.from.0, st.from.1, REACH_PX * points).into_iter().collect()
            } else {
                if let (Some(a), Some(b)) = (to_screen(st.page, st.from.0, st.from.1), to_screen(st.page, st.to.0, st.to.1)) {
                    painter.rect_stroke(Rect::from_two_pos(a, b), 0.0, Stroke::new(1.0, t.red), egui::StrokeKind::Middle);
                }
                table_draw::erase_targets(doc, layout, st.page, st.from, st.to, REACH_PX * points)
            };
            let ink = if app.session.view.dark_mode { egui::Color32::WHITE } else { egui::Color32::BLACK };
            for e in found {
                if let (Some(a), Some(b)) = (to_screen(st.page, e.from.0, e.from.1), to_screen(st.page, e.to.0, e.to.1)) {
                    painter.line_segment([a, b], Stroke::new(3.0, ink));
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At 400 % (a quarter of a point per pixel) 23 px is under the 6 pt the command needs for a
    /// line: such a stroke is a click, not an error in the status bar.
    #[test]
    fn a_stroke_shorter_than_the_shortest_line_is_a_click_at_any_zoom() {
        let st = PenStroke { page: 0, from: (100.0, 100.0), to: (105.9, 100.0) };
        assert!(pen_short(&st, 0.25), "5.9 pt at 400 %");
        assert!(!pen_short(&PenStroke { to: (106.1, 100.0), ..st }, 0.25), "6.1 pt at 400 % is a line");
        assert!(pen_short(&PenStroke { to: (117.0, 100.0), ..st }, 0.75), "17 pt (22.7 px) at 100 %");
    }

    /// The preview stands out from the paper it is drawn on (contrast at least 3:1), white or the
    /// dark paper of Switch Modes, whatever the interface theme.
    #[test]
    fn the_pen_preview_shows_on_white_and_on_dark_paper() {
        let lum = |g: u8| {
            let c = f32::from(g) / 255.0;
            if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let contrast = |a: u8, b: u8| (lum(a).max(lum(b)) + 0.05) / (lum(a).min(lum(b)) + 0.05);
        assert!(contrast(preview_grey(false).r(), 0xFF) >= 3.0, "on white paper");
        assert!(contrast(preview_grey(true).r(), wordcraft_render::DARK_PAPER) >= 3.0, "on dark paper");
    }

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

    /// A tool shows the gridlines and stopping it puts View Gridlines back as it was, unless
    /// the user set View Gridlines while the tool was on: then that choice works at once and
    /// stays after the tool stops.
    #[test]
    fn view_gridlines_while_a_tool_is_on_stays() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
        for tool in ["table.draw", "table.eraser"] {
            for before in [false, true] {
                // None: no click; Some(None): the button (a toggle); Some(Some(v)): set to v.
                for click in [None, Some(None), Some(Some(false)), Some(Some(true))] {
                    let case = format!("{tool}, gridlines {before}, click {click:?}");
                    a.session.view.table_gridlines = before;
                    a.run(tool, json!({"on": true})).unwrap();
                    assert!(a.session.view.table_gridlines, "{case}: the tool shows the gridlines");
                    let after = match click {
                        None => before,
                        Some(v) => {
                            let p = v.map_or(json!({}), |v| json!({ "value": v }));
                            let r = a.run("table.viewGridlines", p).unwrap();
                            let want = v.unwrap_or(false);
                            assert_eq!(r["value"], want, "{case}");
                            assert_eq!(a.session.view.table_gridlines, want, "{case}: at once");
                            want
                        }
                    };
                    assert!(stop(&mut a), "{case}");
                    assert_eq!(a.session.view.table_gridlines, after, "{case}: after the tool stops");
                }
            }
        }
    }

    /// Deleting a table or removing its last cells turns off the eraser.
    #[test]
    fn deleting_table_turns_off_the_eraser() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
        a.run("table.eraser", json!({"on": true})).unwrap();
        assert_eq!(a.canvas.table_tool, Some(TableTool::Erase));
        a.run("table.deleteTable", json!({})).unwrap();
        assert_eq!(a.canvas.table_tool, None);

        // Deleting the only row also removes the table and stops the eraser.
        a.run("insert.table", json!({"rows": 1, "cols": 1})).unwrap();
        a.run("table.eraser", json!({"on": true})).unwrap();
        assert_eq!(a.canvas.table_tool, Some(TableTool::Erase));
        a.run("table.deleteRow", json!({})).unwrap();
        assert_eq!(a.canvas.table_tool, None);
    }

    /// A real mouse drag with the eraser (its drawn pointer over the page): across a row it
    /// merges the row's cells on release, one undo step, and the eraser stays on. Esc during a
    /// drag drops only the drag. The gridlines show while the eraser is on; a click off the
    /// table puts it down and the gridlines back as they were, as in Word.
    #[test]
    fn the_eraser_drags_across_borders_and_esc_drops_only_the_drag() {
        use crate::table_resize::tests::{button, cell, screen, steps, table_window};
        let row_spans = |a: &WordApp, r: usize| -> Vec<usize> {
            let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
            t.rows[r].cells.iter().map(|c| c.span()).collect()
        };
        let mut h = table_window();
        h.state_mut().session.view.table_gridlines = false;
        h.state_mut().run("table.eraser", json!({})).unwrap();
        assert!(h.state().session.view.table_gridlines, "the eraser shows the gridlines");
        let (a, c) = (cell(&mut h, 0, 0), cell(&mut h, 0, 2));
        let (from, to) = (screen(&mut h, a.x + a.w / 2.0, a.y + a.h / 2.0), screen(&mut h, c.x + c.w / 2.0, c.y + c.h / 2.0));
        h.event(egui::Event::PointerMoved(from));
        steps(&mut h, 2);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "the drawn eraser");
        // Pink at the pointer (the ribbon's pink icons are far from it).
        let pink = |s: &egui::epaint::ClippedShape| matches!(&s.shape, egui::Shape::Path(p) if p.fill == crate::icons::ERASER_PINK && p.points.iter().all(|q| q.distance(from) < 30.0));
        assert!(h.output().shapes.iter().any(pink), "the eraser's end is pink, as on its button");
        let depth = h.state().session.undo_depth();
        button(&h, from, true);
        h.step();
        h.event(egui::Event::PointerMoved(to));
        h.step();
        assert!(h.state().canvas.table_stroke.is_some());
        button(&h, to, false);
        steps(&mut h, 2);
        assert_eq!(row_spans(h.state(), 0), [3]);
        assert_eq!(h.state().session.undo_depth(), depth + 1);
        assert_eq!(h.state().canvas.table_tool, Some(TableTool::Erase), "the eraser stays on");
        let (a, c) = (cell(&mut h, 1, 0), cell(&mut h, 1, 2));
        let (from, to) = (screen(&mut h, a.x + a.w / 2.0, a.y + a.h / 2.0), screen(&mut h, c.x + c.w / 2.0, c.y + c.h / 2.0));
        h.event(egui::Event::PointerMoved(from));
        h.step();
        button(&h, from, true);
        h.step();
        h.event(egui::Event::PointerMoved(to));
        h.step();
        let esc = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        h.event(esc);
        h.step();
        assert!(h.state().canvas.table_stroke.is_none(), "Esc drops the drag");
        assert_eq!(h.state().canvas.table_tool, Some(TableTool::Erase), "and leaves the eraser on");
        button(&h, to, false);
        steps(&mut h, 2);
        assert_eq!(row_spans(h.state(), 1), [1, 1, 1], "nothing erased");
        let below = screen(&mut h, a.x + a.w / 2.0, a.bottom() + 60.0);
        h.event(egui::Event::PointerMoved(below));
        h.step();
        button(&h, below, true);
        h.step();
        button(&h, below, false);
        steps(&mut h, 2);
        assert_eq!(h.state().canvas.table_tool, None, "a click off the table puts the eraser down");
        assert!(!h.state().session.view.table_gridlines, "and the gridlines are back as they were");
    }

    /// The pen (its drawn pointer over the page): a stroke shorter than Word's minimum draws
    /// nothing, a long one across a cell splits its row, and a click off the table puts the pen
    /// down, as in Word.
    #[test]
    fn the_pen_ignores_short_strokes_and_a_click_off_the_table_ends_it() {
        use crate::table_resize::tests::{H, button, cell, screen, steps, table_window};
        fn drag(h: &mut H, from: Pos2, to: Pos2) {
            h.event(egui::Event::PointerMoved(from));
            h.step();
            button(h, from, true);
            h.step();
            h.event(egui::Event::PointerMoved(to));
            h.step();
            button(h, to, false);
            steps(h, 2);
        }
        let rows = |a: &WordApp| a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap().rows.len();
        let mut h = table_window();
        h.state_mut().run("table.draw", json!({})).unwrap();
        let a = cell(&mut h, 0, 0);
        let y = a.y + a.h / 2.0;
        let over = screen(&mut h, a.x + 20.0, y);
        h.event(egui::Event::PointerMoved(over));
        steps(&mut h, 2);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "the drawn pencil");
        // The accent at the pointer (the ribbon's accent is far from it).
        let accent = Tokens::get(&h.ctx).accent;
        let end = |s: &egui::epaint::ClippedShape| matches!(&s.shape, egui::Shape::Path(p) if p.fill == accent && p.points.iter().all(|q| q.distance(over) < 30.0));
        assert!(h.output().shapes.iter().any(end), "the pencil's end is the accent, as on the Draw Table button");
        let (from, to) = (screen(&mut h, a.x + 20.0, y), screen(&mut h, a.x + 30.0, y));
        drag(&mut h, from, to);
        assert_eq!(rows(h.state()), 2, "too short to draw");
        assert_eq!(h.state().canvas.table_tool, Some(TableTool::Draw));
        let (from, to) = (screen(&mut h, a.x + 10.0, y), screen(&mut h, a.right() - 10.0, y));
        drag(&mut h, from, to);
        assert_eq!(rows(h.state()), 3, "the line splits the first row");
        let last = cell(&mut h, 2, 0);
        let below = screen(&mut h, a.x + 20.0, last.bottom() + 60.0);
        drag(&mut h, below, below);
        assert_eq!(h.state().canvas.table_tool, None, "a click off the table puts the pen down");
    }
}
