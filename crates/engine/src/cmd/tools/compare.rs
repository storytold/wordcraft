use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use wordcraft_doc::para::OBJ;

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character", "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared)}"#;

/// What Compare / Combine compares (the More >> settings of Word's dialog).
#[derive(Clone, Copy, Debug)]
pub(super) struct CompareOptions {
    /// Differences only in letter case count as changes.
    pub case: bool,
    /// Differences only in white space count as changes.
    pub white_space: bool,
    /// Formatting differences become tracked formatting changes.
    pub formatting: bool,
    /// Compare table structure/content.
    #[allow(dead_code)] // Parsed for option fidelity/reporting; body compare always compares tables today.
    pub tables: bool,
    /// Changed paragraphs are compared character by character (else word by word).
    pub characters: bool,
}

impl CompareOptions {
    /// The text a comparison sees: case folded and white space collapsed as the options say.
    fn key(&self, t: &str) -> String {
        let t = if self.white_space { t.to_string() } else { t.split_whitespace().collect::<Vec<_>>().join(" ") };
        if self.case { t } else { t.to_lowercase() }
    }

    pub(super) fn from_params(v: &Value) -> Self {
        let level = p::str(v, "level").unwrap_or("word");
        Self {
            case: p::bool(v, "caseChanges").unwrap_or(true),
            white_space: p::bool(v, "whiteSpace").unwrap_or(true),
            formatting: p::bool(v, "formatting").unwrap_or(true),
            tables: p::bool(v, "tables").unwrap_or(true),
            characters: matches!(level, "character" | "char"),
        }
    }
}

/// Options accepted by Compare/Combine that are not fully honoured by the current engine.
/// Reported when the caller requests comparison behavior the body diff does not implement with
/// fidelity (move detection, non-body stories, field semantics, or text-box story diffing).
/// `tables` is compared in the body; `tables:false` is reported because suppression is not
/// implemented with Word fidelity.
pub(super) fn not_honoured_options(v: &Value) -> Vec<&'static str> {
    let mut out = Vec::new();
    for key in ["moves", "comments", "headersFooters", "footnotes", "textBoxes", "fields"] {
        if matches!(v.get(key).and_then(Value::as_bool), Some(true) | None if v.get(key).is_some()) {
            out.push(key);
        }
    }
    if v.get("tables").and_then(Value::as_bool) == Some(false) {
        out.push("tables");
    }
    out
}

pub(super) struct CompareRun {
    pub revised: Document,
    pub show_in: String,
    pub original_loaded: bool,
    pub opts: CompareOptions,
    pub author: String,
    pub not_honoured: Vec<&'static str>,
    pub clear_versions_on_untitled: bool,
    pub combine: bool,
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
    let original_loaded = p::str(v, "original").is_some();
    if let Some(path) = p::str(v, "original") {
        let doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
        s.doc = doc;
        s.doc.ensure_nonempty();
        s.note_body_replaced();
        s.math = None;
    }
    let opts = CompareOptions::from_params(v);
    let not_honoured = not_honoured_options(v);
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    compare_loaded(
        s,
        CompareRun {
            revised,
            show_in: show_in.to_string(),
            original_loaded,
            opts,
            author,
            not_honoured,
            clear_versions_on_untitled: true,
            combine: false,
        },
    )
}

/// Combine revisions from an original and revised document. Uses the compare diff engine with
/// combine mode metadata; full multi-reviewer revision merging fidelity is limited (see
/// `notHonoured`/result `mode`).
pub(super) fn combine(s: &mut Session, v: &Value) -> CmdResult {
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
    let original_loaded = p::str(v, "original").is_some();
    if let Some(path) = p::str(v, "original") {
        let doc = crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?;
        s.doc = doc;
        s.doc.ensure_nonempty();
        s.note_body_replaced();
        s.math = None;
    }
    let opts = CompareOptions::from_params(v);
    let not_honoured = not_honoured_options(v);
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    compare_loaded(
        s,
        CompareRun {
            revised,
            show_in: show_in.to_string(),
            original_loaded,
            opts,
            author,
            not_honoured,
            clear_versions_on_untitled: true,
            combine: true,
        },
    )
}

