use super::{CmdError, CmdResult, InlineObject, Paragraph, Pos, Selection, Session, StoryRef, Value, fmt_revisions, json, p, pos_json, sel_result};

/// Accept or reject the tracked formatting and text of `from..to` in one paragraph (not its mark).
fn resolve_runs(para: &mut wordcraft_doc::Paragraph, from: usize, to: usize, props: bool, accept: bool) -> Result<(), CmdError> {
    if props && (para.props.fmt_change.is_some() || para.props.num_change.is_some()) {
        fmt_revisions::resolve_para(&mut para.props, accept);
        para.touch();
    }
    let ranges: Vec<(usize, usize, bool, bool, bool)> = para
        .run_ranges()
        .filter(|(r, _)| r.end > from && r.start < to)
        .map(|(r, c)| (r.start.max(from), r.end.min(to), c.ins.is_some(), c.del.is_some(), c.fmt_change.is_some()))
        .collect();
    for (a, b, ins, del, fmt) in ranges.into_iter().rev() {
        if (del && accept) || (ins && !accept) {
            para.delete(a, b)?;
        } else if ins || del || fmt {
            para.format(a, b, &|c| {
                c.ins = None;
                c.del = None;
                fmt_revisions::resolve_char(c, accept);
            })?;
        }
    }
    Ok(())
}

/// Accept or reject a revision range in one paragraph; `mark` includes its paragraph mark (and
/// the section break on it), `props` the paragraph's own formatting change.
fn resolve_para(
    s: &mut Session,
    story: StoryRef,
    path: &wordcraft_doc::Path,
    (from, to): (usize, usize),
    (mark, props): (bool, bool),
    accept: bool,
) -> Result<(), CmdError> {
    let para = s.doc.para_mut(story, path)?;
    resolve_runs(para, from, to, props, accept)?;
    if !mark {
        return Ok(());
    }
    if para.mark.fmt_change.is_some() || para.section.as_ref().is_some_and(|x| x.fmt_change.is_some()) {
        fmt_revisions::resolve_char(&mut para.mark, accept);
        if let Some(sec) = para.section.as_deref_mut() {
            fmt_revisions::resolve_section(sec, accept);
        }
        para.touch();
    }
    let (ins, del) = (para.mark.ins, para.mark.del);
    if ins.is_none() && del.is_none() {
        return Ok(());
    }
    // An inserted paragraph mark that is rejected, or a deleted one that is accepted, goes away:
    // the next paragraph joins this one (the split is undone). A paragraph left empty goes away
    // whole, so the next one keeps its own style and numbering, as in Word. The mark at the end
    // of a story or cell can't go, so it just stops being a revision. An empty paragraph whose
    // mark carries a section break goes whole too: its section joins the following one, as when
    // the break is deleted (#5667). Joining instead gave the next paragraph its properties.
    if (del.is_some() && accept) || (ins.is_some() && !accept) {
        let sectioned = story == StoryRef::Body
            && path.0.len() == 1
            && s.doc.para(story, path).is_some_and(|p| p.is_empty() && p.section.is_some())
            && crate::cmd::has_next_para(s, story, path);
        if sectioned {
            s.doc.remove_section_break(path.last());
        }
        if crate::cmd::remove_empty_para(s, story, path)? || crate::cmd::join_next_para(s, story, path)? {
            return Ok(());
        }
    }
    let para = s.doc.para_mut(story, path)?;
    para.mark.ins = None;
    para.mark.del = None;
    para.touch();
    Ok(())
}

/// Accept or reject formatting changes on the table, row and cell containing `path`.
/// Returns whether any table, row or cell carried a formatting change.
fn resolve_cell_fmt(s: &mut Session, story: StoryRef, path: &wordcraft_doc::Path, accept: bool) -> Result<bool, CmdError> {
    let Some((tp, r, c)) = path.cell() else { return Ok(false) };
    let has = s.doc.table(story, &tp).is_some_and(|t| {
        let row = t.rows.get(r);
        t.props.fmt_change.is_some()
            || row.is_some_and(|x| x.props.fmt_change.is_some())
            || row.and_then(|x| x.cells.get(c)).is_some_and(|x| x.props.fmt_change.is_some())
    });
    if !has {
        return Ok(false);
    }
    let t = s.doc.table_mut(story, &tp)?;
    fmt_revisions::resolve_table(t, accept);
    if let Some(row) = t.rows.get_mut(r) {
        fmt_revisions::resolve_row(row, accept);
        if let Some(cell) = row.cells.get_mut(c) {
            fmt_revisions::resolve_cell(cell, accept);
        }
    }
    Ok(true)
}

/// Clear matching row-level insertion or deletion marks (`rev`) across all paragraphs and nested
/// tables inside `blocks` when a tracked row is kept (`accept` on insert, reject on delete).
fn clear_blocks_rev(blocks: &mut wordcraft_doc::Blocks, rev: u32, accept: bool, depth: usize) -> Result<(), CmdError> {
    if depth > wordcraft_doc::table::MAX_TABLE_DEPTH {
        return Ok(());
    }
    for b in blocks.iter_mut() {
        let needs = match &**b {
            wordcraft_doc::Block::Para(p) => {
                let mark_hit = if accept { p.mark.ins == Some(rev) } else { p.mark.del == Some(rev) };
                mark_hit || p.runs.iter().any(|r| if accept { r.props.ins == Some(rev) } else { r.props.del == Some(rev) })
            }
            wordcraft_doc::Block::Table(_) => true,
        };
        if !needs {
            continue;
        }
        match std::sync::Arc::make_mut(b) {
            wordcraft_doc::Block::Para(p) => {
                let ranges: Vec<(usize, usize)> = p
                    .run_ranges()
                    .filter(|(_, c)| if accept { c.ins == Some(rev) } else { c.del == Some(rev) })
                    .map(|(rng, _)| (rng.start, rng.end))
                    .collect();
                for (a, b) in ranges.into_iter().rev() {
                    p.format(a, b, &|c| {
                        if accept {
                            c.ins = None;
                        } else {
                            c.del = None;
                        }
                    })?;
                }
                if accept && p.mark.ins == Some(rev) {
                    p.mark.ins = None;
                } else if !accept && p.mark.del == Some(rev) {
                    p.mark.del = None;
                }
                p.touch();
            }
            wordcraft_doc::Block::Table(t) => {
                for row in &mut t.rows {
                    clear_row_rev(row, rev, accept, depth + 1)?;
                }
            }
        }
    }
    Ok(())
}

