use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::para::OBJ;

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character" (character: paragraphs over 2000 characters fall back to word; see characterFallback/notHonoured), "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared)}"#;

/// What Compare / Combine compares (the More >> settings of Word's dialog).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CompareOptions {
    /// Differences only in letter case count as changes.
    pub(super) case: bool,
    /// Differences only in white space count as changes.
    pub(super) white_space: bool,
    /// Formatting differences become tracked formatting changes.
    pub(super) formatting: bool,
    /// Changed paragraphs are compared character by character (else word by word).
    pub(super) characters: bool,
}

impl CompareOptions {
    /// The text a comparison sees: case folded and white space collapsed as the options say.
    pub(super) fn key(&self, t: &str) -> String {
        let t = if self.white_space { t.to_string() } else { t.split_whitespace().collect::<Vec<_>>().join(" ") };
        if self.case { t } else { t.to_lowercase() }
    }
}

/// Context and tracking counters for diffing paragraph contents.
struct DiffCtx<'a> {
    ins: u32,
    del: u32,
    fmt: u32,
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
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Format, author: author.clone(), date: date.clone(), move_name: None });
    let fmt = (s.doc.revisions.len() - 1) as u32;
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => opts.key(&p.plain_text()),
        // Tables match as a whole, by their text.
        Block::Table(_) => format!("\u{0}table\u{0}{}", opts.key(&block_text(b, 0))),
    };
    let mut diff_budget = MAX_COMPARE_CELLS;
    let mut align_budget = MAX_COMPARE_CELLS;
    let mut char_fallbacks = 0usize;
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), &mut diff_budget);
    let mut ctx = DiffCtx { ins, del, fmt, opts: &opts, budget: &mut diff_budget, char_fallbacks: &mut char_fallbacks };
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
                    if opts.formatting && pa.section.is_some() && pb.section.is_none() {
                        if let Some(x) = a {
                            out.push(Arc::new(mark_block(x, &|c| c.del = Some(del), 0)));
                            changes += 1;
                        }
                        if let Some(x) = b {
                            out.push(Arc::new(mark_inserted_block(x, ins, 0)));
                            changes += 1;
                        }
                        i += 1;
                        j += 1;
                        k += 1;
                        continue;
                    }
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
                    let (p, changed) = build_same_para(pa, pb, &opts, &mut s.doc.revisions, &author, &date, fmt);
                    if changed {
                        changes += 1;
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
                // A run of deletions and insertions: changed paragraphs pair up by text similarity
                // and are diffed word by word (or by character); unpartnered blocks are deleted
                // or inserted whole.
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                let old_slice = old.get(i..i + dels).unwrap_or_default();
                let new_slice = new.get(j..j + inss).unwrap_or_default();
                let pairs = align_block_run(old_slice, new_slice, &opts, &mut align_budget);
                changes += emit_run_pairs(&s.doc, &pairs, old_slice, new_slice, &mut ctx, &mut out);
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

/// Builds the comparison output paragraph for an unchanged block pair, preserving the original's
/// tracked revision metadata and recording formatting differences when formatting comparison is enabled.
fn build_same_para(
    pa: &Paragraph,
    pb: &Paragraph,
    opts: &CompareOptions,
    revisions: &mut Vec<wordcraft_doc::Revision>,
    author: &str,
    date: &str,
    fmt: u32,
) -> (Paragraph, bool) {
    let mut p = pb.clone();
    p.props.fmt_change = pa.props.fmt_change.clone();
    for r in &mut p.runs {
        r.props.ins = None;
        r.props.del = None;
        r.props.fmt_change = None;
    }
    let new_fmt = std::cell::Cell::new(false);
    let map = map_byte_offsets(&pa.text, &pb.text, opts);
    for (r, props_a) in pa.run_ranges() {
        let start_b = map.get(r.start).copied().unwrap_or(p.len());
        let end_b = map.get(r.end).copied().unwrap_or(p.len());
        if start_b < end_b {
            let _ = p.format(start_b, end_b, &|c| {
                c.ins = props_a.ins;
                c.del = props_a.del;
                if opts.formatting && pa.text != pb.text && props_a.formatting() != c.formatting() {
                    c.fmt_change = wordcraft_doc::props::PropChange::boxed(fmt, props_a.formatting());
                    new_fmt.set(true);
                } else {
                    c.fmt_change = props_a.fmt_change.clone();
                }
            });
        }
    }
    p.mark.ins = pa.mark.ins;
    p.mark.del = pa.mark.del;
    if opts.formatting && pa.text != pb.text && pa.mark.formatting() != p.mark.formatting() {
        p.mark.fmt_change = wordcraft_doc::props::PropChange::boxed(fmt, pa.mark.formatting());
        new_fmt.set(true);
    } else {
        p.mark.fmt_change = pa.mark.fmt_change.clone();
    }
    let mut changed = false;
    if !opts.formatting {
        p.props = pa.props.clone();
        p.section = pa.section.clone();
    } else {
        let before = revisions.len();
        if pa.text == pb.text {
            crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, revisions, author, date);
        } else if pa.props.formatting() != p.props.formatting() {
            p.props.fmt_change = wordcraft_doc::props::PropChange::boxed(fmt, pa.props.formatting());
            new_fmt.set(true);
        } else {
            p.props.fmt_change = pa.props.fmt_change.clone();
        }
        if let (Some(sa), Some(sb)) = (&pa.section, &mut p.section) {
            if strip_section(sa) != strip_section(sb) {
                sb.fmt_change = wordcraft_doc::props::PropChange::boxed(fmt, strip_section(sa));
                new_fmt.set(true);
            } else {
                sb.fmt_change = sa.fmt_change.clone();
            }
        }
        if revisions.len() > before
            || new_fmt.get()
            || p.runs.iter().any(|r| r.props.fmt_change.as_ref().is_some_and(|c| c.rev == fmt))
            || p.mark.fmt_change.as_ref().is_some_and(|c| c.rev == fmt)
            || p.props.fmt_change.as_ref().is_some_and(|c| c.rev == fmt)
            || p.section.as_deref().and_then(|s| s.fmt_change.as_ref()).is_some_and(|c| c.rev == fmt)
        {
            changed = true;
        }
    }
    (p, changed)
}