/// Compare `s.doc` (the original) against `revised`, replacing `s.doc` with the result.
/// When `clear_versions_on_untitled` is true, an untitled compare result clears saved versions,
/// preserving the historical `review.compare`/`review.combine` behavior.
pub(super) fn compare_loaded(s: &mut Session, run: CompareRun) -> CmdResult {
    let CompareRun { revised, show_in, original_loaded, opts, author, not_honoured, clear_versions_on_untitled, combine } = run;
    if !matches!(show_in.as_str(), "original" | "revised" | "new") {
        return Err(CmdError::Params("`showIn` is \"original\", \"revised\" or \"new\"".into()));
    }
    let date = crate::cmd::now_iso();
    let old: Vec<Arc<Block>> = s.doc.body.clone();
    // Adopt revised content so revision ids, styles, numbering, media, notes and text boxes are
    // remapped into this document. Combine keeps revised styles/identity (`keep_styles`); compare
    // uses the engine's normal adoption.
    let mut perms = 0u32;
    let new: Vec<Arc<Block>> = s.doc.adopt_blocks(revised, combine, &mut perms).into_iter().map(Arc::new).collect();
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
                // A run of deletions and insertions: changed paragraphs pair up and are diffed
                // word by word (or by character); the rest are deleted or inserted whole.
                let run = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o != Op::Same).count();
                let dels = ops.get(k..k + run).unwrap_or_default().iter().filter(|o| **o == Op::Del).count();
                let inss = run - dels;
                for x in 0..dels.max(inss) {
                    let (a, b) = (if x < dels { old.get(i + x) } else { None }, if x < inss { new.get(j + x) } else { None });
                    match (a.map(|x| &**x), b.map(|x| &**x)) {
                        (Some(Block::Para(pa)), Some(Block::Para(pb))) => {
                            out.push(para_block(word_diff(&s.doc, pa, pb, ins, del, &opts, &mut budget)))
                        }
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
    if show_in != "original" || original_loaded {
        // The result is a new, untitled document: saving it never overwrites either file.
        s.path = None;
        s.file_name = None;
        // Version snapshots belong to the document they were taken from. Compare/Combine clear
        // them for untitled results; version-to-version compare can preserve them explicitly.
        if clear_versions_on_untitled {
            s.clear_versions();
        }
    }
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    Ok(json!({"changes": changes, "author": author, "notHonoured": not_honoured, "mode": if combine { "combine" } else { "compare" }}))
}

/// Compare saved document versions. `version` is the original snapshot index; `other` is another
/// snapshot index or `"current"` (default) for the live document. The result defaults to a new
/// untitled document and preserves version snapshots unless `preserveVersions` is false.
pub(super) fn compare_versions(s: &mut Session, v: &Value) -> CmdResult {
    let version = if v.get("version").is_some_and(|x| !x.is_null()) {
        let i = p::u64(v, "version").ok_or_else(|| CmdError::Params("version (non-negative integer) is required".into()))?;
        usize::try_from(i).map_err(|_| CmdError::Params("no such version".into()))?
    } else {
        return Err(CmdError::Params("`version` is required".into()));
    };
    let original = s.versions.get(version).map(|(_, _, d)| d.clone()).ok_or_else(|| CmdError::Params("no such version".into()))?;
    let revised = match v.get("other") {
        None | Some(Value::Null) => s.doc.clone(),
        Some(Value::String(name)) if name == "current" => s.doc.clone(),
        Some(Value::String(_)) => return Err(CmdError::Params("`other` is a version index or \"current\"".into())),
        Some(other) => {
            let i = other
                .as_u64()
                .or_else(|| other.as_i64().and_then(|i| u64::try_from(i).ok()))
                .ok_or_else(|| CmdError::Params("`other` is a version index or \"current\"".into()))?;
            let i = usize::try_from(i).map_err(|_| CmdError::Params("no such version".into()))?;
            s.versions.get(i).map(|(_, _, d)| d.clone()).ok_or_else(|| CmdError::Params("no such version".into()))?
        }
    };
    let show_in = p::str(v, "showIn").unwrap_or("new");
    if !matches!(show_in, "original" | "revised" | "new") {
        return Err(CmdError::Params("`showIn` is \"original\", \"revised\" or \"new\"".into()));
    }
    // Comparing into the live document replaces content; a protected document must be stopped
    // first, the same as restoring a version.
    if show_in == "original" {
        crate::cmd::protect::check_version_restore(s)?;
    }
    let opts = CompareOptions::from_params(v);
    let not_honoured = not_honoured_options(v);
    let author = p::str(v, "author").map(str::trim).filter(|a| !a.is_empty()).map(str::to_string).unwrap_or_else(|| s.author.clone());
    let preserve_versions = p::bool(v, "preserveVersions").unwrap_or(true);
    s.checkpoint("Compare Versions");
    s.doc = original;
    s.doc.ensure_nonempty();
    s.note_body_replaced();
    s.math = None;
    let mut result = compare_loaded(
        s,
        CompareRun {
            revised,
            show_in: show_in.to_string(),
            original_loaded: true,
            opts,
            author,
            not_honoured,
            clear_versions_on_untitled: !preserve_versions,
            combine: false,
        },
    )?;
    if let Value::Object(map) = &mut result {
        map.insert("version".into(), json!(version));
        map.insert("other".into(), v.get("other").cloned().unwrap_or_else(|| json!("current")));
        map.insert("preserveVersions".into(), json!(preserve_versions));
    }
    Ok(result)
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

pub(super) const MERGE3_PARAMS: &str = r#"{"base": {"path"?: string, "text"?: string} | path string (common ancestor), "ours"?: {"path"?: string, "text"?: string} | path string (default: current document), "theirs": {"path"?: string, "text"?: string} | path string, "showIn"?: "new"|"original" (default new), "conflicts"?: "markers"|"ours"|"theirs" (default markers)}. Merges body blocks only (block/paragraph-granular with word-level refinement for simple 1:1 paragraph text conflicts). Headers/footers, notes, comments, text boxes and other non-body stories are not merged; tables merge as whole blocks. Reports scope/nonBodyStoriesMerged/tables/wordLevelRefinements/notes."#;

fn load_source(s: &Session, v: &Value, key: &str, ours_default_current: bool) -> Result<Document, CmdError> {
    let src = v.get(key);
    let (path, text) = match src {
        Some(Value::String(p)) => (Some(p.as_str()), None),
        Some(Value::Object(m)) => (m.get("path").and_then(Value::as_str), m.get("text").and_then(Value::as_str)),
        None if ours_default_current && key == "ours" => return Ok(s.doc.clone()),
        None => (None, None),
        _ => return Err(CmdError::Params(format!("`{key}` must be a path string or {{\"path\"|\"text\"}}"))),
    };
    if let Some(t) = text {
        return Ok(Document::from_text(t));
    }
    if let Some(path) = path {
        return crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed);
    }
    if ours_default_current && key == "ours" {
        return Ok(s.doc.clone());
    }
    Err(CmdError::Params(format!("`{key}.path` or `{key}.text` (or `{key}` as a path string) is required")))
}

fn block_key(b: &Block) -> String {
    match b {
        Block::Para(p) => p.plain_text(),
        Block::Table(_) => format!("\u{0}table\u{0}{}", block_text(b, 0)),
    }
}

#[derive(Clone, Debug)]
struct SideView {
    /// Insertions before each base index, and one trailing slot after the last base block.
    ins: Vec<Vec<Arc<Block>>>,
    /// Per base block: kept (side's version) or deleted.
    kept: Vec<Option<Arc<Block>>>,
}

fn side_view(base: &[Arc<Block>], side: &[Arc<Block>]) -> SideView {
    let mut budget = MAX_COMPARE_CELLS;
    let ops = lcs_ops(&base.iter().map(|b| block_key(b)).collect::<Vec<_>>(), &side.iter().map(|b| block_key(b)).collect::<Vec<_>>(), &mut budget);
    let mut ins = vec![Vec::new(); base.len() + 1];
    let mut kept = Vec::with_capacity(base.len());
    let (mut bi, mut si, mut k) = (0usize, 0usize, 0usize);
    let mut at = 0usize;
    while k < ops.len() {
        match ops.get(k).copied() {
            Some(Op::Same) => {
                kept.push(side.get(si).cloned().or_else(|| base.get(bi).cloned()));
                bi += 1;
                si += 1;
                at = at.saturating_add(1);
                k += 1;
            }
            Some(Op::Ins) if matches!(ops.get(k + 1), Some(Op::Del)) => {
                kept.push(side.get(si).cloned().or_else(|| base.get(bi).cloned()));
                bi += 1;
                si += 1;
                at = at.saturating_add(1);
                k += 2;
            }
            Some(Op::Del) if matches!(ops.get(k + 1), Some(Op::Ins)) => {
                kept.push(side.get(si).cloned().or_else(|| base.get(bi).cloned()));
                bi += 1;
                si += 1;
                at = at.saturating_add(1);
                k += 2;
            }
            Some(Op::Ins) => {
                if let Some(sb) = side.get(si)
                    && let Some(slot) = ins.get_mut(at)
                {
                    slot.push(sb.clone());
                }
                si += 1;
                k += 1;
            }
            Some(Op::Del) => {
                kept.push(None);
                bi += 1;
                at = at.saturating_add(1);
                k += 1;
            }
            None => break,
        }
    }
    while bi < base.len() {
        kept.push(None);
        bi += 1;
    }
    while si < side.len() {
        if let Some(sb) = side.get(si)
            && let Some(slot) = ins.last_mut()
        {
            slot.push(sb.clone());
        }
        si += 1;
    }
    SideView { ins, kept }
}

fn same_blocks(a: &[Arc<Block>], b: &[Arc<Block>]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| block_key(x) == block_key(y))
}

