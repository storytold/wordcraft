//! The rest of Word's table mouse (resizing is `table_resize`): selection areas just above a
//! column, left of a row and along a cell's left edge inside it select the column, row or cell,
//! and a drag from one selects every one up to the pointer; insert buttons left of a row border
//! and above a column border add a row or column there; the move handle just past the table's
//! top-left corner selects the table or, dragged, moves it (the table then floats where it's
//! dropped, the text wrapping around it, as in Word); a double-click on a column border fits the
//! column left of it to its contents. The app sends each click or finished drag here in page
//! points, so each is one command (one undo step when it changes the document).

use std::collections::BTreeMap;

use serde_json::Value;
use wordcraft_doc::para::Anchor;
use wordcraft_doc::props::TableFloat;
use wordcraft_doc::{Document, Path, Pos, StoryRef, Table};
use wordcraft_geom::{Point, Rect};
use wordcraft_layout::{DocLayout, Placed};

use super::table_resize::{self, CellBox, Edge, REACH, ResizeTarget};
use super::{sel_result, table};
use crate::{CellRange, CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// The column selection area: from this far above a table's top border (points) to this far
/// below it.
const COLUMN_ABOVE: f32 = 6.0;
const COLUMN_BELOW: f32 = 1.5;
/// How far left of a table (points) the row selection area and the row insert buttons reach.
const ROW_LEFT: f32 = 24.0;
/// The cell selection area's width inside a cell's left edge (points), at the least; it starts
/// past the column border's resize reach.
const CELL_EDGE: f32 = 5.5;
/// The insert buttons answer this near a border (points), and above the table this far out.
const INSERT_REACH: f32 = 3.5;
const INSERT_OUT: f32 = 14.0;
/// An insert button's centre sits this far out from the table's edge (points).
const BUTTON_OUT: f32 = 8.0;
/// The move handle's side and its gap from the table's top-left corner (points).
const HANDLE: f32 = 8.0;
const HANDLE_GAP: f32 = 2.0;
/// The farthest a drag moves a table (points; Word's 22 inches).
const MAX_MOVE: f32 = 1584.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("table.selectAt", "Select Table Cells by Clicking", "Table Layout › Table › Select", select_at)
            .params(
                r#"{"page": n, "x": pt, "y": pt, "toX"?: pt, "toY"?: pt, "reach"?: pt, "extend"?: bool, "anchorRow"?: n} (a click in page points, 0-based page, on a selection area of a table in the caret's story: just above a column selects the column, left of a row the row, along a cell's left edge inside it the cell, the move handle past the top-left corner the table; with toX/toY (a drag) every column, row or cell from there to that point; the page margin beside a table (its leading side: right for a right-to-left table) is a row area too, and `extend` (Shift+click) on one takes the rows from the selected ones' anchor, or from row `anchorRow` (0-based) when given, so a drag keeps its press's anchor. `reach` (default 3) is the column borders' resize reach, which the cell areas keep clear of.)"#,
            )
            .pure(),
        CommandSpec::new("table.insertAt", "Insert Row or Column by Clicking", "Table Layout › Rows & Columns", insert_at).params(
            r#"{"page": n, "x": pt, "y": pt, "reach"?: pt} (a click on a table's insert button: left of a row border adds a row there, above a column border a column; the new row or column is selected, as in Word)"#,
        ),
        CommandSpec::new("table.moveDrag", "Move Table by Dragging", "Table Layout › Table", move_drag).params(
            r#"{"page": n, "x": pt, "y": pt, "toX": pt, "toY": pt, "reach"?: pt} (a drag from the move handle past a table's top-left corner: the table floats where it's dropped, the text wrapping around it; tables in the main text only)"#,
        ),
        CommandSpec::new("table.fitColumn", "AutoFit Column by Double-Clicking", "Table Layout › Cell Size", fit_column).params(
            r#"{"page": n, "x": pt, "y": pt, "reach"?: pt} (a double-click on a column border: the column left of it fits its contents and the columns right of it keep their widths, moving with the border, so the table narrows or widens (up to the right margin), as in Word)"#,
        ),
    ]
}

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    /// The move handle.
    Handle,
    /// The insert button of horizontal border `n` (0 = the table's top, `n` = the bottom of row
    /// `n - 1`).
    InsertRow(usize),
    /// The insert button of grid line `n` (0 = the table's left edge).
    InsertColumn(usize),
    /// The selection area above grid column `n`.
    Column(usize),
    /// The selection area left of row `n`.
    Row(usize),
    /// The selection area along cell (`row`, `cell`)'s left edge.
    Cell(usize, usize),
}

/// A spot and the table's drawn geometry on the page (`geom.edge` means nothing here).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub spot: Spot,
    pub geom: ResizeTarget,
}

/// An insert button: its centre and the border it inserts at, end to end (page points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InsertMark {
    pub center: (f32, f32),
    pub from: (f32, f32),
    pub to: (f32, f32),
}

/// The move handle's square (page points), just past the table's top-left corner.
pub fn handle(geom: &ResizeTarget) -> Rect {
    Rect::new(geom.left() - HANDLE_GAP - HANDLE, geom.top() - HANDLE_GAP - HANDLE, HANDLE, HANDLE)
}

/// Whether the table's first row is on this page (its move handle is there).
fn starts_here(geom: &ResizeTarget) -> bool {
    geom.rows.first().is_some_and(|r| r.row == 0)
}

fn on_handle(geom: &ResizeTarget, x: f32, y: f32, reach: f32) -> bool {
    let h = handle(geom);
    starts_here(geom) && x >= h.x - reach / 2.0 && x <= h.right() + reach / 2.0 && y >= h.y - reach / 2.0 && y <= h.bottom() + reach / 2.0
}

fn clean_reach(reach: f32) -> f32 {
    if reach.is_finite() { reach.clamp(0.5, 24.0) } else { REACH }
}

/// Of `lines` (index, position), the one nearest `at` within `reach`.
fn nearest(lines: impl Iterator<Item = (usize, f32)>, at: f32, reach: f32) -> Option<usize> {
    lines.map(|(i, v)| (i, (at - v).abs())).filter(|(_, d)| *d <= reach).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
}

/// The grid column at page x `x` (the first or the last beyond the table).
fn column_at(geom: &ResizeTarget, x: f32) -> usize {
    let n = geom.cols.len().saturating_sub(1);
    geom.cols.iter().skip(1).take_while(|cx| **cx <= x).count().min(n.saturating_sub(1))
}

/// The drawn row at page y `y` (the first or the last beyond the table).
fn row_at(geom: &ResizeTarget, y: f32) -> usize {
    geom.rows.iter().find(|r| y < r.bottom).or(geom.rows.last()).map_or(0, |r| r.row)
}

/// The spot of one table at (`x`, `y`): the handle, then the insert buttons, then the selection
/// areas. `cells` are its cell boxes on the page.
fn spot_in(geom: &ResizeTarget, cells: &[&CellBox], x: f32, y: f32, reach: f32, bar: Option<(f32, f32)>) -> Option<Spot> {
    if on_handle(geom, x, y, reach) {
        return Some(Spot::Handle);
    }
    let (left, right, top, bottom) = (geom.left(), geom.right(), geom.top(), geom.bottom());
    // Above the table: the insert buttons over its column borders, and the column areas.
    if y >= top - INSERT_OUT && y <= top + COLUMN_BELOW && x >= left - INSERT_REACH && x <= right + INSERT_REACH {
        let lines = geom.cols.iter().copied().enumerate().filter(|(i, _)| geom.rows.iter().any(|r| r.edges.contains(i)));
        if y < top
            && let Some(line) = nearest(lines, x, INSERT_REACH)
        {
            return Some(Spot::InsertColumn(line));
        }
        if y >= top - COLUMN_ABOVE && x >= left && x <= right {
            return Some(Spot::Column(column_at(geom, x)));
        }
    }
    // Left of the table: the insert buttons beside its row borders, and the row areas.
    if x >= left - ROW_LEFT && x < left && y >= top - INSERT_REACH && y <= bottom + INSERT_REACH {
        let borders = geom.rows.iter().flat_map(|r| [(r.row, r.top), (r.row + 1, r.bottom)]);
        if let Some(line) = nearest(borders, y, INSERT_REACH) {
            return Some(Spot::InsertRow(line));
        }
        if let Some(r) = geom.rows.iter().find(|r| y >= r.top && y <= r.bottom) {
            return Some(Spot::Row(r.row));
        }
    }
    // The page margin beside the table is the selection bar: a row's area too.
    if let Some((x0, x1)) = bar
        && x >= x0
        && x <= x1
        && let Some(r) = geom.rows.iter().find(|r| y >= r.top && y <= r.bottom)
    {
        return Some(Spot::Row(r.row));
    }
    // Inside a cell, along its left edge.
    let edge = CELL_EDGE.max(reach + 2.5);
    cells.iter().find(|b| x >= b.rect.x && x <= b.rect.x + edge && y >= b.rect.y && y <= b.rect.bottom()).map(|b| Spot::Cell(b.row, b.cell))
}

