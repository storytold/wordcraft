use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::para::OBJ;

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character", "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared), "footnotes"?: bool (default true: compare footnotes and endnotes)}"#;

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

/// Changed paragraphs longer than this (in characters) are compared word by word even at
/// character level, so the comparison table stays small.
const MAX_CHAR_DIFF: usize = 2000;

/// Compare the current document (original, or `original` from a file) with a revised one: the
/// result shows the differences as tracked insertions, deletions and formatting changes, labelled
/// with `author`. Options Word offers that the comparison can't honour yet (moves, comments,
/// headers and footers, text boxes, fields) are accepted and listed in the result. Footnotes and
/// endnotes are compared by default (`footnotes`: true), tracking edits, insertions, deletions,
/// and paragraph formatting changes within note stories.
pub(super) fn compare(s: &mut Session, v: &Value) -> CmdResult {
    let mut revised = if let Some(t) = p::str(v, "text") {
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
    let compare_notes = p::bool(v, "footnotes").unwrap_or(true);
    let not_honoured: Vec<&str> =
        ["moves", "comments", "headersFooters", "textBoxes", "fields"].into_iter().filter(|k| v.get(*k).is_some()).collect();
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    let date = crate::cmd::now_iso();
    let mut budget = MAX_COMPARE_CELLS;
    let mut note_budget = MAX_NOTE_ALIGN_BUDGET;
    let mut note_map = std::collections::HashMap::new();
    let mut paired_notes = Vec::new();
    let mut inserted_notes = Vec::new();
    let mut deleted_notes = Vec::new();

    if compare_notes {
        for kind in [wordcraft_doc::PartKind::Footnote, wordcraft_doc::PartKind::Endnote] {
            let orig_ids = document_note_ids(&s.doc, kind);
            let rev_ids = document_note_ids(&revised, kind);
            let (paired, inserted, deleted) = align_notes(&s.doc, &orig_ids, &revised, &rev_ids, kind, &opts, &mut note_budget);
            for (o, r) in paired {
                note_map.insert(r, o);
                paired_notes.push((o, r));
            }
            for r in inserted {
                let blocks = revised.parts.get(&r).map(|p| p.blocks.clone()).unwrap_or_default();
                let new_id = s.doc.add_part(kind, blocks);
                note_map.insert(r, new_id);
                inserted_notes.push((new_id, r));
            }
            deleted_notes.extend(deleted);
        }
        if !note_map.is_empty() {
            revised.body = revised.body.iter().map(|b| Arc::new(remap_note_refs(b, &note_map, 0))).collect();
            for part in revised.parts.values_mut() {
                part.blocks = part.blocks.iter().map(|b| Arc::new(remap_note_refs(b, &note_map, 0))).collect();
            }
            for &(new_id, _) in &inserted_notes {
                if let Some(p) = s.doc.parts.get_mut(&new_id) {
                    p.blocks = p.blocks.iter().map(|b| Arc::new(remap_note_refs(b, &note_map, 0))).collect();
                }
            }
        }
    }

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

    let run = CompareRun { ins, del, opts: &opts, author: &author, date: &date };
    let (out, body_changes) = compare_blocks(&s.doc.parts, &mut s.doc.revisions, &run, &old, &new, &mut budget);
    s.doc.body = out;
    let mut changes = body_changes;

    if compare_notes {
        for (orig_id, rev_id) in paired_notes {
            let orig_blocks = s.doc.parts.get(&orig_id).map(|p| p.blocks.clone()).unwrap_or_default();
            let rev_blocks = revised.parts.get(&rev_id).map(|p| p.blocks.clone()).unwrap_or_default();
            let (note_out, note_changes) = compare_blocks(&s.doc.parts, &mut s.doc.revisions, &run, &orig_blocks, &rev_blocks, &mut budget);
            if let Some(part) = s.doc.parts.get_mut(&orig_id) {
                part.blocks = note_out;
            }
            changes += note_changes;
        }
        for &(new_id, _) in &inserted_notes {
            if let Some(part) = s.doc.parts.get_mut(&new_id) {
                part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.ins = Some(run.ins), 0))).collect();
            }
        }
        for orig_id in deleted_notes {
            if let Some(part) = s.doc.parts.get_mut(&orig_id) {
                part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.del = Some(run.del), 0))).collect();
            }
        }
    }
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

