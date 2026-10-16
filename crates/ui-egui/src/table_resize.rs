//! Resizing tables with the mouse, like Word: over a column border (or the left edge) the pointer
//! turns into a column-resize arrow, over a row's bottom border a row-resize arrow; over a table
//! a small handle shows past its bottom-right corner. Dragging shows a dashed preview of the new border
//! (or outline); releasing runs `table.resizeDrag` with page coordinates, one undo step. Shift
//! while releasing a column border changes only the column on its left; a double-click on a
//! column border fits the column to its contents (`table.fitColumn`). The geometry decisions
//! are the engine's (`cmd::table_resize`).

use egui::{Pos2, Rect, Stroke, Ui, pos2};
use serde_json::json;
use wordcraft_engine::cmd::table_resize::{self, Edge, Plan, ResizeTarget, Share};
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::theme::Tokens;

/// How near (screen pixels) the pointer has to be to a border to grab it.
pub(crate) const REACH_PX: f32 = 4.0;

/// A table border (or the handle) being dragged.
#[derive(Clone, Debug)]
pub(crate) struct ResizeDrag {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
    target: ResizeTarget,
    /// Started on a ruler marker, where Shift works the other way round, as on Word's ruler:
    /// without it a column border changes only the column on its left (the columns right of it
    /// move along), with it the neighbour makes up the difference.
    ruler: bool,
}

impl ResizeDrag {
    /// Who makes up a column border's move, by the keys held now: Ctrl the columns on the
    /// right; Shift nobody on the page but the neighbour on a ruler marker, which works the other
    /// way round; else the neighbour on the page and nobody on the ruler.
    fn share(&self, ctx: &egui::Context) -> Share {
        let keys = ctx.input(|i| i.modifiers);
        if keys.command {
            Share::Proportional
        } else if keys.shift != self.ruler {
            Share::LeftOnly
        } else {
            Share::Neighbour
        }
    }

    /// What the drag so far makes of the table, with the keys held now.
    fn plan(&self, ctx: &egui::Context) -> Plan {
        self.target.plan(self.to.0 - self.from.0, self.to.1 - self.from.1, self.share(ctx))
    }

    /// Where the dragged column border (page x) or row bottom (page y) is now; `None` for the
    /// handle.
    fn line_now(&self, plan: &Plan) -> Option<f32> {
        match self.target.edge {
            Edge::Column(i) => Some(self.target.planned_x(plan, plan.cells.first().map(|(r, _)| *r), i)),
            Edge::Row(r) => planned_bottom(&self.target, plan, r),
            Edge::Corner => None,
        }
    }
}

/// Page y of the bottom of `tg`'s row `row` with `plan` applied (rows above it that change move
/// it too); `None` when the row isn't drawn on the page.
fn planned_bottom(tg: &ResizeTarget, plan: &Plan, row: usize) -> Option<f32> {
    let bottom = tg.rows.iter().find(|r| r.row == row)?.bottom;
    let grown: f32 = tg
        .rows
        .iter()
        .filter(|r| r.row <= row)
        .filter_map(|r| plan.heights.iter().find(|(i, _)| *i == r.row).map(|(_, h)| h - (r.bottom - r.top)))
        .sum();
    Some(bottom + grown)
}

/// Screen pixels per page point on `page`.
pub(crate) fn page_scale(rects: &[Rect], layout: &DocLayout, scale: f32, page: usize) -> Option<f32> {
    let r = rects.get(page)?;
    let sc = layout.pages.get(page).filter(|pg| pg.w > 0.0).map_or(scale, |pg| r.width() / pg.w);
    (sc > 0.0 && sc.is_finite()).then_some(sc)
}

pub(crate) fn to_screen(rects: &[Rect], layout: &DocLayout, scale: f32, page: usize, x: f32, y: f32) -> Option<Pos2> {
    let r = rects.get(page)?;
    let sc = page_scale(rects, layout, scale, page)?;
    Some(pos2(r.min.x + x * sc, r.min.y + y * sc))
}

