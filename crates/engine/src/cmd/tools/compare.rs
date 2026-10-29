use super::{
    Arc, Block, CharProps, CmdError, CmdResult, Document, InlineObject, Paragraph, RevisionKind, Selection, Session, StoryRef, Value, json, p,
    para_block,
};
use std::collections::BTreeSet;

/// Compare and Combine take the same settings.
pub(super) const COMPARE_PARAMS: &str = r#"{"path"?: string (revised document), "text"?: string (revised text), "original"?: string (original document; default: this one), "author"?: string (label changes with), "caseChanges"?: bool, "whiteSpace"?: bool, "formatting"?: bool (default true: differences count), "level"?: "word|character", "showIn"?: "original|revised|new" (revised/new: an untitled document), "moves"|"comments"|"tables"|"headersFooters"|"footnotes"|"textBoxes"|"fields"?: bool (accepted; not honoured yet except tables, always compared)} → {changes: n, author, notHonoured: [option], dropped: [{what: "textBoxStory|noteReference|commentAnchor|embeddedObject", count: n, why}] (what of the revised document could not be brought over)}"#;

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
/// headers and footers, footnotes, text boxes, fields) are accepted and listed in the result.
pub(super) fn compare(s: &mut Session, v: &Value) -> CmdResult {
    let revised = if let Some(t) = p::str(v, "text") {
        Document::from_text(t)
    } else if let Some(path) = p::str(v, "path") {
        crate::io::open_path(std::path::Path::new(path)).map_err(CmdError::Failed)?
    } else {
        return Err(CmdError::Params("`path` or `text` of the revised document is required".into()));
    };
    compare_with(s, revised, v)
}