/// Clear matching row-level insertion or deletion marks (`rev`) on `row` and its cells.
fn clear_row_rev(row: &mut wordcraft_doc::table::Row, rev: u32, accept: bool, depth: usize) -> Result<(), CmdError> {
    if depth > wordcraft_doc::table::MAX_TABLE_DEPTH {
        return Ok(());
    }
    if accept && row.props.ins == Some(rev) {
        row.props.ins = None;
    } else if !accept && row.props.del == Some(rev) {
        row.props.del = None;
    }
    for cell in &mut row.cells {
        clear_blocks_rev(&mut cell.blocks, rev, accept, depth)?;
    }
    Ok(())
}

/// Accept or reject a row-level insertion or deletion marker on row `r` of table `tp`.
///
/// Rejecting an inserted row or accepting a deleted row deletes the row (removing the table if
/// it becomes empty). Accepting an inserted row or rejecting a deleted row clears the row
/// marker and matching revision marks inside its cells.
fn resolve_row_marker(s: &mut Session, story: StoryRef, tp: &wordcraft_doc::Path, r: usize, accept: bool) -> Result<bool, CmdError> {
    let Some((ins, del)) = s.doc.table(story, tp).and_then(|t| t.rows.get(r)).map(|row| (row.props.ins, row.props.del)) else {
        return Ok(false);
    };
    if ins.is_none() && del.is_none() {
        return Ok(false);
    }
    if (!accept && ins.is_some()) || (accept && del.is_some()) {
        let t = s.doc.table_mut(story, tp)?;
        t.delete_row(r);
        if t.rows.is_empty() {
            if let Ok(bl) = s.doc.container_mut(story, tp) {
                bl.retain(|b| !matches!(&**b, wordcraft_doc::Block::Table(t) if t.rows.is_empty()));
                if bl.is_empty() {
                    bl.push(wordcraft_doc::para_block(Paragraph::new()));
                }
            }
            s.doc.ensure_nonempty();
        }
        s.clamp_selection();
        return Ok(true);
    }
    let Some(rev) = (if accept { ins } else { del }) else {
        return Ok(false);
    };
    let t = s.doc.table_mut(story, tp)?;
    if let Some(row) = t.rows.get_mut(r) {
        clear_row_rev(row, rev, accept, 0)?;
    }
    Ok(true)
}

/// Accept or reject the changes of the table cell `path` is in: the table's, its row's
/// formatting or insertion/deletion marker, and the cell's own. Returns whether there were any.
fn resolve_cell(s: &mut Session, story: StoryRef, path: &wordcraft_doc::Path, accept: bool) -> Result<bool, CmdError> {
    let Some((tp, r, _)) = path.cell() else { return Ok(false) };
    let fmt = resolve_cell_fmt(s, story, path, accept)?;
    let row = resolve_row_marker(s, story, &tp, r, accept)?;
    Ok(fmt || row)
}

/// Accept or reject every style-definition change when the whole body is selected. A text range
/// that merely begins at the document start must not resolve unrelated style revisions.
fn resolve_styles_sel(s: &mut Session, whole_body: bool, accept: bool) -> Result<bool, CmdError> {
    if !whole_body {
        return Ok(false);
    }
    if !s.doc.styles.styles.iter().any(|st| st.para_change.is_some() || st.chr_change.is_some()) {
        return Ok(false);
    }
    for st in &mut std::sync::Arc::make_mut(&mut s.doc.styles).styles {
        fmt_revisions::resolve_style(st, accept);
    }
    Ok(true)
}

/// The style-definition or row-marker change Next / Previous Change selected (#2697, #5921).
/// Collapsed changes share an anchor with the start of the document or cell, so the selection alone
/// can't distinguish them: this tracks which one was navigated to for the Accept or Reject that
/// follows, while the selection stays where navigation left it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StyleTarget {
    document: u64,
    sel: Selection,
    style: Option<String>,
    row: Option<(wordcraft_doc::Path, usize)>,
    rev: u32,
}

/// The navigated style or row change, when the selection is still the one navigation made.
fn navigated_style(s: &Session) -> Option<StyleTarget> {
    s.review_style.clone().filter(|t| t.document == s.document_id() && t.sel == s.sel)
}

/// Accept or reject the navigated style change alone: that revision in that style. Returns whether
/// it was still there.
fn resolve_style_change(s: &mut Session, t: &StyleTarget, accept: bool) -> bool {
    let Some(style) = t.style.as_deref() else {
        return false;
    };
    let revs = |st: &wordcraft_doc::styles::Style| [st.para_change.as_ref().map(|c| c.rev), st.chr_change.as_ref().map(|c| c.rev)];
    if !s.doc.styles.styles.iter().any(|st| st.id == style && revs(st).contains(&Some(t.rev))) {
        return false;
    }
    for st in std::sync::Arc::make_mut(&mut s.doc.styles).styles.iter_mut().filter(|st| st.id == style) {
        fmt_revisions::resolve_style_rev(st, t.rev, accept);
    }
    true
}

/// Accept or reject the navigated row-marker change alone when that row still carries `t.rev`.
fn resolve_navigated_row(s: &mut Session, t: &StyleTarget, accept: bool) -> Result<bool, CmdError> {
    let Some((tp, r)) = &t.row else {
        return Ok(false);
    };
    let story = s.sel.anchor.story;
    let has_rev =
        s.doc.table(story, tp).and_then(|tbl| tbl.rows.get(*r)).is_some_and(|row| row.props.ins == Some(t.rev) || row.props.del == Some(t.rev));
    if !has_rev {
        return Ok(false);
    }
    resolve_row_marker(s, story, tp, *r, accept)?;
    Ok(true)
}

