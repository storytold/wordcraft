//! The rest of Word's table mouse (resizing is `table_resize`): over a selection area — just
//! above a column, left of a row, along a cell's left edge — the pointer becomes a selection
//! arrow and a click selects the column, row or cell (a drag goes on selecting); left of a row
//! border or above a column border a "+" button shows, and a click adds a row or column there;
//! over a table a move handle shows past its top-left corner: a click selects the table, a drag
//! moves it, with a dashed outline of where it goes. What each spot is and does is the engine's
//! (`cmd::table_mouse`); this sends the presses as its commands, in page points.
//!
//! [`TableMouse`] is the canvas control: the engine lists the spots of the tables on each page in view
//! (`table_mouse::spots`), and each is a part, at its place in the list, that takes the press, the cursor and
//! the drag. The press slot keeps the engine target the press began on, never a part index (the indices are only
//! valid in one pass); the index kept beside it only names the held press's own part again.

use craft::behavior::{Cursor, CursorIcon, HitLevel, HitPriority, Metrics, PaintedCursor, PartKey, PointerButton, PressPhase, PressStart, Sense};
use craft::keymap::Mods;
use craft::pages::ui::{PageBody, PlacedPage};
use craft::ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, PartsCx, Pointer as _, Props, Remember as _, Role, Show as _};
use craft::ui::primitives::{Draw, PaintShape, StrokeWidth};
use craft::ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Ui, Vec2, vec2};
use serde_json::json;
use wordcraft_engine::cmd::table_mouse::{self, InsertMark, Spot, Target};
use wordcraft_engine::cmd::table_resize::{Edge, ResizeTarget};
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::table_resize::{REACH_PX, page_at, page_point, reach_on, screen_area, screen_of};

/// The part of a table's move handle: `HANDLE.at(i)`, `i` the handle's place in this pass's list of spots.
const HANDLE: PartKey = PartKey::new("handle");
/// The part of the insert button of a row border.
const INSERT_ROW: PartKey = PartKey::new("insert_row");
/// The part of the insert button of a column border.
const INSERT_COLUMN: PartKey = PartKey::new("insert_column");
/// The part of the selection area above a column.
const COLUMN_AREA: PartKey = PartKey::new("column_area");
/// The part of the selection area left of a row (or the page margin beside it).
const ROW_AREA: PartKey = PartKey::new("row_area");
/// The part of the selection area along a cell's left edge.
const CELL_EDGE: PartKey = PartKey::new("cell_edge");

/// A press on a table's selection area or move handle, while the button is down. Kept in the control's press
/// slot with the engine target the press began on, never a part index (the indices are only valid in one pass).
#[derive(Clone, Debug)]
struct TableDrag {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
    /// Where the selection was last made to (a drag from a selection area selects as it goes).
    sent: (f32, f32),
    /// The resize reach at the press (page points).
    reach: f32,
    target: Target,
    /// Shift was down at the press on a row area: the rows extend from the anchor row.
    extend: bool,
    /// The row that Shift+click extends from, read at the press: the rows it selects would
    /// otherwise move it on the next frame.
    anchor: Option<usize>,
    /// The index of the part the press began on, so the held press is declared under the same part reference.
    index: u64,
    /// The press selected the whole document (a triple-click or Ctrl/Cmd+click beside a row): the press is held
    /// only so its part is still declared when the button goes up, which ends the document hold. While the hold
    /// is on, the page counts as dragging and no part is declared otherwise, so the release would never arrive.
    document: bool,
}

impl Default for TableDrag {
    /// A slot value only has to exist; a press always replaces it.
    fn default() -> Self {
        let geom = ResizeTarget {
            edge: Edge::Corner,
            story: wordcraft_doc::StoryRef::Body,
            table: wordcraft_doc::Path(Vec::new()),
            cols: Vec::new(),
            rows: Vec::new(),
            ends_here: false,
            rtl: false,
            only: Vec::new(),
        };
        TableDrag {
            page: 0,
            from: (0.0, 0.0),
            to: (0.0, 0.0),
            sent: (0.0, 0.0),
            reach: REACH_PX,
            target: Target { spot: Spot::Handle, geom },
            extend: false,
            anchor: None,
            index: 0,
            document: false,
        }
    }
}

