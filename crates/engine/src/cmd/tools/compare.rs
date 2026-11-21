use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::{
    PARA_WALK_DEPTH, Row, Table,
    para::OBJ,
    props::{CellProps, RowProps, TableFloat, TableProps},
    table::MAX_ROWS,
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
    plain_revised: bool,
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
    // A revised side given as plain text carries no real runs or properties; one opened from a
    // path does. The diff takes the revised formatting only in the latter case.
    let (revised, plain_revised) = if let Some(t) = p::str(v, "text") {
        (Document::from_text(t), true)
    } else if let Some(path) = p::str(v, "path") {
        (crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?, false)
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
    let out = {
        let mut ctx = CompareCtx {
            doc: &mut s.doc,
            ins,
            del,
            opts: &opts,
            budget: &mut budget,
            char_fallbacks: &mut char_fallbacks,
            plain_revised,
            changes: &mut changes,
            author: &author,
            date: &date,
        };
        diff_blocks(&mut ctx, &old, &new, 0)
    };
    s.doc.body = out;
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

/// Shared state and counters threaded through block, table and row comparison.
struct CompareCtx<'a> {
    doc: &'a mut Document,
    ins: u32,
    del: u32,
    opts: &'a CompareOptions,
    budget: &'a mut u64,
    char_fallbacks: &'a mut usize,
    plain_revised: bool,
    changes: &'a mut usize,
    author: &'a str,
    date: &'a str,
}

impl CompareCtx<'_> {
    /// Diff two matched paragraphs using the shared options, budget and fallback counter.
    fn word_diff_para(&mut self, pa: &Paragraph, pb: &Paragraph) -> Paragraph {
        let mut dctx = DiffCtx {
            ins: self.ins,
            del: self.del,
            opts: self.opts,
            budget: self.budget,
            char_fallbacks: self.char_fallbacks,
            plain_revised: self.plain_revised,
        };
        word_diff(self.doc, pa, pb, &mut dctx)
    }
}

