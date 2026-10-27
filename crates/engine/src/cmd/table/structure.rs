use super::cells::{cell, selected_cells};
use super::clear::{cell_prefix, guard_delete, text_spans};
use super::layout::{fix_caret, sync_cell_widths};
use super::*;

/// Insert Above / Below: rows like the caret's, as many as the selection spans; the caret goes into the first.
pub(in crate::cmd) fn row(s: &mut Session, below: bool) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let cells = selected_cells(s);
    let n = cells.iter().map(|x| x.0).collect::<std::collections::BTreeSet<_>>().len().max(1);
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    for _ in 0..n {
        t.insert_row(if below { r + 1 } else { r }, r);
    }
    fix_caret(s, &tp, if below { r + 1 } else { r }, c)
}

/// Insert Left / Right: a column beside the caret's cell; the caret goes into it.
pub(in crate::cmd) fn col(s: &mut Session, right: bool) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    let g = t.grid_col(r, c);
    let span = t.rows.get(r).and_then(|x| x.cells.get(c)).map(|x| x.span()).unwrap_or(1);
    t.insert_col(if right { g + span } else { g });
    // Keep the table within its width: shrink all columns proportionally.
    let total: f32 = t.grid.iter().sum();
    let before: f32 = total - t.grid.get(if right { g + span } else { g }).copied().unwrap_or(0.0);
    if before > 0.0 {
        let k = before / total;
        t.grid.iter_mut().for_each(|w| *w *= k);
    }
    // The inserted cells carry the pre-scale column width, and the layout prefers a cell width
    // over the grid: sync them, or the shrink never reaches the layout or the saved `w:tcW`.
    sync_cell_widths(t);
    fix_caret(s, &tp, r, if right { c + 1 } else { c })
}

pub(super) fn del_row(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, _) = cell(s)?;
    let rows: std::collections::BTreeSet<usize> = selected_cells(s).into_iter().map(|x| x.0).collect();
    let story = s.sel.focus.story;
    if rows.len() >= s.doc.table(story, &tp).map_or(0, |t| t.rows.len()) {
        return del_table(s, &Value::Null);
    }
    let prefixes: Vec<Vec<u32>> = rows.iter().map(|rr| tp.0.iter().copied().chain([*rr as u32]).collect()).collect();
    let spans = text_spans(s, story, &prefixes);
    if let Some(done) = guard_delete(s, &spans)? {
        return Ok(done);
    }
    let t = s.doc.table_mut(story, &tp)?;
    for rr in rows.iter().rev() {
        t.delete_row(*rr);
    }
    fix_caret(s, &tp, r, 0)
}

pub(super) fn del_col(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let cells = selected_cells(s);
    let story = s.sel.focus.story;
    let Some(t) = s.doc.table(story, &tp) else { return Err(CmdError::Failed("no table".into())) };
    if t.grid.len() <= 1 {
        return del_table(s, &Value::Null);
    }
    // Every grid column a selected cell covers goes (the caret's alone without a selection).
    let mut cols = std::collections::BTreeSet::new();
    for (rr, cc) in cells {
        let g = t.grid_col(rr, cc);
        let span = t.rows.get(rr).and_then(|x| x.cells.get(cc)).map_or(1, wordcraft_doc::Cell::span);
        cols.extend(g..g.saturating_add(span).min(t.grid.len()));
    }
    if cols.is_empty() {
        cols.insert(t.grid_col(r, c));
    }
    if cols.len() >= t.grid.len() {
        return del_table(s, &Value::Null);
    }
    // The cells that go with those columns, and their text: those all of whose columns go
    // (a merged cell that also spans a kept column just narrows, keeping its text).
    let mut prefixes = Vec::new();
    for (rr, row) in t.rows.iter().enumerate() {
        for (cc, x) in row.cells.iter().enumerate() {
            let g = t.grid_col(rr, cc);
            let covered = g..g.saturating_add(x.span()).min(t.grid.len());
            if !covered.is_empty() && covered.clone().all(|k| cols.contains(&k)) {
                prefixes.push(cell_prefix(&tp, rr, cc));
            }
        }
    }
    let spans = text_spans(s, story, &prefixes);
    if let Some(done) = guard_delete(s, &spans)? {
        return Ok(done);
    }
    let t = s.doc.table_mut(story, &tp)?;
    for g in cols.iter().rev() {
        t.delete_col(*g);
    }
    // Rows left without cells are dropped; if that was every row, no table is left.
    if t.rows.is_empty() {
        return del_table(s, &Value::Null);
    }
    fix_caret(s, &tp, r, c.saturating_sub(1))
}

pub(super) fn del_table(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let story = s.sel.focus.story;
    let spans = text_spans(s, story, std::slice::from_ref(&tp.0));
    if let Some(done) = guard_delete(s, &spans)? {
        return Ok(done);
    }
    s.doc.remove_block(story, &tp)?;
    s.sel = Selection::caret(s.doc.clamp(&Pos { story, path: tp, off: 0 }));
    sel_result(s)
}

