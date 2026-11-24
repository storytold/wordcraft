//! Paste with the caret inside a table cell (#6739). Word does not nest a copied table there (that
//! is its Paste as Nested Table, `edit.pasteNestedTable` here); observed black-box with synthetic
//! tables:
//!
//! - a copied table (or whole rows) whose caret is in the first column of a table of the same
//!   width: its rows are inserted above the caret's row;
//! - otherwise the copied cells overwrite the cells from the caret's cell rightwards and down,
//!   adding the rows and columns they need (new cells empty).
//!
//! With a cell selection, Paste overwrites from its top-left, including when the copy is wider
//! than the selection.
//!
//! Pasted cells take the destination's cell formatting (Word's default keeps the source's; not
//! done yet). What this does not cover falls back to the old behaviour, a nested table (see
//! [`paste_cells`]).

use wordcraft_doc::edit::Fragment;
use wordcraft_doc::props::VMerge;
use wordcraft_doc::table::{MAX_COLS, MAX_ROWS};
use wordcraft_doc::{Block, Cell, InlineObject, Path, Pos, Row, StoryRef, Table};

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

/// The width of a rectangular table that can be pasted cell by cell. Merges, offsets,
/// objects and content control wrappers require preserving the table structure.
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
        && t.controls.is_empty()
        && t.rows.iter().all(|r| {
            r.cells.len() == cols
                && r.first_grid() == 0
                && r.grid_after() == 0
                && r.props.exceptions.is_none()
                && r.controls.is_empty()
                && r.cells.iter().all(simple)
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

/// How deep [`has_control`] looks into tables nested in a cell; a deeper cell counts as holding a
/// control, so a hostile document can't make the walk recurse without bound.
const MAX_CONTROL_DEPTH: usize = 16;

/// Whether a cell holds a content control, or a control's start or end in its text, at any depth
/// of nested tables, or a text box anchor (whose story may hold one): overwriting the cell
/// replaces all of its blocks, which could remove a locked control (the caret's own cell is
/// checked by the caller's edit rules).
fn has_control(c: &Cell) -> bool {
    cell_has_control(c, 0)
}

fn cell_has_control(c: &Cell, depth: usize) -> bool {
    if !c.controls.is_empty() || depth > MAX_CONTROL_DEPTH {
        return true;
    }
    c.blocks.iter().any(|b| match &**b {
        // A text box (or a group holding one) anchored here has its own story, which may hold a locked
        // control; it is not walked, so a cell with one counts as holding a control.
        Block::Para(p) => {
            p.objects.iter().any(|o| matches!(o, InlineObject::ControlStart { .. } | InlineObject::ControlEnd) || !o.text_boxes().is_empty())
        }
        Block::Table(t) => {
            !t.controls.is_empty() || t.rows.iter().any(|r| !r.controls.is_empty() || r.cells.iter().any(|c| cell_has_control(c, depth + 1)))
        }
    })
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
fn plan(dest: &Table, src: &Table, src_cols: usize, row0: usize, col0: usize, allow_row_insertion: bool) -> Option<Plan> {
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
    if allow_row_insertion && col0 == 0 && src_cols == dest_cols && dest_cols > 1 {
        return (dest.rows.len().saturating_add(src_rows) <= MAX_ROWS).then_some(Plan::InsertRows);
    }
    if dest.rows.len().max(row0.saturating_add(src_rows)) > MAX_ROWS || dest_cols.max(col0.saturating_add(src_cols)) > MAX_COLS {
        return None;
    }
    // A wrapper may open before the first overwritten row or cell and close after the last.
    // Use ordinary paste for wrapped grids so an overwrite cannot bypass a structural control.
    if !dest.controls.is_empty() || dest.rows.iter().any(|row| !row.controls.is_empty() || row.cells.iter().any(|cell| !cell.controls.is_empty())) {
        return None;
    }
    let hit = |r: usize| dest.rows.get(r).is_some_and(|row| row.cells.iter().skip(col0).take(src_cols).any(has_control));
    (!(row0..row0.saturating_add(src_rows).min(dest.rows.len())).any(hit)).then_some(Plan::Overwrite)
}

/// Overwrite cells from the top-left of a selection while it still identifies a rectangle.
pub(super) fn paste_over_selection(s: &mut Session, frag: &Fragment) -> Result<bool, CmdError> {
    let Some(range) = s.cell_range() else { return Ok(false) };
    if s.doc.settings.track_changes || s.doc.control_at(&s.sel.ordered().0).is_some() {
        return Ok(false);
    }
    let Some(src) = lone_table(frag) else { return Ok(false) };
    let Some(src_cols) = plain_source(src) else { return Ok(false) };
    let (row, col) = (range.rows.0, range.cols.0);
    let story = range.story;
    let table = range.table.clone();
    let Some(dest) = s.doc.table(story, &table) else { return Ok(false) };
    let Some(edit) = plan(dest, src, src_cols, row, col, false) else { return Ok(false) };
    apply_plan(s, src, story, &table, row, col, edit)?;
    s.column = None;
    s.goal_x = None;
    s.pending = None;
    Ok(true)
}

/// A copied table pasted with the caret in a table cell, as Word's default Paste does it.
/// `false` (nothing done; the caller pastes as before, nesting the table) when:
/// - the selection is not a caret in a cell, or the clipboard is not one table;
/// - Track Changes is on (the cell edits aren't tracked here);
/// - the copied table has merged cells, ragged rows, empty cells, pictures or other objects,
///   a paragraph with a section break, or a control wrapping the table, a row or a cell;
/// - the destination table has merged cells or offset rows, or an overwrite would affect a
///   destination with structural wrappers or a cell containing a control in its blocks;
/// - the caret is in a content control that shows its placeholder (the ordinary paste replaces
///   the placeholder);
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
    if plan(dest, src, src_cols, row0, col0, true).is_none() {
        return Ok(false);
    }
    // A caret in a control that shows its placeholder: the edit rules would select the
    // placeholder to replace it, which the row insertion would not do. The ordinary paste does.
    // This also covers a caret at the edge of such a control (drop-down, check box): rows used to
    // be inserted there, now the table nests. Overwrite already declined for cells with controls.
    if s.doc.control_at(&s.sel.focus).is_some_and(|r| r.control.showing_placeholder) {
        return Ok(false);
    }
    // Pasting ends Backspace's carried formatting, as in the ordinary paste path.
    s.forget_carried();
    // Protection: the edit rules may move the caret (to where typing goes) or refuse.
    if super::controls::prepare_edit(s)?.is_some() {
        // Normalizing the caret can enter a control and select its placeholder.
        // Let ordinary paste replace that selection.
        return Ok(false);
    }
    let at = s.sel.focus.clone();
    let Some((tp, row0, col0)) = at.path.cell() else { return Ok(false) };
    let story = at.story;
    let Some(dest) = s.doc.table(story, &tp) else { return Ok(false) };
    let Some(edit) = plan(dest, src, src_cols, row0, col0, true) else { return Ok(false) };
    apply_plan(s, src, story, &tp, row0, col0, edit)?;
    Ok(true)
}

fn apply_plan(s: &mut Session, src: &Table, story: StoryRef, tp: &Path, row0: usize, col0: usize, plan: Plan) -> Result<(), CmdError> {
    let t = s.doc.table_mut(story, tp)?;
    let first = match plan {
        Plan::InsertRows => insert_source_rows(t, src, row0),
        Plan::Overwrite => overwrite_cells(t, src, row0, col0),
    };
    super::table::touch_cells(s, tp)?;
    let mut path = tp.0.clone();
    path.extend([first.0 as u32, first.1 as u32, 0]);
    s.sel = Selection::caret(Pos { story, path: Path(path), off: 0 });
    Ok(())
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
