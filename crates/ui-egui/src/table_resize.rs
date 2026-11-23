//! Resizing tables with the mouse, like Word: over a column border (or the left edge) the pointer
//! turns into a column-resize arrow, over a row's bottom border a row-resize arrow; over a table
//! a small handle shows past its bottom-right corner. Dragging shows a dashed preview of the new border
//! (or outline); releasing runs `table.resizeDrag` with page coordinates, one undo step. Shift
//! while releasing a column border changes only the column on its left; a double-click on a
//! column border fits the column to its contents (`table.fitColumn`). The geometry decisions
//! are the engine's (`cmd::table_resize`).
//!
//! [`TableBorders`] is the canvas control: the border under the pointer is one part (the engine finds it
//! at the pointer, so the list of parts is the pointer's), and the part takes the press, the cursor and the
//! drag. A ruler drag ends in [`ruler_params`] and shows the same preview.

use craft::behavior::kurbo::Point;
use craft::behavior::{CursorIcon, HitLevel, HitPriority, Metrics, PartKey, PointerButton, PressPhase, Sense};
use craft::keymap::Mods;
use craft::pages::ui::{PageBody, PlacedPage};
use craft::ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Pointer as _, Props, Remember as _, Role, Show as _};
use craft::ui::primitives::{PaintShape, StrokeWidth};
use craft::ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Ui, Vec2, pos2};
use serde_json::{Value, json};
use wordcraft_engine::cmd::table_resize::{self, Edge, Plan, ResizeTarget, Share};
use wordcraft_layout::DocLayout;

use crate::WordApp;

/// How near (screen pixels) the pointer has to be to a border to grab it.
pub(crate) const REACH_PX: f32 = 4.0;