/// The x range of the selection bar beside the table's row at page height `y`: the page margin
/// on the table's leading side (left, or right for a right-to-left table). Only a non-floating
/// table of the main text has one, and only where it is what stands beside that margin: no line
/// of the text (another column, text wrapped round the table) lies between the two at that
/// height. A nested table's rows are the outermost table's.
fn row_bar(l: &DocLayout, doc: &Document, page: usize, geom: &ResizeTarget, y: f32) -> Option<(f32, f32)> {
    let p = l.pages.get(page)?;
    if geom.story != StoryRef::Body || geom.table.0.len() != 1 {
        return None;
    }
    let t = doc.table(geom.story, &geom.table)?;
    let row = geom.rows.iter().find(|r| y >= r.top && y <= r.bottom)?;
    if t.props.float.is_some() {
        return None;
    }
    let rtl = t.props.bidi_visual;
    let blocked = wordcraft_layout::hit::page_lines(p, StoryRef::Body).iter().any(|ln| {
        ln.path.0.len() <= 1
            && ln.top < row.bottom - 0.5
            && ln.bottom > row.top + 0.5
            && if rtl { ln.right > geom.right() } else { ln.left < geom.left() }
    });
    if blocked {
        return None;
    }
    Some(if rtl { (geom.right().max(p.body.right()), p.w) } else { (0.0, geom.left().min(p.body.x)) })
}

/// The spot under page point (`x`, `y`) of a table in `story`, of nested tables the innermost's.
/// None on a column or row border or the resize handle: those resize (`table_resize`). `reach`
/// is their resize reach (points).
pub fn target_at(l: &DocLayout, doc: &Document, story: StoryRef, page: usize, x: f32, y: f32, reach: f32) -> Option<Target> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let reach = clean_reach(reach);
    if table_resize::resize_target(l, doc, page, x, y, reach).is_some() {
        return None;
    }
    let boxes = table_resize::cell_boxes(l, page);
    table_resize::tables_on(l, doc, page)
        .into_iter()
        .filter(|g| g.story == story)
        .filter_map(|geom| {
            let cells: Vec<&CellBox> = boxes.iter().filter(|b| b.story == geom.story && b.table == geom.table).collect();
            spot_in(&geom, &cells, x, y, reach, row_bar(l, doc, page, &geom, y)).map(|spot| Target { spot, geom })
        })
        .max_by_key(|t| t.geom.table.0.len())
}

/// The table whose move handle shows with the pointer at (`x`, `y`): the innermost table of
/// `story` starting on this page that the pointer is over, or on its way to the handle.
pub fn handle_table(l: &DocLayout, doc: &Document, story: StoryRef, page: usize, x: f32, y: f32, reach: f32) -> Option<ResizeTarget> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let pad = clean_reach(reach) / 2.0;
    table_resize::tables_on(l, doc, page)
        .into_iter()
        .filter(|g| {
            let h = handle(g);
            g.story == story && starts_here(g) && x >= h.x - pad && x <= g.right() && y >= h.y - pad && y <= g.bottom()
        })
        .max_by_key(|g| g.table.0.len())
}

/// The move and resize handles of table `table` of `story` on this page, as Word shows them
/// while the caret is in the table: the move handle where it starts, the resize handle where it
/// ends.
pub fn caret_handles(l: &DocLayout, doc: &Document, story: StoryRef, table: &Path, page: usize) -> (Option<Rect>, Option<Rect>) {
    let rows = doc.table(story, table).map_or(0, |t| t.rows.len());
    let has_row = |p: usize, r: usize| {
        l.pages.get(p).is_some_and(|pg| {
            pg.items.iter().any(|it| matches!(it, Placed::Cell { table: t, story: st, row, .. } if *st == story && t == table && *row == r))
        })
    };
    // A repeated header row, or a row running on to the next page, is drawn on more than one
    // page: the handles go where the table starts and where it ends (its pages are consecutive).
    let first = has_row(page, 0) && !page.checked_sub(1).is_some_and(|p| has_row(p, 0));
    let last = rows.checked_sub(1).is_some_and(|r| has_row(page, r) && !page.checked_add(1).is_some_and(|p| has_row(p, r)));
    if !first && !last {
        return (None, None);
    }
    // The table's geometry on this page that the pointer handles are drawn and grabbed from.
    let Some(g) = table_resize::tables_on(l, doc, page).into_iter().find(|g| g.story == story && g.table == *table) else {
        return (None, None);
    };
    (first.then(|| handle(&g)), last.then(|| g.handle()))
}

/// The insert button a target is, if it's one.
pub fn insert_mark(t: &Target) -> Option<InsertMark> {
    let g = &t.geom;
    match t.spot {
        Spot::InsertRow(line) => {
            // The bottom of the row above the border, or the top of the first row.
            let y = g.rows.iter().find(|r| r.row + 1 == line).map(|r| r.bottom).or_else(|| g.rows.iter().find(|r| r.row == line).map(|r| r.top))?;
            Some(InsertMark { center: (g.left() - BUTTON_OUT, y), from: (g.left(), y), to: (g.right(), y) })
        }
        Spot::InsertColumn(line) => {
            let x = g.cols.get(line).copied()?;
            Some(InsertMark { center: (x, g.top() - BUTTON_OUT), from: (x, g.top()), to: (x, g.bottom()) })
        }
        _ => None,
    }
}

/// Whether dragging the move handle moves the table: tables in the main text, not nested ones.
pub fn movable(geom: &ResizeTarget) -> bool {
    geom.story == StoryRef::Body && geom.table.0.len() == 1
}

/// Where a move-handle drag by (`dx`, `dy`) puts the table's top-left corner on `page`: on the
/// page, its part there above the bottom margin.
pub fn moved_to(l: &DocLayout, page: usize, geom: &ResizeTarget, dx: f32, dy: f32) -> (f32, f32) {
    let clean = |d: f32| if d.is_finite() { d.clamp(-MAX_MOVE, MAX_MOVE) } else { 0.0 };
    let (w, h) = (geom.right() - geom.left(), geom.bottom() - geom.top());
    let (page_w, body_bottom) = l.pages.get(page).map_or((0.0, 0.0), |pg| (pg.w, pg.body.bottom()));
    let x = (geom.left() + clean(dx)).clamp(0.0, (page_w - w).max(0.0));
    let y = (geom.top() + clean(dy)).clamp(0.0, (body_bottom - h).max(0.0));
    (x, y)
}

/// Where the selected cells are drawn (page, rect), for shading them.
pub fn cell_rects(l: &DocLayout, doc: &Document, range: &CellRange) -> Vec<(usize, Rect)> {
    let Some(t) = doc.table(range.story, &range.table) else { return Vec::new() };
    let picked: std::collections::BTreeSet<(usize, usize)> = range.cells(t).into_iter().collect();
    let mut out = Vec::new();
    for (pi, pg) in l.pages.iter().enumerate() {
        for it in &pg.items {
            // A vertically merged cell's box spans its rows (#640); its continuation rows'
            // `covered` boxes would shade it twice.
            if let Placed::Cell { rect, table, row, cell, story, covered: false, .. } = it
                && *story == range.story
                && *table == range.table
                && picked.contains(&(*row, *cell))
            {
                out.push((pi, *rect));
            }
        }
    }
    out
}

/// What a stream selection from `a` to `b` shades, as in Word: its text, except that each row of
/// a table it takes from start to end (running into or across the table, `table::extend_selection`
/// takes whole rows) shades all its cells instead (an empty one too), on every page. A table
/// both ends are in keeps its text (whole cells there are a `CellRange`).
pub fn selection_rects(l: &DocLayout, doc: &Document, a: &Pos, b: &Pos, page_hint: usize) -> Vec<(usize, Rect)> {
    let text = l.selection_rects(doc, a, b, page_hint);
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    if a.story != b.story {
        return text;
    }
    let mut seen = BTreeMap::new();
    let mut cells = Vec::new();
    for (pi, pg) in l.pages.iter().enumerate() {
        for it in &pg.items {
            // A vertically merged cell's box spans its rows (#640), as in `cell_rects`.
            if let Placed::Cell { rect, table, row, story, covered: false, .. } = it
                && *story == a.story
                && row_taken(doc, a, b, table, *row, &mut seen)
                && !outer_row_taken(doc, a, b, table, &mut seen)
            {
                cells.push((pi, *rect));
            }
        }
    }
    if cells.is_empty() {
        return text;
    }
    let mut out: Vec<(usize, Rect)> = text
        .into_iter()
        .filter(|(pi, r)| {
            let mid = Point::new(r.x + r.w / 2.0, r.y + r.h / 2.0);
            !cells.iter().any(|(cp, c)| cp == pi && c.contains(mid))
        })
        .collect();
    out.extend(cells);
    out
}

