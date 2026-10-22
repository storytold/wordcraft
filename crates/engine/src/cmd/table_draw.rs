//! Table Layout › Draw: Draw Table and Eraser. A pen stroke on a page either draws a new one-cell
//! table (a rectangle outside any table) or splits the cells a straight line crosses; the eraser
//! removes the borders a click or a drag touches: the two cells beside a border merge when they
//! make one rectangle, else it is hidden (as the table's outside always is). Coordinates are page
//! points, like the layout's; the app turns the pen and eraser on as canvas modes and sends each
//! finished stroke or click here, so every stroke is one undo step.

use serde_json::Value;
use wordcraft_doc::props::{HeightRule, VMerge};
use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Block, Cell, Document, Path, Pos, StoryRef, Table, para_block};
use wordcraft_geom::Rect;
use wordcraft_layout::{DocLayout, Placed};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// A drag shorter than this (points) both ways isn't a rectangle.
pub const MIN_RECT: f32 = 12.0;
/// How near (points) a click has to be to a border when the command doesn't say (`reach`).
pub const ERASER_REACH: f32 = 4.0;
/// The farthest `reach` (points): the app's six screen points at its smallest page scale (0.05
/// screen points per point, One Page or Multiple Pages in a small window).
pub const ERASER_MAX_REACH: f32 = 120.0;
/// A line closer than this (points) to an existing border lands on it.
const SNAP: f32 = 3.0;
/// The shortest stroke (points) that counts as a line.
const MIN_LINE: f32 = 6.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("table.draw", "Draw Table", "Insert › Tables", draw).params(
            r#"{"page": n, "x0": pt, "y0": pt, "x1": pt, "y1": pt, "story"?: StoryRef} (a pen stroke in page points, 0-based page: a rectangle at least 12 pt each way outside any table inserts a one-cell table of that size at the paragraph under its start; a straight vertical or horizontal line inside a table splits the cells it crosses there. In the app the button turns the pen on.)"#,
        ),
        CommandSpec::new("table.eraser", "Eraser", "Table Layout › Draw", eraser).params(
            r#"{"page": n, "x": pt, "y": pt} or {"page": n, "x0": pt, "y0": pt, "x1": pt, "y1": pt}, "reach"?: pt (a click erases the border within `reach` (default 4, at most 120) of the point, a border between two cells before the table's outside; a drag erases every border the rectangle from (x0, y0) to (x1, y1) touches, horizontal lines first. Two cells that make one rectangle merge, including cells merged down the same rows; otherwise the border is hidden: along the whole edge for a vertical line, where the two cells meet for a horizontal one, and always on the table's outside. In the app the button turns the eraser on.)"#,
        ),
    ]
}

/// What a pen stroke does, judged from the layout (also the app's live preview).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stroke {
    /// Draw a new table in this rectangle (page points).
    Rect(Rect),
    /// Split the cells the vertical line at `x` crosses between `y0` and `y1`.
    Vertical { x: f32, y0: f32, y1: f32 },
    /// Split the cells the horizontal line at `y` crosses between `x0` and `x1`.
    Horizontal { y: f32, x0: f32, x1: f32 },
}

/// A table cell's area on a page.
#[derive(Clone, Debug)]
struct CellBox {
    rect: Rect,
    table: Path,
    story: StoryRef,
    row: usize,
    cell: usize,
}

fn cell_boxes(l: &DocLayout, page: usize) -> Vec<CellBox> {
    let Some(pg) = l.pages.get(page) else { return Vec::new() };
    pg.items
        .iter()
        .filter_map(|it| match it {
            Placed::Cell { rect, table, row, cell, story, .. } => {
                Some(CellBox { rect: *rect, table: table.clone(), story: *story, row: *row, cell: *cell })
            }
            _ => None,
        })
        .collect()
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.right() && y >= r.y && y <= r.bottom()
}

/// The innermost table with a cell under the point.
fn table_at(boxes: &[CellBox], x: f32, y: f32) -> Option<(StoryRef, Path)> {
    boxes.iter().filter(|b| inside(&b.rect, x, y)).max_by_key(|b| b.table.0.len()).map(|b| (b.story, b.table.clone()))
}

/// Classify a pen stroke: a straight line inside a table, a rectangle outside one, else `None`.
pub fn stroke(l: &DocLayout, page: usize, x0: f32, y0: f32, x1: f32, y1: f32) -> Option<Stroke> {
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
    let boxes = cell_boxes(l, page);
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    if table_at(&boxes, x0, y0).is_some() || table_at(&boxes, mx, my).is_some() {
        if dy >= dx && dy >= MIN_LINE && dx <= MIN_RECT.max(dy * 0.2) {
            return Some(Stroke::Vertical { x: mx, y0: y0.min(y1), y1: y0.max(y1) });
        }
        if dx > dy && dx >= MIN_LINE && dy <= MIN_RECT.max(dx * 0.2) {
            return Some(Stroke::Horizontal { y: my, x0: x0.min(x1), x1: x0.max(x1) });
        }
        return None;
    }
    (dx >= MIN_RECT && dy >= MIN_RECT).then(|| Stroke::Rect(Rect::new(x0.min(x1), y0.min(y1), dx, dy)))
}

fn page_param(v: &Value) -> Result<usize, CmdError> {
    p::u64(v, "page").map(|n| usize::try_from(n).unwrap_or(usize::MAX)).ok_or_else(|| CmdError::Params("`page` (number) is required".into()))
}

fn draw(s: &mut Session, v: &Value) -> CmdResult {
    let page = page_param(v)?;
    let (x0, y0, x1, y1) = (p::req_f32(v, "x0")?, p::req_f32(v, "y0")?, p::req_f32(v, "x1")?, p::req_f32(v, "y1")?);
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    match stroke(&l, page, x0, y0, x1, y1) {
        Some(Stroke::Rect(r)) => draw_table(s, v, &l, page, (x0, y0), r),
        Some(Stroke::Vertical { x, y0, y1 }) => split_columns(s, &l, page, x, (y0, y1)),
        Some(Stroke::Horizontal { y, x0, x1 }) => split_rows(s, &l, page, y, (x0, x1)),
        None => Err(CmdError::Failed(
            "draw a rectangle (at least 12 pt each way) outside a table, or a straight horizontal or vertical line across table cells".into(),
        )),
    }
}

/// A one-cell table the size of `r`, at the paragraph under the stroke's start.
fn draw_table(s: &mut Session, v: &Value, l: &DocLayout, page: usize, start: (f32, f32), r: Rect) -> CmdResult {
    let story = super::story_param(s, v);
    let pos = l.hit(page, start.0, start.1, story).ok_or_else(|| CmdError::Failed("there's no text to put a table at there".into()))?;
    let pos = s.doc.clamp(&pos);
    if pos.path.cell().is_some() {
        return Err(CmdError::Failed("draw new tables outside a table; inside one, draw a line to split cells".into()));
    }
    let len = s.doc.para(pos.story, &pos.path).map(|q| q.len()).ok_or_else(|| CmdError::Failed("there's no paragraph there".into()))?;
    let tw = super::page::sect(s).text_width().max(18.0);
    let w = r.w.clamp(18.0, tw);
    let mut t = Table::new(1, 1, w);
    t.props.width = Some(w);
    let body_x = l.pages.get(page).map_or(0.0, |pg| pg.body.x);
    let indent = r.x - body_x;
    if indent > 1.0 {
        t.props.indent = Some(indent.min((tw - w).max(0.0)));
    }
    if let Some(row) = t.rows.first_mut() {
        row.props.height = Some(r.h.clamp(4.0, 1584.0));
        row.props.height_rule = HeightRule::AtLeast;
    }
    // Before the paragraph when the stroke starts at its beginning (or it's empty), else after it.
    let at = if pos.off == 0 || len == 0 { pos.path.clone() } else { s.doc.split_paragraph(&Pos { off: len, ..pos.clone() })?.path };
    s.doc.insert_block(pos.story, &at, Block::Table(t))?;
    let mut first = at.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story: pos.story, path: Path(first), off: 0 });
    sel_result(s)
}

