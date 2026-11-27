use std::collections::BTreeMap;

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

/// Context and tracking counters for diffing document blocks and paragraph contents.
struct DiffCtx<'a> {
    ins: u32,
    del: u32,
    opts: &'a CompareOptions,
    author: &'a str,
    date: &'a str,
    budget: &'a mut u64,
    char_fallbacks: &'a mut usize,
    table_fallbacks: &'a mut usize,
    changes: usize,
    plain_revised: bool,
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
/// fallback count in `"characterFallback"`, and includes `"level"` in `"notHonoured"`. When a
/// same-text table has media changes but an incompatible layout or exceeds [`wordcraft_doc::table::MAX_TABLE_DEPTH`],
/// structural table replacement is deferred and `"tables"` is listed in `"notHonoured"`.
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
    let compare_notes = p::bool(v, "footnotes").unwrap_or(true);
    let mut not_honoured: Vec<&str> =
        ["moves", "comments", "headersFooters", "textBoxes", "fields"].into_iter().filter(|k| v.get(*k).is_some()).collect();
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    let date = crate::cmd::now_iso();
    let mut budget = MAX_COMPARE_CELLS;
    let mut revised = revised;
    let mut raw_media = std::mem::take(&mut revised.media);
    let mut media_map = BTreeMap::new();
    let notes = prepare_notes(&mut s.doc, &mut revised, &mut raw_media, &mut media_map, &opts, compare_notes);
    let old: Vec<Arc<Block>> = s.doc.body.clone();
    let new: Vec<Arc<Block>> = if revised.parts.values().any(|p| p.kind == wordcraft_doc::PartKind::TextBox) {
        let mut frag = revised.body_fragment();
        for b in &mut frag.blocks {
            remap_block_media(b, &mut |k| adopt_media_key(&mut s.doc, &mut raw_media, &mut media_map, k));
        }
        for part in frag.parts.values_mut() {
            for b in &mut part.blocks {
                remap_block_media(Arc::make_mut(b), &mut |k| adopt_media_key(&mut s.doc, &mut raw_media, &mut media_map, k));
            }
        }
        // The revised document's text boxes get stories of their own in the result, so its
        // boxes never show a story of the original's (or none at all).
        s.doc.adopt_fragment(&frag).into_iter().map(Arc::new).collect()
    } else {
        for b in &mut revised.body {
            remap_block_media(Arc::make_mut(b), &mut |k| adopt_media_key(&mut s.doc, &mut raw_media, &mut media_map, k));
        }
        std::mem::take(&mut revised.body)
    };
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Insert, author: author.clone(), date: date.clone(), move_name: None });
    let ins = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Delete, author: author.clone(), date: date.clone(), move_name: None });
    let del = (s.doc.revisions.len() - 1) as u32;
    let mut char_fallbacks = 0usize;
    let mut table_fallbacks = 0usize;
    let (out, changes) = {
        let mut ctx = DiffCtx {
            ins,
            del,
            opts: &opts,
            author: &author,
            date: &date,
            budget: &mut budget,
            char_fallbacks: &mut char_fallbacks,
            table_fallbacks: &mut table_fallbacks,
            changes: 0,
            plain_revised,
        };
        let out = diff_blocks(&mut s.doc, &old, &new, &mut ctx, 0);
        if compare_notes {
            diff_notes(&mut s.doc, &revised, &mut raw_media, &mut media_map, &notes, &mut ctx);
        }
        (out, ctx.changes)
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
    if table_fallbacks > 0 {
        log::warn!(
            "compare: {table_fallbacks} table(s) with media changes could not be compared cell by cell due to incompatible layout or nesting depth"
        );
        if !not_honoured.contains(&"tables") {
            not_honoured.push("tables");
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

/// Paired, inserted, and deleted footnote/endnote IDs resolved by [`prepare_notes`].
struct NotePlan {
    paired: Vec<(u32, u32)>,
    inserted: Vec<u32>,
    deleted: Vec<u32>,
}

/// Aligns footnotes and endnotes between `doc` and `revised`, remaps note references in `revised`,
/// and adopts newly inserted note parts (along with their text boxes and media) into `doc`.
fn prepare_notes(
    doc: &mut Document,
    revised: &mut Document,
    raw_media: &mut BTreeMap<String, Arc<Vec<u8>>>,
    media_map: &mut BTreeMap<String, String>,
    opts: &CompareOptions,
    compare_notes: bool,
) -> NotePlan {
    let mut note_budget = MAX_NOTE_ALIGN_BUDGET;
    let mut note_map = std::collections::HashMap::new();
    let mut paired_notes = Vec::new();
    let mut adopted_notes = Vec::new();
    let mut inserted_notes = Vec::new();
    let mut deleted_notes = Vec::new();

    for kind in [wordcraft_doc::PartKind::Footnote, wordcraft_doc::PartKind::Endnote] {
        let orig_ids = document_note_ids(doc, kind);
        let rev_ids = document_note_ids(revised, kind);
        let (paired, inserted, deleted) = align_notes(doc, &orig_ids, revised, &rev_ids, kind, opts, &mut note_budget);
        for (o, r) in paired {
            note_map.insert(r, o);
            if compare_notes {
                paired_notes.push((o, r));
            }
        }
        for r in inserted {
            let new_id = doc.add_part(kind, Vec::new());
            note_map.insert(r, new_id);
            adopted_notes.push((new_id, r));
            if compare_notes {
                inserted_notes.push(new_id);
            }
        }
        if compare_notes {
            deleted_notes.extend(deleted);
        }
    }
    if !note_map.is_empty() {
        revised.body = revised.body.iter().map(|b| Arc::new(remap_note_refs(b, &note_map, 0))).collect();
        for part in revised.parts.values_mut() {
            part.blocks = part.blocks.iter().map(|b| Arc::new(remap_note_refs(b, &note_map, 0))).collect();
        }
    }
    for (new_id, rev_id) in adopted_notes {
        let blocks = revised.parts.get(&rev_id).map(|p| p.blocks.clone()).unwrap_or_default();
        let adopted = adopt_revised_blocks(doc, revised, raw_media, media_map, &blocks);
        if let Some(part) = doc.parts.get_mut(&new_id) {
            part.blocks = adopted;
        }
    }
    NotePlan { paired: paired_notes, inserted: inserted_notes, deleted: deleted_notes }
}

/// Diffs paired footnotes/endnotes and marks inserted or deleted note parts in `doc`.
fn diff_notes(
    doc: &mut Document,
    revised: &Document,
    raw_media: &mut BTreeMap<String, Arc<Vec<u8>>>,
    media_map: &mut BTreeMap<String, String>,
    notes: &NotePlan,
    ctx: &mut DiffCtx<'_>,
) {
    for &(orig_id, rev_id) in &notes.paired {
        let orig_blocks = doc.parts.get(&orig_id).map(|p| p.blocks.clone()).unwrap_or_default();
        let rev_blocks_raw = revised.parts.get(&rev_id).map(|p| p.blocks.clone()).unwrap_or_default();
        let rev_blocks = adopt_revised_blocks(doc, revised, raw_media, media_map, &rev_blocks_raw);
        let note_out = diff_blocks(doc, &orig_blocks, &rev_blocks, ctx, 0);
        if let Some(part) = doc.parts.get_mut(&orig_id) {
            part.blocks = note_out;
        }
    }
    for &new_id in &notes.inserted {
        if let Some(part) = doc.parts.get_mut(&new_id) {
            part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.ins = Some(ctx.ins)))).collect();
        }
    }
    for &orig_id in &notes.deleted {
        if let Some(part) = doc.parts.get_mut(&orig_id) {
            part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.del = Some(ctx.del)))).collect();
        }
    }
}

