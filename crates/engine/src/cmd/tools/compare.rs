use std::collections::BTreeMap;

use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::para::OBJ;

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character" (character: paragraphs over 2000 characters fall back to word; see characterFallback/notHonoured), "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared), "headersFooters"?: bool (default true: compare headers and footers)}"#;

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
/// footnotes, text boxes, fields) are accepted and listed in the result. Headers and footers are
/// compared by default (`headersFooters`: true), tracking edits, insertions, deletions, and paragraph
/// formatting changes within header and footer stories. When character-level comparison is requested
/// (`"level": "character"`) but any paragraph exceeds [`MAX_CHAR_DIFF`] characters, it falls back to
/// word-level diff for that paragraph, surfaces the fallback count in `"characterFallback"`, and
/// includes `"level"` in `"notHonoured"`. When a same-text table has media changes but an incompatible
/// layout or exceeds [`wordcraft_doc::table::MAX_TABLE_DEPTH`], structural table replacement is
/// deferred and `"tables"` is listed in `"notHonoured"`.
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
    s.doc.clear_compare_hf();
    let level = p::str(v, "level").unwrap_or("word");
    let opts = CompareOptions {
        case: p::bool(v, "caseChanges").unwrap_or(true),
        white_space: p::bool(v, "whiteSpace").unwrap_or(true),
        formatting: p::bool(v, "formatting").unwrap_or(true),
        characters: matches!(level, "character" | "char"),
    };
    let compare_headers_footers = p::bool(v, "headersFooters").unwrap_or(true);
    let mut not_honoured: Vec<&str> = ["moves", "comments", "footnotes", "textBoxes", "fields"].into_iter().filter(|k| v.get(*k).is_some()).collect();
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    let date = crate::cmd::now_iso();
    let orig_sec_pairs = s.doc.sections();
    let mut orig_ends = Vec::with_capacity(orig_sec_pairs.len());
    let mut orig_sections = Vec::with_capacity(orig_sec_pairs.len());
    for (e, sec) in orig_sec_pairs {
        orig_ends.push(e);
        orig_sections.push(sec.clone());
    }

    let rev_sec_pairs = revised.sections();
    let mut rev_ends = Vec::with_capacity(rev_sec_pairs.len());
    let mut rev_sections = Vec::with_capacity(rev_sec_pairs.len());
    for (e, sec) in rev_sec_pairs {
        rev_ends.push(e);
        rev_sections.push(sec.clone());
    }

    let mut orig_eff_headers = Vec::with_capacity(orig_sections.len());
    let mut orig_eff_footers = Vec::with_capacity(orig_sections.len());
    let mut prev_h = wordcraft_doc::section::HeaderSet::default();
    let mut prev_f = wordcraft_doc::section::HeaderSet::default();
    for sec in &orig_sections {
        prev_h = wordcraft_doc::section::HeaderSet {
            default: sec.headers.default.or(prev_h.default),
            first: sec.headers.first.or(prev_h.first),
            even: sec.headers.even.or(prev_h.even),
        };
        prev_f = wordcraft_doc::section::HeaderSet {
            default: sec.footers.default.or(prev_f.default),
            first: sec.footers.first.or(prev_f.first),
            even: sec.footers.even.or(prev_f.even),
        };
        orig_eff_headers.push(prev_h);
        orig_eff_footers.push(prev_f);
    }

    let rev_has_text_boxes = revised.parts.values().any(|p| p.kind == wordcraft_doc::PartKind::TextBox);
    // Adopt media referenced by the revised body and reachable text boxes into the original
    // document: identical bytes reuse the original's key (deduplicated by content), while
    // replaced or new media receives a fresh key in s.doc.media so both media parts are
    // preserved in the result (#6086).
    let mut revised = revised;
    let mut raw_media = std::mem::take(&mut revised.media);
    let mut media_map = BTreeMap::new();
    let old: Vec<Arc<Block>> = s.doc.body.clone();
    let new: Vec<Arc<Block>> = if rev_has_text_boxes {
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
    revised.media = raw_media;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Insert, author: author.clone(), date: date.clone(), move_name: None });
    let ins = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Delete, author: author.clone(), date: date.clone(), move_name: None });
    let del = (s.doc.revisions.len() - 1) as u32;

    let run = CompareRun {
        ins,
        del,
        compare_hf: compare_headers_footers,
        orig_ends: &orig_ends,
        orig_eff_headers: &orig_eff_headers,
        orig_eff_footers: &orig_eff_footers,
    };
    let mut budget = MAX_COMPARE_CELLS;
    let mut char_fallbacks = 0usize;
    let mut table_fallbacks = 0usize;
    let mut sources = Vec::with_capacity(old.len() + new.len());
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
    let out = diff_blocks_inner(&mut s.doc, &old, &new, &mut ctx, 0, Some(&run), Some(&mut sources));
    let body_changes = ctx.changes;
    s.doc.body = out;
    let mut changes = body_changes;

    if compare_headers_footers {
        let snaps = SectionSnapshots {
            orig_sections: &orig_sections,
            rev_sections: &rev_sections,
            orig_ends: &orig_ends,
            rev_ends: &rev_ends,
            orig_eff_headers: &orig_eff_headers,
            orig_eff_footers: &orig_eff_footers,
        };
        changes += compare_headers_and_footers(s, &revised, &snaps, &sources, rev_has_text_boxes, &run, &mut ctx);
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

/// Context describing the author, date, options, and change IDs for compare emissions.
struct CompareRun<'a> {
    /// Revision ID for insertions.
    ins: u32,
    /// Revision ID for deletions.
    del: u32,
    /// Whether header and footer comparison is enabled.
    compare_hf: bool,
    /// Boundaries of sections in the original document.
    orig_ends: &'a [usize],
    /// Effective headers for each section in the original document.
    orig_eff_headers: &'a [wordcraft_doc::section::HeaderSet],
    /// Effective footers for each section in the original document.
    orig_eff_footers: &'a [wordcraft_doc::section::HeaderSet],
}

