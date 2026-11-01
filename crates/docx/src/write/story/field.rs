use super::toc::is_toc;
use super::{CharProps, W, Writer, rpr};

pub(super) struct FieldData<'a> {
    pub instr: &'a str,
    pub result: &'a str,
    pub locked: bool,
    pub dirty: bool,
    pub form: &'a str,
    pub code_xml: &'a str,
}

impl Writer<'_> {
    pub(super) fn field(&mut self, w: &mut W, field: FieldData<'_>, props: &CharProps) {
        let FieldData { instr, result, locked, dirty, form, code_xml } = field;
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
        self.field_start(w, instr, locked, dirty, form, code_xml, props);
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
                        '\u{000C}' | '\u{000E}' => {
                            flush(w, &mut buf);
                            w.empty("w:br", &[("w:type", if c == '\u{000C}' { "page" } else { "column" })]);
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
    /// belongs inside the begin. `code_xml` is a code holding nested fields as runs
    /// ([`InlineObject::Field`]), written in place of `instr` when it is well-formed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn field_start(&self, w: &mut W, instr: &str, locked: bool, dirty: bool, form: &str, code_xml: &str, props: &CharProps) {
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
        match nested_code(code_xml, del) {
            Some(code) => w.raw(&code),
            None => {
                let instr_text = format!(" {} ", instr.trim());
                run(w, &|w| w.leaf(if del { "w:delInstrText" } else { "w:instrText" }, &[("xml:space", "preserve")], &instr_text));
            }
        }
        run(w, &|w| w.empty("w:fldChar", &[("w:fldCharType", "separate")]));
    }
}

/// A field code holding nested fields ([`InlineObject::Field`]'s `code_xml`), checked and
/// re-serialised from its parsed runs. Only what a field code's runs hold is allowed (run
/// properties, code and result text, tabs, breaks, hyphens and `w:fldChar`s), and the nested
/// fields must be complete (begin, optional separate, end); anything else gives `None`, and the
/// field is written with its flat `instr`. In a tracked deletion its text becomes deleted text
/// (ECMA-376 §17.13.5.13–14).
fn nested_code(code_xml: &str, del: bool) -> Option<String> {
    use crate::xml::{El, Node};
    /// Elements that may never appear inside a run's properties or a form field's definition.
    fn clean(e: &El, depth: usize) -> bool {
        depth <= crate::xml::MAX_DEPTH
            && !e.attrs.iter().any(|(k, _)| k.starts_with("r:"))
            && e.kids.iter().all(|k| match k {
                Node::El(c) => {
                    !matches!(
                        c.name.as_str(),
                        "w:p"
                            | "w:r"
                            | "w:tbl"
                            | "w:sdt"
                            | "w:drawing"
                            | "w:pict"
                            | "w:object"
                            | "w:fldChar"
                            | "w:instrText"
                            | "w:t"
                            | "w:delText"
                            | "w:delInstrText"
                    ) && clean(c, depth + 1)
                }
                Node::Text(_) => true,
            })
    }
    fn text_only(e: &El) -> bool {
        e.kids.iter().all(|k| matches!(k, Node::Text(_)))
    }
    if code_xml.is_empty() {
        return None;
    }
    let wrapped = format!(r#"<w:code xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{code_xml}</w:code>"#);
    let mut root = crate::xml::parse(wrapped.as_bytes()).ok()?;
    // Open nested fields: whether each has reached its separate.
    let mut open: Vec<bool> = Vec::new();
    let mut out = String::new();
    for k in &mut root.kids {
        let Node::El(r) = k else { return None };
        if r.name != "w:r" || r.attrs.iter().any(|(k, _)| k.starts_with("r:")) {
            return None;
        }
        for c in &mut r.kids {
            let Node::El(c) = c else { return None };
            match c.name.as_str() {
                "w:rPr" if clean(c, 0) => {}
                "w:t" | "w:delText" | "w:instrText" | "w:delInstrText" if text_only(c) => {
                    if del {
                        c.name = if c.name.ends_with("nstrText") { "w:delInstrText" } else { "w:delText" }.into();
                    }
                }
                "w:tab" | "w:br" | "w:cr" | "w:noBreakHyphen" | "w:softHyphen" if c.kids.is_empty() => {}
                "w:fldChar" if clean(c, 0) => match c.attr("w:fldCharType") {
                    Some("begin") => open.push(false),
                    Some("separate") => match open.last_mut() {
                        Some(sep) if !*sep => *sep = true,
                        _ => return None,
                    },
                    Some("end") => {
                        open.pop()?;
                    }
                    _ => return None,
                },
                _ => return None,
            }
        }
        out.push_str(&r.to_xml());
    }
    (open.is_empty() && !out.is_empty()).then_some(out)
}

/// One `w:r` with `props`, holding what `f` writes (a field character or code).
pub(super) fn field_run(w: &mut W, props: &CharProps, f: &dyn Fn(&mut W)) {
    w.open("w:r", &[]);
    rpr(w, props);
    f(w);
    w.close("w:r");
}
