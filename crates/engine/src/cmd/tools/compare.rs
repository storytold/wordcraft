use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::para::OBJ;

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character", "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared), "headersFooters"?: bool (default true: compare headers and footers)}"#;

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
/// footnotes, text boxes, fields) are accepted and listed in the result. Headers and footers are
/// compared by default (`headersFooters`: true), tracking edits, insertions, deletions, and paragraph
/// formatting changes within header and footer stories.
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
    let compare_headers_footers = p::bool(v, "headersFooters").unwrap_or(true);
    let not_honoured: Vec<&str> = ["moves", "comments", "footnotes", "textBoxes", "fields"].into_iter().filter(|k| v.get(*k).is_some()).collect();
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
    let old: Vec<Arc<Block>> = s.doc.body.clone();
    let new: Vec<Arc<Block>> = if rev_has_text_boxes {
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

    let run = CompareRun {
        ins,
        del,
        opts: &opts,
        author: &author,
        date: &date,
        compare_hf: compare_headers_footers,
        orig_ends: &orig_ends,
        orig_eff_headers: &orig_eff_headers,
        orig_eff_footers: &orig_eff_footers,
    };
    let mut budget = MAX_COMPARE_CELLS;
    let (out, body_changes, sources) = compare_blocks(&s.doc.parts, &mut s.doc.revisions, &run, &old, &new, true, &mut budget);
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
        changes += compare_headers_and_footers(s, &revised, &snaps, &sources, rev_has_text_boxes, &run, &mut budget);
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

/// Assigns a header or footer part ID to a specific section index in the session's document,
/// optionally enabling section or document flags (such as `title_page` or `even_odd_headers`)
/// so inserted First or Even stories remain selectable under layout. Precomputed `res_ends`
/// avoids rescanning document sections on every slot assignment.
fn assign_section_slot(s: &mut Session, res_ends: &[usize], res_si: usize, footer: bool, slot: HfSlot, id: u32, enable_flag: bool) {
    if res_si + 1 < res_ends.len()
        && let Some(&end) = res_ends.get(res_si)
    {
        let sect = s.doc.section_mut(end);
        set_hf_slot(sect, footer, slot, Some(id));
        if enable_flag && slot == HfSlot::First {
            sect.title_page = true;
        }
    } else {
        set_hf_slot(&mut s.doc.last_section, footer, slot, Some(id));
        if enable_flag && slot == HfSlot::First {
            s.doc.last_section.title_page = true;
        }
    }
    if enable_flag && slot == HfSlot::Even {
        s.doc.settings.even_odd_headers = true;
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

/// Adopts any text boxes contained within `blocks` from `revised` into `doc`, returning the adapted blocks.
/// If `blocks` contains no text boxes, returns `blocks` directly without cloning `revised.parts`.
fn adopt_hf_blocks(doc: &mut Document, revised: &Document, blocks: &[Arc<Block>], rev_has_text_boxes: bool) -> Vec<Arc<Block>> {
    if !rev_has_text_boxes {
        return blocks.to_vec();
    }
    let mut todo = Vec::new();
    for b in blocks {
        block_box_ids(b, &mut todo);
    }
    if todo.is_empty() {
        return blocks.to_vec();
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
    let frag = wordcraft_doc::edit::Fragment { blocks: blocks.iter().map(|b| (**b).clone()).collect(), parts };
    doc.adopt_fragment(&frag).into_iter().map(Arc::new).collect()
}

/// Returns true if two slices of header/footer blocks and all referenced text boxes in `doc_parts`
/// and `rev_parts` have identical structure and content.
fn hf_blocks_identical(
    snap_blocks: &[Arc<Block>],
    rev_blocks: &[Arc<Block>],
    doc_parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>,
    rev_parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>,
) -> bool {
    if snap_blocks.len() != rev_blocks.len() {
        return false;
    }
    if !snap_blocks.iter().zip(rev_blocks).all(|(a, b)| Arc::ptr_eq(a, b) || a == b) {
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
    for id in snap_boxes {
        match (doc_parts.get(&id), rev_parts.get(&id)) {
            (Some(p1), Some(p2)) => {
                if p1.blocks != p2.blocks {
                    return false;
                }
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
    budget: &mut u64,
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

    let mut mapped_orig = std::collections::HashSet::new();

    for (res_si, &(orig_si, rev_si)) in mapping.iter().enumerate() {
        if let Some(osi) = orig_si {
            mapped_orig.insert(osi);
        }

        let orig_sec = orig_si.and_then(|si| snaps.orig_sections.get(si));
        let rev_sec = rev_si.and_then(|si| snaps.rev_sections.get(si));

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

                            let are_identical = hf_blocks_identical(snap_blocks, rev_blocks_raw, &s.doc.parts, &revised.parts);

                            let target = if matched_orig_parts.insert(o) && !seen_deleted.contains(&o) {
                                o
                            } else {
                                s.doc.add_part(snap_kind, snap_blocks.to_vec())
                            };
                            pair_to_target.insert((o, r), target);

                            if !are_identical {
                                let rev_blocks = adopt_hf_blocks(&mut s.doc, revised, rev_blocks_raw, rev_has_text_boxes);
                                let (hf_out, hf_changes, _) =
                                    compare_blocks(&s.doc.parts, &mut s.doc.revisions, run, snap_blocks, &rev_blocks, false, budget);
                                if let Some(part) = s.doc.parts.get_mut(&target) {
                                    part.blocks = hf_out;
                                }
                                changes += hf_changes;
                            }
                            target
                        };
                        assign_section_slot(s, &res_ends, res_si, footer, slot, target, false);
                    }
                    (None, Some(r)) => {
                        let enable_flag = (slot == HfSlot::First && rev_sec.is_some_and(|sec| sec.title_page))
                            || (slot == HfSlot::Even && revised.settings.even_odd_headers);
                        let doc_id = if let Some(&existing_id) = inserted_clones.get(&r) {
                            existing_id
                        } else {
                            let rev_blocks_raw = revised.parts.get(&r).map(|p| p.blocks.as_slice()).unwrap_or(&[]);
                            let rev_blocks = adopt_hf_blocks(&mut s.doc, revised, rev_blocks_raw, rev_has_text_boxes);
                            let kind = if footer { wordcraft_doc::PartKind::Footer } else { wordcraft_doc::PartKind::Header };
                            let new_id = s.doc.add_part(kind, rev_blocks);
                            if let Some(part) = s.doc.parts.get_mut(&new_id) {
                                part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.ins = Some(run.ins), 0))).collect();
                            }
                            inserted_clones.insert(r, new_id);
                            if seen_inserted.insert(r) {
                                changes += s.doc.parts.get(&new_id).map(|p| p.blocks.len().max(1)).unwrap_or(1);
                            }
                            new_id
                        };
                        assign_section_slot(s, &res_ends, res_si, footer, slot, doc_id, enable_flag);
                    }
                    (Some(o), None) => {
                        if seen_deleted.contains(&o) {
                            assign_section_slot(s, &res_ends, res_si, footer, slot, o, false);
                        } else if matched_orig_parts.contains(&o) {
                            let target = if let Some(&existing) = deleted_clones.get(&o) {
                                existing
                            } else {
                                let (kind, blocks) =
                                    orig_parts_snapshot.get(&o).map(|(k, b)| (*k, b.as_slice())).unwrap_or((wordcraft_doc::PartKind::Header, &[]));
                                let marked: Vec<_> = blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.del = Some(run.del), 0))).collect();
                                let n = marked.len().max(1);
                                let id = s.doc.add_part(kind, marked);
                                changes += n;
                                deleted_clones.insert(o, id);
                                id
                            };
                            assign_section_slot(s, &res_ends, res_si, footer, slot, target, false);
                        } else {
                            seen_deleted.insert(o);
                            if let Some(part) = s.doc.parts.get_mut(&o) {
                                part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.del = Some(run.del), 0))).collect();
                            }
                            changes += s.doc.parts.get(&o).map(|p| p.blocks.len().max(1)).unwrap_or(1);
                            assign_section_slot(s, &res_ends, res_si, footer, slot, o, false);
                        }
                    }
                    (None, None) => {}
                }
            }
        }
    }

    for (osi, orig_sec) in snaps.orig_sections.iter().enumerate() {
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
                        part.blocks = part.blocks.iter().map(|b| Arc::new(mark_block(b, &|c| c.del = Some(run.del), 0))).collect();
                    }
                    changes += s.doc.parts.get(&o).map(|p| p.blocks.len().max(1)).unwrap_or(1);
                }
            }
        }
    }

    changes
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