/// The cells of one table a stroke crosses, as (row, cell) with their areas, and that table.
fn crossed(boxes: &[CellBox], at: (f32, f32), hit: impl Fn(&Rect) -> bool) -> Option<(StoryRef, Path, Vec<(usize, usize, Rect)>)> {
    let (story, table) = table_at(boxes, at.0, at.1)
        .or_else(|| boxes.iter().filter(|b| hit(&b.rect)).max_by_key(|b| b.table.0.len()).map(|b| (b.story, b.table.clone())))?;
    let mut cells: Vec<(usize, usize, Rect)> =
        boxes.iter().filter(|b| b.story == story && b.table == table && hit(&b.rect)).map(|b| (b.row, b.cell, b.rect)).collect();
    cells.sort_by_key(|c| (c.0, c.1));
    cells.dedup_by_key(|c| (c.0, c.1));
    Some((story, table, cells))
}

/// Grid column offsets: `cum[i]` is where grid column `i` starts.
fn offsets(t: &Table) -> Vec<f32> {
    let mut cum = Vec::with_capacity(t.grid.len() + 1);
    let mut x = 0.0f32;
    cum.push(0.0);
    for w in &t.grid {
        x += w.max(0.0);
        cum.push(x);
    }
    cum
}

/// The rows of the vertically merged group cell `c` of row `r` belongs to (just `r` when none).
fn vgroup(t: &Table, r: usize, c: usize) -> (usize, usize) {
    let g = t.grid_col(r, c);
    let starts_at = |row: usize| t.cell_at_grid(row, g).filter(|&ci| t.grid_col(row, ci) == g).and_then(|ci| t.rows.get(row)?.cells.get(ci));
    let Some(cell) = starts_at(r) else { return (r, r) };
    if cell.props.vmerge == VMerge::None {
        return (r, r);
    }
    let mut r0 = r;
    while r0 > 0 && starts_at(r0).is_some_and(|x| x.props.vmerge == VMerge::Continue) {
        r0 -= 1;
    }
    let mut r1 = r;
    while starts_at(r1 + 1).is_some_and(|x| x.props.vmerge == VMerge::Continue) {
        r1 += 1;
    }
    (r0, r1)
}

/// A cell as the grid rectangle it covers: rows `top..=bot`, grid columns `g0..g1`.
#[derive(Clone, Copy, Debug)]
struct Logical {
    top: usize,
    bot: usize,
    g0: usize,
    g1: usize,
}

fn logical(t: &Table, r: usize, c: usize) -> Logical {
    let g0 = t.grid_col(r, c);
    let span = t.rows.get(r).and_then(|row| row.cells.get(c)).map_or(1, Cell::span);
    let (top, bot) = vgroup(t, r, c);
    Logical { top, bot, g0, g1: g0 + span }
}

/// The rows (`top`, `bot`) two cells side by side share when erasing the border between them
/// leaves one rectangle: every row of both has exactly their cells there, merged the same way.
fn aligned_rows(t: &Table, a: &Logical, b: &Logical) -> Option<(usize, usize)> {
    if a.top != b.top || a.bot != b.bot || a.g1 != b.g0 {
        return None;
    }
    for row in a.top..=a.bot {
        let ci = t.cell_at_grid(row, a.g0).filter(|&ci| t.grid_col(row, ci) == a.g0)?;
        let cells = &t.rows.get(row)?.cells;
        let (x, y) = (cells.get(ci)?, cells.get(ci + 1)?);
        if x.span() != a.g1 - a.g0 || y.span() != b.g1 - b.g0 || x.props.vmerge != y.props.vmerge {
            return None;
        }
    }
    Some((a.top, a.bot))
}

/// Hide the border line on both sides along an edge: set no border on every cell beside it.
/// `vertical`: the line sits left of grid column `line` and the edge covers rows `lo..=hi`;
/// otherwise it sits above row `line` and covers grid columns `lo..hi`.
/// Whether anything was not hidden yet.
fn hide_edge(t: &mut Table, vertical: bool, line: usize, (lo, hi): (usize, usize)) -> bool {
    let nil = wordcraft_doc::props::Border { style: wordcraft_doc::props::BorderStyle::None, width: 0.0, color: None, space: 0.0 };
    // Only a cell whose edge lies on the line is touched.
    let set_v = |t: &mut Table, r: usize, g: usize, before: bool| {
        let Some(ci) = t.cell_at_grid(r, g) else { return false };
        let start = t.grid_col(r, ci);
        let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(ci)) else { return false };
        let end = start + cell.span();
        let b = cell.props.borders.get_or_insert_with(Default::default);
        let side = if before && end == line {
            &mut b.right
        } else if !before && start == line {
            &mut b.left
        } else {
            return false;
        };
        side.replace(nil) != Some(nil)
    };
    let set_h = |t: &mut Table, r: usize, g: usize, above: bool| {
        let Some(ci) = t.cell_at_grid(r, g) else { return false };
        let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(ci)) else { return false };
        let b = cell.props.borders.get_or_insert_with(Default::default);
        let side = if above { &mut b.bottom } else { &mut b.top };
        side.replace(nil) != Some(nil)
    };
    let mut changed = false;
    if vertical {
        for r in lo..=hi.min(t.rows.len().saturating_sub(1)) {
            if let Some(l) = line.checked_sub(1) {
                changed |= set_v(t, r, l, true);
            }
            changed |= set_v(t, r, line, false);
        }
    } else {
        let below_cont = |t: &Table, g: usize| {
            t.cell_at_grid(line, g).and_then(|ci| t.rows.get(line)?.cells.get(ci)).is_some_and(|c| c.props.vmerge == VMerge::Continue)
        };
        for g in lo..hi.min(MAX_COLS) {
            if below_cont(t, g) {
                continue;
            }
            if let Some(above) = line.checked_sub(1) {
                changed |= set_h(t, above, g, true);
            }
            changed |= set_h(t, line, g, false);
        }
    }
    changed
}