/// Diff two block sequences (the document body or a table cell's blocks), pairing matching
/// paragraphs and tables recursively and marking unpaired blocks as inserted or deleted.
fn diff_blocks(ctx: &mut CompareCtx<'_>, old: &[Arc<Block>], new: &[Arc<Block>], depth: usize) -> Vec<Arc<Block>> {
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => (false, ctx.opts.key(&p.plain_text())),
        // Tables match by their text.
        Block::Table(_) => (true, ctx.opts.key(&block_text(b, depth))),
    };
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), ctx.budget);
    let mut out: Vec<Arc<Block>> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut k = 0;
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                let (a, b) = (old.get(i), new.get(j));
                // The revised block, with its source revision indices stripped and formatting
                // changes tracked against `ctx.doc.revisions`.
                if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    if !same_objects(ctx.doc, pa, pb) {
                        // Same text, but objects (text boxes, pictures…) differ: diffed, so the
                        // original's objects aren't lost.
                        out.push(para_block(ctx.word_diff_para(pa, pb)));
                        *ctx.changes += 1;
                        i += 1;
                        j += 1;
                        k += 1;
                        continue;
                    }
                    let stripped = strip_para_revs(pb);
                    let mut p = stripped.clone();
                    if ctx.opts.formatting && pa.text == pb.text {
                        let before = ctx.doc.revisions.len();
                        crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, &mut ctx.doc.revisions, ctx.author, ctx.date);
                        if ctx.doc.revisions.len() > before || p != stripped {
                            *ctx.changes += 1;
                        }
                    }
                    out.push(para_block(p));
                } else if let (Some(ab), Some(bb)) = (a, b)
                    && let (Block::Table(ta), Block::Table(tb)) = (&**ab, &**bb)
                {
                    diff_tables(ctx, &mut out, ab, ta, bb, tb, depth);
                } else if let Some(x) = b {
                    out.push(clean_revised_block(x));
                } else if let Some(x) = a {
                    out.push(x.clone());
                }
                i += 1;
                j += 1;
                k += 1;
            }
            Some(_) => {
                // A run of deletions and insertions: changed paragraphs or tables pair up and are
                // diffed in place; the rest are deleted or inserted whole.
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run.saturating_sub(dels);
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
                            let mut emitted = false;
                            if let Some(x) = a
                                && !(depth >= PARA_WALK_DEPTH && x.as_table().is_some())
                            {
                                out.push(Arc::new(mark_block(x, &|c| c.del = Some(ctx.del), depth)));
                                emitted = true;
                            }
                            if let Some(x) = b {
                                if depth >= PARA_WALK_DEPTH && x.as_table().is_some() {
                                    out.push(clean_revised_block(x));
                                } else {
                                    out.push(Arc::new(mark_inserted_block(x, ctx.ins, depth)));
                                    emitted = true;
                                }
                            }
                            if emitted {
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

/// Compare two paired table blocks: keep equal tables untouched, diff matching-row tables cell by
/// cell via [`table_diff`], and fall back to whole-table replacement when grid, size, nesting
/// depth, row structure or container properties prevent a lossless in-place cell diff.
fn diff_tables(ctx: &mut CompareCtx<'_>, out: &mut Vec<Arc<Block>>, a: &Arc<Block>, ta: &Table, b: &Arc<Block>, tb: &Table, depth: usize) {
    // Cells inside a table at `PARA_WALK_DEPTH` or deeper sit beyond `Document::para_paths`, so
    // `mark_block` leaves them unmarked and `Accept All` / `Reject All` never visit them; keep the
    // stripped revised table once rather than emitting two unmarked copies.
    if ta == tb || depth >= PARA_WALK_DEPTH {
        out.push(clean_revised_block(b));
        return;
    }
    if same_grid(&ta.grid, &tb.grid)
        && ta.rows.len() == tb.rows.len()
        && tb.rows.len() <= MAX_ROWS
        && let Some(tbl) = table_diff(ctx, ta, tb, depth)
    {
        out.push(Arc::new(Block::Table(tbl)));
        return;
    }
    out.push(Arc::new(mark_block(a, &|c| c.del = Some(ctx.del), depth)));
    out.push(Arc::new(mark_inserted_block(b, ctx.ins, depth)));
    *ctx.changes += 1;
}

/// Whether `p` carries any revision bookkeeping (`ins`, `del`, `fmt_change`, `num_change`).
fn para_has_revs(p: &Paragraph) -> bool {
    p.props.fmt_change.is_some()
        || p.props.num_change.is_some()
        || p.mark.ins.is_some()
        || p.mark.del.is_some()
        || p.mark.fmt_change.is_some()
        || p.section.as_ref().is_some_and(|s| s.fmt_change.is_some())
        || p.runs.iter().any(|run| run.props.ins.is_some() || run.props.del.is_some() || run.props.fmt_change.is_some())
}

/// A copy of `pb` with source-document revision indices cleared across its properties, mark, runs
/// and section break so foreign indices never leak into `ctx.doc.revisions`.
fn strip_para_revs(pb: &Paragraph) -> Paragraph {
    if !para_has_revs(pb) {
        return pb.clone();
    }
    let mut p = pb.clone();
    p.props = p.props.formatting();
    p.mark = CharProps { ins: None, del: None, fmt_change: None, ..p.mark };
    for r in &mut p.runs {
        r.props.ins = None;
        r.props.del = None;
        r.props.fmt_change = None;
    }
    if let Some(sec) = p.section.as_deref_mut() {
        sec.fmt_change = None;
    }
    p.normalize();
    p
}

/// Maximum table nesting stripped recursively (`MAX_TABLE_DEPTH` is 18; `.docx` XML depth caps
/// nested `<w:tbl>` well below 100). Any deeper subtree is cleared to an empty paragraph so
/// foreign revision indices can never survive into the result document.
const MAX_STRIP_DEPTH: usize = 128;

/// Whether `b` (or any nested block at any depth) carries revision indices from its source
/// document, checked iteratively so deep tables never blow the stack.
fn block_has_revs(b: &Block) -> bool {
    let mut stack = vec![b];
    while let Some(cur) = stack.pop() {
        match cur {
            Block::Para(p) => {
                if para_has_revs(p) {
                    return true;
                }
            }
            Block::Table(t) => {
                if t.props.fmt_change.is_some() {
                    return true;
                }
                for r in &t.rows {
                    if r.props.fmt_change.is_some() || r.props.exceptions.as_ref().is_some_and(|e| e.fmt_change.is_some()) {
                        return true;
                    }
                    for c in &r.cells {
                        if c.props.fmt_change.is_some() {
                            return true;
                        }
                        stack.extend(c.blocks.iter().map(|x| &**x));
                    }
                }
            }
        }
    }
    false
}

/// Returns `b` with source-document revision indices stripped across all supported table depths,
/// sharing the `Arc` unchanged when `b` carries none.
fn clean_revised_block(b: &Arc<Block>) -> Arc<Block> {
    if block_has_revs(b) { Arc::new(strip_block_revs(b, 0)) } else { b.clone() }
}

/// Diff two tables row by row when every row pairs 1-to-1 with matching cell structure and
/// container properties.
///
/// Until `w:trPr` row insertion/deletion marks (`w:ins` / `w:del`) land in the shared DOCX model,
/// added, removed, reordered or structurally reshaped rows return `None` so the caller replaces
/// the table whole rather than emitting row marks that a `.docx` save would drop. Likewise, when
/// table, row or cell properties differ (unless `formatting` is off and no text/structural edit
/// occurred), returns `None` so Reject All can still restore the original properties.
fn table_diff(ctx: &mut CompareCtx<'_>, ta: &Table, tb: &Table, depth: usize) -> Option<Table> {
    let props_match = same_table_props(ta, tb) && ta.rows.iter().zip(&tb.rows).all(|(ra, rb)| same_row_props(ra, rb));
    if !props_match && ctx.opts.formatting {
        return None;
    }

    let row_key = |r: &Row| ctx.opts.key(&row_text(r, depth));
    let keys_a: Vec<String> = ta.rows.iter().map(row_key).collect();
    let keys_b: Vec<String> = tb.rows.iter().map(row_key).collect();
    let ops = lcs_ops(&keys_a, &keys_b, ctx.budget);

    let mut pairs: Vec<(&Row, &Row)> = Vec::with_capacity(tb.rows.len());
    let (mut i, mut j, mut k) = (0usize, 0usize, 0usize);
    while let Some(&op) = ops.get(k) {
        if op == Op::Same {
            let (ra, rb) = (ta.rows.get(i)?, tb.rows.get(j)?);
            if !same_shape(ra, rb) {
                return None;
            }
            pairs.push((ra, rb));
            i += 1;
            j += 1;
            k += 1;
        } else {
            if !props_match {
                return None;
            }
            let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
            let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
            let inss = run.saturating_sub(dels);
            if dels != inss {
                return None;
            }
            for x in 0..dels {
                let (ra, rb) = (ta.rows.get(i + x)?, tb.rows.get(j + x)?);
                if !same_shape(ra, rb) {
                    return None;
                }
                pairs.push((ra, rb));
            }
            i += dels;
            j += inss;
            k += run.max(1);
        }
    }

    let before_changes = *ctx.changes;
    let before_fallbacks = *ctx.char_fallbacks;
    let out_rows: Vec<Row> = pairs.into_iter().map(|(ra, rb)| diff_row(ctx, ra, rb, depth)).collect();
    if !props_match && *ctx.changes > before_changes {
        *ctx.changes = before_changes;
        *ctx.char_fallbacks = before_fallbacks;
        return None;
    }
    Some(Table { props: strip_table_props(&tb.props), grid: tb.grid.clone(), rows: out_rows, controls: tb.controls.clone() })
}

/// Diff two shape-matched rows cell by cell, stripping any source `fmt_change` indices from the
/// adopted revised row and cell properties.
fn diff_row(ctx: &mut CompareCtx<'_>, ra: &Row, rb: &Row, depth: usize) -> Row {
    let mut cells = Vec::with_capacity(rb.cells.len());
    for (ca, cb) in ra.cells.iter().zip(&rb.cells) {
        cells.push(wordcraft_doc::table::Cell {
            props: CellProps { fmt_change: None, ..cb.props.clone() },
            blocks: diff_blocks(ctx, &ca.blocks, &cb.blocks, depth + 1),
            controls: cb.controls.clone(),
        });
    }
    Row { props: strip_row_props(&rb.props), cells, controls: rb.controls.clone() }
}

fn strip_table_props(t: &TableProps) -> TableProps {
    TableProps { fmt_change: None, ..t.clone() }
}

fn strip_row_props(r: &RowProps) -> RowProps {
    let mut out = RowProps { fmt_change: None, ..r.clone() };
    if let Some(ex) = out.exceptions.as_deref_mut() {
        ex.fmt_change = None;
    }
    out
}

/// Round a point measurement to the nearest OOXML twip (`1/20` pt) for comparison across `.docx`
/// serialization.
fn norm_twip(v: f32) -> f32 {
    if v.is_finite() { ((f64::from(v) * 20.0).round() / 20.0) as f32 } else { v }
}

/// Round a percentage measurement to the nearest OOXML fiftieth of a percent (`1/50`%) as written
/// by `w:tblW` / `w:tcW` (`w:type="pct"`).
fn norm_pct(v: f32) -> f32 {
    if v.is_finite() { ((f64::from(v.clamp(0.0, 1000.0)) * 50.0).round() / 50.0) as f32 } else { v }
}

/// Whether two point measurements serialize to the same OOXML twip integer (`1/20` pt).
fn same_twips(a: f32, b: f32) -> bool {
    a == b || (a.is_finite() && b.is_finite() && (f64::from(a) * 20.0).round() == (f64::from(b) * 20.0).round())
}

/// Whether two optional point measurements serialize to the same OOXML twip integer.
fn same_opt_twips(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => same_twips(x, y),
        _ => false,
    }
}

/// Whether two optional percentage measurements serialize to the same OOXML `pct` integer.
fn same_opt_pct(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y || (x.is_finite() && y.is_finite() && norm_pct(x) == norm_pct(y)),
        _ => false,
    }
}