/// [`compare`] with the revised document already open.
pub(super) fn compare_with(s: &mut Session, revised: Document, v: &Value) -> CmdResult {
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
    // The revised content, pointing at our copies of the pictures, lists, styles, notes and
    // comments it refers to (#1317).
    let mut dropped: Vec<Value> = Vec::new();
    // What the original holds before anything is brought over: copies made for content that turns
    // out unchanged are removed again, and only embedded objects not from the original are checked.
    let parts_before: BTreeSet<u32> = s.doc.parts.keys().copied().collect();
    let comments_before: BTreeSet<u32> = s.doc.comments.keys().copied().collect();
    let originals = embedded_ptrs(&s.doc);
    let revised_parts = revised.passthrough.clone();
    let new = import_revised(&mut s.doc, revised, &mut dropped);
    let old: Vec<Arc<Block>> = s.doc.body.clone();
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Insert, author: author.clone(), date: date.clone(), move_name: None });
    let ins = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(wordcraft_doc::Revision { kind: RevisionKind::Delete, author: author.clone(), date: date.clone(), move_name: None });
    let del = (s.doc.revisions.len() - 1) as u32;
    // Formatting changes are recorded here while the document is read for the comparison.
    let mut revisions = std::mem::take(&mut s.doc.revisions);
    // Comments of the revised document matched to the original's by one of their marks: the copy's
    // other marks are pointed at the original, so a comment whose range changed stays one.
    let mut merged: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    // Paragraphs match by their text and objects (by what they show, never by id); tables as a
    // whole, by their text and objects and, when formatting counts, their formatting.
    let doc = &s.doc;
    let key = |b: &Arc<Block>| match &**b {
        Block::Para(p) => para_key(doc, p, &opts),
        Block::Table(_) => block_key(doc, b, &opts, 0),
    };
    let mut budget = MAX_COMPARE_CELLS;
    let ops = lcs_ops(&old.iter().map(key).collect::<Vec<_>>(), &new.iter().map(key).collect::<Vec<_>>(), &mut budget);
    let mut out: Vec<(Arc<Block>, Fate)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut changes = 0usize;
    let mut k = 0;
    while k < ops.len() {
        match ops.get(k) {
            Some(Op::Same) => {
                let (a, b) = (old.get(i), new.get(j));
                // The revised block, with its formatting changes tracked.
                if let (Some(Block::Para(pa)), Some(Block::Para(pb))) = (a.map(|x| &**x), b.map(|x| &**x)) {
                    // The same objects (they match one for one): the original's, so its notes,
                    // comments and text boxes stay themselves rather than copies.
                    let mut base = pb.clone();
                    if base.objects.len() == pa.objects.len() {
                        pair_comments(&base.objects, &pa.objects, &mut merged);
                        base.objects = pa.objects.clone();
                    }
                    keep_sections(&mut base, Some(pa));
                    let mut p = base.clone();
                    if opts.formatting && pa.text == pb.text {
                        let before = revisions.len();
                        crate::cmd::fmt_revisions::compare_formatting(pa, &mut p, &mut revisions, &author, &date);
                        if revisions.len() > before || p != base {
                            changes += 1;
                        }
                    }
                    out.push((para_block(p), Fate::Kept));
                } else if let (Some(x), Some(y)) = (a, b) {
                    // A table the same as compared: the original's when its text is the same too
                    // (so its notes and text boxes stay themselves), else the revised one.
                    out.push((if block_text(x, 0) == block_text(y, 0) { x.clone() } else { y.clone() }, Fate::Kept));
                } else if let Some(x) = b.or(a) {
                    out.push((x.clone(), Fate::Kept));
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
                            let mut p = word_diff(doc, pa, pb, (ins, del), &opts, (&mut revisions, &author, &date, &mut merged), &mut budget);
                            keep_sections(&mut p, Some(pa));
                            out.push((para_block(p), Fate::Kept));
                        }
                        _ => {
                            if let Some(x) = a {
                                out.push((Arc::new(mark_whole(x, Fate::Deleted, del)), Fate::Deleted));
                            }
                            if let Some(x) = b {
                                let mut x = mark_whole(x, Fate::Inserted, ins);
                                if let Block::Para(p) = &mut x {
                                    keep_sections(p, None);
                                }
                                out.push((Arc::new(x), Fate::Inserted));
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
    s.doc.revisions = revisions;
    let mut body = mark_paragraph_marks(out, ins, del);
    if !merged.is_empty() {
        for b in &mut body {
            let untouched = matches!(&**b, Block::Para(p) if !p.objects.iter().any(|o| comment_mark(o).is_some_and(|id| merged.contains_key(&id))));
            if untouched {
                continue;
            }
            crate::cmd::paste::each_para(Arc::make_mut(b), 0, &mut |p| {
                for o in &mut p.objects {
                    if let InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } = o
                        && let Some(to) = merged.get(id)
                    {
                        *id = *to;
                    }
                }
            });
        }
    }
    s.doc.body = body;
    s.doc.ensure_nonempty();
    s.note_body_replaced();
    s.math = None;
    drop_unused_copies(&mut s.doc, &parts_before, &comments_before);
    let lost = drop_foreign_embedded(&mut s.doc, &originals, &revised_parts);
    if lost > 0 {
        dropped.push(json!({"what": "embeddedObject", "count": lost, "why": "a chart, diagram, ink or embedded object whose package parts can't be brought over (a part of the original has the same name) keeps only its picture"}));
    }
    if show_in != "original" || p::str(v, "original").is_some() {
        // The result is a new, untitled document: saving it never overwrites either file.
        s.path = None;
        s.file_name = None;
        // Version snapshots belong to the document they were taken from, including when
        // Compare creates an untitled result from the current document.
        s.clear_versions();
    }
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    Ok(json!({"changes": changes, "author": author, "notHonoured": not_honoured, "dropped": dropped}))
}

/// The comment a comment mark belongs to.
fn comment_mark(o: &InlineObject) -> Option<u32> {
    match o {
        InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => Some(*id),
        _ => None,
    }
}

/// Note which revised comments (copies) the original's comment marks matched, one for one.
fn pair_comments(revised: &[InlineObject], original: &[InlineObject], merged: &mut std::collections::BTreeMap<u32, u32>) {
    for (r, o) in revised.iter().zip(original) {
        if let (Some(r), Some(o)) = (comment_mark(r), comment_mark(o))
            && r != o
        {
            merged.insert(r, o);
        }
    }
}

/// What became of a block of the compared document: kept (perhaps changed inside), or inserted
/// or deleted whole.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fate {
    Kept,
    Inserted,
    Deleted,
}

/// A block inserted or deleted whole: every run marked (in tables, every cell's) and, for a
/// table, its rows, so resolving the change removes the table rather than emptying it.
fn mark_whole(b: &Block, fate: Fate, rid: u32) -> Block {
    let ins = fate == Fate::Inserted;
    let mut b = mark_block(b, &|c| if ins { c.ins = Some(rid) } else { c.del = Some(rid) }, 0);
    if let Block::Table(t) = &mut b {
        for r in &mut t.rows {
            if ins {
                r.props.ins = Some(rid);
            } else {
                r.props.del = Some(rid);
            }
        }
    }
    b
}

/// The compared blocks, with the paragraph marks of paragraphs inserted or deleted whole marked
/// too (ECMA-376 Part 1 §17.13.5.15, §17.13.5.17), so Accept All and Reject All remove those
/// paragraphs instead of leaving them empty. A container's last paragraph mark can't go, so for
/// whole paragraphs at the end the mark before each one carries its change instead.
fn mark_paragraph_marks(out: Vec<(Arc<Block>, Fate)>, ins: u32, del: u32) -> Vec<Arc<Block>> {
    let whole = |(b, f): &(Arc<Block>, Fate)| *f != Fate::Kept && matches!(**b, Block::Para(_));
    let tail = out.len() - out.iter().rev().take_while(|x| whole(x)).count();
    let fates: Vec<Fate> = out.iter().map(|(_, f)| *f).collect();
    let mut blocks: Vec<Arc<Block>> = out.into_iter().map(|(b, _)| b).collect();
    let set = |b: Option<&mut Arc<Block>>, f: Fate| {
        if let Some(b) = b
            && let Block::Para(p) = Arc::make_mut(b)
        {
            match f {
                Fate::Inserted => p.mark.ins = Some(ins),
                Fate::Deleted => p.mark.del = Some(del),
                Fate::Kept => {}
            }
        }
    };
    for (i, f) in fates.iter().enumerate() {
        if *f == Fate::Kept || !matches!(blocks.get(i).map(|b| &**b), Some(Block::Para(_))) {
            continue;
        }
        if i < tail {
            set(blocks.get_mut(i), *f);
        } else if let Some(prev) = i.checked_sub(1) {
            set(blocks.get_mut(prev), *f);
        }
    }
    blocks
}

/// A section break from the revised document keeps the original's headers and footers (the
/// revised document's aren't brought over: comparing them is not done), or, where the original
/// had none, links to the previous section's, as pasting does.
fn keep_sections(p: &mut Paragraph, original: Option<&Paragraph>) {
    let Some(sec) = p.section.as_deref_mut() else { return };
    match original.and_then(|o| o.section.as_deref()) {
        Some(o) => {
            sec.headers = o.headers;
            sec.footers = o.footers;
            sec.title_page = o.title_page;
        }
        None => {
            sec.headers = Default::default();
            sec.footers = Default::default();
            sec.title_page = false;
        }
    }
}

/// The revised document's body as content of `doc`: its pictures, lists, styles and revisions
/// brought over with ids that can't collide with `doc`'s (the importer paste uses), its text
/// boxes, notes and comments given stories of their own in `doc`. What can't be brought over
/// (a reference to a note or comment the revised document lacks) is left out and listed in
/// `dropped`.
fn import_revised(doc: &mut Document, revised: Document, dropped: &mut Vec<Value>) -> Vec<Arc<Block>> {
    use std::collections::BTreeMap;
    use wordcraft_doc::{Part, PartKind};
    let im = crate::cmd::paste::import_resources(doc, &revised, true);
    let mut blocks: Vec<Block> = revised.body.iter().map(|b| (**b).clone()).collect();
    for b in &mut blocks {
        im.remap_block(b);
    }
    // Text boxes get stories of their own (their content pointed at our copies too); a box
    // whose story the revised document lacks shows none rather than one of ours.
    let mut boxes: BTreeMap<u32, Part> = revised.parts.iter().filter(|(_, p)| p.kind == PartKind::TextBox).map(|(k, p)| (*k, p.clone())).collect();
    for part in boxes.values_mut() {
        for b in &mut part.blocks {
            im.remap_block(Arc::make_mut(b));
        }
    }
    let mut lost_boxes = 0usize;
    for b in &mut blocks {
        crate::cmd::paste::each_para(b, 0, &mut |p| {
            for o in &mut p.objects {
                for story in o.text_box_slots() {
                    if story.is_some_and(|id| !boxes.contains_key(&id)) {
                        *story = None;
                        lost_boxes += 1;
                    }
                }
            }
        });
    }
    // Note references and comment marks, in the body and in the text boxes, point at our copies.
    let mut refs =
        Refs { doc: &mut *doc, src: &revised, im: &im, notes: BTreeMap::new(), comments: BTreeMap::new(), lost_notes: 0, lost_comments: 0 };
    for b in &mut blocks {
        refs.remap(b, 0);
    }
    for part in boxes.values_mut() {
        for b in &mut part.blocks {
            refs.remap(Arc::make_mut(b), 0);
        }
    }
    let (lost_notes, lost_comments) = (refs.lost_notes, refs.lost_comments);
    doc.adopt_text_box_stories(&mut blocks, &boxes);
    for (what, count, why) in [
        ("textBoxStory", lost_boxes, "a text box whose text the revised document lacks shows none"),
        ("noteReference", lost_notes, "a note reference whose note the revised document lacks"),
        ("commentAnchor", lost_comments, "a comment mark whose comment the revised document lacks"),
    ] {
        if count > 0 {
            dropped.push(json!({"what": what, "count": count, "why": why}));
        }
    }
    blocks.into_iter().map(Arc::new).collect()
}

/// Footnotes, endnotes and comments of the revised document copied into ours as they are
/// referenced (each once), by their ids there.
struct Refs<'a> {
    doc: &'a mut Document,
    src: &'a Document,
    im: &'a crate::cmd::paste::Imported,
    notes: std::collections::BTreeMap<u32, Option<u32>>,
    comments: std::collections::BTreeMap<u32, Option<u32>>,
    /// Note references and comment marks left out: their story couldn't be brought over.
    lost_notes: usize,
    lost_comments: usize,
}

/// How deep notes and comments whose text refers to more notes and comments are followed.
const MAX_REF_DEPTH: usize = 8;

impl Refs<'_> {
    /// Point the note references and comment marks in `b` at our copies of their stories; one
    /// whose story can't be brought over (missing, a cycle, or deeper than [`MAX_REF_DEPTH`]) is
    /// deleted and counted.
    fn remap(&mut self, b: &mut Block, depth: usize) {
        crate::cmd::paste::each_para(b, 0, &mut |p| {
            let mut gone = Vec::new();
            for (k, o) in p.objects.iter_mut().enumerate() {
                // Whether the reference is lost: `Some(true)` a note's, `Some(false)` a comment's.
                let lost = match o {
                    InlineObject::NoteRef { id, .. } => self.note(*id, depth).map(|n| *id = n).is_none().then_some(true),
                    InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => {
                        self.comment(*id, depth).map(|n| *id = n).is_none().then_some(false)
                    }
                    _ => None,
                };
                match lost {
                    Some(true) => self.lost_notes += 1,
                    Some(false) => self.lost_comments += 1,
                    None => continue,
                }
                gone.push(k);
            }
            if !gone.is_empty() {
                let offsets = p.object_offsets();
                for k in gone.iter().rev() {
                    if let Some(off) = offsets.get(*k) {
                        let _ = p.delete(*off, off + wordcraft_doc::para::OBJ.len_utf8());
                    }
                }
            }
        });
    }

    /// A copy of the source's story `id` when it is one of `kinds`; its content points at our
    /// copies of what it refers to.
    fn part(&mut self, id: u32, kinds: &[wordcraft_doc::PartKind], depth: usize) -> Option<u32> {
        let part = self.src.parts.get(&id).filter(|p| kinds.contains(&p.kind))?;
        let mut blocks = part.blocks.clone();
        for b in &mut blocks {
            let b = Arc::make_mut(b);
            self.im.remap_block(b);
            self.remap(b, depth.saturating_add(1));
        }
        self.doc.try_add_part(part.kind, blocks).ok()
    }

    /// Our id for the source's footnote or endnote story `id`.
    fn note(&mut self, id: u32, depth: usize) -> Option<u32> {
        use wordcraft_doc::PartKind;
        if let Some(n) = self.notes.get(&id) {
            return *n;
        }
        if depth > MAX_REF_DEPTH {
            return None;
        }
        // Lost while it is being copied, so a note that refers back to itself ends.
        self.notes.insert(id, None);
        let n = self.part(id, &[PartKind::Footnote, PartKind::Endnote], depth);
        self.notes.insert(id, n);
        n
    }

    /// Our id for the source's comment `id` (its text copied, a reply kept a reply when its
    /// parent came first).
    fn comment(&mut self, id: u32, depth: usize) -> Option<u32> {
        if let Some(n) = self.comments.get(&id) {
            return *n;
        }
        if depth > MAX_REF_DEPTH {
            return None;
        }
        // Lost while it is being copied, so a comment that refers back to itself ends.
        self.comments.insert(id, None);
        let n = self.src.comments.get(&id).cloned().and_then(|c| {
            let part = self.part(c.part, &[wordcraft_doc::PartKind::Comment], depth)?;
            let new = match self.doc.comments.keys().next_back() {
                None => Some(0),
                Some(k) => k.checked_add(1),
            };
            // At the top of the range: the first free id (there is one below the number of comments).
            let new = new.or_else(|| (0..=u32::MAX).find(|i| !self.doc.comments.contains_key(i)))?;
            let parent = c.parent.and_then(|p| self.comments.get(&p).copied().flatten());
            self.doc.comments.insert(new, wordcraft_doc::Comment { part, parent, durable_id: None, ..c });
            Some(new)
        });
        self.comments.insert(id, n);
        n
    }
}

