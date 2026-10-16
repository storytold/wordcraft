//! The rest of Word's table mouse (resizing is `table_resize`): over a selection area — just
//! above a column, left of a row, along a cell's left edge — the pointer becomes a selection
//! arrow and a click selects the column, row or cell (a drag goes on selecting); left of a row
//! border or above a column border a "+" button shows, and a click adds a row or column there;
//! over a table a move handle shows past its top-left corner: a click selects the table, a drag
//! moves it, with a dashed outline of where it goes. What each spot is and does is the engine's
//! (`cmd::table_mouse`); this sends the presses as its commands, in page points.

use egui::{Color32, Pos2, Rect, Shape, Stroke, Ui, vec2};
use serde_json::json;
use wordcraft_engine::cmd::table_mouse::{self, Spot, Target};
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::table_resize::{REACH_PX, page_scale, to_screen};
use crate::theme::Tokens;

/// A press on a table's selection area or move handle, while the button is down.
#[derive(Clone, Debug)]
pub(crate) struct TableDrag {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
    /// Where the selection was last made to (a drag from a selection area selects as it goes).
    sent: (f32, f32),
    /// The resize reach at the press (page points).
    reach: f32,
    target: Target,
}

/// Whether the table mouse is in reach: when resizing is, and not while a border is dragged.
fn available(app: &WordApp) -> bool {
    crate::table_resize::available(app) && app.canvas.table_resize.is_none()
}

/// The spot under screen point `p`: page, page point, resize reach and the spot.
fn target_at(app: &WordApp, layout: &DocLayout, rects: &[Rect], scale: f32, p: Pos2) -> Option<(usize, f32, f32, f32, Target)> {
    let (page, x, y) = crate::canvas::page_at(rects, layout, scale, p)?;
    let reach = REACH_PX / page_scale(rects, layout, scale, page)?;
    let t = table_mouse::target_at(layout, &app.session.doc, app.session.sel.focus.story, page, x, y, reach)?;
    Some((page, x, y, reach, t))
}

/// Mouse input: a press on a selection area selects (and a drag goes on selecting), on an insert
/// button inserts, on the move handle starts a move (released without moving, it selects the
/// table). `false` (nothing done) when the pointer isn't on any of them, so the caret logic runs.
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) -> bool {
    let latest = ui.input(|i| i.pointer.latest_pos());
    if let Some(mut d) = app.canvas.table_drag.take() {
        if let Some(p) = latest
            && let Some(r) = rects.get(d.page)
            && let Some(sc) = page_scale(rects, layout, scale, d.page)
        {
            d.to = ((p.x - r.min.x) / sc, (p.y - r.min.y) / sc);
        }
        let down = ui.input(|i| i.pointer.primary_down());
        let at = json!({"page": d.page, "x": d.from.0, "y": d.from.1, "reach": d.reach, "toX": d.to.0, "toY": d.to.1});
        if d.target.spot == Spot::Handle {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                return true;
            }
            if down {
                app.canvas.table_drag = Some(d);
                return true;
            }
            // Released: a drag moves the table (one undo step), a click selects it.
            let moved = (d.to.0 - d.from.0).abs() >= 0.5 || (d.to.1 - d.from.1).abs() >= 0.5;
            let id = if moved && table_mouse::movable(&d.target.geom) { "table.moveDrag" } else { "table.selectAt" };
            if let Err(e) = app.run_keep_view(id, at) {
                app.status(e);
            }
            return true;
        }
        // A selection area: the selection follows the pointer until the button goes up. A
        // double-click along a cell's left edge selects its row, as in Word.
        if down {
            if d.to != d.sent {
                d.sent = d.to;
                let _ = app.run_keep_view("table.selectAt", at);
            }
            app.canvas.table_drag = Some(d);
        } else if matches!(d.target.spot, Spot::Cell(..))
            && resp.double_clicked()
            && let Err(e) = app.run("table.selectRow", json!({}))
        {
            app.status(e);
        }
        return true;
    }
    if !(available(app) && ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer()) {
        return false;
    }
    let Some(p) = latest else { return false };
    let Some((page, x, y, reach, target)) = target_at(app, layout, rects, scale, p) else { return false };
    let at = json!({"page": page, "x": x, "y": y, "reach": reach});
    match target.spot {
        Spot::InsertRow(_) | Spot::InsertColumn(_) => {
            if let Err(e) = app.run_keep_view("table.insertAt", at) {
                app.status(e);
            }
        }
        spot => {
            // A selection area selects at once; the move handle waits for the release.
            if spot != Spot::Handle
                && let Err(e) = app.run_keep_view("table.selectAt", at)
            {
                app.status(e);
            }
            app.canvas.table_drag = Some(TableDrag { page, from: (x, y), to: (x, y), sent: (x, y), reach, target });
        }
    }
    true
}

