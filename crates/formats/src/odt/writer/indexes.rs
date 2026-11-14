//! Tables of contents and other generated indexes written (ODF 1.3 §8): a WordCraft table — a
//! `TOC`, `INDEX` or `BIBLIOGRAPHY` field in its heading followed by its entries, or the field
//! around its entries — becomes the ODF index with its title, its source (what LibreOffice
//! regenerates it from) and its entries as last generated.

use super::{FBlock, Inline, Para, Writer, x};

/// What a Word index field stands for in ODF.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::odt) enum IndexKind {
    /// A table of contents of headings down to this outline level (`TOC \o "1-n"`).
    Contents(u8),
    /// A table of figures (or of any captioned thing): `TOC \c "Figure"`.
    Captions(String),
    /// An alphabetical index (`INDEX`).
    Alphabetical,
    /// A bibliography (`BIBLIOGRAPHY`).
    Bibliography,
}

/// The quoted (or bare) argument of switch `\sw` in field code `instr`.
fn switch_arg(instr: &str, sw: char) -> Option<String> {
    let at = instr.find(&format!("\\{sw}"))?;
    let rest = instr.get(at + 2..)?.trim_start();
    let arg = match rest.strip_prefix('"') {
        Some(q) => q.split('"').next()?,
        None => rest.split_whitespace().next()?,
    };
    (!arg.starts_with('\\')).then(|| arg.chars().take(64).collect())
}

/// The index field code `instr` is, if it is one.
pub(in crate::odt) fn index_kind(instr: &str) -> Option<IndexKind> {
    let name = instr.split_whitespace().next()?.to_ascii_uppercase();
    match name.as_str() {
        "TOC" => match switch_arg(instr, 'c').or_else(|| switch_arg(instr, 'C')) {
            Some(c) => Some(IndexKind::Captions(c)),
            None => {
                // `\o "1-3"`: the deepest level shown (3 when none is given, as Word's default).
                let deepest =
                    switch_arg(instr, 'o').and_then(|r| r.rsplit('-').next().and_then(|n| n.trim().parse::<u8>().ok())).unwrap_or(3).clamp(1, 10);
                Some(IndexKind::Contents(deepest))
            }
        },
        "INDEX" => Some(IndexKind::Alphabetical),
        "BIBLIOGRAPHY" => Some(IndexKind::Bibliography),
        _ => None,
    }
}

/// The field code an ODF index stands for in Word (the reverse of [`index_kind`]).
pub(in crate::odt) fn index_instr(kind: &IndexKind) -> String {
    match kind {
        IndexKind::Contents(n) => format!("TOC \\o \"1-{n}\" \\h \\z \\u"),
        IndexKind::Captions(c) => format!("TOC \\h \\z \\c \"{}\"", c.replace('"', "")),
        IndexKind::Alphabetical => "INDEX".into(),
        IndexKind::Bibliography => "BIBLIOGRAPHY".into(),
    }
}

/// Whether `p`'s paragraph style is an index entry's (WordCraft's TOC, table of figures, index
/// and bibliography styles).
fn is_entry(p: &Para) -> bool {
    let style = p.extra.as_ref().and_then(|x| x.style.as_deref()).unwrap_or("");
    (style.starts_with("TOC") && style != "TOCHeading") || style == "TableofFigures" || style.starts_with("Index") || style == "Bibliography"
}

/// An index starting at `blocks[i]`: how many blocks it takes, its kind, and whether its first
/// block is its title. Either a paragraph holding an index field followed by its entries (as
/// WordCraft inserts a table), or a field starting a paragraph and ending in a later one.
pub(in crate::odt) fn index_at(blocks: &[FBlock], i: usize) -> Option<(usize, IndexKind, bool)> {
    let Some(FBlock::Para(p)) = blocks.get(i) else { return None };
    // A heading holding the field, anywhere in it.
    if let Some(kind) = p.inlines.iter().find_map(|inl| match inl {
        Inline::Field { instr, .. } => index_kind(instr),
        _ => None,
    }) {
        let entries =
            blocks.get(i + 1..).unwrap_or(&[]).iter().take_while(|b| matches!(b, FBlock::Para(q) if is_entry(q) && q.list.is_none())).count();
        return Some((entries + 1, kind, true));
    }
    // A field starting the paragraph.
    for inline in &p.inlines {
        match inline {
            Inline::FieldStart(instr) => {
                let kind = index_kind(instr)?;
                // Its end: where the fields opened from here close.
                let mut open = 0usize;
                for (k, b) in blocks.get(i..).unwrap_or(&[]).iter().enumerate() {
                    let FBlock::Para(q) = b else { return None };
                    for inl in &q.inlines {
                        match inl {
                            Inline::FieldStart(_) => open += 1,
                            Inline::FieldEnd => {
                                open = open.saturating_sub(1);
                                if open == 0 {
                                    return Some((k + 1, kind, false));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                return None;
            }
            Inline::Text(t, _) if t.trim().is_empty() => {}
            Inline::Anchor(_) => {}
            _ => return None,
        }
    }
    None
}

impl Writer {
    /// The index `blocks` of `kind` (its first block its title when `titled`).
    pub(in crate::odt) fn index(&mut self, blocks: &[FBlock], kind: &IndexKind, titled: bool, out: &mut String) {
        self.indexes += 1;
        let n = self.indexes;
        let name = format!("{}Index{n}", self.prefix);
        let (element, source) = match kind {
            IndexKind::Contents(levels) => {
                // An entry per level as LibreOffice makes it: linked text, a right tab with dots
                // and the page number.
                let templates: String = (1..=*levels)
                    .map(|l| {
                        format!(
                            "<text:table-of-content-entry-template text:outline-level=\"{l}\" text:style-name=\"Contents_20_{l}\"><text:index-entry-link-start/><text:index-entry-chapter/><text:index-entry-text/><text:index-entry-tab-stop style:type=\"right\" style:leader-char=\".\"/><text:index-entry-page-number/><text:index-entry-link-end/></text:table-of-content-entry-template>"
                        )
                    })
                    .collect();
                (
                    "text:table-of-content",
                    format!("<text:table-of-content-source text:outline-level=\"{levels}\">{templates}</text:table-of-content-source>"),
                )
            }
            IndexKind::Captions(c) => (
                "text:illustration-index",
                format!("<text:illustration-index-source text:caption-sequence-name=\"{}\" text:caption-sequence-format=\"text\"/>", x(c)),
            ),
            IndexKind::Alphabetical => ("text:alphabetical-index", "<text:alphabetical-index-source/>".to_string()),
            IndexKind::Bibliography => ("text:bibliography", "<text:bibliography-source/>".to_string()),
        };
        out.push_str(&format!("<{element} text:name=\"{name}\">{source}<text:index-body>"));
        let (title, entries) = if titled { blocks.split_at(blocks.len().min(1)) } else { (&[][..], blocks) };
        if !title.is_empty() {
            out.push_str(&format!("<text:index-title text:name=\"{name}_Head\">"));
            self.blocks(title, out, 1);
            out.push_str("</text:index-title>");
        }
        self.in_index = true;
        self.blocks(entries, out, 1);
        self.in_index = false;
        self.close_ins(out);
        out.push_str(&format!("</text:index-body></{element}>\n"));
    }
}