/// A paragraph's text as compared: objects stand for what they show (a different picture or
/// field is a change even where the text around it is the same).
fn para_key(doc: &Document, p: &Paragraph, opts: &CompareOptions) -> String {
    let mut out = String::with_capacity(p.text.len());
    let mut k = 0;
    for c in p.text.chars() {
        if c == wordcraft_doc::para::OBJ {
            out.push_str(&object_key(doc, p.objects.get(k)));
            k += 1;
        } else {
            out.push(c);
        }
    }
    opts.key(&out)
}

/// How an object compares: by what it shows, never by the ids that point at its parts (a
/// footnote by its text, a comment mark by its comment's author and text, a text box by its
/// text), and without the package parts an embedded object was read with.
fn object_key(doc: &Document, o: Option<&InlineObject>) -> String {
    let story = |id: u32| doc.plain_text(StoryRef::Part(id));
    let comment = |id: &u32| doc.comments.get(id).map(|c| format!("{}\u{1}{}", c.author, story(c.part))).unwrap_or_default();
    let body = match o {
        None => String::new(),
        Some(InlineObject::NoteRef { kind, id, custom }) => format!("note\u{1}{kind:?}\u{1}{custom}\u{1}{}", story(*id)),
        Some(InlineObject::CommentStart { id }) => format!("commentStart\u{1}{}", comment(id)),
        Some(InlineObject::CommentEnd { id }) => format!("commentEnd\u{1}{}", comment(id)),
        Some(o) => {
            let mut x = o.clone();
            let boxes: Vec<String> = x.text_boxes().into_iter().map(story).collect();
            for slot in x.text_box_slots() {
                *slot = None;
            }
            strip_embedded(&mut x, 0);
            format!("{}\u{1}{}", serde_json::to_string(&x).unwrap_or_default(), boxes.join("\u{1}"))
        }
    };
    format!("\u{FFFC}{body}\u{FFFC}")
}