/// Slot of a header or footer within a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HfSlot {
    /// Default header or footer for normal pages.
    Default,
    /// First-page header or footer when Different First Page is enabled.
    First,
    /// Even-page header or footer when Different Odd & Even Pages is enabled.
    Even,
}

/// Retrieves the part ID for a specific slot from a `HeaderSet`.
fn get_set_slot(set: &wordcraft_doc::section::HeaderSet, slot: HfSlot) -> Option<u32> {
    match slot {
        HfSlot::Default => set.default,
        HfSlot::First => set.first,
        HfSlot::Even => set.even,
    }
}

/// Retrieves the header or footer part ID for a section given its footer flag and slot.
fn get_hf_slot(sec: &wordcraft_doc::section::SectionProps, footer: bool, slot: HfSlot) -> Option<u32> {
    let set = if footer { &sec.footers } else { &sec.headers };
    get_set_slot(set, slot)
}

/// Sets the header or footer part ID for a section given its footer flag and slot.
fn set_hf_slot(sec: &mut wordcraft_doc::section::SectionProps, footer: bool, slot: HfSlot, id: Option<u32>) {
    let set = if footer { &mut sec.footers } else { &mut sec.headers };
    match slot {
        HfSlot::Default => set.default = id,
        HfSlot::First => set.first = id,
        HfSlot::Even => set.even = id,
    }
}