fn simple_para_text(b: &Block) -> Option<String> {
    match b {
        Block::Para(p) if p.objects.is_empty() && p.text.chars().all(|c| c != OBJ) => Some(p.plain_text()),
        _ => None,
    }
}

fn merge_text_tokens(base: &[&str], ours: &[&str], theirs: &[&str], conflict_mode: &str) -> (Vec<String>, usize) {
    let ov = side_view_tokens(base, ours);
    let tv = side_view_tokens(base, theirs);
    let mut out = Vec::new();
    let mut conflicts = 0usize;
    for i in 0..=base.len() {
        let oi = ov.ins.get(i).cloned().unwrap_or_default();
        let ti = tv.ins.get(i).cloned().unwrap_or_default();
        if !oi.is_empty() || !ti.is_empty() {
            if oi == ti || ti.is_empty() {
                out.extend(oi);
            } else if oi.is_empty() {
                out.extend(ti);
            } else {
                conflicts += 1;
                match conflict_mode {
                    "ours" => out.extend(oi),
                    "theirs" => out.extend(ti),
                    _ => {
                        out.push("<<<<<<< ours".to_string());
                        out.extend(oi);
                        out.push("=======".to_string());
                        out.extend(ti);
                        out.push(">>>>>>> theirs".to_string());
                    }
                }
            }
        }
        if i == base.len() {
            break;
        }
        let b = base.get(i).copied().unwrap_or("");
        let ob = ov.kept.get(i).and_then(|x| x.as_deref());
        let tb = tv.kept.get(i).and_then(|x| x.as_deref());
        match (ob, tb) {
            (Some(o), Some(t)) => {
                if o == t {
                    out.push(o.to_string());
                } else if o == b {
                    out.push(t.to_string());
                } else if t == b {
                    out.push(o.to_string());
                } else {
                    conflicts += 1;
                    match conflict_mode {
                        "ours" => out.push(o.to_string()),
                        "theirs" => out.push(t.to_string()),
                        _ => {
                            out.push("<<<<<<< ours".to_string());
                            out.push(o.to_string());
                            out.push("=======".to_string());
                            out.push(t.to_string());
                            out.push(">>>>>>> theirs".to_string());
                        }
                    }
                }
            }
            (None, None) => {}
            (Some(o), None) => {
                if o != b {
                    conflicts += 1;
                    match conflict_mode {
                        "ours" => out.push(o.to_string()),
                        "theirs" => {}
                        _ => {
                            out.push("<<<<<<< ours".to_string());
                            out.push(o.to_string());
                            out.push("=======".to_string());
                            out.push(">>>>>>> theirs (deleted)".to_string());
                        }
                    }
                }
            }
            (None, Some(t)) => {
                if t != b {
                    conflicts += 1;
                    match conflict_mode {
                        "ours" => {}
                        "theirs" => out.push(t.to_string()),
                        _ => {
                            out.push("<<<<<<< ours (deleted)".to_string());
                            out.push("=======".to_string());
                            out.push(t.to_string());
                            out.push(">>>>>>> theirs".to_string());
                        }
                    }
                }
            }
        }
    }
    (out, conflicts)
}