/// Clears tracked revision metadata from a block tree so foreign revision indices from `revised`
/// never leak into the comparison result document.
fn strip_block_revisions(b: &mut Block) {
    let mut stack = vec![b];
    while let Some(block) = stack.pop() {
        match block {
            Block::Para(p) => {
                let mut changed = false;
                for r in &mut p.runs {
                    if r.props.ins.take().is_some() {
                        changed = true;
                    }
                    if r.props.del.take().is_some() {
                        changed = true;
                    }
                    if r.props.fmt_change.take().is_some() {
                        changed = true;
                    }
                }
                changed |= p.mark.ins.take().is_some();
                changed |= p.mark.del.take().is_some();
                changed |= p.mark.fmt_change.take().is_some();
                changed |= p.props.fmt_change.take().is_some();
                changed |= p.props.num_change.take().is_some();
                if let Some(sect) = p.section.as_deref_mut() {
                    changed |= sect.fmt_change.take().is_some();
                }
                if changed {
                    p.normalize();
                    p.touch();
                }
            }
            Block::Table(t) => {
                t.props.fmt_change = None;
                for row in &mut t.rows {
                    row.props.fmt_change = None;
                    if let Some(ex) = row.props.exceptions.as_deref_mut() {
                        ex.fmt_change = None;
                    }
                    for cell in &mut row.cells {
                        cell.props.fmt_change = None;
                        stack.extend(cell.blocks.iter_mut().map(Arc::make_mut));
                    }
                }
            }
        }
    }
}

/// Pushes every text box story ID referenced inside `blocks` (walking nested tables).
fn collect_text_box_ids(blocks: &[Arc<Block>], todo: &mut Vec<u32>) {
    let mut stack: Vec<&Block> = blocks.iter().map(|b| &**b).collect();
    while let Some(b) = stack.pop() {
        match b {
            Block::Para(p) => {
                for o in &p.objects {
                    todo.extend(o.text_boxes());
                }
            }
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        stack.extend(cell.blocks.iter().map(|cb| &**cb));
                    }
                }
            }
        }
    }
}

/// Collects the `TextBox` parts in `revised` reachable from `blocks` (including nested text boxes),
/// stripping pre-existing source revision metadata from their blocks.
fn reachable_text_boxes(revised: &Document, blocks: &[Arc<Block>]) -> std::collections::BTreeMap<u32, wordcraft_doc::Part> {
    let mut out = std::collections::BTreeMap::new();
    let mut todo = Vec::new();
    collect_text_box_ids(blocks, &mut todo);
    while let Some(id) = todo.pop() {
        if out.contains_key(&id) {
            continue;
        }
        let Some(part) = revised.parts.get(&id).filter(|p| p.kind == wordcraft_doc::PartKind::TextBox) else {
            continue;
        };
        collect_text_box_ids(&part.blocks, &mut todo);
        let mut clean = part.clone();
        for b in &mut clean.blocks {
            strip_block_revisions(Arc::make_mut(b));
        }
        out.insert(id, clean);
    }
    out
}

/// Adopts revised blocks into `doc`, stripping source revision metadata and copying any reachable
/// text-box stories and referenced media from `revised` / `raw_media`.
fn adopt_revised_blocks(
    doc: &mut Document,
    revised: &Document,
    raw_media: &mut BTreeMap<String, Arc<Vec<u8>>>,
    media_map: &mut BTreeMap<String, String>,
    blocks: &[Arc<Block>],
) -> Vec<Arc<Block>> {
    let mut frag_blocks: Vec<Block> = blocks.iter().map(|b| (**b).clone()).collect();
    for b in &mut frag_blocks {
        strip_block_revisions(b);
        remap_block_media(b, &mut |k| adopt_media_key(doc, raw_media, media_map, k));
    }
    let mut parts = reachable_text_boxes(revised, blocks);
    for part in parts.values_mut() {
        for b in &mut part.blocks {
            remap_block_media(Arc::make_mut(b), &mut |k| adopt_media_key(doc, raw_media, media_map, k));
        }
    }
    if parts.is_empty() {
        return frag_blocks.into_iter().map(Arc::new).collect();
    }
    let frag = wordcraft_doc::edit::Fragment { parts, media: BTreeMap::new(), blocks: frag_blocks };
    doc.adopt_fragment(&frag).into_iter().map(Arc::new).collect()
}

/// Compare two block slices (`old` vs `new`) at table nesting `depth`, emitting tracked changes.
fn diff_blocks(doc: &mut Document, old: &[Arc<Block>], new: &[Arc<Block>], ctx: &mut DiffCtx<'_>, depth: usize) -> Vec<Arc<Block>> {
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => ctx.opts.key(&p.plain_text()),
        // Tables match as a whole, by their text.
        Block::Table(_) => format!("\u{0}table\u{0}{}", ctx.opts.key(&block_text(b, depth))),
    };
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), ctx.budget);
    let mut out: Vec<Arc<Block>> = Vec::new();
    let (mut i, mut j, mut k) = (0usize, 0usize, 0usize);
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                let (a, b) = (old.get(i), new.get(j));
                // The revised block, with its formatting and object changes tracked.
                if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    if !same_objects(&doc.parts, ctx.opts, pa, pb) {
                        // Same text, but objects (text boxes, pictures…) differ: diffed, so the
                        // original's objects aren't lost.
                        out.push(para_block(word_diff(doc, pa, pb, ctx)));
                        ctx.changes = ctx.changes.saturating_add(1);
                    } else {
                        let mut p = pb.clone();
                        if ctx.opts.formatting && pa.text == pb.text {
                            let before = doc.revisions.len();
                            crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, &mut doc.revisions, ctx.author, ctx.date);
                            if doc.revisions.len() > before || p != *pb {
                                ctx.changes = ctx.changes.saturating_add(1);
                            }
                        }
                        out.push(para_block(p));
                    }
                } else if let (Some(Block::Table(ta)), Some(Block::Table(tb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    diff_matched_table(doc, (a, ta), (b, tb), ctx, depth, &mut out);
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
                        (Some(Block::Para(pa)), Some(Block::Para(pb))) => out.push(para_block(word_diff(doc, pa, pb, ctx))),
                        _ => {
                            if let Some(x) = a {
                                out.push(Arc::new(mark_block(x, &|c| c.del = Some(ctx.del))));
                            }
                            if let Some(x) = b {
                                out.push(Arc::new(mark_block(x, &|c| c.ins = Some(ctx.ins))));
                            }
                        }
                    }
                    ctx.changes = ctx.changes.saturating_add(1);
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
pub(super) const MAX_NOTE_ALIGN_RUN: usize = 200;

/// Maximum number of DP matrix cells allowed for note run alignment.
const MAX_NOTE_ALIGN_CELLS: usize = 40_000;

/// Maximum look-ahead, in tokens, for the greedy fallback match.
const GREEDY_WINDOW: usize = 16;

/// Maximum characters of normalized inline context captured on each side of a note reference.
const MAX_REF_CONTEXT_CHARS: usize = 128;

/// Location and inline surrounding context of a note reference in the document body.
struct RefContext {
    /// Top-level body block index containing the note reference (or its anchor).
    block_idx: usize,
    /// Normalized plain text of the paragraph containing the note reference.
    para: String,
    /// Normalized plain text immediately preceding the note reference.
    before: String,
    /// Normalized plain text immediately following the note reference.
    after: String,
}

/// Metadata and content summary for aligning footnote and endnote parts.
struct NoteInfo {
    /// Note ID.
    id: u32,
    /// Normalized plain text key of the note content.
    key: String,
    /// Tokens for similarity comparison.
    tokens: Vec<String>,
    /// Location and inline context of the note reference.
    ref_pos: Option<RefContext>,
}

/// Returns the trailing slice of `s` containing at most `max_chars` Unicode scalar values.
fn suffix_chars(s: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    match s.char_indices().rev().nth(max_chars - 1) {
        Some((idx, _)) => s.get(idx..).unwrap_or(s),
        None => s,
    }
}

/// Returns the leading slice of `s` containing at most `max_chars` Unicode scalar values.
fn prefix_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => s.get(..idx).unwrap_or(s),
        None => s,
    }
}