/// Compares two slices of blocks (`old` and `new`) and returns the combined blocks with tracked
/// revisions along with the count of changes and block source indices. When `track_sources` is false,
/// block correspondence tracking is skipped to avoid allocations and vector operations.
fn compare_blocks(
    parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>,
    revisions: &mut Vec<wordcraft_doc::Revision>,
    run: &CompareRun<'_>,
    old: &[Arc<Block>],
    new: &[Arc<Block>],
    track_sources: bool,
    budget: &mut u64,
) -> (Vec<Arc<Block>>, usize, Vec<(Option<usize>, Option<usize>)>) {
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => run.opts.key(&p.plain_text()),
        // Tables match as a whole, by their text.
        Block::Table(_) => format!("\u{0}table\u{0}{}", run.opts.key(&block_text(b, 0))),
    };
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), budget);
    let mut out: Vec<Arc<Block>> = Vec::new();
    let mut sources: Vec<(Option<usize>, Option<usize>)> = if track_sources { Vec::with_capacity(old.len() + new.len()) } else { Vec::new() };
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
                        out.push(para_block(word_diff(parts, pa, pb, run, i, budget)));
                        if track_sources {
                            sources.push((Some(i), Some(j)));
                        }
                        changes += 1;
                        i += 1;
                        j += 1;
                        k += 1;
                        continue;
                    }
                    let mut p = pb.clone();
                    if let Some(sb) = p.section.as_mut() {
                        sanitize_section_hf(pa.section.as_deref(), sb, run, i);
                    }
                    if run.opts.formatting && pa.text == pb.text {
                        let before = revisions.len();
                        let unformatted = p.clone();
                        crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, revisions, run.author, run.date);
                        if revisions.len() > before || p != unformatted {
                            changes += 1;
                        }
                    }
                    out.push(para_block(p));
                    if track_sources {
                        sources.push((Some(i), Some(j)));
                    }
                } else if let Some(x) = b.or(a) {
                    out.push(x.clone());
                    if track_sources {
                        sources.push((a.is_some().then_some(i), b.is_some().then_some(j)));
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
                            out.push(para_block(word_diff(parts, pa, pb, run, i + x, budget)));
                            if track_sources {
                                sources.push((Some(i + x), Some(j + x)));
                            }
                        }
                        _ => {
                            if let Some(bx) = a {
                                out.push(Arc::new(mark_block(bx, &|c| c.del = Some(run.del), 0)));
                                if track_sources {
                                    sources.push((Some(i + x), None));
                                }
                            }
                            if let Some(bx) = b {
                                let mut marked = mark_block(bx, &|c| c.ins = Some(run.ins), 0);
                                if let Block::Para(p) = &mut marked
                                    && let Some(sb) = p.section.as_mut()
                                {
                                    let orig_idx = if x < dels { i + x } else { i + dels };
                                    sanitize_section_hf(None, sb, run, orig_idx);
                                }
                                out.push(Arc::new(marked));
                                if track_sources {
                                    sources.push((None, Some(j + x)));
                                }
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
    (out, changes, sources)
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

/// Extracts the plain text of a document part by joining the text of its blocks.
fn part_text(p: &wordcraft_doc::Part) -> String {
    p.blocks.iter().map(|b| block_text(b, 0)).collect::<Vec<_>>().join("\n")
}

/// What a comparison sees of a token: the text as the options say, an object with the text of
/// the text boxes it shows instead of their story ids (each document numbers its own stories).
#[derive(PartialEq)]
enum Key {
    Text(String),
    Obj(Box<InlineObject>, Vec<String>),
}

/// Generates a comparison key for an inline object, extracting text from referenced text box parts.
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

/// Diffs two paragraphs token-by-token (by word or character), producing tracked insertions and deletions.
fn word_diff(
    parts: &std::collections::BTreeMap<u32, wordcraft_doc::Part>,
    a: &Paragraph,
    b: &Paragraph,
    run: &CompareRun<'_>,
    orig_idx: usize,
    budget: &mut u64,
) -> Paragraph {
    let (at, bt) = (a.plain_text(), b.plain_text());
    let chars = run.opts.characters && at.chars().count().max(bt.chars().count()) <= MAX_CHAR_DIFF;
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let key = |t: &Tok<'_>| match t {
        Tok::Text(t) => Key::Text(run.opts.key(t)),
        Tok::Obj(o) => obj_key(parts, o),
    };
    let ops = lcs_ops(&ta.iter().map(key).collect::<Vec<_>>(), &tb.iter().map(key).collect::<Vec<_>>(), budget);
    // Only at character granularity: a matched whole word almost never happens by coincidence,
    // but a matched single character very often does (digits, punctuation), so only there does
    // a short interior match need folding back into its surrounding edit.
    let ops = if chars { collapse_noise_matches(&ops) } else { ops };
    let mut p = Paragraph::new();
    p.props = b.props.clone();
    p.section = b.section.clone();
    if let Some(sb) = p.section.as_mut() {
        sanitize_section_hf(a.section.as_deref(), sb, run, orig_idx);
    }
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
                put(&mut p, ta.get(i), &CharProps { del: Some(run.del), ..base.clone() });
                i += 1;
            }
            Op::Ins => {
                put(&mut p, tb.get(j), &CharProps { ins: Some(run.ins), ..base.clone() });
                j += 1;
            }
        }
    }
    p
}
