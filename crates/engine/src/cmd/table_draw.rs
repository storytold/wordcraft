//! Table Layout › Draw: Draw Table and Eraser. A pen stroke on a page either draws a new one-cell
//! table (a rectangle outside any table) or splits the cells a straight line crosses; the eraser
//! removes the border nearest a point by merging the two cells beside it. Coordinates are page
//! points, like the layout's; the app turns the pen and eraser on as canvas modes and sends each
//! finished stroke or click here, so every stroke is one undo step.

use serde_json::Value;
use wordcraft_doc::props::{HeightRule, VMerge};
use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Block, Cell, Path, Pos, StoryRef, Table, para_block};
use wordcraft_geom::Rect;
use wordcraft_layout::{DocLayout, Placed};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// A drag shorter than this (points) both ways isn't a rectangle.
pub const MIN_RECT: f32 = 12.0;
/// How near (points) the eraser has to be to a border.
pub const ERASER_REACH: f32 = 4.0;
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
            r#"{"page": n, "x": pt, "y": pt} (removes the border between two cells within 4 pt of the point by merging them. In the app the button turns the eraser on.)"#,
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
            Placed::Cell { rect, table, row, cell, story } => {
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
    let crossed_set: std::collections::BTreeSet<usize> = cells.iter().map(|c| c.1).collect();
    let merged = |ci: &usize| t.rows.get(r).and_then(|row| row.cells.get(*ci)).is_some_and(|c| c.props.vmerge != VMerge::None);
    if crossed_set.iter().any(merged) {
        return Err(CmdError::Failed("splitting a vertically merged cell across rows isn't supported; erase its merge first".into()));
    }
    let t = s.doc.table_mut(story, &tp)?;
    let Some(row) = t.rows.get_mut(r) else { return Err(CmdError::Failed("the row isn't there any more".into())) };
    let mut new_row = wordcraft_doc::Row { props: row.props.clone(), cells: Vec::with_capacity(row.cells.len()) };
    new_row.props.header = false;
    if top.is_finite() && bottom.is_finite() && bottom > top {
        let rule = if row.props.height_rule == HeightRule::Exact { HeightRule::Exact } else { HeightRule::AtLeast };
        row.props.height = Some((y - top).clamp(1.0, 1584.0));
        row.props.height_rule = rule;
        new_row.props.height = Some((bottom - y).clamp(1.0, 1584.0));
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

/// What the eraser would remove at a point: the border's two ends (page points) and the merge.
#[derive(Clone, Debug, PartialEq)]
pub struct EraseTarget {
    pub from: (f32, f32),
    pub to: (f32, f32),
    story: StoryRef,
    table: Path,
    /// (row, cell) of the cell left of / above the border.
    first: (usize, usize),
    /// (row, cell) of the cell right of / below it.
    second: (usize, usize),
    across: bool,
}

/// The border between two cells nearest page point (`x`, `y`), within [`ERASER_REACH`].
pub fn erase_target(l: &DocLayout, page: usize, x: f32, y: f32) -> Option<EraseTarget> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let boxes = cell_boxes(l, page);
    let mut best: Option<(f32, usize, EraseTarget)> = None;
    let mut offer = |d: f32, depth: usize, e: EraseTarget| {
        if d <= ERASER_REACH && best.as_ref().is_none_or(|(bd, bdepth, _)| d < *bd - 0.01 || (d <= *bd + 0.01 && depth > *bdepth)) {
            best = Some((d, depth, e));
        }
    };
    for a in &boxes {
        let (ar, ab) = (a.rect.right(), a.rect.bottom());
        let same = |b: &&CellBox| b.story == a.story && b.table == a.table;
        // The border on a's right, shared with the next cell of its row.
        if (x - ar).abs() <= ERASER_REACH
            && y >= a.rect.y
            && y <= ab
            && let Some(b) = boxes.iter().filter(same).find(|b| b.row == a.row && b.cell == a.cell + 1 && (b.rect.x - ar).abs() <= 1.5)
        {
            let e = EraseTarget {
                from: (ar, a.rect.y.max(b.rect.y)),
                to: (ar, ab.min(b.rect.bottom())),
                story: a.story,
                table: a.table.clone(),
                first: (a.row, a.cell),
                second: (b.row, b.cell),
                across: true,
            };
            offer((x - ar).abs(), a.table.0.len(), e);
        }
        // The border below a, shared with a cell of the next row.
        if (y - ab).abs() <= ERASER_REACH
            && x >= a.rect.x
            && x <= ar
            && let Some(b) =
                boxes.iter().filter(same).find(|b| b.row == a.row + 1 && (b.rect.y - ab).abs() <= 1.5 && x >= b.rect.x && x <= b.rect.right())
        {
            let e = EraseTarget {
                from: (a.rect.x.max(b.rect.x), ab),
                to: (ar.min(b.rect.right()), ab),
                story: a.story,
                table: a.table.clone(),
                first: (a.row, a.cell),
                second: (b.row, b.cell),
                across: false,
            };
            offer((y - ab).abs(), a.table.0.len(), e);
        }
    }
    best.map(|b| b.2)
}

fn eraser(s: &mut Session, v: &Value) -> CmdResult {
    let page = page_param(v)?;
    let (x, y) = (p::req_f32(v, "x")?, p::req_f32(v, "y")?);
    let l = s.layout();
    if l.pages.get(page).is_none() {
        return Err(CmdError::Params(format!("no page {page}")));
    }
    let e = erase_target(&l, page, x, y).ok_or_else(|| CmdError::Failed("there's no border between two cells there".into()))?;
    let (story, tp) = (e.story, e.table.clone());
    let t = s.doc.table_mut(story, &tp)?;
    let ((r, c), (r2, c2)) = (e.first, e.second);
    let cell = |t: &Table, r: usize, c: usize| t.rows.get(r).and_then(|row| row.cells.get(c)).map(|x| (x.props.vmerge, x.span()));
    let (Some((va, sa)), Some((vb, sb))) = (cell(t, r, c), cell(t, r2, c2)) else {
        return Err(CmdError::Failed("the cells aren't there any more".into()));
    };
    if e.across {
        if va != VMerge::None || vb != VMerge::None {
            return Err(CmdError::Failed("erasing a border beside a vertically merged cell isn't supported".into()));
        }
        t.merge_right(r, c, c2);
        super::table::sync_cell_widths(t);
        return finish(s, story, &tp, r, c);
    }
    let (ga, gb) = (t.grid_col(r, c), t.grid_col(r2, c2));
    if ga != gb || sa != sb {
        return Err(CmdError::Failed("the cells above and below the border don't line up, so they can't be merged".into()));
    }
    if vb == VMerge::Continue {
        return Err(CmdError::Failed("those cells are already merged".into()));
    }
    let (top, _) = vgroup(t, r, c);
    if let Some(a) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c))
        && a.props.vmerge == VMerge::None
    {
        a.props.vmerge = VMerge::Restart;
    }
    let mut moved = Vec::new();
    if let Some(b) = t.rows.get_mut(r2).and_then(|row| row.cells.get_mut(c2)) {
        b.props.vmerge = VMerge::Continue;
        let blocks = std::mem::replace(&mut b.blocks, vec![para_block(wordcraft_doc::Paragraph::new())]);
        moved.extend(blocks.into_iter().filter(|x| x.as_para().is_none_or(|q| !q.is_empty())));
    }
    let tc = t.cell_at_grid(top, ga).unwrap_or(0);
    if !moved.is_empty()
        && let Some(dst) = t.rows.get_mut(top).and_then(|row| row.cells.get_mut(tc))
    {
        if dst.blocks.len() == 1 && dst.blocks.first().and_then(|b| b.as_para()).is_some_and(wordcraft_doc::Paragraph::is_empty) {
            dst.blocks.clear();
        }
        dst.blocks.extend(moved);
    }
    finish(s, story, &tp, top, tc)
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
}
