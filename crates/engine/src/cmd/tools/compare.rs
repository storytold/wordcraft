use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::para::OBJ;

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
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => opts.key(&p.plain_text()),
        // Tables match as a whole, by their text.
        Block::Table(_) => format!("\u{0}table\u{0}{}", opts.key(&block_text(b, 0))),
    };
    let mut budget = MAX_COMPARE_CELLS;
    let mut char_fallbacks = 0usize;
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), &mut budget);
    let mut ctx = DiffCtx { ins, del, opts: &opts, budget: &mut budget, char_fallbacks: &mut char_fallbacks };
    let mut out: Vec<Arc<Block>> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut changes = 0usize;
    let mut k = 0;
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                let (a, b) = (old.get(i), new.get(j));
                // The revised block, with its formatting changes tracked.
                if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    if !same_objects(&s.doc, pa, pb) {
                        // Same text, but objects (text boxes, pictures…) differ: diffed, so the
                        // original's objects aren't lost.
                        out.push(para_block(word_diff(&s.doc, pa, pb, &mut ctx)));
                        changes += 1;
                        i += 1;
                        j += 1;
                        k += 1;
                        continue;
                    }
                    let mut p = pb.clone();
                    if opts.formatting && pa.text == pb.text {
                        let before = s.doc.revisions.len();
                        crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, &mut s.doc.revisions, &author, &date);
                        if s.doc.revisions.len() > before || p != *pb {
                            changes += 1;
                        }
                    }
                    out.push(para_block(p));
                } else if let Some(x) = b.or(a) {
                    out.push(x.clone());
                }
                i += 1;
                j += 1;
                k += 1;
            }
            Some(_) => {
                // A run of deletions and insertions: changed paragraphs pair up and are diffed
                // word by word (or by character); the rest are deleted or inserted whole.
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                for x in 0..dels.max(inss) {
                    let (a, b) = (if x < dels { old.get(i + x) } else { None }, if x < inss { new.get(j + x) } else { None });
                    match (a.map(|x| &**x), b.map(|x| &**x)) {
                        (Some(Block::Para(pa)), Some(Block::Para(pb))) => out.push(para_block(word_diff(&s.doc, pa, pb, &mut ctx))),
                        _ => {
                            if let Some(x) = a {
                                out.push(Arc::new(mark_block(x, &|c| c.del = Some(del), 0)));
                            }
                            if let Some(x) = b {
                                out.push(Arc::new(mark_block(x, &|c| c.ins = Some(ins), 0)));
                            }
                        }
                    }
                    changes += 1;
                }
                i += dels;
                j += inss;
                k += run.max(1);
            }
            None => break,
        }
    }
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

/// A block's text (a table's cells in order), for matching blocks.
fn block_text(b: &Block, depth: usize) -> String {
    match b {
        Block::Para(p) => p.plain_text(),
        Block::Table(t) if depth < 16 => {
            t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|x| block_text(x, depth + 1)).collect::<Vec<_>>().join("\t")
        }
        Block::Table(_) => String::new(),
    }
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
            if depth < 16 {
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
enum Tok<'a> {
    Text(String),
    Obj(&'a InlineObject),
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
/// between them. Objects with text (fields, equations…) count as their text.
fn tokens(p: &Paragraph, characters: bool) -> Vec<Tok<'_>> {
    fn push(v: &mut Vec<Tok<'_>>, cur: &mut String, c: char, characters: bool) {
        cur.push(c);
        if characters || c == ' ' {
            v.push(Tok::Text(std::mem::take(cur)));
        }
    }
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut objects = p.objects.iter();
    for c in p.text.chars() {
        if c != OBJ {
            push(&mut v, &mut cur, c, characters);
            continue;
        }
        match objects.next() {
            Some(o) if o.plain_text().is_empty() => {
                if !cur.is_empty() {
                    v.push(Tok::Text(std::mem::take(&mut cur)));
                }
                v.push(Tok::Obj(o));
            }
            Some(o) => o.plain_text().chars().for_each(|c| push(&mut v, &mut cur, c, characters)),
            None => {
                // A marker without its object (a broken paragraph): kept as a visible
                // placeholder, so the loss shows in the result instead of vanishing.
                log::warn!("compare: an object marker without its object");
                push(&mut v, &mut cur, char::REPLACEMENT_CHARACTER, characters);
            }
        }
    }
    if !cur.is_empty() {
        v.push(Tok::Text(cur));
    }
    v
}

/// Add a token at the end of `p` with `props`.
fn put(p: &mut Paragraph, t: Option<&Tok<'_>>, props: &CharProps) {
    let at = p.len();
    match t {
        Some(Tok::Text(t)) => {
            let _ = p.insert_text(at, t, props);
        }
        Some(Tok::Obj(o)) => {
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
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let key = |t: &Tok<'_>| match t {
        Tok::Text(t) => Key::Text(ctx.opts.key(t)),
        Tok::Obj(o) => obj_key(doc, o),
    };
    let ops = lcs_ops(&ta.iter().map(key).collect::<Vec<_>>(), &tb.iter().map(key).collect::<Vec<_>>(), ctx.budget);
    // Only at character granularity: a matched whole word almost never happens by coincidence,
    // but a matched single character very often does (digits, punctuation), so only there does
    // a short interior match need folding back into its surrounding edit.
    let ops = if chars { collapse_noise_matches(&ops) } else { ops };
    let mut p = Paragraph::new();
    p.props = b.props.clone();
    let base = a.runs.first().map(|r| r.props.clone()).unwrap_or_default();
    let (mut i, mut j) = (0, 0);
    for op in ops {
        match op {
            Op::Same => {
                // Equal as compared (perhaps up to case or spacing): the revised text.
                put(&mut p, tb.get(j), &base);
                i += 1;
                j += 1;
            }
            Op::Del => {
                put(&mut p, ta.get(i), &CharProps { del: Some(ctx.del), ..base.clone() });
                i += 1;
            }
            Op::Ins => {
                put(&mut p, tb.get(j), &CharProps { ins: Some(ctx.ins), ..base.clone() });
                j += 1;
            }
        }
    }
    p
}
