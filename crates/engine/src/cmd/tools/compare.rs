use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::{
    Row, Table,
    para::OBJ,
    table::{MAX_ROWS, MAX_TABLE_DEPTH},
};

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character" (character: paragraphs over 2000 characters fall back to word; see characterFallback/notHonoured), "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared)}"#;

/// What Compare / Combine compares (the More >> settings of Word's dialog).
struct CompareOptions {
    /// Differences only in letter case count as changes.
    case: bool,
    /// Differences only in white space count as changes.
    white_space: bool,
    /// Formatting differences become tracked formatting changes.
    formatting: bool,
    /// Changed paragraphs are compared character by character (else word by word).
    characters: bool,
}

impl CompareOptions {
    /// The text a comparison sees: case folded and white space collapsed as the options say.
    fn key(&self, t: &str) -> String {
        let t = if self.white_space { t.to_string() } else { t.split_whitespace().collect::<Vec<_>>().join(" ") };
        if self.case { t } else { t.to_lowercase() }
    }
}

/// Context and tracking counters for diffing paragraph contents.
struct DiffCtx<'a> {
    ins: u32,
    del: u32,
    opts: &'a CompareOptions,
    budget: &'a mut u64,
    char_fallbacks: &'a mut usize,
}

/// Changed paragraphs longer than this (in characters) are compared word by word even at
/// character level, so the comparison table stays small.
const MAX_CHAR_DIFF: usize = 2000;

/// Compare the current document (original, or `original` from a file) with a revised one: the
/// result shows the differences as tracked insertions, deletions and formatting changes, labelled
/// with `author`. Options Word offers that the comparison can't honour yet (moves, comments,
/// headers and footers, footnotes, text boxes, fields) are accepted and listed in the result.
/// When character-level comparison is requested (`"level": "character"`) but any paragraph exceeds
/// [`MAX_CHAR_DIFF`] characters, it falls back to word-level diff for that paragraph, surfaces the
/// fallback count in `"characterFallback"`, and includes `"level"` in `"notHonoured"`.
pub(super) fn compare(s: &mut Session, v: &Value) -> CmdResult {
    let revised = if let Some(t) = p::str(v, "text") {
        Document::from_text(t)
    } else if let Some(path) = p::str(v, "path") {
        crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?
    } else {
        return Err(CmdError::Params("`path` or `text` of the revised document is required".into()));
    };
    let show_in = p::str(v, "showIn").unwrap_or("original");
    if !matches!(show_in, "original" | "revised" | "new") {
        return Err(CmdError::Params("`showIn` is \"original\", \"revised\" or \"new\"".into()));
    }
    if let Some(path) = p::str(v, "original") {
        let doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
        s.doc = doc;
        s.doc.ensure_nonempty();
        s.note_body_replaced();
        s.math = None;
    }
    let level = p::str(v, "level").unwrap_or("word");
    let opts = CompareOptions {
        case: p::bool(v, "caseChanges").unwrap_or(true),
        white_space: p::bool(v, "whiteSpace").unwrap_or(true),
        formatting: p::bool(v, "formatting").unwrap_or(true),
        characters: matches!(level, "character" | "char"),
    };
    let mut not_honoured: Vec<&str> =
        ["moves", "comments", "headersFooters", "footnotes", "textBoxes", "fields"].into_iter().filter(|k| v.get(*k).is_some()).collect();
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    let date = crate::cmd::now_iso();
    let old: Vec<Arc<Block>> = s.doc.body.clone();
    let new: Vec<Arc<Block>> = if revised.parts.values().any(|p| p.kind == wordcraft_doc::PartKind::TextBox) {
        // The revised document's text boxes get stories of their own in the result, so its
        // boxes never show a story of the original's (or none at all).
        s.doc.adopt_fragment(&revised.body_fragment()).into_iter().map(Arc::new).collect()
    } else {
        revised.body.clone()
    };
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Insert, author: author.clone(), date: date.clone(), move_name: None });
    let ins = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Delete, author: author.clone(), date: date.clone(), move_name: None });
    let del = (s.doc.revisions.len() - 1) as u32;
    let mut budget = MAX_COMPARE_CELLS;
    let mut char_fallbacks = 0usize;
    let mut changes = 0usize;
    let mut revisions = std::mem::take(&mut s.doc.revisions);
    let mut ctx = CompareCtx {
        doc: &s.doc,
        ins,
        del,
        opts: &opts,
        budget: &mut budget,
        char_fallbacks: &mut char_fallbacks,
        changes: &mut changes,
        revisions: &mut revisions,
        author: &author,
        date: &date,
    };
    s.doc.body = diff_blocks(&mut ctx, &old, &new, 0);
    s.doc.revisions = revisions;
    s.doc.ensure_nonempty();
    s.note_body_replaced();
    s.math = None;
    if show_in != "original" || p::str(v, "original").is_some() {
        // The result is a new, untitled document: saving it never overwrites either file.
        s.path = None;
        s.file_name = None;
        // Version snapshots belong to the document they were taken from, including when
        // Compare creates an untitled result from the current document.
        s.clear_versions();
    }
    if char_fallbacks > 0 {
        log::warn!(
            "compare: {char_fallbacks} paragraph(s) exceeded character diff limit of {MAX_CHAR_DIFF}; fell back from character to word level diff"
        );
        if !not_honoured.contains(&"level") {
            not_honoured.push("level");
        }
    }
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    Ok(json!({
        "changes": changes,
        "author": author,
        "notHonoured": not_honoured,
        "characterFallback": char_fallbacks,
    }))
}

