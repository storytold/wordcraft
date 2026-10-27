use super::layout::touch_cells;
use super::*;

pub(super) fn cell(s: &Session) -> Result<(Path, usize, usize), CmdError> {
    s.sel.focus.path.cell().ok_or_else(|| CmdError::Disabled("the cursor isn't in a table".into()))
}

pub(super) fn with_table(s: &mut Session, f: impl Fn(&mut Table)) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    f(t);
    touch_cells(s, &tp)?;
    sel_result(s)
}

/// Selected cells: (row, cell) pairs covered by the selection (at least the caret's cell).
pub fn selected_cells(s: &Session) -> Vec<(usize, usize)> {
    // Whole cells selected (a column, say): just those, by grid column, so merged cells count.
    if let Some(range) = s.cell_range()
        && s.sel.focus.path.cell().is_some_and(|(tp, _, _)| tp == range.table)
        && let Some(t) = s.doc.table(range.story, &range.table)
    {
        return range.cells(t);
    }
    let (a, b) = s.sel.ordered();
    let (Some((ta, ra, ca)), Some((tb, rb, cb))) = (a.path.cell(), b.path.cell()) else { return Vec::new() };
    if ta != tb {
        return vec![(rb, cb)];
    }
    let (r0, r1) = (ra.min(rb), ra.max(rb));
    let (c0, c1) = (ca.min(cb), ca.max(cb));
    let mut v = Vec::new();
    for r in r0..=r1 {
        for c in c0..=c1 {
            v.push((r, c));
        }
    }
    v
}

pub(super) fn with_cells(s: &mut Session, f: impl Fn(&mut wordcraft_doc::Cell)) -> CmdResult {
    let (tp, _, _) = cell(s)?;
    let cells = selected_cells(s);
    let t = s.doc.table_mut(s.sel.focus.story, &tp)?;
    for (r, c) in cells {
        if let Some(cl) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) {
            f(cl);
        }
    }
    touch_cells(s, &tp)?;
    sel_result(s)
}

pub(crate) fn cell_text(c: &wordcraft_doc::Cell) -> String {
    c.blocks.iter().filter_map(|b| b.as_para().map(|p| p.plain_text())).collect::<Vec<_>>().join("\n")
}

/// The language of a cell's text: that of its first paragraph with text.
pub(super) fn cell_lang(doc: &wordcraft_doc::Document, c: &wordcraft_doc::Cell) -> Option<String> {
    c.blocks.iter().filter_map(|b| b.as_para()).find(|p| !p.plain_text().trim().is_empty()).and_then(|p| crate::cmd::para_lang(doc, p))
}

/// The grid columns cell `c` of row `r` covers (first, last).
pub(in crate::cmd) fn grid_cols(t: &Table, r: usize, c: usize) -> (usize, usize) {
    let g = t.grid_col(r, c);
    let span = t.rows.get(r).and_then(|row| row.cells.get(c)).map_or(1, wordcraft_doc::Cell::span);
    (g, g + span - 1)
}

/// The cell of the table at index `i` of paragraph path `p`, if the paragraph is in one there:
/// (the table's path, row, cell).
pub(super) fn cell_at(p: &[u32], i: usize) -> Option<(Path, usize, usize)> {
    match (p.get(..=i), p.get(i + 1), p.get(i + 2)) {
        (Some(t), Some(r), Some(c)) if p.len() > i + 3 => Some((Path(t.to_vec()), *r as usize, *c as usize)),
        _ => None,
    }
}

/// Where row `r` of table `tp` starts and ends: its first cell's start and its last cell's end.
pub(crate) fn row_ends(doc: &Document, story: StoryRef, tp: &Path, r: usize) -> Option<(Pos, Pos)> {
    let mut row = tp.0.clone();
    row.push(u32::try_from(r).ok()?);
    let mut paths = doc.para_paths(story).into_iter().filter(|q| q.0.starts_with(&row));
    let first = paths.next()?;
    let last = paths.next_back().unwrap_or_else(|| first.clone());
    let len = doc.para(story, &last).map_or(0, Paragraph::len);
    Some((Pos { story, path: first, off: 0 }, Pos { story, path: last, off: len }))
}