/// Computes a map from note ID to its reference location and inline surrounding context.
fn note_reference_map(doc: &Document, kind: wordcraft_doc::PartKind, opts: &CompareOptions) -> std::collections::HashMap<u32, RefContext> {
    let note_kind = match kind {
        wordcraft_doc::PartKind::Footnote => wordcraft_doc::para::NoteKind::Footnote,
        wordcraft_doc::PartKind::Endnote => wordcraft_doc::para::NoteKind::Endnote,
        _ => return std::collections::HashMap::new(),
    };
    let mut map = std::collections::HashMap::new();
    doc.objects_in_reading_order_by_block(&mut |block_idx, para, obj_idx| {
        if let Some(InlineObject::NoteRef { kind: k, id, .. }) = para.objects.get(obj_idx)
            && *k == note_kind
            && !map.contains_key(id)
        {
            let mut plain = String::with_capacity(para.text.len());
            let mut k_obj = 0usize;
            let mut ref_offsets: Vec<(u32, usize)> = Vec::new();
            for c in para.text.chars() {
                if c == OBJ {
                    if let Some(o) = para.objects.get(k_obj) {
                        if let InlineObject::NoteRef { kind: nk, id: nid, .. } = o
                            && *nk == note_kind
                        {
                            ref_offsets.push((*nid, plain.len()));
                        }
                        plain.push_str(o.plain_text());
                    }
                    k_obj = k_obj.saturating_add(1);
                } else {
                    plain.push(c);
                }
            }
            let para_key = opts.key(&plain);
            for (nid, off) in ref_offsets {
                let raw_before = plain.get(..off).unwrap_or_default();
                let raw_after = plain.get(off..).unwrap_or_default();
                let before = opts.key(suffix_chars(raw_before, MAX_REF_CONTEXT_CHARS));
                let after = opts.key(prefix_chars(raw_after, MAX_REF_CONTEXT_CHARS));
                map.entry(nid).or_insert_with(|| RefContext { block_idx, para: para_key.clone(), before, after });
            }
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
        (Some(ra), Some(rb)) if !ra.para.is_empty() => ra.para == rb.para,
        _ => false,
    };
    let same_ref_idx = match (&a.ref_pos, &b.ref_pos) {
        (Some(ra), Some(rb)) => ra.block_idx == rb.block_idx,
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

    let ctx_bonus = (ref_context_score(a.ref_pos.as_ref(), b.ref_pos.as_ref()).min(99)) as u32;
    if same_ref_para {
        text_sim.max(500).saturating_add(200).saturating_add(ctx_bonus).min(999)
    } else if same_ref_idx && text_sim >= 300 {
        text_sim.saturating_add(100).saturating_add(ctx_bonus).min(999)
    } else {
        text_sim
    }
}

/// Returns the longest subsequence of `anchors` (sorted by `old_idx`) with strictly increasing `new_idx`.
fn lis_anchors(anchors: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if anchors.is_empty() {
        return Vec::new();
    }
    let mut tails: Vec<usize> = Vec::new();
    let mut prev = vec![usize::MAX; anchors.len()];
    for (i, &(_, new_idx)) in anchors.iter().enumerate() {
        let pos = tails.partition_point(|&ti| anchors.get(ti).is_some_and(|a| a.1 < new_idx));
        if let Some(prev_idx) = pos.checked_sub(1).and_then(|p| tails.get(p)).copied()
            && let Some(slot) = prev.get_mut(i)
        {
            *slot = prev_idx;
        }
        if pos == tails.len() {
            tails.push(i);
        } else if let Some(slot) = tails.get_mut(pos) {
            *slot = i;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut cur = tails.last().copied();
    while let Some(i) = cur {
        if let Some(&pair) = anchors.get(i) {
            out.push(pair);
        }
        cur = prev.get(i).copied().filter(|&p| p != usize::MAX);
    }
    out.reverse();
    out
}

/// Pairs notes inside a run too large for the similarity matrix, using identity anchors only.
///
/// A note is paired when its text is identical, or when its reference paragraph uniquely matches
/// one unpaired note on each side (or matches `(block_idx, para)` when paragraph text repeats).
/// Notes without an order-preserving anchor stay whole deletions or insertions: pairing them by
/// position would attach individual accept/reject revisions to a different original note.
fn align_note_run_by_anchors(old_notes: &[&NoteInfo], new_notes: &[&NoteInfo]) -> (Vec<(u32, u32)>, Vec<u32>, Vec<u32>) {
    let mut exact_anchors: Vec<(usize, usize)> = Vec::new();
    let mut exact_old = vec![false; old_notes.len()];
    let mut exact_new = vec![false; new_notes.len()];

    // Same text, in document order: each revised note answers at most one original note.
    let mut by_key: std::collections::HashMap<&str, std::collections::VecDeque<usize>> = std::collections::HashMap::new();
    for (idx, note) in new_notes.iter().enumerate() {
        if !note.key.is_empty() {
            by_key.entry(note.key.as_str()).or_default().push_back(idx);
        }
    }
    let mut next_new = 0usize;
    for (old_idx, note) in old_notes.iter().enumerate() {
        let Some(queue) = by_key.get_mut(note.key.as_str()) else { continue };
        while queue.front().is_some_and(|&idx| idx < next_new) {
            queue.pop_front();
        }
        if let Some(new_idx) = queue.pop_front() {
            exact_anchors.push((old_idx, new_idx));
            if let Some(flag) = exact_old.get_mut(old_idx) {
                *flag = true;
            }
            if let Some(flag) = exact_new.get_mut(new_idx) {
                *flag = true;
            }
            next_new = new_idx.saturating_add(1);
        }
    }

    // Same reference paragraph: a unique paragraph among unpaired notes anchors even if an earlier
    // paragraph edit shifted `block_idx`; repeated paragraphs keep the `(block_idx, para)` check.
    let mut old_by_para: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (idx, note) in old_notes.iter().enumerate() {
        if let Some(ctx) = &note.ref_pos
            && !ctx.para.is_empty()
            && !exact_old.get(idx).copied().unwrap_or(true)
        {
            old_by_para.entry(ctx.para.as_str()).or_default().push(idx);
        }
    }
    let mut new_by_para: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (idx, note) in new_notes.iter().enumerate() {
        if let Some(ctx) = &note.ref_pos
            && !ctx.para.is_empty()
            && !exact_new.get(idx).copied().unwrap_or(true)
        {
            new_by_para.entry(ctx.para.as_str()).or_default().push(idx);
        }
    }
    let mut ref_anchors: Vec<(usize, usize)> = Vec::new();
    for (para, olds) in old_by_para {
        let Some(news) = new_by_para.get(para) else { continue };
        if let ([old_idx], [new_idx]) = (olds.as_slice(), news.as_slice()) {
            ref_anchors.push((*old_idx, *new_idx));
            continue;
        }
        let mut old_at: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
        for &idx in &olds {
            if let Some(ctx) = old_notes.get(idx).and_then(|n| n.ref_pos.as_ref()) {
                old_at.entry(ctx.block_idx).or_default().push(idx);
            }
        }
        let mut new_at: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
        for &idx in news {
            if let Some(ctx) = new_notes.get(idx).and_then(|n| n.ref_pos.as_ref()) {
                new_at.entry(ctx.block_idx).or_default().push(idx);
            }
        }
        for (block_idx, old_idxs) in old_at {
            let [old_idx] = old_idxs.as_slice() else { continue };
            let Some(new_idxs) = new_at.get(&block_idx) else { continue };
            let [new_idx] = new_idxs.as_slice() else { continue };
            ref_anchors.push((*old_idx, *new_idx));
        }
    }

    // Keep reference anchors compatible with exact-text anchors, then take the longest
    // order-preserving chain so one moved paragraph cannot wipe out shifted neighbours.
    ref_anchors.retain(|&(old_idx, new_idx)| {
        let pos = exact_anchors.partition_point(|&(eo, _)| eo < old_idx);
        let left_ok = pos.checked_sub(1).and_then(|i| exact_anchors.get(i)).is_none_or(|&(_, en)| en < new_idx);
        let right_ok = exact_anchors.get(pos).is_none_or(|&(_, en)| new_idx < en);
        left_ok && right_ok
    });
    ref_anchors.sort_unstable();
    let mut anchors = exact_anchors;
    anchors.extend(lis_anchors(&ref_anchors));

    // Anchors must keep document order on both sides. A dropped anchor stays a whole
    // deletion or insertion, never a silent loss.
    anchors.sort_unstable();
    let mut paired = Vec::new();
    let mut old_paired = vec![false; old_notes.len()];
    let mut new_paired = vec![false; new_notes.len()];
    let mut last_new = 0usize;
    for (old_idx, new_idx) in anchors {
        if new_idx < last_new {
            continue;
        }
        let (Some(o), Some(r)) = (old_notes.get(old_idx), new_notes.get(new_idx)) else { continue };
        paired.push((o.id, r.id));
        if let Some(flag) = old_paired.get_mut(old_idx) {
            *flag = true;
        }
        if let Some(flag) = new_paired.get_mut(new_idx) {
            *flag = true;
        }
        last_new = new_idx.saturating_add(1);
    }

    let deleted = old_notes.iter().zip(&old_paired).filter(|(_, p)| !**p).map(|(info, _)| info.id).collect();
    let inserted = new_notes.iter().zip(&new_paired).filter(|(_, p)| !**p).map(|(info, _)| info.id).collect();
    (paired, inserted, deleted)
}

/// Aligns a run of original notes and revised notes using dynamic programming over note similarity and reference context.
fn align_note_run(old_notes: &[&NoteInfo], new_notes: &[&NoteInfo], budget: &mut u64) -> (Vec<(u32, u32)>, Vec<u32>, Vec<u32>) {
    let n = old_notes.len();
    let m = new_notes.len();
    if n == 0 {
        return (Vec::new(), new_notes.iter().map(|info| info.id).collect(), Vec::new());
    }
    if m == 0 {
        return (Vec::new(), Vec::new(), old_notes.iter().map(|info| info.id).collect());
    }
    if n > MAX_NOTE_ALIGN_RUN || m > MAX_NOTE_ALIGN_RUN || n.saturating_mul(m) > MAX_NOTE_ALIGN_CELLS {
        return align_note_run_by_anchors(old_notes, new_notes);
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

    // No note met the similarity threshold: keep only identity anchors, never positional ones.
    if pairs.is_empty() {
        return align_note_run_by_anchors(old_notes, new_notes);
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

/// Returns the trailing character count shared by `a` and `b` up to a word boundary,
/// provided the shared suffix contains at least one alphanumeric character.
fn common_suffix_context(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut k = a_chars.iter().rev().zip(b_chars.iter().rev()).take_while(|(x, y)| x == y).count();
    while k > 0 {
        let a_ok = k == a_chars.len()
            || !a_chars.get(a_chars.len().saturating_sub(k)).is_some_and(|c| c.is_alphanumeric())
            || !a_chars.len().checked_sub(k + 1).and_then(|idx| a_chars.get(idx)).is_some_and(|c| c.is_alphanumeric());
        let b_ok = k == b_chars.len()
            || !b_chars.get(b_chars.len().saturating_sub(k)).is_some_and(|c| c.is_alphanumeric())
            || !b_chars.len().checked_sub(k + 1).and_then(|idx| b_chars.get(idx)).is_some_and(|c| c.is_alphanumeric());
        if a_ok && b_ok {
            break;
        }
        k -= 1;
    }
    let matched = a_chars.get(a_chars.len().saturating_sub(k)..).unwrap_or_default();
    if matched.iter().any(|c| c.is_alphanumeric()) { k } else { 0 }
}

/// Returns the leading character count shared by `a` and `b` up to a word boundary,
/// provided the shared prefix contains at least one alphanumeric character.
fn common_prefix_context(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut k = a_chars.iter().zip(&b_chars).take_while(|(x, y)| x == y).count();
    while k > 0 {
        let a_ok = k == a_chars.len()
            || !k.checked_sub(1).and_then(|idx| a_chars.get(idx)).is_some_and(|c| c.is_alphanumeric())
            || !a_chars.get(k).is_some_and(|c| c.is_alphanumeric());
        let b_ok = k == b_chars.len()
            || !k.checked_sub(1).and_then(|idx| b_chars.get(idx)).is_some_and(|c| c.is_alphanumeric())
            || !b_chars.get(k).is_some_and(|c| c.is_alphanumeric());
        if a_ok && b_ok {
            break;
        }
        k -= 1;
    }
    if a_chars.get(..k).is_some_and(|s| s.iter().any(|c| c.is_alphanumeric())) { k } else { 0 }
}

/// Scores how well two note references' immediate inline surrounding text matches.
fn ref_context_score(a: Option<&RefContext>, b: Option<&RefContext>) -> usize {
    let (Some(a), Some(b)) = (a, b) else { return 0 };
    common_suffix_context(&a.before, &b.before).saturating_add(common_prefix_context(&a.after, &b.after))
}

/// Returns the unique strict-maximum `(row, col)` index for a slice of scores, or `None` on zero or ties.
fn unique_strict_best<I>(scores: I) -> Option<(usize, usize)>
where
    I: Iterator<Item = (usize, usize)>,
{
    let mut best_idx = None;
    let mut best_score = 0usize;
    let mut tied = false;
    for (idx, score) in scores {
        if score == 0 {
            continue;
        }
        if score > best_score {
            best_score = score;
            best_idx = Some(idx);
            tied = false;
        } else if score == best_score {
            tied = true;
        }
    }
    (!tied).then_some(best_idx.map(|idx| (idx, best_score))).flatten()
}

/// Pairs notes whose text repeats on either side by inline reference context, before the text LCS.
///
/// Identical text cannot tell two notes apart, and `lcs_ops` would commit a surviving note to the
/// first duplicate. Notes with a unique inline `(before, after)` reference context pair directly;
/// remaining duplicates within a paragraph or across edited paragraphs pair only when their
/// immediate inline context forms a mutual strict-best match. Returns sorted `(old_idx, new_idx)`
/// anchor pairs.
fn pair_duplicate_note_texts(old_info: &[NoteInfo], new_info: &[NoteInfo]) -> Vec<(usize, usize)> {
    let mut counts: std::collections::HashMap<&str, (usize, usize)> = std::collections::HashMap::new();
    for note in old_info.iter().filter(|n| !n.key.is_empty()) {
        counts.entry(note.key.as_str()).or_default().0 += 1;
    }
    for note in new_info.iter().filter(|n| !n.key.is_empty()) {
        counts.entry(note.key.as_str()).or_default().1 += 1;
    }
    let duplicated = |key: &str| counts.get(key).is_some_and(|&(old, new)| old > 1 || new > 1);

    let mut old_groups: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (idx, note) in old_info.iter().enumerate() {
        if !note.key.is_empty() && duplicated(note.key.as_str()) {
            old_groups.entry(note.key.as_str()).or_default().push(idx);
        }
    }
    let mut new_groups: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (idx, note) in new_info.iter().enumerate() {
        if !note.key.is_empty() && duplicated(note.key.as_str()) {
            new_groups.entry(note.key.as_str()).or_default().push(idx);
        }
    }

    let mut pairs = Vec::new();
    for (key, olds) in old_groups {
        let Some(news) = new_groups.get(key) else { continue };
        let mut old_matched = vec![false; olds.len()];
        let mut new_matched = vec![false; news.len()];

        // 1. Unique exact inline reference context `(before, after)` on each side.
        let mut old_at: std::collections::HashMap<(&str, &str), Vec<usize>> = std::collections::HashMap::new();
        for (pos, &idx) in olds.iter().enumerate() {
            if let Some(ctx) = old_info.get(idx).and_then(|n| n.ref_pos.as_ref())
                && (!ctx.before.is_empty() || !ctx.after.is_empty())
            {
                old_at.entry((ctx.before.as_str(), ctx.after.as_str())).or_default().push(pos);
            }
        }
        let mut new_at: std::collections::HashMap<(&str, &str), Vec<usize>> = std::collections::HashMap::new();
        for (pos, &idx) in news.iter().enumerate() {
            if let Some(ctx) = new_info.get(idx).and_then(|n| n.ref_pos.as_ref())
                && (!ctx.before.is_empty() || !ctx.after.is_empty())
            {
                new_at.entry((ctx.before.as_str(), ctx.after.as_str())).or_default().push(pos);
            }
        }
        for (loc, old_poss) in old_at {
            let [old_pos] = old_poss.as_slice() else { continue };
            let Some(new_poss) = new_at.get(&loc) else { continue };
            let [new_pos] = new_poss.as_slice() else { continue };
            if let (Some(&old_idx), Some(&new_idx)) = (olds.get(*old_pos), news.get(*new_pos)) {
                pairs.push((old_idx, new_idx));
                if let Some(flag) = old_matched.get_mut(*old_pos) {
                    *flag = true;
                }
                if let Some(flag) = new_matched.get_mut(*new_pos) {
                    *flag = true;
                }
            }
        }

        // 2. Remaining duplicates: pair mutual strict-best inline context matches (e.g. when a
        //    clause and its note reference in the same paragraph were deleted together).
        let rem_olds: Vec<usize> = olds.iter().zip(&old_matched).filter(|(_, m)| !**m).map(|(&idx, _)| idx).collect();
        let rem_news: Vec<usize> = news.iter().zip(&new_matched).filter(|(_, m)| !**m).map(|(&idx, _)| idx).collect();
        if rem_olds.is_empty() || rem_news.is_empty() || rem_olds.len() > MAX_NOTE_ALIGN_RUN || rem_news.len() > MAX_NOTE_ALIGN_RUN {
            continue;
        }
        let scores: Vec<Vec<usize>> = rem_olds
            .iter()
            .map(|&oi| {
                let o_ctx = old_info.get(oi).and_then(|n| n.ref_pos.as_ref());
                rem_news.iter().map(|&ni| ref_context_score(o_ctx, new_info.get(ni).and_then(|n| n.ref_pos.as_ref()))).collect()
            })
            .collect();
        for (u, row) in scores.iter().enumerate() {
            let Some((v, _)) = unique_strict_best(row.iter().copied().enumerate()) else { continue };
            let Some((best_u, _)) = unique_strict_best(scores.iter().enumerate().map(|(r, r_scores)| (r, r_scores.get(v).copied().unwrap_or(0))))
            else {
                continue;
            };
            if best_u == u
                && let (Some(&old_idx), Some(&new_idx)) = (rem_olds.get(u), rem_news.get(v))
            {
                pairs.push((old_idx, new_idx));
            }
        }
    }
    pairs.sort_unstable();
    pairs
}

/// Filters `anchors` (already sorted by `old_idx`) to the subset that respects strict document
/// order on both sides, so every kept anchor safely partitions `old_info` and `new_info`.
fn non_crossing_anchors(anchors: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if anchors.is_empty() {
        return Vec::new();
    }
    let mut prefix_max = Vec::with_capacity(anchors.len());
    let mut cur_max = 0usize;
    for &(_, new_idx) in anchors {
        cur_max = cur_max.max(new_idx);
        prefix_max.push(cur_max);
    }
    let mut suffix_min = vec![usize::MAX; anchors.len()];
    let mut cur_min = usize::MAX;
    for (i, &(_, new_idx)) in anchors.iter().enumerate().rev() {
        cur_min = cur_min.min(new_idx);
        if let Some(slot) = suffix_min.get_mut(i) {
            *slot = cur_min;
        }
    }
    anchors
        .iter()
        .enumerate()
        .filter_map(|(i, &(old_idx, new_idx))| {
            let left_ok = i == 0 || prefix_max.get(i - 1).is_some_and(|&m| m < new_idx);
            let right_ok = i + 1 == anchors.len() || suffix_min.get(i + 1).is_some_and(|&m| new_idx < m);
            (left_ok && right_ok).then_some((old_idx, new_idx))
        })
        .collect()
}

/// Collects unused [`NoteInfo`] references within `info[start..end]`.
fn collect_unused_segment<'a>(info: &'a [NoteInfo], used: &[bool], start: usize, end: usize) -> Vec<&'a NoteInfo> {
    let slice = info.get(start..end).unwrap_or_default();
    let flags = used.get(start..end).unwrap_or_default();
    slice.iter().zip(flags).filter(|(_, is_used)| !**is_used).map(|(note, _)| note).collect()
}

/// Aligns one bounded segment of unused original and revised notes between prepaired anchors.
fn align_note_segment(
    old_seg: &[&NoteInfo],
    new_seg: &[&NoteInfo],
    budget: &mut u64,
    paired: &mut Vec<(u32, u32)>,
    inserted: &mut Vec<u32>,
    deleted: &mut Vec<u32>,
) {
    if old_seg.is_empty() {
        inserted.extend(new_seg.iter().map(|n| n.id));
        return;
    }
    if new_seg.is_empty() {
        deleted.extend(old_seg.iter().map(|n| n.id));
        return;
    }
    let keys_orig: Vec<&str> = old_seg.iter().map(|n| n.key.as_str()).collect();
    let keys_rev: Vec<&str> = new_seg.iter().map(|n| n.key.as_str()).collect();
    let ops = lcs_ops(&keys_orig, &keys_rev, budget);
    let (mut i, mut j, mut k) = (0, 0, 0);
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                if let (Some(o), Some(r)) = (old_seg.get(i), new_seg.get(j)) {
                    paired.push((o.id, r.id));
                }
                i += 1;
                j += 1;
                k += 1;
            }
            Some(_) => {
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                let old_slice = old_seg.get(i..i + dels).unwrap_or_default();
                let new_slice = new_seg.get(j..j + inss).unwrap_or_default();
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

    let prepaired = pair_duplicate_note_texts(&old_info, &new_info);
    let mut paired = Vec::with_capacity(prepaired.len());
    let mut old_used = vec![false; old_info.len()];
    let mut new_used = vec![false; new_info.len()];
    for &(old_idx, new_idx) in &prepaired {
        if let (Some(o), Some(r)) = (old_info.get(old_idx), new_info.get(new_idx)) {
            paired.push((o.id, r.id));
            if let Some(flag) = old_used.get_mut(old_idx) {
                *flag = true;
            }
            if let Some(flag) = new_used.get_mut(new_idx) {
                *flag = true;
            }
        }
    }

    let boundaries = non_crossing_anchors(&prepaired);
    let mut inserted = Vec::new();
    let mut deleted = Vec::new();
    let mut old_start = 0usize;
    let mut new_start = 0usize;
    for (old_anchor, new_anchor) in boundaries.into_iter().chain(std::iter::once((old_info.len(), new_info.len()))) {
        let old_seg = collect_unused_segment(&old_info, &old_used, old_start, old_anchor);
        let new_seg = collect_unused_segment(&new_info, &new_used, new_start, new_anchor);
        align_note_segment(&old_seg, &new_seg, budget, &mut paired, &mut inserted, &mut deleted);
        old_start = old_anchor.saturating_add(1);
        new_start = new_anchor.saturating_add(1);
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
            if depth < wordcraft_doc::table::MAX_TABLE_DEPTH {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    c.blocks = c.blocks.iter().map(|x| Arc::new(remap_note_refs(x, map, depth + 1))).collect();
                }
            }
            Block::Table(t)
        }
    }
}

/// Emit a matched (`Op::Same`) table pair into `out`. Layout-only or media-identical matches keep
/// the revised table as on `main`; tables with media differences are diffed cell by cell when
/// compatible and within [`wordcraft_doc::table::MAX_TABLE_DEPTH`], or otherwise preserve the
/// original table and its media and record a `"tables"` fallback in `notHonoured` without
/// emitting duplicate table shells.
fn diff_matched_table(
    doc: &mut Document,
    (a, ta): (Option<&Arc<Block>>, &wordcraft_doc::table::Table),
    (b, tb): (Option<&Arc<Block>>, &wordcraft_doc::table::Table),
    ctx: &mut DiffCtx<'_>,
    depth: usize,
    out: &mut Vec<Arc<Block>>,
) {
    if ta == tb || table_media_keys(ta) == table_media_keys(tb) {
        if let Some(x) = b.or(a) {
            out.push(x.clone());
        }
    } else if depth < wordcraft_doc::table::MAX_TABLE_DEPTH && same_table_cells(ta, tb, ctx) {
        out.push(Arc::new(Block::Table(diff_table(doc, ta, tb, ctx, depth))));
    } else {
        if let Some(x) = a.or(b) {
            out.push(x.clone());
        }
        *ctx.table_fallbacks = ctx.table_fallbacks.saturating_add(1);
    }
}

/// Whether two tables have compatible grid widths, content controls, row counts, cell counts,
/// spans and vertical merges so their properties and cells can be diffed 1-to-1 without losing
/// untrackable layout changes.
fn same_table_cells(a: &wordcraft_doc::table::Table, b: &wordcraft_doc::table::Table, ctx: &DiffCtx<'_>) -> bool {
    let track_fmt = ctx.opts.formatting && !ctx.plain_revised;
    (!track_fmt || a.grid == b.grid)
        && a.controls == b.controls
        && a.rows.len() == b.rows.len()
        && a.rows.iter().zip(&b.rows).all(|(ra, rb)| {
            ra.controls == rb.controls
                && ra.props.grid_before == rb.props.grid_before
                && ra.props.grid_after == rb.props.grid_after
                && ra.cells.len() == rb.cells.len()
                && ra
                    .cells
                    .iter()
                    .zip(&rb.cells)
                    .all(|(ca, cb)| ca.controls == cb.controls && ca.span() == cb.span() && ca.props.vmerge == cb.props.vmerge)
        })
}

/// Strip revision bookkeeping from table properties for formatting comparison.
fn strip_table_props(t: &wordcraft_doc::props::TableProps) -> wordcraft_doc::props::TableProps {
    wordcraft_doc::props::TableProps { fmt_change: None, ..t.clone() }
}

/// Strip revision bookkeeping from row properties (and any nested row table-property exceptions).
fn strip_row_props(r: &wordcraft_doc::props::RowProps) -> wordcraft_doc::props::RowProps {
    wordcraft_doc::props::RowProps { fmt_change: None, exceptions: r.exceptions.as_deref().map(|e| Box::new(strip_table_props(e))), ..r.clone() }
}

/// Strip revision bookkeeping and structural merge fields from cell properties.
fn strip_cell_props(c: &wordcraft_doc::props::CellProps) -> wordcraft_doc::props::CellProps {
    wordcraft_doc::props::CellProps { fmt_change: None, span: 0, vmerge: wordcraft_doc::props::VMerge::None, ..c.clone() }
}

/// Build a formatting [`wordcraft_doc::props::PropChange`] when formatting tracking is active and
/// `old` differs from `new`.
fn fmt_prop_change<T: PartialEq>(doc: &mut Document, ctx: &DiffCtx<'_>, old: T, new: &T) -> Option<Box<wordcraft_doc::props::PropChange<T>>> {
    if !ctx.opts.formatting || ctx.plain_revised || old == *new {
        return None;
    }
    let fmt_rev = format_revision(doc, ctx.author, ctx.date);
    wordcraft_doc::props::PropChange::boxed(fmt_rev, old)
}

/// Like [`fmt_prop_change`], also counting a container-level formatting change in `ctx.changes`.
fn record_prop_change<T: PartialEq>(doc: &mut Document, ctx: &mut DiffCtx<'_>, old: T, new: &T) -> Option<Box<wordcraft_doc::props::PropChange<T>>> {
    let ch = fmt_prop_change(doc, ctx, old, new);
    if ch.is_some() {
        ctx.changes = ctx.changes.saturating_add(1);
    }
    ch
}

/// Record a cell-property change when formatting tracking is active or the cell's background
/// picture changed (`#6086`), so replaced cell media stays rejectable even with `formatting: false`.
fn record_cell_change(
    doc: &mut Document,
    ctx: &mut DiffCtx<'_>,
    ca: &wordcraft_doc::props::CellProps,
    cb: &wordcraft_doc::props::CellProps,
) -> Option<Box<wordcraft_doc::props::PropChange<wordcraft_doc::props::CellProps>>> {
    let old = strip_cell_props(ca);
    let new = strip_cell_props(cb);
    if ctx.plain_revised || (!ctx.opts.formatting && ca.picture == cb.picture) || old == new {
        return None;
    }
    let fmt_rev = format_revision(doc, ctx.author, ctx.date);
    ctx.changes = ctx.changes.saturating_add(1);
    wordcraft_doc::props::PropChange::boxed(fmt_rev, old)
}

/// Diff two compatible tables cell by cell, tracking table, row, and cell property differences
/// alongside cell content edits.
fn diff_table(
    doc: &mut Document,
    a: &wordcraft_doc::table::Table,
    b: &wordcraft_doc::table::Table,
    ctx: &mut DiffCtx<'_>,
    depth: usize,
) -> wordcraft_doc::table::Table {
    let mut t = b.clone();
    t.props = strip_table_props(&t.props);
    t.props.fmt_change = record_prop_change(doc, ctx, strip_table_props(&a.props), &t.props);
    for (ra, rb) in a.rows.iter().zip(&mut t.rows) {
        rb.props = strip_row_props(&rb.props);
        rb.props.fmt_change = record_prop_change(doc, ctx, strip_row_props(&ra.props), &rb.props);
        for (ca, cb) in ra.cells.iter().zip(&mut rb.cells) {
            cb.props.fmt_change = None;
            cb.props.fmt_change = record_cell_change(doc, ctx, &ca.props, &cb.props);
            if ca.blocks != cb.blocks {
                cb.blocks = diff_blocks(doc, &ca.blocks, &cb.blocks, ctx, depth + 1);
                if cb.blocks.is_empty() {
                    cb.blocks.push(para_block(Paragraph::new()));
                }
            }
        }
    }
    t
}

/// Find or create the formatting revision entry for `author` and `date`.
fn format_revision(doc: &mut Document, author: &str, date: &str) -> u32 {
    if let Some(i) = doc.revisions.iter().rposition(|r| r.kind == RevisionKind::Format && r.author == author && r.date == date) {
        return u32::try_from(i).unwrap_or(u32::MAX);
    }
    doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Format, author: author.to_string(), date: date.to_string(), move_name: None });
    u32::try_from(doc.revisions.len().saturating_sub(1)).unwrap_or(u32::MAX)
}

/// Adopt `old_key` from `raw_media` into `doc` on first use, reusing the existing buffer without
/// cloning when uniquely owned, and memoizing the resulting media key in `media_map`.
fn adopt_media_key(
    doc: &mut Document,
    raw_media: &mut BTreeMap<String, Arc<Vec<u8>>>,
    media_map: &mut BTreeMap<String, String>,
    old_key: &str,
) -> Option<String> {
    if let Some(k) = media_map.get(old_key) {
        return Some(k.clone());
    }
    let data = raw_media.remove(old_key)?;
    let ext = old_key.rsplit_once('.').map(|(_, e)| e).unwrap_or("bin");
    let new_key = doc.add_media(Arc::unwrap_or_clone(data), ext);
    media_map.insert(old_key.to_string(), new_key.clone());
    Some(new_key)
}

/// Rewrites media keys in an inline object and all nested group children via `map`, returning
/// whether any media key changed.
pub(super) fn remap_object_media(root: &mut InlineObject, map: &mut dyn FnMut(&str) -> Option<String>) -> bool {
    let mut changed = false;
    let mut todo = vec![root];
    while let Some(o) = todo.pop() {
        match o {
            InlineObject::Image { media, .. } => {
                if let Some(k) = map(media)
                    && k != *media
                {
                    *media = k;
                    changed = true;
                }
            }
            InlineObject::Graphic { graphic, .. } => {
                let mut any = false;
                for i in &graphic.items {
                    if let wordcraft_doc::graphic::GraphicItem::Image { media, .. } = i
                        && map(media).is_some_and(|k| k != *media)
                    {
                        any = true;
                    }
                }
                if any {
                    for i in &mut Arc::make_mut(graphic).items {
                        if let wordcraft_doc::graphic::GraphicItem::Image { media, .. } = i
                            && let Some(k) = map(media.as_str())
                        {
                            *media = k;
                        }
                    }
                    changed = true;
                }
            }
            InlineObject::Group { children, .. } => {
                for c in children {
                    todo.push(&mut c.obj);
                }
            }
            _ => {}
        }
    }
    changed
}

/// Rewrites media keys in all paragraphs and table cells within `b` via `map`.
fn remap_block_media(b: &mut Block, map: &mut dyn FnMut(&str) -> Option<String>) {
    let mut todo = vec![b];
    while let Some(b) = todo.pop() {
        match b {
            Block::Para(p) => {
                let mut changed = false;
                for o in &mut p.objects {
                    changed |= remap_object_media(o, map);
                }
                if changed {
                    p.touch();
                }
            }
            Block::Table(t) => {
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        if let Some(pic) = &mut cell.props.picture
                            && let Some(k) = map(&pic.media)
                        {
                            pic.media = k;
                        }
                        todo.extend(cell.blocks.iter_mut().map(Arc::make_mut));
                    }
                }
            }
        }
    }
}