/// Context describing the author, date, options, and change IDs for compare emissions.
struct CompareRun<'a> {
    /// Revision ID for insertions.
    ins: u32,
    /// Revision ID for deletions.
    del: u32,
    /// Comparison options.
    opts: &'a CompareOptions,
    /// Author name for tracked changes.
    author: &'a str,
    /// ISO timestamp for tracked changes.
    date: &'a str,
}

/// Compares two slices of blocks (`old` and `new`) and returns the combined blocks with tracked
/// revisions along with the count of changes.
fn compare_blocks(
    parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>,
    revisions: &mut Vec<wordcraft_doc::Revision>,
    run: &CompareRun<'_>,
    old: &[Arc<Block>],
    new: &[Arc<Block>],
    budget: &mut u64,
) -> (Vec<Arc<Block>>, usize) {
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => run.opts.key(&p.plain_text()),
        // Tables match as a whole, by their text.
        Block::Table(_) => format!("\u{0}table\u{0}{}", run.opts.key(&block_text(b, 0))),
    };
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), budget);
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
                    if !same_objects(parts, pa, pb) {
                        // Same text, but objects (text boxes, pictures…) differ: diffed, so the
                        // original's objects aren't lost.
                        out.push(para_block(word_diff(parts, pa, pb, run.ins, run.del, run.opts, budget)));
                        changes += 1;
                        i += 1;
                        j += 1;
                        k += 1;
                        continue;
                    }
                    let mut p = pb.clone();
                    if run.opts.formatting && pa.text == pb.text {
                        let before = revisions.len();
                        crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, revisions, run.author, run.date);
                        if revisions.len() > before || p != *pb {
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
                let run_len = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run_len).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run_len - dels;
                for x in 0..dels.max(inss) {
                    let (a, b) = (if x < dels { old.get(i + x) } else { None }, if x < inss { new.get(j + x) } else { None });
                    match (a.map(|x| &**x), b.map(|x| &**x)) {
                        (Some(Block::Para(pa)), Some(Block::Para(pb))) => {
                            out.push(para_block(word_diff(parts, pa, pb, run.ins, run.del, run.opts, budget)))
                        }
                        _ => {
                            if let Some(x) = a {
                                out.push(Arc::new(mark_block(x, &|c| c.del = Some(run.del), 0)));
                            }
                            if let Some(x) = b {
                                out.push(Arc::new(mark_block(x, &|c| c.ins = Some(run.ins), 0)));
                            }
                        }
                    }
                    changes += 1;
                }
                i += dels;
                j += inss;
                k += run_len.max(1);
            }
            None => break,
        }
    }
    (out, changes)
}

/// Collects the IDs of footnote or endnote parts in reading order: first walking all inline note
/// references across the body and nested text boxes, then appending any unreferenced note parts.
fn document_note_ids(doc: &Document, kind: wordcraft_doc::PartKind) -> Vec<u32> {
    let note_kind = match kind {
        wordcraft_doc::PartKind::Footnote => wordcraft_doc::para::NoteKind::Footnote,
        wordcraft_doc::PartKind::Endnote => wordcraft_doc::para::NoteKind::Endnote,
        _ => return Vec::new(),
    };
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    doc.objects_in_reading_order(&mut |o| {
        if let InlineObject::NoteRef { kind: k, id, .. } = o
            && *k == note_kind
            && doc.parts.get(id).is_some_and(|p| p.kind == kind)
            && seen.insert(*id)
        {
            ids.push(*id);
        }
    });
    for (&id, part) in &doc.parts {
        if part.kind == kind && seen.insert(id) {
            ids.push(id);
        }
    }
    ids
}

/// Minimum similarity score (on 0..=1000 scale) to allow pairing between two notes.
const NOTE_SIMILARITY_THRESHOLD: u32 = 400;

/// Maximum number of tokens inspected per note when computing similarity.
const MAX_SIMILARITY_TOKENS: usize = 200;