/// The pointer: the move arrows on the handle, the plain arrow on an insert button; over a
/// selection area the system pointer hides and `paint` draws the selection arrow.
pub fn cursor(app: &WordApp, ui: &Ui, resp: &egui::Response, layout: &DocLayout, rects: &[Rect], scale: f32) {
    let spot = match &app.canvas.table_drag {
        Some(d) => Some(d.target.spot),
        None if resp.hovered() && available(app) => {
            ui.input(|i| i.pointer.latest_pos()).and_then(|p| target_at(app, layout, rects, scale, p)).map(|(.., t)| t.spot)
        }
        None => None,
    };
    let icon = match spot {
        Some(Spot::Handle) => egui::CursorIcon::Move,
        Some(Spot::InsertRow(_) | Spot::InsertColumn(_)) => egui::CursorIcon::Default,
        Some(Spot::Column(_) | Spot::Row(_) | Spot::Cell(..)) => egui::CursorIcon::None,
        None => return,
    };
    ui.ctx().set_cursor_icon(icon);
}

/// The move handle of the table under the pointer; an insert button under the pointer with the
/// border it inserts at; the selection arrow over a selection area; and while the handle is
/// dragged, a dashed outline of where the table goes.
pub fn paint(app: &WordApp, painter: &egui::Painter, t: &Tokens, layout: &DocLayout, rects: &[Rect], scale: f32) {
    let hover = painter.ctx().input(|i| i.pointer.hover_pos());
    let dark = app.session.view.dark_mode;
    if let Some(d) = &app.canvas.table_drag {
        match d.target.spot {
            Spot::Handle => {
                let g = &d.target.geom;
                let (x, y) = table_mouse::moved_to(layout, d.page, g, d.to.0 - d.from.0, d.to.1 - d.from.1);
                let (w, h) = (g.right() - g.left(), g.bottom() - g.top());
                if let (Some(a), Some(b)) = (to_screen(rects, layout, scale, d.page, x, y), to_screen(rects, layout, scale, d.page, x + w, y + h)) {
                    let r = Rect::from_two_pos(a, b);
                    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                    painter.extend(Shape::dashed_line(&pts, Stroke::new(1.0, t.text), 3.0, 3.0));
                }
            }
            spot => {
                if let Some(p) = hover {
                    arrow(painter, p, spot, dark);
                }
            }
        }
        return;
    }
    if !available(app) {
        return;
    }
    let Some(p) = hover else { return };
    let Some((page, x, y)) = crate::canvas::page_at(rects, layout, scale, p) else { return };
    let Some(sc) = page_scale(rects, layout, scale, page) else { return };
    let (doc, story, reach) = (&app.session.doc, app.session.sel.focus.story, REACH_PX / sc);
    let s = |x: f32, y: f32| to_screen(rects, layout, scale, page, x, y);
    if let Some(g) = table_mouse::handle_table(layout, doc, story, page, x, y, reach) {
        let h = table_mouse::handle(&g);
        if let (Some(a), Some(b)) = (s(h.x, h.y), s(h.right(), h.bottom())) {
            move_handle(painter, Rect::from_two_pos(a, b), t);
        }
    }
    let Some(target) = table_mouse::target_at(layout, doc, story, page, x, y, reach) else { return };
    match table_mouse::insert_mark(&target) {
        Some(m) => {
            if let (Some(c), Some(a), Some(b)) = (s(m.center.0, m.center.1), s(m.from.0, m.from.1), s(m.to.0, m.to.1)) {
                insert_button(painter, c, a, b, sc, t);
            }
        }
        None if target.spot != Spot::Handle => arrow(painter, p, target.spot, dark),
        None => {}
    }
}

