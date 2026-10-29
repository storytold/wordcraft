use super::cells::{cell, cell_at, grid_cols, row_ends, selected_cells};
use super::{CmdError, CmdResult, Paragraph, Path, Pos, Selection, Session, StoryRef, Value, sel_result};

/// Ctrl+A in a table, as in Word: the cell, then its row, then the table. A nested table that
/// is already fully selected grows to the cell around it. Returns `None` when the next step is
/// the whole story (not in a table, or the top-level table is already selected).
pub(crate) fn grow_select_all(s: &mut Session) -> Option<CmdResult> {
    let story = s.sel.focus.story;
    if let Some(cr) = s.cell_range().filter(|cr| cr.story == story).cloned() {
        let tp = cr.table.clone();
        let (last_row, last_col, one_cell) = {
            let t = s.doc.table(story, &tp)?;
            let last_row = t.rows.len().saturating_sub(1);
            let last_col = t.cols().saturating_sub(1);
            let one_cell = cr.rows.0 == cr.rows.1
                && t.rows.get(cr.rows.0).is_some_and(|row| (0..row.cells.len()).any(|c| grid_cols(t, cr.rows.0, c) == cr.cols));
            (last_row, last_col, one_cell)
        };
        if cr.rows == (0, last_row) && cr.cols == (0, last_col) {
            return tp.cell().map(|(otp, or, oc)| select_cells(s, &otp, Some((or, Some(oc)))));
        }
        if cr.rows.0 == cr.rows.1 && cr.cols == (0, last_col) {
            return Some(select_cells(s, &tp, None));
        }
        if one_cell {
            return Some(select_cells(s, &tp, Some((cr.rows.0, None))));
        }
        return Some(select_cells(s, &tp, None));
    }
    let (a, b) = s.sel.ordered();
    let (tp, r, c) = a.path.cell()?;
    if b.path.cell() != Some((tp.clone(), r, c)) {
        return None;
    }
    Some(select_cells(s, &tp, Some((r, Some(c)))))
}

pub(super) fn select_cells(s: &mut Session, tp: &Path, which: Option<(usize, Option<usize>)>) -> CmdResult {
    let story = s.sel.focus.story;
    let Some(t) = s.doc.table(story, tp) else { return sel_result(s) };
    let (last_row, last_col) = (t.rows.len().saturating_sub(1), t.cols().saturating_sub(1));
    let (rows, cols) = match which {
        None => ((0, last_row), (0, last_col)),
        Some((r, None)) => ((r, r), (0, last_col)),
        Some((r, Some(c))) => ((r, r), grid_cols(t, r, c)),
    };
    select_range(s, story, tp, rows, cols)
}

/// Table Layout › Table › Select › Select Column: the grid columns of the selected cells, every row.
pub(super) fn select_column(s: &mut Session, _: &Value) -> CmdResult {
    let (tp, r, c) = cell(s)?;
    let story = s.sel.focus.story;
    let t = s.doc.table(story, &tp).ok_or_else(|| CmdError::Disabled("the cursor isn't in a table".into()))?;
    let (mut g0, mut g1) = grid_cols(t, r, c);
    for (r, c) in selected_cells(s) {
        let (a, b) = grid_cols(t, r, c);
        (g0, g1) = (g0.min(a), g1.max(b));
    }
    let rows = (0, t.rows.len().saturating_sub(1));
    select_range(s, story, &tp, rows, (g0, g1))
}

/// Select whole cells of table `tp`: rows `rows`, and of them every cell reaching into grid
/// columns `cols` (both inclusive, either order). The selection runs from the first cell's start
/// to the last cell's end; a block of the cells' paragraphs (`CellRange`) says which cells they
/// are, so the app shades them and, when they aren't one run of the text (part of a column),
/// edits and formatting leave the cells between the corners alone.
pub(crate) fn select_range(s: &mut Session, story: StoryRef, tp: &Path, rows: (usize, usize), cols: (usize, usize)) -> CmdResult {
    let t = s.doc.table(story, tp).ok_or_else(|| CmdError::Failed("there's no table there".into()))?;
    let (last_row, last_col) = (t.rows.len().saturating_sub(1), t.cols().saturating_sub(1));
    let rows = (rows.0.min(rows.1).min(last_row), rows.0.max(rows.1).min(last_row));
    let cols = (cols.0.min(cols.1).min(last_col), cols.0.max(cols.1).min(last_col));
    let stream = rows.0 == rows.1 || (cols.0 == 0 && cols.1 == last_col);
    let range = crate::CellRange { story, table: tp.clone(), rows, cols, stream };
    let picked: std::collections::BTreeSet<(usize, usize)> = range.cells(t).into_iter().collect();
    // Every paragraph of the picked cells (nested tables' too), in document order.
    let depth = tp.0.len();
    let in_picked = |q: &Path| match (q.0.get(depth), q.0.get(depth + 1)) {
        (Some(r), Some(c)) => q.0.len() > depth + 2 && q.0.starts_with(&tp.0) && picked.contains(&(*r as usize, *c as usize)),
        _ => false,
    };
    let segments: Vec<(Pos, Pos)> = s
        .doc
        .para_paths(story)
        .into_iter()
        .filter(|q| in_picked(q))
        .map(|q| {
            let len = s.doc.para(story, &q).map_or(0, Paragraph::len);
            (Pos { story, path: q.clone(), off: 0 }, Pos { story, path: q, off: len })
        })
        .collect();
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return Err(CmdError::Failed("there are no cells to select there".into()));
    };
    // The end corner is in the last cell's own last paragraph (not a nested table's), so it
    // names the cell.
    let focus = segments.iter().rev().find(|(_, b)| b.path.0.len() == depth + 3).map_or(&last.1, |(_, b)| b).clone();
    let sel = Selection { anchor: first.0.clone(), focus };
    s.sel = sel.clone();
    s.column = Some(crate::ColumnBlock { sel, anchor_x: 0.0, focus_x: 0.0, segments, cells: Some(range), pieces: None });
    s.column_mode = false;
    s.widened = None;
    s.goal_x = None;
    s.pending = None;
    sel_result(s)
}