/// A vertical line at page `x`: split each crossed cell into two columns there.
fn split_columns(s: &mut Session, l: &DocLayout, page: usize, x: f32, (ya, yb): (f32, f32)) -> CmdResult {
    let boxes = cell_boxes(l, page);
    let hit = |r: &Rect| {
        let overlap = yb.min(r.bottom()) - ya.max(r.y);
        x > r.x + SNAP && x < r.right() - SNAP && overlap >= r.h * 0.5
    };
    let Some((story, tp, cells)) = crossed(&boxes, (x, (ya + yb) / 2.0), hit) else {
        return Err(CmdError::Failed("the line doesn't cross a table cell".into()));
    };
    let Some(&(r0, c0, rect0)) = cells.first() else { return Err(CmdError::Failed("the line doesn't cross a table cell".into())) };
    let t = s.doc.table(story, &tp).ok_or_else(|| CmdError::Failed("the table isn't there any more".into()))?;
    // Where the line falls in the table's grid, measured in the first crossed cell.
    let cum = offsets(t);
    let a = t.grid_col(r0, c0);
    let span = t.rows.get(r0).and_then(|row| row.cells.get(c0)).map_or(1, Cell::span);
    let (start, end) = (cum.get(a).copied().unwrap_or(0.0), cum.get(a + span).or(cum.last()).copied().unwrap_or(0.0));
    let g = start + (x - rect0.x) / rect0.w.max(1.0) * (end - start);
    if !g.is_finite() || g <= 0.0 {
        return Err(CmdError::Failed("the line doesn't cross a table cell".into()));
    }
    let crossed_set: std::collections::BTreeSet<(usize, usize)> = cells.iter().map(|c| (c.0, c.1)).collect();
    // A vertically merged cell splits only as a whole.
    for &(r, c, _) in &cells {
        let (g0, g1) = vgroup(t, r, c);
        let gc = t.grid_col(r, c);
        for rr in g0..=g1 {
            let ci = t.cell_at_grid(rr, gc).unwrap_or(usize::MAX);
            if !crossed_set.contains(&(rr, ci)) {
                return Err(CmdError::Failed("the line crosses only part of a merged cell; draw it across the whole cell".into()));
            }
        }
    }
    // Snap to an existing grid line, else a new one inside grid column `k`.
    let existing = (1..t.grid.len()).find(|&j| cum.get(j).is_some_and(|c| (c - g).abs() < SNAP));
    let k = (0..t.grid.len()).find(|&k| cum.get(k + 1).is_some_and(|&e| g < e)).unwrap_or(t.grid.len().saturating_sub(1));
    if existing.is_none() && t.grid.len() >= MAX_COLS {
        return Err(CmdError::Failed(format!("a table can't have more than {MAX_COLS} columns")));
    }
    let t = s.doc.table_mut(story, &tp)?;
    let mut changed = false;
    match existing {
        Some(j) => {
            for &(r, c) in crossed_set.iter().rev() {
                let a = t.grid_col(r, c);
                let Some(row) = t.rows.get_mut(r) else { continue };
                let Some(cell) = row.cells.get_mut(c) else { continue };
                let span = cell.span();
                if a < j && j < a + span {
                    cell.props.span = (j - a) as u32;
                    let mut nc = Cell::empty();
                    nc.props = cell.props.clone();
                    nc.props.span = (a + span - j) as u32;
                    row.cells.insert(c + 1, nc);
                    changed = true;
                }
            }
        }
        None => {
            let ks = cum.get(k).copied().unwrap_or(0.0);
            let ke = cum.get(k + 1).copied().unwrap_or(ks);
            let (left, right) = (g - ks, ke - g);
            if left < SNAP || right < SNAP {
                return Err(CmdError::Failed("there's already a border there".into()));
            }
            if let Some(w) = t.grid.get_mut(k) {
                *w = left;
            }
            t.grid.insert((k + 1).min(t.grid.len()), right);
            for r in 0..t.rows.len() {
                let Some(ci) = t.cell_at_grid(r, k) else { continue };
                let a = t.grid_col(r, ci);
                let Some(row) = t.rows.get_mut(r) else { continue };
                let Some(cell) = row.cells.get_mut(ci) else { continue };
                if crossed_set.contains(&(r, ci)) {
                    let span = cell.span();
                    cell.props.span = (k + 1 - a) as u32;
                    let mut nc = Cell::empty();
                    nc.props = cell.props.clone();
                    nc.props.span = (a + span - k) as u32;
                    row.cells.insert(ci + 1, nc);
                } else {
                    cell.props.span = cell.props.span.saturating_add(1);
                }
            }
            changed = true;
        }
    }
    if !changed {
        return Err(CmdError::Failed("there's already a border there".into()));
    }
    super::table::sync_cell_widths(t);
    finish(s, story, &tp, r0, c0 + 1)
}

/// A horizontal line at page `y`: split the row it crosses there. Crossed cells become two;
/// the row's other cells span both rows (a vertical merge).
fn split_rows(s: &mut Session, l: &DocLayout, page: usize, y: f32, (xa, xb): (f32, f32)) -> CmdResult {
    let boxes = cell_boxes(l, page);
    let hit = |r: &Rect| {
        let overlap = xb.min(r.right()) - xa.max(r.x);
        y > r.y + SNAP && y < r.bottom() - SNAP && overlap >= r.w * 0.5
    };
    let Some((story, tp, cells)) = crossed(&boxes, ((xa + xb) / 2.0, y), hit) else {
        return Err(CmdError::Failed("the line doesn't cross a table cell".into()));
    };
    let Some(&(r, c0, _)) = cells.first() else { return Err(CmdError::Failed("the line doesn't cross a table cell".into())) };
    if cells.iter().any(|c| c.0 != r) {
        return Err(CmdError::Failed("draw the line within one row".into()));
    }
    // The row's extent on this page.
    let row_boxes: Vec<&Rect> = boxes.iter().filter(|b| b.story == story && b.table == tp && b.row == r).map(|b| &b.rect).collect();
    let top = row_boxes.iter().map(|b| b.y).fold(f32::INFINITY, f32::min);
    let bottom = row_boxes.iter().map(|b| b.bottom()).fold(f32::NEG_INFINITY, f32::max);
    let t = s.doc.table(story, &tp).ok_or_else(|| CmdError::Failed("the table isn't there any more".into()))?;
    if t.rows.len() >= MAX_ROWS {
        return Err(CmdError::Failed("the table has too many rows".into()));
    }
    // An exact height leaves out the row's bottom margin (its cells' largest), which the layout
    // adds below it: take it off each half so the two rows draw where the line went.
    let row_bottom =
        t.rows.get(r).map_or(0.0, |row| (0..row.cells.len()).map(|ci| wordcraft_layout::cell_margins(&s.doc, t, r, ci)[2]).fold(0.0f32, f32::max));
    let crossed_set: std::collections::BTreeSet<usize> = cells.iter().map(|c| c.1).collect();
    let merged = |ci: &usize| t.rows.get(r).and_then(|row| row.cells.get(*ci)).is_some_and(|c| c.props.vmerge != VMerge::None);
    if crossed_set.iter().any(merged) {
        return Err(CmdError::Failed("splitting a vertically merged cell across rows isn't supported; erase its merge first".into()));
    }
    let t = s.doc.table_mut(story, &tp)?;
    let Some(row) = t.rows.get_mut(r) else { return Err(CmdError::Failed("the row isn't there any more".into())) };
    let mut new_row = wordcraft_doc::Row { props: row.props.clone(), cells: Vec::with_capacity(row.cells.len()), controls: Default::default() };
    new_row.props.header = false;
    if top.is_finite() && bottom.is_finite() && bottom > top {
        let rule = if row.props.height_rule == HeightRule::Exact { HeightRule::Exact } else { HeightRule::AtLeast };
        let less = if rule == HeightRule::Exact { row_bottom } else { 0.0 };
        row.props.height = Some((y - top - less).clamp(1.0, 1584.0));
        row.props.height_rule = rule;
        new_row.props.height = Some((bottom - y - less).clamp(1.0, 1584.0));
        new_row.props.height_rule = rule;
    }
    for (ci, cell) in row.cells.iter_mut().enumerate() {
        let mut nc = Cell::empty();
        nc.props = cell.props.clone();
        if crossed_set.contains(&ci) {
            nc.props.vmerge = VMerge::None;
        } else {
            if cell.props.vmerge == VMerge::None {
                cell.props.vmerge = VMerge::Restart;
            }
            nc.props.vmerge = VMerge::Continue;
        }
        new_row.cells.push(nc);
    }
    t.rows.insert(r + 1, new_row);
    finish(s, story, &tp, r + 1, c0)
}

