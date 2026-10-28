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

/// The words after each `\*` in a field code, in the order they appear. Quoted text in the code is
/// not a switch (a backslash inside it escapes the next character).
fn general_switches(instr: &str) -> Vec<String> {
    let mut chars = instr.chars().peekable();
    let mut quoted = false;
    let mut words = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted => {
                chars.next();
            }
            '"' => quoted = !quoted,
            '\\' if !quoted && chars.peek() == Some(&'*') => {
                chars.next();
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
                let mut word = String::new();
                while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '\\' && *c != '"') {
                    word.push(c);
                }
                words.push(word);
            }
            _ => {}
        }
    }
    words
}

/// The number format a `\*` switch word names (`roman` lowercase, `ROMAN` and `Roman` uppercase;
/// the other names ignore case).
fn number_format(word: &str) -> Option<NumFormat> {
    Some(match word {
        "roman" => NumFormat::LowerRoman,
        "ROMAN" | "Roman" => NumFormat::UpperRoman,
        "alphabetic" => NumFormat::LowerLetter,
        "ALPHABETIC" | "Alphabetic" => NumFormat::UpperLetter,
        w if w.eq_ignore_ascii_case("CardText") => NumFormat::CardinalText,
        w if w.eq_ignore_ascii_case("OrdText") => NumFormat::OrdinalText,
        w if w.eq_ignore_ascii_case("Ordinal") => NumFormat::Ordinal,
        w if w.eq_ignore_ascii_case("Arabic") => NumFormat::Decimal,
        w if w.eq_ignore_ascii_case("ArabicDash") => NumFormat::NumberInDash,
        w if w.eq_ignore_ascii_case("DollarText") => NumFormat::DollarText,
        w if w.eq_ignore_ascii_case("Hex") => NumFormat::Hex,
        // Chinese numerals: counting (一百零一), legal (壹佰零壹), digit by digit (一〇一).
        w if w.eq_ignore_ascii_case("ChineseNum1") => NumFormat::ChineseCountingThousand,
        w if w.eq_ignore_ascii_case("ChineseNum2") => NumFormat::ChineseLegalSimplified,
        w if w.eq_ignore_ascii_case("ChineseNum3") => NumFormat::IdeographDigital,
        _ => return None,
    })
}

/// The `\* FORMAT` switch value (Arabic, roman, ROMAN, alphabetic, ALPHABETIC, CardText, Ordinal,
/// ArabicDash, DollarText, Hex…); the last format switch wins, as in Word (`\* ArabicDash \* roman`
/// shows roman). `{ PAGE \* ArabicDash }` shows "- 31 -".
pub fn format_switch(instr: &str) -> Option<NumFormat> {
    general_switches(instr).iter().rev().find_map(|w| number_format(w))
}

/// A field's capitalization switch (`\* Upper`, `\* Lower`, `\* Caps`, `\* FirstCap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaseSwitch {
    Upper,
    Lower,
    Caps,
    FirstCap,
}

/// The capitalization switches in a field code, in the order they appear.
fn case_switches(instr: &str) -> Vec<CaseSwitch> {
    general_switches(instr)
        .iter()
        .filter_map(|word| match word.to_ascii_lowercase().as_str() {
            "upper" => Some(CaseSwitch::Upper),
            "lower" => Some(CaseSwitch::Lower),
            "caps" => Some(CaseSwitch::Caps),
            "firstcap" => Some(CaseSwitch::FirstCap),
            _ => None,
        })
        .collect()
}

/// `text` with the field's capitalization switches applied in the order they appear, so
/// `\* Lower \* Caps` shows "WEEKLY REPORT" as "Weekly Report". `\* Upper` capitalizes every
/// letter, `\* Lower` makes every letter lowercase, `\* Caps` capitalizes the first letter of each
/// word and lowercases the others (`wEEKLY rEPORT` gives "Weekly Report"), and `\* FirstCap`
/// capitalizes the first letter of the first word and leaves the rest. A word is a run of letters
/// and digits, hyphens, slashes and other punctuation start a new one, and an apostrophe inside a
/// word does not (`o'neil` gives "O'neil"). A letter whose capital is more than one letter (ß)
/// stays as it is. Without a switch, the function returns `text` unchanged.
pub fn apply_case_switch(instr: &str, text: String) -> String {
    case_switches(instr).into_iter().fold(text, apply_case)
}