/// Select change `c` and remember which style or row-marker change it is, if it is one.
fn select_change(s: &mut Session, c: &Change) {
    s.sel = Selection { anchor: c.a.clone(), focus: c.b.clone() };
    s.review_style = if c.style.is_some() || c.row.is_some() {
        c.rev.map(|rev| StyleTarget { document: s.document_id(), sel: s.sel.clone(), style: c.style.clone(), row: c.row.clone(), rev })
    } else {
        None
    };
}

/// A tracked change, for navigation and the Reviewing Pane: its range, kind (`insert`,
/// `delete`, `format`), revision, and for a formatting change the properties it set; for a
/// style-definition change, the style's id; for a row-marker change, `(table_path, row_index)`.
struct Change {
    a: Pos,
    b: Pos,
    kind: &'static str,
    rev: Option<u32>,
    props: Vec<Value>,
    style: Option<String>,
    row: Option<(wordcraft_doc::Path, usize)>,
}

impl Change {
    /// Build a formatting change at `[a, b]` for `rev`.
    fn format(a: Pos, b: Pos, rev: u32, props: Vec<Value>) -> Change {
        Change { a, b, kind: "format", rev: Some(rev), props, style: None, row: None }
    }
}

/// The tracked paragraph mark at the end of the paragraph at `path`, as a change range: from
/// the paragraph's end to the start of the next paragraph (or the end itself when none follows).
fn mark_change(s: &Session, story: StoryRef, path: &wordcraft_doc::Path) -> Option<Change> {
    let p = s.doc.para(story, path)?;
    let kind = run_kind(&s.doc, p.mark.ins, p.mark.del);
    if kind.is_empty() {
        return None;
    }
    let a = Pos { story, path: path.clone(), off: p.len() };
    let b = if crate::cmd::has_next_para(s, story, path) {
        Pos { story, path: path.with_last(path.last().saturating_add(1)), off: 0 }
    } else {
        a.clone()
    };
    Some(Change { a, b, kind, rev: p.mark.ins.or(p.mark.del), props: Vec::new(), style: None, row: None })
}

/// The kind of an insertion / deletion change: `insert`, `delete`, or for one end of a move
/// `moveTo` / `moveFrom`; empty when neither.
fn run_kind(doc: &wordcraft_doc::Document, ins: Option<u32>, del: Option<u32>) -> &'static str {
    match (ins, del) {
        (Some(_), _) if doc.move_name(ins).is_some() => "moveTo",
        (Some(_), _) => "insert",
        (None, Some(_)) if doc.move_name(del).is_some() => "moveFrom",
        (None, Some(_)) => "delete",
        _ => "",
    }
}

/// The moves in the body, for balloons: where each end starts, its author, and whether it is
/// the source (`true`) or the destination.
pub fn move_changes(s: &Session) -> Vec<(Pos, String, bool)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for c in changes(s) {
        let from = match c.kind {
            "moveFrom" => true,
            "moveTo" => false,
            _ => continue,
        };
        let rev = c.rev.and_then(|r| s.doc.revisions.get(r as usize));
        let name = rev.and_then(|r| r.move_name.clone()).unwrap_or_default();
        // One balloon per end (an end runs on across paragraphs).
        if seen.insert((name, from)) {
            out.push((c.a, rev.map(|r| r.author.clone()).unwrap_or_default(), from));
        }
    }
    out
}

/// Accept or reject the tracked text and paragraph marks of the top-level paragraphs in a block
/// list that is not a story (a note separator).
fn resolve_loose_blocks(blocks: &mut wordcraft_doc::Blocks, accept: bool) -> Result<(), CmdError> {
    for b in blocks.iter_mut() {
        let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) else { continue };
        let len = p.len();
        resolve_runs(p, 0, len, true, accept)?;
        if p.mark.fmt_change.is_some() || p.section.as_ref().is_some_and(|x| x.fmt_change.is_some()) {
            fmt_revisions::resolve_char(&mut p.mark, accept);
            if let Some(sec) = p.section.as_deref_mut() {
                fmt_revisions::resolve_section(sec, accept);
            }
        }
        // A mark can't be removed here (no neighbour to join), so it just stops being a revision.
        p.mark.ins = None;
        p.mark.del = None;
        p.touch();
    }
    fmt_revisions::resolve_containers(blocks, accept, 0);
    Ok(())
}

/// Whether a paragraph has any tracked text, formatting, mark or section revision.
fn para_has_rev(p: &Paragraph) -> bool {
    p.mark.ins.is_some()
        || p.mark.del.is_some()
        || p.mark.fmt_change.is_some()
        || p.props.fmt_change.is_some()
        || p.props.num_change.is_some()
        || p.section.as_ref().is_some_and(|x| x.fmt_change.is_some())
        || p.runs.iter().any(|r| r.props.ins.is_some() || r.props.del.is_some() || r.props.fmt_change.is_some())
}

/// Whether `blocks` contains any paragraph revision nested deeper than `Document::para_paths`
/// walks (`depth > 16`, up to [`wordcraft_doc::table::MAX_TABLE_DEPTH`]).
fn has_deep_para_rev(blocks: &wordcraft_doc::Blocks, depth: usize) -> bool {
    const WALK_LIMIT: usize = 16;
    if depth > wordcraft_doc::table::MAX_TABLE_DEPTH {
        return false;
    }
    blocks.iter().any(|b| match &**b {
        wordcraft_doc::Block::Para(p) => depth > WALK_LIMIT && para_has_rev(p),
        wordcraft_doc::Block::Table(t) => t.rows.iter().any(|r| r.cells.iter().any(|c| has_deep_para_rev(&c.blocks, depth + 1))),
    })
}

