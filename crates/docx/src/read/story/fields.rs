use super::{CharProps, FieldState, InlineObject, Item, MAX_FIELD_DEPTH, MAX_INSTR, PB, Phase, Reader, StoryCtx};

/// Whether a piece of a field (`a`) can go in the field's code XML: it is in the same tracked
/// insertion and deletion as the field (`b`), or none, and has no tracked formatting change. The
/// field's own revision is written around its runs, so its code's XML holds none.
pub(super) fn same_rev(a: &CharProps, b: &CharProps) -> bool {
    (a.ins, a.del) == (b.ins, b.del) && a.fmt_change.is_none()
}

/// One run of a field's code or result, as XML: text with tabs and breaks.
pub(super) fn run_xml(w: &mut crate::xml::W, props: &CharProps, tag: &str, s: &str) {
    if s.is_empty() {
        return;
    }
    w.open("w:r", &[]);
    crate::write::props::rpr(w, props);
    let mut buf = String::new();
    let flush = |w: &mut crate::xml::W, buf: &mut String| {
        if !buf.is_empty() {
            w.leaf(tag, &[("xml:space", "preserve")], buf);
            buf.clear();
        }
    };
    for c in s.chars() {
        match c {
            '\t' if tag == "w:t" => {
                flush(w, &mut buf);
                w.empty("w:tab", &[]);
            }
            '\n' | '\r' if tag == "w:t" => {
                flush(w, &mut buf);
                w.empty("w:br", &[]);
            }
            '\u{000C}' | '\u{000E}' if tag == "w:t" => {
                flush(w, &mut buf);
                w.empty("w:br", &[("w:type", if c == '\u{000C}' { "page" } else { "column" })]);
            }
            c if (c as u32) < 0x20 && c != '\t' || c == wordcraft_doc::para::OBJ => {}
            c => buf.push(c),
        }
    }
    flush(w, &mut buf);
    w.close("w:r");
}

/// A field with a field nested in its code, as run XML (ECMA-376 §17.16.18): begin, code
/// (nested fields complete), separate and cached result, end.
fn field_xml(f: &FieldState) -> String {
    let mut w = crate::xml::W::default();
    // A begin keeps its `w:fldLock` and `w:dirty` (#877); separate and end carry neither.
    let fld = |w: &mut crate::xml::W, kind: &str, begin: bool, form: &str| {
        w.open("w:r", &[]);
        crate::write::props::rpr(w, &f.props);
        let mut attrs: Vec<(&str, &str)> = vec![("w:fldCharType", kind)];
        if begin && f.locked {
            attrs.push(("w:fldLock", "1"));
        }
        if begin && f.dirty {
            attrs.push(("w:dirty", "true"));
        }
        let attrs = attrs.as_slice();
        if form.is_empty() {
            w.empty("w:fldChar", attrs);
        } else {
            w.open("w:fldChar", attrs);
            w.raw(form);
            w.close("w:fldChar");
        }
        w.close("w:r");
    };
    fld(&mut w, "begin", true, &f.form);
    if f.code_xml.is_empty() {
        run_xml(&mut w, &f.props, "w:instrText", &f.instr);
    } else {
        w.raw(&f.code_xml);
    }
    if f.phase == Phase::Result {
        fld(&mut w, "separate", false, "");
        for (it, p) in &f.buf {
            match it {
                Item::Text(t) => run_xml(&mut w, p, "w:t", t),
                Item::Obj(o) => run_xml(&mut w, p, "w:t", o.plain_text()),
            }
        }
    }
    fld(&mut w, "end", false, "");
    w.s
}

/// Enclosed characters (`eq \o\ac(○,字)`), with their style from the code's font sizes: Word
/// enlarges the symbol (bigger than the text, which keeps the field's size) or shrinks the text
/// (smaller than the symbol).
fn enclosure(f: &FieldState) -> Option<wordcraft_doc::eq::Enclosure> {
    use wordcraft_doc::eq::{EncloseShape, EncloseStyle, Enclosure};
    let mut e = Enclosure::parse(&f.instr)?;
    let size_at = |at: usize| f.sizes.iter().rev().find(|(start, _)| *start <= at).and_then(|(_, s)| *s);
    let sym = f.instr.char_indices().find(|(_, c)| EncloseShape::from_symbol(&c.to_string()) == Some(e.shape)).map(|(i, _)| i)?;
    let txt = e.text.chars().next().and_then(|c| f.instr.get(sym..)?.find(c).map(|i| i + sym)).unwrap_or(sym);
    let base = f.props.size;
    e.style = match (size_at(sym), size_at(txt)) {
        (Some(s), Some(t)) if s > t + 0.01 && base.is_none_or(|b| t >= b - 0.01) => EncloseStyle::Enlarge,
        (Some(s), None) if base.is_none_or(|b| s > b + 0.01) => EncloseStyle::Enlarge,
        _ => EncloseStyle::Shrink,
    };
    Some(e)
}