pub(super) fn merge(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let cells = selected_cells(s);
    if cells.len() < 2 {
        return Err(CmdError::Failed("select two or more cells to merge".into()));
    }
    let story = s.sel.focus.story;
    let (r0, r1) = (cells.iter().map(|x| x.0).min().unwrap_or(0), cells.iter().map(|x| x.0).max().unwrap_or(0));
    let t = s.doc.table_mut(story, &tp)?;
    let c0 = cells.iter().map(|x| x.1).min().unwrap_or(0);
    let c1 = cells.iter().map(|x| x.1).max().unwrap_or(0);
    let g0 = t.grid_col(r0, c0);
    let g1 = t.grid_col(r0, c1) + t.rows.get(r0).and_then(|r| r.cells.get(c1)).map(|c| c.span()).unwrap_or(1) - 1;
    t.merge(r0, r1, g0, g1);
    fix_caret(s, &tp, r0, c0)
}

/// The selected cells as Split Cells sees them: one entry per cell, a vertically merged cell
/// once (its top row), each as (top row, grid column).
pub fn split_targets(s: &Session) -> Vec<(usize, usize)> {
    let Ok((tp, _, _)) = cell(s) else { return Vec::new() };
    let Some(t) = s.doc.table(s.sel.focus.story, &tp) else { return Vec::new() };
    let mut v: Vec<(usize, usize)> = selected_cells(s)
        .into_iter()
        .filter(|&(r, c)| t.rows.get(r).is_some_and(|row| c < row.cells.len()))
        .map(|(r, c)| (t.vertical_extent(r, c).map_or(r, |(top, _)| top), t.grid_col(r, c)))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Split Cells: the caret's cell, or the selected cells (merged first unless `mergeFirst` is
/// false), into `columns` × `rows` cells.
pub(super) fn split(s: &mut Session, v: &Value) -> CmdResult {
    let cols = p::u64(v, "columns").unwrap_or(2).clamp(1, 63) as usize;
    let rows = p::u64(v, "rows").unwrap_or(1).clamp(1, 63) as usize;
    let mut cells = split_targets(s);
    let merge_first = cells.len() > 1 && p::bool(v, "mergeFirst").unwrap_or(true);
    if merge_first {
        // As in Word: the merged cell splits into at most the rows it covered.
        let mut sel_rows: Vec<usize> = selected_cells(s).iter().map(|&(r, _)| r).collect();
        sel_rows.sort_unstable();
        sel_rows.dedup();
        if rows > sel_rows.len() {
            return Err(CmdError::Params(format!("The number must be between 1 and {}.", sel_rows.len())));
        }
        merge(s, &json!({}))?;
        cells.clear();
    } else if cells.len() > 1 && rows > 1 {
        // Word greys out the row count when the cells split one by one.
        return Err(CmdError::Params("several cells split one by one keep their rows: `rows` must be 1".into()));
    }
    let (tp, r, c) = cell(s)?;
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    if cells.is_empty() {
        cells.push((t.vertical_extent(r, c).map_or(r, |(top, _)| top), t.grid_col(r, c)));
    }
    // As in Word: a merged cell splits into 1 to its height rows; it never adds rows.
    if let [(top, g)] = cells[..]
        && let Some((_, k)) = t.cell_at_grid(top, g).and_then(|ci| t.vertical_extent(top, ci))
        && k > 1
        && rows > k
    {
        return Err(CmdError::Params(format!("The number must be between 1 and {k}.")));
    }
    // Rightmost grid column first, bottom up: a split only adds grid columns inside the cell it
    // splits, so the cells still to split keep their grid places. Each is found again by its grid
    // column when its turn comes.
    let first = cells.first().copied();
    cells.sort_unstable_by_key(|&(top, g)| std::cmp::Reverse((g, top)));
    for &(top, g) in &cells {
        let Some(ci) = t.cell_at_grid(top, g) else { continue };
        // 1 × 1 on a cell that was already merged down changes nothing in Word, at any height; a
        // cell just merged by Merge cells before split stays one cell.
        let height = t.vertical_extent(top, ci).map_or(1, |(_, k)| k);
        if !merge_first && cols == 1 && rows == 1 && height > 1 {
            continue;
        }
        if height > 1 && rows > 1 && !height.is_multiple_of(rows) {
            return Err(CmdError::Params(format!("The number must be a divisor of {height}.")));
        }
        if !t.split_cell_grid(top, ci, cols, rows) {
            return Err(CmdError::Failed(format!("can't split into {rows} rows: the table would get too many rows")));
        }
    }
    // The caret goes to the first part of the first cell (the top of a merged cell).
    let (top, g) = first.unwrap_or((r, 0));
    let c = t.cell_at_grid(top, g).unwrap_or(c);
    fix_caret(s, &tp, top, c)
}

pub(super) fn split_table(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, _) = cell(s)?;
    if r == 0 {
        return Err(CmdError::Failed("can't split at the first row".into()));
    }
    let story = s.sel.focus.story;
    let t = s.doc.table_mut(story, &tp)?;
    let tail = t.split_off(r);
    let after = tp.with_last(tp.last() + 1);
    s.doc.insert_block(story, &after, Block::Para(Paragraph::new()))?;
    s.doc.insert_block(story, &after.with_last(after.last() + 1), Block::Table(tail))?;
    s.sel = Selection::caret(Pos { story, path: after, off: 0 });
    sel_result(s)
}