/// Context passed through block, table and row comparison.
struct CompareCtx<'a> {
    doc: &'a Document,
    ins: u32,
    del: u32,
    opts: &'a CompareOptions,
    budget: &'a mut u64,
    char_fallbacks: &'a mut usize,
    changes: &'a mut usize,
    revisions: &'a mut Vec<wordcraft_doc::Revision>,
    author: &'a str,
    date: &'a str,
}

impl CompareCtx<'_> {
    /// Diff two matched paragraphs using the shared comparison options, budget and fallback counter.
    fn word_diff_para(&mut self, pa: &Paragraph, pb: &Paragraph) -> Paragraph {
        let mut dctx = DiffCtx { ins: self.ins, del: self.del, opts: self.opts, budget: self.budget, char_fallbacks: self.char_fallbacks };
        word_diff(self.doc, pa, pb, &mut dctx)
    }
}

/// Compare two paired table blocks: keep equal tables untouched, replace deeply nested,
/// grid-mismatched or oversized tables whole, and otherwise diff rows and cells via [`table_diff`].
fn diff_tables(ctx: &mut CompareCtx<'_>, out: &mut Vec<Arc<Block>>, a: &Arc<Block>, ta: &Table, b: &Arc<Block>, tb: &Table, depth: usize) {
    if ta == tb {
        out.push(b.clone());
    } else if depth >= MAX_TABLE_DEPTH || ta.grid != tb.grid || ta.rows.len().saturating_add(tb.rows.len()) > MAX_ROWS {
        out.push(Arc::new(mark_block(a, &|c| c.del = Some(ctx.del), depth)));
        out.push(Arc::new(mark_block(b, &|c| c.ins = Some(ctx.ins), depth)));
        *ctx.changes += 2;
    } else {
        out.push(Arc::new(Block::Table(table_diff(ctx, ta, tb, depth))));
    }
}