/// Whether two table column grids have the same column count and twip widths.
fn same_grid(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| same_twips(x, y))
}

/// Normalize `TableProps` across `.docx` twip/percent serialization, ignoring `fmt_change`.
fn norm_table_props(t: &TableProps) -> TableProps {
    TableProps {
        width: t.width.map(norm_twip),
        width_pct: t.width_pct.map(norm_pct),
        indent: t.indent.map(norm_twip),
        cell_margins: t.cell_margins.map(|m| m.map(norm_twip)),
        float: t.float.map(|f| TableFloat { x: norm_twip(f.x), y: norm_twip(f.y), dist: f.dist.map(norm_twip), ..f }),
        space_before: t.space_before.map(norm_twip),
        space_after: t.space_after.map(norm_twip),
        fmt_change: None,
        ..t.clone()
    }
}

/// Normalize `RowProps` across `.docx` twip serialization, ignoring `fmt_change`.
fn norm_row_props(r: &RowProps) -> RowProps {
    RowProps {
        height: r.height.map(norm_twip),
        exceptions: r.exceptions.as_deref().map(|e| Box::new(norm_table_props(e))),
        grid_before: r.grid_before.min(63),
        w_before: r.w_before.map(norm_twip),
        grid_after: r.grid_after.min(63),
        w_after: r.w_after.map(norm_twip),
        fmt_change: None,
        ..r.clone()
    }
}