/// Whether the selection `a`..`b` (in order, one story) takes row `r` of table `tp` whole: it
/// runs from the row's start (or before it) to its end (or past it) and doesn't both start and
/// end in the table. Remembered in `seen`.
fn row_taken(doc: &Document, a: &Pos, b: &Pos, tp: &Path, r: usize, seen: &mut BTreeMap<(Path, usize), bool>) -> bool {
    if let Some(&taken) = seen.get(&(tp.clone(), r)) {
        return taken;
    }
    let inside = |p: &Pos| p.path.0.len() > tp.0.len() && p.path.0.starts_with(&tp.0);
    let taken = if inside(a) && inside(b) {
        false
    } else {
        let mut row = tp.0.clone();
        row.push(u32::try_from(r).unwrap_or(u32::MAX));
        let (a_in, b_in) = (a.path.0.starts_with(&row), b.path.0.starts_with(&row));
        // Where the row starts and ends matters only for an end in it.
        let ends = if a_in || b_in { table::row_ends(doc, a.story, tp, r) } else { None };
        let from = if a_in { ends.as_ref().is_some_and(|(start, _)| start == a) } else { a.path.0 < row };
        let to = if b_in { ends.as_ref().is_some_and(|(_, end)| end == b) } else { b.path.0 > row };
        from && to
    };
    seen.insert((tp.clone(), r), taken);
    taken
}

/// Whether a row of a table that table `tp` is nested in is taken whole (it shades instead).
fn outer_row_taken(doc: &Document, a: &Pos, b: &Pos, tp: &Path, seen: &mut BTreeMap<(Path, usize), bool>) -> bool {
    let mut p = tp.clone();
    while let Some((outer, r, _)) = p.cell() {
        if row_taken(doc, a, b, &outer, r, seen) {
            return true;
        }
        p = outer;
    }
    false
}

/// The click: `page`, `x`, `y` and the resize `reach`.
fn point(v: &Value) -> Result<(usize, f32, f32, f32), CmdError> {
    let page =
        p::u64(v, "page").map(|n| usize::try_from(n).unwrap_or(usize::MAX)).ok_or_else(|| CmdError::Params("`page` (number) is required".into()))?;
    Ok((page, p::req_f32(v, "x")?, p::req_f32(v, "y")?, p::f32(v, "reach").unwrap_or(REACH)))
}

/// The spot under the click, in the caret's story.
fn target(s: &mut Session, page: usize, x: f32, y: f32, reach: f32) -> Result<Target, CmdError> {
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    target_at(&l, &s.doc, s.sel.focus.story, page, x, y, reach)
        .ok_or_else(|| CmdError::Failed("there's no table selection area or button there".into()))
}

fn select_at(s: &mut Session, v: &Value) -> CmdResult {
    let (page, x, y, reach) = point(v)?;
    let Target { spot, geom } = target(s, page, x, y, reach)?;
    let to = p::f32(v, "toX").zip(p::f32(v, "toY"));
    let story = geom.story;
    let t = s.doc.table(story, &geom.table).ok_or_else(|| CmdError::Failed("there's no table there".into()))?;
    let (last_row, last_col) = (t.rows.len().saturating_sub(1), t.cols().saturating_sub(1));
    let (rows, cols) = match spot {
        Spot::Column(g) => ((0, last_row), (g, to.map_or(g, |(tx, _)| column_at(&geom, tx)))),
        Spot::Row(r) => {
            let r2 = to.map_or(r, |(_, ty)| row_at(&geom, ty));
            // Shift+click: the rows from the selection's anchor row to the one clicked.
            // A drag after it keeps the anchor the press had (`anchorRow`).
            let anchor = if v.get("extend").and_then(Value::as_bool).unwrap_or(false) {
                p::u64(v, "anchorRow").map_or_else(|| anchor_row(s, &geom.table, r), |a| usize::try_from(a).unwrap_or(usize::MAX).min(last_row))
            } else {
                r
            };
            ((anchor, r2), (0, last_col))
        }
        Spot::Cell(r, c) => {
            let (g0, g1) = table::grid_cols(t, r, c);
            match to {
                Some((tx, ty)) => {
                    let g = column_at(&geom, tx);
                    ((r, row_at(&geom, ty)), (g0.min(g), g1.max(g)))
                }
                None => ((r, r), (g0, g1)),
            }
        }
        Spot::Handle => ((0, last_row), (0, last_col)),
        Spot::InsertRow(_) | Spot::InsertColumn(_) => return Err(CmdError::Failed("there's an insert button there, not a selection area".into())),
    };
    table::select_range(s, story, &geom.table, rows, cols)
}

/// The row a Shift+click on row `clicked` of table `tp` extends from: the end of the selected
/// rows farther from it, or the caret's row; the clicked row itself when nothing of the table is
/// selected. The app reads it at the press and sends it as `anchorRow` while the button is held.
pub fn anchor_row(s: &Session, tp: &Path, clicked: usize) -> usize {
    if let Some(range) = s.cell_range().filter(|c| &c.table == tp) {
        return if clicked >= range.rows.0 { range.rows.0 } else { range.rows.1 };
    }
    match s.sel.anchor.path.cell() {
        Some((t, row, _)) if &t == tp => row,
        _ => clicked,
    }
}

/// A cell to insert a column beside at grid line `line`: one starting there (the column goes
/// before it) or, at the table's right edge, a row's last cell (after it). (row, cell, after)
fn beside_line(t: &Table, line: usize) -> Option<(usize, usize, bool)> {
    t.rows.iter().enumerate().find_map(|(r, row)| {
        let mut g = row.first_grid();
        for (c, cell) in row.cells.iter().enumerate() {
            if g == line {
                return Some((r, c, false));
            }
            g = g.saturating_add(cell.span());
            if g == line && c + 1 == row.cells.len() {
                return Some((r, c, true));
            }
        }
        None
    })
}

fn insert_at(s: &mut Session, v: &Value) -> CmdResult {
    let (page, x, y, reach) = point(v)?;
    let Target { spot, geom } = target(s, page, x, y, reach)?;
    let story = geom.story;
    // The row or column goes beside a cell at the border, as Insert Above / Below / Left / Right
    // would put it there, so it takes the border's index (`line`); then, as in Word, it's selected.
    let (r, c, after, rows, line) = match spot {
        Spot::InsertRow(line) => match line.checked_sub(1) {
            Some(above) => (above, 0, true, true, line),
            None => (0, 0, false, true, line),
        },
        Spot::InsertColumn(line) => {
            let t = s.doc.table(story, &geom.table).ok_or_else(|| CmdError::Failed("there's no table there".into()))?;
            let (r, c, after) = beside_line(t, line).ok_or_else(|| CmdError::Failed("there's no column border there".into()))?;
            (r, c, after, false, line)
        }
        _ => return Err(CmdError::Failed("there's a selection area there, not an insert button".into())),
    };
    let size = |s: &Session| s.doc.table(story, &geom.table).map(|t| (t.rows.len(), t.cols()));
    let before = size(s);
    let mut path = geom.table.0.clone();
    path.extend([r as u32, c as u32, 0]);
    s.column = None;
    s.sel = Selection::caret(Pos { story, path: Path(path), off: 0 });
    if rows {
        table::row(s, after)?;
    } else {
        table::col(s, after)?;
    }
    if size(s) == before {
        return Err(CmdError::Failed("the table can't take another row or column".into()));
    }
    let (sel_rows, sel_cols) = if rows { ((line, line), (0, usize::MAX)) } else { ((0, usize::MAX), (line, line)) };
    table::select_range(s, story, &geom.table, sel_rows, sel_cols)
}