/// Resolve tracked text, paragraph marks and formatting on paragraphs nested deeper than
/// `Document::para_paths` walks (`depth > 16`, up to [`wordcraft_doc::table::MAX_TABLE_DEPTH`]).
fn resolve_deep_paras(blocks: &mut wordcraft_doc::Blocks, accept: bool, depth: usize) -> Result<(), CmdError> {
    const WALK_LIMIT: usize = 16;
    if depth > wordcraft_doc::table::MAX_TABLE_DEPTH || !has_deep_para_rev(blocks, depth) {
        return Ok(());
    }
    for b in blocks.iter_mut() {
        let needs = match &**b {
            wordcraft_doc::Block::Para(p) => depth > WALK_LIMIT && para_has_rev(p),
            wordcraft_doc::Block::Table(t) => t.rows.iter().any(|r| r.cells.iter().any(|c| has_deep_para_rev(&c.blocks, depth + 1))),
        };
        if !needs {
            continue;
        }
        match std::sync::Arc::make_mut(b) {
            wordcraft_doc::Block::Para(p) => {
                let len = p.len();
                resolve_runs(p, 0, len, true, accept)?;
                if p.mark.fmt_change.is_some() || p.section.as_ref().is_some_and(|x| x.fmt_change.is_some()) {
                    fmt_revisions::resolve_char(&mut p.mark, accept);
                    if let Some(sec) = p.section.as_deref_mut() {
                        fmt_revisions::resolve_section(sec, accept);
                    }
                }
                p.mark.ins = None;
                p.mark.del = None;
                p.touch();
            }
            wordcraft_doc::Block::Table(t) => {
                for r in &mut t.rows {
                    for c in &mut r.cells {
                        resolve_deep_paras(&mut c.blocks, accept, depth + 1)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Accept (`accept = true`) or reject (`accept = false`) every tracked change in every story,
/// container, style and note separator of the document.
fn resolve_all(s: &mut Session, accept: bool) -> CmdResult {
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k))).collect();
    for st in stories {
        for path in s.doc.para_paths(st).into_iter().rev() {
            let len = s.doc.para(st, &path).map(|p| p.len()).unwrap_or(0);
            resolve_para(s, st, &path, (0, len), (true, true), accept)?;
        }
    }
    resolve_deep_paras(&mut s.doc.body, accept, 0)?;
    for part in s.doc.parts.values_mut() {
        resolve_deep_paras(&mut part.blocks, accept, 0)?;
    }
    // Tables, rows, cells, sections and style definitions.
    fmt_revisions::resolve_containers(&mut s.doc.body, accept, 0);
    for part in s.doc.parts.values_mut() {
        fmt_revisions::resolve_containers(&mut part.blocks, accept, 0);
    }
    fmt_revisions::resolve_section(&mut s.doc.last_section, accept);
    if s.doc.styles.styles.iter().any(|st| st.para_change.is_some() || st.chr_change.is_some()) {
        for st in &mut std::sync::Arc::make_mut(&mut s.doc.styles).styles {
            fmt_revisions::resolve_style(st, accept);
        }
    }
    // The note separators are block lists of their own, not stories; they must not keep ids of
    // revisions that are about to be dropped.
    for sep in [&mut s.doc.footnote_separators, &mut s.doc.endnote_separators] {
        for blocks in [&mut sep.separator, &mut sep.continuation_separator, &mut sep.continuation_notice].into_iter().flatten() {
            resolve_loose_blocks(blocks, accept)?;
        }
    }
    s.doc.revisions.clear();
    s.clamp_selection();
    sel_result(s)
}

/// Accept or Reject All; with `stopTracking`, Track Changes is turned off as well.
pub(super) fn resolve_all_then(s: &mut Session, v: &Value, accept: bool) -> CmdResult {
    let out = resolve_all(s, accept)?;
    if p::bool(v, "stopTracking") == Some(true) {
        s.doc.settings.track_changes = false;
    }
    Ok(out)
}

/// Accept or Reject at the selection; with `next`, the next change is selected afterwards.
pub(super) fn resolve_sel_then(s: &mut Session, v: &Value, accept: bool) -> CmdResult {
    resolve_sel(s, accept, p::bool(v, "next") == Some(true))
}

/// The span of the tracked change around `off` in `p`: its run, joined across the comment anchors
/// that split one revision's text into several runs.
fn revision_span(p: &Paragraph, off: usize) -> Option<std::ops::Range<usize>> {
    let runs: Vec<_> = p.run_ranges().collect();
    let i = runs.iter().position(|(r, c)| r.start <= off && off <= r.end && (c.ins.is_some() || c.del.is_some() || c.fmt_change.is_some()))?;
    let (mut start, mut end) = (runs[i].0.start, runs[i].0.end);
    let key = (runs[i].1.ins, runs[i].1.del);
    if key == (None, None) {
        return Some(start..end);
    }
    // Comment anchors split a revision's text into several runs; the revision goes on past them.
    let is_anchor = |k: usize| matches!(p.object_at(runs[k].0.start), Some(InlineObject::CommentStart { .. } | InlineObject::CommentEnd { .. }));
    let same = |k: usize| (runs[k].1.ins, runs[k].1.del) == key;
    let mut k = i;
    while k > 0 {
        if same(k - 1) {
            k -= 1;
        } else if is_anchor(k - 1) && k > 1 && same(k - 2) {
            k -= 2;
        } else {
            break;
        }
        start = runs[k].0.start;
    }
    let mut k = i;
    while k + 1 < runs.len() {
        if same(k + 1) {
            k += 1;
        } else if is_anchor(k + 1) && k + 2 < runs.len() && same(k + 2) {
            k += 2;
        } else {
            break;
        }
        end = runs[k].0.end;
    }
    Some(start..end)
}

/// Whether any revision in `p` overlapping `(from..to)` (or its paragraph mark when `mark` is
/// set) belongs to the enclosing row's insertion or deletion revision `(row_ins, row_del)`.
fn para_slice_matches_row_rev(p: &Paragraph, (from, to): (usize, usize), mark: bool, (row_ins, row_del): (Option<u32>, Option<u32>)) -> bool {
    let run_hit = p.run_ranges().any(|(rng, cp)| {
        rng.end > from && rng.start < to && ((row_ins.is_some() && cp.ins == row_ins && cp.del.is_none()) || (row_del.is_some() && cp.del == row_del))
    });
    let mark_hit = mark && ((row_ins.is_some() && p.mark.ins == row_ins && p.mark.del.is_none()) || (row_del.is_some() && p.mark.del == row_del));
    run_hit || mark_hit
}

/// Whether selection `(a, b)` spans row `r` of table `tp` across cell/row boundaries (such as
/// Select All or a multi-cell selection from start to end of the row), rather than staying inside
/// an individual paragraph change within the row.
fn row_spanned_by_sel(
    doc: &wordcraft_doc::Document,
    story: StoryRef,
    tp: &wordcraft_doc::Path,
    r: usize,
    (a, b): (&Pos, &Pos),
    whole_body: bool,
) -> bool {
    if whole_body {
        return true;
    }
    if a == b || a.path == b.path {
        return false;
    }
    let Ok(row_idx) = u32::try_from(r) else {
        return false;
    };
    let paths = doc.para_paths(story);
    let mut row_paths = paths.iter().filter(|p| p.0.len() > tp.0.len() + 1 && p.0.starts_with(&tp.0) && p.0.get(tp.0.len()) == Some(&row_idx));
    let Some(first) = row_paths.next() else {
        return false;
    };
    let last = row_paths.next_back().unwrap_or(first);
    let last_len = doc.para(story, last).map(|p| p.len()).unwrap_or(0);
    a <= &Pos { story, path: first.clone(), off: 0 } && b >= &Pos { story, path: last.clone(), off: last_len }
}

/// Accept or reject the tracked change at the current selection (or advance to the next change
/// when no tracked change is under a collapsed caret).
fn resolve_sel(s: &mut Session, accept: bool, next: bool) -> CmdResult {
    // The style-definition or row-marker change Next / Previous Change selected: that one alone. A
    // caret put at the anchor any other way resolves no style change and expands at the caret.
    let navigated = navigated_style(s);
    s.review_style = None;
    if let Some(t) = navigated
        && (resolve_style_change(s, &t, accept) || resolve_navigated_row(s, &t, accept)?)
    {
        return if next { select_next_unresolved(s) } else { sel_result(s) };
    }
    let (a, b) = s.sel.ordered();
    // Capture this before text revisions are resolved: accepting a deletion or rejecting an
    // insertion can change the body's end position.
    let whole_body = a != b && a == s.doc.start_of(StoryRef::Body) && b == s.doc.end_of(StoryRef::Body);
    // A caret right before a tracked paragraph mark (and no tracked text around it): that mark.
    let mut caret_mark = None;
    // A caret in a paragraph whose own formatting changed (and no tracked text around it).
    let mut caret_props = None;
    let ranged = a != b;
    let (a, b) = if a == b {
        // The change at the caret: the run around it, else the paragraph mark right after it,
        // else the paragraph's formatting, else the table cell's.
        let found = s.doc.para_at(&a).and_then(|p| revision_span(p, a.off));
        let para_fmt = s.doc.para_at(&a).is_some_and(|p| p.props.fmt_change.is_some() || p.props.num_change.is_some());
        let mark_fmt = s.doc.para_at(&a).is_some_and(|p| a.off == p.len() && p.mark.fmt_change.is_some());
        match found {
            Some(r) => (Pos { off: r.start, ..a.clone() }, Pos { off: r.end, ..a }),
            None if mark_fmt || mark_change(s, a.story, &a.path).is_some_and(|m| m.a == a) => {
                caret_mark = Some(a.path.clone());
                (a.clone(), a)
            }
            None if para_fmt => {
                caret_props = Some(a.path.clone());
                (a.clone(), a)
            }
            None => {
                if resolve_cell(s, a.story, &a.path, accept)? {
                    return if next { select_next_unresolved(s) } else { sel_result(s) };
                }
                return nav_change(s, 1);
            }
        }
    } else {
        (a, b)
    };
    // Moves with an end in the range: their other ends go with them.
    let moves = crate::cmd::moves::names_in(s, &a, &b, caret_mark.as_ref());
    let mut row_targets: std::collections::BTreeSet<(usize, wordcraft_doc::Path, usize)> = std::collections::BTreeSet::new();
    for path in s.doc.paths_between(&a, &b).into_iter().rev() {
        let len = s.doc.para(a.story, &path).map(|p| p.len()).unwrap_or(0);
        let from = if path == a.path { a.off } else { 0 };
        let to = if path == b.path { b.off } else { len };
        // A paragraph's mark is in the selection when the selection goes on past it.
        let mark = path != b.path || caret_mark.as_ref() == Some(&path);
        let props = (ranged && (path != b.path || to > from)) || caret_props.as_ref() == Some(&path);
        if let Some((tp, r, _)) = path.cell()
            && let Some(row) = s.doc.table(a.story, &tp).and_then(|t| t.rows.get(r))
            && (row.props.ins.is_some() || row.props.del.is_some())
            && !row_targets.contains(&(tp.0.len(), tp.clone(), r))
        {
            let rev_match =
                s.doc.para(a.story, &path).is_some_and(|p| para_slice_matches_row_rev(p, (from, to), mark, (row.props.ins, row.props.del)));
            if rev_match || row_spanned_by_sel(&s.doc, a.story, &tp, r, (&a, &b), whole_body) {
                row_targets.insert((tp.0.len(), tp, r));
            }
        }
        resolve_para(s, a.story, &path, (from, to), (mark, props), accept)?;
        if ranged {
            resolve_cell_fmt(s, a.story, &path, accept)?;
        }
    }
    // Resolve tracked row insertions/deletions after paragraph traversal (deepest tables and
    // highest row indices first) so deleting a row never invalidates earlier paths in the loop.
    for (_, tp, r) in row_targets.into_iter().rev() {
        resolve_row_marker(s, a.story, &tp, r, accept)?;
    }
    crate::cmd::moves::resolve(s, &moves, accept)?;
    resolve_styles_sel(s, whole_body, accept)?;
    s.sel = Selection::caret(a);
    s.clamp_selection();
    if next { select_next_unresolved(s) } else { sel_result(s) }
}

/// Collect every tracked change in the document body in reading order (including style, table,
/// row, cell, paragraph, run and section revisions).
fn changes(s: &Session) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    let mk = |path: &wordcraft_doc::Path, off| Pos { story: StoryRef::Body, path: path.clone(), off };
    // Style-definition changes sit at the start of the document, as Word shows them.
    let start = s.doc.start_of(StoryRef::Body);
    for st in &s.doc.styles.styles {
        for (rev, props) in fmt_revisions::style_diffs(st) {
            out.push(Change { style: Some(st.id.clone()), ..Change::format(start.clone(), start.clone(), rev, props) });
        }
    }
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        // A table's, row's and cell's changes sit at the start of the cell's first paragraph.
        if path.last() == 0
            && let Some((tp, r, c)) = path.cell()
            && let Some(t) = s.doc.table(StoryRef::Body, &tp)
        {
            let row = t.rows.get(r);
            let cell = row.and_then(|x| x.cells.get(c));
            if c == 0
                && let Some(row) = row
            {
                let kind = run_kind(&s.doc, row.props.ins, row.props.del);
                let covered = p.runs.first().is_some_and(|run| (run.props.ins, run.props.del) == (row.props.ins, row.props.del));
                if !kind.is_empty() && !covered {
                    out.push(Change {
                        a: mk(&path, 0),
                        b: mk(&path, 0),
                        kind,
                        rev: row.props.ins.or(row.props.del),
                        props: Vec::new(),
                        style: None,
                        row: Some((tp.clone(), r)),
                    });
                }
            }
            let found = [
                (r == 0 && c == 0).then(|| t.props.fmt_change.as_ref().map(|ch| (ch.rev, fmt_revisions::table_diff(&t.props)))).flatten(),
                (c == 0).then(|| row.and_then(|x| x.props.fmt_change.as_ref().map(|ch| (ch.rev, fmt_revisions::row_diff(&x.props))))).flatten(),
                cell.and_then(|x| x.props.fmt_change.as_ref().map(|ch| (ch.rev, fmt_revisions::cell_diff(&x.props)))),
            ];
            for (rev, props) in found.into_iter().flatten() {
                out.push(Change::format(mk(&path, 0), mk(&path, 0), rev, props));
            }
        }
        if let Some(ch) = &p.props.fmt_change {
            out.push(Change::format(mk(&path, 0), mk(&path, p.len()), ch.rev, fmt_revisions::para_diff(&p.props)));
        }
        for (r, c) in p.run_ranges() {
            let kind = run_kind(&s.doc, c.ins, c.del);
            if !kind.is_empty() {
                out.push(Change {
                    a: mk(&path, r.start),
                    b: mk(&path, r.end),
                    kind,
                    rev: c.ins.or(c.del),
                    props: Vec::new(),
                    style: None,
                    row: None,
                });
            }
            let Some(ch) = &c.fmt_change else { continue };
            let props = fmt_revisions::char_diff(c);
            // Neighbouring runs changed the same way by the same revision are one change.
            if let Some(last) =
                out.last_mut().filter(|l| l.kind == "format" && l.rev == Some(ch.rev) && l.b == mk(&path, r.start) && l.props == props)
            {
                last.b = mk(&path, r.end);
            } else {
                out.push(Change::format(mk(&path, r.start), mk(&path, r.end), ch.rev, props));
            }
        }
        if let Some(ch) = &p.mark.fmt_change {
            let end = mk(&path, p.len());
            out.push(Change::format(end.clone(), end, ch.rev, fmt_revisions::char_diff(&p.mark)));
        }
        out.extend(mark_change(s, StoryRef::Body, &path));
        if let Some(ch) = p.section.as_ref().and_then(|x| x.fmt_change.as_ref().map(|ch| (ch.rev, x))) {
            let end = mk(&path, p.len());
            out.push(Change::format(end.clone(), end, ch.0, fmt_revisions::section_diff(ch.1)));
        }
    }
    if let Some(ch) = &s.doc.last_section.fmt_change {
        let end = s.doc.end_of(StoryRef::Body);
        out.push(Change::format(end.clone(), end, ch.rev, fmt_revisions::section_diff(&s.doc.last_section)));
    }
    out
}

/// The formatting changes in the body, for balloons: (position, author, properties set).
pub fn format_changes(s: &Session) -> Vec<(Pos, String, Vec<Value>)> {
    changes(s)
        .into_iter()
        .filter(|c| c.kind == "format")
        .map(|c| {
            let author = c.rev.and_then(|r| s.doc.revisions.get(r as usize)).map(|r| r.author.clone()).unwrap_or_default();
            (c.a, author, c.props)
        })
        .collect()
}

/// Whether the document has any tracked formatting change (cheap: stops at the first).
pub fn has_format_changes(doc: &wordcraft_doc::Document) -> bool {
    if doc.last_section.fmt_change.is_some() {
        return true;
    }
    if doc.styles.styles.iter().any(|st| st.para_change.is_some() || st.chr_change.is_some()) {
        return true;
    }
    doc.para_paths(StoryRef::Body).iter().any(|path| {
        doc.para(StoryRef::Body, path).is_some_and(|p| {
            p.props.fmt_change.is_some()
                || p.mark.fmt_change.is_some()
                || p.runs.iter().any(|r| r.props.fmt_change.is_some())
                || p.section.as_ref().is_some_and(|x| x.fmt_change.is_some())
                || path.cell().and_then(|(tp, r, c)| doc.table(StoryRef::Body, &tp).map(|t| (t, r, c))).is_some_and(|(t, r, c)| {
                    t.props.fmt_change.is_some()
                        || t.rows.get(r).is_some_and(|x| x.props.fmt_change.is_some() || x.cells.get(c).is_some_and(|x| x.props.fmt_change.is_some()))
                })
        })
    })
}

/// A tracked deletion in the body shown in a balloon (Show revisions in balloons).
#[derive(Clone, Debug, PartialEq)]
pub struct DeletionBalloon {
    /// Where the deleted text was: the balloon's anchor.
    pub at: Pos,
    pub author: String,
    /// The deleted text, a deleted paragraph mark as ¶, shortened to [`DELETION_TEXT_CHARS`].
    pub text: String,
}

/// At most this many characters of deleted text show in a balloon (a deleted page shows its start).
pub const DELETION_TEXT_CHARS: usize = 200;

/// At most this many deletion balloons (a document of many small deletions can't flood the area).
const MAX_DELETION_BALLOONS: usize = 500;

/// The tracked deletions in the body, for balloons, in document order. Deleted text running on
/// across runs, revisions and paragraph marks by the same author is one balloon. The source of a
/// move is left out: it has its "Moved from" balloon ([`move_changes`]).
pub fn deletion_balloons(s: &Session) -> Vec<DeletionBalloon> {
    let doc = &s.doc;
    let author = |rev: u32| doc.revisions.get(rev as usize).map(|r| r.author.clone()).unwrap_or_default();
    let mk = |path: &wordcraft_doc::Path, off| Pos { story: StoryRef::Body, path: path.clone(), off };
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for (r, c) in p.run_ranges() {
            // Inserted and then deleted text is a deletion (it's drawn and left out as one).
            let Some(rev) = plain_deletion(doc, c.del) else { continue };
            push_deletion(&mut out, mk(&path, r.start), mk(&path, r.end), author(rev), p.text.get(r).unwrap_or(""));
        }
        if let Some(rev) = plain_deletion(doc, p.mark.del) {
            let at = mk(&path, p.len());
            let next = path.with_last(path.last().saturating_add(1));
            let end = if crate::cmd::has_next_para(s, StoryRef::Body, &path) { mk(&next, 0) } else { at.clone() };
            push_deletion(&mut out, at, end, author(rev), "¶");
        }
    }
    out.into_iter().map(|(d, _, _)| d).collect()
}