/// Normalize `CellProps` across `.docx` twip/percent serialization, ignoring `fmt_change`.
fn norm_cell_props(c: &CellProps) -> CellProps {
    CellProps {
        width: c.width.map(norm_twip),
        width_pct: c.width_pct.map(norm_pct),
        span: c.span.clamp(1, 63),
        margins: c.margins.map(|m| m.map(norm_twip)),
        partial_margins: c.partial_margins.map(|pm| pm.map(|v| v.map(norm_twip))),
        fmt_change: None,
        ..c.clone()
    }
}

fn same_table_props(ta: &Table, tb: &Table) -> bool {
    ta.controls == tb.controls && norm_table_props(&ta.props) == norm_table_props(&tb.props)
}

fn same_row_props(ra: &Row, rb: &Row) -> bool {
    ra.controls == rb.controls
        && ra.cells.len() == rb.cells.len()
        && norm_row_props(&ra.props) == norm_row_props(&rb.props)
        && ra.cells.iter().zip(&rb.cells).all(|(ca, cb)| ca.controls == cb.controls && norm_cell_props(&ca.props) == norm_cell_props(&cb.props))
}

/// Whether two rows share the same column/cell geometry (cell count, grid gaps, cell widths,
/// horizontal spans and vertical merge modes).
fn same_shape(ra: &Row, rb: &Row) -> bool {
    ra.cells.len() == rb.cells.len()
        && ra.first_grid() == rb.first_grid()
        && same_opt_twips(ra.props.w_before, rb.props.w_before)
        && ra.grid_after() == rb.grid_after()
        && same_opt_twips(ra.props.w_after, rb.props.w_after)
        && ra.cells.iter().zip(&rb.cells).all(|(ca, cb)| {
            ca.span() == cb.span()
                && ca.props.vmerge == cb.props.vmerge
                && same_opt_twips(ca.props.width, cb.props.width)
                && same_opt_pct(ca.props.width_pct, cb.props.width_pct)
        })
}