/// `c` in capitals when that is one letter (ß has no single-letter capital).
fn upper_char(c: char) -> char {
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

/// `c` in lowercase when that is one letter.
fn lower_char(c: char) -> char {
    let mut low = c.to_lowercase();
    match (low.next(), low.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}')
}

/// Whether the character at `i` starts a word: nothing before it, or something other than a letter,
/// a digit or an apostrophe that follows one.
fn starts_word(chars: &[char], i: usize) -> bool {
    let before = |back: usize| i.checked_sub(back).and_then(|at| chars.get(at)).copied();
    match before(1) {
        None => true,
        Some(c) if c.is_alphanumeric() => false,
        Some(c) if is_apostrophe(c) => !before(2).is_some_and(char::is_alphanumeric),
        Some(_) => true,
    }
}

/// `text` with one capitalization switch applied (see [`apply_case_switch`]).
fn apply_case(text: String, case: CaseSwitch) -> String {
    match case {
        CaseSwitch::Upper => text.chars().map(upper_char).collect(),
        CaseSwitch::Lower => text.chars().map(lower_char).collect(),
        CaseSwitch::Caps => {
            let chars: Vec<char> = text.chars().collect();
            chars.iter().enumerate().map(|(i, &c)| if c.is_alphabetic() && starts_word(&chars, i) { upper_char(c) } else { lower_char(c) }).collect()
        }
        CaseSwitch::FirstCap => {
            // The first letter, digit or apostrophe decides: only a letter is raised.
            let mut raised = false;
            text.chars()
                .map(|c| {
                    let first = !raised && (c.is_alphanumeric() || is_apostrophe(c));
                    raised |= first;
                    if first { upper_char(c) } else { c }
                })
                .collect()
        }
    }
}

/// What Word shows for a number a `\*` format can't write.
pub const NUMBER_FORMAT_ERROR: &str = "Error! Number cannot be represented in specified format.";

/// The largest number cardinal, ordinal and dollar text write in words.
const MAX_TEXT_NUMBER: u32 = 999_999;

/// Formats `n` in `fmt` as a field shows it (see [`field_value`]).
pub fn field_number(instr: &str, fmt: NumFormat, n: u32) -> String {
    field_value(instr, fmt, f64::from(n))
}

/// Formats `value` in `fmt` as a field shows it, then applies the field's capitalization switches
/// (`\* CardText \* Caps` shows "Twenty-One"). A format writes a whole number, so `value` is rounded
/// first (2.5 gives 3, -2.5 gives -3); one between 0.5 and 1 in size, or past the format's range,
/// shows [`NUMBER_FORMAT_ERROR`] instead. As Word writes them: cardinal and ordinal text for 0 to
/// 999,999 in lowercase (`\* CardText` shows "twenty-one"), roman numerals for 1 to 32,767 and
/// letters for 1 to 780 (0 shows a space), `\* Hex` uppercase hexadecimal digits for 0 to 65,535
/// (`1CA`), `\* DollarText` as [`dollar_text`] does, and `\* Arabic`, `\* ArabicDash` and
/// `\* Ordinal` any number.
pub fn field_value(instr: &str, fmt: NumFormat, value: f64) -> String {
    let text = value_text(fmt, value).unwrap_or_else(|| NUMBER_FORMAT_ERROR.to_string());
    apply_case_switch(instr, text)
}

fn value_text(fmt: NumFormat, value: f64) -> Option<String> {
    if fmt == NumFormat::DollarText {
        return dollar_text(value);
    }
    // Word rounds half away from zero, except that a fraction between a half and one is an error.
    let size = value.abs();
    let bounded_format = !matches!(fmt, NumFormat::Decimal | NumFormat::NumberInDash | NumFormat::Ordinal);
    if !size.is_finite() || (bounded_format && size > 1e15) || (0.5..1.0).contains(&size) {
        return None;
    }
    let rounded = value.round();
    // `i64::MAX as f64` rounds up to 2^63, so use an exclusive upper bound.
    if rounded < i64::MIN as f64 || rounded >= -(i64::MIN as f64) {
        return None;
    }
    let n = rounded as i64;
    let in_range = |max: u32| u32::try_from(n).ok().filter(|n| *n <= max);
    match fmt {
        NumFormat::CardinalText | NumFormat::OrdinalText => in_range(MAX_TEXT_NUMBER).map(|n| fmt.format(n).to_lowercase()),
        NumFormat::LowerRoman | NumFormat::UpperRoman => in_range(32_767).map(|n| if n == 0 { " ".into() } else { fmt.format(n) }),
        NumFormat::LowerLetter | NumFormat::UpperLetter => in_range(780).map(|n| if n == 0 { " ".into() } else { fmt.format(n) }),
        NumFormat::Hex => in_range(65_535).map(|n| format!("{n:X}")),
        NumFormat::Decimal => Some(n.to_string()),
        NumFormat::NumberInDash => Some(format!("- {n} -")),
        NumFormat::Ordinal => {
            let abs = n.unsigned_abs();
            let suffix = match (abs % 100, abs % 10) {
                (11..=13, _) => "th",
                (_, 1) => "st",
                (_, 2) => "nd",
                (_, 3) => "rd",
                _ => "th",
            };
            Some(format!("{}{}{}", if n < 0 { "-" } else { "" }, abs, suffix))
        }
        _ => in_range(u32::MAX).map(|n| fmt.format(n)),
    }
}

/// `value` as `\* DollarText` writes it, before any capitalization switch: the whole part (cut off,
/// not rounded) in lowercase cardinal text, "and", and the first two decimals of its size, rounded
/// in decimal, over 100 (14.555 gives "fourteen and 56/100", -0.5 "zero and 50/100"). Decimals that
/// round up to 100 show 00 without adding to the whole part (1.995 gives "one and 00/100").
/// `None` for NaN, or for a whole part below 0 or past 999,999 (shown as [`NUMBER_FORMAT_ERROR`]).
pub fn dollar_text(value: f64) -> Option<String> {
    let whole = value.trunc();
    if !value.is_finite() || !(0.0..=f64::from(MAX_TEXT_NUMBER)).contains(&whole) {
        return None;
    }
    let digits = value.abs().to_string();
    let mut decimals = digits.split_once('.').map_or("", |(_, d)| d).chars().map(|c| c.to_digit(10).unwrap_or(0));
    let first_two = decimals.next().unwrap_or(0) * 10 + decimals.next().unwrap_or(0);
    let cents = (first_two + u32::from(decimals.next().unwrap_or(0) >= 5)) % 100;
    // `whole` is within 0..=999,999 here.
    Some(format!("{} and {cents:02}/100", NumFormat::CardinalText.format(whole as u32).to_lowercase()))
}

/// The text argument of a field such as `TITLE "Quarterly report"`: the quoted text after the
/// field's name, where a backslash makes the next character plain (`"x \* Upper"` is x * Upper), or
/// one word without quotes. `None` when a switch (or nothing) follows the name.
pub fn text_argument(instr: &str) -> Option<String> {
    let rest = instr.trim_start().split_once(char::is_whitespace)?.1.trim_start();
    let mut chars = rest.chars();
    match chars.next()? {
        '"' => {
            let mut text = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => text.extend(chars.next()),
                    c => text.push(c),
                }
            }
            Some(text)
        }
        '\\' => None,
        _ => Some(rest.split_whitespace().next().unwrap_or_default().to_string()),
    }
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
        "TITLE" if result.is_empty() => (apply_case_switch(instr, text_argument(instr).unwrap_or_else(|| ctx.title.to_string())), false),
        "AUTHOR" if result.is_empty() => (apply_case_switch(instr, text_argument(instr).unwrap_or_else(|| ctx.author.to_string())), false),
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
        "STYLEREF" => match &ctx.styleref {
            Some((ix, at)) => (apply_case_switch(instr, ix.eval(instr, *at)), true),
            None => (result.to_string(), false),
        },
        _ => (result.to_string(), false),
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
    /// FILENAME fallbacks. A switch inside quotes is text. Every expected value below is what Word
    /// 16.0 (build 16.0.20527) showed on 2026-10-26 for the same field code in a new document.
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
        assert_eq!(text(r"PAGE \* OrdText \* Caps"), "Twenty-First");
        assert_eq!(text(r"PAGE \* CardText \* Upper"), "TWENTY-ONE");
        assert_eq!(text(r"PAGE \* roman \* upper"), "XXI");
        assert_eq!(text(r"PAGE \* roman \* Caps"), "Xxi");
        assert_eq!(text(r"PAGE \* hex \* Caps"), "15");
        assert_eq!(text(r"TITLE \* FirstCap \* MERGEFORMAT"), "Weekly report on sales");
        assert_eq!(text(r"TITLE \*Caps"), "Weekly Report On Sales");
        assert_eq!(text(r"FILENAME \* Lower"), "weekly sales report.doc");
        assert_eq!(text(r"AUTHOR \* Upper"), "");
        // `\* Caps` lowercases the other letters; `\* FirstCap` leaves them.
        for (title, caps, first_cap) in [
            ("WEEKLY REPORT", "Weekly Report", "WEEKLY REPORT"),
            ("wEEKLY rEPORT", "Weekly Report", "WEEKLY rEPORT"),
            ("(weekly) report", "(Weekly) Report", "(Weekly) report"),
            ("  q3 - final", "  Q3 - Final", "  Q3 - final"),
            ("don't stop-now o'neil x-ray 3rd", "Don't Stop-Now O'neil X-Ray 3rd", "Don't stop-now o'neil x-ray 3rd"),
            ("hello_world a.b.c a/b (x)y", "Hello_World A.B.C A/B (X)Y", "Hello_world a.b.c a/b (x)y"),
            ("ab 'cd' ef x'y 'z", "Ab 'Cd' Ef X'y 'Z", "Ab 'cd' ef x'y 'z"),
            ("don\u{2019}t \u{2019}x", "Don\u{2019}t \u{2019}X", "Don\u{2019}t \u{2019}x"),
            ("a1b2 c3d", "A1b2 C3d", "A1b2 c3d"),
            ("1a b", "1a B", "1a b"),
            ("ÀBC DÉF", "Àbc Déf", "ÀBC DÉF"),
            ("ΑΒΓ ΔΕΖ", "Αβγ Δεζ", "ΑΒΓ ΔΕΖ"),
            ("-x y", "-X Y", "-X y"),
            ("a--b", "A--B", "A--b"),
            // The first letter, digit or apostrophe decides FirstCap, and only a letter is raised.
            ("'quoted' text", "'Quoted' Text", "'quoted' text"),
            ("3 apples", "3 Apples", "3 apples"),
        ] {
            let ctx = FieldCtx { title: title.into(), ..Default::default() };
            assert_eq!(field_text(r"TITLE \* Caps", "", &ctx).0, caps, "Caps of {title}");
            assert_eq!(field_text(r"TITLE \* FirstCap", "", &ctx).0, first_cap, "FirstCap of {title}");
        }
        // Quoted text is an argument, not a switch, and it shows in place of the title.
        assert_eq!(text(r#"TITLE "WEEKLY REPORT" \* Caps"#), "Weekly Report");
        assert_eq!(text(r#"TITLE "wEEKLY rEPORT" \* FirstCap"#), "WEEKLY rEPORT");
        assert_eq!(text(r#"TITLE "x \* Upper""#), "x * Upper");
        assert_eq!(text(r#"AUTHOR "x \* Lower""#), "x * Lower");
        assert_eq!(text(r#"TITLE "a \"q\" b" \* Upper"#), "A \"Q\" B");
        assert_eq!(text(r#"TITLE "a \\ b""#), r"a \ b");
        assert_eq!(text(r#"TITLE "a \n b""#), "a n b", "a backslash only makes the next character plain");
        assert_eq!(text(r#"TITLE "a \* b \\ c" \* Caps"#), r"A * B \ C");
        assert_eq!(text(r#"TITLE "a \\* b""#), r"a \* b");
        // Word keeps ß, and any letter without a one-letter capital, as it is.
        assert_eq!(text(r#"TITLE "éé straße" \* Upper"#), "ÉÉ STRAßE");
        assert_eq!(text(r#"TITLE "ABC straße" \* Lower"#), "abc straße");
        assert_eq!(field_text(r"REF mark \* Upper", "stored", &ctx).0, "stored", "a stored result shows as it is");
    }

    /// `\* DollarText` writes lowercase cardinal text with the decimals over 100, `\* Hex` uppercase
    /// hexadecimal digits, and `\* ArabicDash` the number between hyphens. Expected values are
    /// Word 16.0 (build 16.0.20527) results on 2026-10-26.
    #[test]
    fn dollar_text_hex_and_arabic_dash_switches() {
        let ctx = FieldCtx { page: 31, pages: 458, section: 790, ..Default::default() };
        let text = |instr: &str| field_text(instr, "", &ctx).0;
        assert_eq!(text(r"PAGE \* ArabicDash"), "- 31 -");
        assert_eq!(text(r"PAGE \*arabicdash \* MERGEFORMAT"), "- 31 -");
        assert_eq!(text(r"NUMPAGES \* ArabicDash"), "- 458 -", "any number field reads ArabicDash");
        assert_eq!(text(r"NUMPAGES \* Hex"), "1CA");
        for instr in [r"NUMPAGES \* hex", r"NUMPAGES \* HEX", r"NUMPAGES \* Hex \* Upper"] {
            assert_eq!(text(instr), "1CA", "{instr}");
        }
        assert_eq!(text(r"NUMPAGES \* hex \* Lower"), "1ca");
        assert_eq!(text(r"PAGE \* Hex"), "1F");
        assert_eq!(text(r"SECTION \* DollarText"), "seven hundred ninety and 00/100");
        assert_eq!(text(r"SECTION \* DollarText \* Upper"), "SEVEN HUNDRED NINETY AND 00/100");
        assert_eq!(text(r"SECTION \* DollarText \* Caps"), "Seven Hundred Ninety And 00/100");
        assert_eq!(text(r"PAGE \* DollarText \* FirstCap"), "Thirty-one and 00/100");
        assert_eq!(text(r"PAGE \* Ordinal"), "31st");
    }

    /// With several format switches the last one counts: Word showed `i` for `NUMPAGES \*
    /// ArabicDash \* roman` and `- 1 -` with them the other way round, `one` for `PAGE \* Hex \*
    /// CardText` and `1` for `PAGE \* roman \* Arabic`. A switch inside quotes is not one.
    #[test]
    fn the_last_format_switch_wins() {
        let text = |instr: &str, n: u32| field_text(instr, "", &FieldCtx { page: n, pages: n, ..Default::default() }).0;
        assert_eq!(text(r"NUMPAGES \* ArabicDash \* roman", 1), "i");
        assert_eq!(text(r"NUMPAGES \* roman \* ArabicDash", 1), "- 1 -");
        assert_eq!(text(r"PAGE \* Hex \* CardText", 1), "one");
        assert_eq!(text(r"PAGE \* roman \* Arabic", 1), "1");
        assert_eq!(text(r"PAGE \* roman \* MERGEFORMAT", 4), "iv", "a switch that is no format doesn't count");
        assert_eq!(text(r"PAGE \* ROMAN \* roman", 5), "v");
        assert_eq!(text(r#"PAGE \* roman "\* CardText""#, 5), "v", "quoted text is not a switch");
        // Word reads the names without regard to case, except that roman and alphabetic are
        // lowercase only for lowercase.
        for (instr, shown) in [
            (r"PAGE \* CARDTEXT", "five"),
            (r"PAGE \* cardtext", "five"),
            (r"PAGE \* ordtext", "fifth"),
            (r"PAGE \* ORDTEXT", "fifth"),
            (r"PAGE \* Roman", "V"),
            (r"PAGE \* ROMAN", "V"),
            (r"PAGE \* Alphabetic", "E"),
            (r"PAGE \* alphabetic", "e"),
            (r"PAGE \* ordinal", "5th"),
            (r"PAGE \* ARABIC", "5"),
        ] {
            assert_eq!(text(instr, 5), shown, "{instr}");
        }
    }

    /// `\* DollarText` cuts the whole part off and rounds the first two decimals in decimal, half
    /// up; decimals that round to 100 show 00 and leave the whole part alone. Word 16.0 (build
    /// 16.0.20527), 2026-10-26.
    #[test]
    fn dollar_text_follows_words_rounding() {
        for (value, text) in [
            (0.999, "zero and 00/100"),
            (1.995, "one and 00/100"),
            (2.999, "two and 00/100"),
            (0.995, "zero and 00/100"),
            (999_999.995, "nine hundred ninety-nine thousand nine hundred ninety-nine and 00/100"),
            (999_999.999, "nine hundred ninety-nine thousand nine hundred ninety-nine and 00/100"),
            (999_999.99, "nine hundred ninety-nine thousand nine hundred ninety-nine and 99/100"),
            (12.345, "twelve and 35/100"),
            (14.555, "fourteen and 56/100"),
            (2.675, "two and 68/100"),
            (1.4, "one and 40/100"),
            (0.5, "zero and 50/100"),
            (0.0, "zero and 00/100"),
            (0.004, "zero and 00/100"),
            (0.005, "zero and 01/100"),
            (-0.004, "zero and 00/100"),
            (-0.005, "zero and 01/100"),
            (-0.01, "zero and 01/100"),
            (-0.5, "zero and 50/100"),
            (1234.5, "one thousand two hundred thirty-four and 50/100"),
            (9.20 + 5.35, "fourteen and 55/100"),
        ] {
            assert_eq!(dollar_text(value).as_deref(), Some(text), "{value}");
        }
        for value in [-1.0, -1.5, 1_000_000.0, 1_000_000.4, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
            assert_eq!(dollar_text(value), None, "{value}");
        }
    }

    /// A number a format can't write shows Word's error text, in capitals under `\* Upper`; the
    /// others are rounded first. Word 16.0 (build 16.0.20527), 2026-10-26, for `=` fields with the
    /// same results.
    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn numbers_outside_a_format_show_words_error() {
        let shown = |instr: &str, fmt: NumFormat, value: f64| field_value(instr, fmt, value);
        let error = NUMBER_FORMAT_ERROR;
        assert_eq!(error, "Error! Number cannot be represented in specified format.");
        for (fmt, value) in [
            (NumFormat::CardinalText, 1_000_140.0),
            (NumFormat::CardinalText, 1_000_000.0),
            (NumFormat::CardinalText, 999_999.5),
            (NumFormat::CardinalText, -1.0),
            (NumFormat::CardinalText, -0.6),
            (NumFormat::OrdinalText, 1_000_000.0),
            (NumFormat::OrdinalText, -5.0),
            (NumFormat::LowerRoman, -5.0),
            (NumFormat::LowerRoman, 32_768.0),
            (NumFormat::UpperRoman, 50_000.0),
            (NumFormat::LowerRoman, 32_767.5),
            (NumFormat::LowerRoman, 1_000_000.0),
            (NumFormat::LowerLetter, -5.0),
            (NumFormat::LowerLetter, 781.0),
            (NumFormat::UpperLetter, 999_999.0),
            (NumFormat::Hex, -5.0),
            (NumFormat::Hex, 65_536.0),
            (NumFormat::Hex, 999_999.0),
            (NumFormat::Hex, 65_535.5),
            (NumFormat::DollarText, -1.0),
            (NumFormat::DollarText, 1_000_000.0),
            // From a half up to one, in size, Word shows the error for every format.
            (NumFormat::CardinalText, 0.5),
            (NumFormat::OrdinalText, 0.7),
            (NumFormat::LowerRoman, 0.6),
            (NumFormat::Hex, 0.9),
            (NumFormat::LowerLetter, 0.5),
            (NumFormat::NumberInDash, 0.5),
            (NumFormat::Ordinal, 0.9),
            (NumFormat::Decimal, 0.99),
            (NumFormat::Decimal, -0.99),
            (NumFormat::LowerRoman, -0.5),
            (NumFormat::CardinalText, f64::NAN),
        ] {
            assert_eq!(shown("=0", fmt, value), error, "{fmt:?} {value}");
        }
        assert_eq!(shown(r"=1000140 \* CardText \* Upper", NumFormat::CardinalText, 1_000_140.0), error.to_uppercase());
        // At the ends of each range.
        assert_eq!(shown("", NumFormat::CardinalText, 999_999.0), "nine hundred ninety-nine thousand nine hundred ninety-nine");
        assert_eq!(shown("", NumFormat::CardinalText, 999_999.4), "nine hundred ninety-nine thousand nine hundred ninety-nine");
        assert_eq!(shown("", NumFormat::OrdinalText, 999_999.0), "nine hundred ninety-nine thousand nine hundred ninety-ninth");
        assert_eq!(shown("", NumFormat::OrdinalText, 0.0), "zeroth");
        assert_eq!(shown("", NumFormat::CardinalText, 0.0), "zero");
        assert_eq!(shown("", NumFormat::CardinalText, -0.4), "zero");
        assert_eq!(shown("", NumFormat::OrdinalText, 0.4), "zeroth");
        assert_eq!(shown("", NumFormat::LowerRoman, 32_767.0), format!("{}dcclxvii", "m".repeat(32)));
        assert_eq!(shown("", NumFormat::LowerRoman, 32_767.4), format!("{}dcclxvii", "m".repeat(32)));
        assert_eq!(shown("", NumFormat::LowerRoman, 5000.0), "mmmmm");
        assert_eq!(shown("", NumFormat::UpperRoman, 1000.0), "M");
        assert_eq!(shown("", NumFormat::LowerRoman, 3999.0), "mmmcmxcix");
        assert_eq!(shown("", NumFormat::LowerLetter, 780.0), "z".repeat(30));
        assert_eq!(shown("", NumFormat::LowerLetter, 27.0), "aa");
        assert_eq!(shown("", NumFormat::UpperLetter, 53.0), "AAA");
        assert_eq!(shown("", NumFormat::Hex, 65_535.0), "FFFF");
        assert_eq!(shown("", NumFormat::Hex, 0.0), "0");
        assert_eq!(shown("", NumFormat::Hex, -0.4), "0");
        // Roman numerals and letters show a space for 0.
        for fmt in [NumFormat::LowerRoman, NumFormat::UpperLetter] {
            for value in [0.0, 0.49, -0.49] {
                assert_eq!(shown("", fmt, value), " ", "{fmt:?} {value}");
            }
        }
        // Rounding is half away from zero for every format.
        assert_eq!(shown("", NumFormat::CardinalText, 2.5), "three");
        assert_eq!(shown("", NumFormat::CardinalText, 2.6), "three");
        assert_eq!(shown("", NumFormat::OrdinalText, 2.5), "third");
        assert_eq!(shown("", NumFormat::LowerRoman, 2.5), "iii");
        assert_eq!(shown("", NumFormat::LowerRoman, 1.4), "i");
        assert_eq!(shown("", NumFormat::LowerRoman, 1.5), "ii");
        assert_eq!(shown("", NumFormat::Hex, 2.5), "3");
        // Arabic, ArabicDash and Ordinal write any number.
        assert_eq!(shown("", NumFormat::Decimal, 1_234_567.891), "1234568");
        assert_eq!(shown("", NumFormat::Decimal, 2.5), "3");
        assert_eq!(shown("", NumFormat::Decimal, -2.5), "-3");
        assert_eq!(shown("", NumFormat::Decimal, 0.4), "0");
        assert_eq!(shown("", NumFormat::NumberInDash, -5.0), "- -5 -");
        assert_eq!(shown("", NumFormat::NumberInDash, -1.4), "- -1 -");
        assert_eq!(shown("", NumFormat::NumberInDash, 2.5), "- 3 -");
        assert_eq!(shown("", NumFormat::NumberInDash, 1_000_000.0), "- 1000000 -");
        assert_eq!(shown("", NumFormat::Ordinal, 2.5), "3rd");
        assert_eq!(shown("", NumFormat::Ordinal, -1.0), "-1st");
        assert_eq!(shown("", NumFormat::Ordinal, 1_000_000.0), "1000000th");
        // Page-number fields use the same rules.
        assert_eq!(field_number(r"SEQ x \* Upper", NumFormat::CardinalText, 1_000_140), error.to_uppercase());
        assert_eq!(field_number("SEQ x", NumFormat::CardinalText, 999_999), "nine hundred ninety-nine thousand nine hundred ninety-nine");
    }

    #[test]
    fn unbounded_numeric_field_formats_accept_large_values() {
        let value = 1_000_000_000_000_001.0;
        assert_eq!(field_value("", NumFormat::Decimal, value), "1000000000000001");
        assert_eq!(field_value("", NumFormat::NumberInDash, value), "- 1000000000000001 -");
        assert_eq!(field_value("", NumFormat::Ordinal, value), "1000000000000001st");
        for (fmt, expected) in [
            (NumFormat::Decimal, "-9223372036854775808"),
            (NumFormat::NumberInDash, "- -9223372036854775808 -"),
            (NumFormat::Ordinal, "-9223372036854775808th"),
        ] {
            assert_eq!(field_value("", fmt, 2_f64.powi(63)), NUMBER_FORMAT_ERROR);
            assert_eq!(field_value("", fmt, -2_f64.powi(63)), expected);
        }
    }

    /// The text argument of a TITLE-like field: quoted text with a backslash making the next
    /// character plain, or one word; a switch or nothing after the name is no argument.
    #[test]
    fn text_arguments() {
        for (instr, text) in [
            (r#"TITLE "Quarterly report""#, Some("Quarterly report")),
            (r#" SUBJECT   "a \"q\" b" \* Upper"#, Some(r#"a "q" b"#)),
            (r#"TITLE "x \* Upper""#, Some("x * Upper")),
            (r#"TITLE "a \\ b""#, Some(r"a \ b")),
            (r#"TITLE "end\""#, Some("end\"")),
            (r#"TITLE "open"#, Some("open")),
            (r#"TITLE "trailing\"#, Some("trailing")),
            (r#"TITLE """#, Some("")),
            ("SUBJECT Draft two", Some("Draft")),
            ("TITLE", None),
            (r"TITLE \* Caps", None),
            ("", None),
        ] {
            assert_eq!(text_argument(instr).as_deref(), text, "{instr}");
        }
    }

    /// Several capitalization switches apply in the order they appear (Word 16.0, 2026-10-26).
    #[test]
    fn capitalization_switches_apply_in_order() {
        let ctx = FieldCtx { title: "WEEKLY REPORT".into(), ..Default::default() };
        let text = |instr: &str| field_text(instr, "", &ctx).0;
        assert_eq!(text(r"TITLE \* Lower \* Caps"), "Weekly Report");
        assert_eq!(text(r"TITLE \* Lower \* FirstCap"), "Weekly report");
        assert_eq!(text(r"TITLE \* Caps \* Lower"), "weekly report");
        assert_eq!(text(r"TITLE \* Lower \* MERGEFORMAT \* Upper"), "WEEKLY REPORT");
        assert_eq!(text(r#"TITLE "ABC DEF" \* Lower \* FirstCap"#), "Abc def");
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