/// Maps each section index in the result document to the corresponding section index in the
/// original and revised documents, based on body block correspondence. Pre-allocated count
/// vectors and binary search over section end boundaries avoid O(blocks x sections) scans
/// and per-section heap allocations.
fn map_result_sections_to_snapshots(
    res_sections: &[(usize, &wordcraft_doc::section::SectionProps)],
    orig_ends: &[usize],
    rev_ends: &[usize],
    sources: &[(Option<usize>, Option<usize>)],
) -> Vec<(Option<usize>, Option<usize>)> {
    let mut mapping = Vec::with_capacity(res_sections.len());
    let mut prev_end = 0;
    let mut orig_counts = vec![0usize; orig_ends.len()];
    let mut rev_counts = vec![0usize; rev_ends.len()];

    for (res_si, &(end_block, _)) in res_sections.iter().enumerate() {
        let block_range = prev_end..=end_block;
        prev_end = end_block + 1;

        orig_counts.fill(0);
        rev_counts.fill(0);

        for bi in block_range {
            if let Some(&(orig_b, rev_b)) = sources.get(bi) {
                if let Some(ob) = orig_b
                    && !orig_ends.is_empty()
                {
                    let osi = orig_ends.partition_point(|&e| e < ob).min(orig_ends.len().saturating_sub(1));
                    if let Some(c) = orig_counts.get_mut(osi) {
                        *c += 1;
                    }
                }
                if let Some(rb) = rev_b
                    && !rev_ends.is_empty()
                {
                    let rsi = rev_ends.partition_point(|&e| e < rb).min(rev_ends.len().saturating_sub(1));
                    if let Some(c) = rev_counts.get_mut(rsi) {
                        *c += 1;
                    }
                }
            }
        }

        let orig_si = orig_counts
            .iter()
            .enumerate()
            .filter(|&(_, &count)| count > 0)
            .max_by(|(si_a, count_a), (si_b, count_b)| count_a.cmp(count_b).then_with(|| si_b.cmp(si_a)))
            .map(|(si, _)| si);
        let rev_si = rev_counts
            .iter()
            .enumerate()
            .filter(|&(_, &count)| count > 0)
            .max_by(|(si_a, count_a), (si_b, count_b)| count_a.cmp(count_b).then_with(|| si_b.cmp(si_a)))
            .map(|(si, _)| si);

        let both_none = orig_si.is_none() && rev_si.is_none();
        let orig_si = orig_si.or_else(|| (both_none && res_si < orig_ends.len()).then_some(res_si));
        let rev_si = rev_si.or_else(|| (both_none && res_si < rev_ends.len()).then_some(res_si));

        mapping.push((orig_si, rev_si));
    }

    mapping
}

/// Section resolution context used during header and footer slot assignment.
struct SectionContext<'a> {
    res_ends: &'a [usize],
    res_si: usize,
}

/// Assigns a header or footer part ID to a specific section index in the session's document.
/// Precomputed `res_ends` avoids rescanning document sections on every slot assignment.
fn assign_section_slot(s: &mut Session, ctx: &SectionContext<'_>, footer: bool, slot: HfSlot, id: u32) {
    let sec_key = (ctx.res_si + 1 < ctx.res_ends.len()).then_some(ctx.res_si);
    let end_opt = sec_key.and_then(|si| ctx.res_ends.get(si).copied());

    if let Some(end) = end_opt {
        let sect = s.doc.section_mut(end);
        set_hf_slot(sect, footer, slot, Some(id));
    } else {
        set_hf_slot(&mut s.doc.last_section, footer, slot, Some(id));
    }
}

/// Collects all text box story IDs referenced by inline objects within `b`, recursively inspecting table cells.
fn block_box_ids(b: &Block, ids: &mut Vec<u32>) {
    match b {
        Block::Para(p) => {
            for o in &p.objects {
                ids.extend(o.text_boxes());
            }
        }
        Block::Table(t) => {
            for r in &t.rows {
                for c in &r.cells {
                    for blk in &c.blocks {
                        block_box_ids(blk, ids);
                    }
                }
            }
        }
    }
}

/// Adopts the text boxes and media of `blocks` from `revised` into `doc`, returning blocks that
/// belong to `doc`. Only the stories and media actually referenced are brought over; blocks with
/// neither are returned as they are.
fn adopt_hf_blocks(doc: &mut Document, revised: &Document, blocks: &[Arc<Block>], rev_has_text_boxes: bool) -> Vec<Arc<Block>> {
    let mut todo = Vec::new();
    if rev_has_text_boxes {
        for b in blocks {
            block_box_ids(b, &mut todo);
        }
    }
    let mut parts = std::collections::BTreeMap::new();
    while let Some(id) = todo.pop() {
        if parts.contains_key(&id) {
            continue;
        }
        if let Some(part) = revised.parts.get(&id).filter(|p| p.kind == wordcraft_doc::PartKind::TextBox) {
            for b in &part.blocks {
                block_box_ids(b, &mut todo);
            }
            parts.insert(id, part.clone());
        }
    }
    let blocks: Vec<Block> = blocks.iter().map(|b| (**b).clone()).collect();
    let media = revised.media_of_blocks(&blocks);
    if parts.is_empty() && media.is_empty() {
        return blocks.into_iter().map(Arc::new).collect();
    }
    let frag = wordcraft_doc::edit::Fragment { blocks, parts, media };
    doc.adopt_fragment(&frag).into_iter().map(Arc::new).collect()
}