/// A row's text: cell texts concatenated, separated by tabs.
fn row_text(r: &Row, depth: usize) -> String {
    if depth >= PARA_WALK_DEPTH {
        return String::new();
    }
    r.cells.iter().flat_map(|c| c.blocks.iter()).map(|b| block_text(b, depth + 1)).collect::<Vec<_>>().join("\t")
}

/// A block's text (a table's cells in order), for matching blocks.
fn block_text(b: &Block, depth: usize) -> String {
    match b {
        Block::Para(p) => p.plain_text(),
        Block::Table(t) if depth < PARA_WALK_DEPTH => {
            t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|x| block_text(x, depth + 1)).collect::<Vec<_>>().join("\t")
        }
        Block::Table(_) => String::new(),
    }
}

/// Returns a copy of `b` with source revision indices stripped across paragraphs and tables.
fn strip_block_revs(b: &Block, depth: usize) -> Block {
    match b {
        Block::Para(p) => Block::Para(strip_para_revs(p)),
        Block::Table(t) => {
            if !block_has_revs(b) {
                return Block::Table(t.clone());
            }
            let mut t = t.clone();
            t.props = strip_table_props(&t.props);
            for r in &mut t.rows {
                r.props = strip_row_props(&r.props);
                for c in &mut r.cells {
                    c.props.fmt_change = None;
                    if depth < MAX_STRIP_DEPTH {
                        c.blocks = c.blocks.iter().map(|x| Arc::new(strip_block_revs(x, depth + 1))).collect();
                    } else {
                        c.blocks = vec![para_block(Paragraph::new())];
                    }
                }
            }
            Block::Table(t)
        }
    }
}

/// Returns a copy of `b` (from the revised document) stripped of foreign revision indices and
/// marked inserted with `ins`.
fn mark_inserted_block(b: &Block, ins: u32, depth: usize) -> Block {
    let clean = strip_block_revs(b, 0);
    mark_block(&clean, &|c| c.ins = Some(ins), depth)
}