/// Returns the plain text of a part's blocks.
fn part_text(p: &wordcraft_doc::Part) -> String {
    p.blocks.iter().map(|b| block_text(b, 0)).collect::<Vec<_>>().join("\n")
}

/// Collects media keys referenced by `root` and any nested graphic/group children iteratively.
fn collect_object_media<'a>(root: &'a InlineObject, out: &mut Vec<&'a str>) {
    let mut todo = vec![root];
    while let Some(o) = todo.pop() {
        match o {
            InlineObject::Image { media, .. } => out.push(media.as_str()),
            InlineObject::Graphic { graphic, .. } => {
                for item in &graphic.items {
                    if let wordcraft_doc::graphic::GraphicItem::Image { media, .. } = item {
                        out.push(media.as_str());
                    }
                }
            }
            InlineObject::Group { children, .. } => {
                for child in children.iter().rev() {
                    todo.push(&child.obj);
                }
            }
            _ => {}
        }
    }
}

/// Collects media keys referenced by `table` across all cell pictures, paragraphs, and nested
/// tables in document order without recursion.
fn table_media_keys(table: &wordcraft_doc::table::Table) -> Vec<&str> {
    enum Item<'a> {
        Table(&'a wordcraft_doc::table::Table),
        Block(&'a Block),
    }
    let mut keys = Vec::new();
    let mut todo = vec![Item::Table(table)];
    while let Some(item) = todo.pop() {
        match item {
            Item::Table(t) => {
                for row in t.rows.iter().rev() {
                    for cell in row.cells.iter().rev() {
                        for block in cell.blocks.iter().rev() {
                            todo.push(Item::Block(block));
                        }
                        if let Some(pic) = &cell.props.picture {
                            keys.push(pic.media.as_str());
                        }
                    }
                }
            }
            Item::Block(Block::Para(p)) => {
                for o in &p.objects {
                    collect_object_media(o, &mut keys);
                }
            }
            Item::Block(Block::Table(nested)) => {
                todo.push(Item::Table(nested));
            }
        }
    }
    keys
}

/// A block's text (a table's cells in order), for matching blocks.
fn block_text(b: &Block, depth: usize) -> String {
    match b {
        Block::Para(p) => p.plain_text(),
        Block::Table(t) if depth < wordcraft_doc::table::MAX_TABLE_DEPTH => {
            t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|x| block_text(x, depth + 1)).collect::<Vec<_>>().join("\t")
        }
        Block::Table(_) => String::new(),
    }
}