/// Returns true if two slices of header/footer blocks, the text boxes they reference and the
/// images those blocks draw have identical structure and content. Two stories may name an image
/// with the same key and hold different bytes (each document keys its own media), so the bytes
/// are compared too: otherwise the revised image would be called unchanged and never adopted.
fn hf_blocks_identical(doc: &Document, revised: &Document, snap_blocks: &[Arc<Block>], rev_blocks: &[Arc<Block>]) -> bool {
    if snap_blocks.len() != rev_blocks.len() {
        return false;
    }
    if !snap_blocks.iter().zip(rev_blocks).all(|(a, b)| Arc::ptr_eq(a, b) || a == b) {
        return false;
    }
    // `media_of_blocks` walks plain blocks; cloning a header's is cheap next to reading a file.
    let owned = |blocks: &[Arc<Block>]| blocks.iter().map(|b| (**b).clone()).collect::<Vec<Block>>();
    let snap_media = doc.media_of_blocks(&owned(snap_blocks));
    let rev_media = revised.media_of_blocks(&owned(rev_blocks));
    if snap_media.len() != rev_media.len() || snap_media.iter().any(|(key, bytes)| rev_media.get(key) != Some(bytes)) {
        return false;
    }
    let mut snap_boxes = Vec::new();
    for b in snap_blocks {
        block_box_ids(b, &mut snap_boxes);
    }
    let mut rev_boxes = Vec::new();
    for b in rev_blocks {
        block_box_ids(b, &mut rev_boxes);
    }
    if snap_boxes.is_empty() && rev_boxes.is_empty() {
        return true;
    }
    if snap_boxes != rev_boxes {
        return false;
    }
    let mut todo: Vec<(u32, u32)> = snap_boxes.into_iter().zip(rev_boxes).collect();
    let mut seen = std::collections::HashSet::new();
    while let Some((id1, id2)) = todo.pop() {
        if !seen.insert((id1, id2)) {
            continue;
        }
        match (doc.parts.get(&id1), revised.parts.get(&id2)) {
            (Some(p1), Some(p2)) => {
                if p1.blocks != p2.blocks {
                    return false;
                }
                let mut b1 = Vec::new();
                for b in &p1.blocks {
                    block_box_ids(b, &mut b1);
                }
                let mut b2 = Vec::new();
                for b in &p2.blocks {
                    block_box_ids(b, &mut b2);
                }
                if b1.len() != b2.len() {
                    return false;
                }
                todo.extend(b1.into_iter().zip(b2));
            }
            (None, None) => {}
            _ => return false,
        }
    }
    true
}

/// Section snapshots and boundaries for original and revised documents during comparison.
struct SectionSnapshots<'a> {
    orig_sections: &'a [wordcraft_doc::section::SectionProps],
    rev_sections: &'a [wordcraft_doc::section::SectionProps],
    orig_ends: &'a [usize],
    rev_ends: &'a [usize],
    orig_eff_headers: &'a [wordcraft_doc::section::HeaderSet],
    orig_eff_footers: &'a [wordcraft_doc::section::HeaderSet],
}

