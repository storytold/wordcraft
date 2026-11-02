use super::{Block, CmdError, CmdResult, Document, InlineObject, Paragraph, Pos, Session, StoryRef, Value, json};

pub(super) fn accessibility(s: &mut Session, _: &Value) -> CmdResult {
    let issues = crate::cmd::accessibility::check(&s.doc);
    Ok(json!({"ok": issues.is_empty(), "issues": issues}))
}

pub(super) fn inspect_doc(s: &mut Session, v: &Value) -> CmdResult {
    let remove: Vec<String> =
        v.get("remove").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    // The removals aren't tracked: they delete straight from the document, so on a document that
    // tracks changes the text would go with no revision to show for it, and the revision trail
    // would be broken behind the user's back. Refuse the whole removal instead of quietly
    // untracking it. Reading the document (no `remove`) is always allowed.
    if !remove.is_empty() && s.doc.settings.track_changes {
        return Err(CmdError::Disabled("the document tracks changes: stop tracking before removing content".into()));
    }
    let hidden = hidden_text(&s.doc);
    let report = json!({
        "comments": s.doc.comments.len(),
        "revisions": s.doc.para_paths(StoryRef::Body).iter().filter(|p| s.doc.para(StoryRef::Body, p).is_some_and(|x| x.runs.iter().any(|r| r.props.ins.is_some() || r.props.del.is_some()))).count(),
        "properties": !s.doc.core.creator.is_empty() || !s.doc.core.last_modified_by.is_empty(),
        "hiddenText": !hidden.is_empty(),
        "hiddenCount": hidden.len(),
        "hidden": hidden.iter().take(MAX_HIDDEN_LISTED).map(|h| h.report(&s.doc)).collect::<Vec<_>>(),
        "truncated": hidden.len() > MAX_HIDDEN_LISTED,
        "unsupported": unsupported_hidden(&s.doc),
        "headers": s.doc.parts.values().filter(|p| matches!(p.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer)).count(),
    });
    let mut removed_notes = Vec::new();
    for r in &remove {
        match r.as_str() {
            "comments" => {
                s.run("review.deleteComment", &json!({"all": true}))?;
            }
            "revisions" => {
                s.run("review.acceptAll", &json!({}))?;
            }
            "properties" => {
                let title = s.doc.core.title.clone();
                s.doc.core = Default::default();
                s.doc.core.title = title;
            }
            "hidden" => removed_notes = remove_hidden(&mut s.doc)?,
            "headers" => {
                s.run("insert.removeHeader", &json!({}))?;
                s.run("insert.removeFooter", &json!({}))?;
            }
            _ => {}
        }
    }
    s.clamp_selection();
    // What a removal couldn't reach: hidden text still there, or kept in objects we don't model.
    let residual: Vec<Value> = if remove.iter().any(|r| r == "hidden") {
        let mut v: Vec<Value> = hidden_text(&s.doc).iter().take(MAX_HIDDEN_LISTED).map(|h| h.report(&s.doc)).collect();
        v.extend(unsupported_hidden(&s.doc));
        v
    } else {
        Vec::new()
    };
    Ok(json!({"found": report, "removed": remove, "residual": residual, "removedNotes": removed_notes}))
}

/// Most hidden pieces (and removed notes) `file.inspect` lists (the count is always exact).
const MAX_HIDDEN_LISTED: usize = 1000;

/// Most characters of a removed note's text `file.inspect` echoes.
const MAX_NOTE_TEXT: usize = 1000;

/// A piece of hidden text: a byte range of a paragraph in a story. `whole`: the paragraph's mark
/// is hidden too and so is all its text, so the paragraph itself goes.
struct Hidden {
    story: StoryRef,
    path: wordcraft_doc::Path,
    range: std::ops::Range<usize>,
    whole: bool,
}