/// Returns a copy of `b` with all text runs and paragraph marks (or nested table cells) updated by `f`.
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
            if depth < PARA_WALK_DEPTH {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    c.blocks = c.blocks.iter().map(|x| Arc::new(mark_block(x, f, depth + 1))).collect();
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

/// A token of a changed paragraph: a word (or character), or an inline object without text
/// of its own (a text box anchor, a picture, a bookmark…), which is compared and kept whole.
/// Each token also carries the byte range it came from in its source paragraph, so the output
/// segment can take the formatting of the run it came from instead of the paragraph's first run.
enum Tok<'a> {
    Text(String, std::ops::Range<usize>),
    Obj(&'a InlineObject, std::ops::Range<usize>),
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
/// between them. Objects with text (fields, equations…) count as their text. Each token records
/// the byte range it came from in `p`, so its output segment can take that source run's
/// formatting. An object's flattened text (or a broken marker's placeholder) maps to the marker's
/// own range, which is the nearest formatting the object has.
fn tokens(p: &Paragraph, characters: bool) -> Vec<Tok<'_>> {
    fn push(v: &mut Vec<Tok<'_>>, cur: &mut String, start: usize, end: usize, c: char, characters: bool) {
        cur.push(c);
        if characters || c == ' ' {
            v.push(Tok::Text(std::mem::take(cur), start..end));
        }
    }
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut cur_start = 0usize;
    let mut objects = p.objects.iter();
    for (i, c) in p.text.char_indices() {
        let marker = i..i + OBJ.len_utf8();
        if c != OBJ {
            if cur.is_empty() {
                cur_start = i;
            }
            push(&mut v, &mut cur, cur_start, i + c.len_utf8(), c, characters);
            continue;
        }
        match objects.next() {
            Some(o) if o.plain_text().is_empty() => {
                if !cur.is_empty() {
                    v.push(Tok::Text(std::mem::take(&mut cur), cur_start..i));
                }
                v.push(Tok::Obj(o, marker));
            }
            Some(o) => {
                for c in o.plain_text().chars() {
                    if cur.is_empty() {
                        cur_start = i;
                    }
                    push(&mut v, &mut cur, cur_start, marker.end, c, characters);
                }
            }
            None => {
                // A marker without its object (a broken paragraph): kept as a visible
                // placeholder, so the loss shows in the result instead of vanishing.
                log::warn!("compare: an object marker without its object");
                if cur.is_empty() {
                    cur_start = i;
                }
                push(&mut v, &mut cur, cur_start, marker.end, char::REPLACEMENT_CHARACTER, characters);
            }
        }
    }
    if !cur.is_empty() {
        v.push(Tok::Text(cur, cur_start..p.text.len()));
    }
    v
}

/// One side's run formatting for `word_diff`: the paragraph's run ranges (from
/// [`Paragraph::run_ranges`]), walked with a forward cursor. `word_diff` consumes each side's
/// tokens in increasing offset order, so the cursor makes a token's lookup amortised O(1)
/// instead of rescanning the runs from the start for every token; an out-of-order offset falls
/// back to a binary search, so the lookup stays correct for any offset.
struct SideProps<'a> {
    ranges: Vec<(std::ops::Range<usize>, &'a CharProps)>,
    mark: &'a CharProps,
    cursor: usize,
}

impl<'a> SideProps<'a> {
    fn new(p: &'a Paragraph) -> Self {
        SideProps { ranges: p.run_ranges().collect(), mark: &p.mark, cursor: 0 }
    }

    /// The props of the run holding byte `off`: that run's props, or the paragraph mark when
    /// `off` is past the last run — the same answer `Paragraph::props_of_char` gives.
    fn run_props(&mut self, off: usize) -> &'a CharProps {
        while self.cursor < self.ranges.len() && self.ranges[self.cursor].0.end <= off {
            self.cursor += 1;
        }
        if let Some((range, props)) = self.ranges.get(self.cursor)
            && range.contains(&off)
        {
            return props;
        }
        let idx = self.ranges.partition_point(|(r, _)| r.start <= off);
        match idx.checked_sub(1).and_then(|i| self.ranges.get(i)) {
            Some((range, props)) if range.contains(&off) => props,
            _ => self.mark,
        }
    }

    /// The formatting of the source run a token came from: the run holding the token's first
    /// byte, with the revision metadata stripped. `ins`, `del` and `fmt_change` are indices into
    /// the *source* document's own revision table, and the rebuilt paragraph lives in the result
    /// document, whose table is a different one, so a source mark can never be carried across;
    /// `word_diff` stamps the compare's own `ins`/`del` on the segments it marks. The consequence
    /// is plain: a pre-existing pending revision inside a changed paragraph (say another author's
    /// deletion) is superseded by the compare result rather than preserved. The hyperlink is
    /// kept, because Reject restores it on a deleted run (at token granularity: the link is taken
    /// from the token's first byte, so a token that starts unlinked can lose a mid-token link).
    fn props_of(&mut self, t: &Tok<'_>) -> CharProps {
        let off = match t {
            Tok::Text(_, range) | Tok::Obj(_, range) => range.start,
        };
        let c = self.run_props(off);
        CharProps { ins: None, del: None, fmt_change: None, ..c.clone() }
    }
}

