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
    pub filepath: Arc<str>,
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
    pub fn page_key(&self) -> (u32, u32, u32, u32, u64, NumFormat) {
        (self.page, self.pages, self.section_pages, self.section, self.bookmarks_key, self.page_format)
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

/// REF's hyperlink switch links its cached result without updating that result.
pub(crate) fn reference_link(instr: &str) -> Option<String> {
    let (name, args) = instr.trim_start().split_once(char::is_whitespace)?;
    if !name.eq_ignore_ascii_case("REF") {
        return None;
    }
    let args = args.trim_start();
    let (target, switches) = if let Some(quoted) = args.strip_prefix('"') {
        let end = quoted.find('"')?;
        (quoted.get(..end)?, quoted.get(end + 1..)?)
    } else {
        let end = args.find(|c: char| c.is_whitespace() || c == '\\').unwrap_or(args.len());
        (args.get(..end)?, args.get(end..)?)
    };
    if target.is_empty() {
        return None;
    }
    // Switch arguments can contain quoted backslashes; those are not switches.
    let mut chars = switches.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        if quoted && c == '\\' {
            chars.next();
        } else if c == '"' {
            quoted = !quoted;
        } else if !quoted
            && c == '\\'
            && chars.next().is_some_and(|s| s.eq_ignore_ascii_case(&'h'))
            && chars.peek().is_none_or(|c| !c.is_alphanumeric())
        {
            return Some(format!("#{target}"));
        }
    }
    None
}

/// Return the single numeric literal from `{ = <lit> \* <fmt> }`, or `None` when
/// the expression has more than one operand (e.g. `{ = 100 + 50 \* Hex }`).
/// Prevents F9 and the display path from picking up only the first token of a
/// multi-operand expression and discarding a Word-computed cached result.
pub fn literal_after_eq(instr: &str) -> Option<&str> {
    let rest = instr.trim_start().strip_prefix('=')?;
    // Strip everything from the first switch (`\`) onwards; switches are not operands.
    let expr = match rest.find('\\') {
        Some(i) => &rest[..i],
        None => rest,
    };
    let mut tokens = expr.split_whitespace();
    let lit = tokens.next()?;
    if tokens.next().is_some() {
        None // multi-operand: operators or extra numbers present
    } else {
        Some(lit)
    }
}

/// `\* FORMAT` switch value (Arabic, roman, ROMAN, alphabetic, ALPHABETIC, CardText, Ordinal,
/// ArabicDash, DollarText, Hex…). `{ PAGE \* ArabicDash }` shows "- 31 -".
pub fn format_switch(instr: &str) -> Option<NumFormat> {
    let mut it = instr.split_whitespace();
    while let Some(w) = it.next() {
        if let Some(rest) = w.strip_prefix("\\*") {
            let v = if rest.is_empty() { it.next()? } else { rest };
            let v = v.split('\\').next().unwrap_or(v);
            let fmt = match v {
                "roman" => NumFormat::LowerRoman,
                "ROMAN" | "Roman" => NumFormat::UpperRoman,
                "alphabetic" => NumFormat::LowerLetter,
                "ALPHABETIC" | "Alphabetic" => NumFormat::UpperLetter,
                "CardText" | "cardtext" => NumFormat::CardinalText,
                "OrdText" | "ordtext" => NumFormat::OrdinalText,
                "Ordinal" | "ordinal" => NumFormat::Ordinal,
                "Arabic" | "arabic" => NumFormat::Decimal,
                "ArabicDash" | "arabicdash" | "ARABICDASH" => NumFormat::NumberInDash,
                "DollarText" | "dollartext" | "DOLLARTEXT" => NumFormat::DollarText,
                "Hex" | "hex" | "HEX" => NumFormat::Hex,
                // Chinese numerals: counting (一百零一), legal (壹佰零壹), digit by digit (一〇一).
                v if v.eq_ignore_ascii_case("ChineseNum1") => NumFormat::ChineseCountingThousand,
                v if v.eq_ignore_ascii_case("ChineseNum2") => NumFormat::ChineseLegalSimplified,
                v if v.eq_ignore_ascii_case("ChineseNum3") => NumFormat::IdeographDigital,
                v if v.eq_ignore_ascii_case("BahtText") => NumFormat::BahtText,
                _ => continue,
            };
            return Some(fmt);
        }
    }
    None
}