/// The part of a column border between two columns.
const COLUMN_BORDER: PartKey = PartKey::new("column_border");
/// The part of a row's bottom border.
const ROW_BORDER: PartKey = PartKey::new("row_border");
/// The part of the table's left edge, which drags the table in.
const LEFT_EDGE: PartKey = PartKey::new("left_edge");
/// The part of the resize handle past the bottom-right corner.
const CORNER: PartKey = PartKey::new("corner");

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
    /// Who makes up a column border's move, by the keys held: Ctrl the columns on the
    /// right; Shift nobody on the page but the neighbour on a ruler marker, which works the other
    /// way round; else the neighbour on the page and nobody on the ruler. The one place the key rule lives.
    fn share(&self, mods: Mods) -> Share {
        if mods.cmd {
            Share::Proportional
        } else if mods.shift != self.ruler {
            Share::LeftOnly
        } else {
            Share::Neighbour
        }
    }

    /// What the drag so far makes of the table, with the keys `mods` held.
    fn plan(&self, mods: Mods) -> Plan {
        self.target.plan(self.to.0 - self.from.0, self.to.1 - self.from.1, self.share(mods))
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

    /// Where grid line `line` (the table's left edge is 0) stands on the page while a ruler drag of a column border is
    /// in progress, with the keys `mods` held; `None` when no such drag runs. The ruler's markers follow it: the
    /// framework ruler draws each marker where the app says its border is.
    pub(crate) fn ruler_line_x(&self, mods: Mods, line: usize) -> Option<f32> {
        if !self.ruler || !matches!(self.target.edge, Edge::Column(_)) {
            return None;
        }
        let plan = self.plan(mods);
        Some(self.target.planned_x(&plan, plan.cells.first().map(|(r, _)| *r), line))
    }

    /// Whether the pointer has moved from where it went down.
    fn moved(&self) -> bool {
        (self.to.0 - self.from.0).abs() >= 0.5 || (self.to.1 - self.from.1).abs() >= 0.5
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

/// Whether table resizing is in reach: not while a text drag, an object gesture or a press on a
/// table's selection area or move handle (`table_mouse`) owns the pointer. The pen and ink tools'
/// regions decide the rest.
pub(crate) fn available(app: &WordApp) -> bool {
    // PLAN: assumed `CanvasState::object_gesture(&self) -> Option<_>` (canvas-w11, cv.A10)
    !crate::table_mouse::dragging(app) && !app.canvas.drag_selecting() && app.canvas.object_gesture().is_none()
}

/// The page whose shown part holds screen point `p`.
pub(crate) fn page_at(pages: &[PlacedPage], p: Pos2) -> Option<&PlacedPage> {
    pages.iter().find(|placed| {
        let (top, bottom) = placed.band;
        placed.scale.is_finite()
            && placed.scale > 0.0
            && placed.rect.contains(p)
            && p.y >= placed.rect.min.y + top
            && p.y <= placed.rect.min.y + bottom
    })
}

/// The page point of screen point `p` on `placed`, as the engine takes it.
pub(crate) fn page_point(placed: &PlacedPage, p: Pos2) -> (f32, f32) {
    let at = placed.to_page(p);
    (at.x as f32, at.y as f32)
}

/// Screen pixels of reach as page points on `page`; the reach itself when the page is not in view.
pub(crate) fn reach_on(pages: &[PlacedPage], page: usize) -> f32 {
    pages.iter().find(|placed| placed.page == page).map_or(REACH_PX, |placed| REACH_PX / (placed.scale as f32).max(f32::EPSILON))
}

/// The page point (`x`, `y`) as a screen position on `placed`.
pub(crate) fn screen_of(placed: &PlacedPage, x: f32, y: f32) -> Pos2 {
    placed.to_screen(Point::new(f64::from(x), f64::from(y)))
}

/// The modifiers held now, for a preview that follows the keys while a drag runs.
pub(crate) fn mods_now(ctx: &egui::Context) -> Mods {
    let held = ctx.input(|i| i.modifiers);
    Mods { cmd: held.command, alt: held.alt, shift: held.shift, ..Mods::NONE }
}

/// The part kind of a border.
fn key_of(edge: Edge) -> PartKey {
    match edge {
        Edge::Column(0) => LEFT_EDGE,
        Edge::Column(_) => COLUMN_BORDER,
        Edge::Row(_) => ROW_BORDER,
        Edge::Corner => CORNER,
    }
}

/// The cursor over a border.
fn resize_cursor(edge: Edge) -> CursorIcon {
    match edge {
        Edge::Column(_) => CursorIcon::ResizeColumn,
        Edge::Row(_) => CursorIcon::ResizeVertical,
        Edge::Corner => CursorIcon::ResizeNwSe,
    }
}

/// The screen rectangle of page-point area `a` on `placed`, cut to the part of the page that shows; `None` when
/// none of it shows.
pub(crate) fn screen_area(placed: &PlacedPage, a: &wordcraft_geom::Rect) -> Option<Rect> {
    let r = Rect::from_two_pos(screen_of(placed, a.x, a.y), screen_of(placed, a.right(), a.bottom()));
    let (top, bottom) = placed.band;
    let shown = Rect::from_min_max(pos2(placed.rect.min.x, placed.rect.min.y + top), pos2(placed.rect.max.x, placed.rect.min.y + bottom));
    let r = r.intersect(shown);
    r.is_positive().then_some(r)
}

/// The slot value of a press this control owns: a drag held in `CanvasState::table_resize` is this control's
/// (a ruler drag also writes it, and has no press here). `index` names the press's own part again while it is held.
#[derive(Clone, Copy, Debug, Default)]
struct Grab {
    index: u64,
}

/// The colours of the borders' handle and preview.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(crate) struct TableBordersStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The dashed preview of the new border or outline.
    preview: StyleColor,
    /// The resize handle's face.
    face: StyleColor,
    /// The resize handle's edge.
    edge: StyleColor,
}

/// Table borders on the page: the grabbable border under the pointer, the drag of it and its preview, and the
/// resize handles. A press on a border drags it (in the selected cells' rows only, when it is a border of
/// theirs); releasing applies it with the keys held then (Ctrl: the columns right of the border share the
/// change); Escape drops it without a command; a double-click on a column border fits the column.
///
/// The engine finds the border at the pointer, so the control reads the pointer's position (a held fact, the
/// same on every pass of a frame) to declare the one part there. A press applies only through the part's own
/// phases, each reported once, so a second pass in a frame sees none of it again.
#[derive(Component, Props)]
#[component(canvas_control)]
pub(crate) struct TableBorders<'a> {
    #[common]
    common: Common,
    app: &'a mut WordApp,
    layout: &'a DocLayout,
    pages: &'a [PlacedPage],
    /// Where the pointer is over the page view, `None` when it is elsewhere or under another layer.
    pointer: Option<Pos2>,
    /// The modifiers held, which the preview of a drag follows.
    mods: Mods,
    /// The parts declared this pass: kind, place in the list, page and the border (none for a held press's part).
    declared: Vec<(PartKey, u64, usize, Option<ResizeTarget>)>,
}