/// Fields kept as ranges: those whose result another program writes, with formatting and often
/// several paragraphs (TOCs and citation managers' `ADDIN` fields).
fn is_range_instr(instr: &str) -> bool {
    crate::read::is_toc(instr) || instr.trim_start().get(..5).is_some_and(|k| k.eq_ignore_ascii_case("ADDIN"))
}

/// Where a `HYPERLINK` field goes, as a `w:hyperlink` reads: `url`, `url#bookmark` (`\l`) or
/// `#bookmark`. None for any other field, or one with no target.
fn hyperlink_target(instr: &str) -> Option<String> {
    // Only a `HYPERLINK` code is split up (a field's code may run to megabytes).
    if !instr.split_whitespace().next().is_some_and(|k| k.eq_ignore_ascii_case("HYPERLINK")) {
        return None;
    }
    let args = field_args(instr);
    let mut args = args.iter();
    if !args.next().is_some_and(|(k, quoted)| !quoted && k.eq_ignore_ascii_case("HYPERLINK")) {
        return None;
    }
    let (mut url, mut anchor) = (None, None);
    while let Some((a, quoted)) = args.next() {
        if *quoted || !a.starts_with('\\') {
            url = url.or(Some(a.as_str()));
            continue;
        }
        // `\l` names a bookmark, `\o` a tooltip and `\t` a target frame; `\m` and `\n` take none.
        let value = matches!(a.to_ascii_lowercase().as_str(), "\\l" | "\\o" | "\\t").then(|| args.next()).flatten();
        if a.eq_ignore_ascii_case("\\l") {
            anchor = value.map(|(v, _)| v.as_str());
        }
    }
    let (url, anchor) = (url.map(str::trim).filter(|u| !u.is_empty()), anchor.map(str::trim).filter(|a| !a.is_empty()));
    match (url, anchor) {
        (Some(u), Some(a)) => Some(format!("{u}#{a}")),
        (Some(u), None) => Some(u.to_string()),
        (None, Some(a)) => Some(format!("#{a}")),
        (None, None) => None,
    }
}

/// A field code's words: the field's name, its switches and its arguments, each with whether
/// it was quoted. In quotes, `\"` is a quote and `\\` a backslash.
fn field_args(instr: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut chars = instr.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        let mut word = String::new();
        if c == '"' {
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' if matches!(chars.peek(), Some('"' | '\\')) => word.extend(chars.next()),
                    _ => word.push(c),
                }
            }
            out.push((word, true));
        } else {
            word.push(c);
            while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '"') {
                word.push(c);
            }
            out.push((word, false));
        }
    }
    out
}