/// `table.selectAt`'s params, with the press's anchor row when it has one.
fn select_params(mut at: serde_json::Value, anchor: Option<usize>) -> serde_json::Value {
    if let (Some(a), Some(m)) = (anchor, at.as_object_mut()) {
        m.insert("anchorRow".into(), json!(a));
    }
    at
}

/// Whether a table drag (a press on a selection area or a move handle) is under way.
pub(crate) fn dragging(app: &WordApp) -> bool {
    app.canvas.table_dragging
}

/// Whether the table mouse is in reach: when resizing is, and not while a border is dragged.
fn available(app: &WordApp) -> bool {
    crate::table_resize::available(app) && app.canvas.table_resize.is_none()
}

/// Whether the selection's anchor (or its selected cells) is in the table at `tp`.
fn in_table(app: &WordApp, tp: &wordcraft_doc::Path) -> bool {
    let body = |story| story == wordcraft_doc::StoryRef::Body;
    app.session.cell_range().is_some_and(|c| body(c.story) && &c.table == tp)
        || (body(app.session.sel.anchor.story) && app.session.sel.anchor.path.cell().is_some_and(|(t, ..)| &t == tp))
}

/// The part kind of a spot.
fn key_of(spot: Spot) -> PartKey {
    match spot {
        Spot::Handle => HANDLE,
        Spot::InsertRow(_) => INSERT_ROW,
        Spot::InsertColumn(_) => INSERT_COLUMN,
        Spot::Column(_) => COLUMN_AREA,
        Spot::Row(_) => ROW_AREA,
        Spot::Cell(..) => CELL_EDGE,
    }
}

/// The pointer over a spot: the move arrows on the handle, the plain arrow on an insert button, the selection
/// arrow over a selection area — down over a column, up and right in a cell and beside a row.
fn cursor_of(spot: Spot) -> Cursor {
    match spot {
        Spot::Handle => CursorIcon::Move.into(),
        Spot::InsertRow(_) | Spot::InsertColumn(_) => CursorIcon::Default.into(),
        Spot::Column(_) => PaintedCursor::arrow_toward(vec2(0.0, 1.0)).into(),
        Spot::Row(_) | Spot::Cell(..) => PaintedCursor::arrow_toward(vec2(1.0, -1.0).normalized()).into(),
    }
}

/// The pointer while a spot is dragged: the move arrows on the handle, else the plain arrow (a drag cursor is a
/// standard icon; the painted selection arrows are only the hover cursor).
fn drag_cursor_of(spot: Spot) -> CursorIcon {
    match spot {
        Spot::Handle => CursorIcon::Move,
        _ => CursorIcon::Default,
    }
}

/// The colours of the handle, the insert button and the move preview.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(crate) struct TableMouseStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The face of the handle and the button.
    face: StyleColor,
    /// The handle's edge.
    edge: StyleColor,
    /// The handle's arrows.
    glyph: StyleColor,
    /// The insert button and the border it marks.
    accent: StyleColor,
    /// The dashed outline of where a moved table goes.
    preview: StyleColor,
    /// The insert button's radius at 100% zoom (points).
    button: f32,
}

/// One part of this pass: its kind and place in the list, the page it lies on and the spot it stands for.
struct Declared {
    key: PartKey,
    index: u64,
    page: usize,
    target: Target,
}