/// The props of an output token: those of the source run it came from, or — never expected, since
/// [`tokens`] and [`lcs_ops`] consume both sides in step — the given fallback. A miss is loud.
fn output_props(side: &mut SideProps<'_>, t: Option<&Tok<'_>>, fallback: &CharProps) -> CharProps {
    if t.is_none() {
        log::warn!("compare: a token range lookup missed; `tokens` and `lcs_ops` are out of step");
    }
    t.map(|t| side.props_of(t)).unwrap_or_else(|| fallback.clone())
}

/// Add a token at the end of `p` with `props`.
fn put(p: &mut Paragraph, t: Option<&Tok<'_>>, props: &CharProps) {
    let at = p.len();
    match t {
        Some(Tok::Text(t, _)) => {
            let _ = p.insert_text(at, t, props);
        }
        Some(Tok::Obj(o, _)) => {
            let _ = p.insert_object(at, (*o).clone(), props);
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
    let plain_revised = ctx.plain_revised;
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let (mut a_props, mut b_props) = (SideProps::new(a), SideProps::new(b));
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
    // The revised paragraph's properties are used with its revision bookkeeping stripped, like runs
    // (`SideProps::props_of`) and the mark, because `fmt_change`/`num_change` are indices into the revised
    // document's own revision table and are meaningless in the result document. A plain-text
    // revised side, which carries no paragraph properties of its own, falls back to the original's,
    // stripped the same way, so pre-existing revision state never leaks into the rebuilt paragraph.
    p.props = if plain_revised && b.props.is_empty() { a.props.formatting() } else { b.props.formatting() };
    // The paragraph mark (the formatting typing into the paragraph's end would get) is the revised
    // paragraph's when it comes from a document, and the original's when the revised side is plain
    // text and has no mark of its own. Its own revision bookkeeping (`ins`, `del`, `fmt_change`)
    // is cleared because those indices belong to another document's revision table.
    p.mark = if plain_revised {
        CharProps { ins: None, del: None, fmt_change: None, ..a.mark.clone() }
    } else {
        CharProps { ins: None, del: None, fmt_change: None, ..b.mark.clone() }
    };
    let fallback = a.runs.first().map(|r| CharProps { ins: None, del: None, fmt_change: None, ..r.props.clone() }).unwrap_or_default();
    let fallback_b = b.runs.first().map(|r| CharProps { ins: None, del: None, fmt_change: None, ..r.props.clone() }).unwrap_or_default();
    let (mut i, mut j) = (0, 0);
    for op in ops {
        match op {
            Op::Same => {
                // Equal as compared (perhaps up to case or spacing): the revised text, but the
                // formatting of the run it stands for. For a document revised side that is the
                // revised run, as issue #3749 prescribes ("Copy unchanged tokens from the revised
                // paragraph with their own properties"); a plain-text revised side has no runs to
                // speak of, so the original run's formatting is kept instead.
                let props =
                    if plain_revised { output_props(&mut a_props, ta.get(i), &fallback) } else { output_props(&mut b_props, tb.get(j), &fallback_b) };
                put(&mut p, tb.get(j), &props);
                i += 1;
                j += 1;
            }
            Op::Del => {
                let props = output_props(&mut a_props, ta.get(i), &fallback);
                put(&mut p, ta.get(i), &CharProps { del: Some(ctx.del), ..props });
                i += 1;
            }
            Op::Ins => {
                let props = tb.get(j).map(|t| b_props.props_of(t)).unwrap_or_else(|| fallback.clone());
                put(&mut p, tb.get(j), &CharProps { ins: Some(ctx.ins), ..props });
                j += 1;
            }
        }
    }
    p
}