/// `o` without the package parts it was read with (an OLE object's, a chart's, diagram's or ink's).
fn strip_embedded(o: &mut InlineObject, depth: usize) {
    match o {
        InlineObject::Image { ole, .. } => *ole = None,
        InlineObject::Graphic { graphic, .. } if graphic.source.is_some() => Arc::make_mut(graphic).source = None,
        InlineObject::Group { children, .. } if depth < MAX_KEY_DEPTH => {
            for c in children {
                strip_embedded(&mut c.obj, depth + 1);
            }
        }
        _ => {}
    }
}

/// Deepest group or table nesting a comparison key follows.
const MAX_KEY_DEPTH: usize = 16;

/// A block as compared inside a table: its text and objects and, when formatting counts, its
/// paragraph, run, cell, row and table formatting (a table matched as unchanged is not diffed
/// further, so every difference has to show here).
fn block_key(doc: &Document, b: &Block, opts: &CompareOptions, depth: usize) -> String {
    let json = |v: Result<String, serde_json::Error>| if opts.formatting { v.unwrap_or_default() } else { String::new() };
    match b {
        Block::Para(p) => {
            let runs: Vec<(usize, CharProps)> = p.runs.iter().map(|r| (r.len, r.props.formatting())).collect();
            format!(
                "{}\u{2}{}\u{2}{}",
                para_key(doc, p, opts),
                json(serde_json::to_string(&p.props.formatting())),
                json(serde_json::to_string(&runs))
            )
        }
        Block::Table(t) if depth < MAX_KEY_DEPTH => {
            let mut out =
                format!("\u{0}table\u{0}{}", json(serde_json::to_string(&wordcraft_doc::props::TableProps { fmt_change: None, ..t.props.clone() })));
            for r in &t.rows {
                let props = wordcraft_doc::props::RowProps { fmt_change: None, ins: None, del: None, ..r.props.clone() };
                out.push_str(&format!("\u{3}{}", json(serde_json::to_string(&props))));
                for c in &r.cells {
                    out.push_str(&format!(
                        "\u{4}{}",
                        json(serde_json::to_string(&wordcraft_doc::props::CellProps { fmt_change: None, ..c.props.clone() }))
                    ));
                    for x in &c.blocks {
                        out.push_str(&format!("\u{5}{}", block_key(doc, x, opts, depth + 1)));
                    }
                }
            }
            out
        }
        Block::Table(_) => String::new(),
    }
}