/// Maximum comparison budget allocated specifically for aligning footnote and endnote sequences.
const MAX_NOTE_ALIGN_BUDGET: u64 = 200_000;

/// Maximum number of notes in a single run allowed for dynamic programming matrix allocation.
const MAX_NOTE_ALIGN_RUN: usize = 200;

/// Maximum number of DP matrix cells allowed for note run alignment.
const MAX_NOTE_ALIGN_CELLS: usize = 40_000;

/// Metadata and content summary for aligning footnote and endnote parts.
struct NoteInfo {
    /// Note ID.
    id: u32,
    /// Normalized plain text key of the note content.
    key: String,
    /// Tokens for similarity comparison.
    tokens: Vec<String>,
    /// Location of the note reference: (body_block_index, paragraph_key).
    ref_pos: Option<(usize, String)>,
}

/// Computes a map from note ID to its reference location: (body_block_index, paragraph_key).
fn note_reference_map(doc: &Document, kind: wordcraft_doc::PartKind, opts: &CompareOptions) -> std::collections::HashMap<u32, (usize, String)> {
    let note_kind = match kind {
        wordcraft_doc::PartKind::Footnote => wordcraft_doc::para::NoteKind::Footnote,
        wordcraft_doc::PartKind::Endnote => wordcraft_doc::para::NoteKind::Endnote,
        _ => return std::collections::HashMap::new(),
    };
    let mut map = std::collections::HashMap::new();
    doc.objects_in_reading_order_by_block(&mut |block_idx, para, obj_idx| {
        if let Some(InlineObject::NoteRef { kind: k, id, .. }) = para.objects.get(obj_idx)
            && *k == note_kind
        {
            map.entry(*id).or_insert_with(|| (block_idx, opts.key(&para.plain_text())));
        }
        true
    });
    map
}

/// Extracts [`NoteInfo`] for a list of note IDs in reading order.
fn collect_note_info(doc: &Document, ids: &[u32], kind: wordcraft_doc::PartKind, opts: &CompareOptions) -> Vec<NoteInfo> {
    let mut ref_map = note_reference_map(doc, kind, opts);
    ids.iter()
        .map(|&id| {
            let text = doc.parts.get(&id).map(part_text).unwrap_or_default();
            let key = opts.key(&text);
            let tokens: Vec<String> = if opts.characters {
                key.chars().filter(|c| !c.is_whitespace()).take(MAX_SIMILARITY_TOKENS).map(|c| c.to_string()).collect()
            } else {
                key.split_whitespace().take(MAX_SIMILARITY_TOKENS).map(str::to_string).collect()
            };
            let ref_pos = ref_map.remove(&id);
            NoteInfo { id, key, tokens, ref_pos }
        })
        .collect()
}