#[derive(Clone, Debug)]
struct TokenSideView {
    ins: Vec<Vec<String>>,
    kept: Vec<Option<String>>,
}

fn side_view_tokens(base: &[&str], side: &[&str]) -> TokenSideView {
    let mut budget = MAX_COMPARE_CELLS;
    let ops = lcs_ops(base, side, &mut budget);
    let mut ins = vec![Vec::new(); base.len() + 1];
    let mut kept = Vec::with_capacity(base.len());
    let (mut bi, mut si, mut k) = (0usize, 0usize, 0usize);
    let mut at = 0usize;
    while k < ops.len() {
        match ops.get(k).copied() {
            Some(Op::Same) => {
                kept.push(Some(side.get(si).copied().unwrap_or_else(|| base.get(bi).copied().unwrap_or("")).to_string()));
                bi += 1;
                si += 1;
                at = at.saturating_add(1);
                k += 1;
            }
            Some(Op::Ins) if matches!(ops.get(k + 1), Some(Op::Del)) => {
                kept.push(Some(side.get(si).copied().unwrap_or("").to_string()));
                bi += 1;
                si += 1;
                at = at.saturating_add(1);
                k += 2;
            }
            Some(Op::Del) if matches!(ops.get(k + 1), Some(Op::Ins)) => {
                kept.push(Some(side.get(si).copied().unwrap_or("").to_string()));
                bi += 1;
                si += 1;
                at = at.saturating_add(1);
                k += 2;
            }
            Some(Op::Ins) => {
                if let Some(slot) = ins.get_mut(at) {
                    slot.push(side.get(si).copied().unwrap_or("").to_string());
                }
                si += 1;
                k += 1;
            }
            Some(Op::Del) => {
                kept.push(None);
                bi += 1;
                at = at.saturating_add(1);
                k += 1;
            }
            None => break,
        }
    }
    while bi < base.len() {
        kept.push(None);
        bi += 1;
    }
    while si < side.len() {
        if let Some(slot) = ins.last_mut() {
            slot.push(side.get(si).copied().unwrap_or("").to_string());
        }
        si += 1;
    }
    TokenSideView { ins, kept }
}