/// Compares header and footer stories across all sections in the original and revised documents,
/// tracking insertions, deletions, and formatting changes in-place on `s.doc.parts`.
fn compare_headers_and_footers(
    s: &mut Session,
    revised: &Document,
    snaps: &SectionSnapshots<'_>,
    sources: &[(Option<usize>, Option<usize>)],
    rev_has_text_boxes: bool,
    run: &CompareRun<'_>,
    ctx: &mut DiffCtx<'_>,
) -> usize {
    let mut changes = 0usize;
    let mut seen_inserted = std::collections::HashSet::new();
    let mut seen_deleted = std::collections::HashSet::new();
    let mut deleted_clones: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut inserted_clones: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut pair_to_target = std::collections::HashMap::new();
    let orig_parts_snapshot: std::collections::HashMap<u32, (wordcraft_doc::PartKind, Vec<Arc<Block>>)> = s
        .doc
        .parts
        .iter()
        .filter(|(_, p)| matches!(p.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer))
        .map(|(&id, p)| (id, (p.kind, p.blocks.clone())))
        .collect();
    let mut matched_orig_parts = std::collections::HashSet::new();

    let orig_eff_headers = snaps.orig_eff_headers;
    let orig_eff_footers = snaps.orig_eff_footers;

    let mut rev_eff_headers = Vec::with_capacity(snaps.rev_sections.len());
    let mut rev_eff_footers = Vec::with_capacity(snaps.rev_sections.len());
    let mut prev_h = wordcraft_doc::section::HeaderSet::default();
    let mut prev_f = wordcraft_doc::section::HeaderSet::default();
    for sec in snaps.rev_sections {
        prev_h = wordcraft_doc::section::HeaderSet {
            default: sec.headers.default.or(prev_h.default),
            first: sec.headers.first.or(prev_h.first),
            even: sec.headers.even.or(prev_h.even),
        };
        prev_f = wordcraft_doc::section::HeaderSet {
            default: sec.footers.default.or(prev_f.default),
            first: sec.footers.first.or(prev_f.first),
            even: sec.footers.even.or(prev_f.even),
        };
        rev_eff_headers.push(prev_h);
        rev_eff_footers.push(prev_f);
    }

    let res_sections = s.doc.sections();
    let res_ends: Vec<usize> = res_sections.iter().map(|(e, _)| *e).collect();
    let mapping = map_result_sections_to_snapshots(&res_sections, snaps.orig_ends, snaps.rev_ends, sources);
    let mut flag_changed = false;
    let orig_even_odd = s.doc.settings.even_odd_headers;
    let rev_even_odd = revised.settings.even_odd_headers;
    if orig_even_odd != rev_even_odd {
        s.doc.settings.even_odd_headers = rev_even_odd;
        s.doc.compare_hf.get_or_insert_with(Default::default).even_odd.get_or_insert(orig_even_odd);
        flag_changed = true;
    }

    let mut mapped_orig = std::collections::HashSet::new();

    for (res_si, &(orig_si, rev_si)) in mapping.iter().enumerate() {
        if let Some(osi) = orig_si {
            mapped_orig.insert(osi);
        }

        let orig_sec = orig_si.and_then(|si| snaps.orig_sections.get(si));
        let rev_sec = rev_si.and_then(|si| snaps.rev_sections.get(si));
        let orig_title_page = orig_sec.map(|sec| sec.title_page).unwrap_or(false);
        let rev_title_page = rev_sec.map(|sec| sec.title_page).unwrap_or(false);
        let sec_key = (res_si + 1 < res_ends.len()).then_some(res_si);

        if orig_title_page != rev_title_page {
            if let Some(end) = sec_key.and_then(|si| res_ends.get(si).copied()) {
                let sec = s.doc.section_mut(end);
                sec.title_page = rev_title_page;
                sec.compare_title_page = wordcraft_doc::section::CompareTitlePage(Some(orig_title_page));
            } else {
                s.doc.last_section.title_page = rev_title_page;
                s.doc.last_section.compare_title_page = wordcraft_doc::section::CompareTitlePage(Some(orig_title_page));
            }
            let flags = s.doc.compare_hf.get_or_insert_with(Default::default);
            if !flags.title_page.iter().any(|(k, _)| *k == sec_key) {
                flags.title_page.push((sec_key, orig_title_page));
            }
            flag_changed = true;
        }

        let sec_ctx = SectionContext { res_ends: &res_ends, res_si };

        for footer in [false, true] {
            for slot in [HfSlot::Default, HfSlot::First, HfSlot::Even] {
                let orig_own = orig_sec.and_then(|sec| get_hf_slot(sec, footer, slot));
                let rev_own = rev_sec.and_then(|sec| get_hf_slot(sec, footer, slot));

                if orig_own.is_none() && rev_own.is_none() {
                    // Both sections leave this slot unlinked/inherited from the previous section.
                    continue;
                }

                let orig_id = orig_si.and_then(|si| {
                    if footer {
                        orig_eff_footers.get(si).and_then(|f| get_set_slot(f, slot))
                    } else {
                        orig_eff_headers.get(si).and_then(|h| get_set_slot(h, slot))
                    }
                });
                let rev_id = rev_si.and_then(|si| {
                    if footer {
                        rev_eff_footers.get(si).and_then(|f| get_set_slot(f, slot))
                    } else {
                        rev_eff_headers.get(si).and_then(|h| get_set_slot(h, slot))
                    }
                });

                match (orig_id, rev_id) {
                    (Some(o), Some(r)) => {
                        let target = if let Some(&existing_target) = pair_to_target.get(&(o, r)) {
                            existing_target
                        } else {
                            let (snap_kind, snap_blocks) =
                                orig_parts_snapshot.get(&o).map(|(k, b)| (*k, b.as_slice())).unwrap_or((wordcraft_doc::PartKind::Header, &[]));
                            let rev_blocks_raw = revised.parts.get(&r).map(|p| p.blocks.as_slice()).unwrap_or(&[]);

                            let are_identical = hf_blocks_identical(&s.doc, revised, snap_blocks, rev_blocks_raw);

                            let target = if matched_orig_parts.insert(o) && !seen_deleted.contains(&o) {
                                o
                            } else {
                                s.doc.add_part(snap_kind, snap_blocks.to_vec())
                            };
                            pair_to_target.insert((o, r), target);

                            if !are_identical {
                                let rev_blocks = adopt_hf_blocks(&mut s.doc, revised, rev_blocks_raw, rev_has_text_boxes);
                                let before_hf = ctx.changes;
                                let hf_out = diff_blocks(&mut s.doc, snap_blocks, &rev_blocks, ctx, 0);
                                let hf_changes = ctx.changes.saturating_sub(before_hf);
                                if let Some(part) = s.doc.parts.get_mut(&target) {
                                    part.blocks = hf_out;
                                }
                                changes += hf_changes;
                            }
                            target
                        };
                        assign_section_slot(s, &sec_ctx, footer, slot, target);
                    }
                    (None, Some(r)) => {
                        let doc_id = if let Some(&existing_id) = inserted_clones.get(&r) {
                            existing_id
                        } else {
                            let rev_blocks_raw = revised.parts.get(&r).map(|p| p.blocks.as_slice()).unwrap_or(&[]);
                            let rev_blocks = adopt_hf_blocks(&mut s.doc, revised, rev_blocks_raw, rev_has_text_boxes);
                            let kind = if footer { wordcraft_doc::PartKind::Footer } else { wordcraft_doc::PartKind::Header };
                            let new_id = s.doc.add_part(kind, rev_blocks);
                            if let Some(part) = s.doc.parts.get_mut(&new_id) {
                                part.blocks = part
                                    .blocks
                                    .iter()
                                    .map(|b| {
                                        Arc::new(mark_block(b, &|c| {
                                            if c.del.is_some() {
                                                c.del = Some(run.del);
                                                c.ins = None;
                                            } else {
                                                c.ins = Some(run.ins);
                                            }
                                        }))
                                    })
                                    .collect();
                            }
                            inserted_clones.insert(r, new_id);
                            if seen_inserted.insert(r) {
                                changes += s.doc.parts.get(&new_id).map(|p| p.blocks.len().max(1)).unwrap_or(1);
                            }
                            new_id
                        };
                        assign_section_slot(s, &sec_ctx, footer, slot, doc_id);
                    }
                    (Some(o), None) => {
                        if seen_deleted.contains(&o) {
                            assign_section_slot(s, &sec_ctx, footer, slot, o);
                        } else if matched_orig_parts.contains(&o) {
                            let target = if let Some(&existing) = deleted_clones.get(&o) {
                                existing
                            } else {
                                let (kind, blocks) =
                                    orig_parts_snapshot.get(&o).map(|(k, b)| (*k, b.as_slice())).unwrap_or((wordcraft_doc::PartKind::Header, &[]));
                                let marked: Vec<_> = blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.del = Some(run.del)))).collect();
                                let n = marked.len().max(1);
                                let id = s.doc.add_part(kind, marked);
                                changes += n;
                                deleted_clones.insert(o, id);
                                id
                            };
                            assign_section_slot(s, &sec_ctx, footer, slot, target);
                        } else {
                            seen_deleted.insert(o);
                            if let Some(part) = s.doc.parts.get_mut(&o) {
                                part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.del = Some(run.del)))).collect();
                            }
                            changes += s.doc.parts.get(&o).map(|p| p.blocks.len().max(1)).unwrap_or(1);
                            assign_section_slot(s, &sec_ctx, footer, slot, o);
                        }
                    }
                    (None, None) => {}
                }
            }
        }
    }

    mark_unmapped_orig_hf_deleted(s, snaps.orig_sections, &mapped_orig, &matched_orig_parts, &mut seen_deleted, run.del, &mut changes);

    if changes == 0 && flag_changed {
        changes += 1;
    }
    changes
}

