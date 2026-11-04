//! Notes and comments written (ODF 1.3 §6.3, §14.1): their citations, authors, dates, replies and
//! content, the paragraph being written put aside meanwhile.

use super::{FBlock, Fmt, Writer, model, x};

/// The `office:name` of comment `id` written by a writer with names prefixed `prefix`.
pub(in crate::odt) fn annotation_name(prefix: &str, id: u32) -> String {
    format!("__Annotation__{prefix}{id}")
}

impl Writer {
    /// A comment (ODF 1.3 §14.1): named, so its `office:annotation-end` closes its range, or for
    /// a reply naming the comment it answers (LibreOffice's `loext:parent-name`); its author,
    /// date, initials, resolved state (`loext:resolved`) and text.
    pub(in crate::odt) fn comment(&mut self, id: u32, c: &model::FComment, out: &mut String) {
        let reopen = self.open_ins.as_ref().map(|(m, _)| m.clone());
        self.close_ins(out);
        let mut attrs = match c.parent {
            Some(p) => format!(" loext:parent-name=\"{}\"", annotation_name(self.prefix, p)),
            None => format!(" office:name=\"{}\"", annotation_name(self.prefix, id)),
        };
        if c.resolved {
            attrs.push_str(" loext:resolved=\"true\"");
        }
        let mut head = String::new();
        if !c.author.is_empty() {
            head.push_str(&format!("<dc:creator>{}</dc:creator>", x(&c.author)));
        }
        if !c.date.is_empty() {
            head.push_str(&format!("<dc:date>{}</dc:date>", x(&c.date)));
        }
        if !c.initials.is_empty() {
            head.push_str(&format!("<meta:creator-initials>{}</meta:creator-initials>", x(&c.initials)));
        }
        let saved = (std::mem::take(&mut self.cur_style), std::mem::take(&mut self.chain), self.part, self.note_parent.take());
        let mut body = String::new();
        // An annotation holds paragraphs and lists only: a table's text goes in as paragraphs.
        self.blocks(&model::flatten_tables(&c.blocks), &mut body, 1);
        self.close_ins(&mut body);
        (self.cur_style, self.chain, self.part, self.note_parent) = saved;
        out.push_str(&format!("<office:annotation{attrs}>{head}{body}</office:annotation>"));
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }

    /// A footnote or endnote (ODF 1.3 §6.3): its citation (the number LibreOffice shows until it
    /// renumbers, or the custom mark) and its content. The paragraph being written is put aside
    /// meanwhile, and an insertion open around the note ends before it and goes on after it.
    /// A note whose reference has its own look `fmt`: in a span of it (the citation takes the
    /// text's formatting).
    pub(in crate::odt) fn styled_note(&mut self, endnote: bool, custom: &str, blocks: &[FBlock], fmt: &Fmt, out: &mut String) {
        match self.text_style(&Fmt { link: None, ins: None, del: None, ..fmt.clone() }) {
            Some(n) => {
                out.push_str(&format!("<text:span text:style-name=\"{n}\">"));
                self.note(endnote, custom, blocks, out);
                out.push_str("</text:span>");
            }
            None => self.note(endnote, custom, blocks, out),
        }
    }

    pub(in crate::odt) fn note(&mut self, endnote: bool, custom: &str, blocks: &[FBlock], out: &mut String) {
        let reopen = self.open_ins.as_ref().map(|(m, _)| m.clone());
        self.close_ins(out);
        self.notes += 1;
        let id = format!("{}ftn{}", self.prefix, self.notes);
        let (class, parent) = if endnote { ("endnote", "Endnote") } else { ("footnote", "Footnote") };
        let saved = (std::mem::take(&mut self.cur_style), std::mem::take(&mut self.chain), self.part, self.note_parent.replace(parent));
        let mut body = String::new();
        self.blocks(blocks, &mut body, 1);
        self.close_ins(&mut body);
        (self.cur_style, self.chain, self.part, self.note_parent) = saved;
        if body.is_empty() {
            body.push_str(&format!("<text:p text:style-name=\"{parent}\"/>"));
        }
        // Its number in document order, in the document's format from its first number (§6.3.3:
        // the citation is the text shown; LibreOffice renumbers on opening, other readers may
        // not). Without the document's numbering, LibreOffice's defaults: 1, 2, 3 and i, ii, iii.
        let nth = if endnote { &mut self.endnotes } else { &mut self.footnotes };
        *nth = nth.saturating_add(1);
        let (fmt, start) = match self.note_numbers {
            Some(n) => n[usize::from(endnote)],
            None if endnote => (wordcraft_doc::section::NumFormat::LowerRoman, 1),
            None => (wordcraft_doc::section::NumFormat::Decimal, 1),
        };
        let number = fmt.format(start.max(1).saturating_add(*nth - 1));
        let (label, citation) = if custom.is_empty() { (String::new(), number) } else { (format!(" text:label=\"{}\"", x(custom)), x(custom)) };
        out.push_str(&format!(
            "<text:note text:id=\"{id}\" text:note-class=\"{class}\"><text:note-citation{label}>{citation}</text:note-citation><text:note-body>{body}</text:note-body></text:note>"
        ));
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }
}