/// Put the caret in cell (`r`, `c`) of the table (clamped) and re-lay its paragraphs out.
fn finish(s: &mut Session, story: StoryRef, tp: &Path, r: usize, c: usize) -> CmdResult {
    let Some(t) = s.doc.table(story, tp) else { return sel_result(s) };
    let r = r.min(t.rows.len().saturating_sub(1));
    let c = c.min(t.rows.get(r).map_or(0, |x| x.cells.len().saturating_sub(1)));
    let mut path = tp.0.clone();
    path.extend([r as u32, c as u32, 0]);
    s.sel = Selection::caret(s.doc.clamp(&Pos { story, path: Path(path), off: 0 }));
    super::table::touch_cells(s, tp)?;
    sel_result(s)
}

/// Where a stretch of border lies in its table's grid. Horizontal lines sort first, top to
/// bottom, so a drag merges down its columns before across.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Edge {
    /// The line above row `line` (`rows.len()`: below the last row) along grid columns `g0..g1`.
    Horizontal { line: usize, g0: usize, g1: usize },
    /// Row `row`'s stretch of the line before grid column `line` (in the table's own order: a
    /// right-to-left table draws it mirrored).
    Vertical { row: usize, line: usize },
}

/// A stretch of border the eraser can remove: one row's part of a vertical line, or the part of
/// a horizontal line two cells (or a cell and the table's outside) share. Its ends are page
/// points, where the layout draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct EraseTarget {
    pub from: (f32, f32),
    pub to: (f32, f32),
    story: StoryRef,
    table: Path,
    edge: Edge,
    /// On the table's outside: erasing hides it (there's no cell beyond to merge with).
    outer: bool,
}

impl EraseTarget {
    /// How far page point (`x`, `y`) is from the stretch.
    fn distance(&self, x: f32, y: f32) -> f32 {
        let (x0, x1) = (self.from.0.min(self.to.0), self.from.0.max(self.to.0));
        let (y0, y1) = (self.from.1.min(self.to.1), self.from.1.max(self.to.1));
        (x0 - x).max(x - x1).max(0.0).hypot((y0 - y).max(y - y1).max(0.0))
    }

    /// Whether rectangle `r` touches the stretch: across the line within `reach`; along it past
    /// `reach` from its ends, so a drag along one line leaves the lines that meet it.
    fn touched_by(&self, r: &Rect, reach: f32) -> bool {
        let (at, s0, s1, across, along) = match self.edge {
            Edge::Vertical { .. } => (self.from.0, self.from.1.min(self.to.1), self.from.1.max(self.to.1), (r.x, r.right()), (r.y, r.bottom())),
            Edge::Horizontal { .. } => (self.from.1, self.from.0.min(self.to.0), self.from.0.max(self.to.0), (r.y, r.bottom()), (r.x, r.right())),
        };
        let m = reach.min((s1 - s0) / 4.0);
        at >= across.0 - reach && at <= across.1 + reach && along.1 >= s0 + m && along.0 <= s1 - m
    }
}

/// Every stretch of border the eraser can remove on a page, table by table.
fn targets(doc: &Document, l: &DocLayout, page: usize) -> Vec<EraseTarget> {
    let boxes = cell_boxes(l, page);
    let mut out = Vec::new();
    let mut seen: Vec<(StoryRef, &Path)> = Vec::new();
    for b in &boxes {
        if seen.iter().any(|(s, p)| *s == b.story && **p == b.table) {
            continue;
        }
        seen.push((b.story, &b.table));
        if let Some(t) = doc.table(b.story, &b.table) {
            table_targets(t, b.story, &b.table, &boxes, &mut out);
        }
    }
    out
}

/// One table's stretches of border, from its cells' boxes on the page.
fn table_targets(t: &Table, story: StoryRef, table: &Path, boxes: &[CellBox], out: &mut Vec<EraseTarget>) {
    let mut rows: std::collections::BTreeMap<usize, Vec<&CellBox>> = Default::default();
    for b in boxes.iter().filter(|b| b.story == story && b.table == *table) {
        rows.entry(b.row).or_default().push(b);
    }
    let cell = |r: usize, c: usize| t.rows.get(r).and_then(|row| row.cells.get(c));
    let cols = |b: &CellBox| {
        let g0 = t.grid_col(b.row, b.cell);
        (g0, g0 + cell(b.row, b.cell).map_or(1, Cell::span))
    };
    let top_of = |cells: &[&CellBox]| cells.iter().map(|b| b.rect.y).fold(f32::INFINITY, f32::min);
    let mut push = |from: (f32, f32), to: (f32, f32), edge: Edge, outer: bool| {
        out.push(EraseTarget { from, to, story, table: table.clone(), edge, outer });
    };
    for (&r, cells) in &rows {
        // The row's top and bottom: the first cell of a vertical merge is as tall as all its
        // rows, so the row's other cells (or the next row) say where it ends.
        let top = top_of(cells);
        let own = cells.iter().filter(|b| cell(r, b.cell).is_none_or(|c| c.props.vmerge != VMerge::Restart)).map(|b| b.rect.bottom());
        let bottom = own.fold(f32::INFINITY, f32::min);
        let bottom = if bottom.is_finite() {
            bottom
        } else {
            let next = rows.get(&(r + 1)).map_or(f32::INFINITY, |c| top_of(c));
            next.min(cells.iter().map(|b| b.rect.bottom()).fold(f32::INFINITY, f32::min))
        };
        if !top.is_finite() || !bottom.is_finite() {
            continue;
        }
        // The vertical lines: each cell's two sides, once per grid line.
        let mut lines: Vec<(usize, f32)> = Vec::new();
        for b in cells {
            let (g0, g1) = cols(b);
            let (left, right) = if t.props.bidi_visual { (g1, g0) } else { (g0, g1) };
            for (line, x) in [(left, b.rect.x), (right, b.rect.right())] {
                if !lines.iter().any(|(l, _)| *l == line) {
                    lines.push((line, x));
                }
            }
        }
        for (line, x) in lines {
            let ends = |c: usize| t.grid_col(r, c) + cell(r, c).map_or(1, Cell::span) == line;
            let before = line.checked_sub(1).and_then(|g| t.cell_at_grid(r, g)).is_some_and(ends);
            let after = t.cell_at_grid(r, line).is_some_and(|c| t.grid_col(r, c) == line);
            push((x, top), (x, bottom), Edge::Vertical { row: r, line }, !(before && after));
        }
        // The line above each cell (none above a merge's continuation): shared with the cells
        // of the row before when that row is on this page, else the table's top above its
        // first row; and the table's bottom below its last row.
        let above = r.checked_sub(1).and_then(|p| rows.get(&p));
        for b in cells {
            let (g0, g1) = cols(b);
            let continues = cell(r, b.cell).is_some_and(|c| c.props.vmerge == VMerge::Continue);
            match above {
                _ if continues => {}
                Some(prev) => {
                    for a in prev {
                        let (h0, h1) = cols(a);
                        let (x0, x1) = (b.rect.x.max(a.rect.x), b.rect.right().min(a.rect.right()));
                        if g0.max(h0) < g1.min(h1) && x1 > x0 {
                            push((x0, top), (x1, top), Edge::Horizontal { line: r, g0: g0.max(h0), g1: g1.min(h1) }, false);
                        }
                    }
                }
                None if r == 0 => push((b.rect.x, top), (b.rect.right(), top), Edge::Horizontal { line: 0, g0, g1 }, true),
                None => {}
            }
            if r + 1 == t.rows.len() {
                push((b.rect.x, bottom), (b.rect.right(), bottom), Edge::Horizontal { line: r + 1, g0, g1 }, true);
            }
        }
    }
}