/// Diff two sequences of blocks (the document body or inside a table cell), pairing corresponding
/// paragraphs and tables, diffing them recursively, and marking unpaired blocks as inserted or deleted.
fn diff_blocks(ctx: &mut CompareCtx<'_>, old: &[Arc<Block>], new: &[Arc<Block>], depth: usize) -> Vec<Arc<Block>> {
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => ctx.opts.key(&p.plain_text()),
        // Tables match by their text.
        Block::Table(_) => format!("\u{0}table\u{0}{}", ctx.opts.key(&block_text(b, depth))),
    };
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), ctx.budget);
    let mut out: Vec<Arc<Block>> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut k = 0;
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                let (a, b) = (old.get(i), new.get(j));
                if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    if !same_objects(ctx.doc, pa, pb) {
                        out.push(para_block(ctx.word_diff_para(pa, pb)));
                        *ctx.changes += 1;
                        i += 1;
                        j += 1;
                        k += 1;
                        continue;
                    }
                    let mut p = pb.clone();
                    if ctx.opts.formatting && pa.text == pb.text {
                        let before = ctx.revisions.len();
                        crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, ctx.revisions, ctx.author, ctx.date);
                        if ctx.revisions.len() > before || p != *pb {
                            *ctx.changes += 1;
                        }
                    }
                    out.push(para_block(p));
                } else if let (Some(ab), Some(bb)) = (a, b)
                    && let (Block::Table(ta), Block::Table(tb)) = (&**ab, &**bb)
                {
                    diff_tables(ctx, &mut out, ab, ta, bb, tb, depth);
                } else if let Some(x) = b.or(a) {
                    out.push(x.clone());
                }
                i += 1;
                j += 1;
                k += 1;
            }
            Some(_) => {
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                for x in 0..dels.max(inss) {
                    let (a, b) = (if x < dels { old.get(i + x) } else { None }, if x < inss { new.get(j + x) } else { None });
                    match (a.map(|x| &**x), b.map(|x| &**x)) {
                        (Some(Block::Para(pa)), Some(Block::Para(pb))) => {
                            out.push(para_block(ctx.word_diff_para(pa, pb)));
                            *ctx.changes += 1;
                        }
                        (Some(Block::Table(ta)), Some(Block::Table(tb))) => {
                            if let (Some(ab), Some(bb)) = (a, b) {
                                diff_tables(ctx, &mut out, ab, ta, bb, tb, depth);
                            }
                        }
                        _ => {
                            if let Some(x) = a {
                                out.push(Arc::new(mark_block(x, &|c| c.del = Some(ctx.del), depth)));
                                *ctx.changes += 1;
                            }
                            if let Some(x) = b {
                                out.push(Arc::new(mark_block(x, &|c| c.ins = Some(ctx.ins), depth)));
                                *ctx.changes += 1;
                            }
                        }
                    }
                }
                i += dels;
                j += inss;
                k += run.max(1);
            }
            None => break,
        }
    }
    out
}

/// Diff two tables row by row: added rows are marked inserted (`row.props.ins`), removed rows
/// are marked deleted (`row.props.del`), and surviving rows diff their cells.
fn table_diff(ctx: &mut CompareCtx<'_>, ta: &Table, tb: &Table, depth: usize) -> Table {
    if depth >= MAX_TABLE_DEPTH || ta.rows.len().saturating_add(tb.rows.len()) > MAX_ROWS {
        return tb.clone();
    }
    let row_key = |r: &Row| ctx.opts.key(&row_text(r, depth));
    let keys_a: Vec<String> = ta.rows.iter().map(row_key).collect();
    let keys_b: Vec<String> = tb.rows.iter().map(row_key).collect();
    let ops = lcs_ops(&keys_a, &keys_b, ctx.budget);

    let mut out_rows = Vec::new();
    let mut pending_ins = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut k = 0;
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                let (ra, rb) = (ta.rows.get(i), tb.rows.get(j));
                if let (Some(ra), Some(rb)) = (ra, rb) {
                    if same_shape(ra, rb) {
                        out_rows.append(&mut pending_ins);
                        out_rows.push(diff_row(ctx, ra, rb, depth));
                    } else {
                        out_rows.push(mark_row(ctx, ra, false, depth));
                        pending_ins.push(mark_row(ctx, rb, true, depth));
                        *ctx.changes += 2;
                    }
                } else if let Some(r) = rb.or(ra) {
                    out_rows.append(&mut pending_ins);
                    out_rows.push(r.clone());
                }
                i += 1;
                j += 1;
                k += 1;
            }
            Some(_) => {
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                for x in 0..dels.max(inss) {
                    let (ra, rb) = (if x < dels { ta.rows.get(i + x) } else { None }, if x < inss { tb.rows.get(j + x) } else { None });
                    match (ra, rb) {
                        (Some(ra), Some(rb)) if same_shape(ra, rb) => {
                            out_rows.append(&mut pending_ins);
                            out_rows.push(diff_row(ctx, ra, rb, depth));
                        }
                        (Some(ra), Some(rb)) => {
                            out_rows.push(mark_row(ctx, ra, false, depth));
                            pending_ins.push(mark_row(ctx, rb, true, depth));
                            *ctx.changes += 2;
                        }
                        (Some(ra), None) => {
                            out_rows.push(mark_row(ctx, ra, false, depth));
                            *ctx.changes += 1;
                        }
                        (None, Some(rb)) => {
                            pending_ins.push(mark_row(ctx, rb, true, depth));
                            *ctx.changes += 1;
                        }
                        (None, None) => {}
                    }
                }
                i += dels;
                j += inss;
                k += run.max(1);
            }
            None => break,
        }
    }
    out_rows.append(&mut pending_ins);

    let mut tbl = Table { props: tb.props.clone(), grid: tb.grid.clone(), rows: out_rows, controls: tb.controls.clone() };
    if ctx.opts.formatting && crate::cmd::fmt_revisions::compare_table_props(&ta.props, &mut tbl.props, ctx.revisions, ctx.author, ctx.date) {
        *ctx.changes += 1;
    }
    tbl
}