/// Returns a copy of `b` with all text runs and paragraph marks across all nested table cells
/// updated by `f` iteratively, without recursion or depth truncation.
fn mark_block(b: &Block, f: &dyn Fn(&mut CharProps)) -> Block {
    let mut out = b.clone();
    let mut todo = vec![&mut out];
    while let Some(blk) = todo.pop() {
        match blk {
            Block::Para(p) => {
                let len = p.len();
                let _ = p.format(0, len, f);
                f(&mut p.mark);
                p.touch();
            }
            Block::Table(t) => {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    todo.extend(c.blocks.iter_mut().map(Arc::make_mut));
                }
            }
        }
    }
    out
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

/// Computes a comparison key for an inline object, taking text box content from `parts` into account.
fn obj_key(parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>, o: &InlineObject) -> Key {
    let texts = o.text_boxes().into_iter().map(|id| parts.get(&id).map(part_text).unwrap_or_default()).collect();
    let mut o = o.clone();
    for story in o.text_box_slots() {
        *story = None;
    }
    Key::Obj(Box::new(o), texts)
}

/// Normalized plain-text prefixes immediately preceding each `NoteRef` in `p`.
fn note_ref_positions(p: &Paragraph, opts: &CompareOptions) -> Vec<String> {
    let mut positions = Vec::new();
    let mut plain = String::with_capacity(p.text.len());
    let mut k_obj = 0usize;
    for c in p.text.chars() {
        if c == OBJ {
            if let Some(o) = p.objects.get(k_obj) {
                if matches!(o, InlineObject::NoteRef { .. }) {
                    positions.push(opts.key(&plain));
                }
                plain.push_str(o.plain_text());
            }
            k_obj = k_obj.saturating_add(1);
        } else {
            plain.push(c);
        }
    }
    positions
}