impl<'a> TableBorders<'a> {
    /// The borders of the tables on `pages`, laid out as `layout`, with the pointer at `pointer` and `mods` held.
    pub(crate) fn new(app: &'a mut WordApp, layout: &'a DocLayout, pages: &'a [PlacedPage], pointer: Option<Pos2>, mods: Mods) -> Self {
        TableBorders { common: Common::new(), app, layout, pages, pointer, mods, declared: Vec::new() }
    }

    /// A press went down at `origin` on `target`, the border of part `index` on `page`: the drag starts there.
    fn press(&mut self, cx: &mut Cx<'_, Self>, origin: Pos2, page: usize, index: u64, mut target: ResizeTarget) {
        let Some(placed) = self.pages.iter().find(|placed| placed.page == page).copied() else { return };
        let from = page_point(&placed, origin);
        target.limit_to_selection(&self.app.session, from.1);
        self.app.canvas.table_resize = Some(ResizeDrag { page, from, to: from, target, ruler: false });
        *cx.press_slot::<Grab>() = Some(Grab { index });
    }

    /// The drag follows the pointer to `pos` while this control holds the press.
    fn follow(&mut self, cx: &mut Cx<'_, Self>, pos: Pos2) {
        if cx.press_slot::<Grab>().is_none() {
            return;
        }
        let Some(d) = self.app.canvas.table_resize.as_mut() else { return };
        if let Some(placed) = self.pages.iter().find(|placed| placed.page == d.page) {
            d.to = page_point(placed, pos);
        }
    }

    /// The button went up at `pos` after `clicks` presses in a row: a click without a move changes nothing (a
    /// double-click on a column border fits the column left of it to its contents); a drag applies.
    fn release(&mut self, cx: &mut Cx<'_, Self>, clicks: u32, pos: Pos2, mods: Mods) {
        self.follow(cx, pos);
        if cx.press_slot::<Grab>().take().is_none() {
            return;
        }
        let Some(d) = self.app.canvas.table_resize.take() else { return };
        let reach = reach_on(self.pages, d.page);
        let (id, params) = if d.moved() {
            let share = d.share(mods);
            let params = json!({
                "page": d.page, "x": d.from.0, "y": d.from.1, "toX": d.to.0, "toY": d.to.1,
                "shift": share == Share::LeftOnly, "ctrl": share == Share::Proportional, "reach": reach,
            });
            ("table.resizeDrag", params)
        } else if clicks == 2 && matches!(d.target.edge, Edge::Column(_)) {
            ("table.fitColumn", json!({"page": d.page, "x": d.from.0, "y": d.from.1, "reach": reach}))
        } else {
            return;
        };
        if let Err(e) = self.app.run_keep_view(id, params) {
            self.app.status(e);
        }
    }

    /// Escape (or another end without a release) drops the drag with no command.
    fn cancel(&mut self, cx: &mut Cx<'_, Self>) {
        if cx.press_slot::<Grab>().take().is_some() {
            self.app.canvas.table_resize = None;
        }
    }
}