/// The embedded objects (their kept package parts) a document's stories hold, by address: the
/// original's own, which never need checking.
fn embedded_ptrs(doc: &Document) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    each_object(doc, &mut |o| {
        for e in embedded_of(o, 0) {
            out.insert(Arc::as_ptr(e) as usize);
        }
    });
    out
}

/// Every object in every story of `doc` (tables included, bounded depth).
fn each_object(doc: &Document, f: &mut dyn FnMut(&InlineObject)) {
    fn walk(blocks: &[Arc<Block>], depth: usize, f: &mut dyn FnMut(&InlineObject)) {
        for b in blocks {
            match &**b {
                Block::Para(p) => p.objects.iter().for_each(&mut *f),
                Block::Table(t) if depth < MAX_KEY_DEPTH => {
                    for c in t.rows.iter().flat_map(|r| r.cells.iter()) {
                        walk(&c.blocks, depth + 1, f);
                    }
                }
                Block::Table(_) => {}
            }
        }
    }
    walk(&doc.body, 0, f);
    for part in doc.parts.values() {
        walk(&part.blocks, 0, f);
    }
}

/// The kept package parts of an object and its group members.
fn embedded_of(o: &InlineObject, depth: usize) -> Vec<&Arc<wordcraft_doc::graphic::Embedded>> {
    match o {
        InlineObject::Image { ole: Some(e), .. } => vec![e],
        InlineObject::Graphic { graphic, .. } => graphic.source.iter().collect(),
        InlineObject::Group { children, .. } if depth < MAX_KEY_DEPTH => children.iter().flat_map(|c| embedded_of(&c.obj, depth + 1)).collect(),
        _ => Vec::new(),
    }
}