/// The stretch of border nearest page point (`x`, `y`) within `reach` points (a click): a line
/// between two cells before the table's outside, then the nearest, then the innermost table's.
pub fn erase_target(doc: &Document, l: &DocLayout, page: usize, x: f32, y: f32, reach: f32) -> Option<EraseTarget> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    targets(doc, l, page)
        .into_iter()
        .map(|e| (e.distance(x, y), e))
        .filter(|(d, _)| *d <= reach)
        .min_by_key(|(d, e)| (e.outer, (d * 100.0).round() as i64, std::cmp::Reverse(e.table.0.len())))
        .map(|(_, e)| e)
}

/// The stretches of border a drag from `from` to `to` (page points) touches, in one table: the
/// innermost under the drag's start, else the outermost it touches.
pub fn erase_targets(doc: &Document, l: &DocLayout, page: usize, from: (f32, f32), to: (f32, f32), reach: f32) -> Vec<EraseTarget> {
    if ![from.0, from.1, to.0, to.1].iter().all(|v| v.is_finite()) {
        return Vec::new();
    }
    let r = Rect::new(from.0.min(to.0), from.1.min(to.1), (to.0 - from.0).abs(), (to.1 - from.1).abs());
    let mut hit: Vec<EraseTarget> = targets(doc, l, page).into_iter().filter(|e| e.touched_by(&r, reach)).collect();
    let start = table_at(&cell_boxes(l, page), from.0, from.1).filter(|(s, p)| hit.iter().any(|e| e.story == *s && e.table == *p));
    if let Some((story, table)) = start.or_else(|| hit.iter().min_by_key(|e| e.table.0.len()).map(|e| (e.story, e.table.clone()))) {
        hit.retain(|e| e.story == story && e.table == table);
    }
    hit
}

/// Erase one stretch of border from `t`: the cells on its two sides merge when they make one
/// rectangle, else it is hidden on both sides. The (row, grid column) of the cell the caret
/// goes to; `None` when nothing is there any more (an earlier stretch merged it away) or the
/// stretch was hidden already.
fn erase(t: &mut Table, edge: Edge) -> Option<(usize, usize)> {
    match edge {
        Edge::Vertical { row, line } => {
            let before = line.checked_sub(1).and_then(|g| t.cell_at_grid(row, g)).map(|c| logical(t, row, c)).filter(|a| a.g1 == line);
            let after = t.cell_at_grid(row, line).map(|c| logical(t, row, c)).filter(|b| b.g0 == line);
            match (before, after) {
                (Some(a), Some(b)) => {
                    // Both sides merged down the same rows (or plain single cells): one cell.
                    if let Some((top, bot)) = aligned_rows(t, &a, &b) {
                        for r in top..=bot {
                            if let Some(ci) = t.cell_at_grid(r, a.g0) {
                                t.merge_right(r, ci, ci + 1);
                            }
                        }
                        super::table::sync_cell_widths(t);
                        return Some((top, a.g0));
                    }
                    // Two sides that can't make one rectangle: hidden along the whole edge.
                    hide_edge(t, true, line, (a.top.min(b.top), a.bot.max(b.bot))).then_some((a.top, a.g0))
                }
                (Some(c), None) | (None, Some(c)) => hide_edge(t, true, line, (c.top, c.bot)).then_some((c.top, c.g0)),
                (None, None) => None,
            }
        }
        Edge::Horizontal { line, g0, g1 } => {
            let above = line.checked_sub(1).and_then(|r| t.cell_at_grid(r, g0).map(|c| (r, c)));
            let below = t.cell_at_grid(line, g0).map(|c| (line, c));
            let (Some((ra, ca)), Some((rb, cb))) = (above, below) else {
                // The table's top or bottom: hidden above or below this cell.
                let (r, c) = above.or(below)?;
                let lc = logical(t, r, c);
                return hide_edge(t, false, line, (lc.g0, lc.g1)).then_some((lc.top, lc.g0));
            };
            let (la, lb) = (logical(t, ra, ca), logical(t, rb, cb));
            if la.top == lb.top {
                return None;
            }
            // Cells that don't line up: only the stretch they share is hidden, as in Word.
            if la.g0 != lb.g0 || la.g1 != lb.g1 {
                return hide_edge(t, false, line, (g0.max(la.g0).max(lb.g0), g1.min(la.g1).min(lb.g1))).then_some((la.top, la.g0));
            }
            if let Some(a) = t.rows.get_mut(ra).and_then(|row| row.cells.get_mut(ca))
                && a.props.vmerge == VMerge::None
            {
                a.props.vmerge = VMerge::Restart;
            }
            let mut moved = Vec::new();
            if let Some(b) = t.rows.get_mut(rb).and_then(|row| row.cells.get_mut(cb)) {
                b.props.vmerge = VMerge::Continue;
                let blocks = std::mem::replace(&mut b.blocks, vec![para_block(wordcraft_doc::Paragraph::new())]);
                moved.extend(blocks.into_iter().filter(|x| x.as_para().is_none_or(|q| !q.is_empty())));
            }
            let tc = t.cell_at_grid(la.top, la.g0).unwrap_or(0);
            if !moved.is_empty()
                && let Some(dst) = t.rows.get_mut(la.top).and_then(|row| row.cells.get_mut(tc))
            {
                if dst.blocks.len() == 1 && dst.blocks.first().and_then(|b| b.as_para()).is_some_and(wordcraft_doc::Paragraph::is_empty) {
                    dst.blocks.clear();
                }
                dst.blocks.extend(moved);
            }
            Some((la.top, la.g0))
        }
    }
}

/// The reach (points) the eraser works with for a requested `reach`: the command's and the
/// app's preview's, so what lights up while pressed is what a release erases.
pub fn eraser_reach(reach: Option<f32>) -> f32 {
    reach.filter(|r| !r.is_nan()).map_or(ERASER_REACH, |r| r.clamp(0.5, ERASER_MAX_REACH))
}