/// Whether table resizing is in reach: not while a pen, the eraser, an object drag, a text drag
/// or a press on a table's selection area or move handle (`table_mouse`) owns the pointer, nor
/// while editing a header or footer from the body (or back).
pub(crate) fn available(app: &WordApp) -> bool {
    app.canvas.table_tool.is_none()
        && app.canvas.table_drag.is_none()
        && app.canvas.ink.is_none()
        && app.canvas.obj_drag.is_none()
        && !app.canvas.drag_selecting()
        && crate::ink::cursor(app).is_none()
}

/// The border or handle under screen point `p`, if any.
fn target_at(app: &WordApp, layout: &DocLayout, rects: &[Rect], scale: f32, p: Pos2) -> Option<(usize, f32, f32, ResizeTarget)> {
    let (page, x, y) = crate::canvas::page_at(rects, layout, scale, p)?;
    let sc = page_scale(rects, layout, scale, page)?;
    let t = table_resize::resize_target(layout, &app.session.doc, page, x, y, REACH_PX / sc)?;
    Some((page, x, y, t))
}

/// Mouse input: a press on a border starts a drag (in the selected cells' rows only, when it's a
/// border of theirs), moving updates it, releasing applies it with the keys held then (Ctrl: the
/// columns right of the border share the change). `false` (nothing done) when the pointer isn't
/// on a border, so the caret logic runs.
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, rects: &[Rect], layout: &DocLayout, scale: f32) -> bool {
    let latest = ui.input(|i| i.pointer.latest_pos());
    if let Some(mut d) = app.canvas.table_resize.take() {
        if let Some(p) = latest
            && let Some(r) = rects.get(d.page)
            && let Some(sc) = page_scale(rects, layout, scale, d.page)
        {
            d.to = ((p.x - r.min.x) / sc, (p.y - r.min.y) / sc);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return true;
        }
        if ui.input(|i| i.pointer.primary_down()) {
            app.canvas.table_resize = Some(d);
            return true;
        }
        // A click without a move changes nothing; a double-click on a column border fits the
        // column left of it to its contents.
        if (d.to.0 - d.from.0).abs() < 0.5 && (d.to.1 - d.from.1).abs() < 0.5 {
            if resp.double_clicked() && matches!(d.target.edge, Edge::Column(_)) {
                let sc = page_scale(rects, layout, scale, d.page).unwrap_or(1.0);
                let params = json!({"page": d.page, "x": d.from.0, "y": d.from.1, "reach": REACH_PX / sc});
                if let Err(e) = app.run_keep_view("table.fitColumn", params) {
                    app.status(e);
                }
            }
            return true;
        }
        let share = d.share(ui.ctx());
        let sc = page_scale(rects, layout, scale, d.page).unwrap_or(1.0);
        let params = json!({
            "page": d.page, "x": d.from.0, "y": d.from.1, "toX": d.to.0, "toY": d.to.1,
            "shift": share == Share::LeftOnly, "ctrl": share == Share::Proportional, "reach": REACH_PX / sc,
        });
        if let Err(e) = app.run_keep_view("table.resizeDrag", params) {
            app.status(e);
        }
        return true;
    }
    if !(available(app) && ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer()) {
        return false;
    }
    let Some(p) = latest else { return false };
    let Some((page, x, y, mut target)) = target_at(app, layout, rects, scale, p) else { return false };
    target.limit_to_selection(&app.session, y);
    app.canvas.table_resize = Some(ResizeDrag { page, from: (x, y), to: (x, y), target, ruler: false });
    true
}

/// Starts dragging the table border at page point (`x`, `y`) on `page` from a ruler marker: it
/// goes on as a drag on the page (`pointer`: dashed preview, Escape cancels, one undo step), but
/// a column border changes only the column on its left.
pub(crate) fn grab(app: &mut WordApp, layout: &DocLayout, page: usize, x: f32, y: f32) {
    if !available(app) {
        return;
    }
    if let Some(mut target) = table_resize::resize_target(layout, &app.session.doc, page, x, y, 1.0) {
        target.limit_to_selection(&app.session, y);
        app.canvas.table_resize = Some(ResizeDrag { page, from: (x, y), to: (x, y), target, ruler: true });
    }
}

