//! Table Layout › Draw: Draw Table and Eraser. A pen stroke on a page either draws a table (a
//! rectangle: a one-cell table, a table nested in a cell, or a new row under a table) or
//! splits the cells a straight line crosses; the eraser removes the borders a click or a drag
//! touches: the two cells beside a border merge when they make one rectangle, else it is hidden
//! (as the table's outside always is). Coordinates are page
//! points, like the layout's; the app turns the pen and eraser on as canvas modes and sends each
//! finished stroke or click here, so every stroke is one undo step.

use std::collections::BTreeSet;

use serde_json::Value;
use wordcraft_doc::props::{HeightRule, VMerge};
use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Block, Cell, Document, PartKind, Path, Pos, StoryRef, Table, para_block};
use wordcraft_geom::Rect;
use wordcraft_layout::{DocLayout, Placed};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// A drag shorter than this (points) both ways isn't a rectangle.
pub const MIN_RECT: f32 = 12.0;
/// How near (points) a click has to be to a border, or a drawn rectangle to a table's bottom,
/// when the command doesn't say (`reach`).
pub const ERASER_REACH: f32 = 4.0;
/// A line closer than this (points) to an existing border lands on it.
const SNAP: f32 = 3.0;
/// The shortest stroke (points) that counts as a line.
pub const MIN_LINE: f32 = 6.0;
/// The share of a cell a line has to run along to split it when it runs on past the cell.
const GRAZE: f32 = 0.4;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("table.draw", "Draw Table", "Insert › Tables", draw).params(
            r#"{"page": n, "x0": pt, "y0": pt, "x1": pt, "y1": pt, "reach"?: pt, "story"?: StoryRef} (a pen stroke in page points, 0-based page. A rectangle at least 12 pt each way makes a table: with its top at a table's bottom edge (within `reach`, default 4, at most 24) a new last row of that table, one cell across and as tall as drawn; held inside one cell, a table nested in that cell; clear of every table, a one-cell table of that size at the body paragraph under its start (in `story` when given). A straight vertical or horizontal line inside a table splits every cell it crosses there. In the app the button turns the pen on.)"#,
        ),
        CommandSpec::new("table.eraser", "Eraser", "Table Layout › Draw", eraser).params(
            r#"{"page": n, "x": pt, "y": pt} or {"page": n, "x0": pt, "y0": pt, "x1": pt, "y1": pt}, "reach"?: pt (a click erases the border within `reach` (default 4, at most 24) of the point, a border between two cells before the table's outside; a drag erases every border the rectangle from (x0, y0) to (x1, y1) touches, horizontal lines first. Two cells that make one rectangle merge, including cells merged down the same rows; otherwise the border is hidden: along the whole edge for a vertical line, where the two cells meet for a horizontal one, and always on the table's outside. In the app the button turns the eraser on.)"#,
        ),
    ]
}

/// What a pen stroke does, judged from the layout (also the app's live preview).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stroke {
    /// Draw a table in this rectangle (page points): a new row of the table it starts under, a
    /// table nested in the cell that holds it, else a new one-cell table.
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
    /// A row a vertical merge covers (its first cell's box spans the rows).
    covered: bool,
}