/// Emits alignment pairs within an edit run, diffing paired paragraphs or treating unmatched
/// and mismatched section blocks as whole-block deletions and insertions. Returns the number of changes added.
fn emit_run_pairs(
    doc: &Document,
    pairs: &[RunPair],
    old_slice: &[Arc<Block>],
    new_slice: &[Arc<Block>],
    ctx: &mut DiffCtx<'_>,
    out: &mut Vec<Arc<Block>>,
) -> usize {
    let mut changes = 0;
    for &pair in pairs {
        match pair {
            RunPair::Diff(u, v) => {
                if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (old_slice.get(u).map(|x| &**x), new_slice.get(v).map(|x| &**x)) {
                    if ctx.opts.formatting && pa.section.is_some() && pb.section.is_none() {
                        if let Some(x) = old_slice.get(u) {
                            out.push(Arc::new(mark_block(x, &|c| c.del = Some(ctx.del), 0)));
                            changes += 1;
                        }
                        if let Some(x) = new_slice.get(v) {
                            out.push(Arc::new(mark_inserted_block(x, ctx.ins, 0)));
                            changes += 1;
                        }
                    } else {
                        out.push(para_block(word_diff(doc, pa, pb, ctx)));
                        changes += 1;
                    }
                }
            }
            RunPair::Del(u) => {
                if let Some(x) = old_slice.get(u) {
                    out.push(Arc::new(mark_block(x, &|c| c.del = Some(ctx.del), 0)));
                    changes += 1;
                }
            }
            RunPair::Ins(v) => {
                if let Some(x) = new_slice.get(v) {
                    out.push(Arc::new(mark_inserted_block(x, ctx.ins, 0)));
                    changes += 1;
                }
            }
        }
    }
    changes
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
            // Marking the paragraph mark preserves accept/reject behavior for paragraph marks.
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

/// Returns a copy of `b` with all text runs and paragraph marks marked as inserted under `ins`,
/// clearing foreign revision metadata including paragraph-level and section formatting changes.
fn mark_inserted_block(b: &Block, ins: u32, depth: usize) -> Block {
    match b {
        Block::Para(p) => {
            let mut p = p.clone();
            let len = p.len();
            let _ = p.format(0, len, &|c| {
                c.ins = Some(ins);
                c.del = None;
                c.fmt_change = None;
            });
            p.mark.ins = Some(ins);
            p.mark.del = None;
            p.mark.fmt_change = None;
            p.props.fmt_change = None;
            if let Some(sec) = p.section.as_deref_mut() {
                sec.fmt_change = None;
            }
            Block::Para(p)
        }
        Block::Table(t) => {
            let mut t = t.clone();
            if depth < 16 {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    c.blocks = c.blocks.iter().map(|x| Arc::new(mark_inserted_block(x, ins, depth + 1))).collect();
                }
            }
            Block::Table(t)
        }
    }
}

/// An alignment step within a run of changed blocks: pairing an old block with a revised
/// one for within-block diffing, or emitting an unpartnered block as deleted or inserted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunPair {
    /// An original block at index `u` and a revised block at index `v` are paired and diffed.
    Diff(usize, usize),
    /// An original block at index `u` has no partner in the revised document and is deleted.
    Del(usize),
    /// A revised block at index `v` has no partner in the original document and is inserted.
    Ins(usize),
}

/// Minimum similarity (on a 0..=1000 scale) required to pair two paragraphs for within-block diffing.
///
/// Pairs scoring below this threshold are treated as independent deletions and insertions,
/// preventing accidental pairing of unrelated paragraphs across edit runs.
///
/// Exception: when no pair in an edit run reaches this threshold and both sides have the
/// same number of blocks (`n == m`), paragraph blocks fall back to 1-to-1 positional pairing
/// to preserve paragraph slots. See the fallback in [`align_block_run`]. This exception
/// does not apply when block counts differ.
const SIMILARITY_THRESHOLD: u32 = 400;

/// Minimum character similarity required to pair single-token paragraphs (such as headings)
/// when word tokenization shares zero common words.
///
/// A stricter threshold than [`SIMILARITY_THRESHOLD`] is enforced for single tokens to avoid
/// false matches from coincidental shared consonants between unrelated short words (such as
/// "first" vs "fifth"), while allowing genuine heading edits (e.g. "Introduction" vs "Introductions",
/// "Color" vs "Colour") to pair cleanly.
const SINGLE_TOKEN_SIMILARITY_THRESHOLD: u32 = 700;

/// Maximum number of DP cells (`old_len * new_len`) computed when aligning an edit run.
///
/// When an edit run exceeds this limit, alignment falls back to 1-to-1 positional pairing
/// rather than computing the full similarity matrix, bounding compute time on very large runs
/// at the cost of reintroducing mispairing risk for oversized runs.
const MAX_RUN_ALIGN_CELLS: usize = 1_000_000;

/// Maximum number of word tokens considered per paragraph when computing similarity scores.
pub(super) const MAX_SIM_WORDS: usize = 500;

/// Maximum number of characters considered per paragraph when computing character similarity scores.
pub(super) const MAX_SIM_CHARS: usize = 1000;

/// Precomputed representation of a block for fast paragraph similarity scoring.
#[derive(Clone, Debug)]
pub(super) enum PrecomputedBlock<'a> {
    /// A non-paragraph block (e.g. table), which does not participate in similarity scoring.
    NonPara,
    /// A paragraph block with normalized text and token representations.
    Para {
        /// Normalized comparison key.
        key: &'a str,
        /// Whether the text contains CJK ideographs.
        has_cjk: bool,
        /// Word tokens (capped at `MAX_SIM_WORDS`).
        words: Vec<&'a str>,
        /// Whitespace-stripped character tokens (capped at `MAX_SIM_CHARS`).
        chars: Vec<char>,
    },
}

