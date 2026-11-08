//! Notes, comments and fields (ODF 1.3 §6.3, §14.1, §7): read as containers of their own while the
//! paragraph they are in is put aside.

use super::{Attrs, Body, FBlock, FComment, Fmt, Frame, Inline, MAX_COMMENTS, MAX_INFO_CHARS, Para, get, length, model};

/// Make every comment range in `blocks` close: an end before its comment (or of no comment) is
/// dropped, and a comment whose end never comes covers nothing. Replies have no range.
pub(in crate::odt) fn balance_comments(blocks: &mut [FBlock]) {
    fn each_para(blocks: &mut [FBlock], depth: usize, f: &mut dyn FnMut(&mut Para)) {
        for b in blocks {
            match b {
                FBlock::Para(p) => f(p),
                FBlock::Table(t) if depth < model::MAX_DEPTH => {
                    for c in t.rows.iter_mut().flatten() {
                        each_para(&mut c.blocks, depth + 1, f);
                    }
                }
                FBlock::Table(_) => {}
            }
        }
    }
    let mut open = std::collections::HashSet::new();
    each_para(blocks, 0, &mut |p| {
        p.inlines.retain(|i| match i {
            Inline::Comment { id, comment } if comment.parent.is_none() => {
                open.insert(*id);
                true
            }
            Inline::CommentEnd(id) => open.remove(id),
            _ => true,
        })
    });
    if open.is_empty() {
        return;
    }
    each_para(blocks, 0, &mut |p| {
        let mut k = 0;
        while k < p.inlines.len() {
            if let Some(Inline::Comment { id, comment }) = p.inlines.get(k)
                && comment.parent.is_none()
                && open.remove(id)
            {
                let id = *id;
                p.inlines.insert(k + 1, Inline::CommentEnd(id));
                k += 1;
            }
            k += 1;
        }
    });
}

/// A comment being read (ODF 1.3 §14.1): what `office:annotation` and its `dc:creator`,
/// `dc:date` and `meta:creator-initials` say.
#[derive(Default)]
pub(in crate::odt) struct CommentInfo {
    /// Its flow id.
    pub(in crate::odt) id: u32,
    /// Its `office:name`, which its `office:annotation-end` names.
    pub(in crate::odt) named: bool,
    /// The flow id of the comment it answers (`loext:parent-name`).
    pub(in crate::odt) parent: Option<u32>,
    pub(in crate::odt) resolved: bool,
    pub(in crate::odt) author: String,
    pub(in crate::odt) date: String,
    pub(in crate::odt) initials: String,
    /// The child whose text is being read: 0 author, 1 date, 2 initials.
    pub(in crate::odt) field: Option<u8>,
}

/// A footnote, endnote or comment being read (ODF 1.3 §6.3, §14.1): what it is, and the
/// reader state of the paragraph it sits in, put aside while its content is read into a
/// container of its own.
pub(in crate::odt) struct OpenNote {
    /// The `text:note` or `office:annotation` element's depth.
    pub(in crate::odt) depth: usize,
    pub(in crate::odt) endnote: bool,
    /// The citation's `text:label` (a custom mark).
    pub(in crate::odt) custom: String,
    /// Set for a comment.
    pub(in crate::odt) comment: Option<CommentInfo>,
    /// Set for a text box: the frame it is in, put aside with its placement while its content is
    /// read (a frame inside it is a frame of its own).
    pub(in crate::odt) text_box: Option<(Option<Frame>, Option<wordcraft_doc::para::Float>, model::BoxLook)>,
    pub(in crate::odt) para: Option<Para>,
    pub(in crate::odt) fmt: Vec<Fmt>,
    pub(in crate::odt) xml_space_end: bool,
    pub(in crate::odt) para_cont: bool,
    pub(in crate::odt) lists: Vec<(Option<String>, bool)>,
    pub(in crate::odt) item_used: Vec<bool>,
    pub(in crate::odt) list_id: u32,
    /// Stack heights when the note began: what the note opens and leaves open goes with it.
    pub(in crate::odt) containers: usize,
    pub(in crate::odt) tables: usize,
    pub(in crate::odt) cells: usize,
    pub(in crate::odt) lists_saved: usize,
}

