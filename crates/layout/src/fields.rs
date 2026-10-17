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
    /// Body paragraphs for STYLEREF fields in headers and footers, and where the page being
    /// drawn is among them (set only while laying out headers and footers that need it).
    pub styleref: Option<(Arc<crate::styleref::StyleIndex>, crate::styleref::At)>,
    /// Where the body's bookmarks start, for `PAGEREF` in headers and footers: known once the
    /// body is paginated, empty while it is.
    pub bookmark_pages: Arc<BookmarkPages>,
    /// A key for caches standing for `bookmark_pages` (0 when it's empty).
    pub bookmarks_key: u64,
    /// Where a paragraph runs onto later pages: from text offset `.0` on, its fields are on page
    /// `.1` (ascending offsets). Empty except during the layout of a body paragraph split across
    /// pages, for its later pages (#1352); nothing else laid out meanwhile sees it.
    pub page_from: Vec<(usize, u32)>,
}

/// Bookmark name → the number and number format of the page it starts on.
pub type BookmarkPages = HashMap<String, (u32, NumFormat)>;

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
    /// The page a field at text offset `off` of the paragraph being laid out is on.
    pub fn page_at(&self, off: usize) -> u32 {
        self.page_from.iter().rev().find(|(from, _)| *from <= off).map_or(self.page, |(_, page)| *page)
    }
    /// A key for caches: the parts of the context a page-dependent paragraph depends on.
    pub fn page_key(&self) -> (u32, u32, u32, u32, u64) {
        (self.page, self.pages, self.section_pages, self.section, self.bookmarks_key)
    }
    /// A key for caches of paragraph `p` with STYLEREF fields (0 when they show their stored
    /// result): a hash of the results its STYLEREF fields have on this page, so an edit to body
    /// text that no STYLEREF shows leaves header paragraphs cached.
    pub fn styleref_key(&self, p: &wordcraft_doc::Paragraph) -> u64 {
        let Some((ix, at)) = &self.styleref else { return 0 };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for o in &p.objects {
            if let wordcraft_doc::para::InlineObject::Field { instr, .. } = o
                && field_name(instr) == "STYLEREF"
            {
                std::hash::Hash::hash(&ix.eval(instr, *at), &mut h);
            }
        }
        std::hash::Hasher::finish(&h).max(1)
    }
}

/// The field's name (first word of the instruction, upper-cased).
pub fn field_name(instr: &str) -> String {
    instr.split_whitespace().next().unwrap_or("").trim_start_matches('=').to_ascii_uppercase()
}

/// `\* FORMAT` switch value (Arabic, roman, ROMAN, alphabetic, ALPHABETIC, CardText, Ordinal…).
pub fn format_switch(instr: &str) -> Option<NumFormat> {
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
                // Chinese numerals: counting (一百零一), legal (壹佰零壹), digit by digit (一〇一).
                v if v.eq_ignore_ascii_case("ChineseNum1") => NumFormat::ChineseCountingThousand,
                v if v.eq_ignore_ascii_case("ChineseNum2") => NumFormat::ChineseLegalSimplified,
                v if v.eq_ignore_ascii_case("ChineseNum3") => NumFormat::IdeographDigital,
                _ => continue,
            });
        }
    }
    None
}

/// The light grey behind a field shown as its code.
pub const FIELD_SHADING: wordcraft_doc::props::Rgb = wordcraft_doc::props::Rgb(0xD9, 0xD9, 0xD9);

/// What a field shows in code view: its instruction in braces, `{ PAGE }`.
pub fn field_code_text(instr: &str) -> String {
    format!("{{ {} }}", instr.trim())
}

/// Display text for a field and whether it depends on the page it lands on.
pub fn field_text(instr: &str, result: &str, ctx: &FieldCtx) -> (String, bool) {
    field_text_on(instr, result, ctx, ctx.page)
}

/// Display text for a field on page `page` (see [`field_text`]).
pub fn field_text_on(instr: &str, result: &str, ctx: &FieldCtx, page: u32) -> (String, bool) {
    let name = field_name(instr);
    let fmt = format_switch(instr);
    match name.as_str() {
        "PAGE" => (fmt.unwrap_or(ctx.page_format).format(page), true),
        "NUMPAGES" => (fmt.unwrap_or(NumFormat::Decimal).format(ctx.pages), true),
        "SECTIONPAGES" => (fmt.unwrap_or(NumFormat::Decimal).format(ctx.section_pages), true),
        "SECTION" => (fmt.unwrap_or(NumFormat::Decimal).format(ctx.section), true),
        "PAGEREF" => match pageref_page(instr, ctx) {
            Some((n, page_format)) => (fmt.unwrap_or(page_format).format(n), true),
            None => (result.to_string(), false),
        },
        "TITLE" if result.is_empty() => (ctx.title.to_string(), false),
        "AUTHOR" if result.is_empty() => (ctx.author.to_string(), false),
        "FILENAME" if result.is_empty() => (ctx.filename.to_string(), false),
        "STYLEREF" => match &ctx.styleref {
            Some((ix, at)) => (ix.eval(instr, *at), true),
            None => (result.to_string(), false),
        },
        _ => (result.to_string(), false),
    }
}