impl Reader<'_> {
    // ---- fields ----

    pub(super) fn begin_field(&mut self, sc: &mut StoryCtx, props: CharProps, instr: String, locked: bool, dirty: bool, form: String) {
        if sc.fields.len() >= MAX_FIELD_DEPTH || sc.ignored_begins > 0 {
            sc.ignored_begins += 1;
            return;
        }
        // Fields nested deeper than the cap are ignored (their content flows to the enclosing field).
        sc.fields.push(FieldState {
            instr,
            phase: Phase::Instr,
            spilled: false,
            buf: Vec::new(),
            props,
            locked,
            dirty,
            range: false,
            sizes: Vec::new(),
            form,
            code_xml: String::new(),
            code_xml_lost: false,
        });
    }

    /// Whether the field below the innermost one is still collecting its code (a field nested
    /// in another's code contributes text to that code, never content).
    fn parent_in_instr(sc: &StoryCtx) -> bool {
        let n = sc.fields.len();
        n >= 2 && sc.fields.get(n - 2).is_some_and(|p| !p.spilled && !p.range && p.phase == Phase::Instr)
    }

    /// `separate`: the code is complete; a range field starts its result here.
    pub(super) fn separate_field(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        if sc.ignored_begins > 0 {
            return;
        }
        let nested_in_instr = Self::parent_in_instr(sc);
        let Some(f) = sc.fields.last_mut() else { return };
        if f.range {
            return;
        }
        f.phase = Phase::Result;
        if !f.spilled && !nested_in_instr && is_range_instr(&f.instr) {
            f.range = true;
            let (obj, props) = (InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, f.props.clone());
            self.emit_obj(sc, pb, obj, &props);
        }
    }

    pub(super) fn end_field(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        if sc.ignored_begins > 0 {
            sc.ignored_begins -= 1;
            return;
        }
        let nested_in_instr = Self::parent_in_instr(sc);
        let Some(f) = sc.fields.pop() else { return };
        if f.range {
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        if f.spilled {
            return;
        }
        if f.phase == Phase::Instr && !nested_in_instr && is_range_instr(&f.instr) {
            // A range field with no result yet.
            self.emit_obj(sc, pb, InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, &f.props);
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        // A `HYPERLINK` field is a link, as a `w:hyperlink` is: its result stays ordinary content
        // carrying the link, so a tab in it reaches its tab stop and the `PAGEREF` in a table of
        // contents entry (Word for the web writes them this way) stays a field of its own (#2174).
        if !nested_in_instr && let Some(target) = hyperlink_target(&f.instr) {
            for (it, mut p) in f.buf {
                p.link.get_or_insert_with(|| target.clone());
                match it {
                    Item::Text(t) => self.emit_text(sc, pb, &t, &p),
                    Item::Obj(o) => self.emit_obj(sc, pb, *o, &p),
                }
            }
            return;
        }
        // A cached result holding a picture, chart or shape (`INCLUDEPICTURE`'s embedded
        // picture) can't be a text result: keep the field as a range around it so the picture
        // stays in the document and is saved again. The linked source is never read.
        // A tracked insertion or deletion inside the result (one the field itself isn't wrapped
        // in) keeps the field a range too: a cached text result has one revision at most, so
        // each piece stays an ordinary run with its own `w:ins` / `w:del` (#1037).
        let tracked_inside = f.buf.iter().any(|(_, p)| (p.ins, p.del) != (f.props.ins, f.props.del));
        if !nested_in_instr && (tracked_inside || f.buf.iter().any(|(it, _)| matches!(it, Item::Obj(o) if o.frame().is_some()))) {
            self.emit_obj(sc, pb, InlineObject::FieldStart { instr: f.instr.trim().to_string(), locked: f.locked, dirty: f.dirty }, &f.props);
            for (it, p) in f.buf {
                match it {
                    Item::Text(t) => self.emit_text(sc, pb, &t, &p),
                    Item::Obj(o) => self.emit_obj(sc, pb, *o, &p),
                }
            }
            self.emit_obj(sc, pb, InlineObject::FieldEnd, &f.props);
            return;
        }
        let mut result = String::new();
        for (it, _) in &f.buf {
            match it {
                Item::Text(t) => result.push_str(t),
                Item::Obj(o) => result.push_str(o.plain_text()),
            }
        }
        // This field nested in its parent's code (in the parent's revision, if any): the parent
        // keeps it as XML.
        let nested_xml = sc
            .fields
            .last()
            .is_some_and(|p| !p.spilled && p.phase == Phase::Instr && !p.code_xml_lost && same_rev(&f.props, &p.props))
            .then_some(())
            .filter(|()| !f.code_xml_lost && f.buf.iter().all(|(_, p)| same_rev(p, &f.props)))
            .map(|()| field_xml(&f));
        let (instr, result) = match enclosure(&f) {
            // Word writes no result for enclosed characters: the text is their plain text.
            Some(e) => (e.instr(), if result.is_empty() { e.text } else { result }),
            None => (f.instr.trim().to_string(), result),
        };
        // The visible result owns the field's formatting; empty results keep the begin's.
        let props = f
            .buf
            .iter()
            .find_map(|(it, p)| match it {
                Item::Text(t) if !t.is_empty() => Some(p),
                Item::Obj(o) if !o.plain_text().is_empty() => Some(p),
                _ => None,
            })
            .unwrap_or(&f.props)
            .clone();
        let code_xml = if f.code_xml_lost { String::new() } else { f.code_xml };
        let obj = InlineObject::Field { instr, result, locked: f.locked, dirty: f.dirty, code: false, form: f.form, code_xml };
        if let Some(parent) = sc.fields.last_mut()
            && !parent.spilled
            && parent.phase == Phase::Instr
        {
            if !parent.code_xml_lost {
                match nested_xml {
                    Some(x) if parent.code_xml.len().saturating_add(x.len()) < MAX_INSTR => {
                        if parent.code_xml.is_empty() {
                            // The code so far, in the field's own formatting.
                            let mut w = crate::xml::W::default();
                            run_xml(&mut w, &parent.props, "w:instrText", &parent.instr);
                            parent.code_xml = w.s;
                        }
                        parent.code_xml.push_str(&x);
                    }
                    _ => {
                        parent.code_xml_lost = true;
                        parent.code_xml = String::new();
                    }
                }
            }
            parent.instr.push_str(obj.plain_text());
            return;
        }
        self.emit_obj(sc, pb, obj, &props);
    }

    /// At a paragraph end, open fields can't keep buffering: emit them (empty result) and
    /// let the rest of their result flow as ordinary text.
    pub(super) fn spill_fields(&mut self, sc: &mut StoryCtx, pb: &mut PB) {
        for f in sc.fields.iter_mut() {
            if f.spilled || f.range {
                continue;
            }
            f.spilled = true;
            let form = std::mem::take(&mut f.form);
            pb.push_obj(
                InlineObject::Field {
                    instr: f.instr.trim().to_string(),
                    result: String::new(),
                    locked: f.locked,
                    dirty: f.dirty,
                    code: false,
                    form,
                    code_xml: String::new(),
                },
                &f.props,
            );
            for (it, p) in f.buf.drain(..) {
                match it {
                    Item::Text(t) => pb.push_text(&t, &p),
                    Item::Obj(o) => pb.push_obj(*o, &p),
                }
            }
        }
    }
}