/// A text field being read ([`super::fields`]).
pub(in crate::odt) struct OpenField {
    /// Its Word field code.
    pub(in crate::odt) instr: String,
    /// Its shown text so far.
    pub(in crate::odt) result: String,
    /// Its element's name and depth.
    pub(in crate::odt) name: String,
    pub(in crate::odt) depth: usize,
    /// The formatting it is in.
    pub(in crate::odt) fmt: Fmt,
    /// A sequence's reference name: a bookmark over the caption's label and number, which
    /// references to it show.
    pub(in crate::odt) anchor: Option<String>,
    /// Fixed without a value to be fixed at: a field while it shows nothing, else its text.
    pub(in crate::odt) fixed: bool,
}

impl Body<'_> {
    /// A note or its citation starting: true when handled. A note inside a note (allowed by the
    /// schema, never written by Word or LibreOffice), or one in deleted text or a frame, is left
    /// out with its content.
    pub(in crate::odt) fn start_note_part(&mut self, q: &str, a: &Attrs) -> bool {
        let free = self.note.is_none() && self.del.is_none() && !self.in_tc && self.frame.is_none();
        if let Some(c) = self.note.as_mut().and_then(|n| n.comment.as_mut()) {
            c.field = match q {
                "dc:creator" => Some(0),
                "dc:date" => Some(1),
                "meta:creator-initials" => Some(2),
                _ => None,
            };
            if c.field.is_some() {
                return true;
            }
        }
        match q {
            "office:annotation" if free && self.comment_names.len() < MAX_COMMENTS => self.start_comment(a),
            // A text box: its content read as a note's, the frame it is in put aside meanwhile.
            "draw:text-box" if self.frame.is_some() && self.note.is_none() && self.del.is_none() && !self.in_tc && self.frame_box.is_none() => {
                // A least height (ODF 1.3 §20.207 `fo:min-height`): the box grows to its text.
                if let Some(min) = get(a, "fo:min-height").and_then(length).filter(|v| v.is_finite() && *v >= 0.0)
                    && let Some(fr) = &mut self.frame
                {
                    self.frame_look.auto_fit = true;
                    fr.1 = fr.1.or(Some(min));
                }
                // Its name, and the box its text continues in (§19.112 `draw:chain-next-name`).
                let next = get(a, "draw:chain-next-name").map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(256).collect::<String>());
                if let Some(name) = self.frame_name.take().filter(|n| !n.is_empty()) {
                    self.frame_look.chain = Some(Box::new(model::Chain { name, next }));
                }
                let frame = (self.frame.take(), self.frame_float.take(), std::mem::take(&mut self.frame_look));
                self.start_note(a);
                if let Some(n) = &mut self.note {
                    n.text_box = Some(frame);
                }
            }
            "office:annotation-end" => {
                if let Some(id) = get(a, "office:name").and_then(|n| self.comment_names.get(n)).copied() {
                    self.push_inline(Inline::CommentEnd(id));
                }
            }
            "text:note" if free => self.start_note(a),
            // The citation's text is LibreOffice's number: only a custom mark is kept.
            "text:note-citation" => {
                if let Some(n) = &mut self.note {
                    n.custom = get(a, "text:label").map(|l| l.chars().filter(|c| !c.is_control()).take(10).collect()).unwrap_or_default();
                }
                self.skip = 1;
            }
            _ => return false,
        }
        true
    }

    /// Begin reading a comment, as a note: its text in a container of its own.
    pub(in crate::odt) fn start_comment(&mut self, a: &Attrs) {
        let id = self.next_comment;
        self.next_comment = self.next_comment.saturating_add(1);
        let name = get(a, "office:name").filter(|n| !n.is_empty() && n.len() <= MAX_INFO_CHARS);
        if let Some(n) = name {
            // The first comment of a name keeps it.
            self.comment_names.entry(n.to_string()).or_insert(id);
        }
        let parent = get(a, "loext:parent-name").and_then(|n| self.comment_names.get(n)).copied();
        let resolved = get(a, "loext:resolved").is_some_and(|r| r.trim() == "true");
        let info = CommentInfo { id, named: name.is_some(), parent, resolved, ..Default::default() };
        self.start_note(a);
        if let Some(n) = &mut self.note {
            n.comment = Some(info);
        }
    }

    /// `inline` at the end of the paragraph being read (a new one if none is).
    pub(in crate::odt) fn push_inline(&mut self, inline: Inline) {
        // A drawing in the text outside any paragraph waits for the next one.
        let drawing = matches!(inline, Inline::Image(_) | Inline::Shape { .. } | Inline::Chart(_) | Inline::Group { .. });
        if self.para.is_none() && drawing && self.at_body_level() && self.loose.len() < super::MAX_LOOSE {
            self.loose.push(inline);
            return;
        }
        if self.para.is_none() {
            self.start_para(None, None);
        }
        if let Some(p) = &mut self.para {
            p.inlines.push(inline);
        }
    }

    /// Begin reading a note: put the paragraph it is in aside and read its content into a
    /// container of its own, outside any list.
    pub(in crate::odt) fn start_note(&mut self, a: &Attrs) {
        let note = OpenNote {
            depth: self.depth,
            endnote: get(a, "text:note-class").is_some_and(|c| c.trim() == "endnote"),
            custom: String::new(),
            comment: None,
            text_box: None,
            para: self.para.take(),
            fmt: std::mem::take(&mut self.fmt),
            xml_space_end: std::mem::take(&mut self.xml_space_end),
            para_cont: std::mem::take(&mut self.para_cont),
            lists: std::mem::take(&mut self.lists),
            item_used: std::mem::take(&mut self.item_used),
            list_id: self.list_id,
            containers: self.containers.len(),
            tables: self.tables.len(),
            cells: self.cells.len(),
            lists_saved: self.lists_saved.len(),
        };
        self.containers.push(Vec::new());
        self.note = Some(note);
    }

    /// End the note being read: its content becomes an [`Inline::Note`] in the paragraph put
    /// aside, which goes on. Tables and cells the note left open (malformed) are dropped.
    pub(in crate::odt) fn end_note(&mut self) {
        self.end_para();
        let Some(n) = self.note.take() else { return };
        self.tables.truncate(n.tables);
        self.cells.truncate(n.cells);
        self.formulas.truncate(n.cells);
        self.cell_tops.truncate(n.cells);
        self.lists_saved.truncate(n.lists_saved);
        self.containers.truncate(n.containers + 1);
        let blocks = if self.containers.len() > n.containers { self.containers.pop().unwrap_or_default() } else { Vec::new() };
        self.para = n.para;
        self.fmt = n.fmt;
        self.xml_space_end = n.xml_space_end;
        self.para_cont = n.para_cont;
        self.lists = n.lists;
        self.item_used = n.item_used;
        self.list_id = n.list_id;
        if let Some((frame, float, look)) = n.text_box {
            // Back in the frame, which holds the box's content until it ends.
            (self.frame, self.frame_float, self.frame_look) = (frame, float, look);
            self.frame_box = Some(blocks);
            return;
        }
        match n.comment {
            Some(c) => {
                // LibreOffice writes an empty date as all zeroes; a date without a zone is
                // read as UTC, as the document's own dates are.
                let date = c.date.trim();
                let date = if date.is_empty() || date.starts_with("0000") {
                    String::new()
                } else if date.ends_with(['Z', 'z']) || date.rsplit_once('T').is_some_and(|(_, t)| t.contains(['+', '-'])) {
                    date.to_string()
                } else {
                    format!("{date}Z")
                };
                let comment = FComment {
                    author: c.author.trim().to_string(),
                    initials: c.initials.trim().to_string(),
                    date,
                    resolved: c.resolved,
                    parent: c.parent,
                    blocks,
                };
                self.push_inline(Inline::Comment { id: c.id, comment: Box::new(comment) });
                // A comment without a name has no end to wait for: it covers nothing.
                if !c.named && c.parent.is_none() {
                    self.push_inline(Inline::CommentEnd(c.id));
                }
            }
            None => {
                let fmt = self.cur_fmt();
                self.push_inline(Inline::Note { endnote: n.endnote, custom: n.custom, blocks, fmt });
            }
        }
    }

    /// An element ending at the current depth, for an open note or field: true when that was all
    /// there was to it.
    pub(in crate::odt) fn end_note_or_field(&mut self, q: &str) -> bool {
        if let Some(c) = self.note.as_mut().and_then(|n| n.comment.as_mut())
            && c.field.is_some()
        {
            c.field = None;
            return true;
        }
        if self.note.as_ref().is_some_and(|n| self.depth < n.depth) {
            // The note's own end, or (malformed) an outer element ending first: the note ends,
            // then that element.
            self.end_field();
            self.end_note();
            if q == "text:note" || q == "office:annotation" || q == "draw:text-box" {
                return true;
            }
            if self.shape_kind.is_some() && crate::odt::shapes::is_shape(q) {
                // A drawing shape's own end: it goes into the paragraph, its text read.
                self.end_frame();
                return true;
            }
        }
        // Everything inside a field was its text: only the field's own end does anything. An
        // outer element ending first (malformed) ends the field, then itself.
        if let Some(f) = &self.field {
            if self.depth >= f.depth {
                return true;
            }
            let own = f.name == q;
            self.end_field();
            return own;
        }
        false
    }

    /// Put the field read so far into the paragraph.
    pub(in crate::odt) fn end_field(&mut self) {
        let Some(OpenField { instr, result, mut fmt, anchor, fixed, .. }) = self.field.take() else { return };
        fmt.ins = self.open_insertion();
        if self.para.is_none() {
            self.start_para(None, None);
        }
        if let Some(p) = &mut self.para {
            if let Some(name) = &anchor {
                // From the start of the paragraph when only the label is before the number (a
                // caption, "Figure 1"), else from the number.
                let label = crate::odt::fields::sequence_name(&instr).unwrap_or("");
                let before: String = p.inlines.iter().map(|i| if let Inline::Text(t, _) = i { t.as_str() } else { "\u{1}" }).collect();
                let at = if before.trim() == label { 0 } else { p.inlines.len() };
                p.inlines.insert(at, Inline::Anchor(name.clone()));
            }
            // A fixed field keeps what it shows (§19.807 `text:fixed`): a locked field.
            p.inlines.push(Inline::Field { instr, result: result.trim().to_string(), fmt, locked: fixed && !result.trim().is_empty() });
            if let Some(name) = anchor {
                p.inlines.push(Inline::AnchorEnd(name));
            }
        }
        self.xml_space_end = false;
    }
}