/// Marks header/footer parts from unmapped original sections as deleted when not already matched or seen.
fn mark_unmapped_orig_hf_deleted(
    s: &mut Session,
    orig_sections: &[wordcraft_doc::section::SectionProps],
    mapped_orig: &std::collections::HashSet<usize>,
    matched_orig_parts: &std::collections::HashSet<u32>,
    seen_deleted: &mut std::collections::HashSet<u32>,
    del_rev: u32,
    changes: &mut usize,
) {
    for (osi, orig_sec) in orig_sections.iter().enumerate() {
        if mapped_orig.contains(&osi) {
            continue;
        }
        for footer in [false, true] {
            for slot in [HfSlot::Default, HfSlot::First, HfSlot::Even] {
                if let Some(o) = get_hf_slot(orig_sec, footer, slot)
                    && !matched_orig_parts.contains(&o)
                    && seen_deleted.insert(o)
                {
                    if let Some(part) = s.doc.parts.get_mut(&o) {
                        part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.del = Some(del_rev)))).collect();
                    }
                    *changes += s.doc.parts.get(&o).map(|p| p.blocks.len().max(1)).unwrap_or(1);
                }
            }
        }
    }
}

/// Returns the effective headers and footers for the original document at block index `orig_idx`.
fn effective_orig_hf(run: &CompareRun<'_>, orig_idx: usize) -> (wordcraft_doc::section::HeaderSet, wordcraft_doc::section::HeaderSet) {
    let osi = run.orig_ends.iter().position(|e| orig_idx <= *e).unwrap_or(run.orig_ends.len().saturating_sub(1));
    let h = run.orig_eff_headers.get(osi).copied().unwrap_or_default();
    let f = run.orig_eff_footers.get(osi).copied().unwrap_or_default();
    (h, f)
}