/// Embedded objects brought from the revised document keep their package parts only when the
/// original holds the very same parts under those names (the parts aren't copied: their names
/// would collide); otherwise the object keeps just its picture. Returns how many lost theirs.
fn drop_foreign_embedded(doc: &mut Document, originals: &BTreeSet<usize>, revised: &std::collections::BTreeMap<String, Arc<Vec<u8>>>) -> usize {
    let have = doc.passthrough.clone();
    let ok = |e: &Arc<wordcraft_doc::graphic::Embedded>| {
        originals.contains(&(Arc::as_ptr(e) as usize))
            || e.refs.iter().filter(|r| !r.external).all(|r| matches!((have.get(&r.target), revised.get(&r.target)), (Some(x), Some(y)) if x == y))
    };
    fn fix(o: &mut InlineObject, ok: &dyn Fn(&Arc<wordcraft_doc::graphic::Embedded>) -> bool, depth: usize) -> usize {
        match o {
            InlineObject::Image { ole, .. } if ole.as_ref().is_some_and(|e| !ok(e)) => {
                *ole = None;
                1
            }
            InlineObject::Graphic { graphic, .. } if graphic.source.as_ref().is_some_and(|e| !ok(e)) => {
                Arc::make_mut(graphic).source = None;
                1
            }
            InlineObject::Group { children, .. } if depth < MAX_KEY_DEPTH => children.iter_mut().map(|c| fix(&mut c.obj, ok, depth + 1)).sum(),
            _ => 0,
        }
    }
    fn bad(b: &Block, ok: &dyn Fn(&Arc<wordcraft_doc::graphic::Embedded>) -> bool, depth: usize) -> bool {
        match b {
            Block::Para(p) => p.objects.iter().any(|o| embedded_of(o, 0).into_iter().any(|e| !ok(e))),
            Block::Table(t) if depth < MAX_KEY_DEPTH => {
                t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).any(|x| bad(x, ok, depth + 1))
            }
            Block::Table(_) => false,
        }
    }
    let mut lost = 0;
    let mut fix_blocks = |blocks: &mut Vec<Arc<Block>>| {
        for b in blocks.iter_mut() {
            if bad(b, &ok, 0) {
                crate::cmd::paste::each_para(Arc::make_mut(b), 0, &mut |p| {
                    for o in &mut p.objects {
                        lost += fix(o, &ok, 0);
                    }
                });
            }
        }
    };
    fix_blocks(&mut doc.body);
    for part in doc.parts.values_mut() {
        fix_blocks(&mut part.blocks);
    }
    lost
}