/// The table mouse on the page: the move handles, the insert buttons and the selection areas of the tables.
/// A press on a selection area selects at once and a drag goes on selecting, on an insert button inserts, on the
/// move handle starts a move (released without moving, it selects the table). Where the pointer is on none of
/// them no part exists, so the text tool takes the press.
#[derive(Component, Props)]
#[component(canvas_control)]
pub(crate) struct TableMouse<'a> {
    #[common]
    common: Common,
    app: &'a mut WordApp,
    layout: &'a DocLayout,
    pages: &'a [PlacedPage],
    /// Where the pointer is over the page view, `None` when it is elsewhere or under another layer. Only the
    /// move handle of the table under it is drawn from it; no part is.
    pointer: Option<Pos2>,
    /// The move handles to draw this pass (page, page points).
    handles: Vec<(usize, wordcraft_geom::Rect)>,
    /// The insert button under the pointer, with its page.
    insert: Option<(usize, InsertMark)>,
    /// The drag as the pass's `update` left it, for the paint.
    held: Option<TableDrag>,
    /// The parts declared this pass.
    declared: Vec<Declared>,
}

impl<'a> TableMouse<'a> {
    /// The table mouse over `pages`, laid out as `layout`, with the pointer at `pointer`.
    pub(crate) fn new(app: &'a mut WordApp, layout: &'a DocLayout, pages: &'a [PlacedPage], pointer: Option<Pos2>) -> Self {
        TableMouse { common: Common::new(), app, layout, pages, pointer, handles: Vec::new(), insert: None, held: None, declared: Vec::new() }
    }

    /// Shift+click beside row `row` with the selection started outside the table is the ordinary Shift+click into a
    /// table: the caret logic extends the selection into the row, as the body's own presses do.
    fn extend_into(&mut self, target: &Target, row: usize) {
        let story = target.geom.story;
        let Some(last) =
            self.app.session.doc.table(story, &target.geom.table).and_then(|t| t.rows.get(row)).and_then(|r| r.cells.len().checked_sub(1))
        else {
            return;
        };
        let mut path = target.geom.table.0.clone();
        path.extend([u32::try_from(row).unwrap_or(u32::MAX), u32::try_from(last).unwrap_or(u32::MAX), 0]);
        let pos = wordcraft_doc::Pos { story, path: wordcraft_doc::Path(path), off: 0 };
        if let Ok(pos) = serde_json::to_value(&pos)
            && let Err(e) = self.app.run("caret.set", json!({"pos": pos, "extend": true}))
        {
            self.app.status(e);
        }
    }

    /// A press went down at `start.origin` on part `index` of `page` with `mods` held: a selection area selects, an
    /// insert button inserts, a handle waits.
    fn press(&mut self, cx: &mut Cx<'_, Self>, start: PressStart, mods: Mods, page: usize, index: u64) {
        let Some(placed) = self.pages.iter().find(|placed| placed.page == page).cloned() else { return };
        let (x, y) = page_point(&placed, start.origin);
        let reach = reach_on(self.pages, page);
        let story = self.app.session.sel.focus.story;
        let Some(target) = table_mouse::target_at(self.layout, &self.app.session.doc, story, page, x, y, reach) else { return };
        if let Spot::Row(r) = target.spot
            && mods.shift
            && !in_table(self.app, &target.geom.table)
        {
            self.extend_into(&target, r);
            return;
        }
        let clicks = start.clicks.min(3);
        // Word: a triple-click or Ctrl/Cmd+click beside a row selects the whole document, as in the body's
        // selection bar.
        if matches!(target.spot, Spot::Row(_)) && !mods.shift && (clicks >= 3 || mods.cmd) {
            if let Err(e) = self.app.run("select.all", json!({"whole": true})) {
                self.app.status(e);
            }
            crate::canvas::hold_document_selection(self.app, u8::try_from(clicks).unwrap_or(3));
            // Held, so the release reaches this part and ends the hold (`release`).
            *cx.press_slot::<TableDrag>() = Some(TableDrag {
                page: placed.page,
                from: (x, y),
                to: (x, y),
                sent: (x, y),
                reach,
                target,
                extend: false,
                anchor: None,
                index,
                document: true,
            });
            return;
        }
        let extend = matches!(target.spot, Spot::Row(_)) && mods.shift;
        let anchor = match target.spot {
            Spot::Row(r) if extend => Some(table_mouse::anchor_row(&self.app.session, &target.geom.table, r)),
            _ => None,
        };
        let at = select_params(json!({"page": placed.page, "x": x, "y": y, "reach": reach, "extend": extend}), anchor);
        match target.spot {
            Spot::InsertRow(_) | Spot::InsertColumn(_) => {
                if let Err(e) = self.app.run_keep_view("table.insertAt", at) {
                    self.app.status(e);
                }
            }
            spot => {
                // A selection area selects at once; the move handle waits for the release.
                if spot != Spot::Handle
                    && let Err(e) = self.app.run_keep_view("table.selectAt", at)
                {
                    self.app.status(e);
                }
                *cx.press_slot::<TableDrag>() = Some(TableDrag {
                    page: placed.page,
                    from: (x, y),
                    to: (x, y),
                    sent: (x, y),
                    reach,
                    target,
                    extend,
                    anchor,
                    index,
                    document: false,
                });
            }
        }
    }

