use super::structure::{del_col, del_row};
use super::*;

/// Backspace with whole cells selected (#1844): whole rows go like Delete Rows (every row: the
/// table, like Delete Table), whole columns like Delete Columns, with their undo and Track
/// Changes. `None` for any other block of cells, and for a single cell of a table with more
/// (Backspace then clears them).
pub(crate) fn backspace_cells(s: &mut Session) -> Option<CmdResult> {
    let cr = s.cell_range()?.clone();
    let (tp, _, _) = s.sel.focus.path.cell()?;
    if cr.story != s.sel.focus.story || cr.table != tp {
        return None;
    }
    let t = s.doc.table(cr.story, &tp)?;
    // One cell of several is just that cell, also in a one-row or one-column table (Word's cell
    // selection doesn't take the end-of-row mark): Backspace clears it.
    let cells: usize = t.rows.iter().map(|row| row.cells.len()).sum();
    if cells > 1 && cr.cells(t).len() <= 1 {
        return None;
    }
    let (last_row, last_col) = (t.rows.len().saturating_sub(1), t.cols().saturating_sub(1));
    let r = if cr.cols == (0, last_col) {
        del_row(s, &Value::Null)
    } else if cr.rows == (0, last_row) {
        del_col(s, &Value::Null)
    } else {
        return None;
    };
    s.column = None;
    Some(r)
}

/// Delete with whole cells selected (#2802): each cell is emptied to one empty paragraph, as in
/// Word, nested tables included. That paragraph keeps the formatting of the cell's last one (its
/// mark is the end-of-cell mark). `None` without whole cells, and with Track Changes on (the text
/// is marked deleted as for any selection).
pub(crate) fn clear_cells(s: &mut Session) -> Option<CmdResult> {
    if s.doc.settings.track_changes {
        return None;
    }
    let cr = s.cell_range()?.clone();
    let t = s.doc.table(cr.story, &cr.table)?;
    let prefixes: Vec<Vec<u32>> = cr.cells(t).into_iter().map(|(r, c)| cell_prefix(&cr.table, r, c)).collect();
    let spans = text_spans(s, cr.story, &prefixes);
    Some(clear_spans(s, cr.table.0.len().saturating_add(2), &spans))
}

/// Empty each span (one cell's text, from its first paragraph to its last; the cell's path is the
/// first `depth` steps of its paths) to one paragraph with the last paragraph's formatting, bottom
/// up so earlier spans stay valid; the caret goes to the start of the first cell.
fn clear_spans(s: &mut Session, depth: usize, spans: &[(Pos, Pos)]) -> CmdResult {
    // A cell's first block (its first paragraph may have been in a nested table that goes).
    let first_block = |p: &Pos| {
        let mut v = p.path.0.get(..depth).unwrap_or_default().to_vec();
        v.push(0);
        Pos { story: p.story, path: Path(v), off: 0 }
    };
    for (a, b) in spans {
        crate::cmd::controls::check_delete(s, a, b)?;
    }
    for (a, b) in spans.iter().rev() {
        let Some(last) = s.doc.para(b.story, &b.path).map(|p| (p.props.clone(), p.mark.clone())) else { continue };
        s.doc.delete_range(a, b)?;
        // Now the cell's only block, unless it holds a table nested deeper than the paragraph
        // walk (`delete_range` leaves those).
        let first = first_block(a);
        if s.doc.container(first.story, &first.path).is_some_and(|bl| bl.len() == 1)
            && let Ok(p) = s.doc.para_mut(first.story, &first.path)
        {
            (p.props, p.mark) = last;
            p.touch();
        }
    }
    let at = spans.first().map(|(a, _)| s.doc.clamp(&first_block(a))).unwrap_or_else(|| s.sel.focus.clone());
    s.sel = Selection::caret(at);
    s.column = None;
    s.column_mode = false;
    s.goal_x = None;
    s.pending = None;
    crate::cmd::controls::restore_placeholder(s)?;
    sel_result(s)
}

/// The path prefix of cell (`r`, `c`) of the table at `tp`.
pub(super) fn cell_prefix(tp: &Path, r: usize, c: usize) -> Vec<u32> {
    tp.0.iter().copied().chain([r as u32, c as u32]).collect()
}

/// The text under each path prefix (a cell, a row or a whole table): from its first paragraph's
/// start to its last paragraph's end. Prefixes without paragraphs are left out.
pub(super) fn text_spans(s: &Session, story: StoryRef, prefixes: &[Vec<u32>]) -> Vec<(Pos, Pos)> {
    let paths = s.doc.para_paths(story);
    prefixes
        .iter()
        .filter_map(|pre| {
            let mut inside = paths.iter().filter(|p| p.0.starts_with(pre));
            let first = inside.next()?.clone();
            let last = inside.next_back().cloned().unwrap_or_else(|| first.clone());
            let end = s.doc.para(story, &last).map_or(0, Paragraph::len);
            Some((Pos { story, path: first, off: 0 }, Pos { story, path: last, off: end }))
        })
        .collect()
}

/// Before a table deletion removes the text `spans`: refuse when that deletes a content control
/// locked against deletion or edits content locked against editing (#1329). With Track Changes
/// on, text not yet deleted is marked deleted instead and the structure stays (like Word deleting
/// a table with tracking on; the model has no tracked row or column deletion), returning the
/// result. With no text at all (an empty table, or one whose deletion was accepted) there is
/// nothing for Reject All to restore and the structure goes as without tracking; with only text
/// already marked deleted, nothing changes (removing it would lose what Reject All restores).
pub(super) fn guard_delete(s: &mut Session, spans: &[(Pos, Pos)]) -> Result<Option<Value>, CmdError> {
    for (a, b) in spans {
        crate::cmd::controls::check_delete(s, a, b)?;
    }
    if !s.doc.settings.track_changes {
        return Ok(None);
    }
    if !spans.iter().any(|(a, b)| has_text(s, a, b, true)) {
        if spans.iter().any(|(a, b)| has_text(s, a, b, false)) {
            let msg = "The table's text is already marked deleted; accept or reject the change first.";
            s.status = msg.to_string();
            return Err(CmdError::Failed(msg.to_string()));
        }
        return Ok(None);
    }
    let rid = crate::cmd::new_revision(s, RevisionKind::Delete);
    for (a, b) in spans.iter().rev() {
        crate::cmd::track_delete_as(s, a, b, rid)?;
    }
    if let Some((a, _)) = spans.first() {
        s.sel = Selection::caret(s.doc.clamp(a));
    }
    sel_result(s).map(Some)
}

/// Whether `a..b` holds text: any text, or (`live`) text that isn't a tracked deletion.
fn has_text(s: &Session, a: &Pos, b: &Pos, live: bool) -> bool {
    s.doc.paths_between(a, b).iter().any(|path| {
        let Some(p) = s.doc.para(a.story, path) else { return false };
        let from = if *path == a.path { a.off } else { 0 };
        let to = if *path == b.path { b.off } else { p.len() };
        p.run_ranges().any(|(r, c)| (!live || c.del.is_none()) && r.end > from && r.start < to && !r.is_empty())
    })
}
