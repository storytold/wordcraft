use super::*;

impl Writer<'_> {
    pub(super) fn para(&mut self, w: &mut W, p: &Paragraph, rels: &mut PartRels, top: bool, depth: usize) {
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
                // The letter's own font goes on its run (`w:rFonts`).
                if let Some(f) = p.props.drop_cap_font.as_ref().filter(|f| !f.trim().is_empty()) {
                    for r in &mut head.runs {
                        r.props.font = Some(f.clone());
                    }
                }
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

    fn write_mark_rpr(&mut self, w: &mut W, mark: &CharProps, mark_revs: &[(u32, &str)]) {
        w.open("w:rPr", &[]);
        for (idx, tag) in mark_revs {
            let (id, author, date) = self.rev_attrs(*idx);
            if date.is_empty() {
                w.empty(tag, &[("w:id", &id), ("w:author", &author)]);
            } else {
                w.empty(tag, &[("w:id", &id), ("w:author", &author), ("w:date", &date)]);
            }
        }
        rpr_inner(w, mark);
        if let Some(ch) = mark.fmt_change.as_deref() {
            self.rpr_change(w, ch);
        }
        w.close("w:rPr");
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
        let has_mark = crate::write::props::has_rpr(&p.mark) || !mark_revs.is_empty() || p.mark.fmt_change.is_some();
        if !pp.is_empty() || has_mark || section.is_some() {
            w.open("w:pPr", &[]);
            let num_change = pp.num_change.as_ref().map(|c| {
                let mut a = self.change_attrs(c.rev);
                a.push(("w:original", c.original.clone()));
                a
            });
            ppr_inner(w, &pp, framed, num_change.as_ref());
            if has_mark {
                self.write_mark_rpr(w, &p.mark, &mark_revs);
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
        // Runs of mixed direction are split so Word orders them as WordCraft does (#1543).
        let rtl_para = self.doc.styles.resolve_para(&p.props).bidi;
        let styles = &self.doc.styles;
        let para_style = p.props.style.as_deref();
        let para_lang = crate::write::bidi::para_inherits_lang_bidi(styles, para_style);
        let spans = crate::write::bidi::direction_spans(&p.text, rtl_para, p.run_ranges(), |c| {
            para_lang || crate::write::bidi::char_style_has_lang_bidi(styles, c)
        });
        for (range, props) in &spans {
            let props: &CharProps = props;
            let range = range.clone();
            let base = range.start;
            let Some(text) = p.text.get(range) else { continue };
            let sym_font = props.sym_font.as_deref().filter(|f| !f.trim().is_empty());
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
                    // `w:char` is a two-byte code (ST_ShortHexNumber) from U+0020, as the reader
                    // takes it; anything else stays text.
                    c if (0x20..=0xFFFF).contains(&u32::from(c)) => sym_font.map(|f| Piece::Sym(f, c)),
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
                        InlineObject::ControlStart { control } => crate::write::sdt::open(w, control),
                        _ => crate::write::sdt::close(w),
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
                            Piece::Sym(font, c) => w.empty("w:sym", &[("w:font", font), ("w:char", &format!("{:04X}", u32::from(*c)))]),
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
    pub(super) fn is_del(&self, props: &CharProps) -> bool {
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

    pub(super) fn rev_attrs(&mut self, idx: u32) -> (String, String, String) {
        self.next_rev_id += 1;
        let r = self.doc.revisions.get(idx as usize);
        let author = crate::write::annotation_author(self.doc, r.map_or("", |r| r.author.as_str())).to_string();
        // Remove date and time (§17.15.1.69): tracked changes are saved without `w:date`.
        let date = if self.doc.settings.remove_date_time { String::new() } else { r.map(|r| r.date.clone()).unwrap_or_default() };
        (self.next_rev_id.to_string(), author, date)
    }

    /// `w:id`, `w:author` and `w:date` of a tracked change by revision `idx`.
    pub(super) fn change_attrs(&mut self, idx: u32) -> ChangeAttrs {
        let (id, author, date) = self.rev_attrs(idx);
        let mut a = vec![("w:id", id), ("w:author", author)];
        if !date.is_empty() {
            a.push(("w:date", date));
        }
        a
    }

    /// A run's `w:rPr`, with its tracked formatting change last (§17.13.5.31).
    pub(super) fn rpr(&mut self, w: &mut W, c: &CharProps) {
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

    pub(super) fn rev_open(&mut self, w: &mut W, props: &CharProps) {
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

    pub(super) fn rev_close(&mut self, w: &mut W, props: &CharProps) {
        for (idx, tag) in [(props.del, "w:del"), (props.ins, "w:ins")] {
            if let Some(i) = idx
                && self.rev_kind(idx) != Some(RevisionKind::Format)
            {
                w.close(self.rev_tag(i, tag));
            }
        }
    }
}
