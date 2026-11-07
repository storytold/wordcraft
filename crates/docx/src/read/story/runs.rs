use super::{
    BreakClear, CharProps, El, InlineObject, Item, MAX_FORM, MAX_INSTR, NoteKind, PB, PTabAlign, PTabBase, Phase, Reader, Rels, RevisionKind, RunCtx,
    StoryCtx, TextColor, children_of_choice, clean_text, int, measure, on_off_attr, placeholder_of, sink,
};

impl Reader<'_> {
    pub(super) fn read_inline_children(&mut self, sc: &mut StoryCtx, pb: &mut PB, e: &El, rels: &Rels, ctx: &RunCtx, depth: usize) {
        if depth > crate::xml::MAX_DEPTH {
            return;
        }
        for k in e.els() {
            self.read_inline(sc, pb, k, rels, ctx, depth);
        }
    }

    fn read_inline(&mut self, sc: &mut StoryCtx, pb: &mut PB, k: &El, rels: &Rels, ctx: &RunCtx, depth: usize) {
        match k.name.as_str() {
            "w:r" => self.read_run(sc, pb, k, rels, ctx, depth),
            "w:hyperlink" => {
                let mut c = ctx.clone();
                let url = k.attr("r:id").and_then(|id| rels.by_id(id)).map(|r| r.target.trim().to_string()).filter(|u| !u.is_empty());
                let anchor = k.attr("w:anchor").filter(|a| !a.is_empty());
                c.link = match (url, anchor) {
                    (Some(u), Some(a)) => Some(format!("{u}#{a}")),
                    (Some(u), None) => Some(u),
                    (None, Some(a)) => Some(format!("#{a}")),
                    (None, None) => ctx.link.clone(),
                };
                self.read_inline_children(sc, pb, k, rels, &c, depth + 1);
            }
            "w:ins" | "w:moveTo" => {
                let mut c = ctx.clone();
                c.ins = Some(if k.name == "w:moveTo" { self.move_revision(true, k, false) } else { self.revision(RevisionKind::Insert, k) });
                self.read_inline_children(sc, pb, k, rels, &c, depth + 1);
            }
            "w:del" | "w:moveFrom" => {
                let mut c = ctx.clone();
                c.del = Some(if k.name == "w:moveFrom" { self.move_revision(false, k, false) } else { self.revision(RevisionKind::Delete, k) });
                self.read_inline_children(sc, pb, k, rels, &c, depth + 1);
            }
            "w:moveFromRangeStart" | "w:moveFromRangeEnd" | "w:moveToRangeStart" | "w:moveToRangeEnd" => self.move_range(k),
            "w:fldSimple" => {
                let instr = k.attr("w:instr").unwrap_or("").to_string();
                let props = k.child("w:r").and_then(|r| r.child("w:rPr")).map(|p| self.run_props(p, ctx)).unwrap_or_else(|| self.run_props_none(ctx));
                self.begin_field(sc, props, instr, on_off_attr(k, "w:fldLock"), on_off_attr(k, "w:dirty"), String::new());
                self.separate_field(sc, pb);
                self.read_inline_children(sc, pb, k, rels, ctx, depth + 1);
                self.end_field(sc, pb);
            }
            "w:bookmarkStart" | "w:bookmarkEnd" | "w:commentRangeStart" | "w:commentRangeEnd" | "w:permStart" | "w:permEnd" => {
                if let Some(o) = self.marker(k) {
                    let props = self.run_props_none(ctx);
                    // Markers go straight into the paragraph (never into a field's cached result).
                    pb.push_obj(o, &props);
                }
            }
            "w:sdt" => {
                let Some(c) = k.child("w:sdtContent") else { return };
                let mut ctl = crate::read::sdt::sdt_pr(k.child("w:sdtPr"), false);
                ctl.end_pr_xml = crate::read::sdt::sdt_end_pr(k.child("w:sdtEndPr"));
                ctl.run_props = k.child("w:sdtPr").and_then(|pr| pr.child("w:rPr")).map(|r| self.pc.rpr(r));
                // The markers take the formatting of the content's first run (typing at the
                // start of the content continues it).
                let props = c.find("w:rPr").map(|p| self.run_props(p, ctx)).unwrap_or_else(|| self.run_props_none(ctx));
                let (at, k0) = (pb.text.len(), pb.objects.len());
                pb.push_obj(InlineObject::ControlStart { control: Box::new(ctl) }, &props);
                self.read_inline_children(sc, pb, c, rels, ctx, depth + 1);
                if let Some(InlineObject::ControlStart { control }) = pb.objects.get_mut(k0)
                    && control.showing_placeholder
                {
                    let shown: String = pb.text.get(at..).unwrap_or("").chars().filter(|c| *c != wordcraft_doc::para::OBJ).collect();
                    control.placeholder_text = placeholder_of(&shown);
                }
                let end_props = pb.runs.last().map(|r| r.props.clone()).unwrap_or(props);
                pb.push_obj(InlineObject::ControlEnd, &end_props);
            }
            "w:smartTag" | "w:customXml" | "w:dir" | "w:bdo" => self.read_inline_children(sc, pb, k, rels, ctx, depth + 1),
            "m:oMath" => {
                let mut props = self.run_props_none(ctx);
                let mut math = crate::read::math::read_omath(k, Default::default());
                if let Some(c) = crate::read::math::lift_color(&mut math) {
                    props.color = Some(TextColor::Rgb(c));
                }
                let linear = wordcraft_doc::math::to_linear(&math.nodes);
                self.emit_obj(sc, pb, InlineObject::Equation { linear, display: false, math }, &props);
            }
            "m:oMathPara" => {
                let props = self.run_props_none(ctx);
                let jc = crate::read::math::para_jc(k);
                for m in k.children("m:oMath") {
                    let mut math = crate::read::math::read_omath(m, jc);
                    let mut props = props.clone();
                    if let Some(c) = crate::read::math::lift_color(&mut math) {
                        props.color = Some(TextColor::Rgb(c));
                    }
                    let linear = wordcraft_doc::math::to_linear(&math.nodes);
                    self.emit_obj(sc, pb, InlineObject::Equation { linear, display: true, math }, &props);
                }
            }
            "mc:AlternateContent" => {
                if let Some(o) = self.read_ink_alternate(sc, k, rels).or_else(|| self.read_chartex_alternate(sc, k, rels)) {
                    let props = self.run_props_none(ctx);
                    self.emit_obj(sc, pb, o, &props);
                } else if let Some(c) = children_of_choice(k) {
                    self.read_inline_children(sc, pb, c, rels, ctx, depth + 1);
                }
            }
            // Some generators write a break directly under the paragraph instead of inside a run.
            "w:br" | "w:cr" => {
                let props = self.run_props_none(ctx);
                self.read_run_child(sc, pb, k, (&props, rels), &mut None, depth + 1);
            }
            _ => {}
        }
    }

    fn run_props(&mut self, rpr: &El, ctx: &RunCtx) -> CharProps {
        let mut p = self.pc.rpr(rpr);
        p.link = ctx.link.clone();
        p.ins = ctx.ins;
        p.del = ctx.del;
        p.fmt_change = self.rpr_change(rpr);
        p
    }

    fn run_props_none(&self, ctx: &RunCtx) -> CharProps {
        CharProps { link: ctx.link.clone(), ins: ctx.ins, del: ctx.del, ..Default::default() }
    }

    fn read_run(&mut self, sc: &mut StoryCtx, pb: &mut PB, r: &El, rels: &Rels, ctx: &RunCtx, depth: usize) {
        let props = match r.child("w:rPr") {
            Some(p) => self.run_props(p, ctx),
            None => self.run_props_none(ctx),
        };
        let mut note: Option<(NoteKind, u32, Option<String>)> = None;
        for k in r.els() {
            self.read_run_child(sc, pb, k, (&props, rels), &mut note, depth + 1);
        }
        if let Some((kind, id, custom)) = note.take() {
            self.emit_obj(sc, pb, InlineObject::NoteRef { kind, id, custom: custom.unwrap_or_default() }, &props);
        }
    }

    fn read_run_child(
        &mut self,
        sc: &mut StoryCtx,
        pb: &mut PB,
        k: &El,
        (props, rels): (&CharProps, &Rels),
        note: &mut Option<(NoteKind, u32, Option<String>)>,
        depth: usize,
    ) {
        if depth > crate::xml::MAX_DEPTH {
            return;
        }
        // A custom footnote mark: the text right after the reference is the mark.
        if let Some((_, _, Some(custom))) = note.as_mut()
            && k.name == "w:t"
        {
            custom.push_str(&clean_text(&k.text()));
            return;
        }
        if k.name != "w:rPr"
            && let Some((kind, id, custom)) = note.take()
        {
            self.emit_obj(sc, pb, InlineObject::NoteRef { kind, id, custom: custom.unwrap_or_default() }, props);
        }
        match k.name.as_str() {
            "w:t" | "w:delText" => {
                let t = clean_text(&k.text());
                self.emit_text(sc, pb, &t, props);
            }
            "w:instrText" | "w:delInstrText" => {
                if let Some(f) = sc.fields.last_mut()
                    && f.phase == Phase::Instr
                    && !f.spilled
                    && f.instr.len() < MAX_INSTR
                {
                    if f.sizes.len() < 64 {
                        f.sizes.push((f.instr.len(), props.size));
                    }
                    f.instr.push_str(&k.text());
                }
            }
            "w:tab" => self.emit_text(sc, pb, "\t", props),
            // Alignment and base are required (ECMA-376 §17.3.3.23); without them it stays a plain tab.
            "w:ptab" => match (k.attr("w:alignment").and_then(PTabAlign::from_ooxml), k.attr("w:relativeTo").and_then(PTabBase::from_ooxml)) {
                (Some(align), Some(relative_to)) => {
                    let leader = crate::read::props::tab_leader(k.attr("w:leader").unwrap_or("none"));
                    self.emit_obj(sc, pb, InlineObject::PositionalTab { align, relative_to, leader }, props)
                }
                _ => self.emit_text(sc, pb, "\t", props),
            },
            "w:br" => {
                if sc.fields.iter().any(|f| f.phase == Phase::Instr) {
                    return;
                }
                let ty = k.attr("w:type").map(|t| t.trim().to_ascii_lowercase());
                let c = match ty.as_deref() {
                    Some("page") => "\u{000C}",
                    Some("column") => "\u{000E}",
                    _ => "\n",
                };
                if c == "\n" {
                    let mut break_props = props.clone();
                    break_props.break_clear = k.attr("w:clear").and_then(BreakClear::from_ooxml);
                    self.emit_text(sc, pb, c, &break_props);
                } else {
                    self.emit_text(sc, pb, c, props);
                }
            }
            "w:cr" if !sc.fields.iter().any(|f| f.phase == Phase::Instr) => {
                self.emit_text(sc, pb, "\n", props);
            }
            "w:cr" => {}
            "w:noBreakHyphen" => self.emit_text(sc, pb, "\u{2011}", props),
            "w:softHyphen" => self.emit_text(sc, pb, "\u{00AD}", props),
            "w:sym" => {
                let h_str = k.attr("w:char").map(|h| {
                    let s = h.trim();
                    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
                    let s = s.strip_prefix("U+").or_else(|| s.strip_prefix("u+")).unwrap_or(s);
                    s.strip_prefix("#x").or_else(|| s.strip_prefix("#X")).unwrap_or(s)
                });
                let ch = h_str.and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32).filter(|c| *c as u32 >= 0x20);
                if let Some(ch) = ch {
                    let mut p = props.clone();
                    // The symbol's own font, kept apart from the run's (ECMA-376 §17.3.3.30).
                    if let Some(f) = k.attr("w:font").filter(|f| !f.trim().is_empty()) {
                        p.sym_font = Some(f.to_string());
                    }
                    self.emit_text(sc, pb, &ch.to_string(), &p);
                }
            }
            "w:fldChar" => match k.attr("w:fldCharType") {
                Some("begin") => {
                    let form = k.child("w:ffData").map(|f| f.to_xml()).filter(|x| x.len() <= MAX_FORM).unwrap_or_default();
                    self.begin_field(sc, props.clone(), String::new(), on_off_attr(k, "w:fldLock"), on_off_attr(k, "w:dirty"), form)
                }
                Some("separate") => self.separate_field(sc, pb),
                Some("end") => self.end_field(sc, pb),
                _ => {}
            },
            "w:drawing" => {
                if let Some(o) = self.read_drawing(sc, k, rels) {
                    self.emit_obj(sc, pb, o, props);
                }
            }
            "w:pict" => {
                if let Some(o) = self.read_signature_line(sc, k, rels) {
                    self.emit_obj(sc, pb, o, props);
                } else {
                    for o in self.read_vml_all(sc, k, rels) {
                        self.emit_obj(sc, pb, o, props);
                    }
                }
            }
            "w:object" => {
                if let Some(o) = self.read_object(sc, k, rels) {
                    self.emit_obj(sc, pb, o, props);
                }
            }
            "w:contentPart" => {
                if let Some(o) = self.read_ink_content_part(k, rels) {
                    self.emit_obj(sc, pb, o, props);
                }
            }
            "w:footnoteReference" | "w:endnoteReference" => {
                let foot = k.local() == "footnoteReference";
                let fid = k.attr("w:id").and_then(int).unwrap_or(i64::MIN);
                let map = if foot { &self.footnotes } else { &self.endnotes };
                if let Some(id) = map.get(&fid).copied() {
                    let kind = if foot { NoteKind::Footnote } else { NoteKind::Endnote };
                    let custom = on_off_attr(k, "w:customMarkFollows").then(String::new);
                    *note = Some((kind, id, custom));
                }
            }
            // The note's own number at the start of its text: a reference to the note being read.
            "w:footnoteRef" | "w:endnoteRef" => {
                let kind = if k.local() == "footnoteRef" { NoteKind::Footnote } else { NoteKind::Endnote };
                if let Some((nk, id)) = self.current_note
                    && nk == kind
                {
                    *note = Some((kind, id, None));
                }
            }
            // The lines of a note separator (only there).
            "w:separator" | "w:continuationSeparator" if self.in_note_sep => {
                self.emit_obj(sc, pb, InlineObject::note_separator(k.name == "w:continuationSeparator"), props);
            }
            "w:commentReference" => {
                if let Some(cid) = k.attr("w:id").and_then(|i| self.comment_id(i))
                    && !self.comments_ended.contains(&cid)
                {
                    self.comments_ended.insert(cid);
                    pb.push_obj(InlineObject::CommentEnd { id: cid }, props);
                }
            }
            "w:pgNum" => self.emit_obj(
                sc,
                pb,
                InlineObject::Field { instr: "PAGE".into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() },
                props,
            ),
            "w:ruby" => self.read_ruby(sc, pb, k, props),
            "mc:AlternateContent" => {
                if let Some(o) = self.read_ink_alternate(sc, k, rels).or_else(|| self.read_chartex_alternate(sc, k, rels)) {
                    self.emit_obj(sc, pb, o, props);
                } else if let Some(c) = children_of_choice(k) {
                    for x in c.els() {
                        self.read_run_child(sc, pb, x, (props, rels), note, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    /// `w:ruby` (ECMA-376 §17.3.3.25): ruby text (`w:rt`) over base text (`w:rubyBase`), kept as
    /// one [`InlineObject::Ruby`] in the base text's formatting (the first base run's).
    fn read_ruby(&mut self, sc: &mut StoryCtx, pb: &mut PB, k: &El, props: &CharProps) {
        const MAX: usize = 4096;
        // The text of a part's runs and the formatting of its first run.
        let part = |name: &str| -> (String, Option<CharProps>) {
            let mut text = String::new();
            let mut first = None;
            for r in k.child(name).map(|e| e.children("w:r").collect::<Vec<_>>()).unwrap_or_default() {
                if first.is_none() {
                    first = Some(r.child("w:rPr").map(|p| self.pc.rpr(p)).unwrap_or_default());
                }
                for t in r.els().filter(|t| matches!(t.name.as_str(), "w:t" | "w:delText")) {
                    if text.len() < MAX {
                        text.push_str(&clean_text(&t.text()).replace('\t', " "));
                    }
                }
            }
            (text.chars().take(MAX).collect(), first)
        };
        let (base, base_props) = part("w:rubyBase");
        let (ruby, ruby_props) = part("w:rt");
        let mut bp = base_props.unwrap_or_default();
        (bp.link, bp.ins, bp.del) = (props.link.clone(), props.ins, props.del);
        if base.is_empty() {
            return;
        }
        if ruby.is_empty() {
            self.emit_text(sc, pb, &base, &bp);
            return;
        }
        let pr = k.child("w:rubyPr");
        let val = |n: &str| pr.and_then(|p| p.child(n)).and_then(|e| e.attr("w:val"));
        let pt = |n: &str| val(n).and_then(|v| measure(v, 2.0)).filter(|v| v.is_finite() && *v >= 0.0);
        let mut rp = ruby_props.unwrap_or_default();
        let base_size = pt("w:hpsBaseText").filter(|v| *v > 0.0).or(bp.size).unwrap_or(0.0).min(1584.0);
        let size = pt("w:hps").filter(|v| *v > 0.0).or(rp.size).unwrap_or(if base_size > 0.0 { base_size / 2.0 } else { 5.0 }).clamp(1.0, 1584.0);
        let raise = pt("w:hpsRaise").unwrap_or_else(|| wordcraft_doc::para::ruby_raise(0.0, base_size.max(size), size)).min(1584.0);
        // The ruby's size is `size`; its run keeps what it sets beyond the base text's formatting
        // (writers repeat the base's).
        (rp.size, rp.size_cs) = (None, None);
        let rp = rp.minus(&bp);
        let obj = InlineObject::Ruby {
            base,
            ruby,
            align: val("w:rubyAlign").and_then(wordcraft_doc::para::RubyAlign::from_ooxml).unwrap_or_default(),
            size,
            raise,
            base_size,
            lang: val("w:lid").map(|l| l.chars().take(32).collect()).unwrap_or_default(),
            props: Box::new(rp),
        };
        self.emit_obj(sc, pb, obj, &bp);
    }

    pub(super) fn emit_text(&mut self, sc: &mut StoryCtx, pb: &mut PB, s: &str, props: &CharProps) {
        match sink(sc) {
            Some(f) => {
                if f.phase == Phase::Result && f.buf.len() < 100_000 {
                    f.buf.push((Item::Text(s.to_string()), props.clone()));
                }
            }
            _ => pb.push_text(s, props),
        }
    }

    pub(super) fn emit_obj(&mut self, sc: &mut StoryCtx, pb: &mut PB, o: InlineObject, props: &CharProps) {
        match sink(sc) {
            Some(f) => {
                if f.phase == Phase::Result && f.buf.len() < 100_000 {
                    f.buf.push((Item::Obj(Box::new(o)), props.clone()));
                }
            }
            _ => pb.push_obj(o, props),
        }
    }
}