/// A field's capitalization switch (`\* Upper`, `\* Lower`, `\* Caps`, `\* FirstCap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaseSwitch {
    Upper,
    Lower,
    Caps,
    FirstCap,
}

/// The capitalization switches in a field code, in the order they appear. Quoted text in the code
/// is not a switch.
fn case_switches(instr: &str) -> Vec<CaseSwitch> {
    let mut chars = instr.chars().peekable();
    let mut quoted = false;
    let mut cases = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted => {
                chars.next();
            }
            '"' => quoted = !quoted,
            '\\' if chars.peek() == Some(&'*') => {
                chars.next();
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
                let mut word = String::new();
                while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '\\' && *c != '"') {
                    word.push(c);
                }
                let case = match word.to_ascii_lowercase().as_str() {
                    "upper" => CaseSwitch::Upper,
                    "lower" => CaseSwitch::Lower,
                    "caps" => CaseSwitch::Caps,
                    "firstcap" => CaseSwitch::FirstCap,
                    _ => continue,
                };
                cases.push(case);
            }
            _ => {}
        }
    }
    cases
}

/// `text` with the field's capitalization switches applied in the order they appear, so
/// `\* Lower \* Caps` shows "WEEKLY REPORT" as "Weekly Report". `\* Upper` capitalizes every
/// letter, `\* Lower` makes every letter lowercase, `\* Caps` capitalizes the first letter of each
/// word, and `\* FirstCap` capitalizes the first letter of the first word. A word's first letter is
/// its first letter or digit, after any leading quote, bracket or other punctuation. Without a
/// switch, the function returns `text` unchanged.
pub fn apply_case_switch(instr: &str, text: String) -> String {
    case_switches(instr).into_iter().fold(text, apply_case)
}

/// `text` with one capitalization switch applied (see [`apply_case_switch`]).
fn apply_case(text: String, case: CaseSwitch) -> String {
    match case {
        CaseSwitch::Upper => text.to_uppercase(),
        CaseSwitch::Lower => text.to_lowercase(),
        CaseSwitch::Caps | CaseSwitch::FirstCap => {
            let mut out = String::with_capacity(text.len());
            let mut word_start = true;
            let mut done = false;
            for c in text.chars() {
                if c.is_whitespace() {
                    word_start = true;
                    out.push(c);
                } else if word_start && !done && c.is_alphanumeric() {
                    out.extend(c.to_uppercase());
                    word_start = false;
                    done = case == CaseSwitch::FirstCap;
                } else {
                    if c.is_alphanumeric() {
                        word_start = false;
                    }
                    out.push(c);
                }
            }
            out
        }
    }
}

/// Formats `n` in `fmt` as a field shows it. Cardinal, ordinal and dollar text is lowercase
/// (`\* CardText` shows "twenty-one", `\* DollarText` shows "twenty-one and 00/100") and writes
/// numbers up to 999,999; a larger number keeps its digits. `\* Hex` writes uppercase hexadecimal
/// digits (`1CA`). The field's capitalization switches apply after that (`\* CardText \* Caps`
/// shows "Twenty-one").
pub fn field_number(instr: &str, fmt: NumFormat, n: u32) -> String {
    let text = match fmt {
        NumFormat::CardinalText | NumFormat::OrdinalText | NumFormat::DollarText if n > MAX_TEXT_NUMBER => n.to_string(),
        NumFormat::CardinalText | NumFormat::OrdinalText => fmt.format(n).to_lowercase(),
        NumFormat::DollarText => dollar_text(f64::from(n)).unwrap_or_else(|| n.to_string()),
        NumFormat::Hex => format!("{n:X}"),
        _ => fmt.format(n),
    };
    apply_case_switch(instr, text)
}

/// The largest number cardinal, ordinal and dollar text write in words.
const MAX_TEXT_NUMBER: u32 = 999_999;