/// Returns true if two rows share the same column/cell structure (cell count, grid gaps,
/// and per-cell horizontal spans and vertical merge modes).
fn same_shape(ra: &Row, rb: &Row) -> bool {
    ra.cells.len() == rb.cells.len()
        && ra.props.grid_before == rb.props.grid_before
        && ra.props.grid_after == rb.props.grid_after
        && ra.cells.iter().zip(&rb.cells).all(|(ca, cb)| ca.props.span == cb.props.span && ca.props.vmerge == cb.props.vmerge)
}

/// A copy of the row marked as inserted (ins=true) or deleted (ins=false).
fn mark_row(ctx: &CompareCtx<'_>, r: &Row, ins: bool, depth: usize) -> Row {
    let mut row = r.clone();
    let rev_id = if ins { ctx.ins } else { ctx.del };
    if ins {
        row.props.ins = Some(rev_id);
    } else {
        row.props.del = Some(rev_id);
    }
    let mark_fn: &dyn Fn(&mut CharProps) = if ins { &|cp| cp.ins = Some(rev_id) } else { &|cp| cp.del = Some(rev_id) };
    for c in &mut row.cells {
        c.blocks = c.blocks.iter().map(|b| Arc::new(mark_block(b, mark_fn, depth + 1))).collect();
    }
    row
}

/// Diff two corresponding rows of a table cell by cell.
fn diff_row(ctx: &mut CompareCtx<'_>, ra: &Row, rb: &Row, depth: usize) -> Row {
    let mut row = rb.clone();
    if ctx.opts.formatting && crate::cmd::fmt_revisions::compare_row_props(&ra.props, &mut row.props, ctx.revisions, ctx.author, ctx.date) {
        *ctx.changes += 1;
    }
    let mut cells = Vec::with_capacity(rb.cells.len());
    let min_len = ra.cells.len().min(rb.cells.len());
    for (ca, cb) in ra.cells.iter().zip(&rb.cells) {
        let mut cell = cb.clone();
        if ctx.opts.formatting && crate::cmd::fmt_revisions::compare_cell_props(&ca.props, &mut cell.props, ctx.revisions, ctx.author, ctx.date) {
            *ctx.changes += 1;
        }
        cell.blocks = diff_blocks(ctx, &ca.blocks, &cb.blocks, depth + 1);
        cells.push(cell);
    }
    for src in rb.cells.iter().skip(min_len) {
        let mut cb = src.clone();
        cb.blocks = cb.blocks.iter().map(|b| Arc::new(mark_block(b, &|cp| cp.ins = Some(ctx.ins), depth + 1))).collect();
        cells.push(cb);
        *ctx.changes += 1;
    }
    for src in ra.cells.iter().skip(min_len) {
        let mut ca = src.clone();
        ca.blocks = ca.blocks.iter().map(|b| Arc::new(mark_block(b, &|cp| cp.del = Some(ctx.del), depth + 1))).collect();
        cells.push(ca);
        *ctx.changes += 1;
    }
    row.cells = cells;
    row
}