/// Computes the length of the longest common subsequence between two slices,
/// charging dynamic programming cells to `budget` and using a bounded greedy fallback
/// when the comparison cell budget is exhausted.
pub(super) fn lcs_len<T: PartialEq>(a: &[T], b: &[T], budget: &mut u64) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let a_mid = a.get(pre..).unwrap_or_default();
    let b_mid = b.get(pre..).unwrap_or_default();
    let suf = a_mid.iter().rev().zip(b_mid.iter().rev()).take_while(|(x, y)| x == y).count();
    let a_core = a_mid.get(..a_mid.len().saturating_sub(suf)).unwrap_or_default();
    let b_core = b_mid.get(..b_mid.len().saturating_sub(suf)).unwrap_or_default();
    let n = a_core.len();
    let m = b_core.len();
    if n == 0 || m == 0 {
        return pre.saturating_add(suf);
    }
    let cells = (n as u64).saturating_mul(m as u64);
    if cells > *budget {
        // Budget exhausted: use bounded forward and backward greedy subsequence matching
        // to find a genuine in-order common subsequence in O(n + m) time without allocating DP tables.
        let mut fwd = 0usize;
        let mut b_idx = 0usize;
        for x in a_core {
            if let Some(pos) = b_core.get(b_idx..).and_then(|s| s.iter().position(|y| x == y)) {
                fwd = fwd.saturating_add(1);
                b_idx = b_idx.saturating_add(pos).saturating_add(1);
                if b_idx >= b_core.len() {
                    break;
                }
            }
        }
        let mut bwd = 0usize;
        let mut b_rev_idx = 0usize;
        for x in a_core.iter().rev() {
            if let Some(pos) = b_core.get(..b_core.len().saturating_sub(b_rev_idx)).and_then(|s| s.iter().rposition(|y| x == y)) {
                bwd = bwd.saturating_add(1);
                b_rev_idx = b_core.len().saturating_sub(pos);
                if b_rev_idx >= b_core.len() {
                    break;
                }
            }
        }
        pre.saturating_add(suf).saturating_add(fwd.max(bwd))
    } else {
        *budget = budget.saturating_sub(cells);
        let mut prev = vec![0u32; m.saturating_add(1)];
        let mut curr = vec![0u32; m.saturating_add(1)];
        for x in a_core {
            for (j, y) in b_core.iter().enumerate() {
                let diag = prev.get(j).copied().unwrap_or(0).saturating_add(1);
                let up = prev.get(j.saturating_add(1)).copied().unwrap_or(0);
                let left = curr.get(j).copied().unwrap_or(0);
                if let Some(c) = curr.get_mut(j.saturating_add(1)) {
                    *c = if x == y { diag } else { up.max(left) };
                }
            }
            std::mem::swap(&mut prev, &mut curr);
        }
        pre.saturating_add(suf).saturating_add(prev.get(m).copied().unwrap_or(0) as usize)
    }
}

/// Scores similarity between an original note and a revised note on a 0..=1000 scale.
///
/// Combines text similarity of the note content with reference location in the body:
/// - Exact text match: 1000.
/// - Text LCS similarity: 0..=1000 (falls back to bounded greedy subsequence when budget is exhausted).
/// - Reference paragraph match bonus: if the body paragraph containing the note reference
///   matches between original and revised, gives a substantial bonus or pairs directly.
fn note_score(a: &NoteInfo, b: &NoteInfo, budget: &mut u64) -> u32 {
    if !a.key.is_empty() && a.key == b.key {
        return 1000;
    }
    let same_ref_para = match (&a.ref_pos, &b.ref_pos) {
        (Some((_, text_a)), Some((_, text_b))) if !text_a.is_empty() => text_a == text_b,
        _ => false,
    };
    let same_ref_idx = match (&a.ref_pos, &b.ref_pos) {
        (Some((idx_a, _)), Some((idx_b, _))) => idx_a == idx_b,
        _ => false,
    };

    let total_tokens = a.tokens.len().saturating_add(b.tokens.len());
    let min_tokens = a.tokens.len().min(b.tokens.len());
    let text_sim = if total_tokens == 0 {
        1000
    } else {
        let max_possible = (2000u64 * min_tokens as u64) / total_tokens as u64;
        if max_possible < NOTE_SIMILARITY_THRESHOLD as u64 && !same_ref_para && !same_ref_idx {
            0
        } else {
            let lcs = lcs_len(&a.tokens, &b.tokens, budget);
            ((2000u64 * lcs as u64) / total_tokens as u64) as u32
        }
    };

    if same_ref_para {
        text_sim.max(500).saturating_add(200).min(1000)
    } else if same_ref_idx && text_sim >= 300 {
        text_sim.saturating_add(100).min(1000)
    } else {
        text_sim
    }
}