/// The face and edge of a resize handle at `r`.
fn resize_handle(draw: &craft::ui::primitives::Draw<'_>, r: Rect, style: &TableBordersStyle) {
    let shape = PaintShape::Rect(r, CornerRadius::ZERO);
    draw.fill(&shape, style.face);
    draw.stroke(&shape, StrokeWidth::Thin, style.edge);
}

impl Component for TableBorders<'_> {
    const ROLE: Role = Role::None;
    const OVERLAYS: bool = false;
    const STOPS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = TableBordersStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: TableBordersStyleKey) -> TableBordersStyle {
        let overlay = &cx.tokens.color.overlay;
        TableBordersStyle {
            metrics: Metrics::new(),
            preview: cx.color(overlay.stroke),
            face: cx.color(overlay.handle_fill),
            edge: cx.color(overlay.handle_stroke),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// One part per border the engine lists for the tables on the pages in view, at its place in the list, or the
    /// one being dragged (so the press keeps its part while the pointer is anywhere). A border ranks above the
    /// selection areas of `table_mouse`, as the engine's `target_at` answers none where a border is.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.declared.clear();
        let priority = HitPriority::new(HitLevel::Handle, 1);
        if let Some(grab) = cx.press_slot::<Grab>() {
            let Some(d) = &self.app.canvas.table_resize else { return };
            let (key, edge) = (key_of(d.target.edge), d.target.edge);
            cx.part(key.at(grab.index), rect)
                .sense(Sense::DRAG)
                .priority(priority)
                .cursor(resize_cursor(edge))
                .drag_cursor(resize_cursor(edge))
                .pointer_only("the table.resizeDrag command sets the same borders");
            self.declared.push((key, grab.index, d.page, None));
            return;
        }
        if !available(self.app) || self.app.canvas.table_resize.is_some() {
            return;
        }
        let mut index = 0u64;
        for placed in self.pages {
            let reach = reach_on(self.pages, placed.page);
            for (area, target) in table_resize::borders(self.layout, &self.app.session.doc, placed.page, reach) {
                let at = index;
                index += 1;
                let Some(shape) = screen_area(placed, &area) else { continue };
                let (key, edge) = (key_of(target.edge), target.edge);
                cx.part(key.at(at), shape)
                    .sense(Sense::DRAG)
                    .priority(priority)
                    .cursor(resize_cursor(edge))
                    .drag_cursor(resize_cursor(edge))
                    .pointer_only("the table.resizeDrag command sets the same borders");
                self.declared.push((key, at, placed.page, Some(target)));
            }
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let declared = std::mem::take(&mut self.declared);
        for (key, index, page, target) in &declared {
            let Some(result) = cx.part_if_declared(key.at(*index)) else { continue };
            let press = result.press;
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    if let Some(t) = target {
                        self.press(cx, start.origin, *page, *index, t.clone());
                    }
                }
                PressPhase::Held { pos, .. } | PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => self.follow(cx, pos),
                PressPhase::Release { start, pos, .. } => self.release(cx, start.clicks, pos, press.mods),
                PressPhase::Cancel { .. } => self.cancel(cx),
                _ => {}
            }
        }
        self.declared = declared;
    }

    /// The dashed preview of any drag in `CanvasState::table_resize` (a ruler's too), and the resize handles.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let style = &cx.style;
        let s = |page: usize, x: f32, y: f32| self.pages.iter().find(|placed| placed.page == page).map(|placed| screen_of(placed, x, y));
        if let Some(d) = &self.app.canvas.table_resize {
            let tg = &d.target;
            let plan = d.plan(self.mods);
            let (left, top) = (tg.left(), tg.top());
            let right = plan.widths.as_ref().map_or(tg.right(), |w| left + w.iter().sum::<f32>());
            let bottom =
                top + tg.rows.iter().map(|r| plan.heights.iter().find(|(i, _)| *i == r.row).map_or(r.bottom - r.top, |(_, h)| *h)).sum::<f32>();
            let line = |a: Option<Pos2>, b: Option<Pos2>| {
                if let (Some(a), Some(b)) = (a, b) {
                    draw.dashed(&[a, b], draw.dashes(), StrokeWidth::Thin, style.preview);
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
                        draw.dashed(&pts, draw.dashes(), StrokeWidth::Thin, style.preview);
                    }
                }
            }
            return;
        }
        if !available(self.app) {
            return;
        }
        // The caret's table shows its handle wherever the pointer is, as in Word.
        for (page, _, h) in caret_handles(self.app, self.pages, self.layout) {
            if let Some(h) = h
                && let (Some(a), Some(b)) = (s(page, h.x, h.y), s(page, h.right(), h.bottom()))
            {
                resize_handle(&draw, Rect::from_two_pos(a, b), style);
            }
        }
        // The handle shows while the pointer is over the table (or on the handle itself).
        let Some(p) = self.pointer else { return };
        let Some(placed) = page_at(self.pages, p) else { return };
        let (x, y) = page_point(placed, p);
        let Some(tg) = table_resize::table_under(self.layout, &self.app.session.doc, placed.page, x, y, REACH_PX / placed.scale as f32) else {
            return;
        };
        let h = tg.handle();
        resize_handle(&draw, Rect::from_two_pos(screen_of(placed, h.x, h.y), screen_of(placed, h.right(), h.bottom())), style);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Shows the table borders over the page view's body.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody) {
    let pointer = ui.input(|i| i.pointer.latest_pos()).filter(|_| ui.rect_contains_pointer(body.rect));
    let mods = mods_now(ui.ctx());
    TableBorders::new(app, layout, body.pages, pointer, mods).show_at(ui, body.rect);
}