fn cell_boxes(l: &DocLayout, page: usize) -> Vec<CellBox> {
    let Some(pg) = l.pages.get(page) else { return Vec::new() };
    pg.items
        .iter()
        .filter_map(|it| match it {
            Placed::Cell { rect, table, row, cell, story, covered, .. } => {
                Some(CellBox { rect: *rect, table: table.clone(), story: *story, row: *row, cell: *cell, covered: *covered })
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

/// Classify a pen stroke: a straight line inside a table, a rectangle that makes a table (see
/// [`Stroke::Rect`]; `reach` is how near its top has to be to a table's bottom), else `None`.
pub fn stroke(l: &DocLayout, page: usize, x0: f32, y0: f32, x1: f32, y1: f32, reach: f32) -> Option<Stroke> {
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    // The same reach as the command's, so the preview shows what the stroke will do at any zoom.
    let reach = if reach.is_finite() { reach.clamp(0.5, 24.0) } else { ERASER_REACH };
    let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
    let boxes = cell_boxes(l, page);
    let r = Rect::new(x0.min(x1), y0.min(y1), dx, dy);
    let big = dx >= MIN_RECT && dy >= MIN_RECT;
    let made = big.then(|| drawn(&boxes, &r, (x0, y0), reach)).flatten();
    // A rectangle from a table's bottom edge adds a row, and one held in a cell nests a table,
    // also when it is wide and low (or narrow and tall) or starts on the edge, where it would
    // otherwise read as a line; a stroke that wavers less than four reaches (24 screen px in the
    // app, at any zoom) off straight stays a line.
    let wobble = dx.min(dy) <= MIN_RECT.max(dx.max(dy) * 0.2);
    if matches!(made, Some(Drawn::Row(..) | Drawn::Nested(..))) && (!wobble || dx.min(dy) >= 4.0 * reach) {
        return Some(Stroke::Rect(r));
    }
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    // Either end or the middle in a table: a line is classed the same whichever way it is drawn.
    if table_at(&boxes, x0, y0).is_some() || table_at(&boxes, mx, my).is_some() || table_at(&boxes, x1, y1).is_some() {
        if dy >= dx && dy >= MIN_LINE && dx <= MIN_RECT.max(dy * 0.2) {
            return Some(Stroke::Vertical { x: mx, y0: y0.min(y1), y1: y0.max(y1) });
        }
        if dx > dy && dx >= MIN_LINE && dy <= MIN_RECT.max(dx * 0.2) {
            return Some(Stroke::Horizontal { y: my, x0: x0.min(x1), x1: x0.max(x1) });
        }
    }
    match made {
        // Clear of the cells but beside a table (in the margin next to a row): the text under the
        // start is a cell's, where no new table can go, so the preview doesn't promise one.
        Some(Drawn::Clear) if l.hit(page, x0, y0, StoryRef::Body).is_some_and(|p| p.path.cell().is_some()) => None,
        Some(_) => Some(Stroke::Rect(r)),
        None => None,
    }
}

/// What a rectangle drawn with the pen makes.
#[derive(Clone, Debug)]
enum Drawn {
    /// A new last row of this table: the rectangle starts at its bottom edge.
    Row(StoryRef, Path),
    /// A table nested in this cell, which holds the whole rectangle.
    Nested(CellBox),
    /// A new one-cell table, the rectangle clear of every table.
    Clear,
}

/// What rectangle `r`, drawn from `start`, makes among the cells `boxes`; `None` when it covers
/// part of a table.
fn drawn(boxes: &[CellBox], r: &Rect, start: (f32, f32), reach: f32) -> Option<Drawn> {
    // Each table's left, right and bottom.
    let mut tables: Vec<(StoryRef, &Path, f32, f32, f32)> = Vec::new();
    for b in boxes {
        match tables.iter_mut().find(|t| t.0 == b.story && *t.1 == b.table) {
            Some(t) => (t.2, t.3, t.4) = (t.2.min(b.rect.x), t.3.max(b.rect.right()), t.4.max(b.rect.bottom())),
            None => tables.push((b.story, &b.table, b.rect.x, b.rect.right(), b.rect.bottom())),
        }
    }
    // The table whose bottom is nearest the rectangle's top, the rectangle reaching below it; on
    // a tie the innermost, so a nested table's row wins only when it really ends there.
    let near = tables
        .iter()
        .filter(|t| (r.y - t.4).abs() <= reach && r.bottom() >= t.4 + MIN_RECT && r.x < t.3 && r.right() > t.2)
        .min_by(|a, b| (r.y - a.4).abs().total_cmp(&(r.y - b.4).abs()).then(b.1.0.len().cmp(&a.1.0.len())));
    if let Some(t) = near {
        return Some(Drawn::Row(t.0, t.1.clone()));
    }
    if table_at(boxes, start.0, start.1).is_some() {
        // Among the innermost cells under the start (two on a border between cells), the one that
        // holds the whole rectangle: a start on a border nests in the cell drawn into, and one in
        // a nested table that runs out of it makes nothing (it covers part of that table).
        let under: Vec<&CellBox> = boxes.iter().filter(|b| !b.covered && inside(&b.rect, start.0, start.1)).collect();
        let depth = under.iter().map(|b| b.table.0.len()).max()?;
        let holds = |c: &Rect| r.x >= c.x && r.right() <= c.right() && r.y >= c.y && r.bottom() <= c.bottom();
        let cell = under.into_iter().find(|b| b.table.0.len() == depth && holds(&b.rect));
        return cell.map(|b| Drawn::Nested(b.clone()));
    }
    let clear = |c: &Rect| r.right() <= c.x || r.x >= c.right() || r.bottom() <= c.y || r.y >= c.bottom();
    boxes.iter().all(|b| clear(&b.rect)).then_some(Drawn::Clear)
}

/// Where a pen line goes when drawn (the app's preview): across the whole of every cell it
/// splits, and no further than the table, as in Word. `None` for a rectangle or a line that
/// splits nothing.
pub fn line_extent(l: &DocLayout, page: usize, stroke: Stroke) -> Option<((f32, f32), (f32, f32))> {
    let boxes = cell_boxes(l, page);
    match stroke {
        Stroke::Vertical { x, y0, y1 } => {
            let (_, _, cells) = crossed(&boxes, (x, (y0 + y1) / 2.0), |r| splits_column(r, x, (y0, y1)))?;
            let top = cells.iter().map(|c| c.2.y).fold(f32::INFINITY, f32::min);
            let bottom = cells.iter().map(|c| c.2.bottom()).fold(f32::NEG_INFINITY, f32::max);
            (top < bottom).then_some(((x, top), (x, bottom)))
        }
        Stroke::Horizontal { y, x0, x1 } => {
            let (_, _, cells) = crossed(&boxes, ((x0 + x1) / 2.0, y), |r| splits_row(r, y, (x0, x1)))?;
            let left = cells.iter().map(|c| c.2.x).fold(f32::INFINITY, f32::min);
            let right = cells.iter().map(|c| c.2.right()).fold(f32::NEG_INFINITY, f32::max);
            (left < right).then_some(((left, y), (right, y)))
        }
        Stroke::Rect(_) => None,
    }
}

/// Whether a pen line from `a` to `b` runs along enough of a cell from `lo` to `hi` to split it
/// (Word splits every cell it crosses): more than a hair, and either all of the line is in the
/// cell or it covers two fifths of it (Word: a line from the middle of A1 to the middle of B1
/// splits both). A line only grazing the next cell leaves it whole: at 100 %
/// a row is about 20 px and a stroke needs 23, so a line in one row always runs a little over.
fn runs_along(lo: f32, hi: f32, a: f32, b: f32) -> bool {
    let over = b.min(hi) - a.max(lo);
    over > SNAP && ((a >= lo - SNAP && b <= hi + SNAP) || over >= (hi - lo) * GRAZE)
}

/// Whether a vertical pen line at `x` from `ya` to `yb` splits the cell drawn at `r`: it runs
/// down the cell (see [`runs_along`]), clear of its sides.
fn splits_column(r: &Rect, x: f32, (ya, yb): (f32, f32)) -> bool {
    x > r.x + SNAP && x < r.right() - SNAP && runs_along(r.y, r.bottom(), ya, yb)
}

/// Whether a horizontal pen line at `y` from `xa` to `xb` splits the cell drawn at `r`.
fn splits_row(r: &Rect, y: f32, (xa, xb): (f32, f32)) -> bool {
    y > r.y + SNAP && y < r.bottom() - SNAP && runs_along(r.x, r.right(), xa, xb)
}

fn page_param(v: &Value) -> Result<usize, CmdError> {
    p::u64(v, "page").map(|n| usize::try_from(n).unwrap_or(usize::MAX)).ok_or_else(|| CmdError::Params("`page` (number) is required".into()))
}

fn draw(s: &mut Session, v: &Value) -> CmdResult {
    let page = page_param(v)?;
    let (x0, y0, x1, y1) = (p::req_f32(v, "x0")?, p::req_f32(v, "y0")?, p::req_f32(v, "x1")?, p::req_f32(v, "y1")?);
    let reach = p::f32(v, "reach").map_or(ERASER_REACH, |r| r.clamp(0.5, 24.0));
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    match stroke(&l, page, x0, y0, x1, y1, reach) {
        Some(Stroke::Rect(r)) => draw_table(s, v, &l, page, (x0, y0), r, reach),
        Some(Stroke::Vertical { x, y0, y1 }) => split_columns(s, &l, page, x, (y0, y1)),
        Some(Stroke::Horizontal { y, x0, x1 }) => split_rows(s, &l, page, y, (x0, x1)),
        None => Err(CmdError::Failed(
            "draw a rectangle (at least 12 pt each way) clear of the tables, inside one cell or under a table, or a straight horizontal or vertical line across table cells".into(),
        )),
    }
}

/// A rectangle drawn with the pen, as in Word: a new last row of the table it starts under, a
/// table nested in the cell that holds it, else a one-cell table at the paragraph under its start.
fn draw_table(s: &mut Session, v: &Value, l: &DocLayout, page: usize, start: (f32, f32), r: Rect, reach: f32) -> CmdResult {
    match drawn(&cell_boxes(l, page), &r, start, reach) {
        Some(Drawn::Row(story, tp)) => add_row(s, story, &tp, r.h),
        Some(Drawn::Nested(cell)) => nested_table(s, l, page, start, r, &cell),
        Some(Drawn::Clear) => new_table(s, v, l, page, start, r),
        None => Err(CmdError::Failed("draw a new table clear of the tables, inside one cell or just under a table".into())),
    }
}

/// A one-cell table `w` wide whose row is at least `h` tall.
fn one_cell(w: f32, h: f32) -> Table {
    let mut t = Table::new(1, 1, w);
    t.props.width = Some(w);
    if let Some(row) = t.rows.first_mut() {
        row.props.height = Some(h.clamp(4.0, 1584.0));
        row.props.height_rule = HeightRule::AtLeast;
    }
    t
}

/// A one-cell table the size of `r`, at the paragraph under the stroke's start.
fn new_table(s: &mut Session, v: &Value, l: &DocLayout, page: usize, start: (f32, f32), r: Rect) -> CmdResult {
    // The pen draws on the page: the body, not wherever the caret was (a footnote, a header left
    // open); the caret's header or footer only for a rectangle drawn above or below the body text.
    let given = v.get("story").and_then(|x| serde_json::from_value::<StoryRef>(x.clone()).ok());
    let body = l.pages.get(page).map(|pg| pg.body);
    let in_margin = body.is_some_and(|b| start.1 < b.y || start.1 > b.bottom());
    let caret = s.sel.focus.story;
    let hf = matches!(caret, StoryRef::Part(id) if s.doc.parts.get(&id).is_some_and(|p| matches!(p.kind, PartKind::Header | PartKind::Footer)));
    let story = given.unwrap_or(if in_margin && hf { caret } else { StoryRef::Body });
    let pos = l.hit(page, start.0, start.1, story).ok_or_else(|| CmdError::Failed("there's no text to put a table at there".into()))?;
    let pos = s.doc.clamp(&pos);
    if pos.path.cell().is_some() {
        return Err(CmdError::Failed("draw new tables outside a table; inside one, draw a line to split cells".into()));
    }
    let len = s.doc.para(pos.story, &pos.path).map(|q| q.len()).ok_or_else(|| CmdError::Failed("there's no paragraph there".into()))?;
    let tw = super::page::sect(s).text_width().max(18.0);
    let w = r.w.clamp(18.0, tw);
    let mut t = one_cell(w, r.h);
    let body_x = l.pages.get(page).map_or(0.0, |pg| pg.body.x);
    let indent = r.x - body_x;
    if indent > 1.0 {
        t.props.indent = Some(indent.min((tw - w).max(0.0)));
    }
    // Before the paragraph when the stroke starts at its beginning (or it's empty), else after it.
    let at = if pos.off == 0 || len == 0 { pos.path.clone() } else { s.doc.split_paragraph(&Pos { off: len, ..pos.clone() })?.path };
    s.doc.insert_block(pos.story, &at, Block::Table(t))?;
    let mut first = at.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story: pos.story, path: Path(first), off: 0 });
    sel_result(s)
}

/// A one-cell table the size of `r` nested in `cell`, at the paragraph under the stroke's start:
/// before it when the stroke starts at its beginning (or it's empty), else after it.
fn nested_table(s: &mut Session, l: &DocLayout, page: usize, start: (f32, f32), r: Rect, cell: &CellBox) -> CmdResult {
    let hit = l.hit(page, start.0, start.1, cell.story).map(|p| s.doc.clamp(&p));
    // The nearest text can be another cell's (a neighbour whose text sits nearer the start than
    // this cell's, a table nested here): then the end of this cell's last paragraph.
    let pos = match hit.filter(|p| p.path.cell().is_some_and(|(tp, row, col)| tp == cell.table && row == cell.row && col == cell.cell)) {
        Some(p) => p,
        None => last_in_cell(s, cell).ok_or_else(|| CmdError::Failed("there's no cell text to put a table at there".into()))?,
    };
    let len = s.doc.para(pos.story, &pos.path).map(|q| q.len()).ok_or_else(|| CmdError::Failed("there's no paragraph there".into()))?;
    // The room between the cell's margins.
    let m = s.doc.table(cell.story, &cell.table).map_or([0.0, 5.4, 0.0, 5.4], |t| wordcraft_layout::cell_margins(&s.doc, t, cell.row, cell.cell));
    let room = (cell.rect.w - m[1] - m[3]).max(18.0);
    let w = r.w.clamp(18.0, room);
    let mut t = one_cell(w, r.h);
    let indent = r.x - cell.rect.x - m[1];
    if indent > 1.0 {
        t.props.indent = Some(indent.min((room - w).max(0.0)));
    }
    let at = if pos.off == 0 || len == 0 { pos.path.clone() } else { s.doc.split_paragraph(&Pos { off: len, ..pos.clone() })?.path };
    s.doc.insert_block(pos.story, &at, Block::Table(t))?;
    let mut first = at.0.clone();
    first.extend([0, 0, 0]);
    s.sel = Selection::caret(Pos { story: pos.story, path: Path(first), off: 0 });
    sel_result(s)
}

/// The end of the last paragraph directly in `cell` (not in a table nested there).
fn last_in_cell(s: &Session, cell: &CellBox) -> Option<Pos> {
    let blocks = &s.doc.table(cell.story, &cell.table)?.rows.get(cell.row)?.cells.get(cell.cell)?.blocks;
    let (i, len) = blocks.iter().enumerate().rev().find_map(|(i, b)| b.as_para().map(|q| (i, q.len())))?;
    let mut path = cell.table.0.clone();
    path.extend([u32::try_from(cell.row).ok()?, u32::try_from(cell.cell).ok()?, u32::try_from(i).ok()?]);
    Some(Pos { story: cell.story, path: Path(path), off: len })
}

/// A new last row of table `tp`: one cell across the whole table, at least `h` tall (Word draws
/// it as one cell however many columns the table has).
fn add_row(s: &mut Session, story: StoryRef, tp: &Path, h: f32) -> CmdResult {
    let t = s.doc.table_mut(story, tp)?;
    if t.rows.len() >= MAX_ROWS {
        return Err(CmdError::Failed("the table has too many rows".into()));
    }
    let mut cell = Cell::empty();
    cell.props.span = u32::try_from(t.grid.len().clamp(1, 63)).unwrap_or(1);
    let mut row = wordcraft_doc::Row { props: Default::default(), cells: vec![cell], controls: Default::default() };
    row.props.height = Some(h.clamp(4.0, 1584.0));
    row.props.height_rule = HeightRule::AtLeast;
    t.rows.push(row);
    super::table::sync_cell_widths(t);
    let r = t.rows.len().saturating_sub(1);
    finish(s, story, tp, r, 0)
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
fn hide_edge(t: &mut Table, vertical: bool, line: usize, (lo, hi): (usize, usize)) {
    let nil = wordcraft_doc::props::Border { style: wordcraft_doc::props::BorderStyle::None, width: 0.0, color: None, space: 0.0 };
    // Only a cell whose edge lies on the line is touched.
    let set_v = |t: &mut Table, r: usize, g: usize, before: bool| {
        let Some(ci) = t.cell_at_grid(r, g) else { return };
        let start = t.grid_col(r, ci);
        let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(ci)) else { return };
        let end = start + cell.span();
        let b = cell.props.borders.get_or_insert_with(Default::default);
        if before && end == line {
            b.right = Some(nil);
        } else if !before && start == line {
            b.left = Some(nil);
        }
    };
    let set_h = |t: &mut Table, r: usize, g: usize, above: bool| {
        let Some(ci) = t.cell_at_grid(r, g) else { return };
        let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(ci)) else { return };
        let b = cell.props.borders.get_or_insert_with(Default::default);
        if above {
            b.bottom = Some(nil);
        } else {
            b.top = Some(nil);
        }
    };
    if vertical {
        for r in lo..=hi.min(t.rows.len().saturating_sub(1)) {
            if let Some(l) = line.checked_sub(1) {
                set_v(t, r, l, true);
            }
            set_v(t, r, line, false);
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
                set_h(t, above, g, true);
            }
            set_h(t, line, g, false);
        }
    }
}