    /// `table.selectAt`'s params for the drag `d` as it stands.
    fn drag_params(d: &TableDrag) -> serde_json::Value {
        select_params(
            json!({"page": d.page, "x": d.from.0, "y": d.from.1, "reach": d.reach, "toX": d.to.0, "toY": d.to.1, "extend": d.extend}),
            d.anchor,
        )
    }

    /// The drag moved to `pos`: a selection area selects as it goes; the table waits for the release. The
    /// first move of a held press is the start of a table drag.
    fn drag(&mut self, cx: &mut Cx<'_, Self>, pos: Pos2) {
        let Some(mut d) = cx.press_slot::<TableDrag>().clone() else { return };
        if d.document {
            return;
        }
        self.app.canvas.table_dragging = true;
        if let Some(placed) = self.pages.iter().find(|placed| placed.page == d.page) {
            d.to = page_point(placed, pos);
        }
        if d.target.spot != Spot::Handle && d.to != d.sent {
            d.sent = d.to;
            let _ = self.app.run_keep_view("table.selectAt", Self::drag_params(&d));
        }
        *cx.press_slot::<TableDrag>() = Some(d);
    }

    /// The button went up at `pos` after `clicks` presses in a row. A handle moves the table when the pointer
    /// moved (one undo step) and selects it otherwise; a selection area ends its selection, and a double-click
    /// along a cell's left edge selects its row, as in Word.
    fn release(&mut self, cx: &mut Cx<'_, Self>, clicks: u32, pos: Pos2) {
        let Some(mut d) = cx.press_slot::<TableDrag>().take() else {
            // No table drag: the press may have selected the whole document (`press`), which holds until now.
            crate::canvas::release_document_selection(self.app);
            return;
        };
        if d.document {
            crate::canvas::release_document_selection(self.app);
            return;
        }
        self.app.canvas.table_dragging = false;
        if let Some(placed) = self.pages.iter().find(|placed| placed.page == d.page) {
            d.to = page_point(placed, pos);
        }
        if d.target.spot == Spot::Handle {
            let moved = (d.to.0 - d.from.0).abs() >= 0.5 || (d.to.1 - d.from.1).abs() >= 0.5;
            let id = if moved && table_mouse::movable(&d.target.geom) { "table.moveDrag" } else { "table.selectAt" };
            if let Err(e) = self.app.run_keep_view(id, Self::drag_params(&d)) {
                self.app.status(e);
            }
            return;
        }
        if d.to != d.sent {
            let _ = self.app.run_keep_view("table.selectAt", Self::drag_params(&d));
        } else if matches!(d.target.spot, Spot::Cell(..))
            && clicks == 2
            && let Err(e) = self.app.run("table.selectRow", json!({}))
        {
            self.app.status(e);
        }
    }

    /// Escape (or another end without a release) ends the press with no command.
    fn cancel(&mut self, cx: &mut Cx<'_, Self>) {
        if cx.press_slot::<TableDrag>().take().is_some_and(|d| d.document) {
            crate::canvas::release_document_selection(self.app);
        }
        self.app.canvas.table_dragging = false;
    }
}