/// Where table `table` of `story` is first drawn: its page and top-left corner.
pub fn corner(l: &DocLayout, story: StoryRef, table: &Path) -> Option<(usize, f32, f32)> {
    l.pages.iter().enumerate().find_map(|(pi, pg)| {
        let cells = pg.items.iter().filter_map(|it| match it {
            Placed::Cell { rect, table: t, story: st, .. } if *st == story && t == table => Some(*rect),
            _ => None,
        });
        cells.fold(None, |acc: Option<(f32, f32)>, r| Some(acc.map_or((r.x, r.y), |(x, y)| (x.min(r.x), y.min(r.y))))).map(|(x, y)| (pi, x, y))
    })
}

fn move_drag(s: &mut Session, v: &Value) -> CmdResult {
    let (page, x, y, reach) = point(v)?;
    let (to_x, to_y) = (p::req_f32(v, "toX")?, p::req_f32(v, "toY")?);
    let Target { spot, geom } = target(s, page, x, y, reach)?;
    if spot != Spot::Handle {
        return Err(CmdError::Failed("there's no table move handle there".into()));
    }
    if !movable(&geom) {
        return Err(CmdError::Failed("only a table in the main text can be moved".into()));
    }
    let (gx, gy) = moved_to(&s.layout(), page, &geom, to_x - x, to_y - y);
    if (gx - geom.left()).abs() < 0.01 && (gy - geom.top()).abs() < 0.01 {
        return sel_result(s);
    }
    // Dragged, a table floats where it's dropped, the text wrapping around it (Word): placed from
    // its column and the text where it stands, 0.125" from the text at its sides.
    let t = s.doc.table_mut(geom.story, &geom.table)?;
    let mut f = t.props.float.unwrap_or(TableFloat {
        h_rel: Anchor::Column,
        v_rel: Anchor::Paragraph,
        dist: [9.0, 0.0, 9.0, 0.0],
        overlap: true,
        ..Default::default()
    });
    f.h_align = None;
    f.v_align = None;
    t.props.float = Some(f);
    table_resize::touch(s, &geom)?;
    // Placed by offsets, the table moves as much as they do: one correction from where it landed
    // puts it where it was dropped.
    if let Some((p0, x0, y0)) = corner(&s.layout(), geom.story, &geom.table)
        && p0 == page
    {
        let fix = |v: f32| if v.is_finite() { v.clamp(-MAX_MOVE, MAX_MOVE) } else { 0.0 };
        let t = s.doc.table_mut(geom.story, &geom.table)?;
        if let Some(f) = t.props.float.as_mut() {
            f.x = fix(f.x + gx - x0);
            f.y = fix(f.y + gy - y0);
        }
        table_resize::touch(s, &geom)?;
    }
    sel_result(s)
}

fn fit_column(s: &mut Session, v: &Value) -> CmdResult {
    let (page, x, y, reach) = point(v)?;
    let l = s.layout();
    let Some((target, line)) = table_resize::resize_target(&l, &s.doc, page, x, y, reach).and_then(|t| match t.edge {
        Edge::Column(line) => Some((t, line)),
        _ => None,
    }) else {
        return Err(CmdError::Failed("there's no column border there".into()));
    };
    let t = s.doc.table(target.story, &target.table).ok_or_else(|| CmdError::Failed("there's no table there".into()))?;
    let g = line.checked_sub(1).ok_or_else(|| CmdError::Failed("there's no column left of that border".into()))?;
    let (narrowest, widest) =
        wordcraft_layout::measure_table_columns(&s.doc, t).get(g).copied().ok_or_else(|| CmdError::Failed("there's no such column".into()))?;
    let current = target.widths().get(g).copied().unwrap_or(0.0);
    // As in Word, the columns right of it keep their widths and move with the border, so the
    // table narrows or widens: up to the right margin (a nested table's cell's), never below the
    // column's narrowest content.
    let room = current + (room_right(&l, &s.doc, page, &target) - target.right()).max(0.0);
    let want = widest.min(room.max(narrowest));
    if let Some(w) = target.plan(want - current, 0.0, true).widths {
        let t = s.doc.table_mut(target.story, &target.table)?;
        table_resize::set_widths(t, w);
        table_resize::touch(s, &target)?;
    }
    sel_result(s)
}