/// A ruler drag of the table border `edge` at page point `at` on `page`, now at `to`, as the params of
/// `table.resizeDrag`. It builds the drag as a press on the page does (the border at `at` with a reach of one
/// point, in the selected cells' rows only when it is a border of theirs), keeps it in
/// `CanvasState::table_resize` so the page shows the same dashed preview, and takes its share from `mods`
/// (a ruler's Shift works the other way round). `None` when the layout has no such border at `at`.
/// The ruler clears the slot when its drag ends.
pub(crate) fn ruler_params(
    app: &mut WordApp,
    layout: &DocLayout,
    page: usize,
    edge: Edge,
    at: (f32, f32),
    to: (f32, f32),
    mods: Mods,
) -> Option<Value> {
    let mut target = table_resize::resize_target(layout, &app.session.doc, page, at.0, at.1, 1.0)?;
    if target.edge != edge {
        return None;
    }
    target.limit_to_selection(&app.session, at.1);
    let drag = ResizeDrag { page, from: at, to, target, ruler: true };
    let share = drag.share(mods);
    app.canvas.table_resize = Some(drag);
    Some(json!({
        "page": page, "x": at.0, "y": at.1, "toX": to.0, "toY": to.1,
        "shift": share == Share::LeftOnly, "ctrl": share == Share::Proportional, "reach": 1.0,
    }))
}

/// The move and resize handles (page points) of the caret's table on the pages in view: Word
/// shows them while the caret is in a table, not only under the pointer.
pub(crate) fn caret_handles(
    app: &WordApp,
    pages: &[PlacedPage],
    layout: &DocLayout,
) -> Vec<(usize, Option<wordcraft_geom::Rect>, Option<wordcraft_geom::Rect>)> {
    let focus = &app.session.sel.focus;
    let Some((tp, _, _)) = focus.path.cell() else { return Vec::new() };
    pages
        .iter()
        .map(|placed| {
            let (m, z) = wordcraft_engine::cmd::table_mouse::caret_handles(layout, &app.session.doc, focus.story, &tp, placed.page);
            (placed.page, m, z)
        })
        .filter(|(_, m, z)| m.is_some() || z.is_some())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests;