/// Whether two paragraphs hold the same objects, as compared, with note references at the same
/// normalized text positions.
fn same_objects(parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>, opts: &CompareOptions, a: &Paragraph, b: &Paragraph) -> bool {
    if a.objects.len() != b.objects.len() || !a.objects.iter().zip(&b.objects).all(|(x, y)| x == y || obj_key(parts, x) == obj_key(parts, y)) {
        return false;
    }
    if a.text != b.text && a.objects.iter().any(|o| matches!(o, InlineObject::NoteRef { .. })) {
        return note_ref_positions(a, opts) == note_ref_positions(b, opts);
    }
    true
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
fn word_diff(doc: &mut Document, a: &Paragraph, b: &Paragraph, ctx: &mut DiffCtx<'_>) -> Paragraph {
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
    let (ka, kb): (Vec<_>, Vec<_>) = {
        let key = |t: &Tok<'_>| match t {
            Tok::Text(t, _) => Key::Text(ctx.opts.key(t)),
            Tok::Obj(o, _) => obj_key(&doc.parts, o),
        };
        (ta.iter().map(&key).collect(), tb.iter().map(&key).collect())
    };
    let ops = lcs_ops(&ka, &kb, ctx.budget);
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
    p.props.fmt_change = fmt_prop_change(doc, ctx, a.props.formatting(), &p.props);
    // The paragraph mark (the formatting typing into the paragraph's end would get) is the revised
    // paragraph's when it comes from a document, and the original's when the revised side is plain
    // text and has no mark of its own. Its own revision bookkeeping (`ins`, `del`, `fmt_change`)
    // is cleared because those indices belong to another document's revision table.
    p.mark = if plain_revised {
        CharProps { ins: None, del: None, fmt_change: None, ..a.mark.clone() }
    } else {
        CharProps { ins: None, del: None, fmt_change: None, ..b.mark.clone() }
    };
    p.mark.fmt_change = fmt_prop_change(doc, ctx, a.mark.formatting(), &p.mark.formatting());
    let fallback = a.runs.first().map(|r| CharProps { ins: None, del: None, fmt_change: None, ..r.props.clone() }).unwrap_or_default();
    let fallback_b = b.runs.first().map(|r| CharProps { ins: None, del: None, fmt_change: None, ..r.props.clone() }).unwrap_or_default();
    let (mut i, mut j) = (0, 0);
    for op in ops {
        match op {
            Op::Same => {
                // Equal as compared (perhaps up to case or spacing): the revised text, but the
                // formatting of the run it stands for. For a document revised side that is the
                // revised run, as issue #3749 prescribes ("Copy unchanged tokens from the revised
                // paragraph with their own properties"), recording a formatting revision when
                // `ctx.opts.formatting` is enabled so Reject All restores the original formatting;
                // a plain-text revised side has no runs to speak of, so the original run's
                // formatting is kept instead.
                let old_props = output_props(&mut a_props, ta.get(i), &fallback);
                let props = if plain_revised {
                    old_props
                } else {
                    let mut p_b = output_props(&mut b_props, tb.get(j), &fallback_b);
                    p_b.fmt_change = fmt_prop_change(doc, ctx, old_props.formatting(), &p_b.formatting());
                    p_b
                };
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
