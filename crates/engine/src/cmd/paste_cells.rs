//! Paste with the caret inside a table cell (#6739). Word does not nest a copied table there (that
//! is its Paste as Nested Table, `edit.pasteNestedTable` here); observed black-box with synthetic
//! tables:
//!
//! - a copied table (or whole rows) whose caret is in the first column of a table of the same
//!   width: its rows are inserted above the caret's row;
//! - otherwise the copied cells overwrite the cells from the caret's cell rightwards and down,
//!   adding the rows and columns they need (new cells empty).
//!
//! Pasted cells take the destination's cell formatting (Word's default keeps the source's; not
//! done yet). What this does not cover falls back to the old behaviour, a nested table (see
//! [`paste_cells`]).

use wordcraft_doc::edit::Fragment;
use wordcraft_doc::props::VMerge;
use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Block, Cell, InlineObject, Path, Pos, Row, Table};

use crate::{CmdError, Selection, Session};

/// The fragment's table when it is one table and nothing else (empty paragraphs around it aside).
fn lone_table(frag: &Fragment) -> Option<&Table> {
    let mut table = None;
    for b in &frag.blocks {
        match b {
            Block::Table(t) if table.is_none() => table = Some(t),
            Block::Para(p) if p.is_empty() && p.objects.is_empty() => {}
            _ => return None,
        }
    }
    table
}

/// Whether every row has one cell of one column and no merge, offset or inline object, and every
/// cell has a paragraph: a table that can be laid over another cell by cell. Its width in columns.
fn plain_source(t: &Table) -> Option<usize> {
    let cols = t.rows.first()?.cells.len();
    let simple = |c: &Cell| {
        c.span() == 1
            && c.props.vmerge == VMerge::None
            && !c.blocks.is_empty()
            && c.controls.is_empty()
            && c.blocks.iter().all(|b| matches!(&**b, Block::Para(p) if p.objects.is_empty() && p.section.is_none()))
    };
    let ok = cols > 0
        && t.rows.iter().all(|r| {
            r.cells.len() == cols && r.first_grid() == 0 && r.grid_after() == 0 && r.props.exceptions.is_none() && r.cells.iter().all(simple)
        });
    ok.then_some(cols)
}

/// Whether the destination table's rows are a plain grid: one cell per grid column, no merges,
/// offsets or row exceptions.
fn plain_destination(t: &Table) -> bool {
    let cols = t.grid.len();
    cols > 0
        && t.rows.iter().all(|r| {
            r.cells.len() == cols
                && r.first_grid() == 0
                && r.grid_after() == 0
                && r.props.exceptions.is_none()
                && r.cells.iter().all(|c| c.span() == 1 && c.props.vmerge == VMerge::None)
        })
}

/// Whether a cell holds a content control, or a control's start or end in its text: overwriting
/// it could remove a locked control (the caret's own cell is checked by the caller's edit rules).
fn has_control(c: &Cell) -> bool {
    !c.controls.is_empty()
        || c.blocks.iter().any(
            |b| matches!(&**b, Block::Para(p) if p.objects.iter().any(|o| matches!(o, InlineObject::ControlStart { .. } | InlineObject::ControlEnd))),
        )
}

/// What a paste into a cell does with the copied table `src` at (`row0`, `col0`) of `dest`.
enum Plan {
    /// Insert the copy's rows above `row0`.
    InsertRows,
    /// Lay the copy's cells over the table from the cell.
    Overwrite,
}

/// How `src` goes into `dest` at (`row0`, `col0`), or `None` when it can't be laid over it (the
/// paste nests the table instead).
fn plan(dest: &Table, src: &Table, src_cols: usize, row0: usize, col0: usize) -> Option<Plan> {
    if !plain_destination(dest) || col0 >= dest.grid.len() || row0 >= dest.rows.len() {
        return None;
    }
    let (src_rows, dest_cols) = (src.rows.len(), dest.grid.len());
    // A one-column table is a layout wrapper more often than a grid: a wider copy nests in it.
    if dest_cols == 1 && src_cols > 1 {
        return None;
    }
    // Word inserts whole rows above the caret's row when the caret is in the first column and
    // the rows are as wide as the table; a one-column table can't tell rows from a cell.
    if col0 == 0 && src_cols == dest_cols && dest_cols > 1 {
        return (dest.rows.len().saturating_add(src_rows) <= MAX_ROWS).then_some(Plan::InsertRows);
    }
    if dest.rows.len().max(row0.saturating_add(src_rows)) > MAX_ROWS || dest_cols.max(col0.saturating_add(src_cols)) > MAX_COLS {
        return None;
    }
    // The cells that get overwritten must not hold controls.
    let hit = |r: usize| dest.rows.get(r).is_some_and(|row| row.cells.iter().skip(col0).take(src_cols).any(has_control));
    (!(row0..row0.saturating_add(src_rows).min(dest.rows.len())).any(hit)).then_some(Plan::Overwrite)
}