/// Where the room right of table `target` ends on page `page` (page x): the right margin, or
/// for a table nested in a cell the right edge of that cell's text.
fn room_right(l: &DocLayout, doc: &Document, page: usize, target: &ResizeTarget) -> f32 {
    let margin = l.pages.get(page).map_or(target.right(), |pg| pg.body.right());
    let Some((outer, row, cell)) = target.table.cell() else { return margin };
    let Some(t) = doc.table(target.story, &outer) else { return margin };
    let boxes = table_resize::cell_boxes(l, page);
    let Some(b) = boxes.iter().find(|b| b.story == target.story && b.table == outer && b.row == row && b.cell == cell) else {
        return margin;
    };
    // A right-to-left table has its cells' start (left) margin on their right.
    let m = wordcraft_layout::cell_margins(doc, t, row, cell);
    let inset = if t.props.bidi_visual { m[1] } else { m[3] };
    (b.rect.right() - inset).min(margin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wordcraft_doc::props::Align;

    fn table_session(rows: usize, cols: usize) -> Session {
        let mut s = Session::new(Document::new());
        s.run("insert.table", &json!({"rows": rows, "cols": cols})).unwrap();
        s
    }

    fn table_path(s: &Session) -> Path {
        Path(vec![s.doc.body.iter().position(|b| b.as_table().is_some()).unwrap() as u32])
    }

    fn first_table(s: &Session) -> Table {
        s.doc.body.iter().find_map(|b| b.as_table()).cloned().unwrap()
    }

    fn cell_pos(s: &Session, r: usize, c: usize) -> Pos {
        let mut path = table_path(s).0;
        path.extend([r as u32, c as u32, 0]);
        Pos { story: StoryRef::Body, path: Path(path), off: 0 }
    }

    /// Type "r{row}c{cell}" into every cell.
    fn fill(s: &mut Session) {
        let t = first_table(s);
        for (r, row) in t.rows.iter().enumerate() {
            for c in 0..row.cells.len() {
                s.sel = Selection::caret(cell_pos(s, r, c));
                s.run("text.insert", &json!({"text": format!("r{r}c{c}")})).unwrap();
            }
        }
    }

    fn para(s: &Session, r: usize, c: usize) -> wordcraft_doc::Paragraph {
        s.doc.para(StoryRef::Body, &cell_pos(s, r, c).path).cloned().unwrap()
    }

    fn bold(s: &Session, r: usize, c: usize) -> bool {
        let p = para(s, r, c);
        if p.is_empty() { p.mark.bold == Some(true) } else { p.run_ranges().all(|(_, ch)| ch.bold == Some(true)) }
    }

    fn cell_rect(s: &mut Session, row: usize, cell: usize) -> Rect {
        s.layout().pages[0]
            .items
            .iter()
            .find_map(|it| match it {
                Placed::Cell { rect, row: r, cell: c, .. } if *r == row && *c == cell => Some(*rect),
                _ => None,
            })
            .unwrap()
    }

    fn spot_at(s: &mut Session, x: f32, y: f32) -> Option<Spot> {
        let l = s.layout();
        target_at(&l, &s.doc, StoryRef::Body, 0, x, y, REACH).map(|t| t.spot)
    }

    fn mid_y(r: Rect) -> f32 {
        r.y + r.h / 2.0
    }

    /// The areas around and in a table: column, row and cell selection areas, the insert buttons
    /// and the move handle; borders stay the resize module's, the text the caret's.
    #[test]
    fn the_areas_of_a_table_are_found() {
        let mut s = table_session(3, 3);
        let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
        let (left, top) = (a.x, a.y);
        assert_eq!(spot_at(&mut s, b.x + b.w / 2.0, top - 3.0), Some(Spot::Column(1)));
        assert_eq!(spot_at(&mut s, b.x + b.w / 2.0, top + 1.0), Some(Spot::Column(1)));
        let r1 = cell_rect(&mut s, 1, 0);
        assert_eq!(spot_at(&mut s, left - 10.0, mid_y(r1)), Some(Spot::Row(1)));
        let c = cell_rect(&mut s, 1, 2);
        assert_eq!(spot_at(&mut s, c.x + 4.5, mid_y(c)), Some(Spot::Cell(1, 2)));
        assert_eq!(spot_at(&mut s, r1.x + 2.0, mid_y(r1)), Some(Spot::Cell(1, 0)), "the first column's edge has no resize border");
        assert_eq!(spot_at(&mut s, c.x + 20.0, mid_y(c)), None, "the text is the caret's");
        assert_eq!(spot_at(&mut s, b.x, mid_y(a)), None, "a column border resizes");
        assert_eq!(spot_at(&mut s, b.x, top - 8.0), Some(Spot::InsertColumn(1)));
        assert_eq!(spot_at(&mut s, left - 10.0, a.bottom()), Some(Spot::InsertRow(1)));
        assert_eq!(spot_at(&mut s, left - 10.0, top), Some(Spot::InsertRow(0)));
        let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
        assert_eq!(spot_at(&mut s, h.x + h.w / 2.0, h.y + h.h / 2.0), Some(Spot::Handle));
        // Another story's tables aren't in reach.
        let l = s.layout();
        assert!(target_at(&l, &s.doc, StoryRef::Part(7), 0, b.x + b.w / 2.0, top - 3.0, REACH).is_none());
    }

    /// A selected column's text is its own cells', so the status bar and Word Count count only
    /// them, not the cells between its ends in the text.
    #[test]
    fn a_selected_columns_text_is_its_cells() {
        let mut s = table_session(3, 3);
        fill(&mut s);
        let b = cell_rect(&mut s, 0, 1);
        s.run("table.selectAt", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": b.y - 3.0})).unwrap();
        assert_eq!(s.selected_text(), "r0c1\nr1c1\nr2c1");
        assert_eq!(s.run("review.wordCount", &json!({})).unwrap()["words"], 3);
    }

    /// A click above a column selects just its cells: formatting, alignment and Delete leave the
    /// cells between its ends alone, each one undo step; table commands take the column.
    #[test]
    fn a_selected_column_is_edited_cell_by_cell() {
        let mut s = table_session(3, 3);
        fill(&mut s);
        let b = cell_rect(&mut s, 0, 1);
        s.run("table.selectAt", &json!({"page": 0, "x": b.x + b.w / 2.0, "y": b.y - 3.0})).unwrap();
        let range = s.cell_range().cloned().unwrap();
        assert_eq!((range.rows, range.cols, range.stream), ((0, 2), (1, 1), false));
        assert_eq!(s.column_segments().unwrap().len(), 3);
        s.run("format.bold", &json!({})).unwrap();
        for r in 0..3 {
            assert!(bold(&s, r, 1), "row {r} of the column is bold");
            assert!(!bold(&s, r, 0) && !bold(&s, r, 2), "row {r}'s other cells aren't");
        }
        s.run("para.alignCenter", &json!({})).unwrap();
        assert_eq!(para(&s, 1, 1).props.align, Some(Align::Center));
        assert_eq!(para(&s, 0, 2).props.align, None, "the cells between the corners keep theirs");
        assert!(s.cell_range().is_some(), "formatting keeps the selection");
        s.run("table.shading", &json!({"color": "FF0000"})).unwrap();
        let t = first_table(&s);
        assert!(t.rows.iter().all(|row| row.cells[1].props.shading.is_some() && row.cells[0].props.shading.is_none()));
        let depth = s.undo_depth();
        s.run("text.delete", &json!({})).unwrap();
        assert_eq!(s.undo_depth(), depth + 1);
        for r in 0..3 {
            assert!(para(&s, r, 1).is_empty());
            assert_eq!(para(&s, r, 0).plain_text(), format!("r{r}c0"));
            assert_eq!(para(&s, r, 2).plain_text(), format!("r{r}c2"));
        }
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(para(&s, 2, 1).plain_text(), "r2c1");
        assert_eq!(s.cell_range().map(|r| r.cols), Some((1, 1)), "undo brings the column selection back");
    }

    /// Numbering a column numbers its cells as one list, like Word.
    #[test]
    fn numbering_a_column_makes_one_list() {
        let mut s = table_session(3, 2);
        fill(&mut s);
        s.sel = Selection::caret(cell_pos(&s, 1, 0));
        s.run("table.selectColumn", &json!({})).unwrap();
        s.run("para.numbering", &json!({})).unwrap();
        let nums: Vec<_> = (0..3).map(|r| para(&s, r, 0).props.numbering.map(|n| n.num)).collect();
        assert!(nums[0].is_some_and(|n| n != 0) && nums.iter().all(|n| *n == nums[0]), "{nums:?}");
        assert!((0..3).all(|r| para(&s, r, 1).props.numbering.is_none()));
    }

    /// An empty column's cells take character formatting in their marks; Tab moves on without
    /// clearing a selected column; a click in place ends a selected empty cell.
    #[test]
    fn empty_cells_tab_and_clicks() {
        let mut s = table_session(2, 2);
        s.sel = Selection::caret(cell_pos(&s, 0, 0));
        s.run("table.selectColumn", &json!({})).unwrap();
        s.run("format.bold", &json!({})).unwrap();
        assert!(bold(&s, 0, 0) && bold(&s, 1, 0) && !bold(&s, 0, 1) && !bold(&s, 1, 1));
        fill(&mut s);
        s.sel = Selection::caret(cell_pos(&s, 0, 1));
        s.run("table.selectColumn", &json!({})).unwrap();
        s.run("text.tab", &json!({})).unwrap();
        assert_eq!(para(&s, 0, 1).plain_text(), "r0c1");
        assert_eq!(para(&s, 1, 1).plain_text(), "r1c1");
        // A selected empty cell's corners are one place; clicking there ends the selection.
        let mut s = table_session(2, 2);
        let c = cell_rect(&mut s, 1, 1);
        s.run("table.selectAt", &json!({"page": 0, "x": c.x + 4.5, "y": mid_y(c)})).unwrap();
        assert!(s.cell_range().is_some() && s.sel.is_collapsed());
        let at = serde_json::to_value(s.sel.focus.clone()).unwrap();
        s.run("caret.set", &json!({"pos": at})).unwrap();
        assert!(s.cell_range().is_none());
    }

    /// Rows, cells and drags: whole rows and cells of one row are an ordinary stream selection;
    /// a drag over cells of several rows selects their rectangle only.
    #[test]
    fn rows_cells_and_drags_select_rectangles() {
        let mut s = table_session(3, 3);
        fill(&mut s);
        let r1 = cell_rect(&mut s, 1, 0);
        s.run("table.selectAt", &json!({"page": 0, "x": r1.x - 10.0, "y": mid_y(r1)})).unwrap();
        let range = s.cell_range().cloned().unwrap();
        assert_eq!((range.rows, range.cols, range.stream), ((1, 1), (0, 2), true));
        assert!(s.column_segments().is_none(), "a row is one run of the text");
        let r2 = cell_rect(&mut s, 2, 0);
        s.run("table.selectAt", &json!({"page": 0, "x": r1.x - 10.0, "y": mid_y(r1), "toX": r1.x - 10.0, "toY": mid_y(r2) + 50.0})).unwrap();
        assert_eq!(s.cell_range().map(|r| r.rows), Some((1, 2)));
        // From cell (0, 0) to cell (1, 1): four cells, not the row between.
        let (a, d) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 1, 1));
        s.run("table.selectAt", &json!({"page": 0, "x": a.x + 2.0, "y": mid_y(a), "toX": d.x + d.w / 2.0, "toY": mid_y(d)})).unwrap();
        let range = s.cell_range().cloned().unwrap();
        assert_eq!((range.rows, range.cols, range.stream), ((0, 1), (0, 1), false));
        s.run("format.italic", &json!({})).unwrap();
        let italic = |s: &Session, r, c| para(s, r, c).run_ranges().all(|(_, ch)| ch.italic == Some(true));
        assert!(italic(&s, 0, 0) && italic(&s, 1, 1) && !italic(&s, 0, 2) && !italic(&s, 1, 2));
        // Dragging across columns: from the first to the last is every column, all rows.
        let (c0, c2) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 2));
        s.run("table.selectAt", &json!({"page": 0, "x": c0.x + 20.0, "y": c0.y - 3.0, "toX": c2.right() + 30.0, "toY": c0.y})).unwrap();
        let range = s.cell_range().cloned().unwrap();
        assert_eq!((range.rows, range.cols, range.stream), ((0, 2), (0, 2), true));
        // The move handle selects the table.
        let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
        s.run("table.selectAt", &json!({"page": 0, "x": h.x + 4.0, "y": h.y + 4.0})).unwrap();
        assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 2), (0, 2))));
        // The ribbon's Select Row is the same whole-cell selection.
        s.sel = Selection::caret(cell_pos(&s, 2, 1));
        s.run("table.selectRow", &json!({})).unwrap();
        assert_eq!(s.cell_range().map(|r| (r.rows, r.cols, r.stream)), Some(((2, 2), (0, 2), true)));
    }

    /// The page margin beside a table is a row selection area too, as far as the selection bar
    /// reaches (Word): left of a left-to-right table, right of a right-to-left one.
    #[test]
    fn the_margin_beside_a_row_selects_it() {
        let mut s = table_session(3, 3);
        let r1 = cell_rect(&mut s, 1, 0);
        let body_x = s.layout().pages[0].body.x;
        assert!(r1.x - 40.0 > 0.0 && r1.x - body_x < 24.0);
        // Far past the 24pt of the plain row area, still in the margin: the row.
        assert_eq!(spot_at(&mut s, 10.0, mid_y(r1)), Some(Spot::Row(1)));
        assert_eq!(spot_at(&mut s, body_x - 35.0, mid_y(r1)), Some(Spot::Row(1)));
        // The right margin is not a row bar of a left-to-right table.
        let right = s.layout().pages[0].body.right();
        assert_eq!(spot_at(&mut s, right + 20.0, mid_y(r1)), None);
        // A right-to-left table has its bars on the right.
        let tp = table_path(&s);
        s.doc.table_mut(StoryRef::Body, &tp).unwrap().props.bidi_visual = true;
        s.touch();
        let r1 = cell_rect(&mut s, 1, 0);
        let (page_w, right) = (s.layout().pages[0].w, s.layout().pages[0].body.right());
        assert_eq!(spot_at(&mut s, right + 30.0, mid_y(r1)), Some(Spot::Row(1)));
        assert_eq!(spot_at(&mut s, page_w - 5.0, mid_y(r1)), Some(Spot::Row(1)));
        assert_eq!(spot_at(&mut s, 10.0, mid_y(r1)), None);
        // Shift+click in the bar extends the selected rows from the anchor row.
        s.run("table.selectAt", &json!({"page": 0, "x": right + 30.0, "y": mid_y(r1)})).unwrap();
        let r2 = cell_rect(&mut s, 2, 0);
        s.run("table.selectAt", &json!({"page": 0, "x": right + 30.0, "y": mid_y(r2), "extend": true})).unwrap();
        assert_eq!(s.cell_range().map(|r| r.rows), Some((1, 2)));
        let r0 = cell_rect(&mut s, 0, 0);
        s.run("table.selectAt", &json!({"page": 0, "x": right + 30.0, "y": mid_y(r0), "extend": true})).unwrap();
        assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 2)), "the far edge of the old rows stays");
    }

    /// `anchorRow` fixes the row a Shift+click extends from, whatever is selected (a drag after
    /// an upward Shift+click keeps its rows); out of range it's the last row.
    #[test]
    fn an_anchor_row_param_fixes_the_extend_anchor() {
        let mut s = table_session(3, 3);
        let (r0, r2) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 2, 0));
        let x = r0.x - 10.0;
        s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r2)})).unwrap();
        s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r0), "extend": true})).unwrap();
        assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 2)));
        // Without it, the next frame of the drag would re-read the anchor as row 0.
        s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r0), "toX": x, "toY": mid_y(r0) + 1.0, "extend": true, "anchorRow": 2}))
            .unwrap();
        assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 2)), "the press's anchor stays");
        s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r0), "extend": true, "anchorRow": 99999999999u64})).unwrap();
        assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 2)), "clamped to the last row");
        s.run("table.selectAt", &json!({"page": 0, "x": x, "y": mid_y(r0), "extend": true, "anchorRow": 1})).unwrap();
        assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 1)));
    }

    /// The margin is the row bar only where the table is what's beside it: not at a column-2
    /// table's height beside column 1's lines (the selection bar's lines), nor beside text
    /// wrapped round a floating table.
    #[test]
    fn the_row_bar_needs_the_table_beside_the_margin() {
        // A two-column section: 36 short lines fill column 1 and start column 2, then a table.
        let mut s = Session::new(Document::new());
        s.run("layout.columns", &json!({"count": 2})).unwrap();
        let lines: Vec<String> = (0..36).map(|i| format!("Line {i}")).collect();
        s.run("text.insert", &json!({"text": format!("{}\r", lines.join("\r"))})).unwrap();
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        let r0 = cell_rect(&mut s, 0, 0);
        let body = s.layout().pages[0].body;
        assert!(r0.x > body.x + body.w / 2.0, "the table is in column 2: {r0:?}");
        assert_eq!(spot_at(&mut s, 10.0, mid_y(r0)), None, "column 1's line is the selection bar's");
        // A floating table with text wrapped to its left.
        let mut d = Document::from_text(&"Words that wrap round the floating table beside them. ".repeat(40));
        let mut t = Table::new(3, 1, 150.0);
        t.props.float = Some(wordcraft_doc::props::TableFloat {
            h_rel: wordcraft_doc::para::Anchor::Margin,
            v_rel: wordcraft_doc::para::Anchor::Paragraph,
            x: 300.0,
            overlap: false,
            ..Default::default()
        });
        d.insert_block(StoryRef::Body, &Path(vec![0]), wordcraft_doc::Block::Table(t)).unwrap();
        let mut s = Session::new(d);
        let r0 = cell_rect(&mut s, 0, 0);
        assert!(r0.x > 300.0, "the table floats at the right: {r0:?}");
        assert_eq!(spot_at(&mut s, 10.0, mid_y(r0)), None, "the text wrapped beside it has the line bar");
    }

    /// A paragraph right above the table (no space after) doesn't hide the first row's bar.
    #[test]
    fn a_paragraph_flush_above_the_table_keeps_the_row_bar() {
        let mut d = Document::from_text("Above");
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.space_after = Some(0.0);
        d.insert_block(StoryRef::Body, &Path::top(1), wordcraft_doc::Block::Table(Table::new(2, 2, 400.0))).unwrap();
        let mut s = Session::new(d);
        let r0 = cell_rect(&mut s, 0, 0);
        assert_eq!(spot_at(&mut s, 10.0, mid_y(r0)), Some(Spot::Row(0)));
        assert_eq!(spot_at(&mut s, 10.0, r0.y + 0.3), Some(Spot::Row(0)), "even at the very top of the row");
    }

    /// The insert buttons add a row or column at their border, one undo step, and select it (as
    /// Word does); typing then goes into its first cell.
    #[test]
    fn the_insert_buttons_add_rows_and_columns() {
        let mut s = table_session(2, 3);
        fill(&mut s);
        let a = cell_rect(&mut s, 0, 0);
        s.run("table.insertAt", &json!({"page": 0, "x": a.x - 10.0, "y": a.bottom()})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows.len(), 3);
        assert!(para(&s, 1, 0).is_empty() && para(&s, 2, 0).plain_text() == "r1c0", "the new row is the second");
        assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 1), (0, 2))), "it's selected");
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert_eq!(para(&s, 1, 0).plain_text(), "x");
        assert_eq!((first_table(&s).rows.len(), para(&s, 2, 0).plain_text()), (3, "r1c0".to_string()));
        s.run("edit.undo", &json!({})).unwrap();
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(first_table(&s).rows.len(), 2);
        // Above the first row.
        s.run("table.insertAt", &json!({"page": 0, "x": a.x - 10.0, "y": a.y})).unwrap();
        assert!(para(&s, 0, 0).is_empty() && para(&s, 1, 0).plain_text() == "r0c0");
        assert_eq!(s.cell_range().map(|r| r.rows), Some((0, 0)));
        s.run("edit.undo", &json!({})).unwrap();
        // A column at the border between the first two, the table keeping its width.
        let width: f32 = first_table(&s).grid.iter().sum();
        let b = cell_rect(&mut s, 0, 1);
        s.run("table.insertAt", &json!({"page": 0, "x": b.x, "y": a.y - 8.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.cols(), 4);
        assert!(para(&s, 0, 1).is_empty() && para(&s, 0, 2).plain_text() == "r0c1");
        assert!((t.grid.iter().sum::<f32>() - width).abs() < 0.5);
        assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (1, 1))), "it's selected");
        // And at the right edge.
        let e = cell_rect(&mut s, 0, 3);
        s.run("table.insertAt", &json!({"page": 0, "x": e.right(), "y": a.y - 8.0})).unwrap();
        assert_eq!(first_table(&s).cols(), 5);
        assert!(para(&s, 0, 4).is_empty() && para(&s, 0, 3).plain_text() == "r0c2");
        assert_eq!(s.cell_range().map(|r| r.cols), Some((4, 4)));
    }

    /// Dragging the move handle floats the table where it's dropped; one undo puts it back; a
    /// click on the handle moves nothing.
    #[test]
    fn dragging_the_handle_moves_the_table() {
        let mut s = table_session(2, 2);
        let a = cell_rect(&mut s, 0, 0);
        let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
        let (hx, hy) = (h.x + h.w / 2.0, h.y + h.h / 2.0);
        s.run("table.moveDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx, "toY": hy})).unwrap();
        assert!(first_table(&s).props.float.is_none(), "a click doesn't move it");
        s.run("table.moveDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx + 50.0, "toY": hy + 30.0})).unwrap();
        let f = first_table(&s).props.float.unwrap();
        assert_eq!((f.h_rel, f.v_rel, f.overlap), (Anchor::Column, Anchor::Paragraph, true));
        let moved = cell_rect(&mut s, 0, 0);
        assert!((moved.x - (a.x + 50.0)).abs() < 0.5 && (moved.y - (a.y + 30.0)).abs() < 0.5, "{a:?} → {moved:?}");
        // Moving it again starts from where it floats.
        let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
        let (hx, hy) = (h.x + h.w / 2.0, h.y + h.h / 2.0);
        s.run("table.moveDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx - 20.0, "toY": hy + 10.0})).unwrap();
        let again = cell_rect(&mut s, 0, 0);
        assert!((again.x - (moved.x - 20.0)).abs() < 0.5 && (again.y - (moved.y + 10.0)).abs() < 0.5, "{moved:?} → {again:?}");
        s.run("edit.undo", &json!({})).unwrap();
        s.run("edit.undo", &json!({})).unwrap();
        assert!(first_table(&s).props.float.is_none());
        assert_eq!(cell_rect(&mut s, 0, 0), a);
        // Nested tables and tables outside the main text stay where they are.
        let mut nested = table_resize::tables_on(&s.layout(), &s.doc, 0)[0].clone();
        assert!(movable(&nested));
        nested.table = Path(vec![0, 0, 0, 1]);
        assert!(!movable(&nested));
    }

    /// A double-click on a column border fits the column left of it to its text; as in Word, the
    /// columns right of it keep their widths and move along, so the table narrows. The right
    /// edge's column fits the same way.
    #[test]
    fn a_double_click_on_a_border_fits_the_column() {
        let mut s = table_session(1, 3);
        s.sel = Selection::caret(cell_pos(&s, 0, 0));
        s.run("text.insert", &json!({"text": "ab"})).unwrap();
        let (a, b, c) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1), cell_rect(&mut s, 0, 2));
        let fit = wordcraft_layout::measure_table_columns(&s.doc, &first_table(&s));
        s.run("table.fitColumn", &json!({"page": 0, "x": a.right(), "y": mid_y(a)})).unwrap();
        let (a1, b1, c1) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1), cell_rect(&mut s, 0, 2));
        assert!((a1.w - fit[0].1).abs() < 0.5 && a1.w < a.w - 50.0, "{a:?} → {a1:?}, {fit:?}");
        assert!((b1.w - b.w).abs() < 0.5 && (c1.w - c.w).abs() < 0.5, "the columns right of it keep their widths");
        assert!((b1.x - b.x - (a1.w - a.w)).abs() < 0.5 && (c1.right() - c.right() - (a1.w - a.w)).abs() < 0.5, "and move along");
        s.run("table.fitColumn", &json!({"page": 0, "x": c1.right(), "y": mid_y(c1)})).unwrap();
        let c2 = cell_rect(&mut s, 0, 2);
        assert!((c2.w - fit[2].1).abs() < 0.5 && c2.w < c.w, "{c:?} → {c2:?}");
    }

    /// A nested table's column fits within the cell around the table: the table widens only to
    /// the right edge of that cell's text, not to the page's right margin, and the table around
    /// it keeps its columns.
    #[test]
    fn a_nested_tables_column_fits_within_its_cell() {
        let mut s = table_session(1, 2);
        s.sel = Selection::caret(cell_pos(&s, 0, 0));
        s.run("insert.table", &json!({"rows": 1, "cols": 2})).unwrap();
        let (inner, _, _) = s.sel.focus.path.cell().unwrap();
        let rect = |s: &mut Session, table: &Path, cell: usize| {
            s.layout().pages[0]
                .items
                .iter()
                .find_map(|it| match it {
                    Placed::Cell { rect, table: t, row: 0, cell: c, .. } if t == table && *c == cell => Some(*rect),
                    _ => None,
                })
                .unwrap()
        };
        // Its empty columns narrow first, leaving room right of it in its cell.
        for col in [1, 0] {
            let r = rect(&mut s, &inner, col);
            s.run("table.fitColumn", &json!({"page": 0, "x": r.right(), "y": mid_y(r)})).unwrap();
        }
        s.run("text.insert", &json!({"text": "word ".repeat(40)})).unwrap();
        let outer = table_path(&s);
        let host = rect(&mut s, &outer, 0);
        let (a, b) = (rect(&mut s, &inner, 0), rect(&mut s, &inner, 1));
        assert!(b.right() < host.right() - 40.0, "{b:?} in {host:?}");
        s.run("table.fitColumn", &json!({"page": 0, "x": a.right(), "y": mid_y(a)})).unwrap();
        let (a1, b1) = (rect(&mut s, &inner, 0), rect(&mut s, &inner, 1));
        let text_right = host.right() - wordcraft_layout::cell_margins(&s.doc, &first_table(&s), 0, 0)[3];
        assert!(a1.w > a.w + 40.0 && (b1.w - b.w).abs() < 0.5, "{a:?} {b:?} → {a1:?} {b1:?}");
        assert!((b1.right() - text_right).abs() < 0.5, "the table fills its cell's text: {b1:?}, {text_right}");
        let host1 = rect(&mut s, &outer, 0);
        assert_eq!((host1.x, host1.w), (host.x, host.w), "the table around it keeps its columns");
    }

    /// Pasting formatting onto a selected column leaves its text (it used to clear it).
    #[test]
    fn the_format_painter_keeps_a_selected_columns_text() {
        let mut s = table_session(2, 2);
        fill(&mut s);
        s.sel = Selection::caret(cell_pos(&s, 0, 0));
        s.run("edit.formatPainter", &json!({})).unwrap();
        s.sel = Selection::caret(cell_pos(&s, 0, 1));
        s.run("table.selectColumn", &json!({})).unwrap();
        s.run("edit.pasteFormat", &json!({})).unwrap();
        assert_eq!(para(&s, 0, 1).plain_text(), "r0c1");
        assert_eq!(para(&s, 1, 1).plain_text(), "r1c1");
    }

    /// Where the app extends the selection while dragging (`caret.set` with `extend`).
    fn drag_to(s: &mut Session, to: Pos) {
        s.run("caret.set", &json!({"pos": to, "extend": true})).unwrap();
    }

    /// An ordinary drag from one cell into another selects the rectangle of whole cells between
    /// them, so bold takes just those, as in Word; back in the first cell it's text again, from
    /// where the drag started. Out of a nested table, the cells are the table's around it.
    #[test]
    fn a_drag_across_cells_selects_their_rectangle() {
        let mut s = table_session(3, 3);
        fill(&mut s);
        let start = Pos { off: 1, ..cell_pos(&s, 0, 1) };
        s.run("caret.set", &json!({"pos": start})).unwrap();
        let down = Pos { off: 2, ..cell_pos(&s, 2, 1) };
        drag_to(&mut s, down);
        let range = s.cell_range().cloned().unwrap();
        assert_eq!((range.rows, range.cols, range.stream), ((0, 2), (1, 1), false));
        s.run("format.bold", &json!({})).unwrap();
        for r in 0..3 {
            assert!(bold(&s, r, 1) && !bold(&s, r, 0) && !bold(&s, r, 2), "row {r}: just the dragged column");
        }
        // On to the left: the rectangle still runs from the cell the drag started in.
        let left = cell_pos(&s, 1, 0);
        drag_to(&mut s, left);
        assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 1))));
        // Back in that cell: its text, from where the drag started.
        let back = Pos { off: 3, ..cell_pos(&s, 0, 1) };
        drag_to(&mut s, back.clone());
        assert!(s.cell_range().is_none());
        assert_eq!(s.sel, Selection { anchor: start, focus: back });
        // From a nested table's cell into another cell of the table around it.
        let mut s = table_session(2, 2);
        s.sel = Selection::caret(cell_pos(&s, 0, 0));
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        assert_eq!(s.sel.focus.path.0.len(), 7, "the caret is in the nested table");
        let to = cell_pos(&s, 1, 1);
        drag_to(&mut s, to);
        let range = s.cell_range().cloned().unwrap();
        assert_eq!((range.table, range.rows, range.cols), (table_path(&s), (0, 1), (0, 1)));
    }

    /// Shift+Down from a cell takes it and the cell below, whole; on past the table, the anchor's
    /// row and every row after it, then the text; Shift+Up comes back to the cells, then into the
    /// first cell's text. Shift+click goes on from where the selection started, and from text
    /// before the table a drag into a cell takes the rows down to it, whole.
    #[test]
    fn shift_arrows_and_clicks_select_cells_then_rows() {
        let mut s = Session::new(Document::new());
        s.run("text.insert", &json!({"text": "Intro"})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("insert.table", &json!({"rows": 3, "cols": 3})).unwrap();
        fill(&mut s);
        let after = table_path(&s).last() + 1;
        let shift = |s: &mut Session, id: &str| {
            s.run(id, &json!({"extend": true})).unwrap();
        };
        s.sel = Selection::caret(cell_pos(&s, 1, 1));
        shift(&mut s, "caret.down");
        assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 2), (1, 1))));
        shift(&mut s, "caret.down");
        assert!(s.cell_range().is_none());
        assert_eq!(s.sel.anchor, cell_pos(&s, 1, 0), "from the start of the anchor's row");
        assert_eq!(s.sel.focus.path, Path::top(after));
        s.run("format.italic", &json!({})).unwrap();
        let italic = |s: &Session, r, c| para(s, r, c).run_ranges().all(|(_, ch)| ch.italic == Some(true));
        assert!(italic(&s, 1, 0) && italic(&s, 2, 2) && !italic(&s, 0, 1), "whole rows from the anchor's");
        shift(&mut s, "caret.up");
        assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((1, 2), (1, 1))));
        shift(&mut s, "caret.up");
        assert!(s.cell_range().is_none());
        assert_eq!(s.sel.anchor, cell_pos(&s, 1, 1));
        assert_eq!(s.sel.focus.path, cell_pos(&s, 1, 1).path);
        // Shift+click: the rectangle from the anchor's cell.
        let corner = cell_pos(&s, 0, 0);
        s.run("caret.set", &json!({"pos": corner, "extend": true})).unwrap();
        assert_eq!(s.cell_range().map(|r| (r.rows, r.cols)), Some(((0, 1), (0, 1))));
        // From the text before the table into a cell: the rows down to that cell's, whole.
        s.sel = Selection::caret(Pos::body(0, 2));
        let into = cell_pos(&s, 1, 1);
        drag_to(&mut s, into);
        assert!(s.cell_range().is_none());
        assert_eq!(s.sel, Selection { anchor: Pos::body(0, 2), focus: Pos { off: 4, ..cell_pos(&s, 1, 2) } });
    }

    /// A drag from the text before a table into it takes whole rows, and they shade every cell
    /// of them (an empty one too), not just their letters; the rows it doesn't take and a table
    /// both ends are in shade their text.
    #[test]
    fn a_selection_into_a_table_shades_its_rows_whole_cells() {
        let mut s = Session::new(Document::new());
        s.run("text.insert", &json!({"text": "Intro"})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("insert.table", &json!({"rows": 3, "cols": 2})).unwrap();
        fill(&mut s);
        let emptied = cell_pos(&s, 0, 1);
        s.sel = Selection { anchor: emptied.clone(), focus: Pos { off: 4, ..emptied } };
        s.run("text.delete", &json!({})).unwrap();
        assert!(para(&s, 0, 1).is_empty());
        s.sel = Selection::caret(Pos::body(0, 2));
        let into = cell_pos(&s, 1, 0);
        drag_to(&mut s, into);
        let (a, b) = s.sel.ordered();
        let l = s.layout();
        let shaded = selection_rects(&l, &s.doc, &a, &b, 0);
        for r in 0..3 {
            for c in 0..2 {
                let cell = cell_rect(&mut s, r, c);
                assert_eq!(shaded.iter().filter(|(_, x)| *x == cell).count(), usize::from(r < 2), "cell {r},{c}");
            }
        }
        let first = cell_rect(&mut s, 0, 0);
        let on = |x: &Rect| first.contains(Point::new(x.x + x.w / 2.0, x.y + x.h / 2.0));
        assert_eq!(shaded.iter().filter(|(_, x)| on(x)).count(), 1, "the cell, its letters not again");
        assert!(shaded.iter().any(|(_, x)| x.bottom() <= first.y), "and the text before the table");
        // Both ends in the table (here a whole row of it): its text.
        let (start, end) = table::row_ends(&s.doc, StoryRef::Body, &table_path(&s), 2).unwrap();
        assert_eq!(selection_rects(&l, &s.doc, &start, &end, 0), l.selection_rects(&s.doc, &start, &end, 0));
    }

    /// Clicks away from the areas, and hostile numbers: an error and no change, never a panic.
    #[test]
    fn misses_and_hostile_params_change_nothing() {
        let mut s = table_session(2, 2);
        let start = first_table(&s);
        let c = cell_rect(&mut s, 1, 1);
        let mid = json!({"page": 0, "x": c.x + c.w / 2.0, "y": mid_y(c), "toX": 1, "toY": 1});
        for id in ["table.selectAt", "table.insertAt", "table.moveDrag", "table.fitColumn"] {
            assert!(s.run(id, &mid).is_err(), "{id}");
            assert!(s.run(id, &json!({"page": 99, "x": 1, "y": 1, "toX": 1, "toY": 1})).is_err(), "{id}");
            assert!(s.run(id, &json!({"page": 0, "x": 1e30, "y": -1e30, "toX": 1e30, "toY": 1e30, "reach": 1e30})).is_err(), "{id}");
            assert!(s.run(id, &json!({})).is_err(), "{id}");
        }
        // An insert button isn't a selection area, nor a selection area a button.
        let a = cell_rect(&mut s, 0, 0);
        assert!(s.run("table.selectAt", &json!({"page": 0, "x": a.x - 10.0, "y": a.bottom()})).is_err());
        assert!(s.run("table.insertAt", &json!({"page": 0, "x": a.x - 10.0, "y": mid_y(a)})).is_err());
        assert_eq!(first_table(&s), start);
        // A drag to the ends of the earth keeps the table on its page.
        let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
        s.run("table.moveDrag", &json!({"page": 0, "x": h.x + 4.0, "y": h.y + 4.0, "toX": 3e38, "toY": -3e38})).unwrap();
        let f = first_table(&s).props.float.unwrap();
        assert!(f.x.is_finite() && f.y.is_finite());
        let moved = cell_rect(&mut s, 0, 0);
        assert!(moved.x >= -0.5 && moved.right() <= 612.5 && moved.y >= -0.5, "{moved:?}");
    }

    /// #2038: a table dragged to the right floats and the empty paragraph after it moves up beside
    /// it. As in Word, a click anywhere below the table then goes to that paragraph, not to a cell of the last row; a click in a cell still lands there.
    #[test]
    fn a_click_below_a_floating_table_goes_to_the_text_not_its_cells() {
        let mut s = table_session(3, 3);
        let h = handle(&table_resize::tables_on(&s.layout(), &s.doc, 0)[0]);
        let (hx, hy) = (h.x + h.w / 2.0, h.y + h.h / 2.0);
        s.run("table.moveDrag", &json!({"page": 0, "x": hx, "y": hy, "toX": hx + 60.0, "toY": hy + 10.0})).unwrap();
        assert!(first_table(&s).props.float.is_some());
        let after = Path(vec![table_path(&s).0[0] + 1]);
        assert!(s.doc.block(StoryRef::Body, &after).is_some_and(|b| b.as_table().is_none()), "a paragraph follows the table");
        let (first, last) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 2, 2));
        let (mid, bottom, margin) = ((first.x + last.right()) / 2.0, last.bottom(), s.layout().pages[0].body.x + 5.0);
        let at = |s: &mut Session, x: f32, y: f32| {
            s.run("caret.set", &json!({"page": 0, "x": x, "y": y})).unwrap();
            s.sel.focus.path.clone()
        };
        for (name, x, y) in [
            ("just below the middle", mid, bottom + 12.0),
            ("far below the middle", mid, bottom + 200.0),
            ("below, at the margin", margin, bottom + 12.0),
        ] {
            assert_eq!(at(&mut s, x, y), after, "a click {name}");
        }
        let b2 = cell_rect(&mut s, 1, 1);
        assert_eq!(at(&mut s, b2.x + b2.w / 2.0, mid_y(b2)), cell_pos(&s, 1, 1).path, "in B2");
        assert_eq!(at(&mut s, last.x + last.w / 2.0, last.bottom() - 1.0), cell_pos(&s, 2, 2).path, "low in C3");
        s.run("caret.docEnd", &json!({})).unwrap();
        assert_eq!(s.sel.focus.path, after, "Ctrl+End");
    }
}