fn try_merge_simple_para_conflict(base_b: &Arc<Block>, ob: &Arc<Block>, tb: &Arc<Block>, conflict_mode: &str) -> Option<(Vec<Arc<Block>>, usize)> {
    let (bt, ot, tt) = (simple_para_text(base_b)?, simple_para_text(ob)?, simple_para_text(tb)?);
    if ot == tt {
        return Some((vec![ob.clone()], 0));
    }
    if ot == bt {
        return Some((vec![tb.clone()], 0));
    }
    if tt == bt {
        return Some((vec![ob.clone()], 0));
    }
    // word-level merge on simple text
    let bw: Vec<&str> = bt.split_inclusive(char::is_whitespace).collect();
    let ow: Vec<&str> = ot.split_inclusive(char::is_whitespace).collect();
    let tw: Vec<&str> = tt.split_inclusive(char::is_whitespace).collect();
    let (merged, conflicts) = merge_text_tokens(&bw, &ow, &tw, conflict_mode);
    if conflicts == 0 {
        return Some((vec![para_block(Paragraph::with_text(&merged.concat(), CharProps::default()))], 0));
    }
    // Render token-level conflicts as separate paragraphs/lines for clarity.
    let mut blocks = Vec::new();
    let mut cur = String::new();
    let flush = |blocks: &mut Vec<Arc<Block>>, cur: &mut String| {
        if !cur.is_empty() {
            blocks.push(para_block(Paragraph::with_text(cur.trim_end_matches('\n'), CharProps::default())));
            cur.clear();
        }
    };
    for t in merged {
        if t == "<<<<<<< ours" || t == "=======" || t == ">>>>>>> theirs" || t == ">>>>>>> theirs (deleted)" || t == "<<<<<<< ours (deleted)" {
            flush(&mut blocks, &mut cur);
            blocks.push(para_block(Paragraph::with_text(&t, CharProps::default())));
        } else {
            cur.push_str(&t);
        }
    }
    flush(&mut blocks, &mut cur);
    if blocks.is_empty() {
        blocks.push(para_block(Paragraph::with_text(&tt, CharProps::default())));
    }
    Some((blocks, conflicts.max(1)))
}

#[derive(Clone, Debug, Default)]
struct Merge3Info {
    tables_involved: bool,
    word_level_refinements: usize,
}