/// Extend the selection from its anchor to `to` the way Word does around tables: from one cell
/// into another of the same table it takes whole cells, the rectangle between them; between a
/// table's cell and text outside the table, whole rows, from the cell's row out to the table's
/// edge facing the other end. Where the selection really runs is kept (`Session::sel_ends`), so
/// extending it again goes on from there.
pub(crate) fn extend_selection(s: &mut Session, to: Pos) {
    let ends = Selection { anchor: s.sel.anchor.clone(), focus: to };
    // Shift+Up and Shift+Down keep their goal column.
    let goal_x = s.goal_x;
    s.sel = ends.clone();
    s.column = None;
    widen(s, &ends);
    s.widened = if s.sel == ends { None } else { Some(crate::session::Widened { sel: s.sel.clone(), ends }) };
    s.goal_x = goal_x;
}

/// Shift+Right (Shift+Left in right-to-left text) out of the cell the selection is in takes that
/// cell whole first, as Word does (its end-of-cell mark); the next press goes on into the next
/// cell. `ends` is where the selection ran before the step, `to` where the step went, `cells` the
/// whole cells selected before it. Whether it took the cell.
pub(crate) fn select_cell_first(s: &mut Session, ends: &Selection, to: &Pos, cells: Option<&crate::CellRange>) -> bool {
    let (a, f) = (&ends.anchor, &ends.focus);
    if a.story != f.story || to.story != f.story || to <= f {
        return false;
    }
    // The innermost cell both ends are in.
    let mut cell = None;
    let mut i = 0;
    while let (Some(x), Some(y)) = (cell_at(&a.path.0, i), cell_at(&f.path.0, i)) {
        if x != y {
            break;
        }
        cell = Some(x);
        i += 3;
    }
    let Some((tp, r, c)) = cell else { return false };
    // Still in the cell, or that cell alone is already selected: nothing to take first.
    if f.path.0.get(..tp.0.len() + 2).is_some_and(|prefix| to.path.0.starts_with(prefix)) {
        return false;
    }
    let Some(cols) = s.doc.table(f.story, &tp).map(|t| grid_cols(t, r, c)) else { return false };
    if cells.is_some_and(|cr| cr.story == f.story && cr.table == tp && cr.rows == (r, r) && cr.cols == cols) {
        return false;
    }
    select_range(s, f.story, &tp, (r, r), cols).is_ok()
}

/// Widen the selection between `ends` (already `s.sel`) by Word's table rule (see
/// `extend_selection`).
fn widen(s: &mut Session, ends: &Selection) {
    let (a, f) = (&ends.anchor, &ends.focus);
    if a.story != f.story {
        return;
    }
    // Down through the tables both ends are in, one nesting level (three indexes) at a time.
    let mut i = 0;
    while let (Some(ab), Some(fb)) = (a.path.0.get(i), f.path.0.get(i)) {
        match (cell_at(&a.path.0, i), cell_at(&f.path.0, i)) {
            // The same cell: look inside it.
            (Some(x), Some(y)) if ab == fb && x == y => i += 3,
            // Two cells of one table: the rectangle between them.
            (Some((tp, ra, ca)), Some((_, rf, cf))) if ab == fb => {
                let cols = s.doc.table(a.story, &tp).map(|t| {
                    let ((g0, g1), (h0, h1)) = (grid_cols(t, ra, ca), grid_cols(t, rf, cf));
                    (g0.min(h0), g1.max(h1))
                });
                if let Some(cols) = cols {
                    // A table without paragraphs keeps the plain selection.
                    let _ = select_range(s, a.story, &tp, (ra, rf), cols);
                }
                return;
            }
            // Different blocks: an end in a table takes its whole row, out to the table's edge
            // facing the other end.
            (x, y) if ab != fb => {
                let forward = ab < fb;
                if let Some((tp, r, _)) = x
                    && let Some((start, end)) = row_ends(&s.doc, a.story, &tp, r)
                {
                    s.sel.anchor = if forward { start } else { end };
                }
                if let Some((tp, r, _)) = y
                    && let Some((start, end)) = row_ends(&s.doc, f.story, &tp, r)
                {
                    s.sel.focus = if forward { end } else { start };
                }
                return;
            }
            // One paragraph.
            _ => return,
        }
    }
}
