use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::para::OBJ;

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character", "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared)}"#;

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
    fn key(&self, t: &str) -> String {
        let t = if self.white_space { t.to_string() } else { t.split_whitespace().collect::<Vec<_>>().join(" ") };
        if self.case { t } else { t.to_lowercase() }
    }
}

/// Changed paragraphs longer than this (in characters) are compared word by word even at
/// character level, so the comparison table stays small.
const MAX_CHAR_DIFF: usize = 2000;

/// Compare the current document (original, or `original` from a file) with a revised one: the
/// result shows the differences as tracked insertions, deletions and formatting changes, labelled
/// with `author`. Options Word offers that the comparison can't honour yet (moves, comments,
/// headers and footers, footnotes, text boxes, fields) are accepted and listed in the result.
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
    let not_honoured: Vec<&str> =
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
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), &mut budget);
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
                        out.push(para_block(word_diff(&s.doc, pa, pb, ins, del, &opts, &mut budget)));
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
                // A run of deletions and insertions: changed paragraphs pair up by text similarity
                // and are diffed word by word (or by character); unpartnered blocks are deleted
                // or inserted whole.
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                let old_slice = old.get(i..i + dels).unwrap_or_default();
                let new_slice = new.get(j..j + inss).unwrap_or_default();
                let pairs = align_block_run(old_slice, new_slice, &opts, &mut budget);
                for pair in pairs {
                    match pair {
                        RunPair::Diff(u, v) => {
                            if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (old_slice.get(u).map(|x| &**x), new_slice.get(v).map(|x| &**x)) {
                                out.push(para_block(word_diff(&s.doc, pa, pb, ins, del, &opts, &mut budget)));
                                changes += 1;
                            }
                        }
                        RunPair::Del(u) => {
                            if let Some(x) = old_slice.get(u) {
                                out.push(Arc::new(mark_block(x, &|c| c.del = Some(del), 0)));
                                changes += 1;
                            }
                        }
                        RunPair::Ins(v) => {
                            if let Some(x) = new_slice.get(v) {
                                out.push(Arc::new(mark_block(x, &|c| c.ins = Some(ins), 0)));
                                changes += 1;
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
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    Ok(json!({"changes": changes, "author": author, "notHonoured": not_honoured}))
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

/// A copy of the block with every run (in tables, every cell's) changed by `f`.
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
/// As an exception, when no pair in an edit run meets this threshold and both sides have the
/// same number of blocks (`n == m`), paragraph blocks fall back to 1-to-1 positional pairing
/// so that whole-run replacements preserve paragraph slots and reject or accept cleanly. This
/// fallback does not apply when block counts differ.
const SIMILARITY_THRESHOLD: u32 = 400;

/// Maximum number of DP cells (`old_len * new_len`) computed when aligning an edit run.
///
/// When an edit run exceeds this limit, alignment falls back to 1-to-1 positional pairing
/// rather than computing the full similarity matrix, bounding compute time on very large runs
/// at the cost of reintroducing mispairing risk for oversized runs.
const MAX_RUN_ALIGN_CELLS: usize = 1_000_000;

/// Maximum number of word tokens considered per paragraph when computing similarity scores.
const MAX_SIM_WORDS: usize = 500;

/// Maximum number of characters considered per paragraph when computing character similarity scores.
const MAX_SIM_CHARS: usize = 1000;

/// Precomputed representation of a block for fast paragraph similarity scoring.
enum PrecomputedBlock<'a> {
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

/// Extracts precomputed token representations for a sequence of blocks.
///
/// Normalizes paragraph plain text via `opts.key`, pre-scans for CJK ideographs,
/// and extracts word and character tokens bounded by `MAX_SIM_WORDS` and `MAX_SIM_CHARS`.
/// Non-paragraph blocks are marked as `NonPara`.
fn precompute_blocks<'a>(blocks: &'a [Arc<Block>], keys: &'a [Option<String>]) -> Vec<PrecomputedBlock<'a>> {
    blocks
        .iter()
        .zip(keys)
        .map(|(b, k)| match (&**b, k) {
            (Block::Para(_), Some(key)) => {
                let chars: Vec<char> = key.chars().filter(|c| !c.is_whitespace()).take(MAX_SIM_CHARS).collect();
                let has_cjk = chars.iter().copied().any(wordcraft_fonts::is_cjk);
                let words: Vec<&'a str> = key.split_whitespace().take(MAX_SIM_WORDS).collect();
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
/// Similarity is scored using `(2000 * lcs) / total`, where `total = len_a + len_b`.
fn para_similarity(
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
    if use_chars { score_tokens(ch_a, ch_b, dp_buf, budget) } else { score_tokens(w_a, w_b, dp_buf, budget) }
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
/// Strips common prefix and suffix tokens before building the DP table. When interior tokens
/// remain, charges the core DP cell count (`a_core.len() * b_core.len()`) to `budget`,
/// returning `None` if the budget is exceeded.
///
/// Uses `dp_buf` to compute LCS length in `O(short.len() * long.len())` time and
/// `O(short.len())` space without per-call allocations. Returns `Some(pre + suf)` when either
/// core sequence is empty.
fn lcs_len<T: PartialEq>(a: &[T], b: &[T], dp_buf: &mut Vec<u32>, budget: &mut u64) -> Option<usize> {
    if a.is_empty() || b.is_empty() {
        return Some(0);
    }
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let a_mid = a.get(pre..).unwrap_or_default();
    let b_mid = b.get(pre..).unwrap_or_default();
    let suf = a_mid.iter().rev().zip(b_mid.iter().rev()).take_while(|(x, y)| x == y).count();
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

/// Longest sequence the comparison table is built for; the rest of a longer sequence is
/// compared as deleted and inserted, never dropped.
const MAX_LCS: usize = 4000;

/// Comparison table cells one compare may fill over all its tables (blocks and every changed
/// paragraph pair); once spent, the remaining middles are compared as deleted and inserted.
const MAX_COMPARE_CELLS: u64 = 50_000_000;

/// LCS edit script between two sequences: the common start and end are matched first, then the
/// middle (bounded size). The middle's table is charged to `budget`; when it doesn't fit, the
/// middle is deleted and inserted whole.
pub(super) fn lcs_ops<T: PartialEq>(a: &[T], b: &[T], budget: &mut u64) -> Vec<Op> {
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (ra, rb) = (a.get(pre..).unwrap_or_default(), b.get(pre..).unwrap_or_default());
    let suf = ra.iter().rev().zip(rb.iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (ra.get(..ra.len() - suf).unwrap_or_default(), rb.get(..rb.len() - suf).unwrap_or_default());
    let mut ops = vec![Op::Same; pre];
    let (mut n, mut m) = (a.len().min(MAX_LCS), b.len().min(MAX_LCS));
    let cells = (n as u64).saturating_mul(m as u64);
    if cells > *budget {
        (n, m) = (0, 0);
    } else {
        *budget -= cells;
    }
    let mut t = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let v = if a.get(i) == b.get(j) {
                t.get(i + 1).and_then(|r| r.get(j + 1)).copied().unwrap_or(0) + 1
            } else {
                t.get(i + 1).and_then(|r| r.get(j)).copied().unwrap_or(0).max(t.get(i).and_then(|r| r.get(j + 1)).copied().unwrap_or(0))
            };
            if let Some(c) = t.get_mut(i).and_then(|r| r.get_mut(j)) {
                *c = v;
            }
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a.get(i) == b.get(j) {
            ops.push(Op::Same);
            i += 1;
            j += 1;
        } else if t.get(i + 1).and_then(|r| r.get(j)) >= t.get(i).and_then(|r| r.get(j + 1)) {
            ops.push(Op::Del);
            i += 1;
        } else {
            ops.push(Op::Ins);
            j += 1;
        }
    }
    ops.extend(std::iter::repeat_n(Op::Del, a.len() - i));
    ops.extend(std::iter::repeat_n(Op::Ins, b.len() - j));
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

fn word_diff(doc: &Document, a: &Paragraph, b: &Paragraph, ins: u32, del: u32, opts: &CompareOptions, budget: &mut u64) -> Paragraph {
    let (at, bt) = (a.plain_text(), b.plain_text());
    let chars = opts.characters && at.chars().count().max(bt.chars().count()) <= MAX_CHAR_DIFF;
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let key = |t: &Tok<'_>| match t {
        Tok::Text(t) => Key::Text(opts.key(t)),
        Tok::Obj(o) => obj_key(doc, o),
    };
    let ops = lcs_ops(&ta.iter().map(key).collect::<Vec<_>>(), &tb.iter().map(key).collect::<Vec<_>>(), budget);
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
                put(&mut p, ta.get(i), &CharProps { del: Some(del), ..base.clone() });
                i += 1;
            }
            Op::Ins => {
                put(&mut p, tb.get(j), &CharProps { ins: Some(ins), ..base.clone() });
                j += 1;
            }
        }
    }
    p
}