/// Adjusts section headers and footers for a paragraph in the result document.
///
/// When `headersFooters` comparison is disabled, section breaks preserve original section
/// headers or inherit the effective headers of the surrounding original section to prevent
/// dangling or colliding revised part IDs.
fn sanitize_section_hf(
    orig_sec: Option<&wordcraft_doc::section::SectionProps>,
    res_sec: &mut wordcraft_doc::section::SectionProps,
    run: &CompareRun<'_>,
    orig_idx: usize,
) {
    if !run.compare_hf {
        if let Some(sa) = orig_sec {
            res_sec.headers = sa.headers;
            res_sec.footers = sa.footers;
            res_sec.title_page = sa.title_page;
        } else {
            let (eff_h, eff_f) = effective_orig_hf(run, orig_idx);
            res_sec.headers = eff_h;
            res_sec.footers = eff_f;
        }
    } else if let Some(sa) = orig_sec {
        res_sec.headers = sa.headers;
        res_sec.footers = sa.footers;
    }
}

/// Compare two block slices (`old` vs `new`) at table nesting `depth`, emitting tracked changes.
fn diff_blocks(doc: &mut Document, old: &[Arc<Block>], new: &[Arc<Block>], ctx: &mut DiffCtx<'_>, depth: usize) -> Vec<Arc<Block>> {
    diff_blocks_inner(doc, old, new, ctx, depth, None, None)
}