/// Remove All for hidden text (#1344): every hidden piece, last first so earlier offsets and
/// paths stay valid; whole hidden paragraphs with their marks; and the notes whose references
/// were hidden, as deleting a reference deletes its note. Returns the notes removed.
fn remove_hidden(doc: &mut Document) -> Result<Vec<Value>, CmdError> {
    let mut refs: Vec<u32> = Vec::new();
    for h in hidden_text(doc).into_iter().rev() {
        if let Some(p) = doc.para(h.story, &h.path) {
            for off in p.object_offsets() {
                if h.range.contains(&off)
                    && let Some(InlineObject::NoteRef { id, .. }) = p.object_at(off)
                {
                    refs.push(*id);
                }
            }
        }
        remove_piece(doc, &h)?;
    }
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    // Notes still referenced from elsewhere (a note's own mark doesn't count) stay.
    let mut kept = std::collections::BTreeSet::new();
    for story in all_stories(doc) {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            for off in p.object_offsets() {
                if let Some(InlineObject::NoteRef { id, .. }) = p.object_at(off)
                    && story != StoryRef::Part(*id)
                {
                    kept.insert(*id);
                }
            }
        }
    }
    refs.sort_unstable();
    refs.dedup();
    let mut out = Vec::new();
    for id in refs {
        if kept.contains(&id)
            || !doc.parts.get(&id).is_some_and(|p| matches!(p.kind, wordcraft_doc::PartKind::Footnote | wordcraft_doc::PartKind::Endnote))
        {
            continue;
        }
        // Only the echo is capped; every such note goes.
        if out.len() < MAX_HIDDEN_LISTED {
            let text: String = doc.plain_text(StoryRef::Part(id)).replace('\u{FFFC}', "").trim().chars().take(MAX_NOTE_TEXT).collect();
            out.push(json!({"kind": story_kind(doc, StoryRef::Part(id)), "part": id, "text": text}));
        }
        doc.parts.remove(&id);
    }
    Ok(out)
}

/// Whether a whole hidden paragraph can go, mark and all. It stays when it is the last block of
/// its story or cell (they end with a paragraph), carries a section break, holds content control
/// markers (their partners may be elsewhere), or sits between two tables (they'd join on reopen).
fn removable(doc: &Document, story: StoryRef, path: &wordcraft_doc::Path, p: &Paragraph) -> bool {
    let i = path.last();
    let Some(c) = doc.container(story, path) else { return false };
    let is_table = |j: Option<usize>| j.and_then(|j| c.get(j)).is_some_and(|b| matches!(**b, Block::Table(_)));
    i + 1 < c.len()
        && p.section.is_none()
        && !p.object_offsets().into_iter().any(|o| matches!(p.object_at(o), Some(InlineObject::ControlStart { .. } | InlineObject::ControlEnd)))
        && !(is_table(i.checked_sub(1)) && is_table(i.checked_add(1)))
}

/// Whether `range` of `p` holds text or an object, not only zero-width markers.
fn has_content(p: &Paragraph, range: &std::ops::Range<usize>) -> bool {
    p.text
        .get(range.clone())
        .is_some_and(|t| t.char_indices().any(|(i, ch)| ch != '\u{FFFC}' || p.object_at(range.start + i).is_some_and(|o| !o.is_marker())))
}

/// Delete one hidden piece. Within a paragraph: straight in the paragraph, unless a content
/// control marker is inside (then through `delete_range`, which keeps markers whose partner is
/// outside). A whole hidden paragraph goes with its mark when [`removable`]; otherwise its text
/// goes and it stays, empty, its mark still hidden: invisible and holding no text, so it isn't
/// reported again.
fn remove_piece(doc: &mut Document, h: &Hidden) -> Result<(), CmdError> {
    let Some(p) = doc.para(h.story, &h.path) else { return Ok(()) };
    let has_control = p
        .object_offsets()
        .into_iter()
        .any(|o| h.range.contains(&o) && matches!(p.object_at(o), Some(InlineObject::ControlStart { .. } | InlineObject::ControlEnd)));
    if h.whole && removable(doc, h.story, &h.path, p) {
        let i = h.path.last();
        let blocks = doc.container_mut(h.story, &h.path)?;
        if i < blocks.len() {
            blocks.remove(i);
        }
        return Ok(());
    }
    if !h.range.is_empty() {
        if has_control {
            let at = |off| Pos { story: h.story, path: h.path.clone(), off };
            doc.delete_range(&at(h.range.start), &at(h.range.end))?;
        } else {
            doc.para_mut(h.story, &h.path)?.delete(h.range.start, h.range.end)?;
        }
    }
    Ok(())
}

impl Hidden {
    fn report(&self, doc: &Document) -> Value {
        let text = doc.para(self.story, &self.path).and_then(|p| p.text.get(self.range.clone())).unwrap_or("");
        let mut v = json!({"story": story_kind(doc, self.story), "text": text});
        if let (StoryRef::Part(id), Value::Object(m)) = (self.story, &mut v) {
            m.insert("part".into(), json!(id));
        }
        v
    }
}