/// `value` as `\* DollarText` writes it, before any capitalization switch: the whole part in
/// lowercase cardinal text, "and", and the first two decimals (rounded) over 100
/// (14.55 gives "fourteen and 55/100"). `None` for NaN, or for a value that rounds below 0 or past
/// 999,999.99.
pub fn dollar_text(value: f64) -> Option<String> {
    let cents = (value * 100.0).round();
    if !(0.0..=f64::from(MAX_TEXT_NUMBER) * 100.0 + 99.0).contains(&cents) {
        return None;
    }
    let cents = cents as u32;
    Some(format!("{} and {:02}/100", NumFormat::CardinalText.format(cents / 100).to_lowercase(), cents % 100))
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
        "PAGE" => (field_number(instr, format_switch(instr).unwrap_or(ctx.page_format), page), true),
        "NUMPAGES" => (field_number(instr, fmt.unwrap_or(NumFormat::Decimal), ctx.pages), true),
        "SECTIONPAGES" => (field_number(instr, fmt.unwrap_or(NumFormat::Decimal), ctx.section_pages), true),
        "SECTION" => (field_number(instr, fmt.unwrap_or(NumFormat::Decimal), ctx.section), true),
        "PAGEREF" => match pageref_page(instr, ctx) {
            Some((n, page_format)) => (field_number(instr, fmt.unwrap_or(page_format), n), true),
            None => (result.to_string(), false),
        },
        "TITLE" if result.is_empty() => (apply_case_switch(instr, ctx.title.to_string()), false),
        "AUTHOR" if result.is_empty() => (apply_case_switch(instr, ctx.author.to_string()), false),
        "FILENAME" if result.is_empty() => {
            let text = if has_switch(instr, 'p') && !ctx.filepath.is_empty() {
                ctx.filepath.to_string()
            } else if !ctx.filename.is_empty() {
                ctx.filename.to_string()
            } else {
                "Document1".into()
            };
            (apply_case_switch(instr, text), false)
        }
        // ASK and SET only assign text to a bookmark: they never show a result (ECMA-376 §17.16.5).
        "ASK" | "SET" => (String::new(), false),
        // BAHTTEXT expr: the numeric expression (a literal for now) in Thai Baht words.
        "BAHTTEXT" => {
            // The second whitespace token after the field name is the numeric literal.
            // Use parse_baht_token to avoid f64 precision loss (e.g. "1.005" → 1.01).
            let val = instr.split_whitespace().nth(1).and_then(wordcraft_doc::section::parse_baht_token);
            match val.and_then(wordcraft_doc::section::baht_text_f64) {
                Some(s) => (s, false),
                None => (result.to_string(), false),
            }
        }
        "STYLEREF" => match &ctx.styleref {
            Some((ix, at)) => (apply_case_switch(instr, ix.eval(instr, *at)), true),
            None => (result.to_string(), false),
        },
        _ => {
            // Apply a \* format switch when a numeric value is available.
            //
            // For BahtText the instruction literal takes precedence over the cached result
            // because the cache may hold a stale "0" while the literal carries the true
            // amount.  Resolution order for BahtText:
            //   1. Field name itself  (`{ 12345 \* BahtText }` — number as field name).
            //   2. First token after `=` (`{ = 12345 \* BahtText }` — expression field).
            //   3. Stored result (fallback).
            // For all other switches the stored result retains its existing first-place
            // priority so that Word-computed values are preserved.
            if let Some(fmt) = fmt {
                let x = if fmt == NumFormat::BahtText {
                    // Use parse_baht_token to avoid f64 precision loss (e.g. "1.005" → 1.01).
                    let parse = wordcraft_doc::section::parse_baht_token;
                    // For `= expr` fields the field name is "=" (or "=token" with no space);
                    // do not parse it as the amount — use literal_after_eq instead so
                    // multi-operand expressions (`{ = 100 + 50 \* BahtText }`) fall through
                    // to the cached result.
                    let from_name = if !instr.trim_start().starts_with('=') {
                        parse(&name).filter(|v| v.is_finite())
                    } else {
                        None
                    };
                    from_name
                        .or_else(|| literal_after_eq(instr).and_then(parse))
                        .or_else(|| parse(result.trim()))
                } else if fmt == NumFormat::Hex {
                    // For \* Hex the stored result is already hex-formatted (e.g. "A", "1F").
                    // Re-parsing it as decimal would give a wrong second conversion.
                    // Use the field name or the literal instead; if neither is available,
                    // preserve the stored result unchanged by returning None here.
                    name.parse::<f64>().ok()
                        .or_else(|| literal_after_eq(instr)?.parse().ok())
                } else {
                    result.trim().parse::<f64>().ok()
                        .or_else(|| name.parse::<f64>().ok())
                        .or_else(|| literal_after_eq(instr)?.parse().ok())
                };
                if let Some(x) = x {
                    if fmt == NumFormat::BahtText {
                        let text = wordcraft_doc::section::baht_text_f64(x)
                            .unwrap_or_else(|| result.to_string());
                        return (text, false);
                    }
                    // DollarText preserves cents from the f64 value; all other integer formats
                    // go through field_number (which handles Hex, case switches, etc.).
                    if fmt == NumFormat::DollarText {
                        let text = dollar_text(x).unwrap_or_else(|| result.to_string());
                        return (apply_case_switch(instr, text), false);
                    }
                    if x >= 0.0 && x <= f64::from(u32::MAX) && x.fract() == 0.0 {
                        return (field_number(instr, fmt, x as u32), false);
                    }
                }
            }
            (result.to_string(), false)
        }
    }
}