/// The move handle: a small square with arrows in four directions.
fn move_handle(draw: &Draw<'_>, r: Rect, style: &TableMouseStyle) {
    let square = PaintShape::Rect(r, CornerRadius::same(1));
    draw.fill(&square, style.face);
    draw.stroke(&square, StrokeWidth::Thin, style.edge);
    let (c, k) = (r.center(), r.width() * 0.34);
    draw.stroke(&PaintShape::Path(vec![c - vec2(k, 0.0), c + vec2(k, 0.0)]), StrokeWidth::Thin, style.glyph);
    draw.stroke(&PaintShape::Path(vec![c - vec2(0.0, k), c + vec2(0.0, k)]), StrokeWidth::Thin, style.glyph);
    let head = k * 0.5;
    for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
        let (tip, side) = (c + d * k, vec2(-d.y, d.x) * head);
        draw.fill(&PaintShape::Polygon(vec![tip, tip - d * head + side, tip - d * head - side]), style.glyph);
    }
}

/// An insert button: a circled "+" out from the table, joined to the border it inserts at,
/// which is marked from end to end. `sc` is the page's scale.
fn insert_button(draw: &Draw<'_>, c: Pos2, from: Pos2, to: Pos2, sc: f32, style: &TableMouseStyle) {
    draw.stroke(&PaintShape::Path(vec![c, from]), StrokeWidth::Thin, style.accent);
    draw.stroke(&PaintShape::Path(vec![from, to]), StrokeWidth::Medium, style.accent);
    let r = style.button * sc;
    draw.fill(&PaintShape::Circle(c, r), style.face);
    draw.stroke(&PaintShape::Circle(c, r), StrokeWidth::Thin, style.accent);
    let k = r * 0.55;
    draw.stroke(&PaintShape::Path(vec![c - vec2(k, 0.0), c + vec2(k, 0.0)]), StrokeWidth::Medium, style.accent);
    draw.stroke(&PaintShape::Path(vec![c - vec2(0.0, k), c + vec2(0.0, k)]), StrokeWidth::Medium, style.accent);
}