/// Extracts a bounded sample of word tokens from a normalized text key.
///
/// When the text contains more than `limit` words, samples from the head, interior
/// (middle), and tail of the text. This ensures paragraphs sharing boilerplate
/// at both the start and the end cannot receive an artificially high similarity score
/// when their middles differ, while bounding memory and compute time to `O(limit)`.
pub(super) fn sample_words(key: &str, limit: usize) -> Vec<&str> {
    let total = key.split_whitespace().count();
    if total <= limit {
        return key.split_whitespace().collect();
    }
    let head_count = limit / 3;
    let tail_count = limit / 3;
    let mid_count = limit.saturating_sub(head_count).saturating_sub(tail_count);
    let mid_start = (total / 2).saturating_sub(mid_count / 2).max(head_count);
    let mid_end = mid_start.saturating_add(mid_count).min(total.saturating_sub(tail_count));
    let tail_start = total.saturating_sub(tail_count);

    let mut words = Vec::with_capacity(limit);
    for (i, w) in key.split_whitespace().enumerate() {
        if i < head_count || (i >= mid_start && i < mid_end) || i >= tail_start {
            words.push(w);
        }
    }
    words
}

/// Extracts a bounded sample of non-whitespace character tokens from a normalized text key.
///
/// When the text contains more than `limit` non-whitespace characters, samples from
/// the head, interior (middle), and tail of the text. This prevents long texts
/// sharing boilerplate at both the start and end from receiving an artificially
/// high similarity score when their middles differ.
pub(super) fn sample_chars(key: &str, limit: usize) -> Vec<char> {
    let total = key.chars().filter(|c| !c.is_whitespace()).count();
    if total <= limit {
        return key.chars().filter(|c| !c.is_whitespace()).collect();
    }
    let head_count = limit / 3;
    let tail_count = limit / 3;
    let mid_count = limit.saturating_sub(head_count).saturating_sub(tail_count);
    let mid_start = (total / 2).saturating_sub(mid_count / 2).max(head_count);
    let mid_end = mid_start.saturating_add(mid_count).min(total.saturating_sub(tail_count));
    let tail_start = total.saturating_sub(tail_count);

    let mut chars = Vec::with_capacity(limit);
    for (i, c) in key.chars().filter(|c| !c.is_whitespace()).enumerate() {
        if i < head_count || (i >= mid_start && i < mid_end) || i >= tail_start {
            chars.push(c);
        }
    }
    chars
}

/// Extracts precomputed token representations for a sequence of blocks.
///
/// Normalizes paragraph plain text via `opts.key`, pre-scans for CJK ideographs,
/// and extracts word and character tokens bounded by `MAX_SIM_WORDS` and `MAX_SIM_CHARS`
/// (sampling from both head and tail for oversized paragraphs). Non-paragraph blocks
/// are marked as `NonPara`.
pub(super) fn precompute_blocks<'a>(blocks: &'a [Arc<Block>], keys: &'a [Option<String>]) -> Vec<PrecomputedBlock<'a>> {
    blocks
        .iter()
        .zip(keys)
        .map(|(b, k)| match (&**b, k) {
            (Block::Para(_), Some(key)) => {
                let chars = sample_chars(key, MAX_SIM_CHARS);
                let has_cjk = chars.iter().copied().any(wordcraft_fonts::is_cjk);
                let words = sample_words(key.as_str(), MAX_SIM_WORDS);
                PrecomputedBlock::Para { key: key.as_str(), has_cjk, words, chars }
            }
            _ => PrecomputedBlock::NonPara,
        })
        .collect()
}

/// Builds a 1-to-1 positional pairing fallback for a block run.
///
/// Pairs paragraph blocks at matching indices as `RunPair::Diff`, and emits any non-paragraph
/// blocks (such as tables) or excess blocks as deletions and insertions.
fn positional_fallback(n: usize, m: usize, old_blocks: &[Arc<Block>], new_blocks: &[Arc<Block>]) -> Vec<RunPair> {
    let mut pairs = Vec::with_capacity(n.max(m));
    for x in 0..n.max(m) {
        match (old_blocks.get(x), new_blocks.get(x)) {
            (Some(a), Some(b)) if matches!((&**a, &**b), (Block::Para(_), Block::Para(_))) => {
                pairs.push(RunPair::Diff(x, x));
            }
            _ => {
                if x < n {
                    pairs.push(RunPair::Del(x));
                }
                if x < m {
                    pairs.push(RunPair::Ins(x));
                }
            }
        }
    }
    pairs
}

