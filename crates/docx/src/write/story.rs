//! Blocks, paragraphs, runs, objects and tables → WordprocessingML.

use wordcraft_doc::effects::ShapeEffects;
use wordcraft_doc::para::{AltText, Anchor, Float, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{CharProps, PropChange, Rgb, TextColor};
use wordcraft_doc::section::{LineNumberRestart, NoteProps, SectionProps, SectionStart};
use wordcraft_doc::table::Table;
use wordcraft_doc::{Block, Blocks, InlineObject, Paragraph, RevisionKind};

use super::props::{ChangeAttrs, borders, open_change, ppr_inner, rpr, rpr_inner, tblpr, tcpr, trpr};
use super::{PartRels, Writer};
use crate::package::rt;
use crate::units::{emu, n, twips};
use crate::xml::W;

const MAX_DEPTH: usize = 24;

enum Piece<'a> {
    Text(String),
    Tab,
    Br(Option<&'static str>),
    NbHyphen,
    SoftHyphen,
    /// An object and its byte offset in the paragraph.
    Obj(&'a InlineObject, usize),
}

impl Writer<'_> {
    /// Write a block list. `top` = body level (sections allowed).
    pub fn blocks(&mut self, w: &mut W, bl: &Blocks, rels: &mut PartRels, top: bool, depth: usize) {
        if depth > MAX_DEPTH {
            w.raw("<w:p/>");
            return;
        }
        let plan = super::sdt::plan(bl);
        // A TOC inside a content control already has one (read from a file).
        let toc = if top { toc_span(bl).filter(|(start, _)| !plan.covers(*start)) } else { None };
        // Whether the heading's TOC field was written and still needs its end.
        let mut toc_field_open = false;
        for (i, b) in bl.iter().enumerate() {
            for c in plan.opens.get(i).into_iter().flatten() {
                super::sdt::open(w, c);
            }
            if let Some((start, end)) = toc {
                if i == start {
                    w.raw(TOC_SDT_OPEN);
                    self.toc_hold_end = true;
                }
                self.toc_begin_here = toc_field_open && i == start + 1;
                self.toc_end_here = toc_field_open && i == end;
            }
            match &**b {
                Block::Para(p) => {
                    self.inline_controls = plan.inline_in(i);
                    self.para(w, p, rels, top, depth);
                }
                Block::Table(t) => self.table(w, t, rels, depth),
            }
            if let Some((start, end)) = toc {
                if i == start {
                    // `field` holds the heading's TOC field back for the first entry.
                    self.toc_hold_end = false;
                    toc_field_open = self.toc_field.is_some();
                }
                if i == end {
                    w.raw("</w:sdtContent></w:sdt>");
                }
            }
            for _ in 0..plan.closes.get(i).copied().unwrap_or(0) {
                super::sdt::close(w);
            }
        }
        if depth == 0 {
            self.move_ranges(w, &[]);
        }
        if bl.last().is_none_or(|b| matches!(**b, Block::Table(_))) {
            // A header with no paragraph of its own before: the watermark goes in this one.
            if depth == 0
                && let Some(run) = self.pending_watermark.take()
            {
                w.raw("<w:p>");
                w.raw(&run);
                w.raw("</w:p>");
            } else {
                w.raw("<w:p/>");
            }
        }
    }

    /// The moves (revision, destination?) in `props`.
    fn moves_of(&self, props: &CharProps) -> Vec<(u32, bool)> {
        [(props.del, false), (props.ins, true)]
            .into_iter()
            .filter_map(|(r, to)| r.filter(|r| self.doc.move_name(Some(*r)).is_some()).map(|r| (r, to)))
            .collect()
    }

    /// Bring the open move ranges to `want` (ECMA-376 §17.13.5.22–27): end the ranges of moves
    /// that stop here, start those that begin. A range is written around each stretch of a
    /// move's text (and paragraph marks), with the move's name, author and date.
    fn move_ranges(&mut self, w: &mut W, want: &[(u32, bool)]) {
        let mut k = 0;
        while k < self.open_moves.len() {
            match self.open_moves.get(k) {
                Some((rev, id, to)) if !want.contains(&(*rev, *to)) => {
                    w.empty(if *to { "w:moveToRangeEnd" } else { "w:moveFromRangeEnd" }, &[("w:id", id)]);
                    self.open_moves.remove(k);
                }
                _ => k += 1,
            }
        }
        for &(rev, to) in want {
            if self.open_moves.iter().any(|(r, _, t)| *r == rev && *t == to) {
                continue;
            }
            let name = self.doc.move_name(Some(rev)).unwrap_or("").to_string();
            let (id, author, date) = self.rev_attrs(rev);
            let tag = if to { "w:moveToRangeStart" } else { "w:moveFromRangeStart" };
            if date.is_empty() {
                w.empty(tag, &[("w:id", &id), ("w:name", &name), ("w:author", &author)]);
            } else {
                w.empty(tag, &[("w:id", &id), ("w:name", &name), ("w:author", &author), ("w:date", &date)]);
            }
            self.open_moves.push((rev, id, to));
        }
    }

    fn para(&mut self, w: &mut W, p: &Paragraph, rels: &mut PartRels, top: bool, depth: usize) {
        // Which content control markers are written inline (set by `blocks`).
        let inline = std::mem::take(&mut self.inline_controls);
        // A drop cap is a separate framed paragraph holding the first character.
        if let Some(lines) = p.props.drop_cap.filter(|l| *l > 0)
            && let Some(first) = p.text.chars().next().filter(|c| !matches!(*c, wordcraft_doc::para::OBJ | '\t' | '\n' | '\u{c}' | '\u{e}'))
            && p.text.len() > first.len_utf8()
        {
            let mut head = p.clone();
            let tail = head.split_off(first.len_utf8()).ok();
            if let Some(mut tail) = tail {
                head.props.drop_cap = Some(lines);
                head.section = None;
                self.para_inner(w, &head, rels, false, depth, true);
                tail.props.drop_cap = None;
                tail.section = p.section.clone();
                self.inline_controls = inline.iter().filter_map(|o| o.checked_sub(first.len_utf8())).collect();
                self.para_inner(w, &tail, rels, top, depth, false);
                return;
            }
        }
        self.inline_controls = inline;
        self.para_inner(w, p, rels, top, depth, false);
    }

    fn para_inner(&mut self, w: &mut W, p: &Paragraph, rels: &mut PartRels, top: bool, depth: usize, framed: bool) {
        let para_id = self.take_para_id(p);
        match &para_id {
            Some(id) => w.open("w:p", &[("w14:paraId", id), ("w14:textId", "77777777")]),
            None => w.open("w:p", &[]),
        }
        let mut pp = p.props.clone();
        if !framed {
            pp.drop_cap = None;
        }
        let section = if top { p.section.as_deref() } else { None };
        // A tracked paragraph mark: `w:ins` / `w:del` first in the mark's `w:rPr` (CT_ParaRPr).
        let mark_revs: Vec<(u32, &str)> = [(p.mark.ins, "w:ins"), (p.mark.del, "w:del")]
            .into_iter()
            .filter_map(|(idx, tag)| idx.filter(|i| self.rev_kind(Some(*i)).is_some_and(|k| k != RevisionKind::Format)).map(|i| (i, tag)))
            .map(|(i, tag)| (i, self.rev_tag(i, tag)))
            .collect();
        let has_mark = super::props::has_rpr(&p.mark) || !mark_revs.is_empty() || p.mark.fmt_change.is_some();
        if !pp.is_empty() || has_mark || section.is_some() {
            w.open("w:pPr", &[]);
            let num_change = pp.num_change.as_ref().map(|c| {
                let mut a = self.change_attrs(c.rev);
                a.push(("w:original", c.original.clone()));
                a
            });
            ppr_inner(w, &pp, framed, num_change.as_ref());
            if has_mark {
                w.open("w:rPr", &[]);
                for (idx, tag) in mark_revs {
                    let (id, author, date) = self.rev_attrs(idx);
                    if date.is_empty() {
                        w.empty(tag, &[("w:id", &id), ("w:author", &author)]);
                    } else {
                        w.empty(tag, &[("w:id", &id), ("w:author", &author), ("w:date", &date)]);
                    }
                }
                rpr_inner(w, &p.mark);
                if let Some(ch) = p.mark.fmt_change.as_deref() {
                    self.rpr_change(w, ch);
                }
                w.close("w:rPr");
            }
            if let Some(s) = section {
                self.sectpr(w, s, rels);
            }
            // The paragraph properties before a tracked change come last (§17.13.5.29).
            if let Some(ch) = &pp.fmt_change {
                let a = self.change_attrs(ch.rev);
                open_change(w, "w:pPrChange", &a);
                w.open("w:pPr", &[]);
                ppr_inner(w, &ch.old, false, None);
                w.close("w:pPr");
                w.close("w:pPrChange");
            }
            w.close("w:pPr");
        }
        // A note's first paragraph starts with its reference mark. When the paragraph holds the
        // note's reference to itself (notes read from a file, or made by references.footnote),
        // the mark is written there instead, with that reference's formatting.
        if !self.holds_own_ref(p)
            && let Some(mark) = self.pending_mark.take()
        {
            w.raw("<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr>");
            w.empty(mark, &[]);
            w.raw("</w:r>");
        }
        // A header's watermark, in its first top-level paragraph (not a table cell or text box).
        if depth == 0
            && let Some(run) = self.pending_watermark.take()
        {
            w.raw(&run);
        }
        // Taken before the content so a nested story (a text box in an entry) can't claim them.
        if std::mem::take(&mut self.toc_begin_here)
            && let Some((instr, locked, dirty, props)) = self.toc_field.take()
        {
            self.rev_open(w, &props);
            self.field_start(w, &instr, locked, dirty, "", &props);
            self.rev_close(w, &props);
        }
        let toc_end = std::mem::take(&mut self.toc_end_here);
        self.para_content(w, p, rels, depth);
        if toc_end {
            w.raw(r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#);
        }
        // Move ranges stay open across a moved paragraph mark; others end with the paragraph.
        let mark_moves = self.moves_of(&p.mark);
        self.move_ranges(w, &mark_moves);
        w.close("w:p");
    }

    fn para_content(&mut self, w: &mut W, p: &Paragraph, rels: &mut PartRels, depth: usize) {
        // Taken first: a text box in this paragraph writes paragraphs of its own.
        let inline = std::mem::take(&mut self.inline_controls);
        // Split into pieces with their props.
        let mut pieces: Vec<(Piece, &CharProps)> = Vec::new();
        let mut k = 0usize;
        for (range, props) in p.run_ranges() {
            let base = range.start;
            let Some(text) = p.text.get(range) else { continue };
            let mut buf = String::new();
            for (ci, c) in text.char_indices() {
                let special = match c {
                    '\t' => Some(Piece::Tab),
                    '\n' => Some(Piece::Br(None)),
                    '\u{000C}' => Some(Piece::Br(Some("page"))),
                    '\u{000E}' => Some(Piece::Br(Some("column"))),
                    '\u{2011}' => Some(Piece::NbHyphen),
                    '\u{00AD}' => Some(Piece::SoftHyphen),
                    wordcraft_doc::para::OBJ => {
                        let o = p.objects.get(k);
                        k += 1;
                        Some(match o {
                            Some(o) => Piece::Obj(o, base + ci),
                            None => Piece::Text(String::new()),
                        })
                    }
                    _ => None,
                };
                match special {
                    Some(sp) => {
                        if !buf.is_empty() {
                            pieces.push((Piece::Text(std::mem::take(&mut buf)), props));
                        }
                        pieces.push((sp, props));
                    }
                    None => buf.push(c),
                }
            }
            if !buf.is_empty() {
                pieces.push((Piece::Text(buf), props));
            }
        }

        let mut open_link: Option<String> = None;
        let mut i = 0;
        while i < pieces.len() {
            let Some((piece, props)) = pieces.get(i) else { break };
            let props: &CharProps = props;
            // Content control boundaries: written inline when the plan says so (else they are
            // written around the paragraph, or were left without a partner). A hyperlink never
            // crosses one.
            if let Piece::Obj(o @ (InlineObject::ControlStart { .. } | InlineObject::ControlEnd), off) = piece {
                if inline.binary_search(off).is_ok() {
                    if open_link.take().is_some() {
                        w.close("w:hyperlink");
                    }
                    match o {
                        InlineObject::ControlStart { control } => super::sdt::open(w, control),
                        _ => super::sdt::close(w),
                    }
                }
                i += 1;
                continue;
            }
            // Hyperlink grouping.
            if props.link != open_link {
                if open_link.is_some() {
                    w.close("w:hyperlink");
                }
                open_link = None;
                if let Some(l) = props.link.as_deref().filter(|l| !l.is_empty()) {
                    if let Some(anchor) = l.strip_prefix('#') {
                        w.open("w:hyperlink", &[("w:anchor", anchor), ("w:history", "1")]);
                    } else {
                        let id = rels.add(rt::HYPERLINK, l, true);
                        w.open("w:hyperlink", &[("r:id", &id), ("w:history", "1")]);
                    }
                    open_link = Some(l.to_string());
                }
            }
            match piece {
                Piece::Obj(o, _) => {
                    self.object(w, o, props, rels, depth);
                    i += 1;
                }
                _ => {
                    // Gather simple pieces sharing these props into one run.
                    let mut j = i;
                    while let Some((pc, pr)) = pieces.get(j) {
                        if matches!(pc, Piece::Obj(..)) || !(std::ptr::eq(*pr, props) || *pr == props) {
                            break;
                        }
                        j += 1;
                    }
                    let del = self.is_del(props);
                    let moves = self.moves_of(props);
                    self.move_ranges(w, &moves);
                    self.rev_open(w, props);
                    w.open("w:r", &[]);
                    self.rpr(w, props);
                    for (pc, _) in pieces.get(i..j).unwrap_or(&[]) {
                        match pc {
                            Piece::Text(t) if !t.is_empty() => w.leaf(if del { "w:delText" } else { "w:t" }, &[("xml:space", "preserve")], t),
                            Piece::Tab => w.empty("w:tab", &[]),
                            Piece::Br(None) => w.empty("w:br", &[]),
                            Piece::Br(Some(t)) => w.empty("w:br", &[("w:type", t)]),
                            Piece::NbHyphen => w.empty("w:noBreakHyphen", &[]),
                            Piece::SoftHyphen => w.empty("w:softHyphen", &[]),
                            _ => {}
                        }
                    }
                    w.close("w:r");
                    self.rev_close(w, props);
                    i = j.max(i + 1);
                }
            }
        }
        if open_link.is_some() {
            w.close("w:hyperlink");
        }
    }

    fn rev_kind(&self, idx: Option<u32>) -> Option<RevisionKind> {
        idx.and_then(|i| self.doc.revisions.get(i as usize)).map(|r| r.kind)
    }

    /// Deleted text goes in `w:delText`; text moved away stays in `w:t` (§17.13.5.22).
    fn is_del(&self, props: &CharProps) -> bool {
        props.del.is_some() && self.rev_kind(props.del) != Some(RevisionKind::Format) && self.doc.move_name(props.del).is_none()
    }

    /// `w:ins` / `w:del`, or `w:moveTo` / `w:moveFrom` when revision `idx` is one end of a move.
    fn rev_tag(&self, idx: u32, tag: &'static str) -> &'static str {
        match (self.doc.move_name(Some(idx)).is_some(), tag) {
            (true, "w:ins") => "w:moveTo",
            (true, "w:del") => "w:moveFrom",
            _ => tag,
        }
    }

    fn rev_attrs(&mut self, idx: u32) -> (String, String, String) {
        self.next_rev_id += 1;
        let r = self.doc.revisions.get(idx as usize);
        let author = super::annotation_author(self.doc, r.map_or("", |r| r.author.as_str())).to_string();
        // Remove date and time (§17.15.1.69): tracked changes are saved without `w:date`.
        let date = if self.doc.settings.remove_date_time { String::new() } else { r.map(|r| r.date.clone()).unwrap_or_default() };
        (self.next_rev_id.to_string(), author, date)
    }

    /// `w:id`, `w:author` and `w:date` of a tracked change by revision `idx`.
    fn change_attrs(&mut self, idx: u32) -> ChangeAttrs {
        let (id, author, date) = self.rev_attrs(idx);
        let mut a = vec![("w:id", id), ("w:author", author)];
        if !date.is_empty() {
            a.push(("w:date", date));
        }
        a
    }

    /// A run's `w:rPr`, with its tracked formatting change last (§17.13.5.31).
    fn rpr(&mut self, w: &mut W, c: &CharProps) {
        let Some(ch) = c.fmt_change.as_deref() else { return rpr(w, c) };
        w.open("w:rPr", &[]);
        rpr_inner(w, c);
        self.rpr_change(w, ch);
        w.close("w:rPr");
    }

    fn rpr_change(&mut self, w: &mut W, ch: &PropChange<CharProps>) {
        let a = self.change_attrs(ch.rev);
        open_change(w, "w:rPrChange", &a);
        w.open("w:rPr", &[]);
        rpr_inner(w, &ch.old);
        w.close("w:rPr");
        w.close("w:rPrChange");
    }

    fn rev_open(&mut self, w: &mut W, props: &CharProps) {
        for (idx, tag) in [(props.ins, "w:ins"), (props.del, "w:del")] {
            let Some(idx) = idx else { continue };
            if self.rev_kind(Some(idx)) == Some(RevisionKind::Format) {
                continue;
            }
            let tag = self.rev_tag(idx, tag);
            let (id, author, date) = self.rev_attrs(idx);
            if date.is_empty() {
                w.open(tag, &[("w:id", &id), ("w:author", &author)]);
            } else {
                w.open(tag, &[("w:id", &id), ("w:author", &author), ("w:date", &date)]);
            }
        }
    }

    fn rev_close(&mut self, w: &mut W, props: &CharProps) {
        for (idx, tag) in [(props.del, "w:del"), (props.ins, "w:ins")] {
            if let Some(i) = idx
                && self.rev_kind(idx) != Some(RevisionKind::Format)
            {
                w.close(self.rev_tag(i, tag));
            }
        }
    }

    fn object(&mut self, w: &mut W, o: &InlineObject, props: &CharProps, rels: &mut PartRels, depth: usize) {
        match o {
            // Written by `para_content` (inline) or `blocks` (around paragraphs).
            InlineObject::ControlStart { .. } | InlineObject::ControlEnd => {}
            InlineObject::BookmarkStart { name } => {
                let id = self.bookmark_id(name);
                w.empty("w:bookmarkStart", &[("w:id", &id), ("w:name", name)]);
            }
            InlineObject::BookmarkEnd { name } => {
                let id = self.bookmark_id(name);
                w.empty("w:bookmarkEnd", &[("w:id", &id)]);
            }
            InlineObject::PermStart { id, group, editor } => {
                let id = id.to_string();
                let mut a = vec![("w:id", id.as_str())];
                if !group.is_empty() {
                    a.push(("w:edGrp", group.as_str()));
                }
                if !editor.is_empty() {
                    a.push(("w:ed", editor.as_str()));
                }
                w.empty("w:permStart", &a);
            }
            InlineObject::PermEnd { id } => w.empty("w:permEnd", &[("w:id", &id.to_string())]),
            InlineObject::CommentStart { id } => {
                if self.doc.comments.contains_key(id) {
                    w.empty("w:commentRangeStart", &[("w:id", &id.to_string())]);
                }
            }
            InlineObject::CommentEnd { id } => {
                if self.doc.comments.contains_key(id) {
                    let s = id.to_string();
                    w.empty("w:commentRangeEnd", &[("w:id", &s)]);
                    w.open("w:r", &[]);
                    w.open("w:rPr", &[]);
                    w.val("w:rStyle", comment_reference_style(self.doc));
                    w.close("w:rPr");
                    w.empty("w:commentReference", &[("w:id", &s)]);
                    w.close("w:r");
                }
            }
            InlineObject::Equation { linear, display, math } => {
                // Automatic equation numbers become text: Word numbers nothing by itself.
                let color = match props.color {
                    Some(TextColor::Rgb(c)) => Some(c),
                    _ => None,
                };
                match super::math::resolve_numbers(math, *display, &mut self.eq_number) {
                    Some(m) => super::math::write_equation(w, linear, *display, &m, color),
                    None => super::math::write_equation(w, linear, *display, math, color),
                }
            }
            InlineObject::Field { instr, result, locked, dirty, form, .. } => self.field(w, instr, result, *locked, *dirty, form, props),
            InlineObject::FieldStart { instr, locked, dirty } => {
                self.rev_open(w, props);
                self.field_start(w, instr, *locked, *dirty, "", props);
                self.rev_close(w, props);
            }
            InlineObject::FieldEnd => {
                self.rev_open(w, props);
                field_run(w, props, &|w| w.empty("w:fldChar", &[("w:fldCharType", "end")]));
                self.rev_close(w, props);
            }
            InlineObject::NoteRef { kind, id, custom } => {
                let foot = *kind == NoteKind::Footnote;
                if self.current_note == Some((foot, *id)) {
                    // The note's own number is the w:footnoteRef / w:endnoteRef mark, written once
                    // per note: here for its first reference to itself, never as a reference.
                    if let Some(mark) = self.pending_mark.take() {
                        let mut p = props.clone();
                        if p.style.is_none() && p.vert_align.is_none() {
                            p.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript);
                        }
                        self.rev_open(w, props);
                        w.open("w:r", &[]);
                        self.rpr(w, &p);
                        w.empty(mark, &[]);
                        w.close("w:r");
                        self.rev_close(w, props);
                    }
                    return;
                }
                let nid = self.note_id(*id, foot);
                let mut p = props.clone();
                if p.style.is_none() && p.vert_align.is_none() {
                    p.vert_align = Some(wordcraft_doc::props::VertAlign::Superscript);
                }
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, &p);
                let tag = if foot { "w:footnoteReference" } else { "w:endnoteReference" };
                if custom.is_empty() {
                    w.empty(tag, &[("w:id", &nid)]);
                } else {
                    w.empty(tag, &[("w:customMarkFollows", "1"), ("w:id", &nid)]);
                    w.leaf("w:t", &[("xml:space", "preserve")], custom);
                }
                w.close("w:r");
                self.rev_close(w, props);
            }
            // Editable charts and SmartArt keep main's model writer; opaque graphics and native
            // ink keep their original source and related parts.
            InlineObject::Graphic { w: gw, h: gh, alt, float, graphic } => {
                let ink = graphic.kind == wordcraft_doc::graphic::GraphicKind::Ink;
                let root = match (graphic.chart.as_deref(), graphic.smart_art.as_deref(), graphic.source.as_deref()) {
                    (Some(spec), _, _) => crate::xml::parse(self.chart_graphic(spec, rels).as_bytes()).ok(),
                    (None, Some(spec), _) => crate::xml::parse(self.smart_art_graphic(spec, *gw, *gh, rels).as_bytes()).ok(),
                    (None, None, Some(src)) => self.embedded_tree(src, rels, ink.then_some((*gw, *gh, float, alt.as_str()))),
                    (None, None, None) => None,
                };
                let Some(mut root) = root else { return };
                let direct_ink = ink && root.name == "w:contentPart";
                if ink && !direct_ink {
                    Self::ink_transform(&mut root, (graphic.w, graphic.h), (*gw, *gh), float, 0);
                }
                // Native ink, or an Office 2016 chart (`cx:chart`) with its fallback picture.
                let alternate = (root.name == "mc:AlternateContent").then_some(&root);
                let fallback = alternate.and_then(|e| e.child("mc:Fallback"));
                let choice = alternate.and_then(|e| e.child("mc:Choice"));
                let native = choice.and_then(|e| e.find("a:graphic"));
                if alternate.is_some() && native.is_none() {
                    return;
                }
                // A chart's picture gets the chart's current frame too; ink's VML preview is resized in place.
                let picture = if ink { None } else { fallback.and_then(|e| e.find("a:graphic")) };
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, props);
                if direct_ink {
                    w.raw(&root.to_xml());
                } else {
                    if fallback.is_some() {
                        let requires = if ink { "wpi" } else { choice.and_then(|c| c.attr("Requires")).unwrap_or_default() };
                        w.open("mc:AlternateContent", &[]);
                        w.open("mc:Choice", &[("Requires", requires)]);
                    }
                    let docpr = self.next_docpr();
                    let name = match graphic.kind {
                        wordcraft_doc::graphic::GraphicKind::Chart => format!("Chart {docpr}"),
                        wordcraft_doc::graphic::GraphicKind::Diagram => format!("Diagram {docpr}"),
                        wordcraft_doc::graphic::GraphicKind::Ink => format!("Ink {docpr}"),
                    };
                    let z = self.z;
                    self.graphic_frame(w, float, (*gw, *gh), &docpr, &name, alt, native.unwrap_or(&root));
                    if let Some(fallback) = fallback {
                        w.close("mc:Choice");
                        match picture {
                            Some(picture) => {
                                // Both branches are one object: same id and stacking order.
                                self.z = z;
                                w.open("mc:Fallback", &[]);
                                self.graphic_frame(w, float, (*gw, *gh), &docpr, &name, alt, picture);
                                w.close("mc:Fallback");
                            }
                            None => w.raw(&fallback.to_xml()),
                        }
                        w.close("mc:AlternateContent");
                    }
                }
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Image { media, w: iw, h: ih, alt, float, ole, .. } => {
                // An OLE object read from a file: the object itself, at its current size.
                if let Some(src) = ole.as_deref()
                    && let Some(obj) = self.embedded_xml(src, rels, Some((*iw, *ih, float, alt.as_str())))
                {
                    self.rev_open(w, props);
                    w.open("w:r", &[]);
                    self.rpr(w, props);
                    w.raw(&obj);
                    w.close("w:r");
                    self.rev_close(w, props);
                    return;
                }
                let Some(file) = self.media_files.get(media).cloned() else { return };
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, props);
                w.open("w:drawing", &[]);
                let docpr = self.next_docpr();
                let name = format!("Picture {docpr}");
                self.drawing_open(w, float, *iw, *ih, &docpr, &name, alt);
                w.empty("wp:cNvGraphicFramePr", &[]);
                w.open("a:graphic", &[]);
                w.open("a:graphicData", &[("uri", "http://schemas.openxmlformats.org/drawingml/2006/picture")]);
                self.pic(w, o, &file, (0.0, 0.0), rels);
                w.close("a:graphicData");
                w.close("a:graphic");
                w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
                w.close("w:drawing");
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Shape { kind, w: sw, h: sh, float, effects, freeform, extra, .. } => {
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, props);
                w.open("w:drawing", &[]);
                let docpr = self.shape_docpr(extra.id);
                // Ink is told apart by its name (and its pen), which reading looks for.
                let name = shape_name(*kind, freeform.as_deref(), &docpr);
                // The effect extent leaves room for the shadow and glow.
                let mut float = float.clone();
                for (e, fx) in float.effect.iter_mut().zip(effects.extent()) {
                    *e = wordcraft_geom::finite(*e).max(fx);
                }
                // Tight and Through wrapping need a polygon: the shape's outline unless it has one.
                if matches!(float.wrap, Wrap::Tight | Wrap::Through) && float.wrap_polygon.is_none() {
                    float.wrap_polygon = wordcraft_doc::wrap::shape_outline(*kind, *sw, *sh, extra.corner)
                        .and_then(|pts| wordcraft_doc::wrap::WrapPolygon::from_unit(false, &pts))
                        .map(std::sync::Arc::new);
                }
                let float = &float;
                self.drawing_open(w, float, *sw, *sh, &docpr, &name, o.alt_text());
                w.empty("wp:cNvGraphicFramePr", &[]);
                w.open("a:graphic", &[]);
                w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingShape")]);
                self.wsp(w, o, None, (0.0, 0.0), rels, depth);
                w.close("a:graphicData");
                w.close("a:graphic");
                w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
                w.close("w:drawing");
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Group { w: gw, h: gh, float, ch_w, ch_h, children, canvas } => {
                self.rev_open(w, props);
                w.open("w:r", &[]);
                self.rpr(w, props);
                w.open("w:drawing", &[]);
                let docpr = self.next_docpr();
                let name = format!("{} {docpr}", if canvas.is_some() { "Canvas" } else { "Group" });
                self.drawing_open(w, float, *gw, *gh, &docpr, &name, o.alt_text());
                w.empty("wp:cNvGraphicFramePr", &[]);
                w.open("a:graphic", &[]);
                let outer = match canvas {
                    Some(c) => {
                        // A Drawing Canvas: its background (`wpc:bg`) and outline (`wpc:whole`), then
                        // its members, at offsets in the canvas's own (unscaled) space.
                        w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas")]);
                        w.open("wpc:wpc", &[]);
                        w.open("wpc:bg", &[]);
                        solid_fill(w, c.fill);
                        w.close("wpc:bg");
                        w.open("wpc:whole", &[]);
                        match c.stroke {
                            Some(_) => w.open("a:ln", &[("w", &emu(c.stroke_width.clamp(0.0, 100.0)))]),
                            None => w.open("a:ln", &[]),
                        }
                        solid_fill(w, c.stroke);
                        w.close("a:ln");
                        w.close("wpc:whole");
                        "wpc:wpc"
                    }
                    None => {
                        w.open("a:graphicData", &[("uri", "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup")]);
                        // ECMA-376 §20.1.7.6 (`a:xfrm` of a group): the members' space `a:chOff`/`a:chExt`
                        // maps onto the group's `a:off`/`a:ext`.
                        w.open("wpg:wgp", &[]);
                        w.empty("wpg:cNvGrpSpPr", &[]);
                        w.open("wpg:grpSpPr", &[]);
                        w.open("a:xfrm", &spin_attrs(float.spin()).iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
                        w.empty("a:off", &[("x", "0"), ("y", "0")]);
                        w.empty("a:ext", &[("cx", &emu(gw.max(0.0))), ("cy", &emu(gh.max(0.0)))]);
                        w.empty("a:chOff", &[("x", "0"), ("y", "0")]);
                        w.empty("a:chExt", &[("cx", &emu(ch_w.max(0.0))), ("cy", &emu(ch_h.max(0.0)))]);
                        w.close("a:xfrm");
                        w.close("wpg:grpSpPr");
                        "wpg:wgp"
                    }
                };
                for c in children.iter().take(wordcraft_doc::para::MAX_GROUP_CHILDREN) {
                    let at = (c.x, c.y);
                    match &c.obj {
                        InlineObject::Image { media, .. } => {
                            if let Some(file) = self.media_files.get(media).cloned() {
                                self.pic(w, &c.obj, &file, at, rels);
                            }
                        }
                        InlineObject::Shape { extra, .. } => {
                            let id = self.shape_docpr(extra.id);
                            self.wsp(w, &c.obj, Some(&id), at, rels, depth);
                        }
                        _ => {}
                    }
                }
                if canvas.is_some() && float.spin() != wordcraft_geom::Spin::default() {
                    // The canvas schema has no transform of its own: keep its turn in our
                    // extension (Word draws it unturned, as it can't turn a canvas).
                    w.open("wpc:extLst", &[]);
                    w.open("a:ext", &[("uri", crate::CANVAS_SPIN_EXT)]);
                    xfrm_spin_only(w, float.spin());
                    w.close("a:ext");
                    w.close("wpc:extLst");
                }
                w.close(outer);
                w.close("a:graphicData");
                w.close("a:graphic");
                w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
                w.close("w:drawing");
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Ruby { base, ruby, align, size, raise, base_size, lang, props: ruby_props } => {
                // ECMA-376 §17.3.3.25: a run holding w:ruby (properties, ruby text, base text).
                let t = if self.is_del(props) { "w:delText" } else { "w:t" };
                let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
                let hps = |v: f32, min: f32| n(wordcraft_geom::to_half_points(v.clamp(min, 1584.0)));
                let size = fin(*size, 5.0).clamp(1.0, 1584.0);
                let base_size = if base_size.is_finite() && *base_size > 0.0 { *base_size } else { props.size.unwrap_or(10.5) };
                self.rev_open(w, props);
                w.open("w:r", &[]);
                rpr(w, props);
                w.open("w:ruby", &[]);
                w.open("w:rubyPr", &[]);
                w.val("w:rubyAlign", align.ooxml());
                w.val("w:hps", &hps(size, 1.0));
                w.val("w:hpsRaise", &hps(fin(*raise, 0.0), 0.0));
                w.val("w:hpsBaseText", &hps(fin(base_size, 10.5), 1.0));
                w.val("w:lid", if lang.is_empty() { "ja-JP" } else { lang });
                w.close("w:rubyPr");
                let mut rt_props = props.clone().overlaid(ruby_props);
                rt_props.size = Some(size);
                for (tag, p, text) in [("w:rt", &rt_props, ruby), ("w:rubyBase", props, base)] {
                    w.open(tag, &[]);
                    w.open("w:r", &[]);
                    rpr(w, p);
                    w.leaf(t, &[("xml:space", "preserve")], text);
                    w.close("w:r");
                    w.close(tag);
                }
                w.close("w:ruby");
                w.close("w:r");
                self.rev_close(w, props);
            }
            InlineObject::Opaque { format, xml, text } => {
                if format == "docx" && crate::xml::parse(xml.as_bytes()).is_ok() {
                    w.raw(xml);
                } else if !text.is_empty() {
                    self.rev_open(w, props);
                    w.open("w:r", &[]);
                    self.rpr(w, props);
                    w.leaf("w:t", &[("xml:space", "preserve")], text);
                    w.close("w:r");
                    self.rev_close(w, props);
                }
            }
        }
    }

    fn field(&mut self, w: &mut W, instr: &str, result: &str, locked: bool, dirty: bool, form: &str, props: &CharProps) {
        if self.toc_hold_end && is_toc(instr) {
            // A TOC heading's field opens in the first entry and closes after the last (see `toc_span`).
            self.toc_hold_end = false;
            self.toc_field = Some((instr.to_string(), locked, dirty, props.clone()));
            return;
        }
        if let Some(e) = wordcraft_doc::eq::Enclosure::parse(instr) {
            self.rev_open(w, props);
            self.enclosure(w, &e, locked, props);
            self.rev_close(w, props);
            return;
        }
        let del = self.is_del(props);
        let run = |w: &mut W, f: &dyn Fn(&mut W)| {
            w.open("w:r", &[]);
            rpr(w, props);
            f(w);
            w.close("w:r");
        };
        self.rev_open(w, props);
        self.field_start(w, instr, locked, dirty, form, props);
        if !result.is_empty() {
            run(w, &|w| {
                let mut buf = String::new();
                let flush = |w: &mut W, buf: &mut String| {
                    if !buf.is_empty() {
                        w.leaf(if del { "w:delText" } else { "w:t" }, &[("xml:space", "preserve")], buf);
                        buf.clear();
                    }
                };
                for c in result.chars() {
                    match c {
                        '\t' => {
                            flush(w, &mut buf);
                            w.empty("w:tab", &[]);
                        }
                        '\n' | '\r' => {
                            flush(w, &mut buf);
                            w.empty("w:br", &[]);
                        }
                        c if (c as u32) < 0x20 || c == wordcraft_doc::para::OBJ => {}
                        c => buf.push(c),
                    }
                }
                flush(w, &mut buf);
            });
        }
        run(w, &|w| w.empty("w:fldChar", &[("w:fldCharType", "end")]));
        self.rev_close(w, props);
    }

    /// Enclosed characters as Word stores them: an `eq \o\ac(○,字)` field with no result, whose
    /// code is split so the symbol and the text get their own sizes: "shrink text" makes the text
    /// smaller (raised to sit in the middle), "enlarge symbol" makes the symbol bigger (lowered).
    fn enclosure(&self, w: &mut W, e: &wordcraft_doc::eq::Enclosure, locked: bool, props: &CharProps) {
        use wordcraft_doc::eq::EncloseStyle;
        let del = self.is_del(props);
        let tag = if del { "w:delInstrText" } else { "w:instrText" };
        let base = props.size.unwrap_or_else(|| self.doc.styles.resolve_char(None, props).size).clamp(1.0, 1638.0);
        let half = |pt: f32| (pt * 2.0).round() / 2.0;
        let (mut sym, mut txt) = (props.clone(), props.clone());
        match e.style {
            EncloseStyle::Shrink => {
                txt.size = Some(half(base * 0.7).max(1.0));
                txt.position = Some(half(base * 0.1));
            }
            EncloseStyle::Enlarge => {
                sym.size = Some(half(base * 1.5));
                sym.position = Some(-half(base * 0.2));
            }
        }
        let run = |w: &mut W, p: &CharProps, f: &dyn Fn(&mut W)| {
            w.open("w:r", &[]);
            rpr(w, p);
            f(w);
            w.close("w:r");
        };
        run(w, props, &|w| {
            if locked {
                w.empty("w:fldChar", &[("w:fldCharType", "begin"), ("w:fldLock", "1")])
            } else {
                w.empty("w:fldChar", &[("w:fldCharType", "begin")])
            }
        });
        let code = e.word_instr();
        // `eq \o\ac(` + symbol, `,`, the text, `)`.
        let open = code.find('(').map(|i| i + 1).unwrap_or(0);
        let comma = open + e.shape.symbol().len_utf8();
        let (head, rest) = code.split_at(comma.min(code.len()));
        let text = rest.get(1..rest.len().saturating_sub(1)).unwrap_or("");
        run(w, &sym, &|w| w.leaf(tag, &[("xml:space", "preserve")], &format!(" {head}")));
        run(w, props, &|w| w.leaf(tag, &[("xml:space", "preserve")], ","));
        run(w, &txt, &|w| w.leaf(tag, &[("xml:space", "preserve")], text));
        run(w, props, &|w| w.leaf(tag, &[("xml:space", "preserve")], ")"));
        run(w, props, &|w| w.empty("w:fldChar", &[("w:fldCharType", "end")]));
    }

    /// A field's begin, instruction and separate runs.
    /// A field's begin, code and separator; `form` is a legacy form field's `w:ffData`, which
    /// belongs inside the begin.
    fn field_start(&self, w: &mut W, instr: &str, locked: bool, dirty: bool, form: &str, props: &CharProps) {
        let del = self.is_del(props);
        let run = |w: &mut W, f: &dyn Fn(&mut W)| {
            w.open("w:r", &[]);
            rpr(w, props);
            f(w);
            w.close("w:r");
        };
        run(w, &|w| {
            let mut attrs: Vec<(&str, &str)> = vec![("w:fldCharType", "begin")];
            if locked {
                attrs.push(("w:fldLock", "1"));
            }
            if dirty {
                attrs.push(("w:dirty", "true"));
            }
            if form.is_empty() {
                w.empty("w:fldChar", &attrs);
            } else {
                w.open("w:fldChar", &attrs);
                w.raw(form);
                w.close("w:fldChar");
            }
        });
        let instr_text = format!(" {} ", instr.trim());
        run(w, &|w| w.leaf(if del { "w:delInstrText" } else { "w:instrText" }, &[("xml:space", "preserve")], &instr_text));
        run(w, &|w| w.empty("w:fldChar", &[("w:fldCharType", "separate")]));
    }

    /// A picture's `pic:pic` (the image in `media/{file}`), at `off` in its group (or 0, 0).
    fn pic(&mut self, w: &mut W, o: &InlineObject, file: &str, off: (f32, f32), rels: &mut PartRels) {
        let InlineObject::Image { media, w: iw, h: ih, alt, crop, float, .. } = o else { return };
        let rid = rels.add(rt::IMAGE, &format!("media/{file}"), false);
        self.used_media.insert(media.clone());
        w.open("pic:pic", &[]);
        w.open("pic:nvPicPr", &[]);
        nv_props(w, "pic:cNvPr", "0", file, alt, &float.alt, float.hidden);
        w.open("pic:cNvPicPr", &[]);
        w.empty("a:picLocks", &[("noChangeAspect", "1"), ("noChangeArrowheads", "1")]);
        w.close("pic:cNvPicPr");
        w.close("pic:nvPicPr");
        w.open("pic:blipFill", &[]);
        w.empty("a:blip", &[("r:embed", &rid)]);
        if crop.iter().any(|c| *c > 0.0) {
            let c = |v: f32| n((wordcraft_geom::finite(v).clamp(0.0, 1.0) * 100_000.0).round() as i64);
            w.empty("a:srcRect", &[("l", &c(crop[0])), ("t", &c(crop[1])), ("r", &c(crop[2])), ("b", &c(crop[3]))]);
        }
        w.open("a:stretch", &[]);
        w.empty("a:fillRect", &[]);
        w.close("a:stretch");
        w.close("pic:blipFill");
        w.open("pic:spPr", &[("bwMode", "auto")]);
        xfrm(w, off, *iw, *ih, float.spin());
        w.open("a:prstGeom", &[("prst", "rect")]);
        w.empty("a:avLst", &[]);
        w.close("a:prstGeom");
        w.close("pic:spPr");
        w.close("pic:pic");
    }

    /// A shape's or text box's `wps:wsp`, at `off` in its group (or 0, 0). In a group it carries
    /// its own `wps:cNvPr` with drawing id `id`.
    fn wsp(&mut self, w: &mut W, o: &InlineObject, id: Option<&str>, off: (f32, f32), rels: &mut PartRels, depth: usize) {
        let InlineObject::Shape { kind, w: sw, h: sh, fill, stroke, stroke_width, story, effects, float, freeform, extra } = o else { return };
        let geom = freeform.as_deref().filter(|_| *kind == ShapeKind::Freeform);
        w.open("wps:wsp", &[]);
        if let Some(id) = id {
            let name = shape_name(*kind, freeform.as_deref(), id);
            nv_props(w, "wps:cNvPr", id, &name, &float.alt.text, &float.alt, float.hidden);
        }
        if kind.is_connector() {
            // A connector's glued ends (ECMA-376 §20.1.2.2.8 `cNvCxnSpPr`).
            let ends = [("a:stCxn", extra.start), ("a:endCxn", extra.end)];
            if ends.iter().any(|(_, e)| e.is_some()) {
                w.open("wps:cNvCnPr", &[]);
                for (tag, e) in ends {
                    if let Some(e) = e {
                        w.empty(tag, &[("id", &e.id.to_string()), ("idx", &e.site.to_string())]);
                    }
                }
                w.close("wps:cNvCnPr");
            } else {
                w.empty("wps:cNvCnPr", &[]);
            }
        } else if *kind == ShapeKind::TextBox {
            w.empty("wps:cNvSpPr", &[("txBox", "1")]);
        } else {
            w.empty("wps:cNvSpPr", &[]);
        }
        w.open("wps:spPr", &[]);
        xfrm(w, off, *sw, *sh, float.spin());
        let prst = kind.ooxml_prst();
        match geom {
            Some(f) => cust_geom(w, f, *sw, *sh),
            None => {
                w.open("a:prstGeom", &[("prst", prst)]);
                match extra.corner.filter(|_| *kind == ShapeKind::RoundedRectangle) {
                    Some(c) => {
                        w.open("a:avLst", &[]);
                        let fmla = format!("val {}", c.min(wordcraft_doc::connector::MAX_CORNER));
                        w.empty("a:gd", &[("name", "adj"), ("fmla", &fmla)]);
                        w.close("a:avLst");
                    }
                    None => w.empty("a:avLst", &[]),
                }
                w.close("a:prstGeom");
            }
        }
        let alpha = geom.map_or(1.0, |f| f.alpha);
        let ink = geom.is_some_and(|f| f.is_ink());
        match fill.filter(|_| !ink && !kind.is_open()) {
            Some(c) => {
                w.open("a:solidFill", &[]);
                w.empty("a:srgbClr", &[("val", &c.hex())]);
                w.close("a:solidFill");
            }
            None => w.empty("a:noFill", &[]),
        }
        match stroke {
            Some(c) => {
                let width = emu(stroke_width.clamp(0.0, 100.0));
                let ln: &[(&str, &str)] = if ink { &[("w", width.as_str()), ("cap", "rnd")] } else { &[("w", width.as_str())] };
                w.open("a:ln", ln);
                w.open("a:solidFill", &[]);
                if alpha < 1.0 {
                    w.open("a:srgbClr", &[("val", &c.hex())]);
                    w.empty("a:alpha", &[("val", &n((alpha.clamp(0.0, 1.0) * 100_000.0).round() as i64))]);
                    w.close("a:srgbClr");
                } else {
                    w.empty("a:srgbClr", &[("val", &c.hex())]);
                }
                w.close("a:solidFill");
                if ink {
                    w.empty("a:round", &[]);
                }
                arrow_ends(w, extra);
                w.close("a:ln");
            }
            None => {
                w.open("a:ln", &[]);
                w.empty("a:noFill", &[]);
                w.close("a:ln");
            }
        }
        effect_list(w, effects);
        w.close("wps:spPr");
        // Its text, within the same bounds layout shows boxes inside boxes with. A box linked
        // from another (Create Link) shows the rest of that box's text: it names the chain and
        // its place in it instead.
        let part = (*story).and_then(|id| self.doc.parts.get(&id).filter(|p| p.kind == wordcraft_doc::PartKind::TextBox).map(|p| (id, p)));
        let link = part.and_then(|(id, _)| self.txbx_links.get(&id).copied());
        if let Some((n, seq)) = link.filter(|(_, seq)| *seq > 0) {
            w.empty("wps:linkedTxbx", &[("id", &n.to_string()), ("seq", &seq.to_string())]);
        } else if let Some((id, part)) = part
            && self.boxes.enter(id)
        {
            match link {
                Some((n, _)) => w.open("wps:txbx", &[("id", &n.to_string())]),
                None => w.open("wps:txbx", &[]),
            }
            w.open("w:txbxContent", &[]);
            let blocks = part.blocks.clone();
            self.blocks(w, &blocks, rels, false, depth + 1);
            w.close("w:txbxContent");
            w.close("wps:txbx");
            self.boxes.leave();
        }
        // Text direction and alignment, when not the defaults.
        let body = part.map(|(_, p)| p.body).unwrap_or_default();
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        if body.vert != wordcraft_doc::props::TextVert::Horz {
            attrs.push(("vert", body.vert.ooxml()));
        }
        if body.anchor != wordcraft_doc::props::VAlign::Top {
            attrs.push(("anchor", wordcraft_doc::props::anchor_ooxml(body.anchor)));
        }
        // Internal margins, when the box has its own.
        let inset = body.inset.map(|i| i.map(|v| v.min(wordcraft_doc::MAX_TEXT_BOX_INSET).to_string()));
        if let Some(inset) = &inset {
            for (n, v) in ["lIns", "tIns", "rIns", "bIns"].into_iter().zip(inset) {
                attrs.push((n, v));
            }
        }
        let warp = extra.warp.as_ref().and_then(|t| t.sanitized());
        if warp.is_none() && !body.auto_fit {
            w.empty("wps:bodyPr", &attrs);
        } else {
            w.open("wps:bodyPr", &attrs);
            if let Some(warp) = &warp {
                text_warp(w, warp);
            }
            if body.auto_fit {
                w.empty("a:spAutoFit", &[]);
            }
            w.close("wps:bodyPr");
        }
        w.close("wps:wsp");
    }

    #[allow(clippy::too_many_arguments)]
    /// A `w:drawing` holding `graphic` (an `a:graphic`) in a frame of the object's size and place.
    #[allow(clippy::too_many_arguments)]
    fn graphic_frame(&mut self, w: &mut W, float: &Float, (gw, gh): (f32, f32), docpr: &str, name: &str, alt: &str, graphic: &crate::xml::El) {
        w.open("w:drawing", &[]);
        self.drawing_open(w, float, gw, gh, docpr, name, alt);
        w.empty("wp:cNvGraphicFramePr", &[]);
        w.raw(&graphic.to_xml());
        w.close(if float.wrap == Wrap::Inline { "wp:inline" } else { "wp:anchor" });
        w.close("w:drawing");
    }

    fn drawing_open(&mut self, w: &mut W, float: &Float, iw: f32, ih: f32, docpr: &str, name: &str, alt: &str) {
        let (cx, cy) = (emu(iw.max(0.0)), emu(ih.max(0.0)));
        if float.wrap == Wrap::Inline {
            w.open("wp:inline", &[("distT", "0"), ("distB", "0"), ("distL", "0"), ("distR", "0")]);
        } else {
            let d = emu(float.dist.clamp(0.0, 1584.0));
            let (dt, db) = (emu(float.dist_top.clamp(0.0, 1584.0)), emu(float.dist_bottom.clamp(0.0, 1584.0)));
            self.z += 1;
            let behind = if float.wrap == Wrap::BehindText { "1" } else { "0" };
            w.open(
                "wp:anchor",
                &[
                    ("distT", &dt),
                    ("distB", &db),
                    ("distL", &d),
                    ("distR", &d),
                    ("simplePos", "0"),
                    ("relativeHeight", &self.z.to_string()),
                    ("behindDoc", behind),
                    ("locked", "0"),
                    ("layoutInCell", "1"),
                    ("allowOverlap", "1"),
                ],
            );
            w.empty("wp:simplePos", &[("x", "0"), ("y", "0")]);
            let rel = |a: Anchor, horiz: bool| match (a, horiz) {
                (Anchor::Margin, _) => "margin",
                (Anchor::Page, _) => "page",
                (Anchor::InsideMargin, _) => "insideMargin",
                (Anchor::OutsideMargin, _) => "outsideMargin",
                (Anchor::LeftMargin, true) => "leftMargin",
                (Anchor::RightMargin, true) => "rightMargin",
                (Anchor::Character, true) => "character",
                (Anchor::TopMargin, false) => "topMargin",
                (Anchor::BottomMargin, false) => "bottomMargin",
                (Anchor::Line, false) => "line",
                (_, true) => "column",
                (_, false) => "paragraph",
            };
            for (tag, horiz, a, rf, off) in
                [("wp:positionH", true, float.h_align, float.h_rel, float.x), ("wp:positionV", false, float.v_align, float.v_rel, float.y)]
            {
                w.open(tag, &[("relativeFrom", rel(rf, horiz))]);
                match a {
                    Some(a) => w.leaf("wp:align", &[], a.ooxml(horiz)),
                    None => w.leaf("wp:posOffset", &[], &emu(off)),
                }
                w.close(tag);
            }
        }
        w.empty("wp:extent", &[("cx", &cx), ("cy", &cy)]);
        // Word's effect extent also covers a rotated object's overhang (its rotated bounds).
        let (px, py) = float.spin_pad(iw, ih);
        let [el, et, er, eb] = float.effect_extent();
        let [el, et, er, eb] = [el + px, et + py, er + px, eb + py].map(emu);
        w.empty("wp:effectExtent", &[("l", &el), ("t", &et), ("r", &er), ("b", &eb)]);
        match float.wrap {
            Wrap::Inline => {}
            Wrap::Square => w.empty("wp:wrapSquare", &[("wrapText", float.wrap_text.ooxml())]),
            Wrap::Tight | Wrap::Through => {
                let tag = if float.wrap == Wrap::Tight { "wp:wrapTight" } else { "wp:wrapThrough" };
                w.open(tag, &[("wrapText", float.wrap_text.ooxml())]);
                // The object's own polygon, or its rectangle (the schema requires one).
                let s = wordcraft_doc::wrap::WRAP_SPACE;
                let (edited, pts) = match float.wrap_polygon.as_deref() {
                    Some(p) if p.points.len() >= 3 => (p.edited, p.points.clone()),
                    _ => (false, vec![[0, 0], [0, s], [s, s], [s, 0]]),
                };
                w.open("wp:wrapPolygon", &[("edited", if edited { "1" } else { "0" })]);
                // Closed: back to the start.
                for (i, [x, y]) in pts.iter().chain(pts.first()).enumerate() {
                    w.empty(if i == 0 { "wp:start" } else { "wp:lineTo" }, &[("x", &x.to_string()), ("y", &y.to_string())]);
                }
                w.close("wp:wrapPolygon");
                w.close(tag);
            }
            Wrap::TopAndBottom => w.empty("wp:wrapTopAndBottom", &[]),
            Wrap::BehindText | Wrap::InFrontOfText => w.empty("wp:wrapNone", &[]),
        }
        nv_props(w, "wp:docPr", docpr, name, alt, &float.alt, float.hidden);
    }

    fn table(&mut self, w: &mut W, t: &Table, rels: &mut PartRels, depth: usize) {
        let rows: Vec<_> = t.rows.iter().filter(|r| !r.cells.is_empty()).collect();
        if rows.is_empty() {
            return;
        }
        w.open("w:tbl", &[]);
        let chg = t.props.fmt_change.as_ref().map(|c| self.change_attrs(c.rev));
        tblpr(w, &t.props, chg.as_ref());
        w.open("w:tblGrid", &[]);
        let cols = t.cols().clamp(1, 63);
        for g in 0..cols {
            let gw = t.grid.get(g).copied().unwrap_or(72.0);
            w.empty("w:gridCol", &[("w:w", &twips(gw.max(0.0)))]);
        }
        w.close("w:tblGrid");
        // Controls around rows and cells (repeating sections): opens and closes balanced
        // whatever editing left, the ones still open closed at the end.
        let mut open_rows = 0u32;
        for r in rows {
            for c in &r.controls.open {
                super::sdt::open(w, c);
                open_rows += 1;
            }
            w.open("w:tr", &[]);
            let chg = r.props.fmt_change.as_ref().map(|c| self.change_attrs(c.rev));
            trpr(w, &r.props, chg.as_ref());
            let mut open_cells = 0u32;
            for c in &r.cells {
                for ctl in &c.controls.open {
                    super::sdt::open(w, ctl);
                    open_cells += 1;
                }
                w.open("w:tc", &[]);
                let chg = c.props.fmt_change.as_ref().map(|c| self.change_attrs(c.rev));
                tcpr(w, &c.props, chg.as_ref());
                self.blocks(w, &c.blocks, rels, false, depth + 1);
                w.close("w:tc");
                let n = c.controls.close.min(open_cells);
                for _ in 0..n {
                    super::sdt::close(w);
                }
                open_cells -= n;
            }
            for _ in 0..open_cells {
                super::sdt::close(w);
            }
            w.close("w:tr");
            let n = r.controls.close.min(open_rows);
            for _ in 0..n {
                super::sdt::close(w);
            }
            open_rows -= n;
        }
        for _ in 0..open_rows {
            super::sdt::close(w);
        }
        w.close("w:tbl");
    }

    /// The section's headers, plus the header holding only the watermark in each slot the
    /// pages show (first page with a title page, even pages with even/odd headers) that has no
    /// header of its own or linked from an earlier section. Sections are written in body order.
    fn watermark_headers(&mut self, s: &SectionProps) -> wordcraft_doc::section::HeaderSet {
        let mut set = s.headers;
        let linked = &mut self.linked_headers;
        if self.doc.settings.watermark.as_ref().and_then(super::watermark::text).is_some() {
            let id = self.wm_header;
            if set.default.or(linked.default).is_none() {
                set.default = Some(id);
            }
            if s.title_page && set.first.or(linked.first).is_none() {
                set.first = Some(id);
            }
            if self.doc.settings.even_odd_headers && set.even.or(linked.even).is_none() {
                set.even = Some(id);
            }
        }
        linked.default = set.default.or(linked.default);
        linked.first = set.first.or(linked.first);
        linked.even = set.even.or(linked.even);
        set
    }

    pub fn sectpr(&mut self, w: &mut W, s: &SectionProps, rels: &mut PartRels) {
        w.open("w:sectPr", &[]);
        let headers = self.watermark_headers(s);
        for (set, footer) in [(&headers, false), (&s.footers, true)] {
            for (kind, id) in [("default", set.default), ("first", set.first), ("even", set.even)] {
                let Some(id) = id else { continue };
                if let Some(rid) = self.hf_rel(id, footer, rels) {
                    w.empty(if footer { "w:footerReference" } else { "w:headerReference" }, &[("w:type", kind), ("r:id", &rid)]);
                }
            }
        }
        sectpr_body(w, s);
        // The properties before a tracked change come last (§17.13.5.32).
        if let Some(ch) = &s.fmt_change {
            let a = self.change_attrs(ch.rev);
            open_change(w, "w:sectPrChange", &a);
            w.open("w:sectPr", &[]);
            sectpr_body(w, &ch.old);
            w.close("w:sectPr");
            w.close("w:sectPrChange");
        }
        w.close("w:sectPr");
    }
}

/// `w:sectPr` content after the header and footer references.
fn sectpr_body(w: &mut W, s: &SectionProps) {
    {
        // The section's own note options come before `w:type` (ECMA-376 §17.6.17).
        for (tag, pr) in [("w:footnotePr", &s.footnote_pr), ("w:endnotePr", &s.endnote_pr)] {
            if !pr.is_empty() {
                w.open(tag, &[]);
                note_pr_inner(w, pr);
                w.close(tag);
            }
        }
        let start = match s.start {
            SectionStart::NextPage => "nextPage",
            SectionStart::Continuous => "continuous",
            SectionStart::EvenPage => "evenPage",
            SectionStart::OddPage => "oddPage",
            SectionStart::NextColumn => "nextColumn",
        };
        w.val("w:type", start);
        let (pw, ph) = (s.page_w.clamp(36.0, crate::units::MAX_LEN_PT), s.page_h.clamp(36.0, crate::units::MAX_LEN_PT));
        if s.landscape {
            w.empty("w:pgSz", &[("w:w", &twips(pw)), ("w:h", &twips(ph)), ("w:orient", "landscape")]);
        } else {
            w.empty("w:pgSz", &[("w:w", &twips(pw)), ("w:h", &twips(ph))]);
        }
        w.empty(
            "w:pgMar",
            &[
                ("w:top", &twips(s.margin_top)),
                ("w:right", &twips(s.margin_right.max(0.0))),
                ("w:bottom", &twips(s.margin_bottom)),
                ("w:left", &twips(s.margin_left.max(0.0))),
                ("w:header", &twips(s.header.max(0.0))),
                ("w:footer", &twips(s.footer.max(0.0))),
                ("w:gutter", &twips(s.gutter.max(0.0))),
            ],
        );
        if let Some(b) = &s.page_borders {
            borders(w, "w:pgBorders", b, None, &[("w:offsetFrom", "text")]);
        }
        if let Some(l) = &s.line_numbers {
            let restart = match l.restart {
                LineNumberRestart::Page => "newPage",
                LineNumberRestart::Section => "newSection",
                LineNumberRestart::Continuous => "continuous",
            };
            let mut a = vec![
                ("w:countBy", n(l.count_by.clamp(1, 100) as i64)),
                ("w:start", n(l.start.saturating_sub(1).min(32_767) as i64)),
                ("w:restart", restart.to_string()),
            ];
            if l.distance > 0.0 {
                a.push(("w:distance", twips(l.distance)));
            }
            let refs: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.empty("w:lnNumType", &refs);
        }
        if s.page_num_set || s.page_num_start.is_some() || s.page_num_format != wordcraft_doc::section::NumFormat::Decimal {
            let mut a = vec![("w:fmt", s.page_num_format.ooxml().to_string())];
            if let Some(st) = s.page_num_start {
                a.push(("w:start", st.to_string()));
            }
            let refs: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.empty("w:pgNumType", &refs);
        }
        let c = &s.columns;
        let count = c.count.clamp(1, 45);
        let mut ca = vec![("w:space", twips(c.space.max(0.0))), ("w:num", n(count as i64))];
        if c.separator {
            ca.push(("w:sep", "1".into()));
        }
        let unequal = !c.widths.is_empty() && c.widths.len() == count as usize;
        if unequal {
            ca.push(("w:equalWidth", "0".into()));
            let refs: Vec<(&str, &str)> = ca.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.open("w:cols", &refs);
            for (cw, sp) in &c.widths {
                w.empty("w:col", &[("w:w", &twips(cw.max(0.0))), ("w:space", &twips(sp.max(0.0)))]);
            }
            w.close("w:cols");
        } else {
            let refs: Vec<(&str, &str)> = ca.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.empty("w:cols", &refs);
        }
        if let Some(on) = s.form_prot {
            w.empty("w:formProt", if on { &[] } else { &[("w:val", "0")] });
        }
        match s.valign {
            wordcraft_doc::section::PageVAlign::Top => {}
            wordcraft_doc::section::PageVAlign::Center => w.val("w:vAlign", "center"),
            wordcraft_doc::section::PageVAlign::Justify => w.val("w:vAlign", "both"),
            wordcraft_doc::section::PageVAlign::Bottom => w.val("w:vAlign", "bottom"),
        }
        if s.title_page {
            w.empty("w:titlePg", &[]);
        }
        if s.text_direction.is_turned() || s.text_direction_set {
            w.val("w:textDirection", s.text_direction.ooxml());
        }
        if s.rtl {
            w.empty("w:bidi", &[]);
        }
        if let Some(g) = &s.doc_grid {
            let mut a = vec![("w:linePitch", twips(g.line_pitch()))];
            if g.kind != wordcraft_doc::section::DocGridType::Default {
                a.insert(0, ("w:type", g.kind.ooxml().to_string()));
            }
            if g.char_space != 0 {
                let cs = g.char_space.clamp(-wordcraft_doc::section::MAX_CHAR_SPACE, wordcraft_doc::section::MAX_CHAR_SPACE);
                a.push(("w:charSpace", n(cs as i64)));
            }
            let refs: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
            w.empty("w:docGrid", &refs);
        }
    }
}

/// A shape's `cNvPr`/`docPr` name: ink is "Ink <pen> <id>" (and "#<order>" when its drawing
/// order is known), which reading looks for.
fn shape_name(kind: ShapeKind, freeform: Option<&wordcraft_doc::freeform::Freeform>, id: &str) -> String {
    let f = freeform.filter(|_| kind == ShapeKind::Freeform);
    match (f.and_then(|f| f.ink), f.and_then(|f| f.order)) {
        // The drawing order rides along for Ink Replay: `Ink Pen 3 #12`.
        (Some(tool), Some(n)) => format!("Ink {} {id} #{n}", ink_name(tool)),
        (Some(tool), None) => format!("Ink {} {id}", ink_name(tool)),
        (None, _) => format!("Shape {id}"),
    }
}

fn ink_name(tool: wordcraft_doc::freeform::InkTool) -> &'static str {
    match tool {
        wordcraft_doc::freeform::InkTool::Pen => "Pen",
        wordcraft_doc::freeform::InkTool::Pencil => "Pencil",
        wordcraft_doc::freeform::InkTool::Highlighter => "Highlighter",
    }
}

/// A freeform's geometry as DrawingML custom geometry (`a:custGeom`): each path in EMUs of its
/// own coordinate space (the shape's size when it has none), from `a:moveTo` along `a:lnTo`.
fn cust_geom(w: &mut W, f: &wordcraft_doc::freeform::Freeform, sw: f32, sh: f32) {
    let (pw, ph) = (if f.w > 0.0 { f.w } else { sw.max(0.0) }, if f.h > 0.0 { f.h } else { sh.max(0.0) });
    w.open("a:custGeom", &[]);
    w.empty("a:avLst", &[]);
    w.empty("a:gdLst", &[]);
    w.empty("a:ahLst", &[]);
    w.empty("a:cxnLst", &[]);
    w.empty("a:rect", &[("l", "0"), ("t", "0"), ("r", "r"), ("b", "b")]);
    w.open("a:pathLst", &[]);
    let pt = |w: &mut W, tag: &str, [x, y]: [f32; 2]| {
        w.open(tag, &[]);
        w.empty("a:pt", &[("x", &emu(x)), ("y", &emu(y))]);
        w.close(tag);
    };
    for p in f.paths.iter().take(wordcraft_doc::freeform::MAX_PATHS) {
        let mut attrs = vec![("w", emu(pw)), ("h", emu(ph))];
        if !p.closed || f.is_ink() {
            attrs.push(("fill", "none".to_string()));
        }
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        w.open("a:path", &attrs);
        for (i, xy) in p.pts.iter().take(wordcraft_doc::freeform::MAX_POINTS).enumerate() {
            pt(w, if i == 0 { "a:moveTo" } else { "a:lnTo" }, *xy);
        }
        // A tap: a zero-length line, so it shows as a dot.
        if let (1, Some(xy)) = (p.pts.len(), p.pts.first()) {
            pt(w, "a:lnTo", *xy);
        }
        if p.closed {
            w.empty("a:close", &[]);
        }
        w.close("a:path");
    }
    w.close("a:pathLst");
    w.close("a:custGeom");
}

/// A shape's `a:effectLst` (ECMA-376 Part 1 §20.1.8.26), children in schema order.
/// A line's arrowheads (`a:headEnd`, `a:tailEnd`, ECMA-376 §20.1.8.38 / §20.1.8.57).
fn arrow_ends(w: &mut W, extra: &wordcraft_doc::connector::ShapeExtra) {
    if extra.arrow_start {
        w.empty("a:headEnd", &[("type", "triangle")]);
    }
    if extra.arrow_end {
        w.empty("a:tailEnd", &[("type", "triangle")]);
    }
}

/// A preset text warp (`a:prstTxWarp`, ECMA-376 §20.1.9.19) with its adjust values.
fn text_warp(w: &mut W, warp: &wordcraft_doc::wordart::TextWarp) {
    w.open("a:prstTxWarp", &[("prst", &warp.preset)]);
    if warp.adj.is_empty() {
        w.empty("a:avLst", &[]);
    } else {
        w.open("a:avLst", &[]);
        for (name, v) in &warp.adj {
            w.empty("a:gd", &[("name", name), ("fmla", &format!("val {v}"))]);
        }
        w.close("a:avLst");
    }
    w.close("a:prstTxWarp");
}

fn effect_list(w: &mut W, effects: &ShapeEffects) {
    let fx = effects.sanitized();
    if fx.is_empty() {
        return;
    }
    let color = |w: &mut W, c: Rgb, transparency: f32| {
        let alpha = n(((100.0 - transparency.clamp(0.0, 100.0)) * 1000.0).round() as i64);
        w.open("a:srgbClr", &[("val", &c.hex())]);
        w.empty("a:alpha", &[("val", &alpha)]);
        w.close("a:srgbClr");
    };
    w.open("a:effectLst", &[]);
    if let Some(g) = fx.glow {
        w.open("a:glow", &[("rad", &emu(g.size))]);
        color(w, g.color, g.transparency);
        w.close("a:glow");
    }
    if let Some(s) = fx.shadow {
        let dir = n((s.angle * 60_000.0).round() as i64);
        // Anchor scaling at the corner the shadow falls away from (no scaling is written, so
        // this only matters to editors that resize it).
        let (dx, dy) = s.offset();
        let algn = match (dy > 0.01, dy < -0.01, dx > 0.01, dx < -0.01) {
            (true, _, true, _) => "tl",
            (true, _, _, true) => "tr",
            (true, _, _, _) => "t",
            (_, true, true, _) => "bl",
            (_, true, _, true) => "br",
            (_, true, _, _) => "b",
            (_, _, true, _) => "l",
            (_, _, _, true) => "r",
            _ => "ctr",
        };
        w.open(
            "a:outerShdw",
            &[
                ("blurRad", &emu(s.blur)),
                ("dist", &emu(s.distance)),
                ("dir", &dir),
                ("algn", algn),
                ("rotWithShape", if s.rot_with_shape { "1" } else { "0" }),
            ],
        );
        color(w, s.color, s.transparency);
        w.close("a:outerShdw");
    }
    if let Some(r) = fx.soft_edge {
        w.empty("a:softEdge", &[("rad", &emu(r))]);
    }
    w.close("a:effectLst");
}

fn xfrm(w: &mut W, (x, y): (f32, f32), cw: f32, ch: f32, spin: wordcraft_geom::Spin) {
    w.open("a:xfrm", &spin_attrs(spin).iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
    w.empty("a:off", &[("x", &emu(x)), ("y", &emu(y))]);
    w.empty("a:ext", &[("cx", &emu(cw.max(0.0))), ("cy", &emu(ch.max(0.0)))]);
    w.close("a:xfrm");
}

/// An `a:xfrm` holding only a rotation and flips.
fn xfrm_spin_only(w: &mut W, spin: wordcraft_geom::Spin) {
    w.empty("a:xfrm", &spin_attrs(spin).iter().map(|(k, v)| (*k, v.as_str())).collect::<Vec<_>>());
}

/// The `a:xfrm` attributes of a rotation and flips (ECMA-376 §20.1.7.6: `rot` in 60000ths of a
/// degree, clockwise), none for an unturned object.
pub(super) fn spin_attrs(spin: wordcraft_geom::Spin) -> Vec<(&'static str, String)> {
    let mut a = Vec::new();
    let rot = (wordcraft_geom::normalize_degrees(spin.deg) as f64 * 60_000.0).round() as i64 % 21_600_000;
    if rot != 0 {
        a.push(("rot", rot.to_string()));
    }
    if spin.flip_h {
        a.push(("flipH", "1".to_string()));
    }
    if spin.flip_v {
        a.push(("flipV", "1".to_string()));
    }
    a
}

/// One `w:r` with `props`, holding what `f` writes (a field character or code).
fn field_run(w: &mut W, props: &CharProps, f: &dyn Fn(&mut W)) {
    w.open("w:r", &[]);
    rpr(w, props);
    f(w);
    w.close("w:r");
}

/// Word's Table of Contents content control, which gives the TOC its frame and Update Table.
const TOC_SDT_OPEN: &str =
    r#"<w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/><w:docPartUnique/></w:docPartObj></w:sdtPr><w:sdtContent>"#;

fn is_toc(instr: &str) -> bool {
    instr.split_whitespace().next().is_some_and(|k| k.eq_ignore_ascii_case("TOC"))
}

/// The engine's TOC: a paragraph holding an empty `TOC` field, then its TOC-styled entries.
/// Returns (heading, last entry) so the field can be written spanning the entries, as Word does.
fn toc_span(bl: &Blocks) -> Option<(usize, usize)> {
    let start = bl.iter().position(|b| {
        b.as_para().is_some_and(|p| {
            p.section.is_none()
                && p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, result, .. } if is_toc(instr) && result.is_empty()))
        })
    })?;
    let entries = bl[start + 1..]
        .iter()
        .take_while(|b| {
            b.as_para().is_some_and(|p| p.section.is_none() && p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
        })
        .count();
    (entries > 0).then_some((start, start + entries))
}

/// A drawing's `wp:docPr` (or a group member's `pic:cNvPr`/`wps:cNvPr`, ECMA-376
/// `CT_NonVisualDrawingProps`) with its alt text `descr`, its title, and the decorative mark as
/// the `adec:decorative` extension ([MS-ODRAWXML]). A description that is only the title (read
/// from a file with no description) is not written twice.
pub(super) fn nv_props(w: &mut W, tag: &str, id: &str, name: &str, descr: &str, alt: &AltText, hidden: bool) {
    let mut attrs = vec![("id", id), ("name", name)];
    if descr != alt.title || descr.is_empty() {
        attrs.push(("descr", descr));
    }
    if !alt.title.is_empty() {
        attrs.push(("title", alt.title.as_str()));
    }
    if hidden {
        attrs.push(("hidden", "1"));
    }
    if !alt.decorative {
        w.empty(tag, &attrs);
        return;
    }
    w.open(tag, &attrs);
    w.open("a:extLst", &[]);
    w.open("a:ext", &[("uri", crate::xml::DECORATIVE_EXT)]);
    w.empty("adec:decorative", &[("xmlns:adec", crate::xml::DECORATIVE_NS), ("val", "1")]);
    w.close("a:ext");
    w.close("a:extLst");
    w.close(tag);
}

/// A DrawingML `a:solidFill` in `c`, or `a:noFill`.
fn solid_fill(w: &mut W, c: Option<wordcraft_doc::props::Rgb>) {
    match c {
        Some(c) => {
            w.open("a:solidFill", &[]);
            w.empty("a:srgbClr", &[("val", &c.hex())]);
            w.close("a:solidFill");
        }
        None => w.empty("a:noFill", &[]),
    }
}

/// The children of a `w:footnotePr` / `w:endnotePr` in schema order (`w:pos`, `w:numFmt`,
/// `w:numStart`, `w:numRestart`; ECMA-376 §17.11.11), for the fields that are set.
pub(super) fn note_pr_inner(w: &mut W, pr: &NoteProps) {
    if let Some(p) = pr.pos {
        w.val("w:pos", p.ooxml());
    }
    if let Some(f) = pr.num_fmt {
        w.val("w:numFmt", f.ooxml());
    }
    if let Some(n) = pr.num_start {
        w.val("w:numStart", &n.clamp(1, wordcraft_doc::section::MAX_NOTE_START).to_string());
    }
    if let Some(r) = pr.num_restart {
        w.val("w:numRestart", r.ooxml());
    }
}

/// The character style for comment reference marks: the document's "annotation reference" style
/// (Word's built-in name; localized documents give it another id, e.g. Polish
/// `Odwoaniedokomentarza`), else `CommentReference`.
fn comment_reference_style(doc: &wordcraft_doc::Document) -> &str {
    doc.styles
        .styles
        .iter()
        .find(|s| s.kind == wordcraft_doc::styles::StyleKind::Character && s.name.eq_ignore_ascii_case("annotation reference"))
        .map_or("CommentReference", |s| s.id.as_str())
}