/// The drag of one of the caret's table's borders on `page`, if one is going on.
fn caret_drag(app: &WordApp, page: usize) -> Option<&ResizeDrag> {
    let caret = app.session.sel.focus.path.cell().map(|(tp, _, _)| tp);
    app.canvas
        .table_resize
        .as_ref()
        .filter(|d| d.page == page && d.target.story == app.session.sel.focus.story && caret.as_ref() == Some(&d.target.table))
}

/// Whether the rulers show the caret's table's column widths: Alt is held while one of its
/// column borders (or its left edge) is dragged on `page`, as in Word.
pub(crate) fn measuring(app: &WordApp, ctx: &egui::Context, page: usize) -> bool {
    ctx.input(|i| i.modifiers.alt) && caret_drag(app, page).is_some_and(|d| matches!(d.target.edge, Edge::Column(_)))
}

/// Where the caret's table's column border drawn at page x `at` (`column`; in the caret's row),
/// or row bottom drawn at page y `at`, is while one of its borders is dragged on `page`: the
/// rulers' markers follow the drag.
pub(crate) fn moved(app: &WordApp, ctx: &egui::Context, page: usize, column: bool, at: f32) -> f32 {
    let Some(d) = caret_drag(app, page) else { return at };
    let (tg, plan) = (&d.target, d.plan(ctx));
    let near = |v: f32| (v - at).abs() < 0.5;
    if column {
        let row = app.session.sel.focus.path.cell().map(|(_, r, _)| r);
        tg.cols.iter().position(|x| near(*x)).map_or(at, |line| tg.planned_x(&plan, row, line))
    } else {
        tg.rows.iter().find(|r| near(r.bottom)).and_then(|r| planned_bottom(tg, &plan, r.row)).unwrap_or(at)
    }
}

/// The resize arrow over a border or the handle, and while dragging.
pub fn cursor(app: &WordApp, ui: &Ui, resp: &egui::Response, layout: &DocLayout, rects: &[Rect], scale: f32) {
    let edge = match &app.canvas.table_resize {
        Some(d) => Some(d.target.edge),
        None if resp.hovered() && available(app) => {
            ui.input(|i| i.pointer.latest_pos()).and_then(|p| target_at(app, layout, rects, scale, p)).map(|(_, _, _, t)| t.edge)
        }
        None => None,
    };
    let icon = match edge {
        Some(Edge::Column(_)) => egui::CursorIcon::ResizeColumn,
        Some(Edge::Row(_)) => egui::CursorIcon::ResizeRow,
        Some(Edge::Corner) => egui::CursorIcon::ResizeNwSe,
        None => return,
    };
    ui.ctx().set_cursor_icon(icon);
}

