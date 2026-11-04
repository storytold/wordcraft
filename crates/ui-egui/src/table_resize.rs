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
    // The caret's table shows it wherever the pointer is, as in Word.
    for (page, _, h) in caret_handles(app, painter, layout, rects) {
        if let Some(h) = h
            && let (Some(a), Some(b)) = (s(page, h.x, h.y), s(page, h.right(), h.bottom()))
        {
            let r = Rect::from_two_pos(a, b);
            painter.rect_filled(r, 0.0, egui::Color32::WHITE);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
        }
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

/// The move and resize handles (page points) of the caret's table on the pages in view: Word
/// shows them while the caret is in a table, not only under the pointer.
pub(crate) fn caret_handles(
    app: &WordApp,
    painter: &egui::Painter,
    layout: &DocLayout,
    rects: &[Rect],
) -> Vec<(usize, Option<wordcraft_geom::Rect>, Option<wordcraft_geom::Rect>)> {
    let focus = &app.session.sel.focus;
    let Some((tp, _, _)) = focus.path.cell() else { return Vec::new() };
    let clip = painter.clip_rect();
    rects
        .iter()
        .enumerate()
        .filter(|(_, r)| r.intersects(clip))
        .map(|(page, _)| {
            let (m, z) = wordcraft_engine::cmd::table_mouse::caret_handles(layout, &app.session.doc, focus.story, &tp, page);
            (page, m, z)
        })
        .filter(|(_, m, z)| m.is_some() || z.is_some())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests;