/// Aligns a run of original notes and revised notes using dynamic programming over note similarity and reference context.
fn align_note_run(old_notes: &[NoteInfo], new_notes: &[NoteInfo], budget: &mut u64) -> (Vec<(u32, u32)>, Vec<u32>, Vec<u32>) {
    let n = old_notes.len();
    let m = new_notes.len();
    if n == 0 {
        return (Vec::new(), new_notes.iter().map(|info| info.id).collect(), Vec::new());
    }
    if m == 0 {
        return (Vec::new(), Vec::new(), old_notes.iter().map(|info| info.id).collect());
    }
    if n > MAX_NOTE_ALIGN_RUN || m > MAX_NOTE_ALIGN_RUN || n.saturating_mul(m) > MAX_NOTE_ALIGN_CELLS {
        if n == m {
            let paired = old_notes.iter().zip(new_notes.iter()).map(|(o, r)| (o.id, r.id)).collect();
            return (paired, Vec::new(), Vec::new());
        }
        let common = n.min(m);
        let paired = old_notes.iter().take(common).zip(new_notes.iter().take(common)).map(|(o, r)| (o.id, r.id)).collect();
        let deleted = old_notes.get(common..).unwrap_or_default().iter().map(|info| info.id).collect();
        let inserted = new_notes.get(common..).unwrap_or_default().iter().map(|info| info.id).collect();
        return (paired, inserted, deleted);
    }

    let mut scores = vec![vec![0u32; m]; n];
    for (row, a) in scores.iter_mut().zip(old_notes.iter()) {
        for (cell, b) in row.iter_mut().zip(new_notes.iter()) {
            *cell = note_score(a, b, budget);
        }
    }

    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for u in 0..n {
        for v in 0..m {
            let skip_old = dp.get(u).and_then(|r| r.get(v + 1)).copied().unwrap_or(0);
            let skip_new = dp.get(u + 1).and_then(|r| r.get(v)).copied().unwrap_or(0);
            let mut best = skip_old.max(skip_new);
            let s = scores.get(u).and_then(|r| r.get(v)).copied().unwrap_or(0);
            if s >= NOTE_SIMILARITY_THRESHOLD {
                let diag = dp.get(u).and_then(|r| r.get(v)).copied().unwrap_or(0);
                best = best.max(diag.saturating_add(s));
            }
            if let Some(cell) = dp.get_mut(u + 1).and_then(|r| r.get_mut(v + 1)) {
                *cell = best;
            }
        }
    }

    let mut u = n;
    let mut v = m;
    let mut pairs = Vec::new();
    let mut dels = Vec::new();
    let mut inss = Vec::new();

    while u > 0 || v > 0 {
        if u > 0 && v > 0 {
            let s = scores.get(u - 1).and_then(|r| r.get(v - 1)).copied().unwrap_or(0);
            let prev_diag = dp.get(u - 1).and_then(|r| r.get(v - 1)).copied().unwrap_or(0);
            let curr = dp.get(u).and_then(|r| r.get(v)).copied().unwrap_or(0);
            if s >= NOTE_SIMILARITY_THRESHOLD && curr == prev_diag.saturating_add(s) {
                pairs.push((u - 1, v - 1));
                u -= 1;
                v -= 1;
                continue;
            }
        }
        let skip_old = if u > 0 { dp.get(u - 1).and_then(|r| r.get(v)).copied().unwrap_or(0) } else { 0 };
        let curr = dp.get(u).and_then(|r| r.get(v)).copied().unwrap_or(0);
        if u > 0 && (v == 0 || curr == skip_old) {
            dels.push(u - 1);
            u -= 1;
        } else if v > 0 {
            inss.push(v - 1);
            v -= 1;
        } else {
            break;
        }
    }

    pairs.reverse();
    dels.reverse();
    inss.reverse();

    // Fall back to 1-to-1 positional pairing only when counts match (n == m) and no pair met threshold.
    if pairs.is_empty() && n == m {
        let paired = old_notes.iter().zip(new_notes.iter()).map(|(o, r)| (o.id, r.id)).collect();
        return (paired, Vec::new(), Vec::new());
    }

    let mut paired_out = Vec::new();
    for (u_idx, v_idx) in pairs {
        if let (Some(o), Some(r)) = (old_notes.get(u_idx), new_notes.get(v_idx)) {
            paired_out.push((o.id, r.id));
        }
    }
    let deleted_out = dels.into_iter().filter_map(|idx| old_notes.get(idx).map(|info| info.id)).collect();
    let inserted_out = inss.into_iter().filter_map(|idx| new_notes.get(idx).map(|info| info.id)).collect();

    (paired_out, inserted_out, deleted_out)
}

