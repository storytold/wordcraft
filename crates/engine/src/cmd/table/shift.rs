//! Insert Cells and Delete Cells (#6330), as Word's dialogs do them on the selected cells (or
//! the caret's cell):
//! - Insert Cells: shift cells right (the cells to their right move along and the row grows;
//!   no column is added), shift cells down (the cells below move down a row each, and the
//!   table gets as many rows at its bottom), insert entire rows above, or entire columns to
//!   the left.
//! - Delete Cells: shift cells left (the cells to their right close the gap and the row gets
//!   shorter), shift cells up (the cells below move up, and blank cells fill the bottom of the
//!   columns), delete entire rows, or entire columns.
//!
//! Cells keep their widths when they move sideways: the table's grid is redrawn around them, as
//! when a single row's border is dragged. A shift that would move part of a vertically merged
//! cell, or a cell wrapped in a content control, is refused instead of breaking it.

use std::collections::BTreeMap;
use std::ops::Range;

use wordcraft_doc::props::VMerge;
use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Cell, Path, Table};

use super::cells::{cell, selected_cells};
use super::clear::{cell_prefix, guard_delete, text_spans};
use super::layout::{fix_caret, touch_cells};
use super::structure::{del_col, del_row, delete_table_at, insert_cols, row};
use super::{CmdError, CmdResult, Session, Value, p};

/// The refusal when a shift would cut through a vertically merged cell or a content control.
const MERGED: &str = "Cells can't be shifted there: a vertically merged cell or a content control is in the way.";

/// The selection as the shifts see it: the table, its selected cells row by row (cell indexes,
/// ascending) and the grid columns `g0..=g1` they cover.
struct Picked {
    tp: Path,
    rows: BTreeMap<usize, Vec<usize>>,
    g0: usize,
    g1: usize,
}