/// Pairs blocks in an edited run between exact matches.
///
/// When an edit run occurs, paragraphs are aligned by text similarity using dynamic programming
/// so that edited paragraphs retain their partners rather than collapsing into wholesale replacements.
///
/// Fallbacks and special cases:
/// - Single-pair run (`n == 1 && m == 1`): If both are paragraphs, they are directly paired as
///   `RunPair::Diff(0, 0)` without running alignment; non-paragraph blocks emit `Del(0)` and `Ins(0)`.
/// - Oversize run (`n * m > MAX_RUN_ALIGN_CELLS`) or exhausted work budget: Falls back to 1-to-1
///   positional pairing via [`positional_fallback`], bounding CPU time on untrusted input at the
///   cost of reintroducing mispairing risk for very large edit runs.
/// - Zero-similarity equal-count run (`dp == 0 && n == m`): When no paragraph pair meets
///   [`SIMILARITY_THRESHOLD`] and both sides have equal block counts, paragraph blocks are paired
///   positionally so that whole-run rewrites preserve paragraph slots and accept/reject cleanly.
///   When counts differ, unpartnered blocks are emitted as independent deletions and insertions.
/// - Non-paragraph blocks (such as tables): Do not participate in similarity scoring and are
///   emitted as deletions and insertions.
pub(super) fn align_block_run(old_blocks: &[Arc<Block>], new_blocks: &[Arc<Block>], opts: &CompareOptions, budget: &mut u64) -> Vec<RunPair> {
    let n = old_blocks.len();
    let m = new_blocks.len();
    if n == 0 {
        return (0..m).map(RunPair::Ins).collect();
    }
    if m == 0 {
        return (0..n).map(RunPair::Del).collect();
    }
    if n == 1 && m == 1 {
        return match (old_blocks.first().map(|x| &**x), new_blocks.first().map(|x| &**x)) {
            (Some(Block::Para(_)), Some(Block::Para(_))) => vec![RunPair::Diff(0, 0)],
            _ => vec![RunPair::Del(0), RunPair::Ins(0)],
        };
    }
    if n.saturating_mul(m) > MAX_RUN_ALIGN_CELLS {
        return positional_fallback(n, m, old_blocks, new_blocks);
    }

    let old_keys: Vec<Option<String>> = old_blocks
        .iter()
        .map(|b| match &**b {
            Block::Para(p) => Some(opts.key(&p.plain_text())),
            _ => None,
        })
        .collect();
    let new_keys: Vec<Option<String>> = new_blocks
        .iter()
        .map(|b| match &**b {
            Block::Para(p) => Some(opts.key(&p.plain_text())),
            _ => None,
        })
        .collect();

    let pre_old = precompute_blocks(old_blocks, &old_keys);
    let pre_new = precompute_blocks(new_blocks, &new_keys);

    let scores = match compute_align_scores(&pre_old, &pre_new, opts, budget) {
        Some(s) => s,
        None => return positional_fallback(n, m, old_blocks, new_blocks),
    };

    let stride = m.saturating_add(1);
    let total_dp_cells = (n.saturating_add(1)).saturating_mul(stride);
    let mut dp = vec![0u32; total_dp_cells];

    for u in 0..n {
        for v in 0..m {
            let skip_old = dp.get(u.saturating_mul(stride).saturating_add(v + 1)).copied().unwrap_or(0);
            let skip_new = dp.get((u + 1).saturating_mul(stride).saturating_add(v)).copied().unwrap_or(0);
            let mut best = skip_old.max(skip_new);
            let pair_score = scores.get(u.saturating_mul(m).saturating_add(v)).copied().unwrap_or(0);
            if pair_score > 0 {
                let diag = dp.get(u.saturating_mul(stride).saturating_add(v)).copied().unwrap_or(0);
                best = best.max(diag.saturating_add(pair_score));
            }
            if let Some(cell) = dp.get_mut((u + 1).saturating_mul(stride).saturating_add(v + 1)) {
                *cell = best;
            }
        }
    }

    // When no paragraph in the run shares enough similarity to form a confident pair,
    // and both sides have an equal number of blocks, pair positionally (1-to-1) so
    // wholesale replacements still preserve paragraph slots and reject cleanly.
    let final_dp = dp.get(n.saturating_mul(stride).saturating_add(m)).copied().unwrap_or(0);
    if final_dp == 0 && n == m {
        return positional_fallback(n, m, old_blocks, new_blocks);
    }

    backtrack_align_dp(&scores, &dp, n, m)
}

