use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::para::OBJ;

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character" (character: paragraphs over 2000 characters fall back to word; see characterFallback/notHonoured), "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared), "footnotes"?: bool (default true: compare footnotes and endnotes)}"#;

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
    revisions: &'a mut Vec<wordcraft_doc::Revision>,
    author: &'a str,
    date: &'a str,
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
/// When character-level comparison is requested (`"level": "character"`) but any paragraph exceeds
/// [`MAX_CHAR_DIFF`] characters, it falls back to word-level diff for that paragraph, surfaces the
/// fallback count in `"characterFallback"`, and includes `"level"` in `"notHonoured"`.
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
    let mut not_honoured: Vec<&str> =
        ["moves", "comments", "headersFooters", "textBoxes", "fields"].into_iter().filter(|k| v.get(*k).is_some()).collect();
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    let date = crate::cmd::now_iso();
    let mut budget = MAX_COMPARE_CELLS;
    let mut note_budget = MAX_NOTE_ALIGN_BUDGET;

    reconcile_media(&mut s.doc, &mut revised);

    let mut note_map = std::collections::HashMap::new();
    let mut paired_notes = Vec::new();
    let mut inserted_notes = Vec::new();
    let mut deleted_notes = Vec::new();

    let mut next_orphan_id = s.doc.parts.keys().next_back().copied().unwrap_or(0).saturating_add(10_000);
    for kind in [wordcraft_doc::PartKind::Footnote, wordcraft_doc::PartKind::Endnote] {
        let orig_ids = document_note_ids(&s.doc, kind);
        let rev_ids = document_note_ids(&revised, kind);
        let (paired, inserted, deleted) = align_notes(&s.doc, &orig_ids, &revised, &rev_ids, kind, &opts, &mut note_budget);
        for (o, r) in paired {
            note_map.insert(r, o);
            if compare_notes {
                paired_notes.push((o, r));
            }
        }
        if compare_notes {
            for r in inserted {
                let blocks = revised.parts.get(&r).map(|p| p.blocks.clone()).unwrap_or_default();
                let frag_blocks: Vec<Block> = blocks.iter().map(|b| (**b).clone()).collect();
                let frag = wordcraft_doc::edit::Fragment {
                    parts: revised.parts.iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::TextBox).map(|(k, p)| (*k, p.clone())).collect(),
                    media: revised.media_of_blocks(&frag_blocks),
                    blocks: frag_blocks,
                };
                let adopted = s.doc.adopt_fragment(&frag);
                let new_id = s.doc.add_part(kind, adopted.into_iter().map(Arc::new).collect());
                note_map.insert(r, new_id);
                inserted_notes.push((new_id, r));
            }
            deleted_notes.extend(deleted);
        } else {
            for r in inserted {
                let orphan_id = next_orphan_id;
                next_orphan_id = next_orphan_id.saturating_add(1);
                note_map.insert(r, orphan_id);
            }
        }
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

    let mut char_fallbacks = 0usize;
    let run = CompareRun { ins, del, opts: &opts, author: &author, date: &date };
    let (out, body_changes) = compare_blocks(&s.doc.parts, &mut s.doc.revisions, &run, &old, &new, &mut budget, &mut char_fallbacks);
    s.doc.body = out;
    let mut changes = body_changes;

    if compare_notes {
        for (orig_id, rev_id) in paired_notes {
            let orig_blocks = s.doc.parts.get(&orig_id).map(|p| p.blocks.clone()).unwrap_or_default();
            let rev_blocks_raw = revised.parts.get(&rev_id).map(|p| p.blocks.clone()).unwrap_or_default();
            let frag_blocks: Vec<Block> = rev_blocks_raw.iter().map(|b| (**b).clone()).collect();
            let frag = wordcraft_doc::edit::Fragment {
                parts: revised.parts.iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::TextBox).map(|(k, p)| (*k, p.clone())).collect(),
                media: revised.media_of_blocks(&frag_blocks),
                blocks: frag_blocks,
            };
            let rev_blocks: Vec<Arc<Block>> = if revised.parts.values().any(|p| p.kind == wordcraft_doc::PartKind::TextBox) {
                s.doc.adopt_fragment(&frag).into_iter().map(Arc::new).collect()
            } else {
                rev_blocks_raw
            };
            let (note_out, note_changes) =
                compare_blocks(&s.doc.parts, &mut s.doc.revisions, &run, &orig_blocks, &rev_blocks, &mut budget, &mut char_fallbacks);
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
    char_fallbacks: &mut usize,
) -> (Vec<Arc<Block>>, usize) {
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => run.opts.key(&p.plain_text()),
        // Tables match as a whole, by their text.
        Block::Table(_) => format!("\u{0}table\u{0}{}", run.opts.key(&block_text(b, 0))),
    };
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), budget);
    let mut ctx = DiffCtx { ins: run.ins, del: run.del, opts: run.opts, budget, char_fallbacks, revisions, author: run.author, date: run.date };
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
                    if !same_objects(parts, pa, pb) || !same_para_links(pa, pb) {
                        // Same text, but objects (text boxes, pictures…) or hyperlink targets differ:
                        // diffed, so the original's objects and links aren't lost.
                        out.push(para_block(word_diff(parts, pa, pb, &mut ctx)));
                        changes += 1;
                        i += 1;
                        j += 1;
                        k += 1;
                        continue;
                    }
                    let mut p = pb.clone();
                    if run.opts.formatting && pa.text != pb.text {
                        // Case or spacing differs only: track paragraph-level formatting.
                        if crate::cmd::fmt_revisions::compare_para_formatting(&pa.props, &pa.mark, &mut p, ctx.revisions, run.author, run.date) {
                            changes += 1;
                        }
                    } else if run.opts.formatting {
                        let before = ctx.revisions.len();
                        crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, ctx.revisions, run.author, run.date);
                        if ctx.revisions.len() > before || p != *pb {
                            changes += 1;
                        }
                    }
                    out.push(para_block(p));
                } else if let (Some(Block::Table(ta)), Some(Block::Table(tb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    let mut table_matched = false;
                    let mut table_changes = 0usize;
                    let mut new_rows = Vec::with_capacity(tb.rows.len());
                    if ta.rows.len() == tb.rows.len() && (!run.opts.formatting || ta.grid == tb.grid) {
                        let mut rows_match = true;
                        for (ra, rb) in ta.rows.iter().zip(&tb.rows) {
                            if ra.cells.len() != rb.cells.len() {
                                rows_match = false;
                                break;
                            }
                            let mut new_cells = Vec::with_capacity(rb.cells.len());
                            for (ca, cb) in ra.cells.iter().zip(&rb.cells) {
                                if ca.props.span != cb.props.span || ca.props.vmerge != cb.props.vmerge {
                                    rows_match = false;
                                    break;
                                }
                                let (cell_blocks, cell_changes) =
                                    compare_blocks(parts, ctx.revisions, run, &ca.blocks, &cb.blocks, ctx.budget, ctx.char_fallbacks);
                                table_changes = table_changes.saturating_add(cell_changes);
                                let mut cell_props = cb.props.clone();
                                if run.opts.formatting
                                    && crate::cmd::fmt_revisions::compare_cell_props(&ca.props, &mut cell_props, ctx.revisions, run.author, run.date)
                                {
                                    table_changes = table_changes.saturating_add(1);
                                }
                                new_cells.push(wordcraft_doc::Cell { props: cell_props, blocks: cell_blocks, controls: cb.controls.clone() });
                            }
                            if !rows_match {
                                break;
                            }
                            let mut row_props = rb.props.clone();
                            if run.opts.formatting
                                && crate::cmd::fmt_revisions::compare_row_props(&ra.props, &mut row_props, ctx.revisions, run.author, run.date)
                            {
                                table_changes = table_changes.saturating_add(1);
                            }
                            new_rows.push(wordcraft_doc::Row { props: row_props, cells: new_cells, controls: rb.controls.clone() });
                        }
                        table_matched = rows_match;
                    }
                    if table_matched {
                        let mut table_props = tb.props.clone();
                        if run.opts.formatting
                            && crate::cmd::fmt_revisions::compare_table_props(&ta.props, &mut table_props, ctx.revisions, run.author, run.date)
                        {
                            table_changes = table_changes.saturating_add(1);
                        }
                        changes = changes.saturating_add(table_changes);
                        let final_grid = if run.opts.formatting { tb.grid.clone() } else { ta.grid.clone() };
                        out.push(Arc::new(Block::Table(wordcraft_doc::Table {
                            props: table_props,
                            grid: final_grid,
                            rows: new_rows,
                            controls: tb.controls.clone(),
                        })));
                    } else {
                        if let Some(x) = a {
                            out.push(Arc::new(mark_block(x, &|c| c.del = Some(run.del), 0)));
                        }
                        if let Some(x) = b {
                            out.push(Arc::new(mark_block(x, &|c| c.ins = Some(run.ins), 0)));
                        }
                        changes += 1;
                    }
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
                        (Some(Block::Para(pa)), Some(Block::Para(pb))) => out.push(para_block(word_diff(parts, pa, pb, &mut ctx))),
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

/// Maximum look-ahead, in tokens, for the greedy fallback match.
const GREEDY_WINDOW: usize = 16;

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
        // (look-ahead of GREEDY_WINDOW tokens) to find a genuine in-order common subsequence
        // in O(GREEDY_WINDOW * n) time without allocating DP tables.
        let mut fwd = 0usize;
        let mut b_idx = 0usize;
        for x in a_core {
            if let Some(pos) = b_core.get(b_idx..).and_then(|s| s.iter().take(GREEDY_WINDOW).position(|y| x == y)) {
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
            if let Some(pos) = b_core
                .get(..b_core.len().saturating_sub(b_rev_idx))
                .and_then(|s| s.iter().rev().take(GREEDY_WINDOW).position(|y| x == y).map(|off| s.len().saturating_sub(1).saturating_sub(off)))
            {
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

/// Remap media references in an inline object.
fn remap_media_object(o: &mut InlineObject, map: &std::collections::BTreeMap<String, String>, depth: usize) {
    match o {
        InlineObject::Image { media, .. } => {
            if let Some(k) = map.get(media.as_str()) {
                *media = k.clone();
            }
        }
        InlineObject::Graphic { graphic, .. } => {
            let mapped = |m: &str| map.get(m).filter(|k| k.as_str() != m);
            if graphic.items.iter().any(|i| matches!(i, wordcraft_doc::graphic::GraphicItem::Image { media, .. } if mapped(media).is_some())) {
                for i in &mut Arc::make_mut(graphic).items {
                    if let wordcraft_doc::graphic::GraphicItem::Image { media, .. } = i
                        && let Some(k) = map.get(media.as_str())
                    {
                        *media = k.clone();
                    }
                }
            }
        }
        InlineObject::Group { children, .. } if depth < 8 => {
            for child in children {
                remap_media_object(&mut child.obj, map, depth + 1);
            }
        }
        _ => {}
    }
}

/// Remap media references in a block and any nested tables up to depth 16.
fn remap_media_block(b: &mut Block, map: &std::collections::BTreeMap<String, String>, depth: usize) {
    match b {
        Block::Para(p) => {
            for o in &mut p.objects {
                remap_media_object(o, map, 0);
            }
        }
        Block::Table(t) => {
            if depth < 16 {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    if let Some(ref mut pic) = c.props.picture
                        && let Some(k) = map.get(&pic.media)
                    {
                        pic.media = k.clone();
                    }
                    for cb in &mut c.blocks {
                        remap_media_block(Arc::make_mut(cb), map, depth + 1);
                    }
                }
            }
        }
    }
}

/// Reconciles media from `revised` into `target`, remapping colliding media keys
/// that have different content to newly allocated keys in `target`.
fn reconcile_media(target: &mut wordcraft_doc::Document, revised: &mut wordcraft_doc::Document) {
    let mut media_remap = std::collections::BTreeMap::new();
    for (k, v) in &revised.media {
        if let Some(existing) = target.media.get(k) {
            if existing.as_slice() != v.as_slice() {
                let ext = k.rsplit_once('.').map_or("png", |(_, ext)| ext);
                let mut i = target.media.len().saturating_add(revised.media.len()).saturating_add(1);
                let mut new_key = format!("image{i}.{ext}");
                while target.media.contains_key(&new_key) || revised.media.contains_key(&new_key) {
                    i = i.saturating_add(1);
                    new_key = format!("image{i}.{ext}");
                }
                target.media.insert(new_key.clone(), v.clone());
                media_remap.insert(k.clone(), new_key);
            }
        } else {
            target.media.insert(k.clone(), v.clone());
        }
    }
    if !media_remap.is_empty() {
        for b in &mut revised.body {
            remap_media_block(Arc::make_mut(b), &media_remap, 0);
        }
        for part in revised.parts.values_mut() {
            for b in &mut part.blocks {
                remap_media_block(Arc::make_mut(b), &media_remap, 0);
            }
        }
        for (old_key, new_key) in media_remap {
            if let Some(data) = revised.media.remove(&old_key) {
                revised.media.insert(new_key, data);
            }
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

/// A piece of text or an inline object with its character properties.
#[derive(Clone)]
enum TokPiece<'a> {
    /// Text slice and its character properties.
    Text(String, CharProps),
    /// Inline object and its character properties.
    Obj(&'a InlineObject, CharProps),
}

/// A token of a changed paragraph: a word (or character), or an inline object without text
/// of its own (a text box anchor, a picture, a bookmark…), which is compared and kept whole.
#[derive(Clone)]
struct Tok<'a> {
    pieces: Vec<TokPiece<'a>>,
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
/// between them, tracking each portion's original character properties.
fn tokens(p: &Paragraph, characters: bool) -> Vec<Tok<'_>> {
    fn flush_text(cur_pieces: &mut Vec<TokPiece<'_>>, cur_text: &mut String, cur_props: &mut Option<CharProps>) {
        if !cur_text.is_empty() {
            let props = cur_props.take().unwrap_or_default();
            cur_pieces.push(TokPiece::Text(std::mem::take(cur_text), props));
        }
    }

    fn flush_tok<'a>(out: &mut Vec<Tok<'a>>, cur_pieces: &mut Vec<TokPiece<'a>>, cur_text: &mut String, cur_props: &mut Option<CharProps>) {
        flush_text(cur_pieces, cur_text, cur_props);
        if !cur_pieces.is_empty() {
            out.push(Tok { pieces: std::mem::take(cur_pieces) });
        }
    }

    let mut out: Vec<Tok<'_>> = Vec::new();
    let mut cur_pieces: Vec<TokPiece<'_>> = Vec::new();
    let mut cur_text = String::new();
    let mut cur_props: Option<CharProps> = None;
    let mut objects = p.objects.iter();

    let mut runs = p.run_ranges().peekable();
    let mut byte_off = 0;
    for c in p.text.chars() {
        let char_len = c.len_utf8();
        while runs.peek().is_some_and(|(r, _)| r.end <= byte_off) {
            runs.next();
        }
        let mut props = runs.peek().map_or_else(|| p.mark.clone(), |(_, pr)| (*pr).clone());
        props.ins = None;
        props.del = None;
        props.fmt_change = None;
        byte_off += char_len;

        if c == OBJ {
            flush_tok(&mut out, &mut cur_pieces, &mut cur_text, &mut cur_props);
            match objects.next() {
                Some(o) if o.plain_text().is_empty() => {
                    out.push(Tok { pieces: vec![TokPiece::Obj(o, props)] });
                }
                Some(o) => {
                    for ch in o.plain_text().chars() {
                        if cur_props.as_ref() != Some(&props) {
                            flush_text(&mut cur_pieces, &mut cur_text, &mut cur_props);
                            cur_props = Some(props.clone());
                        }
                        cur_text.push(ch);
                        if characters || ch == ' ' {
                            flush_tok(&mut out, &mut cur_pieces, &mut cur_text, &mut cur_props);
                        }
                    }
                }
                None => {
                    log::warn!("compare: an object marker without its object");
                    if cur_props.as_ref() != Some(&props) {
                        flush_text(&mut cur_pieces, &mut cur_text, &mut cur_props);
                        cur_props = Some(props.clone());
                    }
                    cur_text.push(char::REPLACEMENT_CHARACTER);
                    if characters {
                        flush_tok(&mut out, &mut cur_pieces, &mut cur_text, &mut cur_props);
                    }
                }
            }
            continue;
        }

        if cur_props.as_ref() != Some(&props) {
            flush_text(&mut cur_pieces, &mut cur_text, &mut cur_props);
            cur_props = Some(props);
        }
        cur_text.push(c);
        if characters || c == ' ' {
            flush_tok(&mut out, &mut cur_pieces, &mut cur_text, &mut cur_props);
        }
    }

    flush_tok(&mut out, &mut cur_pieces, &mut cur_text, &mut cur_props);
    out
}

/// Add a token at the end of `p` with revisions applied if specified.
fn put_tok(p: &mut Paragraph, t: Option<&Tok<'_>>, rev: Option<u32>, is_ins: bool) {
    let Some(t) = t else { return };
    for piece in &t.pieces {
        let at = p.len();
        match piece {
            TokPiece::Text(text, props) => {
                let mut props = props.clone();
                if let Some(r) = rev {
                    if is_ins {
                        props.ins = Some(r);
                    } else {
                        props.del = Some(r);
                    }
                }
                let _ = p.insert_text(at, text, &props);
            }
            TokPiece::Obj(o, props) => {
                let mut props = props.clone();
                if let Some(r) = rev {
                    if is_ins {
                        props.ins = Some(r);
                    } else {
                        props.del = Some(r);
                    }
                }
                let _ = p.insert_object(at, (*o).clone(), &props);
            }
        }
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

/// Looks up character properties in token `tok` at character offset `off`.
fn props_at_offset(tok: &Tok<'_>, off: usize) -> CharProps {
    let mut cur = 0usize;
    for piece in &tok.pieces {
        let piece_len = match piece {
            TokPiece::Text(t, _) => t.chars().count(),
            TokPiece::Obj(o, _) => o.plain_text().chars().count().max(1),
        };
        if off < cur.saturating_add(piece_len) {
            return match piece {
                TokPiece::Text(_, p) => p.clone(),
                TokPiece::Obj(_, p) => p.clone(),
            };
        }
        cur = cur.saturating_add(piece_len);
    }
    tok.pieces
        .last()
        .map(|p| match p {
            TokPiece::Text(_, pr) => pr.clone(),
            TokPiece::Obj(_, pr) => pr.clone(),
        })
        .unwrap_or_default()
}

/// Splits revised text pieces in `b` at formatting boundaries of original token `a`.
fn split_tok_at_boundaries<'a>(a: &Tok<'_>, b: &mut Tok<'a>) {
    let mut a_cuts = Vec::new();
    let mut a_off = 0usize;
    for p in &a.pieces {
        let len = match p {
            TokPiece::Text(t, _) => t.chars().count(),
            TokPiece::Obj(o, _) => o.plain_text().chars().count().max(1),
        };
        a_off = a_off.saturating_add(len);
        a_cuts.push(a_off);
    }

    let mut new_pieces = Vec::new();
    let mut cur_off = 0usize;

    for piece in std::mem::take(&mut b.pieces) {
        match piece {
            TokPiece::Text(text, props) => {
                let char_count = text.chars().count();
                let piece_end = cur_off.saturating_add(char_count);
                let cuts: Vec<usize> = a_cuts.iter().copied().filter(|&c| c > cur_off && c < piece_end).collect();

                if cuts.is_empty() {
                    new_pieces.push(TokPiece::Text(text, props));
                } else {
                    let chars: Vec<char> = text.chars().collect();
                    let mut seg_start = 0usize;
                    for cut in cuts {
                        let seg_end = cut.saturating_sub(cur_off);
                        if seg_end > seg_start && seg_end <= chars.len() {
                            let seg: String = chars[seg_start..seg_end].iter().collect();
                            new_pieces.push(TokPiece::Text(seg, props.clone()));
                            seg_start = seg_end;
                        }
                    }
                    if seg_start < chars.len() {
                        let seg: String = chars[seg_start..].iter().collect();
                        new_pieces.push(TokPiece::Text(seg, props));
                    }
                }
                cur_off = piece_end;
            }
            TokPiece::Obj(o, props) => {
                let len = o.plain_text().chars().count().max(1);
                cur_off = cur_off.saturating_add(len);
                new_pieces.push(TokPiece::Obj(o, props));
            }
        }
    }
    b.pieces = new_pieces;
}

/// Compares character formatting between original token `a` and revised token `b`, recording
/// tracked formatting revisions on `b`'s pieces when character properties differ.
fn apply_tok_formatting(a: &Tok<'_>, b: &mut Tok<'_>, ctx: &mut DiffCtx<'_>) {
    split_tok_at_boundaries(a, b);
    let mut off = 0;
    for piece in &mut b.pieces {
        let old_props = props_at_offset(a, off);
        let piece_len = match piece {
            TokPiece::Text(t, props) => {
                crate::cmd::fmt_revisions::compare_char_props(&old_props, props, ctx.revisions, ctx.author, ctx.date);
                t.chars().count()
            }
            TokPiece::Obj(o, props) => {
                crate::cmd::fmt_revisions::compare_char_props(&old_props, props, ctx.revisions, ctx.author, ctx.date);
                o.plain_text().chars().count().max(1)
            }
        };
        off = off.saturating_add(piece_len);
    }
}

/// Returns the hyperlink target for a token piece if one is defined.
fn piece_link<'a>(p: &'a TokPiece<'_>) -> Option<&'a str> {
    match p {
        TokPiece::Text(_, pr) | TokPiece::Obj(_, pr) => pr.link.as_deref(),
    }
}

/// Returns whether two tokens have identical hyperlink targets across their pieces.
fn same_tok_links(a: &Tok<'_>, b: &Tok<'_>) -> bool {
    if a.pieces.len() != b.pieces.len() {
        let a_has = a.pieces.iter().any(|p| piece_link(p).is_some());
        let b_has = b.pieces.iter().any(|p| piece_link(p).is_some());
        return !a_has && !b_has;
    }
    a.pieces.iter().zip(&b.pieces).all(|(pa, pb)| piece_link(pa) == piece_link(pb))
}

/// Returns whether two paragraphs have identical hyperlink targets across their runs.
fn same_para_links(a: &Paragraph, b: &Paragraph) -> bool {
    let a_has = a.runs.iter().any(|r| r.props.link.is_some());
    let b_has = b.runs.iter().any(|r| r.props.link.is_some());
    if !a_has && !b_has {
        return true;
    }
    if a_has != b_has {
        return false;
    }
    let a_links: Vec<(&str, Option<&str>)> = a.run_ranges().map(|(r, p)| (&a.text[r], p.link.as_deref())).collect();
    let b_links: Vec<(&str, Option<&str>)> = b.run_ranges().map(|(r, p)| (&b.text[r], p.link.as_deref())).collect();
    a_links == b_links
}

/// Diffs two matched paragraphs either at word granularity or at character granularity according
/// to `ctx.opts.characters`. When character-level diff is requested but either paragraph exceeds
/// [`MAX_CHAR_DIFF`] characters, falls back to word-level diff and increments `ctx.char_fallbacks`
/// so callers can discover the fallback.
fn word_diff(parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>, a: &Paragraph, b: &Paragraph, ctx: &mut DiffCtx<'_>) -> Paragraph {
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
    let key = |t: &Tok<'_>| {
        if let Some(TokPiece::Obj(o, _)) = t.pieces.first().filter(|_| t.pieces.len() == 1)
            && o.plain_text().is_empty()
        {
            return obj_key(parts, o);
        }
        let mut s = String::new();
        for piece in &t.pieces {
            match piece {
                TokPiece::Text(text, _) => s.push_str(text),
                TokPiece::Obj(o, _) => s.push_str(o.plain_text()),
            }
        }
        Key::Text(ctx.opts.key(&s))
    };
    let ops = lcs_ops(&ta.iter().map(key).collect::<Vec<_>>(), &tb.iter().map(key).collect::<Vec<_>>(), ctx.budget);
    // Only at character granularity: a matched whole word almost never happens by coincidence,
    // but a matched single character very often does (digits, punctuation), so only there does
    // a short interior match need folding back into its surrounding edit.
    let ops = if chars { collapse_noise_matches(&ops) } else { ops };
    let mut p = Paragraph::new();
    p.props = b.props.clone();
    p.props.fmt_change = None;
    p.mark = b.mark.clone();
    p.mark.ins = None;
    p.mark.del = None;
    p.mark.fmt_change = None;
    let (mut i, mut j) = (0, 0);
    for op in ops {
        match op {
            Op::Same => {
                // Equal as compared (perhaps up to case or spacing): the revised text.
                let ta_tok = ta.get(i);
                let tb_tok = tb.get(j);
                if let (Some(a_tok), Some(b_tok)) = (ta_tok, tb_tok)
                    && !same_tok_links(a_tok, b_tok)
                {
                    put_tok(&mut p, Some(a_tok), Some(ctx.del), false);
                    put_tok(&mut p, Some(b_tok), Some(ctx.ins), true);
                } else {
                    let mut tb_mut = tb_tok.cloned();
                    if ctx.opts.formatting
                        && let (Some(ta_tok), Some(tb_mut)) = (ta.get(i), tb_mut.as_mut())
                    {
                        apply_tok_formatting(ta_tok, tb_mut, ctx);
                    }
                    put_tok(&mut p, tb_mut.as_ref(), None, false);
                }
                i += 1;
                j += 1;
            }
            Op::Del => {
                put_tok(&mut p, ta.get(i), Some(ctx.del), false);
                i += 1;
            }
            Op::Ins => {
                put_tok(&mut p, tb.get(j), Some(ctx.ins), true);
                j += 1;
            }
        }
    }
    if ctx.opts.formatting {
        crate::cmd::fmt_revisions::compare_para_formatting(&a.props, &a.mark, &mut p, ctx.revisions, ctx.author, ctx.date);
    }
    p
}