/// Add `author`'s deleted `text` (from `at` to `end`) to `out`: to the last balloon when that
/// deletion ends at `at` and is theirs, else as a new one. Each balloon carries where its deletion
/// ends and how many characters its text has.
fn push_deletion(out: &mut Vec<(DeletionBalloon, Pos, usize)>, at: Pos, end: Pos, author: String, text: &str) {
    let joins = out.last().is_some_and(|(d, e, _)| *e == at && d.author == author);
    if !joins {
        if out.len() >= MAX_DELETION_BALLOONS {
            return;
        }
        out.push((DeletionBalloon { at, author, text: String::new() }, end.clone(), 0));
    }
    let Some((d, e, chars)) = out.last_mut() else { return };
    *e = end;
    for c in text.chars() {
        if *chars > DELETION_TEXT_CHARS {
            break;
        }
        let c = match c {
            // Pictures, fields and other objects show no text of their own.
            wordcraft_doc::para::OBJ => continue,
            '\t' | wordcraft_doc::para::LINE_BREAK | wordcraft_doc::para::PAGE_BREAK | wordcraft_doc::para::COLUMN_BREAK => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        *chars += 1;
        d.text.push(if *chars > DELETION_TEXT_CHARS { '…' } else { c });
    }
}

/// A deletion revision that is not the source of a move (`w:moveFrom` is a deletion revision
/// with a move name).
fn plain_deletion(doc: &wordcraft_doc::Document, del: Option<u32>) -> Option<u32> {
    del.filter(|_| doc.move_name(del).is_none())
}

/// Whether the body has any tracked deletion other than a move's source (cheap: stops at the
/// first).
pub fn has_deletions(doc: &wordcraft_doc::Document) -> bool {
    doc.para_paths(StoryRef::Body).iter().any(|path| {
        doc.para(StoryRef::Body, path)
            .is_some_and(|p| plain_deletion(doc, p.mark.del).is_some() || p.runs.iter().any(|r| plain_deletion(doc, r.props.del).is_some()))
    })
}

/// Navigate to the next (`dir > 0`) or previous (`dir < 0`) tracked change in reading order.
pub(super) fn nav_change(s: &mut Session, dir: i32) -> CmdResult {
    let list = changes(s);
    let caret = s.sel.ordered();
    // From a navigated style or row-marker change, the one next to it in the list (they can
    // share their collapsed anchor with another change).
    let at = navigated_style(s).and_then(|t| {
        list.iter().position(|c| c.rev == Some(t.rev) && ((t.style.is_some() && c.style == t.style) || (t.row.is_some() && c.row == t.row)))
    });
    let t = match at {
        Some(i) if dir > 0 && i + 1 < list.len() => list.get(i + 1),
        Some(i) if dir < 0 && i > 0 => list.get(i - 1),
        _ if dir > 0 => list.iter().find(|c| c.a >= caret.1 && (c.a > caret.0 || c.b > caret.1)).or(list.first()),
        _ => list.iter().rev().find(|c| c.b <= caret.0 && (c.b < caret.1 || c.a < caret.0)).or(list.last()),
    };
    if let Some(c) = t {
        select_change(s, c);
    }
    sel_result(s)
}

/// After a change at the caret is resolved: select the first change from the caret on, including
/// one that starts and ends there (an empty paragraph's, a paragraph mark's or a table's
/// formatting), else the first change in the document.
fn select_next_unresolved(s: &mut Session) -> CmdResult {
    let list = changes(s);
    let caret = s.sel.ordered().0;
    if let Some(c) = list.iter().find(|c| c.a >= caret).or(list.first()) {
        select_change(s, c);
    }
    sel_result(s)
}

pub(super) fn list_changes(s: &mut Session, _: &Value) -> CmdResult {
    let list = changes(s);
    Ok(Value::Array(
        list.iter()
            .map(|c| {
                let rev = c.rev.and_then(|r| s.doc.revisions.get(r as usize));
                let mut v = json!({
                    "kind": c.kind, "start": pos_json(&c.a), "end": pos_json(&c.b),
                    "text": s.doc.copy_range(&c.a, &c.b).plain_text(),
                    "author": rev.map(|r| r.author.clone()), "date": rev.map(|r| r.date.clone()),
                });
                if c.kind == "format" {
                    v["description"] = json!(fmt_revisions::describe(&c.props));
                    v["props"] = json!(c.props);
                }
                // One end of a move: its name links it with the other end.
                if let Some(name) = rev.and_then(|r| r.move_name.as_deref()).filter(|_| c.kind.starts_with("move")) {
                    v["move"] = json!(name);
                    v["description"] = json!(if c.kind == "moveFrom" { "Moved from" } else { "Moved to" });
                }
                v
            })
            .collect(),
    ))
}

/// Track Changes Options: change the fields given, return them all. A per-user preference.
pub(super) fn tracking_options(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_doc::Rgb;
    use wordcraft_layout::display::{BalloonMode, ChangeBar, CommentDisplay, DeleteMark, InsertMark};
    let mut m = s.prefs.markup.clone();
    let color = |k: &str, auto: &str, cur: Option<Rgb>| -> Result<Option<Rgb>, CmdError> {
        match p::str(v, k) {
            None => Ok(cur),
            Some(c) if c == auto => Ok(None),
            Some(c) => Rgb::parse(c).map(Some).ok_or_else(|| CmdError::Params(format!("`{k}`: \"{auto}\" or RRGGBB"))),
        }
    };
    let pick = |k: &str| p::str(v, k);
    for (k, f) in [
        ("comments", &mut m.comments),
        ("ink", &mut m.ink),
        ("insertionsDeletions", &mut m.insertions_deletions),
        ("formatting", &mut m.formatting),
        ("trackFormatting", &mut m.track_formatting),
    ] {
        if let Some(b) = p::bool(v, k) {
            *f = b;
        }
    }
    if let Some(x) = pick("balloons") {
        m.balloons = BalloonMode::parse(x).ok_or_else(|| CmdError::Params("`balloons`: revisions|inline|commentsAndFormatting".into()))?;
    }
    if let Some(x) = pick("commentDisplay") {
        m.comment_display = CommentDisplay::parse(x).ok_or_else(|| CmdError::Params("`commentDisplay`: margin|selected".into()))?;
    }
    if let Some(x) = pick("insertMark") {
        m.insert_mark = InsertMark::parse(x).ok_or_else(|| CmdError::Params("unknown `insertMark`".into()))?;
    }
    if let Some(x) = pick("deleteMark") {
        m.delete_mark = DeleteMark::parse(x).ok_or_else(|| CmdError::Params("unknown `deleteMark`".into()))?;
    }
    if let Some(x) = pick("moveFromMark") {
        m.move_from_mark = DeleteMark::parse(x).ok_or_else(|| CmdError::Params("unknown `moveFromMark`".into()))?;
    }
    if let Some(x) = pick("moveToMark") {
        m.move_to_mark = InsertMark::parse(x).ok_or_else(|| CmdError::Params("unknown `moveToMark`".into()))?;
    }
    if let Some(x) = pick("changedLines") {
        m.changed_lines = ChangeBar::parse(x).ok_or_else(|| CmdError::Params("unknown `changedLines`".into()))?;
    }
    m.insert_color = color("insertColor", "byAuthor", m.insert_color)?;
    m.delete_color = color("deleteColor", "byAuthor", m.delete_color)?;
    m.changed_lines_color = color("changedLinesColor", "auto", m.changed_lines_color)?;
    m.move_from_color = color("moveFromColor", "byAuthor", m.move_from_color)?;
    m.move_to_color = color("moveToColor", "byAuthor", m.move_to_color)?;
    // Track moves is the document's (`w:doNotTrackMoves`), so changing it is an undo step; the
    // command is pure, so it records the step itself, once every parameter has been accepted.
    // The other options are the user's own, so the command stays available under Restrict
    // Editing; this one edits the document, so it passes the same check as an edit and isn't
    // limited to an exception.
    if let Some(b) = p::bool(v, "trackMoves")
        && b != s.doc.settings.track_moves
    {
        if crate::cmd::protect::check_before(s, "review.trackingOptions", &json!({}))? {
            return Err(CmdError::Disabled(
                "review.trackingOptions: the document is protected; stop protection to change whether moves are tracked".into(),
            ));
        }
        s.checkpoint("Track Changes Options");
        s.doc.settings.track_moves = b;
        s.touch();
    }
    if m != s.prefs.markup {
        s.prefs.markup = m;
        // Hidden deletions leave the layout.
        s.relayout();
    }
    let mut out = tracking_json(&s.prefs.markup);
    out["trackMoves"] = json!(s.doc.settings.track_moves);
    Ok(out)
}

/// The Track Changes Options as `review.trackingOptions` takes them.
pub fn tracking_json(m: &wordcraft_layout::display::MarkupOptions) -> Value {
    let hex = |c: Option<wordcraft_doc::Rgb>, auto: &str| c.map(|c| c.hex()).unwrap_or_else(|| auto.to_string());
    json!({
        "comments": m.comments,
        "ink": m.ink,
        "insertionsDeletions": m.insertions_deletions,
        "formatting": m.formatting,
        "balloons": m.balloons.name(),
        "commentDisplay": m.comment_display.name(),
        "insertMark": m.insert_mark.name(),
        "insertColor": hex(m.insert_color, "byAuthor"),
        "deleteMark": m.delete_mark.name(),
        "deleteColor": hex(m.delete_color, "byAuthor"),
        "changedLines": m.changed_lines.name(),
        "changedLinesColor": hex(m.changed_lines_color, "auto"),
        "trackFormatting": m.track_formatting,
        "moveFromMark": m.move_from_mark.name(),
        "moveFromColor": hex(m.move_from_color, "byAuthor"),
        "moveToMark": m.move_to_mark.name(),
        "moveToColor": hex(m.move_to_color, "byAuthor"),
    })
}