/// Every story of the document: the body, then headers, footers, notes, comments and text boxes.
fn all_stories(doc: &Document) -> Vec<StoryRef> {
    std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|k| StoryRef::Part(*k))).collect()
}

/// What the inspector calls a story.
fn story_kind(doc: &Document, st: StoryRef) -> &'static str {
    use wordcraft_doc::PartKind;
    match st {
        StoryRef::Body => "body",
        StoryRef::Part(id) => match doc.parts.get(&id).map(|p| p.kind) {
            Some(PartKind::Header) => "header",
            Some(PartKind::Footer) => "footer",
            Some(PartKind::Footnote) => "footnote",
            Some(PartKind::Endnote) => "endnote",
            Some(PartKind::Comment) => "comment",
            Some(PartKind::TextBox) => "textBox",
            None => "unknown",
        },
    }
}

/// The hidden text of the document (Inspect Document, #1344): runs whose effective formatting
/// (document defaults, paragraph style chain, character style chain, direct) is hidden, in every
/// story, adjacent runs merged. Ranges holding only zero-width markers (bookmarks, comment
/// ranges, content control and field markers) aren't text and are left out.
fn hidden_text(doc: &Document) -> Vec<Hidden> {
    let mut out = Vec::new();
    for story in all_stories(doc) {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            let style = p.props.style.as_deref();
            let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
            let mut all_hidden = true;
            for (r, c) in p.run_ranges() {
                if r.is_empty() {
                    continue;
                }
                if !doc.styles.resolve_char(style, c).hidden {
                    all_hidden = false;
                    continue;
                }
                match ranges.last_mut() {
                    Some(last) if last.end == r.start => last.end = r.end,
                    _ => ranges.push(r),
                }
            }
            if all_hidden && doc.styles.resolve_char(style, &p.mark).hidden {
                // An empty hidden paragraph that has to stay is invisible and holds no text.
                if removable(doc, story, &path, p) || has_content(p, &(0..p.len())) {
                    out.push(Hidden { story, path: path.clone(), range: 0..p.len(), whole: true });
                }
                continue;
            }
            for range in ranges {
                if has_content(p, &range) {
                    out.push(Hidden { story, path: path.clone(), range, whole: false });
                }
            }
        }
    }
    out
}

/// Hidden text the inspector can't reach: objects kept verbatim from the source file
/// (`Opaque`) whose markup hides text (`w:vanish`), outside hidden runs (those go with the run).
fn unsupported_hidden(doc: &Document) -> Vec<Value> {
    let mut out = Vec::new();
    for story in all_stories(doc) {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            let style = p.props.style.as_deref();
            for off in p.object_offsets() {
                if let Some(InlineObject::Opaque { format, xml, .. }) = p.object_at(off)
                    && xml.contains("vanish")
                    && !doc.styles.resolve_char(style, p.props_of_char(off)).hidden
                {
                    out.push(json!({"story": story_kind(doc, story), "object": "opaque", "format": format, "reason": "hidden text inside an object kept from the source file"}));
                }
            }
        }
        if let Some(blocks) = doc.story(story) {
            tables_hiding_text(doc, story, blocks, 0, &mut out);
        }
    }
    out
}

/// Tables whose table style (or one of its conditional parts: header row, bands, first or last
/// column, total row) hides text. The inspector doesn't resolve table styles, so it lists them.
fn tables_hiding_text(doc: &Document, story: StoryRef, blocks: &wordcraft_doc::Blocks, depth: usize, out: &mut Vec<Value>) {
    if depth > 32 {
        return;
    }
    for b in blocks {
        let Block::Table(t) = &**b else { continue };
        if let Some(id) = t.props.style.as_deref()
            && let Some(ts) = doc.styles.table_style(id)
        {
            let q = &ts.parts;
            if [&ts.chr, &q.header_chr, &q.band_chr, &q.col_band_chr, &q.first_col_chr, &q.last_col_chr, &q.total_chr]
                .iter()
                .any(|c| c.hidden == Some(true))
            {
                out.push(json!({"story": story_kind(doc, story), "object": "table", "style": id, "reason": "text hidden by a table style isn't inspected or removed"}));
            }
        }
        for row in &t.rows {
            for cell in &row.cells {
                tables_hiding_text(doc, story, &cell.blocks, depth + 1, out);
            }
        }
    }
}