/// The move handle: a small square with arrows in four directions.
fn move_handle(painter: &egui::Painter, r: Rect, t: &Tokens) {
    painter.rect_filled(r, 1.0, Color32::WHITE);
    painter.rect_stroke(r, 1.0, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
    let (c, k) = (r.center(), r.width() * 0.34);
    let stroke = Stroke::new(1.0, t.text);
    painter.line_segment([c - vec2(k, 0.0), c + vec2(k, 0.0)], stroke);
    painter.line_segment([c - vec2(0.0, k), c + vec2(0.0, k)], stroke);
    let head = k * 0.5;
    for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
        let (tip, side) = (c + d * k, vec2(-d.y, d.x) * head);
        painter.add(Shape::convex_polygon(vec![tip, tip - d * head + side, tip - d * head - side], t.text, Stroke::NONE));
    }
}

/// An insert button: a circled "+" out from the table, joined to the border it inserts at,
/// which is marked from end to end.
fn insert_button(painter: &egui::Painter, c: Pos2, from: Pos2, to: Pos2, sc: f32, t: &Tokens) {
    painter.line_segment([c, from], Stroke::new(1.0, t.accent));
    painter.line_segment([from, to], Stroke::new(2.0, t.accent));
    let r = (4.5 * sc).clamp(4.0, 12.0);
    painter.circle_filled(c, r, Color32::WHITE);
    painter.circle_stroke(c, r, Stroke::new(1.0, t.accent));
    let (k, plus) = (r * 0.55, Stroke::new(1.5, t.accent));
    painter.line_segment([c - vec2(k, 0.0), c + vec2(k, 0.0)], plus);
    painter.line_segment([c - vec2(0.0, k), c + vec2(0.0, k)], plus);
}

/// The selection arrow at the pointer, its tip on it (the system pointer is hidden there): down
/// over a column; up and right, filled, in a cell; up and right, hollow, beside a row.
fn arrow(painter: &egui::Painter, tip: Pos2, spot: Spot, dark: bool) {
    use std::f32::consts::{FRAC_PI_4, PI};
    let turn = if matches!(spot, Spot::Column(_)) { PI } else { FRAC_PI_4 };
    let (fill, edge) = arrow_colors(spot, dark);
    // An arrow pointing up from its tip at (0, 0), turned clockwise by `turn` (screen points).
    let (sin, cos) = turn.sin_cos();
    let at = |(x, y): (f32, f32)| tip + vec2(x * cos - y * sin, x * sin + y * cos);
    let head = [(0.0, 0.0), (5.5, 6.5), (-5.5, 6.5)].map(at);
    let shaft = [(2.0, 6.5), (2.0, 13.0), (-2.0, 13.0), (-2.0, 6.5)].map(at);
    let outline = [(0.0, 0.0), (5.5, 6.5), (2.0, 6.5), (2.0, 13.0), (-2.0, 13.0), (-2.0, 6.5), (-5.5, 6.5)].map(at);
    painter.add(Shape::convex_polygon(head.to_vec(), fill, Stroke::NONE));
    painter.add(Shape::convex_polygon(shaft.to_vec(), fill, Stroke::NONE));
    painter.add(Shape::closed_line(outline.to_vec(), Stroke::new(1.0, edge)));
}