fn eraser(s: &mut Session, v: &Value) -> CmdResult {
    let page = page_param(v)?;
    let reach = eraser_reach(p::f32(v, "reach"));
    let drag = v.get("x0").is_some();
    let (from, to) = if drag {
        ((p::req_f32(v, "x0")?, p::req_f32(v, "y0")?), (p::req_f32(v, "x1")?, p::req_f32(v, "y1")?))
    } else {
        let at = (p::req_f32(v, "x")?, p::req_f32(v, "y")?);
        (at, at)
    };
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    let found = if drag {
        erase_targets(&s.doc, &l, page, from, to, reach)
    } else {
        erase_target(&s.doc, &l, page, from.0, from.1, reach).into_iter().collect()
    };
    let Some((story, tp)) = found.first().map(|e| (e.story, e.table.clone())) else {
        return Err(CmdError::Failed("there's no table border there".into()));
    };
    let mut edges: Vec<Edge> = found.iter().map(|e| e.edge).collect();
    edges.sort();
    edges.dedup();
    let t = s.doc.table_mut(story, &tp)?;
    let before = t.clone();
    let mut caret = None;
    for edge in edges {
        if let Some(at) = erase(t, edge) {
            caret.get_or_insert(at);
        }
    }
    if *t == before {
        return Err(CmdError::Failed("that border is already erased".into()));
    }
    // The caret goes to the start of the first cell that changed, as in Word.
    let (r, g) = caret.unwrap_or((0, 0));
    let (r, c) = match t.cell_at_grid(r, g).map(|ci| logical(t, r, ci)) {
        Some(lc) => (lc.top, t.cell_at_grid(lc.top, lc.g0).unwrap_or(0)),
        None => (r, 0),
    };
    finish(s, story, &tp, r, c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Each row's cell spans.
    fn shape(t: &Table) -> Value {
        json!(t.rows.iter().map(|r| r.cells.iter().map(|c| c.span()).collect::<Vec<_>>()).collect::<Vec<_>>())
    }

    fn first_table(s: &Session) -> Table {
        s.doc.body.iter().find_map(|b| b.as_table()).cloned().unwrap()
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

    /// A rectangle drawn on the page inserts a one-cell table of its size; undo takes it out.
    #[test]
    fn a_drawn_rectangle_inserts_a_table() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("document.setText", &json!({"text": "Above the table"})).unwrap();
        let before = s.doc.clone();
        let l = s.layout();
        let body = l.pages[0].body;
        let (x0, y0) = (body.x + 40.0, body.y + 2.0);
        s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x0 + 200.0, "y1": y0 + 60.0})).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows.len(), t.rows[0].cells.len()), (1, 1));
        assert_eq!(t.props.width, Some(200.0));
        assert_eq!(t.rows[0].props.height, Some(60.0));
        assert!(t.props.indent.is_some_and(|i| (i - 40.0).abs() < 0.5));
        assert!(s.sel.focus.path.cell().is_some(), "the caret goes into the new cell");
        let r = cell_rect(&mut s, 0, 0);
        assert!((r.w - 200.0).abs() < 1.0 && r.h >= 59.5, "{r:?}");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.body, before.body, "one undo step");
        // Too small, or nowhere: errors, never panics.
        assert!(s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x0 + 5.0, "y1": y0 + 5.0})).is_err());
        assert!(s.run("table.draw", &json!({"page": 9, "x0": 1, "y0": 1, "x1": 100, "y1": 100})).is_err());
        assert!(s.run("table.draw", &json!({})).is_err());
    }

    /// A line drawn through an exact-height row splits it into two exact rows that draw where
    /// the line went: the bottom margin the layout adds below each isn't counted twice.
    #[test]
    fn a_line_through_an_exact_row_keeps_its_drawn_height() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
            Block::Table(t) => Some(t),
            _ => None,
        });
        let t = t.unwrap();
        t.props.cell_margins = Some([4.0, 5.4, 4.0, 5.4]);
        t.rows[0].props.height = Some(60.0);
        t.rows[0].props.height_rule = HeightRule::Exact;
        s.touch();
        let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
        assert!((a.h - 64.0).abs() < 0.5, "an exact 60pt row with a 4pt bottom margin: {a:?}");
        let y = a.y + 25.0;
        s.run("table.draw", &json!({"page": 0, "x0": b.x + 1.0, "y0": y, "x1": b.right() - 1.0, "y1": y})).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows[0].props.height_rule, t.rows[1].props.height_rule), (HeightRule::Exact, HeightRule::Exact));
        let (top, low) = (cell_rect(&mut s, 0, 1), cell_rect(&mut s, 1, 1));
        assert!((top.h - 25.0).abs() < 0.5 && (top.y - a.y).abs() < 0.01, "{top:?}");
        assert!((low.bottom() - a.bottom()).abs() < 0.5, "the pair keeps the row's height: {low:?} vs {a:?}");
    }

    /// Erasing a horizontal border merges the cells into one laid-out cell (#640): the lower
    /// row's box is `covered`, the merged cell's box spans both rows, and erasing the border
    /// below the merge extends it.
    #[test]
    fn erasing_a_horizontal_border_lays_the_merge_out_as_one_cell() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 3, "cols": 2})).unwrap();
        let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 1, 0));
        s.run("table.eraser", &json!({"page": 0, "x": a.x + a.w / 2.0, "y": a.bottom() + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows[0].cells[0].props.vmerge, t.rows[1].cells[0].props.vmerge), (VMerge::Restart, VMerge::Continue));
        let merged = cell_rect(&mut s, 0, 0);
        assert!((merged.bottom() - b.bottom()).abs() < 0.5 && merged.h > a.h * 1.9, "{merged:?}");
        let covered = s.layout().pages[0].items.iter().any(|it| matches!(it, Placed::Cell { row: 1, cell: 0, covered: true, .. }));
        assert!(covered, "the continuation row's box is covered");
        let hit = s.layout().cell_at(0, merged.x + 3.0, b.y + b.h / 2.0).map(|c| (c.2, c.3));
        assert_eq!(hit, Some((0, 0)), "a click in the lower half is in the merged cell");
        // The border below the merge: the merge grows to three rows.
        s.run("table.eraser", &json!({"page": 0, "x": merged.x + merged.w / 2.0, "y": merged.bottom() + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows[2].cells[0].props.vmerge, VMerge::Continue);
    }

    /// A vertical line through a cell splits it into two columns; a horizontal one splits a row;
    /// the eraser merges the cells back; each is one undo step.
    #[test]
    fn lines_split_cells_and_the_eraser_merges_them() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        s.run("text.insert", &json!({"text": "A"})).unwrap();
        let start = first_table(&s);
        let r = cell_rect(&mut s, 0, 0);
        let x = r.x + r.w * 0.25;
        s.run("table.draw", &json!({"page": 0, "x0": x, "y0": r.y + 1.0, "x1": x + 1.0, "y1": r.bottom() - 1.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.grid.len(), 3);
        assert_eq!(shape(&t), json!([[1, 1, 1], [2, 1]]), "the crossed cell splits, the other row's cell spans the new column");
        assert!((t.grid[0] - start.grid[0] * 0.25).abs() < 1.0, "{:?}", t.grid);
        let text = t.rows[0].cells[0].blocks.iter().filter_map(|b| b.as_para().map(|q| q.plain_text())).collect::<String>();
        assert_eq!(text, "A", "the text stays in the left part");
        // The eraser on the new border merges the two cells again.
        let r0 = cell_rect(&mut s, 0, 0);
        s.run("table.eraser", &json!({"page": 0, "x": r0.right() + 1.0, "y": r0.y + r0.h / 2.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[2, 1], [2, 1]]));
        // A horizontal line across the second column's first cell splits its row.
        let r1 = cell_rect(&mut s, 0, 1);
        let y = r1.y + r1.h / 2.0;
        s.run("table.draw", &json!({"page": 0, "x0": r1.x + 1.0, "y0": y, "x1": r1.right() - 1.0, "y1": y})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.rows[0].cells[0].props.vmerge, VMerge::Restart, "the uncrossed cell spans both rows");
        assert_eq!(t.rows[1].cells[0].props.vmerge, VMerge::Continue);
        assert_eq!(t.rows[1].cells[1].props.vmerge, VMerge::None);
        // Erasing that line merges the split cells (a vertical merge).
        let a = cell_rect(&mut s, 0, 1);
        s.run("table.eraser", &json!({"page": 0, "x": a.x + a.w / 2.0, "y": a.bottom() + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows[0].cells[1].props.vmerge, VMerge::Restart);
        assert_eq!(t.rows[1].cells[1].props.vmerge, VMerge::Continue);
        // Four steps, four undos back to the start.
        for _ in 0..4 {
            s.run("edit.undo", &json!({})).unwrap();
        }
        assert_eq!(first_table(&s), start);
        // Nothing near a border, or a hostile point: an error.
        let c = cell_rect(&mut s, 0, 0);
        assert!(s.run("table.eraser", &json!({"page": 0, "x": c.x + c.w / 2.0, "y": c.y + c.h / 2.0})).is_err());
        assert!(s.run("table.eraser", &json!({"page": 0, "x": 1e30, "y": -1e30})).is_err());
        assert_eq!(first_table(&s), start);
    }

    /// A 4×4 table whose cells say where they are ("A1"…"D4").
    fn named_4x4() -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 4, "cols": 4})).unwrap();
        let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
            Block::Table(t) => Some(t),
            _ => None,
        });
        for (r, row) in t.unwrap().rows.iter_mut().enumerate() {
            for (c, cell) in row.cells.iter_mut().enumerate() {
                let name = format!("{}{}", ["A", "B", "C", "D"][c], r + 1);
                cell.blocks = vec![para_block(wordcraft_doc::Paragraph::with_text(&name, Default::default()))];
            }
        }
        s.touch();
        s
    }

    fn texts(t: &Table, r: usize, c: usize) -> Vec<String> {
        t.rows[r].cells[c].blocks.iter().filter_map(|b| b.as_para().map(|q| q.plain_text())).collect()
    }

    /// Which of a cell's top, right, bottom and left borders are hidden.
    fn hidden(t: &Table, r: usize, c: usize) -> [bool; 4] {
        let b = t.rows[r].cells[c].props.borders.unwrap_or_default();
        [b.top, b.right, b.bottom, b.left].map(|e| e.is_some_and(|e| !e.is_visible()))
    }

    /// A drag erases every border its rectangle touches, as in Word, in one undo step: across a
    /// row's vertical lines the row becomes one cell (its text kept, paragraph by paragraph);
    /// along a horizontal line each column merges down on its own; corner to corner, one cell.
    #[test]
    fn a_drag_erases_every_border_it_touches() {
        let mut s = named_4x4();
        let start = first_table(&s);
        let depth = s.undo_depth();
        let (a, d) = (cell_rect(&mut s, 1, 0), cell_rect(&mut s, 1, 3));
        let y = a.y + a.h / 2.0;
        s.run("table.eraser", &json!({"page": 0, "x0": a.x + a.w / 2.0, "y0": y, "x1": d.x + d.w / 2.0, "y1": y + 1.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 1, 1, 1], [4], [1, 1, 1, 1], [1, 1, 1, 1]]));
        assert_eq!(texts(&t, 1, 0), ["A2", "B2", "C2", "D2"]);
        assert_eq!(s.undo_depth(), depth + 1, "one undo step");
        assert_eq!(s.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((1, 0)), "the caret goes to the merged cell");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(first_table(&s), start);
        // Along the line under row 3, from inside A3 to the middle of C3: A, B and C merge down,
        // each on its own; D stays.
        let (a, c) = (cell_rect(&mut s, 2, 0), cell_rect(&mut s, 2, 2));
        let y = a.bottom();
        s.run("table.eraser", &json!({"page": 0, "x0": a.x + a.w / 2.0, "y0": y, "x1": c.x + c.w / 2.0, "y1": y + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), shape(&start), "nothing merges across");
        let vm = |r: usize| t.rows[r].cells.iter().map(|c| c.props.vmerge).collect::<Vec<_>>();
        assert_eq!(vm(2), [VMerge::Restart, VMerge::Restart, VMerge::Restart, VMerge::None]);
        assert_eq!(vm(3), [VMerge::Continue, VMerge::Continue, VMerge::Continue, VMerge::None]);
        s.run("edit.undo", &json!({})).unwrap();
        // Corner to corner, from B1 to C3: one cell two columns wide, three rows deep.
        let (b, c) = (cell_rect(&mut s, 0, 1), cell_rect(&mut s, 2, 2));
        let (from, to) = ((b.x + b.w / 2.0, b.y + b.h / 2.0), (c.x + c.w / 2.0, c.y + c.h / 2.0));
        s.run("table.eraser", &json!({"page": 0, "x0": from.0, "y0": from.1, "x1": to.0, "y1": to.1})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 2, 1], [1, 2, 1], [1, 2, 1], [1, 1, 1, 1]]));
        let vm: Vec<_> = (0..3).map(|r| t.rows[r].cells[1].props.vmerge).collect();
        assert_eq!(vm, [VMerge::Restart, VMerge::Continue, VMerge::Continue]);
        let mut text = texts(&t, 0, 1);
        text.sort();
        assert_eq!(text, ["B1", "B2", "B3", "C1", "C2", "C3"]);
    }

    /// The table's outside borders hide, never merge, as in Word: a click above D1 hides only
    /// D1's top; a drag down the left side hides it in every row; a drag from above the table
    /// into D2 hides the top and merges D1 with D2. A border already hidden is refused, without
    /// an undo step; a one-cell table's sides hide too.
    #[test]
    fn the_eraser_hides_the_tables_outside_borders() {
        let mut s = named_4x4();
        let start = first_table(&s);
        let d1 = cell_rect(&mut s, 0, 3);
        let above = json!({"page": 0, "x": d1.x + d1.w / 2.0, "y": d1.y - 2.0});
        s.run("table.eraser", &above).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), shape(&start));
        assert_eq!(hidden(&t, 0, 3), [true, false, false, false], "only D1's top");
        assert_eq!(hidden(&t, 0, 2), [false; 4]);
        let depth = s.undo_depth();
        assert!(s.run("table.eraser", &above).is_err(), "already hidden");
        assert_eq!(s.undo_depth(), depth);
        s.run("edit.undo", &json!({})).unwrap();
        let (a1, a4) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 3, 0));
        s.run("table.eraser", &json!({"page": 0, "x0": a1.x - 1.0, "y0": a1.y + 2.0, "x1": a1.x + 1.0, "y1": a4.bottom() - 2.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), shape(&start));
        for r in 0..4 {
            assert_eq!(t.rows[r].cells[0].props.vmerge, VMerge::None);
            assert_eq!(hidden(&t, r, 0), [false, false, false, true], "row {r}: only the left side");
        }
        s.run("edit.undo", &json!({})).unwrap();
        // With A1's left hidden already, the drag's caret goes to A2, the first cell that changes.
        s.run("table.eraser", &json!({"page": 0, "x": a1.x - 1.0, "y": a1.y + a1.h / 2.0})).unwrap();
        s.run("table.eraser", &json!({"page": 0, "x0": a1.x - 1.0, "y0": a1.y + 2.0, "x1": a1.x + 1.0, "y1": a4.bottom() - 2.0})).unwrap();
        assert_eq!(s.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((1, 0)), "the caret goes to A2");
        s.run("edit.undo", &json!({})).unwrap();
        s.run("edit.undo", &json!({})).unwrap();
        let d2 = cell_rect(&mut s, 1, 3);
        let x = d1.x + d1.w / 2.0;
        s.run("table.eraser", &json!({"page": 0, "x0": x, "y0": d1.y - 10.0, "x1": x + 1.0, "y1": d2.y + d2.h / 2.0})).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows[0].cells[3].props.vmerge, t.rows[1].cells[3].props.vmerge), (VMerge::Restart, VMerge::Continue));
        assert!(hidden(&t, 0, 3)[0] && !hidden(&t, 0, 2)[0], "the top hides over the merged cell only");
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap();
        let a = cell_rect(&mut s, 0, 0);
        s.run("table.eraser", &json!({"page": 0, "x": a.x + a.w / 2.0, "y": a.y + 1.0})).unwrap();
        s.run("table.eraser", &json!({"page": 0, "x": a.right() + 1.0, "y": a.y + a.h / 2.0})).unwrap();
        assert_eq!(hidden(&first_table(&s), 0, 0), [true, true, false, false]);
    }

    /// A reach goes as far as `ERASER_MAX_REACH` and no further, whatever the caller asks.
    #[test]
    fn the_reach_stops_at_its_limit() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap();
        let a = cell_rect(&mut s, 0, 0);
        let at = |dx: f32| json!({"page": 0, "x": a.right() + dx, "y": a.y + a.h / 2.0, "reach": 1000.0});
        assert!(s.run("table.eraser", &at(ERASER_MAX_REACH + 1.0)).is_err(), "past the limit");
        s.run("table.eraser", &at(ERASER_MAX_REACH - 1.0)).unwrap();
        assert_eq!(hidden(&first_table(&s), 0, 0), [false, true, false, false]);
        assert_eq!(eraser_reach(None), ERASER_REACH);
        assert_eq!(eraser_reach(Some(f32::NAN)), ERASER_REACH);
        assert_eq!(eraser_reach(Some(f32::INFINITY)), ERASER_MAX_REACH);
        assert_eq!(eraser_reach(Some(0.0)), 0.5);
    }

    /// A click on the table's outside takes a line between two cells while one is in reach,
    /// though the outside is nearer; past the reach it hides the outside.
    #[test]
    fn a_line_between_cells_in_reach_wins_over_the_nearer_outside() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        let b = cell_rect(&mut s, 0, 1);
        // On the right side, half a row above the line between the rows.
        let at = |reach: f32| json!({"page": 0, "x": b.right(), "y": b.y + b.h / 2.0, "reach": reach});
        s.run("table.eraser", &at(b.h / 2.0 + 1.0)).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows[0].cells[1].props.vmerge, t.rows[1].cells[1].props.vmerge), (VMerge::Restart, VMerge::Continue), "B1 and B2 merge");
        assert_eq!(hidden(&t, 0, 1), [false; 4], "the outside stays");
        s.run("edit.undo", &json!({})).unwrap();
        s.run("table.eraser", &at(b.h / 2.0 - 1.0)).unwrap();
        let t = first_table(&s);
        assert_eq!(hidden(&t, 0, 1), [false, true, false, false], "the outside hides");
        assert_eq!(t.rows[0].cells[1].props.vmerge, VMerge::None, "nothing merges");
    }

    /// A click near a corner takes the border between two cells before the table's outside
    /// (Word merges A1 and A2 at the left end of their line); the reach is the caller's; a click
    /// on the line just above a table nested in B2 is the outer table's, and the nested table
    /// moves into the merged cell.
    #[test]
    fn a_click_prefers_a_border_between_cells_and_reaches_as_far_as_asked() {
        let mut s = named_4x4();
        let a1 = cell_rect(&mut s, 0, 0);
        s.run("table.eraser", &json!({"page": 0, "x": a1.x + 1.0, "y": a1.bottom() + 1.0})).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows[0].cells[0].props.vmerge, t.rows[1].cells[0].props.vmerge), (VMerge::Restart, VMerge::Continue));
        assert_eq!(hidden(&t, 0, 0), [false; 4], "the left side stays");
        s.run("edit.undo", &json!({})).unwrap();
        let far = json!({"page": 0, "x": a1.x + a1.w / 2.0, "y": a1.bottom() + 6.0});
        assert!(s.run("table.eraser", &far).is_err(), "6 pt is past the default reach");
        let mut wide = far.clone();
        wide["reach"] = json!(8.0);
        s.run("table.eraser", &wide).unwrap();
        assert_eq!(first_table(&s).rows[1].cells[0].props.vmerge, VMerge::Continue);
        let mut s = named_4x4();
        // B2's top and left (the nested table's own cells would come first in the layout).
        let b2 = cell_rect(&mut s, 1, 1);
        let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
            Block::Table(t) => Some(t),
            _ => None,
        });
        t.unwrap().rows[1].cells[1].blocks.insert(0, std::sync::Arc::new(Block::Table(Table::new(2, 2, 80.0))));
        s.touch();
        s.run("table.eraser", &json!({"page": 0, "x": b2.x + 20.0, "y": b2.y + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!((t.rows[0].cells[1].props.vmerge, t.rows[1].cells[1].props.vmerge), (VMerge::Restart, VMerge::Continue));
        assert!(t.rows[0].cells[1].blocks.iter().any(|b| b.as_table().is_some()), "the nested table moves with B2's text");
    }

    /// A 3x3 table with columns 1 and 2 merged down rows 0-1 (made with the eraser itself).
    fn merged_columns_table() -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 3, "cols": 3})).unwrap();
        for cell in [2, 1] {
            let a = cell_rect(&mut s, 0, cell);
            s.run("table.eraser", &json!({"page": 0, "x": a.x + a.w / 2.0, "y": a.bottom() + 0.5})).unwrap();
        }
        s
    }

    /// The border between two cells merged down the same rows merges them into one cell that is
    /// still merged down those rows; one undo step.
    #[test]
    fn erasing_between_cells_merged_down_the_same_rows_merges_them() {
        let mut s = merged_columns_table();
        let start = first_table(&s);
        assert_eq!(shape(&start), json!([[1, 1, 1], [1, 1, 1], [1, 1, 1]]));
        let a = cell_rect(&mut s, 0, 1);
        s.run("table.eraser", &json!({"page": 0, "x": a.right() + 0.5, "y": a.y + 3.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 2], [1, 2], [1, 1, 1]]));
        assert_eq!((t.rows[0].cells[1].props.vmerge, t.rows[1].cells[1].props.vmerge), (VMerge::Restart, VMerge::Continue));
        let merged = cell_rect(&mut s, 0, 1);
        assert!(merged.w > a.w * 1.9 && (merged.h - a.h).abs() < 0.5, "{merged:?}");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(first_table(&s), start);
    }

    /// A border whose two sides can't make one rectangle (a one-row cell beside a cell merged
    /// down two rows) is hidden on both sides instead of refused; nothing merges; one undo step.
    #[test]
    fn erasing_between_cells_that_dont_line_up_hides_the_border() {
        let mut s = merged_columns_table();
        let start = first_table(&s);
        let a = cell_rect(&mut s, 0, 0);
        s.run("table.eraser", &json!({"page": 0, "x": a.right() + 0.5, "y": a.y + a.h / 2.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), shape(&start), "no merge");
        for r in 0..2 {
            let right = t.rows[r].cells[0].props.borders.and_then(|b| b.right);
            let left = t.rows[r].cells[1].props.borders.and_then(|b| b.left);
            assert!(right.is_some_and(|b| !b.is_visible()) && left.is_some_and(|b| !b.is_visible()), "row {r}");
        }
        assert!(t.rows[2].cells[0].props.borders.is_none(), "rows outside the edge are untouched");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(first_table(&s), start);
        // A cell spanning two columns above single cells: the horizontal border is hidden too.
        let a = cell_rect(&mut s, 0, 1);
        s.run("table.eraser", &json!({"page": 0, "x": a.right() + 0.5, "y": a.y + 3.0})).unwrap();
        let merged = cell_rect(&mut s, 0, 1);
        let before = first_table(&s);
        s.run("table.eraser", &json!({"page": 0, "x": merged.x + 3.0, "y": merged.bottom() + 0.5})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), shape(&before), "no merge");
        let vis = |r: usize, c: usize, top: bool| {
            let b = t.rows[r].cells[c].props.borders;
            b.and_then(|b| if top { b.top } else { b.bottom }).is_some_and(|b| !b.is_visible())
        };
        assert!(vis(1, 1, false) && vis(2, 1, true), "both sides where the cells meet");
        assert!(!vis(2, 2, true), "the cell beside keeps its border under the wide cell, as in Word");
        assert!(!vis(2, 0, true), "the first column is not part of the edge");
    }
}
