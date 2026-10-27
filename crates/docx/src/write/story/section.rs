use super::*;

impl Writer<'_> {
    /// The section's headers, plus the header holding only the watermark in each slot the
    /// pages show (first page with a title page, even pages with even/odd headers) that has no
    /// header of its own or linked from an earlier section. Sections are written in body order.
    fn watermark_headers(&mut self, s: &SectionProps) -> wordcraft_doc::section::HeaderSet {
        let mut set = s.headers;
        let linked = &mut self.linked_headers;
        if self.doc.settings.watermark.as_ref().and_then(crate::write::watermark::text).is_some() {
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

/// The children of a `w:footnotePr` / `w:endnotePr` in schema order (`w:pos`, `w:numFmt`,
/// `w:numStart`, `w:numRestart`; ECMA-376 §17.11.11), for the fields that are set.
pub(in super::super) fn note_pr_inner(w: &mut W, pr: &NoteProps) {
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
pub(super) fn comment_reference_style(doc: &wordcraft_doc::Document) -> &str {
    doc.styles
        .styles
        .iter()
        .find(|s| s.kind == wordcraft_doc::styles::StyleKind::Character && s.name.eq_ignore_ascii_case("annotation reference"))
        .map_or("CommentReference", |s| s.id.as_str())
}