/// A copied table pasted with the caret in a table cell, as Word's default Paste does it.
/// `false` (nothing done; the caller pastes as before, nesting the table) when:
/// - the selection is not a caret in a cell, or the clipboard is not one table;
/// - Track Changes is on (the cell edits aren't tracked here);
/// - the copied table has merged cells, ragged rows, empty cells, pictures or other objects, or a
///   paragraph with a section break, or the destination table has merged cells or offset rows, or
///   a control in a cell that would be overwritten (they are not laid over);
/// - the destination has one column and the copy more (a layout wrapper), or the paste would pass
///   the limits on a table's rows or columns.
pub fn paste_cells(s: &mut Session, frag: &Fragment) -> Result<bool, CmdError> {
    if s.doc.settings.track_changes || !s.sel.is_collapsed() || s.cell_range().is_some() {
        return Ok(false);
    }
    let Some(src) = lone_table(frag) else { return Ok(false) };
    let Some(src_cols) = plain_source(src) else { return Ok(false) };
    let Some((tp, row0, col0)) = s.sel.focus.path.cell() else { return Ok(false) };
    let Some(dest) = s.doc.table(s.sel.focus.story, &tp) else { return Ok(false) };
    if plan(dest, src, src_cols, row0, col0).is_none() {
        return Ok(false);
    }
    // Protection: the edit rules may move the caret (to where typing goes) or refuse.
    super::controls::prepare_edit(s)?;
    let at = s.sel.focus.clone();
    let Some((tp, row0, col0)) = at.path.cell() else { return Ok(false) };
    let story = at.story;
    let Some(dest) = s.doc.table(story, &tp) else { return Ok(false) };
    let Some(plan) = plan(dest, src, src_cols, row0, col0) else { return Ok(false) };
    let t = s.doc.table_mut(story, &tp)?;
    let first = match plan {
        Plan::InsertRows => insert_source_rows(t, src, row0),
        Plan::Overwrite => overwrite_cells(t, src, row0, col0),
    };
    super::table::touch_cells(s, &tp)?;
    let mut path = tp.0.clone();
    path.extend([first.0 as u32, first.1 as u32, 0]);
    s.sel = Selection::caret(Pos { story, path: Path(path), off: 0 });
    Ok(true)
}

/// Insert the rows of `src` above row `at` of `t` (same column count), formatted as that row.
/// Returns the (row, cell) of the first pasted cell.
fn insert_source_rows(t: &mut Table, src: &Table, at: usize) -> (usize, usize) {
    let Some(template) = t.rows.get(at) else { return (at, 0) };
    let new_rows: Vec<Row> = src
        .rows
        .iter()
        .map(|sr| {
            let mut row = Row { props: template.props.clone(), ..Row::default() };
            row.props.header = false;
            row.cells = template
                .cells
                .iter()
                .zip(&sr.cells)
                .map(|(dc, sc)| {
                    let mut c = Cell::empty();
                    c.props = dc.props.clone();
                    c.blocks = sc.blocks.clone();
                    c
                })
                .collect();
            row
        })
        .collect();
    t.rows.splice(at..at, new_rows);
    (at, 0)
}

/// Lay the cells of `src` over those of `t` from (`row0`, column `col0`), adding the rows and
/// columns they need. Returns the (row, cell) of the first pasted cell.
fn overwrite_cells(t: &mut Table, src: &Table, row0: usize, col0: usize) -> (usize, usize) {
    let src_cols = src.rows.first().map_or(0, |r| r.cells.len());
    while t.grid.len() < col0.saturating_add(src_cols) && t.grid.len() < MAX_COLS {
        t.insert_col(t.grid.len());
    }
    while t.rows.len() < row0.saturating_add(src.rows.len()) && t.rows.len() < MAX_ROWS {
        let at = t.rows.len();
        t.insert_row(at, at.saturating_sub(1));
    }
    for (i, sr) in src.rows.iter().enumerate() {
        let Some(row) = t.rows.get_mut(row0 + i) else { break };
        for (j, sc) in sr.cells.iter().enumerate() {
            if let Some(dc) = row.cells.get_mut(col0 + j) {
                dc.blocks = sc.blocks.clone();
            }
        }
    }
    (row0, col0)
}