/// A row's text: cell texts concatenated, separated by tabs.
fn row_text(r: &Row, depth: usize) -> String {
    if depth >= MAX_TABLE_DEPTH {
        return String::new();
    }
    r.cells.iter().flat_map(|c| c.blocks.iter()).map(|b| block_text(b, depth + 1)).collect::<Vec<_>>().join("\t")
}

/// A block's text (a table's cells in order), for matching blocks.
fn block_text(b: &Block, depth: usize) -> String {
    match b {
        Block::Para(p) => p.plain_text(),
        Block::Table(t) if depth < MAX_TABLE_DEPTH => {
            t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|x| block_text(x, depth + 1)).collect::<Vec<_>>().join("\t")
        }
        Block::Table(_) => String::new(),
    }
}

/// Returns a copy of `b` with all text runs, paragraph marks and table rows/cells updated by `f`.
fn mark_block(b: &Block, f: &dyn Fn(&mut CharProps), depth: usize) -> Block {
    match b {
        Block::Para(p) => {
            let mut p = p.clone();
            let len = p.len();
            let _ = p.format(0, len, f);
            f(&mut p.mark);
            Block::Para(p)
        }
        Block::Table(t) => {
            let mut t = t.clone();
            let mut probe = CharProps::default();
            f(&mut probe);
            for r in &mut t.rows {
                if probe.ins.is_some() {
                    r.props.ins = probe.ins;
                }
                if probe.del.is_some() {
                    r.props.del = probe.del;
                }
                if depth < MAX_TABLE_DEPTH {
                    for c in &mut r.cells {
                        c.blocks = c.blocks.iter().map(|x| Arc::new(mark_block(x, f, depth + 1))).collect();
                    }
                }
            }
            Block::Table(t)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Op {
    Same,
    Del,
    Ins,
}

/// Longest sequence window per LCS comparison table chunk; sequences longer than this are
/// aligned in successive chunked windows rather than truncated, preserving tail alignment.
const MAX_LCS: usize = 4000;

/// Comparison table cells one compare may fill over all its tables (blocks and every changed
/// paragraph pair); once spent, the remaining middles are compared as deleted and inserted.
const MAX_COMPARE_CELLS: u64 = 50_000_000;

/// Computes the longest common subsequence (LCS) edit script (`Op::Same`, `Op::Del`, `Op::Ins`)
/// between sequences `a` and `b`.
///
/// Strips common prefix and suffix first, then iterates over windows capped at [`MAX_LCS`],
/// charging comparison cells against `budget`. Between windows, intermediate common prefixes
/// are trimmed to re-anchor alignment without table allocations. When the remaining cells
/// exceed `budget`, the unaligned tail degrades honestly as whole deletions and insertions.
pub(super) fn lcs_ops<T: PartialEq>(a: &[T], b: &[T], budget: &mut u64) -> Vec<Op> {
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (ra, rb) = (a.get(pre..).unwrap_or_default(), b.get(pre..).unwrap_or_default());
    let suf = ra.iter().rev().zip(rb.iter().rev()).take_while(|(x, y)| x == y).count();
    let mut rem_a = ra.get(..ra.len().saturating_sub(suf)).unwrap_or_default();
    let mut rem_b = rb.get(..rb.len().saturating_sub(suf)).unwrap_or_default();
    let mut ops = vec![Op::Same; pre];

    while !rem_a.is_empty() && !rem_b.is_empty() {
        let p = rem_a.iter().zip(rem_b).take_while(|(x, y)| x == y).count();
        if p > 0 {
            ops.extend(std::iter::repeat_n(Op::Same, p));
            rem_a = rem_a.get(p..).unwrap_or_default();
            rem_b = rem_b.get(p..).unwrap_or_default();
            if rem_a.is_empty() || rem_b.is_empty() {
                break;
            }
        }

        let n = rem_a.len().min(MAX_LCS);
        let m = rem_b.len().min(MAX_LCS);
        let cells = (n as u64).saturating_mul(m as u64);
        if cells > *budget || n == 0 || m == 0 {
            break;
        }
        *budget = budget.saturating_sub(cells);

        let stride = m.saturating_add(1);
        let table_len = n.saturating_add(1).saturating_mul(stride);
        let mut t = vec![0u32; table_len];

        for i in (0..n).rev() {
            let row = i.saturating_mul(stride);
            let next_row = i.saturating_add(1).saturating_mul(stride);
            for j in (0..m).rev() {
                let v = if rem_a.get(i) == rem_b.get(j) {
                    t.get(next_row.saturating_add(j).saturating_add(1)).copied().unwrap_or(0).saturating_add(1)
                } else {
                    let down = t.get(next_row.saturating_add(j)).copied().unwrap_or(0);
                    let right = t.get(row.saturating_add(j).saturating_add(1)).copied().unwrap_or(0);
                    down.max(right)
                };
                if let Some(c) = t.get_mut(row.saturating_add(j)) {
                    *c = v;
                }
            }
        }

        let (mut i, mut j) = (0usize, 0usize);
        while i < n && j < m {
            let row = i.saturating_mul(stride);
            let next_row = i.saturating_add(1).saturating_mul(stride);
            if rem_a.get(i) == rem_b.get(j) {
                ops.push(Op::Same);
                i = i.saturating_add(1);
                j = j.saturating_add(1);
            } else {
                let down = t.get(next_row.saturating_add(j)).copied().unwrap_or(0);
                let right = t.get(row.saturating_add(j).saturating_add(1)).copied().unwrap_or(0);
                if down >= right {
                    ops.push(Op::Del);
                    i = i.saturating_add(1);
                } else {
                    ops.push(Op::Ins);
                    j = j.saturating_add(1);
                }
            }
        }

        rem_a = rem_a.get(i..).unwrap_or_default();
        rem_b = rem_b.get(j..).unwrap_or_default();
    }

    ops.extend(std::iter::repeat_n(Op::Del, rem_a.len()));
    ops.extend(std::iter::repeat_n(Op::Ins, rem_b.len()));
    ops.extend(std::iter::repeat_n(Op::Same, suf));
    ops
}

/// How short an interior match can be, inside a run of edits, before it's folded into the
/// surrounding delete/insert instead of staying its own little island of "same" text. Two short
/// strings that happen to share a character in a different order (common with digits in dates,
/// IDs, phone numbers) otherwise come out of `lcs_ops` as a true shortest edit script, but render
/// as a "ransom note" of alternating single-character deletions and insertions where a reader
/// (and Word) would expect one clean replacement.
const NOISE_MATCH_LEN: usize = 1;

/// Folds single-token matches that sit strictly inside a run of edits into the surrounding
/// delete/insert, so a character-level diff doesn't fragment a replaced run. A matched run that
/// opens or closes the whole sequence is the genuine common prefix/suffix `lcs_ops` already
/// stripped before running its table, so it's left untouched regardless of length; only a short
/// match produced by the table's own backtracking, which can only ever be interior, is a
/// candidate.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Keep,
    Fold,
}

pub(super) fn collapse_noise_matches(ops: &[Op]) -> Vec<Op> {
    let mut runs: Vec<(Op, usize, Kind)> = Vec::new();
    let mut k = 0;
    while k < ops.len() {
        let op = ops[k];
        let len = ops[k..].iter().take_while(|o| **o == op).count();
        let kind = if op == Op::Same && k != 0 && k + len != ops.len() && len <= NOISE_MATCH_LEN { Kind::Fold } else { Kind::Keep };
        runs.push((op, len, kind));
        k += len;
    }
    if !runs.iter().any(|(_, _, kind)| *kind == Kind::Fold) {
        return ops.to_vec();
    }
    let mut out = Vec::with_capacity(ops.len());
    let mut i = 0;
    while i < runs.len() {
        if runs[i].0 == Op::Same && runs[i].2 == Kind::Keep {
            out.extend(std::iter::repeat_n(Op::Same, runs[i].1));
            i += 1;
            continue;
        }
        let start = i;
        while i < runs.len() && !(runs[i].0 == Op::Same && runs[i].2 == Kind::Keep) {
            i += 1;
        }
        let region = &runs[start..i];
        if region.iter().any(|(_, _, kind)| *kind == Kind::Fold) {
            flatten_region(&mut out, region);
        } else {
            for (op, len, _) in region {
                out.extend(std::iter::repeat_n(*op, *len));
            }
        }
    }
    out
}

/// Flattens a region with a folded run: every original character first (in its own order), then
/// every revised character (in its own order) — one clean delete, one clean insert.
fn flatten_region(out: &mut Vec<Op>, region: &[(Op, usize, Kind)]) {
    let dels: usize = region.iter().map(|(op, len, kind)| if *kind == Kind::Fold || *op == Op::Del { *len } else { 0 }).sum();
    let inss: usize = region.iter().map(|(op, len, kind)| if *kind == Kind::Fold || *op == Op::Ins { *len } else { 0 }).sum();
    out.extend(std::iter::repeat_n(Op::Del, dels));
    out.extend(std::iter::repeat_n(Op::Ins, inss));
}

/// A token of a changed paragraph together with its starting character formatting: a word (or
/// character), or an inline object without text of its own (a text box anchor, a picture, a
/// bookmark…), which is compared and kept whole.
enum Tok<'a> {
    Text(String, CharProps),
    Obj(&'a InlineObject, CharProps),
}

/// What a comparison sees of a token: the text as the options say, an object with the text of
/// the text boxes it shows instead of their story ids (each document numbers its own stories).
#[derive(PartialEq)]
enum Key {
    Text(String),
    Obj(Box<InlineObject>, Vec<String>),
}

fn obj_key(doc: &Document, o: &InlineObject) -> Key {
    let texts = o.text_boxes().into_iter().map(|id| doc.plain_text(StoryRef::Part(id))).collect();
    let mut o = o.clone();
    for story in o.text_box_slots() {
        *story = None;
    }
    Key::Obj(Box::new(o), texts)
}

/// Whether two paragraphs hold the same objects, as compared.
fn same_objects(doc: &Document, a: &Paragraph, b: &Paragraph) -> bool {
    a.objects.len() == b.objects.len() && a.objects.iter().zip(&b.objects).all(|(x, y)| x == y || obj_key(doc, x) == obj_key(doc, y))
}

/// Words with the spaces after them ("The ", "cat "), or single characters, and the objects
/// between them, each paired with its character formatting. Objects with text (fields, equations…)
/// count as their text.
fn tokens(p: &Paragraph, characters: bool) -> Vec<Tok<'_>> {
    /// Push `c` into the active word token (recording its opening `props` if starting one) and
    /// emit the finished token once a word boundary or character mode finishes it.
    fn push(v: &mut Vec<Tok<'_>>, cur: &mut String, cur_props: &mut Option<CharProps>, c: char, props: &CharProps, characters: bool) {
        if cur.is_empty() {
            *cur_props = Some(props.clone());
        }
        cur.push(c);
        if characters || c == ' ' {
            v.push(Tok::Text(std::mem::take(cur), cur_props.take().unwrap_or_default()));
        }
    }
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut cur_props = None;
    let mut objects = p.objects.iter();
    for (byte_idx, c) in p.text.char_indices() {
        let props = p.props_of_char(byte_idx);
        if c != OBJ {
            push(&mut v, &mut cur, &mut cur_props, c, props, characters);
            continue;
        }
        match objects.next() {
            Some(o) if o.plain_text().is_empty() => {
                if !cur.is_empty() {
                    v.push(Tok::Text(std::mem::take(&mut cur), cur_props.take().unwrap_or_default()));
                }
                v.push(Tok::Obj(o, props.clone()));
            }
            Some(o) => o.plain_text().chars().for_each(|ch| push(&mut v, &mut cur, &mut cur_props, ch, props, characters)),
            None => {
                // A marker without its object (a broken paragraph): kept as a visible
                // placeholder, so the loss shows in the result instead of vanishing.
                log::warn!("compare: an object marker without its object");
                push(&mut v, &mut cur, &mut cur_props, char::REPLACEMENT_CHARACTER, props, characters);
            }
        }
    }
    if !cur.is_empty() {
        v.push(Tok::Text(cur, cur_props.unwrap_or_default()));
    }
    v
}

/// Add a token at the end of `p`, mapping its character formatting through `map_props`.
fn put(p: &mut Paragraph, t: Option<&Tok<'_>>, map_props: impl FnOnce(&CharProps) -> CharProps) {
    let at = p.len();
    match t {
        Some(Tok::Text(t, props)) => {
            let _ = p.insert_text(at, t, &map_props(props));
        }
        Some(Tok::Obj(o, props)) => {
            let _ = p.insert_object(at, (*o).clone(), &map_props(props));
        }
        None => {}
    }
}

/// Returns `true` if paragraph `p` contains more characters than `limit`.
///
/// Bounded to scan at most `limit + 1` characters without allocating unless inline objects
/// are present, avoiding O(n) traversal of large paragraphs.
fn para_exceeds_char_limit(p: &Paragraph, limit: usize) -> bool {
    if p.objects.is_empty() {
        p.text.chars().take(limit.saturating_add(1)).count() > limit
    } else {
        p.plain_text().chars().take(limit.saturating_add(1)).count() > limit
    }
}

/// Diffs two matched paragraphs either at word granularity or at character granularity according
/// to `ctx.opts.characters`. When character-level diff is requested but either paragraph exceeds
/// [`MAX_CHAR_DIFF`] characters, falls back to word-level diff and increments `ctx.char_fallbacks`
/// so callers can discover the fallback.
fn word_diff(doc: &Document, a: &Paragraph, b: &Paragraph, ctx: &mut DiffCtx<'_>) -> Paragraph {
    let chars = if ctx.opts.characters {
        if para_exceeds_char_limit(a, MAX_CHAR_DIFF) || para_exceeds_char_limit(b, MAX_CHAR_DIFF) {
            *ctx.char_fallbacks = ctx.char_fallbacks.saturating_add(1);
            false
        } else {
            true
        }
    } else {
        false
    };
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let key = |t: &Tok<'_>| match t {
        Tok::Text(t, _) => Key::Text(ctx.opts.key(t)),
        Tok::Obj(o, _) => obj_key(doc, o),
    };
    let ops = lcs_ops(&ta.iter().map(key).collect::<Vec<_>>(), &tb.iter().map(key).collect::<Vec<_>>(), ctx.budget);
    // Only at character granularity: a matched whole word almost never happens by coincidence,
    // but a matched single character very often does (digits, punctuation), so only there does
    // a short interior match need folding back into its surrounding edit.
    let ops = if chars { collapse_noise_matches(&ops) } else { ops };
    let mut p = Paragraph::new();
    p.props = b.props.clone();
    let (mut i, mut j) = (0, 0);
    for op in ops {
        match op {
            Op::Same => {
                // Equal as compared (perhaps up to case or spacing): the revised text and its formatting.
                put(&mut p, tb.get(j), Clone::clone);
                i += 1;
                j += 1;
            }
            Op::Del => {
                put(&mut p, ta.get(i), |pr| CharProps { del: Some(ctx.del), ..pr.clone() });
                i += 1;
            }
            Op::Ins => {
                put(&mut p, tb.get(j), |pr| CharProps { ins: Some(ctx.ins), ..pr.clone() });
                j += 1;
            }
        }
    }
    p
}