impl Picked {
    fn read(s: &Session) -> Result<Picked, CmdError> {
        let (tp, r, c) = cell(s)?;
        let t = s.doc.table(s.sel.focus.story, &tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
        let mut rows: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (rr, cc) in selected_cells(s) {
            if t.rows.get(rr).is_some_and(|row| cc < row.cells.len()) {
                rows.entry(rr).or_default().push(cc);
            }
        }
        if rows.is_empty() {
            rows.insert(r, vec![c]);
        }
        let (mut g0, mut g1) = (usize::MAX, 0);
        for (&rr, cs) in &mut rows {
            cs.sort_unstable();
            cs.dedup();
            for &cc in cs.iter() {
                let g = t.grid_col(rr, cc);
                let span = t.rows.get(rr).and_then(|x| x.cells.get(cc)).map_or(1, Cell::span);
                g0 = g0.min(g);
                g1 = g1.max(g.saturating_add(span).saturating_sub(1));
            }
        }
        Ok(Picked { tp, rows, g0: g0.min(g1), g1 })
    }

    fn top(&self) -> usize {
        self.rows.keys().next().copied().unwrap_or(0)
    }

    /// How many rows the selection covers, top to bottom.
    fn height(&self) -> usize {
        let bottom = self.rows.keys().next_back().copied().unwrap_or(0);
        bottom.saturating_sub(self.top()).saturating_add(1)
    }

    /// The first selected cell of the top row.
    fn first_cell(&self) -> usize {
        self.rows.values().next().and_then(|cs| cs.first()).copied().unwrap_or(0)
    }
}

/// `table.insertCells`: `shift` is right, down, row or column (Word's dialog starts on down).
pub(super) fn insert_cells(s: &mut Session, v: &Value) -> CmdResult {
    match p::str(v, "shift").unwrap_or("down") {
        "right" => insert_right(s),
        "down" => insert_down(s),
        "row" => row(s, false),
        "column" => {
            let pk = Picked::read(s)?;
            let n = pk.g1.saturating_sub(pk.g0).saturating_add(1);
            insert_cols(s, pk.g0, n, (pk.top(), pk.first_cell()))
        }
        x => Err(CmdError::Params(format!("`shift` is right, down, row or column, not `{x}`"))),
    }
}

/// `table.deleteCells`: `shift` is left (the default, as in Word), up, row or column.
pub(super) fn delete_cells(s: &mut Session, v: &Value) -> CmdResult {
    match p::str(v, "shift").unwrap_or("left") {
        "left" => delete_left(s),
        "up" => delete_up(s),
        "row" => del_row(s, &Value::Null),
        "column" => del_col(s, &Value::Null),
        x => Err(CmdError::Params(format!("`shift` is left, up, row or column, not `{x}`"))),
    }
}

/// Whether `c` can change places: not part of a vertical merge, not wrapped in a content
/// control (whose ends could come apart).
fn movable(c: &Cell) -> bool {
    c.props.vmerge == VMerge::None && c.controls.is_empty()
}

/// A blank cell shaped like `c`: its formatting, one empty paragraph, nothing tracked.
fn blank_like(c: &Cell) -> Cell {
    let mut n = Cell::empty();
    n.props = c.props.clone();
    n.props.vmerge = VMerge::None;
    n.props.fmt_change = None;
    n
}

/// Each cell's width as the grid draws it: the grid columns it spans.
fn drawn_widths(t: &Table, r: usize) -> Vec<f32> {
    let Some(row) = t.rows.get(r) else { return Vec::new() };
    let mut g = row.first_grid();
    row.cells
        .iter()
        .map(|c| {
            let w = t.grid.get(g..g.saturating_add(c.span()).min(t.grid.len())).map_or(0.0, |x| x.iter().sum());
            g = g.saturating_add(c.span());
            w
        })
        .collect()
}

/// Whether the grid can be redrawn around rows whose cells moved sideways: one grid column per
/// column every row reaches, and every row starting and ending on the grid.
fn regriddable(t: &Table) -> bool {
    t.grid.len() == t.cols() && t.rows.iter().all(|r| r.first_grid() == 0 && r.grid_after() == 0)
}

/// The grid lines of `t`, from its left edge.
fn grid_lines(t: &Table) -> Vec<f32> {
    std::iter::once(0.0)
        .chain(t.grid.iter().scan(0.0, |x, w| {
            *x += w;
            Some(*x)
        }))
        .collect()
}

/// Shift cells right: before the selected cells of each row, as many new cells, as wide as
/// they are; the cells after them move right, so the row sticks out past the others.
fn insert_right(s: &mut Session) -> CmdResult {
    let pk = Picked::read(s)?;
    let story = s.sel.focus.story;
    let t = s.doc.table_mut(story, &pk.tp)?;
    if !regriddable(t) {
        return Err(CmdError::Failed("cells can't be shifted right in a table whose rows start or end inside its grid".into()));
    }
    let lines = grid_lines(t);
    let mut changed = Vec::with_capacity(pk.rows.len());
    for (&r, cs) in &pk.rows {
        let mut widths = drawn_widths(t, r);
        let Some(row) = t.rows.get_mut(r) else { continue };
        let at = cs.first().copied().unwrap_or(0);
        if row.cells.get(at..).is_some_and(|moving| !moving.iter().all(movable)) {
            return Err(failed_merged(s));
        }
        if row.cells.len().saturating_add(cs.len()) > MAX_COLS {
            return Err(CmdError::Failed(format!("a row can't have more than {MAX_COLS} cells")));
        }
        let new: Vec<Cell> = cs.iter().filter_map(|&c| row.cells.get(c)).map(blank_like).collect();
        let new_widths: Vec<f32> = cs.iter().filter_map(|&c| widths.get(c).copied()).collect();
        row.cells.splice(at..at, new);
        widths.splice(at..at, new_widths);
        changed.push((r, widths));
    }
    crate::cmd::table_resize::regrid(t, &lines, &changed)?;
    touch_cells(s, &pk.tp)?;
    fix_caret(s, &pk.tp, pk.top(), pk.first_cell())
}

/// Shift cells left: the selected cells go and the cells after them in each row move left,
/// keeping their widths, so the row gets shorter. A row left with no cells goes too, and the
/// table when that is every row.
fn delete_left(s: &mut Session) -> CmdResult {
    let pk = Picked::read(s)?;
    let story = s.sel.focus.story;
    let t = s.doc.table(story, &pk.tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
    if pk.rows.len() >= t.rows.len() && pk.rows.iter().all(|(&r, cs)| t.rows.get(r).is_some_and(|row| cs.len() >= row.cells.len())) {
        return delete_table_at(s, &pk.tp);
    }
    for (&r, cs) in &pk.rows {
        let from = cs.first().copied().unwrap_or(0);
        if t.rows.get(r).and_then(|row| row.cells.get(from..)).is_some_and(|after| !after.iter().all(movable)) {
            return Err(failed_merged(s));
        }
    }
    let prefixes: Vec<Vec<u32>> = pk.rows.iter().flat_map(|(&r, cs)| cs.iter().map(move |&c| cell_prefix(&pk.tp, r, c))).collect();
    let spans = text_spans(s, story, &prefixes);
    if let Some(done) = guard_delete(s, &spans)? {
        return Ok(done);
    }
    let t = s.doc.table_mut(story, &pk.tp)?;
    let regrid = regriddable(t);
    let lines = grid_lines(t);
    let mut changed = Vec::with_capacity(pk.rows.len());
    let mut emptied = Vec::new();
    for (&r, cs) in &pk.rows {
        let mut widths = drawn_widths(t, r);
        let Some(row) = t.rows.get_mut(r) else { continue };
        for &c in cs.iter().rev() {
            if c < row.cells.len() {
                row.cells.remove(c);
                widths.remove(c);
            }
        }
        if row.cells.is_empty() {
            emptied.push(r);
        } else {
            changed.push((r, widths));
        }
    }
    for &r in emptied.iter().rev() {
        t.delete_row(r);
    }
    if regrid {
        // Rows below a removed row moved up by one each.
        let moved: Vec<(usize, Vec<f32>)> = changed.into_iter().map(|(r, w)| (r - emptied.iter().filter(|&&e| e < r).count(), w)).collect();
        crate::cmd::table_resize::regrid(t, &lines, &moved)?;
    }
    touch_cells(s, &pk.tp)?;
    fix_caret(s, &pk.tp, pk.top(), pk.first_cell())
}

/// The cells of row `r` that exactly cover grid columns `g0..=g1`, if its cell borders fall on
/// both ends and every one of them can move.
fn segment(t: &Table, r: usize, g0: usize, g1: usize) -> Option<Range<usize>> {
    let row = t.rows.get(r)?;
    let first = t.cell_at_grid(r, g0)?;
    let last = t.cell_at_grid(r, g1)?;
    let ends = t.grid_col(r, first) == g0 && t.grid_col(r, last).saturating_add(row.cells.get(last)?.span()) == g1.saturating_add(1);
    (ends && row.cells.get(first..=last)?.iter().all(movable)).then_some(first..last.saturating_add(1))
}

/// The cells covering `g0..=g1` in every row from `top` down: the parts the vertical shifts move.
fn segments(t: &Table, top: usize, g0: usize, g1: usize) -> Option<Vec<Range<usize>>> {
    (top..t.rows.len()).map(|r| segment(t, r, g0, g1)).collect()
}

/// Shift cells down: the cells in the selected columns move down as many rows as are selected,
/// from the top selected row on, and the table gets as many rows at its bottom. Blank cells take
/// the moved cells' place.
fn insert_down(s: &mut Session) -> CmdResult {
    let pk = Picked::read(s)?;
    let (top, n) = (pk.top(), pk.height());
    let story = s.sel.focus.story;
    let t = s.doc.table_mut(story, &pk.tp)?;
    if t.rows.len().saturating_add(n) > MAX_ROWS {
        return Err(CmdError::Failed(format!("a table can't have more than {MAX_ROWS} rows")));
    }
    let Some(segs) = segments(t, top, pk.g0, pk.g1) else { return Err(failed_merged(s)) };
    let last = t.rows.len().saturating_sub(1);
    let mut parts: Vec<Vec<Cell>> = Vec::with_capacity(segs.len() + n);
    for (i, seg) in segs.iter().enumerate() {
        let cells = t.rows.get(top + i).and_then(|row| row.cells.get(seg.clone())).map(<[Cell]>::to_vec).unwrap_or_default();
        parts.push(cells);
    }
    // The blank cells the moved ones leave, shaped like the cells that were there.
    let blanks: Vec<Vec<Cell>> = parts.iter().take(n).map(|cs| cs.iter().map(blank_like).collect()).collect();
    for _ in 0..n {
        t.insert_row(t.rows.len(), last);
    }
    // Every row from the top selected one gets the part from `n` rows above it, or a blank one.
    let fill: Vec<Vec<Cell>> = blanks.into_iter().chain(parts).collect();
    for (i, cells) in fill.into_iter().enumerate() {
        let r = top + i;
        // Each row is looked at before it changes; the new rows are copies of the last one.
        let Some(seg) = segment(t, r, pk.g0, pk.g1) else { continue };
        if let Some(row) = t.rows.get_mut(r) {
            let seg = seg.start.min(row.cells.len())..seg.end.min(row.cells.len());
            row.cells.splice(seg, cells);
        }
    }
    touch_cells(s, &pk.tp)?;
    fix_caret(s, &pk.tp, top, segs.first().map_or(0, |x| x.start))
}

/// Shift cells up: the selected cells go and the cells below them in their columns move up as
/// many rows; blank cells fill the bottom of those columns. Rows stay.
fn delete_up(s: &mut Session) -> CmdResult {
    let pk = Picked::read(s)?;
    let (top, n) = (pk.top(), pk.height());
    let story = s.sel.focus.story;
    let t = s.doc.table(story, &pk.tp).ok_or_else(|| CmdError::Failed("no table".into()))?;
    let Some(segs) = segments(t, top, pk.g0, pk.g1) else { return Err(failed_merged(s)) };
    let prefixes: Vec<Vec<u32>> =
        segs.iter().take(n).enumerate().flat_map(|(i, seg)| seg.clone().map(move |c| (top + i, c))).map(|(r, c)| cell_prefix(&pk.tp, r, c)).collect();
    let spans = text_spans(s, story, &prefixes);
    if let Some(done) = guard_delete(s, &spans)? {
        return Ok(done);
    }
    let t = s.doc.table_mut(story, &pk.tp)?;
    let parts: Vec<Vec<Cell>> = segs
        .iter()
        .enumerate()
        .map(|(i, seg)| t.rows.get(top + i).and_then(|row| row.cells.get(seg.clone())).map(<[Cell]>::to_vec).unwrap_or_default())
        .collect();
    // The rows at the bottom get blank cells shaped like the ones they had.
    let keep = parts.len().saturating_sub(n);
    let blanks: Vec<Vec<Cell>> = parts.iter().skip(keep).map(|cs| cs.iter().map(blank_like).collect()).collect();
    let fill: Vec<Vec<Cell>> = parts.into_iter().skip(n).chain(blanks).collect();
    for (i, (cells, seg)) in fill.into_iter().zip(&segs).enumerate() {
        if let Some(row) = t.rows.get_mut(top + i) {
            let seg = seg.start.min(row.cells.len())..seg.end.min(row.cells.len());
            row.cells.splice(seg, cells);
        }
    }
    touch_cells(s, &pk.tp)?;
    fix_caret(s, &pk.tp, top, segs.first().map_or(0, |x| x.start))
}

/// The refusal for a shift through merged cells, also shown on the status line.
fn failed_merged(s: &mut Session) -> CmdError {
    s.status = MERGED.to_string();
    CmdError::Failed(MERGED.to_string())
}
