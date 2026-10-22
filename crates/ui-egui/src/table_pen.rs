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

/// How near (screen points) the eraser has to be to a border: the same at every zoom, as in Word
/// (down to the smallest page scale, where it is the command's limit, `table_draw::ERASER_MAX_REACH`).
const REACH_PX: f32 = 6.0;
/// An eraser press that moves less than this (screen points) both ways is a click.
const ERASER_CLICK_PX: f32 = 3.0;

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
    let was = app.canvas.table_tool.is_some();
    app.canvas.table_tool = on.then_some(tool);
    app.canvas.table_stroke = None;
    // Only turning a tool on or off touches the gridlines: switching between the pen and the
    // eraser keeps a View Gridlines choice made meanwhile.
    if was != on {
        show_gridlines(app);
    }
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
    crate::table_resize::page_scale(rects, layout, scale, page).map_or(1.0, |s| 1.0 / s)
}

/// The eraser's reach (page points) at `points` page points per screen point: the command's own
/// (`table_draw::eraser_reach`), so the preview lights only what a release erases.
fn eraser_reach(points: f32) -> f32 {
    table_draw::eraser_reach(Some(REACH_PX * points))
}

/// Whether an eraser stroke is a click (it works at its start): it barely moved.
fn erase_click(st: &PenStroke, points: f32) -> bool {
    let near = ERASER_CLICK_PX * points;
    (st.to.0 - st.from.0).abs() < near && (st.to.1 - st.from.1).abs() < near
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
            // A click without a drag draws nothing.
            if (st.to.0 - st.from.0).abs() < 2.0 && (st.to.1 - st.from.1).abs() < 2.0 {
                return true;
            }
            json!({"page": st.page, "x0": st.from.0, "y0": st.from.1, "x1": st.to.0, "y1": st.to.1})
        }
        TableTool::Erase => {
            let points = page_points(rects, layout, scale, st.page);
            let reach = eraser_reach(points);
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

/// The pointer while a tool is on: the drawn eraser for the eraser; for the pen a crosshair
/// (the pen's icon is drawn beside it).
pub fn cursor(app: &WordApp, ui: &Ui, resp: &egui::Response) {
    // `contains_pointer`, not `hovered`: the button still down after Esc dropped a stroke keeps
    // the eraser drawn over the page.
    if app.canvas.table_tool.is_some() && (resp.contains_pointer() || app.canvas.table_stroke.is_some()) {
        if app.canvas.table_tool == Some(TableTool::Erase) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                // Above everything and not clipped to the page area, so a drag over the rulers
                // or the ribbon still shows it.
                let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("wordcraft_table_eraser"));
                eraser_pointer(&ui.ctx().layer_painter(layer), p);
            }
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
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

/// The live preview: the rectangle or line being drawn, or the borders the eraser would remove;
/// and the pen's icon beside the pointer.
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
        // While pressed (only then, as in Word): the drag's thin red rectangle and, thick, every
        // border it would erase; for a click, the border at the press.
        (TableTool::Erase, Some(st)) => {
            let (doc, points) = (&app.session.doc, page_points(pages, layout, scale, st.page));
            let found = if erase_click(&st, points) {
                table_draw::erase_target(doc, layout, st.page, st.from.0, st.from.1, eraser_reach(points)).into_iter().collect()
            } else {
                if let (Some(a), Some(b)) = (to_screen(st.page, st.from.0, st.from.1), to_screen(st.page, st.to.0, st.to.1)) {
                    painter.rect_stroke(Rect::from_two_pos(a, b), 0.0, Stroke::new(1.0, t.red), egui::StrokeKind::Middle);
                }
                table_draw::erase_targets(doc, layout, st.page, st.from, st.to, eraser_reach(points))
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
    if tool == TableTool::Draw
        && let Some(p) = hover.filter(|p| pages.iter().any(|r| r.contains(*p)))
    {
        let r = Rect::from_min_size(p + vec2(8.0, 6.0), vec2(16.0, 16.0));
        crate::icons::paint(painter, r, "pencil", t.icon, t.accent);
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
        // Switching from the eraser to the pen keeps a View Gridlines choice made meanwhile.
        for before in [false, true] {
            a.session.view.table_gridlines = before;
            a.run("table.eraser", json!({"on": true})).unwrap();
            a.run("table.viewGridlines", json!({"value": false})).unwrap();
            a.run("table.draw", json!({})).unwrap();
            assert!(!a.session.view.table_gridlines, "gridlines {before}: the pen keeps them off");
            a.run("table.draw", json!({"on": true})).unwrap();
            assert!(!a.session.view.table_gridlines, "gridlines {before}: the pen on again too");
            assert!(stop(&mut a));
            assert!(!a.session.view.table_gridlines, "gridlines {before}: and after it stops");
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
        // Over the ribbon during the drag the drawn eraser still shows, unclipped.
        let up = pos2(to.x, 40.0);
        h.event(egui::Event::PointerMoved(up));
        h.step();
        let shown = |s: &egui::epaint::ClippedShape| {
            s.clip_rect.contains(up)
                && matches!(&s.shape, egui::Shape::Path(p) if p.fill == crate::icons::ERASER_PINK && p.points.iter().all(|q| q.distance(up) < 30.0))
        };
        assert!(h.output().shapes.iter().any(shown), "the eraser shows over the ribbon");
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
        let drawn_at_to = |s: &egui::epaint::ClippedShape| matches!(&s.shape, egui::Shape::Path(p) if p.fill == crate::icons::ERASER_PINK && p.points.iter().all(|q| q.distance(to) < 30.0));
        assert!(h.output().shapes.iter().any(drawn_at_to), "the drawn eraser stays while the button is still down");
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

    /// What the eraser lights while pressed is what the release erases, at every zoom: a click
    /// or a drag down past the table, five screen points right of it, lights its
    /// outside border and erases it; nine points off lights and erases nothing. Below 18.75%
    /// zoom the six-point reach is more page points than the command used to take (24). The
    /// rows are 72 pt tall, so the lines between them stay out of reach (they would win).
    #[test]
    fn what_the_eraser_lights_is_what_it_erases_at_every_zoom() {
        use crate::table_resize::tests::{button, cell, screen, steps, table_window};
        let gone = |a: &WordApp, r: usize| {
            let t = a.session.doc.body.iter().find_map(|b| b.as_table()).unwrap();
            t.rows[r].cells[2]
                .props
                .borders
                .as_ref()
                .and_then(|b| b.right.as_ref())
                .is_some_and(|b| b.style == wordcraft_doc::props::BorderStyle::None)
        };
        // 20 page points per screen point: the smallest page scale, 0.05.
        assert_eq!(eraser_reach(20.0), table_draw::ERASER_MAX_REACH, "six screen points at the smallest page scale");
        for zoom in [10.0, 15.0, 20.0, 25.0, 100.0] {
            for drag in [false, true] {
                for (off, erases) in [(5.0, true), (9.0, false)] {
                    let case = format!("zoom {zoom}%, drag {drag}, {off} screen points off");
                    let mut h = table_window();
                    h.state_mut().run("table.selectTable", json!({})).unwrap();
                    h.state_mut().run("table.rowHeight", json!({"height": 72})).unwrap();
                    h.state_mut().run("view.zoom", json!({"value": zoom})).unwrap();
                    steps(&mut h, 2);
                    h.state_mut().run("table.eraser", json!({"on": true})).unwrap();
                    let (c, d) = (cell(&mut h, 0, 2), cell(&mut h, 1, 2));
                    let edge = screen(&mut h, c.right(), c.y + c.h / 2.0);
                    // Down past the table, so even at 10% it is a drag, not a click.
                    let end = if drag { screen(&mut h, d.right(), d.bottom() + d.h) } else { edge };
                    let (from, to) = (edge + vec2(off, 0.0), end + vec2(off, 0.0));
                    h.event(egui::Event::PointerMoved(from));
                    h.step();
                    let depth = h.state().session.undo_depth();
                    button(&h, from, true);
                    h.step();
                    h.event(egui::Event::PointerMoved(to));
                    steps(&mut h, 2);
                    let lit = h.output().shapes.iter().any(|s| {
                        matches!(&s.shape, egui::Shape::LineSegment { points, stroke }
                            if stroke.width == 3.0 && points.iter().all(|p| (p.x - edge.x).abs() < 1.0))
                    });
                    button(&h, to, false);
                    steps(&mut h, 2);
                    assert_eq!(lit, erases, "{case}: lit while pressed");
                    assert_eq!(gone(h.state(), 0), erases, "{case}: erased on release");
                    if drag {
                        assert_eq!(gone(h.state(), 1), erases, "{case}: the drag reaches row 2");
                    }
                    assert_eq!(h.state().session.undo_depth(), depth + usize::from(erases), "{case}: one undo step");
                }
            }
        }
    }
}