/// Extract the bookmark target from a `PAGEREF` instruction.
fn pageref_target(instr: &str) -> Option<&str> {
    let mut parts = instr.split_whitespace();
    let name = parts.next()?;
    let after_name = &instr[instr.find(name)? + name.len()..];
    let trimmed = after_name.trim_start();
    if let Some(rest) = trimmed.strip_prefix('"') {
        let bookmark = rest.split('"').next()?;
        if !bookmark.is_empty() {
            return Some(bookmark);
        }
    }
    let word = parts.next()?;
    let target = word.split('\\').next().unwrap_or("").trim_matches('"');
    if !target.is_empty() {
        return Some(target);
    }
    None
}

/// The page a `PAGEREF` field's bookmark starts on, when known (see
/// [`FieldCtx::bookmark_pages`]) and the field shows its number (`\p` shows "above" or "below").
fn pageref_page(instr: &str, ctx: &FieldCtx) -> Option<(u32, NumFormat)> {
    let target = pageref_target(instr)?;
    if has_switch(instr, 'p') {
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
    fn ref_link_switches() {
        for instr in [r"REF mark\w\h", r#" REF "mark" \h"#, "REF\tmark \\H"] {
            assert_eq!(reference_link(instr).as_deref(), Some("#mark"), "{instr}");
        }
        for instr in ["", "REF", r"REF \h", r#"REF "" \h"#, r#"REF "mark \h"#, r"REF mark \hidden", r"NOTEREF mark \h", r#"REF mark \d "\h""#] {
            assert_eq!(reference_link(instr), None, "{instr}");
        }
    }

    #[test]
    fn filename_field() {
        let ctx = FieldCtx { filename: "report.docx".into(), filepath: "/home/user/docs/report.docx".into(), ..Default::default() };
        assert_eq!(field_text("FILENAME", "", &ctx), ("report.docx".into(), false));
        assert_eq!(field_text(r"FILENAME \p", "", &ctx), ("/home/user/docs/report.docx".into(), false));
        assert_eq!(field_text(r"FILENAME \* MERGEFORMAT", "", &ctx), ("report.docx".into(), false));
        assert_eq!(field_text("FILENAME", "cached.docx", &ctx), ("cached.docx".into(), false));

        let def_ctx = FieldCtx::default();
        assert_eq!(field_text("FILENAME", "", &def_ctx), ("Document1".into(), false));
        assert_eq!(field_text(r"FILENAME \p", "", &def_ctx), ("Document1".into(), false));
    }

    #[test]
    fn page_fields() {
        let ctx = FieldCtx { page: 4, pages: 12, ..Default::default() };
        assert_eq!(field_text("PAGE", "", &ctx), ("4".into(), true));
        assert_eq!(field_text(" PAGE \\* ROMAN ", "", &ctx).0, "IV");
        assert_eq!(field_text("PAGE \\*roman", "", &ctx).0, "iv");
        assert_eq!(field_text("PAGE \\*roman\\*MERGEFORMAT", "", &ctx).0, "iv");
        assert_eq!(field_text("NUMPAGES \\* CardText", "", &ctx).0, "twelve");
        assert_eq!(field_text("NUMPAGES \\* CHINESENUM1", "", &ctx).0, "十二");
        assert_eq!(field_text("PAGE \\* ChineseNum2", "", &ctx).0, "肆");
        assert_eq!(field_text("DATE \\@ \"M/d/yyyy\"", "1/2/2026", &ctx), ("1/2/2026".into(), false));
        assert_eq!(field_name("=SUM(ABOVE)"), "SUM(ABOVE)");
        assert_eq!(field_code_text(" PAGE \\* ROMAN "), "{ PAGE \\* ROMAN }");
        assert_eq!(field_code_text(""), "{  }");
    }

    /// Cardinal and ordinal text is lowercase unless a capitalization switch follows; `\* Upper`,
    /// `\* Lower`, `\* Caps` and `\* FirstCap` apply to the page fields and the TITLE, AUTHOR and
    /// FILENAME fallbacks. A switch inside quotes is text.
    #[test]
    fn capitalization_switches() {
        let ctx = FieldCtx {
            page: 21,
            pages: 790,
            title: "weekly report on sales".into(),
            filename: "Weekly Sales Report.doc".into(),
            ..Default::default()
        };
        let text = |instr: &str| field_text(instr, "", &ctx).0;
        assert_eq!(text(r"NUMPAGES \* CardText"), "seven hundred ninety");
        assert_eq!(text(r"NUMPAGES \* CardText \* Caps"), "Seven Hundred Ninety");
        assert_eq!(text(r"PAGE \* OrdText"), "twenty-first");
        assert_eq!(text(r"PAGE \* OrdText \* FirstCap"), "Twenty-first");
        assert_eq!(text(r"PAGE \* CardText \* Upper"), "TWENTY-ONE");
        assert_eq!(text(r"PAGE \* roman \* upper"), "XXI");
        assert_eq!(text(r"TITLE \* FirstCap \* MERGEFORMAT"), "Weekly report on sales");
        assert_eq!(text(r"TITLE \*Caps"), "Weekly Report On Sales");
        assert_eq!(text(r"FILENAME \* Lower"), "weekly sales report.doc");
        assert_eq!(text(r"AUTHOR \* Upper"), "");
        for (title, caps, first_cap) in [
            ("\"weekly report\"", "\"Weekly Report\"", "\"Weekly report\""),
            ("(weekly) report", "(Weekly) Report", "(Weekly) report"),
            ("  q3 - final", "  Q3 - Final", "  Q3 - final"),
        ] {
            let ctx = FieldCtx { title: title.into(), ..Default::default() };
            assert_eq!(field_text(r"TITLE \* Caps", "", &ctx).0, caps);
            assert_eq!(field_text(r"TITLE \* FirstCap", "", &ctx).0, first_cap);
        }
        assert_eq!(field_text(r#"TITLE "x \* Upper""#, "", &ctx).0, "weekly report on sales");
        assert_eq!(field_text(r"REF mark \* Upper", "stored", &ctx).0, "stored", "a stored result shows as it is");
    }

    /// `\* DollarText` writes lowercase cardinal text with "and 00/100", `\* Hex` uppercase
    /// hexadecimal digits, and `\* ArabicDash` the number between hyphens. Words
    /// stop at 999,999; a larger number keeps its digits.
    #[test]
    fn dollar_text_hex_and_arabic_dash_switches() {
        let ctx = FieldCtx { page: 31, pages: 458, section: 790, ..Default::default() };
        let text = |instr: &str| field_text(instr, "", &ctx).0;
        assert_eq!(text(r"PAGE \* ArabicDash"), "- 31 -");
        assert_eq!(text(r"PAGE \*arabicdash \* MERGEFORMAT"), "- 31 -");
        assert_eq!(text(r"NUMPAGES \* ArabicDash"), "- 458 -", "any number field reads ArabicDash");
        assert_eq!(text(r"NUMPAGES \* ArabicDash \* roman"), "- 458 -", "the first format switch counts");
        assert_eq!(text(r"NUMPAGES \* Hex"), "1CA");
        assert_eq!(text(r"NUMPAGES \* hex \* Lower"), "1ca");
        assert_eq!(text(r"PAGE \* Hex"), "1F");
        assert_eq!(text(r"SECTION \* DollarText"), "seven hundred ninety and 00/100");
        assert_eq!(text(r"SECTION \* DollarText \* Upper"), "SEVEN HUNDRED NINETY AND 00/100");
        assert_eq!(text(r"PAGE \* DollarText \* FirstCap"), "Thirty-one and 00/100");
        assert_eq!(dollar_text(9.20 + 5.35).as_deref(), Some("fourteen and 55/100"));
        assert_eq!(dollar_text(0.999).as_deref(), Some("one and 00/100"), "the decimals round");
        assert_eq!(dollar_text(999_999.99).as_deref(), Some("nine hundred ninety-nine thousand nine hundred ninety-nine and 99/100"));
        assert_eq!(dollar_text(999_999.995), None);
        assert_eq!(dollar_text(-1.0), None);
        assert_eq!(dollar_text(-0.004).as_deref(), Some("zero and 00/100"), "a value that rounds to 0 is not below 0");
        assert_eq!(dollar_text(f64::NAN), None);
        for fmt in [NumFormat::CardinalText, NumFormat::OrdinalText, NumFormat::DollarText] {
            assert_eq!(field_number(r"SEQ x \* Upper", fmt, 1_000_140), "1000140", "{fmt:?}");
        }
        assert_eq!(field_number("SEQ x", NumFormat::CardinalText, 999_999), "nine hundred ninety-nine thousand nine hundred ninety-nine");
    }

    /// Several capitalization switches apply in the order they appear.
    #[test]
    fn capitalization_switches_apply_in_order() {
        let ctx = FieldCtx { title: "WEEKLY REPORT".into(), ..Default::default() };
        let text = |instr: &str| field_text(instr, "", &ctx).0;
        assert_eq!(text(r"TITLE \* Lower \* Caps"), "Weekly Report");
        assert_eq!(text(r"TITLE \* Lower \* FirstCap"), "Weekly report");
        assert_eq!(text(r"TITLE \* Caps \* Lower"), "weekly report");
        assert_eq!(text(r"TITLE \* Lower \* MERGEFORMAT \* Upper"), "WEEKLY REPORT");
    }

    /// #1765: ASK and SET only assign text to a bookmark; they show nothing, whatever result is stored.
    #[test]
    fn ask_and_set_show_nothing() {
        let ctx = FieldCtx::default();
        assert_eq!(field_text(r#" ASK Nominativo "Inserire nome" \d "«NOME»" "#, "«NOME» «COGNOME»", &ctx), (String::new(), false));
        assert_eq!(field_text(r#"SET Codice "COD""#, "COD", &ctx), (String::new(), false));
        assert_eq!(field_text("set Codice COD", "COD", &ctx).0, "");
        // Other fields still show their stored result.
        assert_eq!(field_text("MERGEFIELD NOME", "«NOME»", &ctx).0, "«NOME»");
    }

    #[test]
    fn page_references_show_their_bookmarks_page() {
        let pages = HashMap::from([
            ("End_1".to_string(), (3, NumFormat::Decimal)),
            ("Intro".to_string(), (4, NumFormat::LowerRoman)),
            ("Chapter One".to_string(), (7, NumFormat::Decimal)),
        ]);
        let ctx = FieldCtx { bookmark_pages: Arc::new(pages), bookmarks_key: 1, ..Default::default() };
        assert_eq!(field_text("PAGEREF End_1 \\* Arabic \\* MERGEFORMAT", "9", &ctx), ("3".into(), true));
        assert_eq!(field_text(" PAGEREF End_1 \\h ", "9", &ctx).0, "3");
        assert_eq!(field_text("PAGEREF End_1\\h", "9", &ctx).0, "3");
        // The page's own format, unless a switch says otherwise.
        assert_eq!(field_text("PAGEREF Intro", "9", &ctx).0, "iv");
        assert_eq!(field_text("PAGEREF Intro \\* Arabic", "9", &ctx).0, "4");
        assert_eq!(field_text("PAGEREF \"End_1\"", "9", &ctx).0, "3");
        assert_eq!(field_text("PAGEREF \"Chapter One\" \\h", "9", &ctx).0, "7");
        assert_eq!(field_text("PAGEREF \"Chapter One\"\\h", "9", &ctx).0, "7");
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