/// Compare two block slices (`old` vs `new`), optionally sanitizing top-level section headers/footers
/// and recording block correspondence in `sources`.
fn diff_blocks_inner(
    doc: &mut Document,
    old: &[Arc<Block>],
    new: &[Arc<Block>],
    ctx: &mut DiffCtx<'_>,
    depth: usize,
    hf_run: Option<&CompareRun<'_>>,
    mut sources: Option<&mut Vec<(Option<usize>, Option<usize>)>>,
) -> Vec<Arc<Block>> {
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
                    if !same_objects(doc, pa, pb) {
                        // Same text, but objects (text boxes, pictures…) differ: diffed, so the
                        // original's objects aren't lost.
                        out.push(para_block(word_diff(doc, pa, pb, ctx)));
                        if let Some(src) = sources.as_deref_mut() {
                            src.push((Some(i), Some(j)));
                        }
                        ctx.changes = ctx.changes.saturating_add(1);
                    } else {
                        let mut p = pb.clone();
                        if let Some(sb) = p.section.as_mut()
                            && let Some(run) = hf_run
                        {
                            sanitize_section_hf(pa.section.as_deref(), sb, run, i);
                        }
                        if ctx.opts.formatting && pa.text == pb.text {
                            let before = doc.revisions.len();
                            let unformatted = p.clone();
                            crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, &mut doc.revisions, ctx.author, ctx.date);
                            if doc.revisions.len() > before || p != unformatted {
                                ctx.changes = ctx.changes.saturating_add(1);
                            }
                        }
                        out.push(para_block(p));
                        if let Some(src) = sources.as_deref_mut() {
                            src.push((Some(i), Some(j)));
                        }
                    }
                } else if let (Some(Block::Table(ta)), Some(Block::Table(tb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    diff_matched_table(doc, (a, ta), (b, tb), ctx, depth, &mut out);
                    if let Some(src) = sources.as_deref_mut() {
                        src.push((a.is_some().then_some(i), b.is_some().then_some(j)));
                    }
                } else if let Some(x) = b.or(a) {
                    out.push(x.clone());
                    if let Some(src) = sources.as_deref_mut() {
                        src.push((a.is_some().then_some(i), b.is_some().then_some(j)));
                    }
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
                            out.push(para_block(word_diff(doc, pa, pb, ctx)));
                            if let Some(src) = sources.as_deref_mut() {
                                src.push((Some(i + x), Some(j + x)));
                            }
                        }
                        _ => {
                            if let Some(bx) = a {
                                out.push(Arc::new(mark_block(bx, &|c| c.del = Some(ctx.del))));
                                if let Some(src) = sources.as_deref_mut() {
                                    src.push((Some(i + x), None));
                                }
                            }
                            if let Some(bx) = b {
                                let mut marked = mark_block(bx, &|c| c.ins = Some(ctx.ins));
                                if let Block::Para(p) = &mut marked
                                    && let Some(sb) = p.section.as_mut()
                                    && let Some(run) = hf_run
                                {
                                    let orig_idx = if x < dels { i + x } else { i + dels };
                                    sanitize_section_hf(None, sb, run, orig_idx);
                                }
                                out.push(Arc::new(marked));
                                if let Some(src) = sources.as_deref_mut() {
                                    src.push((None, Some(j + x)));
                                }
                            }
                        }
                    }
                    ctx.changes = ctx.changes.saturating_add(1);
                }
                i += dels;
                j += inss;
                k += run_len.max(1);
            }
            None => break,
        }
    }
    out
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
            Tok::Obj(o, _) => obj_key(doc, o),
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