impl Component for TableMouse<'_> {
    const ROLE: Role = Role::None;
    const OVERLAYS: bool = false;
    const STOPS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = TableMouseStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: TableMouseStyleKey) -> TableMouseStyle {
        let colors = &cx.tokens.color;
        TableMouseStyle {
            metrics: Metrics::new(),
            face: cx.color(colors.overlay.handle_fill),
            edge: cx.color(colors.overlay.handle_stroke),
            glyph: cx.color(colors.text.normal),
            accent: cx.color(colors.accent.solid),
            preview: cx.color(colors.overlay.stroke),
            // PLAN: assumed `tokens.size.icon.glyph_sm` is an `f32` in points; the button is a glyph-sized disc.
            button: cx.tokens.size.icon.glyph_sm / 2.0,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// One part per spot the engine lists for the tables on the pages in view, at its place in the list; while a
    /// press is held, its own part.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.declared.clear();
        self.handles.clear();
        self.insert = None;
        if let Some(d) = cx.press_slot::<TableDrag>() {
            let key = key_of(d.target.spot);
            cx.part(key.at(d.index), rect)
                .sense(Sense::DRAG)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .cursor(cursor_of(d.target.spot))
                .drag_cursor(CursorIcon::Move)
                .pointer_only("the table.selectAt and table.moveDrag commands do the same");
            self.declared.push(Declared { key, index: d.index, page: d.page, target: d.target });
            return;
        }
        if !available(self.app) {
            return;
        }
        // The caret's table shows its move handle wherever the pointer is, as in Word; the table under the pointer
        // shows its own on the way to it (drawn only).
        for (page, h, _) in crate::table_resize::caret_handles(self.app, self.pages, self.layout) {
            if let Some(h) = h {
                self.handles.push((page, h));
            }
        }
        if let Some(p) = self.pointer
            && let Some(placed) = page_at(self.pages, p)
        {
            let (x, y) = page_point(placed, p);
            let (doc, story, reach) = (&self.app.session.doc, self.app.session.sel.focus.story, reach_on(self.pages, placed.page));
            if let Some(g) = table_mouse::handle_table(self.layout, doc, story, placed.page, x, y, reach) {
                let h = table_mouse::handle(&g);
                if !self.handles.contains(&(placed.page, h)) {
                    self.handles.push((placed.page, h));
                }
            }
        }
        let story = self.app.session.sel.focus.story;
        let mut index = 0u64;
        for placed in self.pages {
            let reach = reach_on(self.pages, placed.page);
            for a in table_mouse::spots(self.layout, &self.app.session.doc, story, placed.page, reach) {
                let at = index;
                index += 1;
                let Some(shape) = screen_area(placed, &a.area) else { continue };
                let spot = a.target.spot;
                let key = key_of(spot);
                let level = if matches!(spot, Spot::Handle | Spot::InsertRow(_) | Spot::InsertColumn(_)) { HitLevel::Handle } else { HitLevel::Edge };
                // A border wins over a spot, as the engine's `target_at` decides: borders rank above (`table_resize`).
                cx.part(key.at(at), shape)
                    .sense(Sense::DRAG)
                    .priority(HitPriority::new(level, 0))
                    .cursor(cursor_of(spot))
                    .drag_cursor(drag_cursor_of(spot))
                    .pointer_only("the table.selectAt, table.insertAt and table.moveDrag commands do the same");
                self.declared.push(Declared { key, index: at, page: placed.page, target: a.target });
            }
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let declared = std::mem::take(&mut self.declared);
        for d in &declared {
            let Some(result) = cx.part_if_declared(d.key.at(d.index)) else { continue };
            let press = result.press;
            match press.phase {
                PressPhase::Hover => {
                    if let Some(mark) = table_mouse::insert_mark(&d.target) {
                        self.insert = Some((d.page, mark));
                    }
                }
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => self.press(cx, start, press.mods, d.page, d.index),
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => self.drag(cx, pos),
                PressPhase::Release { start, pos, .. } => self.release(cx, start.clicks, pos),
                PressPhase::Cancel { .. } => self.cancel(cx),
                _ => {}
            }
        }
        self.declared = declared;
        self.held = cx.press_slot::<TableDrag>().clone();
    }

    /// The handles and the insert button, or, while a handle is dragged, a dashed outline of where the table goes.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let style = &cx.style;
        if let Some(d) = &self.held {
            if d.target.spot == Spot::Handle
                && let Some(placed) = self.pages.iter().find(|placed| placed.page == d.page)
            {
                let g = &d.target.geom;
                let (x, y) = table_mouse::moved_to(self.layout, d.page, g, d.to.0 - d.from.0, d.to.1 - d.from.1);
                let (w, h) = (g.right() - g.left(), g.bottom() - g.top());
                let r = Rect::from_two_pos(screen_of(placed, x, y), screen_of(placed, x + w, y + h));
                let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                draw.dashed(&pts, draw.dashes(), StrokeWidth::Thin, style.preview);
            }
            return;
        }
        for (page, h) in &self.handles {
            if let Some(placed) = self.pages.iter().find(|placed| placed.page == *page) {
                move_handle(&draw, Rect::from_two_pos(screen_of(placed, h.x, h.y), screen_of(placed, h.right(), h.bottom())), style);
            }
        }
        if let Some((page, m)) = &self.insert
            && let Some(placed) = self.pages.iter().find(|placed| placed.page == *page)
        {
            let s = |p: (f32, f32)| screen_of(placed, p.0, p.1);
            insert_button(&draw, s(m.center), s(m.from), s(m.to), placed.scale as f32, style);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// Shows the table mouse over the page view's body.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody) {
    let pointer = ui.input(|i| i.pointer.latest_pos()).filter(|_| ui.rect_contains_pointer(body.rect));
    TableMouse::new(app, layout, body.pages, pointer).show_at(ui, body.rect);
}

#[cfg(test)]
mod tests;