/// A vertical line at page `x`: split each crossed cell into two columns there.
fn split_columns(s: &mut Session, l: &DocLayout, page: usize, x: f32, (ya, yb): (f32, f32)) -> CmdResult {
    let boxes = cell_boxes(l, page);
    let hit = |r: &Rect| splits_column(r, x, (ya, yb));
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
    let mut crossed_set: std::collections::BTreeSet<(usize, usize)> = cells.iter().map(|c| (c.0, c.1)).collect();
    // A vertically merged cell splits as a whole, also when the line runs down only part of it
    // (the preview runs it down the whole cell).
    for &(r, c, _) in &cells {
        let (g0, g1) = vgroup(t, r, c);
        let gc = t.grid_col(r, c);
        for rr in g0..=g1 {
            if let Some(ci) = t.cell_at_grid(rr, gc) {
                crossed_set.insert((rr, ci));
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
/// the row's other cells span both rows (a vertical merge). A line through a vertically merged
/// cell splits the merge there: on a border the eraser took away (between two of its rows, or
/// across a row the merge leaves empty) it draws that border back without adding a row.
fn split_rows(s: &mut Session, l: &DocLayout, page: usize, y: f32, (xa, xb): (f32, f32)) -> CmdResult {
    let boxes = cell_boxes(l, page);
    let hit = |r: &Rect| splits_row(r, y, (xa, xb));
    let Some((story, tp, cells)) = crossed(&boxes, ((xa + xb) / 2.0, y), hit) else {
        return Err(CmdError::Failed("the line doesn't cross a table cell".into()));
    };
    let rows = row_spans(&boxes, story, &tp);
    let Some(&(r, top, bottom)) = rows.iter().find(|row| (row.1..row.2).contains(&y)) else {
        return Err(CmdError::Failed("the line doesn't cross a table cell".into()));
    };
    let t = s.doc.table(story, &tp).ok_or_else(|| CmdError::Failed("the table isn't there any more".into()))?;
    // The grid columns the line crosses (a merged cell's box is its first row's) and row `r`'s
    // cells there.
    let cols: BTreeSet<usize> = cells.iter().map(|&(cr, cc, _)| t.grid_col(cr, cc)).collect();
    let row_len = t.rows.get(r).map_or(0, |row| row.cells.len());
    let crossed_set: BTreeSet<usize> = (0..row_len).filter(|&ci| cols.contains(&t.grid_col(r, ci))).collect();
    let Some(&c0) = crossed_set.first() else { return Err(CmdError::Failed("the line doesn't cross a table cell".into())) };
    // On the border between two rows of a merge: split the merge there.
    let has = |k: usize| rows.iter().any(|x| x.0 == k);
    let border = if r > 0 && has(r - 1) && y - top < SNAP {
        Some(r - 1)
    } else if has(r + 1) && bottom - y < SNAP {
        Some(r)
    } else {
        None
    };
    if let Some(k) = border {
        let t = s.doc.table_mut(story, &tp)?;
        if !unmerge_below(t, k, &cols) {
            return Err(CmdError::Failed("there's already a border there".into()));
        }
        return finish(s, story, &tp, k + 1, c0);
    }
    // An exact height leaves out the row's bottom margin (its cells' largest), which the layout
    // adds below it: take it off each half so the two rows draw where the line went.
    let margin_below = |k: usize| {
        t.rows.get(k).map_or(0.0, |row| (0..row.cells.len()).map(|ci| wordcraft_layout::cell_margins(&s.doc, t, k, ci)[2]).fold(0.0f32, f32::max))
    };
    let exact = |k: usize| t.rows.get(k).is_some_and(|row| row.props.height_rule == HeightRule::Exact);
    let continued = |rr: usize, g: usize| {
        t.cell_at_grid(rr, g)
            .filter(|&ci| t.grid_col(rr, ci) == g)
            .and_then(|ci| t.rows.get(rr)?.cells.get(ci))
            .is_some_and(|c| c.props.vmerge == VMerge::Continue)
    };
    // Crossed cells whose merge runs on below this row.
    let runs_on: BTreeSet<usize> = crossed_set.iter().copied().filter(|&ci| continued(r + 1, t.grid_col(r, ci))).collect();
    // Two rows the eraser joined: every cell of the lower one continues a merge, so the pair
    // draws as one row (the layout shares the merges' height between them). A line through the
    // pair splits the merges between its rows again, which take the drawn heights; no row is added.
    let only_continues =
        |k: usize| t.rows.get(k).is_some_and(|row| !row.cells.is_empty() && row.cells.iter().all(|c| c.props.vmerge == VMerge::Continue));
    let joined = rows.iter().find_map(|x| {
        let below = rows.iter().find(|n| n.0 == x.0 + 1)?;
        (only_continues(x.0 + 1) && (x.1..below.2).contains(&y)).then_some((x.0, x.1, below.2))
    });
    if let Some((k, upper_top, lower_bottom)) = joined {
        let rule = if exact(k) { HeightRule::Exact } else { HeightRule::AtLeast };
        let less = if rule == HeightRule::Exact { margin_below(k) } else { 0.0 };
        let heights = [(k, y - upper_top - less), (k + 1, lower_bottom - y - less)];
        let first = (0..t.rows.get(k + 1).map_or(0, |row| row.cells.len())).find(|&ci| cols.contains(&t.grid_col(k + 1, ci))).unwrap_or(0);
        let t = s.doc.table_mut(story, &tp)?;
        if !unmerge_below(t, k, &cols) {
            return Err(CmdError::Failed("there's already a border there".into()));
        }
        for (k, h) in heights {
            if let Some(row) = t.rows.get_mut(k) {
                row.props.height = Some(h.clamp(1.0, 1584.0));
                row.props.height_rule = rule;
            }
        }
        return finish(s, story, &tp, k + 1, first);
    }
    if t.rows.len() >= MAX_ROWS {
        return Err(CmdError::Failed("the table has too many rows".into()));
    }
    let rule = if exact(r) { HeightRule::Exact } else { HeightRule::AtLeast };
    let less = if rule == HeightRule::Exact { margin_below(r) } else { 0.0 };
    let heights = (bottom > top).then(|| ((y - top - less).clamp(1.0, 1584.0), (bottom - y - less).clamp(1.0, 1584.0)));
    let t = s.doc.table_mut(story, &tp)?;
    let Some(row) = t.rows.get_mut(r) else { return Err(CmdError::Failed("the row isn't there any more".into())) };
    let mut new_row = wordcraft_doc::Row { props: row.props.clone(), cells: Vec::with_capacity(row.cells.len()), controls: Default::default() };
    new_row.props.header = false;
    if let Some((upper, lower)) = heights {
        row.props.height = Some(upper);
        row.props.height_rule = rule;
        new_row.props.height = Some(lower);
        new_row.props.height_rule = rule;
    }
    for (ci, cell) in row.cells.iter_mut().enumerate() {
        let mut nc = Cell::empty();
        nc.props = cell.props.clone();
        if crossed_set.contains(&ci) {
            // The part above the line keeps the text; a merge that started here stands alone,
            // and the part below starts the rest of the merge, if any.
            if cell.props.vmerge == VMerge::Restart {
                cell.props.vmerge = VMerge::None;
            }
            nc.props.vmerge = if runs_on.contains(&ci) { VMerge::Restart } else { VMerge::None };
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

/// Each row's index, top and bottom on the page for table `tp` among `boxes`. A row ends where
/// the next one starts (a merged cell's box runs over its rows, so it only gives its first
/// row's top); a row the layout draws with no height has its top at its bottom.
fn row_spans(boxes: &[CellBox], story: StoryRef, tp: &Path) -> Vec<(usize, f32, f32)> {
    let mut rows: Vec<(usize, f32, f32)> = Vec::new();
    for b in boxes.iter().filter(|b| b.story == story && b.table == *tp) {
        match rows.iter_mut().find(|r| r.0 == b.row) {
            Some(r) => (r.1, r.2) = (r.1.min(b.rect.y), r.2.max(b.rect.bottom())),
            None => rows.push((b.row, b.rect.y, b.rect.bottom())),
        }
    }
    rows.sort_by_key(|r| r.0);
    let tops: Vec<(usize, f32)> = rows.iter().map(|r| (r.0, r.1)).collect();
    for r in rows.iter_mut() {
        if let Some(&(_, next)) = tops.iter().find(|n| n.0 == r.0 + 1) {
            r.2 = next.max(r.1);
        }
    }
    rows
}

/// Split the vertical merges in grid columns `cols` between rows `k` and `k + 1`: the part
/// above keeps its text (a merge of one row is no merge), the part below starts a merge of its
/// own if it runs on. `false` when no merge runs across there.
fn unmerge_below(t: &mut Table, k: usize, cols: &BTreeSet<usize>) -> bool {
    let at = |t: &Table, r: usize, g: usize| t.cell_at_grid(r, g).filter(|&ci| t.grid_col(r, ci) == g);
    let vm = |t: &Table, r: usize, g: usize| at(t, r, g).and_then(|ci| t.rows.get(r)?.cells.get(ci)).map(|c| c.props.vmerge);
    let mut changed = false;
    for &g in cols {
        if vm(t, k + 1, g) != Some(VMerge::Continue) {
            continue;
        }
        let lower = if vm(t, k + 2, g) == Some(VMerge::Continue) { VMerge::Restart } else { VMerge::None };
        let (above, below) = (at(t, k, g), at(t, k + 1, g));
        if vm(t, k, g) == Some(VMerge::Restart)
            && let Some(c) = above.and_then(|ci| t.rows.get_mut(k)?.cells.get_mut(ci))
        {
            c.props.vmerge = VMerge::None;
        }
        if let Some(c) = below.and_then(|ci| t.rows.get_mut(k + 1)?.cells.get_mut(ci)) {
            c.props.vmerge = lower;
            changed = true;
        }
    }
    changed
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
/// goes to; `None` when nothing is there any more (an earlier stretch merged it away).
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
                    hide_edge(t, true, line, (a.top.min(b.top), a.bot.max(b.bot)));
                    Some((a.top, a.g0))
                }
                (Some(c), None) | (None, Some(c)) => {
                    hide_edge(t, true, line, (c.top, c.bot));
                    Some((c.top, c.g0))
                }
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
                hide_edge(t, false, line, (lc.g0, lc.g1));
                return Some((lc.top, lc.g0));
            };
            let (la, lb) = (logical(t, ra, ca), logical(t, rb, cb));
            if la.top == lb.top {
                return None;
            }
            // Cells that don't line up: only the stretch they share is hidden, as in Word.
            if la.g0 != lb.g0 || la.g1 != lb.g1 {
                hide_edge(t, false, line, (g0.max(la.g0).max(lb.g0), g1.min(la.g1).min(lb.g1)));
                return Some((la.top, la.g0));
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

fn eraser(s: &mut Session, v: &Value) -> CmdResult {
    let page = page_param(v)?;
    let reach = p::f32(v, "reach").map_or(ERASER_REACH, |r| r.clamp(0.5, 24.0));
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

    /// As in Word: a rectangle starting at a table's bottom edge adds a row to it (one cell
    /// across, as tall as drawn); one held inside a cell nests a table there; one over part of a
    /// table makes nothing; one with a gap under the table is a second table.
    #[test]
    fn a_drawn_rectangle_adds_a_row_or_nests_a_table() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 3})).unwrap();
        let (a, c) = (cell_rect(&mut s, 1, 0), cell_rect(&mut s, 1, 2));
        s.run("table.draw", &json!({"page": 0, "x0": a.x + 1.0, "y0": a.bottom() + 1.0, "x1": c.right(), "y1": a.bottom() + 41.0})).unwrap();
        let t = first_table(&s);
        assert_eq!(shape(&t), json!([[1, 1, 1], [1, 1, 1], [3]]));
        assert_eq!(t.rows[2].props.height, Some(40.0));
        assert_eq!(s.sel.focus.path.cell().map(|(_, r, c)| (r, c)), Some((2, 0)));
        let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
            Block::Table(t) => Some(t),
            _ => None,
        });
        t.unwrap().rows[0].props.height = Some(80.0);
        s.touch();
        let b = cell_rect(&mut s, 0, 1);
        s.run("table.draw", &json!({"page": 0, "x0": b.x + 10.0, "y0": b.y + 10.0, "x1": b.x + 60.0, "y1": b.y + 50.0})).unwrap();
        let t = first_table(&s);
        let nested = t.rows[0].cells[1].blocks.iter().find_map(|x| x.as_table()).expect("a table nested in B1");
        assert_eq!(nested.props.width, Some(50.0));
        assert!(nested.props.indent.is_some_and(|i| (i - 4.6).abs() < 0.1), "from the cell's text: {:?}", nested.props.indent);
        let before = s.doc.body.clone();
        let a = cell_rect(&mut s, 0, 0);
        assert!(s.run("table.draw", &json!({"page": 0, "x0": a.x + 10.0, "y0": a.y - 30.0, "x1": a.x + 60.0, "y1": a.y + 5.0})).is_err());
        assert_eq!(s.doc.body, before, "over part of a table: nothing");
        let last = cell_rect(&mut s, 2, 0);
        s.run("table.draw", &json!({"page": 0, "x0": last.x + 1.0, "y0": last.bottom() + 20.0, "x1": last.x + 100.0, "y1": last.bottom() + 60.0}))
            .unwrap();
        let tables: Vec<_> = s.doc.body.iter().filter_map(|b| b.as_table()).collect();
        assert_eq!(tables.len(), 2);
        assert_eq!(shape(tables[1]), json!([[1]]), "a second, one-cell table");
    }

    /// A rectangle drawn from on the table's bottom edge, or just above it (inside the last row),
    /// adds a row even when it is wide and low: it isn't taken for a line under the table, and
    /// the preview says the same.
    #[test]
    fn a_wide_low_rectangle_from_the_bottom_edge_adds_a_row() {
        for lift in [0.0, 2.0] {
            let mut s = Session::new(wordcraft_doc::Document::new());
            s.run("insert.table", &json!({"rows": 2, "cols": 3})).unwrap();
            let (a, c) = (cell_rect(&mut s, 1, 0), cell_rect(&mut s, 1, 2));
            let (x0, y0, x1, y1) = (a.x + 1.0, a.bottom() - lift, c.right(), a.bottom() - lift + 20.0);
            assert!(x1 - x0 > 5.0 * (y1 - y0), "wide and low enough to pass for a line");
            let l = s.layout();
            assert!(matches!(stroke(&l, 0, x0, y0, x1, y1, ERASER_REACH), Some(Stroke::Rect(_))), "preview, lift {lift}");
            s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x1, "y1": y1})).unwrap();
            assert_eq!(shape(&first_table(&s)), json!([[1, 1, 1], [1, 1, 1], [3]]), "lift {lift}");
        }
    }

    /// The first table's row heights set in place (the layout picks them up).
    fn set_row_height(s: &mut Session, row: usize, h: f32) {
        let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
            Block::Table(t) => Some(t),
            _ => None,
        });
        t.unwrap().rows[row].props.height = Some(h);
        s.touch();
    }

    /// A rectangle held in one cell nests a table there also when it is wide and low or narrow
    /// and tall: it isn't taken for a line that splits the cell.
    #[test]
    fn a_wide_low_or_narrow_tall_rectangle_in_a_cell_nests_a_table() {
        // Both within a fifth of a line's slant (25 ≤ 140 / 5, 18 ≤ 100 / 5).
        for (w, h) in [(140.0, 25.0), (18.0, 100.0)] {
            let mut s = Session::new(wordcraft_doc::Document::new());
            s.run("insert.table", &json!({"rows": 2, "cols": 3})).unwrap();
            set_row_height(&mut s, 0, 120.0);
            let b = cell_rect(&mut s, 0, 1);
            let (x0, y0) = (b.x + 5.0, b.y + 5.0);
            let l = s.layout();
            assert!(matches!(stroke(&l, 0, x0, y0, x0 + w, y0 + h, ERASER_REACH), Some(Stroke::Rect(_))), "preview {w}×{h}");
            s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x0 + w, "y1": y0 + h})).unwrap();
            let t = first_table(&s);
            assert_eq!(t.rows.len(), 2, "{w}×{h}: no row split");
            assert!(t.rows[0].cells[1].blocks.iter().any(|x| x.as_table().is_some()), "{w}×{h}: a table nested in B1");
        }
    }

    /// A line is classed the same whichever way it is drawn: from outside the table into C1 it
    /// splits C1 as the same line drawn from C1 outwards does.
    #[test]
    fn a_line_splits_the_same_drawn_either_way() {
        let shapes: Vec<Value> = [false, true]
            .into_iter()
            .map(|back| {
                let mut s = Session::new(wordcraft_doc::Document::new());
                s.run("insert.table", &json!({"rows": 1, "cols": 3})).unwrap();
                set_row_height(&mut s, 0, 60.0);
                let c = cell_rect(&mut s, 0, 2);
                let (y, a, b) = (c.y + 30.0, c.x + c.w * 0.25, c.right() + 150.0);
                let (x0, x1) = if back { (b, a) } else { (a, b) };
                s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y, "x1": x1, "y1": y})).unwrap();
                shape(&first_table(&s))
            })
            .collect();
        assert_eq!(shapes[0], shapes[1]);
        assert_eq!(shapes[0], json!([[1, 1, 1], [1, 1, 1]]), "C1 split");
    }

    /// A rectangle started on the border between two cells and drawn into the left one nests in
    /// that one (a start on a border is in both cells).
    #[test]
    fn a_rectangle_from_a_cell_border_nests_in_the_cell_it_fills() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        set_row_height(&mut s, 0, 80.0);
        let a = cell_rect(&mut s, 0, 0);
        assert!(cell_rect(&mut s, 0, 1).x <= a.right(), "the start is in B1 too");
        s.run("table.draw", &json!({"page": 0, "x0": a.right(), "y0": a.y + 10.0, "x1": a.right() - 50.0, "y1": a.y + 50.0})).unwrap();
        let t = first_table(&s);
        assert!(t.rows[0].cells[0].blocks.iter().any(|x| x.as_table().is_some()), "nested in A1");
        assert!(!t.rows[0].cells[1].blocks.iter().any(|x| x.as_table().is_some()), "not in B1");
    }

    /// A rectangle drawn low in a tall cell whose text is at its top nests in that cell, not in
    /// the cell below, whose text is nearer the start.
    #[test]
    fn a_rectangle_drawn_low_in_a_cell_nests_there() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        set_row_height(&mut s, 0, 80.0);
        let b = cell_rect(&mut s, 0, 1);
        s.run("table.draw", &json!({"page": 0, "x0": b.x + 60.0, "y0": b.bottom() - 6.0, "x1": b.x + 10.0, "y1": b.bottom() - 40.0})).unwrap();
        let t = first_table(&s);
        assert!(t.rows[0].cells[1].blocks.iter().any(|x| x.as_table().is_some()), "nested in B1");
        assert!(!t.rows[1].cells[1].blocks.iter().any(|x| x.as_table().is_some()), "not in B2");
        assert!(t.rows[0].cells[1].blocks.last().is_some_and(|x| x.as_para().is_some()), "the cell still ends with a paragraph");
    }

    /// A short vertical line in a vertically merged cell splits all of it, as its preview shows.
    #[test]
    fn a_short_line_in_a_merged_cell_splits_all_of_it() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
            Block::Table(t) => Some(t),
            _ => None,
        });
        let t = t.unwrap();
        t.rows[0].cells[0].props.vmerge = VMerge::Restart;
        t.rows[1].cells[0].props.vmerge = VMerge::Continue;
        s.touch();
        let a = cell_rect(&mut s, 0, 0);
        let x = a.x + a.w / 2.0;
        let line = (a.y + 2.0, a.y + 16.0);
        let l = s.layout();
        assert!(line_extent(&l, 0, Stroke::Vertical { x, y0: line.0, y1: line.1 }).is_some_and(|(p, q)| (q.1 - p.1 - a.h).abs() < 0.5));
        s.run("table.draw", &json!({"page": 0, "x0": x, "y0": line.0, "x1": x, "y1": line.1})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.grid.len(), 3);
        assert_eq!(shape(&t), json!([[1, 1, 1], [1, 1, 1]]), "both rows of the merged cell split");
        assert_eq!(t.rows[1].cells[1].props.vmerge, VMerge::Continue, "the right half stays merged down");
    }

    /// The pen draws on the page: a new table goes in the body even when the caret was in the
    /// header before the pen was picked up.
    #[test]
    fn a_new_table_goes_in_the_body_whatever_the_caret_story() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.editHeader", &json!({"page": 0})).unwrap();
        s.run("text.insert", &json!({"text": "Head"})).unwrap();
        assert_ne!(s.sel.focus.story, StoryRef::Body, "the caret is in the header");
        let body = s.layout().pages[0].body;
        let (x0, y0) = (body.x + 40.0, body.y + 2.0);
        s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x0 + 200.0, "y1": y0 + 60.0})).unwrap();
        assert!(s.doc.body.iter().any(|b| b.as_table().is_some()), "the table is in the body");
        // A `story` that isn't one is no story: still the body, not the caret's header.
        s.run("edit.undo", &json!({})).unwrap();
        s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x0 + 200.0, "y1": y0 + 60.0, "story": null})).unwrap();
        assert!(s.doc.body.iter().any(|b| b.as_table().is_some()), "`story: null` is the body too");
    }

    /// With the caret in the header, a rectangle drawn above the body text (in the header's
    /// area) still goes in the header.
    #[test]
    fn with_the_header_open_a_rectangle_above_the_text_goes_in_it() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let r = s.run("insert.editHeader", &json!({"page": 0})).unwrap();
        let id = u32::try_from(r["story"].as_u64().unwrap()).unwrap();
        s.run("text.insert", &json!({"text": "Head"})).unwrap();
        let body = s.layout().pages[0].body;
        let (x0, y0) = (body.x + 40.0, body.y - 30.0);
        s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x0 + 120.0, "y1": y0 + 20.0})).unwrap();
        assert!(s.doc.parts.get(&id).is_some_and(|p| p.blocks.iter().any(|b| b.as_table().is_some())), "in the header");
        assert!(!s.doc.body.iter().any(|b| b.as_table().is_some()), "not in the body");
    }

    /// A stroke that wavers a little off straight inside a tall cell stays a line at any zoom (the
    /// app's reach is 6 screen px, so 4 pt at 400 % and 24 pt at 25 %); in the last row it adds no
    /// row either.
    #[test]
    fn a_slanted_line_stays_a_line_at_any_zoom() {
        for reach in [4.0f32, 12.0, 24.0] {
            let mut s = Session::new(wordcraft_doc::Document::new());
            s.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap();
            set_row_height(&mut s, 0, 120.0);
            let a = cell_rect(&mut s, 0, 0);
            let (x0, y0) = (a.x + 20.0, a.y + 50.0);
            let l = s.layout();
            assert!(matches!(stroke(&l, 0, x0, y0, x0 + 300.0, y0 + 13.0, reach), Some(Stroke::Horizontal { .. })), "reach {reach}");
            for lift in [0.0f32, 2.0] {
                let (x0, y0) = (a.x + 20.0, a.bottom() - lift);
                assert!(
                    !matches!(stroke(&l, 0, x0, y0, x0 + 300.0, y0 + 13.0, reach), Some(Stroke::Rect(_))),
                    "last row, reach {reach}, lift {lift}"
                );
            }
        }
    }

    /// A rectangle started in a nested table and drawn out of it, down the outer cell, covers
    /// part of that table: it makes nothing, as over part of any table.
    #[test]
    fn a_rectangle_out_of_a_nested_table_makes_nothing() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap();
        set_row_height(&mut s, 0, 200.0);
        let a = cell_rect(&mut s, 0, 0);
        s.run("table.draw", &json!({"page": 0, "x0": a.x + 10.0, "y0": a.y + 5.0, "x1": a.x + 200.0, "y1": a.y + 40.0})).unwrap();
        let before = s.doc.body.clone();
        let r = s.run("table.draw", &json!({"page": 0, "x0": a.x + 30.0, "y0": a.y + 20.0, "x1": a.x + 120.0, "y1": a.y + 150.0}));
        assert!(r.is_err());
        assert_eq!(s.doc.body, before, "nothing changed");
    }

    /// A rectangle in the margin beside a table's row is clear of the cells, but the text under
    /// its start is a cell's: the preview doesn't promise a table the command refuses.
    #[test]
    fn beside_a_table_the_preview_and_the_command_agree() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 1, "cols": 3})).unwrap();
        set_row_height(&mut s, 0, 60.0);
        let c = cell_rect(&mut s, 0, 2);
        let (x0, y0, x1, y1) = (c.right() + 10.0, c.y + 2.0, c.right() + 60.0, c.y + 40.0);
        let l = s.layout();
        let shown = stroke(&l, 0, x0, y0, x1, y1, ERASER_REACH).is_some();
        let done = s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y0, "x1": x1, "y1": y1})).is_ok();
        assert!(!shown && !done, "preview {shown}, command {done}");
    }

    /// A rectangle from the outer table's bottom adds the row to the outer table, not to a table
    /// nested in its last row whose bottom is within a wide reach too (at low zoom).
    #[test]
    fn a_row_goes_to_the_table_whose_bottom_it_starts_at() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap();
        set_row_height(&mut s, 0, 80.0);
        let a = cell_rect(&mut s, 0, 0);
        // A tall nested table, its bottom just above the outer table's (the cell's last paragraph
        // between them).
        s.run("table.draw", &json!({"page": 0, "x0": a.x + 10.0, "y0": a.y + 5.0, "x1": a.x + 200.0, "y1": a.y + 70.0})).unwrap();
        let mut bottom = |depth: usize| {
            s.layout().pages[0]
                .items
                .iter()
                .filter_map(|it| match it {
                    Placed::Cell { rect, table, .. } if table.0.len() == depth => Some(rect.bottom()),
                    _ => None,
                })
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let (outer, inner) = (bottom(1), bottom(4));
        assert!(inner.is_finite() && outer - inner <= 24.0, "the nested table ends within reach: {inner} vs {outer}");
        s.run("table.draw", &json!({"page": 0, "x0": a.x + 1.0, "y0": outer, "x1": a.x + 100.0, "y1": outer + 30.0, "reach": 24})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows.len(), 2, "a row of the outer table");
        let nested = t.rows[0].cells[0].blocks.iter().find_map(|x| x.as_table()).unwrap();
        assert_eq!(nested.rows.len(), 1, "the nested table keeps its one row");
    }

    /// A line splits every cell it crosses (Word: one from the middle of A1 to the middle of B1
    /// splits both, not C1); the app's preview runs across the whole of those cells.
    #[test]
    fn a_line_splits_every_cell_it_crosses() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 1, "cols": 3})).unwrap();
        let t = s.doc.body.iter_mut().find_map(|b| match std::sync::Arc::make_mut(b) {
            Block::Table(t) => Some(t),
            _ => None,
        });
        t.unwrap().rows[0].props.height = Some(60.0);
        s.touch();
        let (a, b) = (cell_rect(&mut s, 0, 0), cell_rect(&mut s, 0, 1));
        let (y, x0, x1) = (a.y + 30.0, a.x + a.w * 0.5, b.x + b.w * 0.5);
        let l = s.layout();
        let line = stroke(&l, 0, x0, y, x1, y, ERASER_REACH).unwrap();
        assert_eq!(line_extent(&l, 0, line), Some(((a.x, y), (b.right(), y))), "the preview spans A1 and B1");
        s.run("table.draw", &json!({"page": 0, "x0": x0, "y0": y, "x1": x1, "y1": y})).unwrap();
        let t = first_table(&s);
        assert_eq!(t.rows.len(), 2);
        let vm: Vec<_> = t.rows[0].cells.iter().map(|c| c.props.vmerge).collect();
        assert_eq!(vm, [VMerge::None, VMerge::None, VMerge::Restart], "A1 and B1 split; C1 spans both rows");
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