fn has_table_blocks(blocks: &[Arc<Block>]) -> bool {
    blocks.iter().any(|b| matches!(b.as_ref(), Block::Table(_)))
}

/// Three-way merge at paragraph/block granularity. Returns (merged blocks, conflicts, info).
fn merge3_blocks(base: &[Arc<Block>], ours: &[Arc<Block>], theirs: &[Arc<Block>], conflict_mode: &str) -> (Vec<Arc<Block>>, usize, Merge3Info) {
    let ov = side_view(base, ours);
    let tv = side_view(base, theirs);
    let mut out = Vec::new();
    let mut conflicts = 0usize;
    let mut info =
        Merge3Info { tables_involved: has_table_blocks(base) || has_table_blocks(ours) || has_table_blocks(theirs), word_level_refinements: 0 };
    for i in 0..=base.len() {
        let oi = ov.ins.get(i).cloned().unwrap_or_default();
        let ti = tv.ins.get(i).cloned().unwrap_or_default();
        if !oi.is_empty() || !ti.is_empty() {
            if same_blocks(&oi, &ti) || ti.is_empty() {
                out.extend(oi);
            } else if oi.is_empty() {
                out.extend(ti);
            } else {
                conflicts += 1;
                match conflict_mode {
                    "ours" => out.extend(oi),
                    "theirs" => out.extend(ti),
                    _ => {
                        out.push(para_block(Paragraph::with_text("<<<<<<< ours", CharProps::default())));
                        out.extend(oi);
                        out.push(para_block(Paragraph::with_text("=======", CharProps::default())));
                        out.extend(ti);
                        out.push(para_block(Paragraph::with_text(">>>>>>> theirs", CharProps::default())));
                    }
                }
            }
        }
        if i == base.len() {
            break;
        }
        let base_b = base.get(i);
        let ob = ov.kept.get(i).and_then(|x| x.as_ref());
        let tb = tv.kept.get(i).and_then(|x| x.as_ref());
        match (ob, tb) {
            (Some(ob), Some(tb)) => {
                let btxt = base_b.map(|b| block_key(b)).unwrap_or_default();
                let ot = block_key(ob);
                let tt = block_key(tb);
                if ot == tt {
                    out.push((*ob).clone());
                } else if ot == btxt {
                    out.push((*tb).clone());
                } else if tt == btxt {
                    out.push((*ob).clone());
                } else if let Some(base_b) = base_b
                    && let Some((blocks, c)) = try_merge_simple_para_conflict(base_b, ob, tb, conflict_mode)
                {
                    info.word_level_refinements += 1;
                    conflicts += c;
                    out.extend(blocks);
                } else {
                    conflicts += 1;
                    match conflict_mode {
                        "ours" => out.push((*ob).clone()),
                        "theirs" => out.push((*tb).clone()),
                        _ => {
                            out.push(para_block(Paragraph::with_text("<<<<<<< ours", CharProps::default())));
                            out.push((*ob).clone());
                            out.push(para_block(Paragraph::with_text("=======", CharProps::default())));
                            out.push((*tb).clone());
                            out.push(para_block(Paragraph::with_text(">>>>>>> theirs", CharProps::default())));
                        }
                    }
                }
            }
            (None, None) => {}
            (Some(ob), None) => {
                let btxt = base_b.map(|b| block_key(b)).unwrap_or_default();
                if block_key(ob) != btxt {
                    conflicts += 1;
                    match conflict_mode {
                        "ours" => out.push((*ob).clone()),
                        "theirs" => {}
                        _ => {
                            out.push(para_block(Paragraph::with_text("<<<<<<< ours", CharProps::default())));
                            out.push((*ob).clone());
                            out.push(para_block(Paragraph::with_text("=======", CharProps::default())));
                            out.push(para_block(Paragraph::with_text(">>>>>>> theirs (deleted)", CharProps::default())));
                        }
                    }
                }
            }
            (None, Some(tb)) => {
                let btxt = base_b.map(|b| block_key(b)).unwrap_or_default();
                if block_key(tb) != btxt {
                    conflicts += 1;
                    match conflict_mode {
                        "ours" => {}
                        "theirs" => out.push((*tb).clone()),
                        _ => {
                            out.push(para_block(Paragraph::with_text("<<<<<<< ours (deleted)", CharProps::default())));
                            out.push(para_block(Paragraph::with_text("=======", CharProps::default())));
                            out.push((*tb).clone());
                            out.push(para_block(Paragraph::with_text(">>>>>>> theirs", CharProps::default())));
                        }
                    }
                }
            }
        }
    }
    if out.is_empty() {
        out.push(para_block(Paragraph::new()));
    }
    (out, conflicts, info)
}