/// The handle past the bottom-right corner of the table under the pointer, and while dragging,
/// a dashed preview of the new border or outline.
pub fn paint(app: &WordApp, painter: &egui::Painter, t: &Tokens, layout: &DocLayout, rects: &[Rect], scale: f32) {
    let s = |page: usize, x: f32, y: f32| to_screen(rects, layout, scale, page, x, y);
    let dash = Stroke::new(1.0, t.text);
    if let Some(d) = &app.canvas.table_resize {
        let tg = &d.target;
        let plan = d.plan(painter.ctx());
        let (left, top) = (tg.left(), tg.top());
        let right = plan.widths.as_ref().map_or(tg.right(), |w| left + w.iter().sum::<f32>());
        let bottom = top + tg.rows.iter().map(|r| plan.heights.iter().find(|(i, _)| *i == r.row).map_or(r.bottom - r.top, |(_, h)| *h)).sum::<f32>();
        let line = |a: Option<Pos2>, b: Option<Pos2>| {
            if let (Some(a), Some(b)) = (a, b) {
                painter.extend(egui::Shape::dashed_line(&[a, b], dash, 3.0, 3.0));
            }
        };
        // The dragged column border's x, or row bottom's y.
        let at = d.line_now(&plan).unwrap_or_default();
        match tg.edge {
            Edge::Column(_) => {
                // Down the rows it moves in (the selected cells'), else the whole table.
                let rows: Vec<_> = tg.rows.iter().filter(|r| plan.cells.is_empty() || plan.cells.iter().any(|(c, _)| *c == r.row)).collect();
                let (y0, y1) =
                    (rows.iter().map(|r| r.top).fold(f32::INFINITY, f32::min), rows.iter().map(|r| r.bottom).fold(f32::NEG_INFINITY, f32::max));
                if y0 <= y1 {
                    line(s(d.page, at, y0), s(d.page, at, y1));
                }
            }
            Edge::Row(_) => line(s(d.page, left, at), s(d.page, tg.right(), at)),
            Edge::Corner => {
                if let (Some(a), Some(b)) = (s(d.page, left, top), s(d.page, right, bottom)) {
                    let r = Rect::from_two_pos(a, b);
                    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                    painter.extend(egui::Shape::dashed_line(&pts, dash, 3.0, 3.0));
                }
            }
        }
        return;
    }
    if !available(app) {
        return;
    }
    // The handle shows while the pointer is over the table (or on the handle itself).
    let Some(p) = painter.ctx().input(|i| i.pointer.hover_pos()) else { return };
    let Some((page, x, y)) = crate::canvas::page_at(rects, layout, scale, p) else { return };
    let Some(sc) = page_scale(rects, layout, scale, page) else { return };
    let Some(tg) = table_resize::table_under(layout, &app.session.doc, page, x, y, REACH_PX / sc) else { return };
    let h = tg.handle();
    if let (Some(a), Some(b)) = (s(page, h.x, h.y), s(page, h.right(), h.bottom())) {
        let r = Rect::from_two_pos(a, b);
        painter.rect_filled(r, 0.0, egui::Color32::WHITE);
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Screen ↔ page conversion survives a page list that doesn't match the layout.
    #[test]
    fn conversions_survive_missing_pages() {
        let layout = DocLayout::default();
        assert!(page_scale(&[], &layout, 1.0, 0).is_none());
        assert!(to_screen(&[], &layout, 1.0, 3, 1.0, 1.0).is_none());
        let rects = [Rect::from_min_size(pos2(10.0, 20.0), egui::vec2(100.0, 200.0))];
        assert_eq!(page_scale(&rects, &layout, 2.0, 0), Some(2.0));
        assert_eq!(to_screen(&rects, &layout, 2.0, 0, 5.0, 5.0), Some(pos2(20.0, 30.0)));
        assert!(page_scale(&rects, &layout, 0.0, 0).is_none());
    }

    pub(crate) type H = egui_kittest::Harness<'static, WordApp>;

    /// The whole window with a 2×3 table whose first cell's text is selected.
    pub(crate) fn table_window() -> H {
        table_window_stepping(0.25)
    }

    /// The same, each step `dt` seconds apart (short enough for a double-click: 1/60).
    pub(crate) fn table_window_stepping(dt: f32) -> H {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("insert.table", json!({"rows": 2, "cols": 3})).unwrap();
        a.run("text.insert", json!({"text": "Product"})).unwrap();
        a.run("select.text", json!({"text": "Product"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(dt).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        h
    }

    /// Cell (`row`, `cell`)'s rect in page points.
    pub(crate) fn cell(h: &mut H, row: usize, cell: usize) -> wordcraft_geom::Rect {
        h.state_mut().session.layout().pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                wordcraft_layout::Placed::Cell { rect, row: r, cell: c, .. } if *r == row && *c == cell => Some(*rect),
                _ => None,
            })
            .unwrap()
    }

    pub(crate) fn screen(h: &mut H, x: f32, y: f32) -> Pos2 {
        crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap()
    }

    pub(crate) fn button(h: &H, at: Pos2, pressed: bool) {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }

    pub(crate) fn steps(h: &mut H, n: usize) {
        for _ in 0..n {
            h.step();
        }
    }

    /// A real mouse drag of a column border: the resize arrow on hover, a preview and no change
    /// while dragging, then one undo step on release, the selection untouched.
    #[test]
    fn dragging_a_column_border_with_the_mouse_is_one_undo_step() {
        let mut h = table_window();
        let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
        let sel = h.state().session.sel.clone();
        let depth = h.state().session.undo_depth();
        let start = screen(&mut h, a.right(), a.y + a.h / 2.0);
        h.event(egui::Event::PointerMoved(start));
        steps(&mut h, 2);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::ResizeColumn, "the resize arrow over the border");
        button(&h, start, true);
        h.step();
        // 30 page points to the right, in a few moves.
        let end = screen(&mut h, a.right() + 30.0, a.y + a.h / 2.0);
        for k in 1..=5 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
            h.step();
        }
        assert!(h.state().canvas.table_resize.is_some(), "a drag is in progress");
        assert_eq!(h.state().session.undo_depth(), depth, "nothing is written while dragging");
        assert_eq!(cell(&mut h, 0, 0), a, "the table waits for the release");
        button(&h, end, false);
        steps(&mut h, 3);
        assert!(h.state().canvas.table_resize.is_none());
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
        let (a1, b1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
        assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
        assert!((b1.w - (b.w - 30.0)).abs() < 1.0, "{b:?} → {b1:?}");
        assert_eq!(h.state().session.sel, sel, "the selection is kept");
        assert!(h.state_mut().session.undo());
        assert_eq!(cell(&mut h, 0, 0), a);
    }

    /// #1738: releasing a column-border drag leaves the view where it is, not scrolled back to
    /// the caret; typing afterwards still follows the caret.
    #[test]
    fn dragging_a_column_border_keeps_the_view_where_it_is() {
        let mut h = table_window();
        let a = cell(&mut h, 0, 0);
        let start = screen(&mut h, a.right(), a.y + a.h / 2.0);
        let end = screen(&mut h, a.right() + 30.0, a.y + a.h / 2.0);
        h.event(egui::Event::PointerMoved(start));
        steps(&mut h, 2);
        button(&h, start, true);
        h.step();
        h.event(egui::Event::PointerMoved(end));
        steps(&mut h, 2);
        h.state_mut().canvas.scroll_to_caret = false;
        let depth = h.state().session.undo_depth();
        button(&h, end, false);
        h.step();
        assert_eq!(h.state().session.undo_depth(), depth + 1, "the drag was applied");
        assert!(!h.state().canvas.scroll_to_caret, "the resize scrolled to the caret");
        h.state_mut().run("text.insert", json!({"text": "x"})).unwrap();
        assert!(h.state().canvas.scroll_to_caret, "typing still brings the caret into view");
    }

    /// The zone of the ruler marker named `label` nearest to screen point `near`.
    fn ruler_marker(h: &H, label: &str, near: Pos2) -> Rect {
        use egui_kittest::kittest::Queryable;
        h.query_all_by_label(label).map(|n| n.rect()).min_by(|p, q| p.center().distance(near).total_cmp(&q.center().distance(near))).expect(label)
    }

    /// A real mouse drag from `from` to `to` (screen points), in a few moves.
    fn drag(h: &mut H, from: Pos2, to: Pos2) {
        h.event(egui::Event::PointerMoved(from));
        h.step();
        button(h, from, true);
        h.step();
        for k in 1..=5 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 5.0)));
            h.step();
        }
        button(h, to, false);
        steps(h, 3);
    }

    /// #49: with the caret in the table, a column marker on the horizontal ruler drags its border,
    /// in one undo step; only the column left of it changes and the columns right of it move
    /// along, as on Word's ruler. The marker follows the drag.
    #[test]
    fn a_column_marker_on_the_ruler_moves_the_columns_right_of_it() {
        let mut h = table_window();
        let (a, b, c) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
        let depth = h.state().session.undo_depth();
        let near = screen(&mut h, a.right(), a.y);
        let start = ruler_marker(&h, "Table Column Width", near).center();
        assert!((start.x - near.x).abs() < 0.5, "a marker over the first column border");
        let end = start + egui::vec2(screen(&mut h, a.right() + 30.0, a.y).x - near.x, 0.0);
        h.event(egui::Event::PointerMoved(start));
        h.step();
        button(&h, start, true);
        h.step();
        for k in 1..=5 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
            h.step();
        }
        assert!((ruler_marker(&h, "Table Column Width", end).center().x - end.x).abs() < 1.0, "the marker follows the pointer");
        assert_eq!(h.state().session.undo_depth(), depth, "nothing is written while dragging");
        button(&h, end, false);
        steps(&mut h, 3);
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
        let (a1, b1, c1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
        assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
        assert!((b1.w - b.w).abs() < 0.5 && (c1.w - c.w).abs() < 0.5, "the columns right of it keep their widths");
        assert!((c1.right() - (c.right() + 30.0)).abs() < 1.0, "and move along, so the table is wider");
    }

    /// With Shift a column marker on the ruler works the other way round, as on Word's ruler: the
    /// neighbour on the right makes up the difference, so the table keeps its width.
    #[test]
    fn a_shift_drag_on_the_ruler_trades_width_with_the_neighbour() {
        let mut h = table_window();
        let (a, b, c) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
        let near = screen(&mut h, a.right(), a.y);
        let start = ruler_marker(&h, "Table Column Width", near).center();
        let end = start + egui::vec2(screen(&mut h, a.right() + 30.0, a.y).x - near.x, 0.0);
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
        drag(&mut h, start, end);
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        let (a1, b1, c1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
        assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
        assert!((b1.w - (b.w - 30.0)).abs() < 1.0, "{b:?} → {b1:?}");
        assert!((c1.w - c.w).abs() < 0.5 && (c1.right() - c.right()).abs() < 1.0, "the table keeps its width");
    }

    /// #49: a row marker on the vertical ruler drags that row's bottom border: the row gets taller,
    /// in one undo step.
    #[test]
    fn a_row_marker_on_the_ruler_sets_the_row_height() {
        let mut h = table_window();
        let a = cell(&mut h, 0, 0);
        let depth = h.state().session.undo_depth();
        let near = screen(&mut h, a.x, a.bottom());
        let start = ruler_marker(&h, "Table Row Height", near).center();
        assert!((start.y - near.y).abs() < 0.5, "a marker at the first row's bottom");
        let end = start + egui::vec2(0.0, screen(&mut h, a.x, a.bottom() + 20.0).y - near.y);
        drag(&mut h, start, end);
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step for the drag");
        let a1 = cell(&mut h, 0, 0);
        assert!((a1.h - (a.h + 20.0)).abs() < 1.0, "{a:?} → {a1:?}");
    }

    /// #49: in a cell the ruler's indents are the cell's: the left-indent marker sits where the
    /// cell's text starts (its box less its margins), and dragging it 18 points indents the
    /// paragraph 18 points.
    #[test]
    fn in_a_cell_the_ruler_measures_indents_from_its_text() {
        let mut h = table_window();
        let b = cell(&mut h, 0, 1);
        // Into the second cell: its text starts far from the page's left margin.
        let inside = screen(&mut h, b.x + b.w / 2.0, b.y + b.h / 2.0);
        h.event(egui::Event::PointerMoved(inside));
        h.step();
        button(&h, inside, true);
        h.step();
        button(&h, inside, false);
        steps(&mut h, 3);
        assert_eq!(h.state().session.sel.focus.path.cell().map(|(_, r, cl)| (r, cl)), Some((0, 1)));
        let layout = h.state_mut().session.layout();
        let (left, w) = crate::table_ruler::text_area(h.state(), &layout, 0).unwrap();
        assert!((left - (b.x + 5.4)).abs() < 0.01 && (w - (b.w - 10.8)).abs() < 0.01, "the cell less its margins: {left}, {w}");
        // The left-indent marker is at the text's start, just above the ruler's bottom edge.
        let near = screen(&mut h, left, b.y);
        let bar_bottom = ruler_marker(&h, "Table Column Width", near).bottom();
        let start = egui::pos2(near.x, bar_bottom - 3.0);
        let end = start + egui::vec2(screen(&mut h, left + 18.0, b.y).x - near.x, 0.0);
        drag(&mut h, start, end);
        let focus = h.state().session.sel.focus.clone();
        assert_eq!(h.state().session.doc.para_at(&focus).and_then(|p| p.props.indent_left), Some(18.0));
    }

    /// With Ctrl held, the change of a dragged column border is shared by the columns right of
    /// it, so the table keeps its width.
    #[test]
    fn a_ctrl_drag_shares_the_change_among_the_columns_right_of_it() {
        let mut h = table_window();
        let (a, b, c) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
        let y = a.y + a.h / 2.0;
        let (start, end) = (screen(&mut h, a.right(), y), screen(&mut h, a.right() + 30.0, y));
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
        drag(&mut h, start, end);
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        let (a1, b1, c1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1), cell(&mut h, 0, 2));
        assert!((a1.w - (a.w + 30.0)).abs() < 1.0, "{a:?} → {a1:?}");
        assert!((b1.w - (b.w - 15.0)).abs() < 1.0 && (c1.w - (c.w - 15.0)).abs() < 1.0, "{b1:?} {c1:?}");
        assert!((c1.right() - c.right()).abs() < 1.0, "the table keeps its width");
    }

    /// The table's left edge, under the resize arrow just outside it, drags the table in; the
    /// first column makes up the difference. The horizontal ruler has a marker on it too.
    #[test]
    fn the_left_edge_drags_the_table_in_and_has_a_ruler_marker() {
        let mut h = table_window();
        let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
        let y = a.y + a.h / 2.0;
        let (start, end) = (screen(&mut h, a.x - 1.5, y), screen(&mut h, a.x + 20.0, y));
        h.event(egui::Event::PointerMoved(start));
        steps(&mut h, 2);
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::ResizeColumn, "the resize arrow on the left edge");
        drag(&mut h, start, end);
        let (a1, b1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
        assert!((a1.x - (a.x + 21.5)).abs() < 1.0 && (a1.right() - a.right()).abs() < 0.5, "{a:?} → {a1:?}");
        assert!((b1.x - b.x).abs() < 0.5 && (b1.w - b.w).abs() < 0.5, "{b:?} → {b1:?}");
        let near = screen(&mut h, a1.x, a1.y);
        let marker = ruler_marker(&h, "Table Column Width", near).center();
        assert!((marker.x - near.x).abs() < 0.5, "a marker on the left edge");
        let back = marker - egui::vec2(near.x - screen(&mut h, a.x, a1.y).x, 0.0);
        drag(&mut h, marker, back);
        assert!((cell(&mut h, 0, 0).x - a.x).abs() < 1.0, "the marker drags it back out");
    }

    /// Alt while a column border is dragged shows each column's width on the ruler, as it is
    /// going to be.
    #[test]
    fn alt_shows_the_column_widths_while_dragging() {
        let mut h = table_window();
        let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
        let y = a.y + a.h / 2.0;
        let (start, end) = (screen(&mut h, a.right(), y), screen(&mut h, a.right() + 18.0, y));
        h.event(egui::Event::PointerMoved(start));
        h.step();
        button(&h, start, true);
        h.step();
        h.event(egui::Event::PointerMoved(end));
        h.step();
        let widths = |h: &mut H| {
            let layout = h.state_mut().session.layout();
            crate::table_ruler::widths(h.state(), &h.ctx, &layout, 0)
        };
        assert!(widths(&mut h).is_empty(), "not without Alt");
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
        h.step();
        let w = widths(&mut h);
        assert_eq!(w.len(), 3, "{w:?}");
        assert!((w[0].1 - (a.w + 18.0)).abs() < 0.5 && (w[1].1 - (b.w - 18.0)).abs() < 0.5, "{w:?}");
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        button(&h, end, false);
        steps(&mut h, 3);
        assert!(widths(&mut h).is_empty(), "gone with the drag");
    }

    /// A cell selected with a click along its left edge: dragging its right border with the
    /// mouse moves it in that cell's row only (#962).
    #[test]
    fn a_selected_cells_border_drags_in_its_row_only() {
        let mut h = table_window();
        let (above, b, c) = (cell(&mut h, 0, 1), cell(&mut h, 1, 1), cell(&mut h, 1, 2));
        let y = b.y + b.h / 2.0;
        let at = screen(&mut h, b.x + 4.5, y);
        h.event(egui::Event::PointerMoved(at));
        h.step();
        button(&h, at, true);
        h.step();
        button(&h, at, false);
        steps(&mut h, 2);
        assert_eq!(h.state().session.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (1, 1))), "the cell is selected");
        let (start, end) = (screen(&mut h, b.right(), y), screen(&mut h, b.right() + 30.0, y));
        drag(&mut h, start, end);
        let (b1, c1) = (cell(&mut h, 1, 1), cell(&mut h, 1, 2));
        assert!((b1.w - (b.w + 30.0)).abs() < 1.0 && (c1.w - (c.w - 30.0)).abs() < 1.0, "{b1:?} {c1:?}");
        assert!((cell(&mut h, 0, 1).right() - above.right()).abs() < 0.5, "the first row keeps its border");
    }

    /// Clicks inside a cell still place the caret (the border doesn't steal them), and Esc drops a
    /// drag without changing anything.
    #[test]
    fn clicks_in_cells_still_move_the_caret_and_esc_cancels_a_drag() {
        let mut h = table_window();
        let c = cell(&mut h, 1, 1);
        let at = screen(&mut h, c.x + c.w / 2.0, c.y + c.h / 2.0);
        h.event(egui::Event::PointerMoved(at));
        h.step();
        button(&h, at, true);
        h.step();
        button(&h, at, false);
        steps(&mut h, 3);
        let focus = &h.state().session.sel.focus;
        assert_eq!(focus.path.cell().map(|(_, r, cl)| (r, cl)), Some((1, 1)), "the caret went into the clicked cell");
        let table = h.state().session.doc.body.iter().find_map(|b| b.as_table()).cloned();
        let depth = h.state().session.undo_depth();
        let a = cell(&mut h, 0, 0);
        let start = screen(&mut h, a.x + a.w / 2.0, a.bottom());
        h.event(egui::Event::PointerMoved(start));
        h.step();
        button(&h, start, true);
        h.step();
        let end = start + egui::vec2(0.0, 40.0);
        h.event(egui::Event::PointerMoved(end));
        h.step();
        assert!(h.state().canvas.table_resize.is_some(), "the row border is being dragged");
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        h.step();
        button(&h, end, false);
        steps(&mut h, 3);
        assert_eq!(h.state().session.undo_depth(), depth, "Esc cancelled it");
        assert_eq!(h.state().session.doc.body.iter().find_map(|b| b.as_table()).cloned(), table);
    }

    /// A double-click on a column border fits the column left of it to its text, the next
    /// column keeping its width and moving along (the table narrows, as in Word), in one undo
    /// step.
    #[test]
    fn double_clicking_a_column_border_fits_the_column() {
        let mut h = table_window_stepping(1.0 / 60.0);
        let (a, b) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
        let depth = h.state().session.undo_depth();
        let at = screen(&mut h, a.right(), a.y + a.h / 2.0);
        h.event(egui::Event::PointerMoved(at));
        h.step();
        for _ in 0..2 {
            button(&h, at, true);
            h.step();
            button(&h, at, false);
            h.step();
        }
        steps(&mut h, 2);
        let (a1, b1) = (cell(&mut h, 0, 0), cell(&mut h, 0, 1));
        assert!(a1.w < a.w - 40.0, "{a:?} → {a1:?}");
        assert!((b1.w - b.w).abs() < 1.0 && (b1.x - b.x - (a1.w - a.w)).abs() < 1.0, "the next column keeps its width and moves along");
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one undo step");
    }
}