/// Remove the notes, comments and text box stories copied from the revised document that
/// nothing refers to any more (the content turned out unchanged and kept the original's).
fn drop_unused_copies(doc: &mut Document, parts_before: &BTreeSet<u32>, comments_before: &BTreeSet<u32>) {
    // Each pass can free stories only another unused copy referred to; bounded by their number.
    for _ in 0..=doc.parts.len() {
        let (mut parts, mut comments) = (BTreeSet::new(), BTreeSet::new());
        each_object(doc, &mut |o| match o {
            InlineObject::NoteRef { id, .. } => {
                parts.insert(*id);
            }
            InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => {
                comments.insert(*id);
            }
            o => parts.extend(o.text_boxes()),
        });
        let unused: Vec<u32> = doc.comments.keys().copied().filter(|k| !comments_before.contains(k) && !comments.contains(k)).collect();
        for k in unused {
            doc.comments.remove(&k);
        }
        parts.extend(doc.comments.values().map(|c| c.part));
        let orphans: Vec<u32> = doc.parts.keys().copied().filter(|k| !parts_before.contains(k) && !parts.contains(k)).collect();
        if orphans.is_empty() {
            return;
        }
        for k in orphans {
            doc.parts.remove(&k);
        }
    }
}

/// A block's text (a table's cells in order), for matching blocks.
pub(super) fn block_text(b: &Block, depth: usize) -> String {
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

/// A paragraph's tokens as byte ranges: words with the spaces after them ("The ", "cat "), or
/// single characters; an object is always a token of its own.
fn tokens(p: &Paragraph, characters: bool) -> Vec<std::ops::Range<usize>> {
    let mut v = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in p.text.char_indices() {
        let end = i + c.len_utf8();
        if c == wordcraft_doc::para::OBJ || characters {
            if let Some(s0) = start.take() {
                v.push(s0..i);
            }
            v.push(i..end);
            continue;
        }
        let s0 = *start.get_or_insert(i);
        if c == ' ' {
            v.push(s0..end);
            start = None;
        }
    }
    if let Some(s0) = start {
        v.push(s0..p.text.len());
    }
    v
}

/// How a token compares: its text as the options see it, or the object it stands for.
fn token_key(doc: &Document, p: &Paragraph, r: &std::ops::Range<usize>, opts: &CompareOptions) -> String {
    let t = p.text.get(r.clone()).unwrap_or("");
    if t.starts_with(wordcraft_doc::para::OBJ) { object_key(doc, p.object_at(r.start)) } else { opts.key(t) }
}

/// The byte range covered by `n` tokens from the `i`th.
fn token_span(t: &[std::ops::Range<usize>], i: usize, n: usize) -> std::ops::Range<usize> {
    let a = t.get(i).map_or(0, |r| r.start);
    let b = n.checked_sub(1).and_then(|l| t.get(i + l)).map_or(a, |r| r.end);
    a..b.max(a)
}

/// The content of `p` in the byte range `r` (text, runs and objects) as a paragraph of its own,
/// without `p`'s paragraph properties.
fn slice(p: &Paragraph, r: std::ops::Range<usize>) -> Paragraph {
    let mut q = Paragraph::new();
    let Some(text) = p.text.get(r.clone()) else { return q };
    let obj = wordcraft_doc::para::OBJ;
    let before = p.text.get(..r.start).map_or(0, |t| t.matches(obj).count());
    q.objects = p.objects.iter().skip(before).take(text.matches(obj).count()).cloned().collect();
    q.runs = p
        .run_ranges()
        .filter_map(|(x, c)| {
            let (a, b) = (x.start.max(r.start), x.end.min(r.end));
            (b > a).then(|| wordcraft_doc::Run { len: b - a, props: c.clone() })
        })
        .collect();
    q.text = text.to_string();
    q.normalize();
    q
}

/// A changed paragraph: the revised paragraph's content where it matches the original (its
/// formatting changes tracked), the original's where it was deleted, the revised one's where
/// inserted, each piece keeping its runs, objects (pictures, fields…) and links. `revs` are the
/// insertion and deletion revisions; `log` where formatting changes are recorded, and the revised
/// comments whose marks matched the original's.
fn word_diff(
    doc: &Document,
    a: &Paragraph,
    b: &Paragraph,
    revs: (u32, u32),
    opts: &CompareOptions,
    log: (&mut Vec<wordcraft_doc::Revision>, &str, &str, &mut std::collections::BTreeMap<u32, u32>),
    budget: &mut u64,
) -> Paragraph {
    let (ins, del) = revs;
    let (revisions, author, date, merged) = log;
    let chars = opts.characters && a.text.chars().count().max(b.text.chars().count()) <= MAX_CHAR_DIFF;
    let (ta, tb) = (tokens(a, chars), tokens(b, chars));
    let key = |p: &Paragraph, t: &[std::ops::Range<usize>]| t.iter().map(|r| token_key(doc, p, r, opts)).collect::<Vec<_>>();
    let ops = lcs_ops(&key(a, &ta), &key(b, &tb), budget);
    // Only at character granularity: a matched whole word almost never happens by coincidence,
    // but a matched single character very often does (digits, punctuation), so only there does
    // a short interior match need folding back into its surrounding edit.
    let ops = if chars { collapse_noise_matches(&ops) } else { ops };
    let mut p = Paragraph::new();
    p.props = b.props.clone();
    p.mark = b.mark.clone();
    p.section = b.section.clone();
    if opts.formatting {
        // The paragraph's own formatting (and its mark's).
        let mut old = Paragraph::new();
        old.props = a.props.clone();
        old.mark = a.mark.clone();
        crate::cmd::fmt_revisions::compare_formatting(&old, &mut p, revisions, author, date);
    }
    let (mut i, mut j, mut k) = (0, 0, 0);
    while let Some(op) = ops.get(k).copied() {
        let n = ops.get(k..).unwrap_or_default().iter().take_while(|o| **o == op).count().max(1);
        let piece = match op {
            Op::Same => {
                // Equal as compared (perhaps up to case or spacing): the revised content.
                let mut piece = slice(b, token_span(&tb, j, n));
                let old = slice(a, token_span(&ta, i, n));
                // The same objects: the original's (its notes and comments stay themselves).
                if piece.objects.len() == old.objects.len() {
                    pair_comments(&piece.objects, &old.objects, merged);
                    piece.objects.clone_from(&old.objects);
                }
                if opts.formatting && old.text == piece.text {
                    crate::cmd::fmt_revisions::compare_formatting(&old, &mut piece, revisions, author, date);
                }
                i += n;
                j += n;
                piece
            }
            Op::Del => {
                let mut piece = slice(a, token_span(&ta, i, n));
                let len = piece.len();
                let _ = piece.format(0, len, &|c| c.del = Some(del));
                i += n;
                piece
            }
            Op::Ins => {
                let mut piece = slice(b, token_span(&tb, j, n));
                let len = piece.len();
                let _ = piece.format(0, len, &|c| c.ins = Some(ins));
                j += n;
                piece
            }
        };
        p.append(piece);
        k += n;
    }
    p
}