/// Three-way merge: merge `ours` and `theirs` relative to `base`. Defaults to a new untitled
/// document; `conflicts:"markers"` inserts conflict markers where both sides changed the same
/// base region differently.
fn merge3_result(conflicts: usize, conflict_mode: &str, info: &Merge3Info) -> Value {
    let mut notes = vec![
        "Merges body blocks only (block/paragraph-granular, with word-level refinement for simple 1:1 paragraph text conflicts).".to_string(),
        "Headers/footers, footnotes/endnotes, comments, text boxes and other non-body stories are not merged; non-body content remains from the target document.".to_string(),
        "Adopted styles, revisions, numbering and parts are remapped; formatting merge, revision provenance and complex part/media relationships may need manual review.".to_string(),
    ];
    if info.tables_involved {
        notes.push("Tables are present and merged as whole blocks (no cell-level merge).".to_string());
    }
    if info.word_level_refinements > 0 {
        notes.push(format!("Applied word-level merge refinement to {} simple paragraph conflict region(s).", info.word_level_refinements));
    }
    json!({
        "merged": true,
        "conflicts": conflicts,
        "mode": "merge3",
        "conflictMode": conflict_mode,
        "scope": "body",
        "nonBodyStoriesMerged": false,
        "tables": { "involved": info.tables_involved, "mergedAs": "whole-block" },
        "wordLevelRefinements": info.word_level_refinements,
        "notes": notes,
    })
}

pub(super) fn merge3(s: &mut Session, v: &Value) -> CmdResult {
    let base = load_source(s, v, "base", false)?;
    let theirs = load_source(s, v, "theirs", false)?;
    let ours_explicit = v.get("ours").filter(|x| !x.is_null()).is_some();
    let ours_doc = if ours_explicit { Some(load_source(s, v, "ours", false)?) } else { None };
    let show_in = p::str(v, "showIn").unwrap_or("new");
    if !matches!(show_in, "new" | "original") {
        return Err(CmdError::Params("`showIn` is \"new\" or \"original\"".into()));
    }
    let conflict_mode = p::str(v, "conflicts").unwrap_or("markers");
    if !matches!(conflict_mode, "markers" | "ours" | "theirs") {
        return Err(CmdError::Params("`conflicts` is \"markers\", \"ours\" or \"theirs\"".into()));
    }
    if show_in == "original" {
        crate::cmd::protect::check_version_restore(s)?;
    }
    s.checkpoint("Merge");
    let (merged, conflicts, info) = if let Some(ours) = ours_doc {
        let mut perms = 0u32;
        let base_blocks = s.doc.adopt_blocks(base, false, &mut perms).into_iter().map(Arc::new).collect::<Vec<_>>();
        let theirs_blocks = s.doc.adopt_blocks(theirs, false, &mut perms).into_iter().map(Arc::new).collect::<Vec<_>>();
        let ours_blocks = s.doc.adopt_blocks(ours, false, &mut perms).into_iter().map(Arc::new).collect::<Vec<_>>();
        merge3_blocks(&base_blocks, &ours_blocks, &theirs_blocks, conflict_mode)
    } else {
        let ours_blocks = s.doc.body.clone();
        let mut perms = 0u32;
        let base_blocks = s.doc.adopt_blocks(base, false, &mut perms).into_iter().map(Arc::new).collect::<Vec<_>>();
        let theirs_blocks = s.doc.adopt_blocks(theirs, false, &mut perms).into_iter().map(Arc::new).collect::<Vec<_>>();
        merge3_blocks(&base_blocks, &ours_blocks, &theirs_blocks, conflict_mode)
    };
    s.doc.body = merged;
    s.doc.ensure_nonempty();
    s.note_body_replaced();
    s.math = None;
    if show_in == "new" {
        s.path = None;
        s.file_name = None;
        s.clear_versions();
    }
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    Ok(merge3_result(conflicts, conflict_mode, &info))
}