/// The arrow's fill and edge: black on white over a column or cell (white on the dark paper's
/// grey in dark mode, where black would vanish), white on black beside a row.
fn arrow_colors(spot: Spot, dark: bool) -> (Color32, Color32) {
    match spot {
        Spot::Column(_) | Spot::Cell(..) if dark => (Color32::WHITE, Color32::from_gray(wordcraft_render::DARK_PAPER)),
        Spot::Column(_) | Spot::Cell(..) => (Color32::BLACK, Color32::WHITE),
        _ => (Color32::WHITE, Color32::BLACK),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table_resize::tests::{H, button, cell, screen, steps, table_window, table_window_stepping};

    /// On the dark paper the column and cell arrows are drawn light, so they stay visible; on
    /// white paper they stay black, and the hollow row arrow is the same on both.
    #[test]
    fn the_selection_arrow_is_light_on_dark_paper() {
        let dark_paper = Color32::from_gray(wordcraft_render::DARK_PAPER);
        assert_eq!(arrow_colors(Spot::Column(0), false), (Color32::BLACK, Color32::WHITE));
        assert_eq!(arrow_colors(Spot::Cell(0, 0), false), (Color32::BLACK, Color32::WHITE));
        assert_eq!(arrow_colors(Spot::Column(0), true), (Color32::WHITE, dark_paper));
        assert_eq!(arrow_colors(Spot::Cell(0, 0), true), (Color32::WHITE, dark_paper));
        assert_eq!(arrow_colors(Spot::Row(0), false), (Color32::WHITE, Color32::BLACK));
        assert_eq!(arrow_colors(Spot::Row(0), true), (Color32::WHITE, Color32::BLACK));
    }

    fn click(h: &mut H, at: Pos2) {
        h.event(egui::Event::PointerMoved(at));
        h.step();
        button(h, at, true);
        h.step();
        button(h, at, false);
        steps(h, 2);
    }

    /// Above a column the pointer is the drawn selection arrow; a click selects the column's
    /// cells (no undo step), and a click in a cell's text still places the caret.
    #[test]
    fn a_click_above_a_column_selects_it() {
        let mut h = table_window();
        let b = cell(&mut h, 0, 1);
        let depth = h.state().session.undo_depth();
        let at = screen(&mut h, b.x + b.w / 2.0, b.y - 3.0);
        h.event(egui::Event::PointerMoved(at));
        steps(&mut h, 2);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "the drawn selection arrow");
        click(&mut h, at);
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (1, 1))));
        assert_eq!(h.state().session.undo_depth(), depth);
        assert!(!crate::mini_toolbar::visible(h.state()), "no mini toolbar over the table's selection areas");
        let c = cell(&mut h, 1, 2);
        let text = screen(&mut h, c.x + c.w / 2.0, c.y + c.h / 2.0);
        click(&mut h, text);
        assert!(h.state().session.cell_range().is_none());
        assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((1, 2)));
    }

    /// A drag down the row selection areas selects every row it passes, as it goes.
    #[test]
    fn dragging_down_the_row_areas_selects_rows() {
        let mut h = table_window();
        let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 1, 0));
        let start = screen(&mut h, a.x - 10.0, a.y + a.h / 2.0);
        let end = screen(&mut h, a.x - 10.0, b.y + b.h / 2.0);
        h.event(egui::Event::PointerMoved(start));
        h.step();
        button(&h, start, true);
        h.step();
        assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((0, 0)));
        h.event(egui::Event::PointerMoved(end));
        h.step();
        assert_eq!(h.state().session.cell_range().map(|r| r.rows), Some((0, 1)), "the selection follows the pointer");
        button(&h, end, false);
        steps(&mut h, 2);
        assert!(h.state().canvas.table_drag.is_none());
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 2))));
    }

    /// An ordinary drag from one cell's text into another's selects the rectangle of whole cells
    /// between them as the pointer goes: no undo step, no mini toolbar over the cells.
    #[test]
    fn dragging_across_cells_selects_their_rectangle() {
        let mut h = table_window();
        let (a, d) = (cell(&mut h, 0, 1), cell(&mut h, 1, 2));
        let depth = h.state().session.undo_depth();
        let start = screen(&mut h, a.x + a.w / 2.0, a.y + a.h / 2.0);
        let end = screen(&mut h, d.x + d.w / 2.0, d.y + d.h / 2.0);
        h.event(egui::Event::PointerMoved(start));
        h.step();
        button(&h, start, true);
        h.step();
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 4.0)));
            h.step();
        }
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (1, 2))), "the selection follows the pointer");
        button(&h, end, false);
        steps(&mut h, 2);
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (1, 2))));
        assert_eq!(h.state().session.undo_depth(), depth);
        assert!(!crate::mini_toolbar::visible(h.state()));
    }

    /// The "+" left of a row border adds a row there and selects it, in one undo step; over it
    /// the pointer is the plain arrow.
    #[test]
    fn the_plus_button_adds_a_row() {
        let mut h = table_window();
        let a = cell(&mut h, 0, 0);
        let depth = h.state().session.undo_depth();
        let at = screen(&mut h, a.x - 10.0, a.bottom());
        h.event(egui::Event::PointerMoved(at));
        steps(&mut h, 2);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Default);
        click(&mut h, at);
        let rows = h.state().session.doc.body.iter().find_map(|b| b.as_table()).map(|t| t.rows.len());
        assert_eq!(rows, Some(3));
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (0, 2))), "the new row is selected");
        assert_eq!(h.state().session.undo_depth(), depth + 1);
    }

    /// A double-click along a cell's left edge selects its row, as in Word.
    #[test]
    fn a_double_click_by_a_cells_edge_selects_its_row() {
        let mut h = table_window_stepping(1.0 / 60.0);
        let c = cell(&mut h, 1, 1);
        let at = screen(&mut h, c.x + 4.5, c.y + c.h / 2.0);
        h.event(egui::Event::PointerMoved(at));
        h.step();
        button(&h, at, true);
        h.step();
        button(&h, at, false);
        h.step();
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (1, 1))), "a click selects the cell");
        button(&h, at, true);
        h.step();
        button(&h, at, false);
        steps(&mut h, 2);
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (0, 2))), "a double-click its row");
    }

    /// Dragging the move handle: the move arrows over it, nothing changes while dragging, then
    /// the table floats where it's dropped, in one undo step.
    #[test]
    fn dragging_the_move_handle_moves_the_table() {
        let mut h = table_window();
        let a = cell(&mut h, 0, 0);
        let depth = h.state().session.undo_depth();
        // The handle's centre, past the top-left corner.
        let (hx, hy) = (a.x - 6.0, a.y - 6.0);
        let start = screen(&mut h, hx, hy);
        h.event(egui::Event::PointerMoved(start));
        steps(&mut h, 2);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Move);
        button(&h, start, true);
        h.step();
        let end = screen(&mut h, hx + 40.0, hy + 30.0);
        for k in 1..=5 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
            h.step();
        }
        assert!(h.state().canvas.table_drag.is_some(), "a move is in progress");
        assert_eq!(cell(&mut h, 0, 0), a, "the table waits for the release");
        button(&h, end, false);
        steps(&mut h, 3);
        let moved = cell(&mut h, 0, 0);
        assert!((moved.x - (a.x + 40.0)).abs() < 1.0 && (moved.y - (a.y + 30.0)).abs() < 1.0, "{a:?} → {moved:?}");
        assert_eq!(h.state().session.undo_depth(), depth + 1);
    }

    /// #1738: dropping the move handle leaves the view where it is.
    #[test]
    fn dropping_the_move_handle_keeps_the_view_where_it_is() {
        let mut h = table_window();
        let a = cell(&mut h, 0, 0);
        let (hx, hy) = (a.x - 6.0, a.y - 6.0);
        let start = screen(&mut h, hx, hy);
        h.event(egui::Event::PointerMoved(start));
        steps(&mut h, 2);
        button(&h, start, true);
        h.step();
        let end = screen(&mut h, hx + 40.0, hy + 30.0);
        h.event(egui::Event::PointerMoved(end));
        steps(&mut h, 2);
        h.state_mut().canvas.scroll_to_caret = false;
        let depth = h.state().session.undo_depth();
        button(&h, end, false);
        h.step();
        assert_eq!(h.state().session.undo_depth(), depth + 1, "the table moved");
        assert!(!h.state().canvas.scroll_to_caret, "the move scrolled to the caret");
    }
}
