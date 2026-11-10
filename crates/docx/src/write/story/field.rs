use super::toc::is_toc;
use super::{CharProps, W, Writer, rpr};

/// A field's begin and code, as [`Writer::field_start`] writes them.
pub(super) struct FieldStart<'a> {
    pub instr: &'a str,
    pub locked: bool,
    pub dirty: bool,
    pub form: &'a str,
    pub runs: &'a [(usize, usize, CharProps)],
}

pub(super) struct FieldData<'a> {
    pub instr: &'a str,
    pub result: &'a str,
    pub locked: bool,
    pub dirty: bool,
    pub form: &'a str,
    /// The code's runs and their formatting, as read (`eq::Formula::runs`): a superscript `2`
    /// in `a2` keeps its run. Empty for a field made here, whose code is one run.
    pub runs: &'a [(usize, usize, CharProps)],
}

impl Writer<'_> {
    pub(super) fn field(&mut self, w: &mut W, field: FieldData<'_>, props: &CharProps) {
        let FieldData { instr, result, locked, dirty, form, runs } = field;
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
        self.field_start(w, FieldStart { instr, locked, dirty, form, runs }, props);
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
    pub(super) fn field_start(&self, w: &mut W, f: FieldStart<'_>, props: &CharProps) {
        let FieldStart { instr, locked, dirty, form, runs } = f;
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
        let code = instr.trim();
        let tag = if del { "w:delInstrText" } else { "w:instrText" };
        // One run with `props` holding `text` as instruction text.
        let leaf = |w: &mut W, p: &CharProps, text: &str| {
            w.open("w:r", &[]);
            rpr(w, p);
            w.leaf(tag, &[("xml:space", "preserve")], text);
            w.close("w:r");
        };
        if runs.is_empty() {
            leaf(w, props, &format!(" {code} "));
        } else {
            // The code's runs as read: their own formatting, gaps and all.
            leaf(w, props, " ");
            let mut at = 0usize;
            for (start, end, p) in runs {
                let (start, end) = (*start, (*end).min(code.len()));
                if start < at || start >= end {
                    continue;
                }
                if start > at
                    && let Some(gap) = code.get(at..start)
                {
                    leaf(w, props, gap);
                }
                if let Some(seg) = code.get(start..end) {
                    leaf(w, p, seg);
                }
                at = end;
            }
            if let Some(tail) = code.get(at..).filter(|t| !t.is_empty()) {
                leaf(w, props, tail);
            }
            leaf(w, props, " ");
        }
        run(w, &|w| w.empty("w:fldChar", &[("w:fldCharType", "separate")]));
    }
}

/// One `w:r` with `props`, holding what `f` writes (a field character or code).
pub(super) fn field_run(w: &mut W, props: &CharProps, f: &dyn Fn(&mut W)) {
    w.open("w:r", &[]);
    rpr(w, props);
    f(w);
    w.close("w:r");
}