#[cfg(test)]
mod tests {
    use crate::odt::{Body, Pages, Styles, events};

    /// The cell stacks after reading `xml`: (cells, formulas, cell tops).
    fn stacks_after(xml: &str) -> (usize, usize, usize) {
        let (styles, pages) = (Styles::default(), Pages::default());
        let mut none = |_: &str| None;
        let mut b = Body::new(&styles, &pages, &mut none);
        b.in_text = true;
        events(xml.as_bytes(), |ev| b.feed(ev));
        (b.cells.len(), b.formulas.len(), b.cell_tops.len())
    }

    /// A formula cell a note or a deletion leaves open (malformed) goes with it: nothing of it is
    /// left for the cell round the note, or for the next cell read.
    #[test]
    fn a_cell_a_note_or_deletion_leaves_open_takes_its_formula_with_it() {
        // The cell, its row and its table end with tags that are none of theirs.
        let open_cell = r#"<table:table><table:table-row><table:table-cell table:formula="ooow:1+1" office:value-type="float"><text:p>2</text:p></wcx:x></wcx:x></wcx:x>"#;
        let in_note = format!(
            "<office:text><table:table><table:table-row><table:table-cell><text:p>a<text:note text:note-class=\"footnote\"><text:note-citation>1</text:note-citation><text:note-body>{open_cell}</text:note-body></text:note></text:p></table:table-cell></table:table-row></table:table></office:text>"
        );
        assert_eq!(stacks_after(&in_note), (0, 0, 0));
        let in_deletion = format!(
            "<office:text><text:tracked-changes><text:changed-region text:id=\"c1\"><text:deletion><office:change-info><dc:creator>A</dc:creator></office:change-info>{open_cell}</text:deletion></text:changed-region></text:tracked-changes><text:p>b</text:p></office:text>"
        );
        assert_eq!(stacks_after(&in_deletion), (0, 0, 0));
    }
}