/// Computes pairwise similarity scores for all block pairs in an edit run.
///
/// Stores scores in a flat `Vec<u32>` of length `n * m`. Pairs scoring below
/// `SIMILARITY_THRESHOLD` or involving non-paragraph blocks are stored as zero.
///
/// Returns `None` if the total similarity computation exceeds the available `budget`,
/// signaling that alignment should fall back to 1-to-1 positional pairing.
/// Runs in worst-case `O(n * m)` time and space.
fn compute_align_scores(
    old_blocks: &[PrecomputedBlock<'_>],
    new_blocks: &[PrecomputedBlock<'_>],
    opts: &CompareOptions,
    budget: &mut u64,
) -> Option<Vec<u32>> {
    let n = old_blocks.len();
    let m = new_blocks.len();
    let total_cells = n.saturating_mul(m);
    let mut scores = vec![0u32; total_cells];
    let mut dp_buf = Vec::new();

    for (u, a) in old_blocks.iter().enumerate() {
        for (v, b) in new_blocks.iter().enumerate() {
            let score = para_similarity(a, b, opts, &mut dp_buf, budget)?;
            if let Some(cell) = scores.get_mut(u.saturating_mul(m).saturating_add(v)) {
                *cell = score;
            }
        }
    }
    Some(scores)
}

/// Backtracks an alignment DP table to produce the sequence of paired and unpartnered blocks.
///
/// The DP table `dp` has dimensions `(n + 1) * (m + 1)` and `scores` has dimensions `n * m`.
/// Tie-breaking order prefers `RunPair::Diff` (pairing paragraphs diagonally), then
/// `RunPair::Ins` (inserting from revised document), then `RunPair::Del` (deleting from
/// original document). Operates in `O(n + m)` time.
fn backtrack_align_dp(scores: &[u32], dp: &[u32], n: usize, m: usize) -> Vec<RunPair> {
    let stride = m.saturating_add(1);
    let mut pairs = Vec::with_capacity(n.max(m));
    let (mut u, mut v) = (n, m);
    while u > 0 || v > 0 {
        let cur_dp = dp.get(u.saturating_mul(stride).saturating_add(v)).copied().unwrap_or(0);
        let pair_score = if u > 0 && v > 0 { scores.get((u - 1).saturating_mul(m).saturating_add(v - 1)).copied().unwrap_or(0) } else { 0 };
        let diag_dp = if u > 0 && v > 0 { dp.get((u - 1).saturating_mul(stride).saturating_add(v - 1)).copied().unwrap_or(0) } else { 0 };
        let prev_v_dp = if v > 0 { dp.get(u.saturating_mul(stride).saturating_add(v - 1)).copied().unwrap_or(0) } else { 0 };

        if u > 0 && v > 0 && pair_score > 0 && cur_dp == diag_dp.saturating_add(pair_score) {
            pairs.push(RunPair::Diff(u - 1, v - 1));
            u -= 1;
            v -= 1;
        } else if v > 0 && (u == 0 || cur_dp == prev_v_dp) {
            pairs.push(RunPair::Ins(v - 1));
            v -= 1;
        } else if u > 0 {
            pairs.push(RunPair::Del(u - 1));
            u -= 1;
        } else {
            break;
        }
    }
    pairs.reverse();
    pairs
}

/// Computes a text similarity score between two precomputed paragraphs, on a 0..=1000 scale.
///
/// Returns `None` if the similarity LCS cell budget is exceeded. Returns 0 if similarity
/// falls below `SIMILARITY_THRESHOLD` or if the theoretical length-ratio upper bound cannot
/// reach `SIMILARITY_THRESHOLD`.
///
/// Uses order-aware longest-common-subsequence matching over word tokens for multi-word or
/// whitespace-separated text, and over whitespace-stripped characters for non-space scripts
/// (such as CJK) or when character-level comparison is requested.
///
/// When default word tokenization yields no comparable words (such as single-word headings
/// where word tokens share zero overlap), falls back to bounded character scoring so modified
/// single-token paragraphs can pair up across edit runs.
///
/// Similarity is scored using `(2000 * lcs) / total`, where `total = len_a + len_b`.
pub(super) fn para_similarity(
    a: &PrecomputedBlock<'_>,
    b: &PrecomputedBlock<'_>,
    opts: &CompareOptions,
    dp_buf: &mut Vec<u32>,
    budget: &mut u64,
) -> Option<u32> {
    let (
        PrecomputedBlock::Para { key: key_a, has_cjk: cjk_a, words: w_a, chars: ch_a },
        PrecomputedBlock::Para { key: key_b, has_cjk: cjk_b, words: w_b, chars: ch_b },
    ) = (a, b)
    else {
        return Some(0);
    };
    if key_a == key_b {
        return Some(1000);
    }
    let use_chars = opts.characters || *cjk_a || *cjk_b;
    let score = if use_chars {
        score_tokens(ch_a, ch_b, dp_buf, budget)?
    } else {
        let word_score = score_tokens(w_a, w_b, dp_buf, budget)?;
        if word_score == 0 && (w_a.len() <= 1 && w_b.len() <= 1) {
            let char_score = score_tokens(ch_a, ch_b, dp_buf, budget)?;
            if char_score >= SINGLE_TOKEN_SIMILARITY_THRESHOLD { char_score } else { 0 }
        } else {
            word_score
        }
    };
    // Paragraphs whose full keys differ cannot receive a perfect score even if their bounded samples match.
    Some(score.min(999))
}

/// Computes similarity on a slice of tokens (words or characters).
///
/// Applies a cheap length-ratio check first: maximum possible score is `2000 * min_len / total`.
/// If that upper bound is below `SIMILARITY_THRESHOLD`, returns 0 immediately without running LCS.
/// Otherwise runs `lcs_len`, charging core DP cells to `budget`.
fn score_tokens<T: PartialEq>(tokens_a: &[T], tokens_b: &[T], dp_buf: &mut Vec<u32>, budget: &mut u64) -> Option<u32> {
    let len_a = tokens_a.len();
    let len_b = tokens_b.len();
    if len_a == 0 && len_b == 0 {
        return Some(1000);
    }
    if len_a == 0 || len_b == 0 {
        return Some(0);
    }
    let total = len_a.saturating_add(len_b);
    let min_len = len_a.min(len_b);
    let max_possible = (2000u64 * min_len as u64) / total as u64;
    if max_possible < SIMILARITY_THRESHOLD as u64 {
        return Some(0);
    }
    let lcs = lcs_len(tokens_a, tokens_b, dp_buf, budget)?;
    let score = ((2000u64 * lcs as u64) / total as u64) as u32;
    Some(if score >= SIMILARITY_THRESHOLD { score } else { 0 })
}

/// Computes the length of the longest common subsequence between two token slices.
///
/// Strips common prefix and suffix tokens before building the DP table, charging each
/// inspected pair of tokens to `budget` and returning `None` if the budget is exhausted.
/// When interior tokens remain, charges the core DP cell count (`a_core.len() * b_core.len()`)
/// to `budget`, returning `None` if the budget is exceeded.
///
/// Uses `dp_buf` to compute LCS length in `O(short.len() * long.len())` time and
/// `O(short.len())` space without per-call allocations. Returns `Some(pre + suf)` when either
/// core sequence is empty.
pub(super) fn lcs_len<T: PartialEq>(a: &[T], b: &[T], dp_buf: &mut Vec<u32>, budget: &mut u64) -> Option<usize> {
    if a.is_empty() || b.is_empty() {
        return Some(0);
    }
    let mut pre = 0usize;
    while pre < a.len() && pre < b.len() {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        if a.get(pre) == b.get(pre) {
            pre = pre.saturating_add(1);
        } else {
            break;
        }
    }
    let a_mid = a.get(pre..).unwrap_or_default();
    let b_mid = b.get(pre..).unwrap_or_default();
    let mut suf = 0usize;
    while suf < a_mid.len() && suf < b_mid.len() {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        let idx_a = a_mid.len().saturating_sub(1).saturating_sub(suf);
        let idx_b = b_mid.len().saturating_sub(1).saturating_sub(suf);
        if a_mid.get(idx_a) == b_mid.get(idx_b) {
            suf = suf.saturating_add(1);
        } else {
            break;
        }
    }
    let a_core = a_mid.get(..a_mid.len().saturating_sub(suf)).unwrap_or_default();
    let b_core = b_mid.get(..b_mid.len().saturating_sub(suf)).unwrap_or_default();

    if a_core.is_empty() || b_core.is_empty() {
        return Some(pre.saturating_add(suf));
    }

    let cells = (a_core.len() as u64).saturating_mul(b_core.len() as u64);
    if cells > *budget {
        return None;
    }
    *budget -= cells;

    let (short, long) = if a_core.len() <= b_core.len() { (a_core, b_core) } else { (b_core, a_core) };
    dp_buf.clear();
    dp_buf.resize(short.len().saturating_add(1), 0);
    for item_l in long {
        let mut prev_diag = 0u32;
        for (j, item_s) in short.iter().enumerate() {
            let temp = dp_buf.get(j + 1).copied().unwrap_or(0);
            if item_l == item_s {
                if let Some(c) = dp_buf.get_mut(j + 1) {
                    *c = prev_diag.saturating_add(1);
                }
            } else {
                let left = dp_buf.get(j).copied().unwrap_or(0);
                if let Some(c) = dp_buf.get_mut(j + 1) {
                    *c = (*c).max(left);
                }
            }
            prev_diag = temp;
        }
    }
    Some(pre.saturating_add(suf).saturating_add(dp_buf.last().copied().unwrap_or(0) as usize))
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
    Text { text: String, runs: Vec<(String, CharProps)> },
    Obj(&'a InlineObject, Box<CharProps>),
}

/// Accumulates characters into words and runs without splitting words at formatting boundaries.
struct WordBuilder<'a> {
    tokens: Vec<Tok<'a>>,
    word_text: String,
    runs: Vec<(String, CharProps)>,
    cur_run_text: String,
    cur_run_props: CharProps,
    characters: bool,
}

impl<'a> WordBuilder<'a> {
    /// Creates a new builder for word or character tokenization.
    fn new(characters: bool) -> Self {
        Self {
            tokens: Vec::new(),
            word_text: String::new(),
            runs: Vec::new(),
            cur_run_text: String::new(),
            cur_run_props: CharProps::default(),
            characters,
        }
    }

    /// Appends a character with its formatting properties, completing the word on space or char mode.
    fn push_char(&mut self, c: char, props: &CharProps) {
        if self.cur_run_text.is_empty() {
            self.cur_run_props = props.clone();
        } else if &self.cur_run_props != props {
            self.runs.push((std::mem::take(&mut self.cur_run_text), self.cur_run_props.clone()));
            self.cur_run_props = props.clone();
        }
        self.word_text.push(c);
        self.cur_run_text.push(c);
        if self.characters || c == ' ' {
            self.finish_word();
        }
    }

    /// Flushes the active character run and completed word token into the token list.
    fn finish_word(&mut self) {
        if !self.cur_run_text.is_empty() {
            self.runs.push((std::mem::take(&mut self.cur_run_text), self.cur_run_props.clone()));
        }
        if !self.word_text.is_empty() {
            self.tokens.push(Tok::Text { text: std::mem::take(&mut self.word_text), runs: std::mem::take(&mut self.runs) });
        }
        self.cur_run_props = CharProps::default();
    }

    /// Flushes any pending word and appends an inline object token.
    fn push_obj(&mut self, o: &'a InlineObject, props: &CharProps) {
        self.finish_word();
        self.tokens.push(Tok::Obj(o, Box::new(props.clone())));
    }
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
    let mut builder = WordBuilder::new(characters);
    let mut objects = p.objects.iter();
    let mut runs = p.run_ranges();
    let mut run = runs.next();
    for (off, c) in p.text.char_indices() {
        while matches!(&run, Some((r, _)) if off >= r.end) {
            run = runs.next();
        }
        let props = match &run {
            Some((r, props)) if off >= r.start => *props,
            _ => &p.mark,
        };
        if c != OBJ {
            builder.push_char(c, props);
            continue;
        }
        match objects.next() {
            Some(o) if o.plain_text().is_empty() => {
                builder.push_obj(o, props);
            }
            Some(o) => {
                for c in o.plain_text().chars() {
                    builder.push_char(c, props);
                }
            }
            None => {
                // A marker without its object (a broken paragraph): kept as a visible
                // placeholder, so the loss shows in the result instead of vanishing.
                log::warn!("compare: an object marker without its object");
                builder.push_char(char::REPLACEMENT_CHARACTER, props);
            }
        }
    }
    builder.finish_word();
    builder.tokens
}

/// Returns the normalized key string and a mapping from each character in the key
/// to its corresponding source byte offset in `s`.
fn key_with_offsets(s: &str, opts: &CompareOptions) -> (String, Vec<usize>) {
    let mut key = String::new();
    let mut offsets = Vec::new();

    if opts.white_space {
        if opts.case {
            for (b_idx, c) in s.char_indices() {
                key.push(c);
                offsets.push(b_idx);
            }
        } else {
            let s_lc = s.to_lowercase();
            let mut lc_chars = s_lc.chars();
            for (b_idx, c) in s.char_indices() {
                for _ in 0..c.to_lowercase().count() {
                    if let Some(lc) = lc_chars.next() {
                        key.push(lc);
                        offsets.push(b_idx);
                    }
                }
            }
        }
    } else {
        let mut in_ws = true;
        let mut word_start_b = 0;
        let mut ws_start_b = 0;
        let mut words = Vec::new();

        for (b_idx, c) in s.char_indices() {
            if c.is_whitespace() {
                if !in_ws {
                    words.push((word_start_b, b_idx, ws_start_b));
                    in_ws = true;
                    ws_start_b = b_idx;
                }
            } else {
                if in_ws {
                    word_start_b = b_idx;
                    in_ws = false;
                }
            }
        }
        if !in_ws {
            words.push((word_start_b, s.len(), ws_start_b));
        }

        for (i, &(wb_start, wb_end, prev_ws)) in words.iter().enumerate() {
            if i > 0 {
                key.push(' ');
                offsets.push(prev_ws);
            }
            let word = s.get(wb_start..wb_end).unwrap_or_default();
            if opts.case {
                for (b_off, c) in word.char_indices() {
                    key.push(c);
                    offsets.push(wb_start.saturating_add(b_off));
                }
            } else {
                let w_lc = word.to_lowercase();
                let mut lc_chars = w_lc.chars();
                for (b_off, c) in word.char_indices() {
                    for _ in 0..c.to_lowercase().count() {
                        if let Some(lc) = lc_chars.next() {
                            key.push(lc);
                            offsets.push(wb_start.saturating_add(b_off));
                        }
                    }
                }
            }
        }
    }
    offsets.push(s.len());
    (key, offsets)
}

/// Maps byte offsets from `a` to `b` when their plain text matches under `opts.key`.
fn map_byte_offsets(a: &str, b: &str, opts: &CompareOptions) -> Vec<usize> {
    if a == b {
        return (0..=a.len()).collect();
    }
    let (key_a, offsets_a) = key_with_offsets(a, opts);
    let (key_b, offsets_b) = key_with_offsets(b, opts);

    if key_a != key_b || offsets_a.is_empty() || offsets_b.is_empty() {
        return (0..=a.len()).map(|off| if a.is_empty() { 0 } else { ((off as u64 * b.len() as u64) / a.len() as u64) as usize }).collect();
    }

    let mut map = Vec::with_capacity(a.len() + 1);
    for byte_a in 0..=a.len() {
        let k = offsets_a.partition_point(|&off| off < byte_a);
        let byte_b = offsets_b.get(k).copied().unwrap_or(b.len());
        map.push(byte_b.min(b.len()));
    }
    map
}

/// Finds the `CharProps` in `runs` containing byte index `byte_idx`.
fn lookup_props_by_byte<'a>(runs: &'a [(String, CharProps)], byte_idx: usize, base: &'a CharProps) -> &'a CharProps {
    let mut acc = 0;
    for (s, p) in runs {
        let next = acc + s.len();
        if byte_idx < next {
            return p;
        }
        acc = next;
    }
    runs.last().map(|(_, p)| p).unwrap_or(base)
}

/// Emit a matched token into `p`, preserving original revision state from `a` while applying
/// revised formatting changes when formatting comparison is enabled.
fn put_same(p: &mut Paragraph, ta: Option<&Tok<'_>>, tb: Option<&Tok<'_>>, ctx: &DiffCtx<'_>, base: &CharProps) {
    match (ta, tb) {
        (Some(Tok::Text { text: text_a, runs: runs_a }), Some(Tok::Text { text: text_b, runs: runs_b })) => {
            if runs_a.len() <= 1 && runs_b.len() <= 1 {
                let props_a = runs_a.first().map(|(_, p)| p).unwrap_or(base);
                let props_b = runs_b.first().map(|(_, p)| p).unwrap_or(base);
                let props = if ctx.opts.formatting {
                    let mut pr = props_b.clone();
                    pr.ins = props_a.ins;
                    pr.del = props_a.del;
                    if props_a.formatting() != props_b.formatting() {
                        pr.fmt_change = wordcraft_doc::props::PropChange::boxed(ctx.fmt, props_a.formatting());
                    } else {
                        pr.fmt_change = props_a.fmt_change.clone();
                    }
                    pr
                } else {
                    CharProps { ins: props_a.ins, del: props_a.del, fmt_change: props_a.fmt_change.clone(), ..props_a.formatting() }
                };
                let _ = p.insert_text(p.len(), text_b, &props);
            } else {
                let map_a_to_b = map_byte_offsets(text_a, text_b, ctx.opts);
                let map_b_to_a = map_byte_offsets(text_b, text_a, ctx.opts);
                let mut splits = Vec::with_capacity(runs_a.len() + runs_b.len() + 2);
                splits.push(0);
                splits.push(text_b.len());
                let mut off_b = 0;
                for (s, _) in runs_b {
                    off_b += s.len();
                    if off_b < text_b.len() {
                        splits.push(off_b);
                    }
                }
                let mut off_a = 0;
                for (s, _) in runs_a {
                    off_a += s.len();
                    if off_a < text_a.len() {
                        let b_split = map_a_to_b.get(off_a).copied().unwrap_or(text_b.len());
                        if b_split < text_b.len() {
                            splits.push(b_split);
                        }
                    }
                }
                splits.sort_unstable();
                splits.dedup();

                for w in splits.windows(2) {
                    let (b0, b1) = (w[0], w[1]);
                    if b0 >= b1 {
                        continue;
                    }
                    let slice = text_b.get(b0..b1).unwrap_or("");
                    if slice.is_empty() {
                        continue;
                    }
                    let a0 = map_b_to_a.get(b0).copied().unwrap_or(0);
                    let props_a = lookup_props_by_byte(runs_a, a0, base);
                    let props_b = lookup_props_by_byte(runs_b, b0, base);
                    let props = if ctx.opts.formatting {
                        let mut pr = props_b.clone();
                        pr.ins = props_a.ins;
                        pr.del = props_a.del;
                        if props_a.formatting() != props_b.formatting() {
                            pr.fmt_change = wordcraft_doc::props::PropChange::boxed(ctx.fmt, props_a.formatting());
                        } else {
                            pr.fmt_change = props_a.fmt_change.clone();
                        }
                        pr
                    } else {
                        CharProps { ins: props_a.ins, del: props_a.del, fmt_change: props_a.fmt_change.clone(), ..props_a.formatting() }
                    };
                    let _ = p.insert_text(p.len(), slice, &props);
                }
            }
        }
        (Some(Tok::Obj(oa, props_a)), Some(Tok::Obj(ob, props_b))) => {
            let _ = oa;
            let props = if ctx.opts.formatting {
                let mut pr = (**props_b).clone();
                pr.ins = props_a.ins;
                pr.del = props_a.del;
                if props_a.formatting() != props_b.formatting() {
                    pr.fmt_change = wordcraft_doc::props::PropChange::boxed(ctx.fmt, props_a.formatting());
                } else {
                    pr.fmt_change = props_a.fmt_change.clone();
                }
                pr
            } else {
                CharProps { ins: props_a.ins, del: props_a.del, fmt_change: props_a.fmt_change.clone(), ..props_a.formatting() }
            };
            let _ = p.insert_object(p.len(), (*ob).clone(), &props);
        }
        (_, Some(Tok::Text { text, runs })) => {
            let props_b = runs.first().map(|(_, p)| p).unwrap_or(base);
            let _ = p.insert_text(p.len(), text, props_b);
        }
        (_, Some(Tok::Obj(o, props_b))) => {
            let _ = p.insert_object(p.len(), (*o).clone(), props_b);
        }
        _ => {}
    }
}

/// Emit a deleted token into `p`, marking all runs with delete revision `ctx.del`.
fn put_del(p: &mut Paragraph, t: Option<&Tok<'_>>, ctx: &DiffCtx<'_>) {
    match t {
        Some(Tok::Text { runs, .. }) => {
            for (text, props_a) in runs {
                let mut props = props_a.clone();
                props.del = Some(ctx.del);
                let _ = p.insert_text(p.len(), text, &props);
            }
        }
        Some(Tok::Obj(o, props_a)) => {
            let mut props = (**props_a).clone();
            props.del = Some(ctx.del);
            let _ = p.insert_object(p.len(), (*o).clone(), &props);
        }
        None => {}
    }
}

/// Emit an inserted token into `p`, marking all runs with insert revision `ctx.ins`.
fn put_ins(p: &mut Paragraph, t: Option<&Tok<'_>>, ctx: &DiffCtx<'_>) {
    match t {
        Some(Tok::Text { runs, .. }) => {
            for (text, props_b) in runs {
                let props = CharProps { ins: Some(ctx.ins), del: None, fmt_change: None, ..props_b.clone() };
                let _ = p.insert_text(p.len(), text, &props);
            }
        }
        Some(Tok::Obj(o, props_b)) => {
            let props = CharProps { ins: Some(ctx.ins), del: None, fmt_change: None, ..(**props_b).clone() };
            let _ = p.insert_object(p.len(), (*o).clone(), &props);
        }
        None => {}
    }
}

/// Strips non-formatting section attributes for formatting comparison.
fn strip_section(s: &wordcraft_doc::SectionProps) -> wordcraft_doc::SectionProps {
    wordcraft_doc::SectionProps { fmt_change: None, headers: Default::default(), footers: Default::default(), ..s.clone() }
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

/// Returns `true` if paragraph `p` contains any CJK character.
fn para_has_cjk(p: &Paragraph) -> bool {
    p.text.chars().any(wordcraft_fonts::is_cjk)
}

/// Diffs two matched paragraphs either at word granularity or at character granularity according
/// to `ctx.opts.characters`. When character-level diff is requested but either paragraph exceeds
/// [`MAX_CHAR_DIFF`] characters, falls back to word-level diff and increments `ctx.char_fallbacks`
/// so callers can discover the fallback. Paired CJK paragraphs also use character-level diffing when
/// within [`MAX_CHAR_DIFF`].
fn word_diff(doc: &Document, a: &Paragraph, b: &Paragraph, ctx: &mut DiffCtx<'_>) -> Paragraph {
    let chars = if ctx.opts.characters {
        if para_exceeds_char_limit(a, MAX_CHAR_DIFF) || para_exceeds_char_limit(b, MAX_CHAR_DIFF) {
            *ctx.char_fallbacks = ctx.char_fallbacks.saturating_add(1);
            false
        } else {
            true
        }
    } else if para_has_cjk(a) || para_has_cjk(b) {
        !para_exceeds_char_limit(a, MAX_CHAR_DIFF) && !para_exceeds_char_limit(b, MAX_CHAR_DIFF)
    } else {
        false
    };
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let key = |t: &Tok<'_>| match t {
        Tok::Text { text, .. } => Key::Text(ctx.opts.key(text)),
        Tok::Obj(o, _) => obj_key(doc, o),
    };
    let ops = lcs_ops(&ta.iter().map(key).collect::<Vec<_>>(), &tb.iter().map(key).collect::<Vec<_>>(), ctx.budget);
    // Only at character granularity: a matched whole word almost never happens by coincidence,
    // but a matched single character very often does (digits, punctuation), so only there does
    // a short interior match need folding back into its surrounding edit.
    let ops = if chars { collapse_noise_matches(&ops) } else { ops };
    let mut p = Paragraph::new();

    if ctx.opts.formatting && a.props.formatting() != b.props.formatting() {
        p.props = b.props.formatting();
        p.props.fmt_change = wordcraft_doc::props::PropChange::boxed(ctx.fmt, a.props.formatting());
    } else if ctx.opts.formatting {
        p.props = b.props.clone();
        p.props.fmt_change = a.props.fmt_change.clone();
    } else {
        p.props = a.props.clone();
    }

    if ctx.opts.formatting && a.mark.formatting() != b.mark.formatting() {
        p.mark = CharProps {
            ins: a.mark.ins,
            del: a.mark.del,
            fmt_change: wordcraft_doc::props::PropChange::boxed(ctx.fmt, a.mark.formatting()),
            ..b.mark.formatting()
        };
    } else if ctx.opts.formatting {
        p.mark = CharProps { ins: a.mark.ins, del: a.mark.del, fmt_change: a.mark.fmt_change.clone(), ..b.mark.formatting() };
    } else {
        p.mark = a.mark.clone();
    }

    match (&a.section, &b.section) {
        (Some(sa), Some(sb)) => {
            if ctx.opts.formatting && strip_section(sa) != strip_section(sb) {
                let mut sec = (**sb).clone();
                sec.fmt_change = wordcraft_doc::props::PropChange::boxed(ctx.fmt, strip_section(sa));
                p.section = Some(Box::new(sec));
            } else if ctx.opts.formatting {
                let mut sec = (**sb).clone();
                sec.fmt_change = sa.fmt_change.clone();
                p.section = Some(Box::new(sec));
            } else {
                p.section = a.section.clone();
            }
        }
        (None, Some(sb)) => {
            if ctx.opts.formatting {
                let mut sec = (**sb).clone();
                sec.fmt_change = wordcraft_doc::props::PropChange::boxed(ctx.fmt, strip_section(sb));
                p.section = Some(Box::new(sec));
            } else {
                p.section = None;
            }
        }
        (Some(_), None) => {
            if ctx.opts.formatting {
                p.section = None;
            } else {
                p.section = a.section.clone();
            }
        }
        (None, None) => {
            p.section = None;
        }
    }

    let base = b.runs.first().map(|r| r.props.clone()).or_else(|| a.runs.first().map(|r| r.props.clone())).unwrap_or_else(|| b.mark.clone());
    let (mut i, mut j) = (0, 0);
    for op in ops {
        match op {
            Op::Same => {
                put_same(&mut p, ta.get(i), tb.get(j), ctx, &base);
                i += 1;
                j += 1;
            }
            Op::Del => {
                put_del(&mut p, ta.get(i), ctx);
                i += 1;
            }
            Op::Ins => {
                put_ins(&mut p, tb.get(j), ctx);
                j += 1;
            }
        }
    }
    p.normalize();
    p
}