/// Pairs original and revised note IDs using sequence alignment over note plain text and body reference positions.
///
/// Returns `(paired, inserted, deleted)`:
/// - `paired`: `(orig_id, rev_id)` pairs to be diffed against each other.
/// - `inserted`: `rev_id`s that represent newly added notes.
/// - `deleted`: `orig_id`s that have no revised counterpart.
fn align_notes(
    orig_doc: &Document,
    orig_ids: &[u32],
    rev_doc: &Document,
    rev_ids: &[u32],
    kind: wordcraft_doc::PartKind,
    opts: &CompareOptions,
    budget: &mut u64,
) -> (Vec<(u32, u32)>, Vec<u32>, Vec<u32>) {
    let old_info = collect_note_info(orig_doc, orig_ids, kind, opts);
    let new_info = collect_note_info(rev_doc, rev_ids, kind, opts);
    let keys_orig: Vec<String> = old_info.iter().map(|n| n.key.clone()).collect();
    let keys_rev: Vec<String> = new_info.iter().map(|n| n.key.clone()).collect();
    let ops = lcs_ops(&keys_orig, &keys_rev, budget);
    let mut paired = Vec::new();
    let mut inserted = Vec::new();
    let mut deleted = Vec::new();
    let (mut i, mut j, mut k) = (0, 0, 0);
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                if let (Some(&o), Some(&r)) = (orig_ids.get(i), rev_ids.get(j)) {
                    paired.push((o, r));
                }
                i += 1;
                j += 1;
                k += 1;
            }
            Some(_) => {
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                let old_slice = old_info.get(i..i + dels).unwrap_or_default();
                let new_slice = new_info.get(j..j + inss).unwrap_or_default();
                let (p, ins, del) = align_note_run(old_slice, new_slice, budget);
                paired.extend(p);
                inserted.extend(ins);
                deleted.extend(del);
                i += dels;
                j += inss;
                k += run.max(1);
            }
            None => break,
        }
    }
    (paired, inserted, deleted)
}

/// Remaps note references within a block and any nested tables using `map`.
fn remap_note_refs(b: &Block, map: &std::collections::HashMap<u32, u32>, depth: usize) -> Block {
    match b {
        Block::Para(p) => {
            let mut p = p.clone();
            for o in &mut p.objects {
                if let InlineObject::NoteRef { id, .. } = o
                    && let Some(&new_id) = map.get(id)
                {
                    *id = new_id;
                }
            }
            Block::Para(p)
        }
        Block::Table(t) => {
            let mut t = t.clone();
            if depth < 16 {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    c.blocks = c.blocks.iter().map(|x| Arc::new(remap_note_refs(x, map, depth + 1))).collect();
                }
            }
            Block::Table(t)
        }
    }
}

/// Returns the plain text of a part's blocks.
fn part_text(p: &wordcraft_doc::Part) -> String {
    p.blocks.iter().map(|b| block_text(b, 0)).collect::<Vec<_>>().join("\n")
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

/// Computes a comparison key for an inline object, taking text box content from parts into account.
fn obj_key(parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>, o: &InlineObject) -> Key {
    let texts = o.text_boxes().into_iter().map(|id| parts.get(&id).map(part_text).unwrap_or_default()).collect();
    let mut o = o.clone();
    for story in o.text_box_slots() {
        *story = None;
    }
    Key::Obj(Box::new(o), texts)
}

/// Whether two paragraphs hold the same objects, as compared.
fn same_objects(parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>, a: &Paragraph, b: &Paragraph) -> bool {
    a.objects.len() == b.objects.len() && a.objects.iter().zip(&b.objects).all(|(x, y)| x == y || obj_key(parts, x) == obj_key(parts, y))
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

/// Diffs two paragraphs word-by-word (or character-by-character) and returns a new paragraph
/// containing tracked deletions and insertions.
fn word_diff(
    parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>,
    a: &Paragraph,
    b: &Paragraph,
    ins: u32,
    del: u32,
    opts: &CompareOptions,
    budget: &mut u64,
) -> Paragraph {
    let (at, bt) = (a.plain_text(), b.plain_text());
    let chars = opts.characters && at.chars().count().max(bt.chars().count()) <= MAX_CHAR_DIFF;
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let key = |t: &Tok<'_>| match t {
        Tok::Text(t) => Key::Text(opts.key(t)),
        Tok::Obj(o) => obj_key(parts, o),
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
