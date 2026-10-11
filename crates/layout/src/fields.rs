//! Field results computed at layout time (page numbers) and note numbering.

use std::collections::HashMap;
use std::sync::Arc;

use wordcraft_doc::section::NumFormat;

/// Context for fields that depend on where text lands.
#[derive(Clone, Debug, Default)]
pub struct FieldCtx {
    /// Current page number (as numbered) and its format.
    pub page: u32,
    pub page_format: NumFormat,
    /// Total pages in the document.
    pub pages: u32,
    /// Pages in the current section.
    pub section_pages: u32,
    pub section: u32,
    /// Note part id → its number.
    pub notes: Arc<HashMap<u32, NoteNum>>,
    /// Document title / author for TITLE / AUTHOR fields.
    pub title: Arc<str>,
    pub author: Arc<str>,
    pub filename: Arc<str>,
}

/// A footnote's or endnote's number, as its section's options give it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NoteNum {
    pub n: u32,
    pub fmt: NumFormat,
    /// Index of the section (`Document::sections`) its reference is in.
    pub section: usize,
    /// Place among the notes of its kind, in document order.
    pub order: u32,
}

impl FieldCtx {
    /// The mark of note `id` (its number in its format); `fmt` for a note not counted.
    pub fn note_mark(&self, id: u32, fmt: NumFormat) -> String {
        match self.notes.get(&id) {
            Some(n) => n.fmt.format(n.n),
            None => fmt.format(1),
        }
    }
    /// A key for caches: the parts of the context a page-dependent paragraph depends on.
    pub fn page_key(&self) -> (u32, u32, u32, u32) {
        (self.page, self.pages, self.section_pages, self.section)
    }
}

/// The field's name (first word of the instruction, upper-cased).
pub fn field_name(instr: &str) -> String {
    instr.split_whitespace().next().unwrap_or("").trim_start_matches('=').to_ascii_uppercase()
}

/// `\* FORMAT` switch value (Arabic, roman, ROMAN, alphabetic, ALPHABETIC, CardText, Ordinal…).
fn format_switch(instr: &str) -> Option<NumFormat> {
    let mut it = instr.split_whitespace();
    while let Some(w) = it.next() {
        if w == "\\*" {
            let v = it.next()?;
            return Some(match v {
                "roman" => NumFormat::LowerRoman,
                "ROMAN" | "Roman" => NumFormat::UpperRoman,
                "alphabetic" => NumFormat::LowerLetter,
                "ALPHABETIC" | "Alphabetic" => NumFormat::UpperLetter,
                "CardText" | "cardtext" => NumFormat::CardinalText,
                "OrdText" | "ordtext" => NumFormat::OrdinalText,
                "Ordinal" | "ordinal" => NumFormat::Ordinal,
                "Arabic" | "arabic" => NumFormat::Decimal,
                _ => continue,
            });
        }
    }
    None
}

/// Display text for a field and whether it depends on the page it lands on.
pub fn field_text(instr: &str, result: &str, ctx: &FieldCtx) -> (String, bool) {
    let name = field_name(instr);
    let fmt = format_switch(instr);
    match name.as_str() {
        "PAGE" => (fmt.unwrap_or(ctx.page_format).format(ctx.page), true),
        "NUMPAGES" => (fmt.unwrap_or(NumFormat::Decimal).format(ctx.pages), true),
        "SECTIONPAGES" => (fmt.unwrap_or(NumFormat::Decimal).format(ctx.section_pages), true),
        "SECTION" => (fmt.unwrap_or(NumFormat::Decimal).format(ctx.section), true),
        "TITLE" if result.is_empty() => (ctx.title.to_string(), false),
        "AUTHOR" if result.is_empty() => (ctx.author.to_string(), false),
        "FILENAME" if result.is_empty() => (ctx.filename.to_string(), false),
        _ => (result.to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_fields() {
        let ctx = FieldCtx { page: 4, pages: 12, ..Default::default() };
        assert_eq!(field_text("PAGE", "", &ctx), ("4".into(), true));
        assert_eq!(field_text(" PAGE \\* ROMAN ", "", &ctx).0, "IV");
        assert_eq!(field_text("NUMPAGES \\* CardText", "", &ctx).0, "Twelve");
        assert_eq!(field_text("DATE \\@ \"M/d/yyyy\"", "1/2/2026", &ctx), ("1/2/2026".into(), false));
        assert_eq!(field_name("=SUM(ABOVE)"), "SUM(ABOVE)");
    }
}