/// The page a `PAGEREF` field's bookmark starts on, when known (see
/// [`FieldCtx::bookmark_pages`]) and the field shows its number (`\p` shows "above" or "below").
fn pageref_page(instr: &str, ctx: &FieldCtx) -> Option<(u32, NumFormat)> {
    // The bookmark is the word after the field name, up to a switch written right after it.
    let word = instr.split_whitespace().nth(1)?;
    let target = word.split('\\').next().unwrap_or("").trim_matches('"');
    if target.is_empty() || has_switch(instr, 'p') {
        return None;
    }
    ctx.bookmark_pages.get(target).copied()
}

/// Whether `instr` has the one-letter switch `\sw`, also run together with another (`\p\h`).
fn has_switch(instr: &str, sw: char) -> bool {
    instr.split('\\').skip(1).any(|s| {
        let mut chars = s.chars();
        chars.next().is_some_and(|c| c.eq_ignore_ascii_case(&sw)) && chars.next().is_none_or(|c| !c.is_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_fields() {
        crate::test_fonts();
        let ctx = FieldCtx { page: 4, pages: 12, ..Default::default() };
        assert_eq!(field_text("PAGE", "", &ctx), ("4".into(), true));
        assert_eq!(field_text(" PAGE \\* ROMAN ", "", &ctx).0, "IV");
        assert_eq!(field_text("NUMPAGES \\* CardText", "", &ctx).0, "Twelve");
        assert_eq!(field_text("NUMPAGES \\* CHINESENUM1", "", &ctx).0, "十二");
        assert_eq!(field_text("PAGE \\* ChineseNum2", "", &ctx).0, "肆");
        assert_eq!(field_text("DATE \\@ \"M/d/yyyy\"", "1/2/2026", &ctx), ("1/2/2026".into(), false));
        assert_eq!(field_name("=SUM(ABOVE)"), "SUM(ABOVE)");
        assert_eq!(field_code_text(" PAGE \\* ROMAN "), "{ PAGE \\* ROMAN }");
        assert_eq!(field_code_text(""), "{  }");
    }

    #[test]
    fn page_references_show_their_bookmarks_page() {
        crate::test_fonts();
        let pages = HashMap::from([("End_1".to_string(), (3, NumFormat::Decimal)), ("Intro".to_string(), (4, NumFormat::LowerRoman))]);
        let ctx = FieldCtx { bookmark_pages: Arc::new(pages), bookmarks_key: 1, ..Default::default() };
        assert_eq!(field_text("PAGEREF End_1 \\* Arabic \\* MERGEFORMAT", "9", &ctx), ("3".into(), true));
        assert_eq!(field_text(" PAGEREF End_1 \\h ", "9", &ctx).0, "3");
        assert_eq!(field_text("PAGEREF End_1\\h", "9", &ctx).0, "3");
        // The page's own format, unless a switch says otherwise.
        assert_eq!(field_text("PAGEREF Intro", "9", &ctx).0, "iv");
        assert_eq!(field_text("PAGEREF Intro \\* Arabic", "9", &ctx).0, "4");
        assert_eq!(field_text("PAGEREF \"End_1\"", "9", &ctx).0, "3");
        // Unknown bookmarks, "above"/"below" (\p) and a context without pages keep the result.
        assert_eq!(field_text("PAGEREF Missing", "9", &ctx), ("9".into(), false));
        for instr in ["PAGEREF End_1 \\p", "PAGEREF End_1 \\P", "PAGEREF End_1 \\p\\h", "PAGEREF End_1\\h\\p"] {
            assert_eq!(field_text(instr, "above", &ctx), ("above".into(), false), "{instr}");
        }
        assert_eq!(field_text("PAGEREF End_1 \\* MERGEFORMAT", "9", &ctx).0, "3", "\\* isn't \\p");
        assert_eq!(field_text("PAGEREF \\h", "9", &ctx), ("9".into(), false));
        assert_eq!(field_text("PAGEREF", "9", &ctx), ("9".into(), false));
        assert_eq!(field_text("PAGEREF End_1", "9", &FieldCtx::default()), ("9".into(), false));
    }
}
