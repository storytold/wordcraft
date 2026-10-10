//! Section properties: page setup, columns, headers/footers, page numbering.

use serde::{Deserialize, Serialize};

use crate::props::{Borders, VAlign};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SectionStart {
    #[default]
    NextPage,
    Continuous,
    EvenPage,
    OddPage,
    NextColumn,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NumFormat {
    #[default]
    Decimal,
    UpperRoman,
    LowerRoman,
    UpperLetter,
    LowerLetter,
    Ordinal,
    CardinalText,
    OrdinalText,
    DecimalZero,
    Bullet,
    None,
    /// Arabic alphabet letters (OOXML `arabicAlpha`, MS-DOCX: أ، ب، ت …): positions 1–28,
    /// then repeated letters.
    ArabicAlpha,
    /// Abjad sequence (OOXML `arabicAbjad`, MS-DOCX: أ، ب، ج …): positions 1–28, then repeated
    /// letters (MS-OI29500 §17.18.59 (g)) — not additive abjad numerals.
    ArabicAbjad,
    /// Devanagari digits (OOXML `hindiNumbers`, MS-DOCX: U+0967 …): १२३. User-entered numbers
    /// are never rewritten; only counters render this way. There is no automatic numeral
    /// substitution: typed Arabic-Indic (U+0660…) or Persian (U+06F0…) digits are preserved as
    /// typed.
    HindiNumbers,
    /// Any other OOXML `ST_NumberFormat` value, kept so save/reopen preserves documents whose
    /// format isn't implemented yet; renders as decimal.
    Custom(String),
}

impl NumFormat {
    pub fn ooxml(&self) -> &str {
        match self {
            NumFormat::Decimal => "decimal",
            NumFormat::UpperRoman => "upperRoman",
            NumFormat::LowerRoman => "lowerRoman",
            NumFormat::UpperLetter => "upperLetter",
            NumFormat::LowerLetter => "lowerLetter",
            NumFormat::Ordinal => "ordinal",
            NumFormat::CardinalText => "cardinalText",
            NumFormat::OrdinalText => "ordinalText",
            NumFormat::DecimalZero => "decimalZero",
            NumFormat::Bullet => "bullet",
            NumFormat::None => "none",
            NumFormat::ArabicAlpha => "arabicAlpha",
            NumFormat::ArabicAbjad => "arabicAbjad",
            NumFormat::HindiNumbers => "hindiNumbers",
            NumFormat::Custom(s) => s,
        }
    }
    pub fn from_ooxml(s: &str) -> NumFormat {
        [
            NumFormat::Decimal,
            NumFormat::UpperRoman,
            NumFormat::LowerRoman,
            NumFormat::UpperLetter,
            NumFormat::LowerLetter,
            NumFormat::Ordinal,
            NumFormat::CardinalText,
            NumFormat::OrdinalText,
            NumFormat::DecimalZero,
            NumFormat::Bullet,
            NumFormat::None,
            NumFormat::ArabicAlpha,
            NumFormat::ArabicAbjad,
            NumFormat::HindiNumbers,
        ]
        .into_iter()
        .find(|f| f.ooxml() == s)
        .unwrap_or(NumFormat::Custom(s.to_string()))
    }
    /// Whether the format numbers items (anything but bullets and placeholders).
    pub fn is_ordered(&self) -> bool {
        *self != NumFormat::Bullet && *self != NumFormat::None
    }
    /// Format `n` (1-based).
    pub fn format(&self, n: u32) -> String {
        match self {
            NumFormat::Decimal => n.to_string(),
            NumFormat::DecimalZero => format!("{n:02}"),
            NumFormat::UpperRoman => roman(n).to_uppercase(),
            NumFormat::LowerRoman => roman(n),
            NumFormat::UpperLetter => letters(n).to_uppercase(),
            NumFormat::LowerLetter => letters(n),
            NumFormat::Ordinal => format!("{n}{}", ordinal_suffix(n)),
            NumFormat::CardinalText => cardinal_text(n),
            NumFormat::OrdinalText => ordinal_text(n),
            NumFormat::ArabicAlpha => arabic_alpha(n),
            NumFormat::ArabicAbjad => arabic_abjad(n),
            NumFormat::HindiNumbers => hindi_numbers(n),
            NumFormat::Bullet | NumFormat::None => String::new(),
            // An unimplemented format still counts: fall back to decimal digits.
            NumFormat::Custom(_) => n.to_string(),
        }
    }
}

fn ordinal_suffix(n: u32) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

/// Lowercase roman numerals (0 → empty; values above 3999 use repeated `m`).
pub fn roman(mut n: u32) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    n = n.min(50_000);
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// Word-style letters: a..z, aa..zz, aaa.. (repeated letter, not base-26).
pub fn letters(n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let n = n.min(780);
    let k = (n - 1) / 26 + 1;
    let c = (b'a' + ((n - 1) % 26) as u8) as char;
    std::iter::repeat_n(c, k as usize).collect()
}

/// The Arabic hija'i alphabet in `arabicAlpha` order (MS-DOCX: أ، ب، ت، … — starting with
/// alef with hamza, U+0623).
const AR_ALPHA: [char; 28] =
    ['أ', 'ب', 'ت', 'ث', 'ج', 'ح', 'خ', 'د', 'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', 'ط', 'ظ', 'ع', 'غ', 'ف', 'ق', 'ك', 'ل', 'م', 'ن', 'ه', 'و', 'ي'];

/// Arabic alphabet numbering: أ..ي, then repeated letters (`29` → `أأ`), as Word does
/// (MS-OI29500 §17.18.59 (h): the character is written once, then repeated).
pub fn arabic_alpha(n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let n = n.min(28 * 30);
    let k = (n - 1) / 28 + 1;
    let c = AR_ALPHA.get(((n - 1) % 28) as usize).copied().unwrap_or('أ');
    std::iter::repeat_n(c, k as usize).collect()
}

/// The abjad sequence in `arabicAbjad` order (MS-DOCX: أ، ب، ج، … — abjad positions, not the
/// hija'i alphabet): ا=1 … ط=9, ي=10 … ص=90, ق=100 … غ=1000, as positions 1–28.
const ABJAD: [char; 28] =
    ['أ', 'ب', 'ج', 'د', 'ه', 'و', 'ز', 'ح', 'ط', 'ي', 'ك', 'ل', 'م', 'ن', 'س', 'ع', 'ف', 'ص', 'ق', 'ر', 'ش', 'ت', 'ث', 'خ', 'ذ', 'ض', 'ظ', 'غ'];

/// Abjad numbering: the 28-letter sequence, then repeated letters (`29` → `أأ`), as Word does
/// (MS-OI29500 §17.18.59 (g)). This is the positional sequence, not additive abjad numerals.
pub fn arabic_abjad(n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let n = n.min(28 * 30);
    let k = (n - 1) / 28 + 1;
    let c = ABJAD.get(((n - 1) % 28) as usize).copied().unwrap_or('أ');
    std::iter::repeat_n(c, k as usize).collect()
}

/// Devanagari digits (U+0966..U+096F) for OOXML `hindiNumbers` (MS-DOCX): `123` → `१२३`.
pub fn hindi_numbers(n: u32) -> String {
    n.to_string().chars().map(|c| c.to_digit(10).map(|d| char::from_u32(0x0966 + d).unwrap_or(c)).unwrap_or(c)).collect()
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

/// English cardinal words, capitalised first letter (`Twenty-One`).
pub fn cardinal_text(n: u32) -> String {
    fn words(n: u32) -> String {
        match n {
            0..=19 => ONES.get(n as usize).copied().unwrap_or("").to_string(),
            20..=99 => {
                let t = TENS.get((n / 10) as usize).copied().unwrap_or("");
                if n.is_multiple_of(10) { t.to_string() } else { format!("{t}-{}", ONES.get((n % 10) as usize).copied().unwrap_or("")) }
            }
            100..=999 => {
                let h = format!("{} hundred", words(n / 100));
                if n.is_multiple_of(100) { h } else { format!("{h} {}", words(n % 100)) }
            }
            _ => {
                let th = format!("{} thousand", words(n / 1000));
                if n.is_multiple_of(1000) { th } else { format!("{th} {}", words(n % 1000)) }
            }
        }
    }
    capitalize(&words(n.min(999_999)))
}

/// English ordinal words (`Twenty-First`).
pub fn ordinal_text(n: u32) -> String {
    let c = cardinal_text(n).to_lowercase();
    let (head, last) = match c.rfind(['-', ' ']) {
        Some(i) => (c.get(..=i).unwrap_or(""), c.get(i + 1..).unwrap_or("")),
        None => ("", c.as_str()),
    };
    let last = match last {
        "one" => "first".to_string(),
        "two" => "second".to_string(),
        "three" => "third".to_string(),
        "five" => "fifth".to_string(),
        "eight" => "eighth".to_string(),
        "nine" => "ninth".to_string(),
        "twelve" => "twelfth".to_string(),
        l if l.ends_with('y') => format!("{}ieth", l.trim_end_matches('y')),
        l => format!("{l}th"),
    };
    capitalize(&format!("{head}{last}"))
}

fn capitalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut up = true;
    for c in s.chars() {
        if up {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        up = c == '-' || c == ' ';
    }
    out
}

/// Header/footer story ids (in `Document::parts`) for a section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HeaderSet {
    pub default: Option<u32>,
    pub first: Option<u32>,
    pub even: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Columns {
    pub count: u32,
    /// Space between columns, points.
    pub space: f32,
    /// Line between columns.
    pub separator: bool,
    /// Unequal widths (width, space after) per column; empty = equal.
    pub widths: Vec<(f32, f32)>,
}

impl Default for Columns {
    fn default() -> Self {
        Columns { count: 1, space: 36.0, separator: false, widths: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LineNumbering {
    pub count_by: u32,
    pub start: u32,
    /// Distance from text, points (0 = auto).
    pub distance: f32,
    /// Restart each page / section / continuous.
    pub restart: LineNumberRestart,
}

impl Default for LineNumbering {
    fn default() -> Self {
        LineNumbering { count_by: 1, start: 1, distance: 0.0, restart: LineNumberRestart::Page }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LineNumberRestart {
    #[default]
    Page,
    Section,
    Continuous,
}

/// Page setup and section-level settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SectionProps {
    pub start: SectionStart,
    /// Page width/height, points (already swapped for landscape).
    pub page_w: f32,
    pub page_h: f32,
    pub landscape: bool,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    /// Distance from the page edge to the header / footer.
    pub header: f32,
    pub footer: f32,
    pub gutter: f32,
    /// Mirror margins (inside/outside) — set from the document settings.
    pub columns: Columns,
    pub headers: HeaderSet,
    pub footers: HeaderSet,
    /// Different first page header/footer.
    pub title_page: bool,
    pub page_num_start: Option<u32>,
    pub page_num_format: NumFormat,
    pub line_numbers: Option<LineNumbering>,
    pub valign: VAlign,
    pub page_borders: Option<Borders>,
    /// Text direction / bidi section.
    pub rtl: bool,
    /// Tracked change of the section's properties (`w:sectPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<crate::props::PropChange<SectionProps>>>,
}

impl Default for SectionProps {
    /// Word's default: US Letter, 1" margins, 0.5" header/footer distance.
    fn default() -> Self {
        SectionProps {
            start: SectionStart::NextPage,
            page_w: 612.0,
            page_h: 792.0,
            landscape: false,
            margin_top: 72.0,
            margin_bottom: 72.0,
            margin_left: 72.0,
            margin_right: 72.0,
            header: 36.0,
            footer: 36.0,
            gutter: 0.0,
            columns: Columns::default(),
            headers: HeaderSet::default(),
            footers: HeaderSet::default(),
            title_page: false,
            page_num_start: None,
            page_num_format: NumFormat::Decimal,
            line_numbers: None,
            valign: VAlign::Top,
            page_borders: None,
            rtl: false,
            fmt_change: None,
        }
    }
}

impl SectionProps {
    /// Width of the text area, points.
    pub fn text_width(&self) -> f32 {
        (self.page_w - self.margin_left - self.margin_right - self.gutter).max(18.0)
    }
    pub fn text_height(&self) -> f32 {
        (self.page_h - self.margin_top - self.margin_bottom).max(18.0)
    }
    /// Swap to landscape/portrait (keeps margins relative to the paper as Word does: rotates them).
    pub fn set_landscape(&mut self, on: bool) {
        if on == self.landscape {
            return;
        }
        self.landscape = on;
        std::mem::swap(&mut self.page_w, &mut self.page_h);
        let (t, b, l, r) = (self.margin_top, self.margin_bottom, self.margin_left, self.margin_right);
        if on {
            self.margin_top = l;
            self.margin_bottom = r;
            self.margin_left = b;
            self.margin_right = t;
        } else {
            self.margin_left = t;
            self.margin_right = b;
            self.margin_bottom = l;
            self.margin_top = r;
        }
    }
    /// Column (x offset from the text area's left, width) for each column.
    pub fn column_boxes(&self) -> Vec<(f32, f32)> {
        let tw = self.text_width();
        let n = self.columns.count.clamp(1, 45) as usize;
        if !self.columns.widths.is_empty() && self.columns.widths.len() == n {
            let mut x = 0.0;
            return self
                .columns
                .widths
                .iter()
                .map(|(w, sp)| {
                    let b = (x, w.max(18.0));
                    x += w + sp;
                    b
                })
                .collect();
        }
        let space = self.columns.space.max(0.0);
        let w = ((tw - space * (n as f32 - 1.0)) / n as f32).max(18.0);
        (0..n).map(|i| (i as f32 * (w + space), w)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats() {
        assert_eq!(NumFormat::UpperRoman.format(1994), "MCMXCIV");
        assert_eq!(NumFormat::LowerLetter.format(27), "aa");
        assert_eq!(NumFormat::LowerLetter.format(0), "");
        assert_eq!(NumFormat::Ordinal.format(12), "12th");
        assert_eq!(NumFormat::Ordinal.format(22), "22nd");
        assert_eq!(NumFormat::CardinalText.format(21), "Twenty-One");
        assert_eq!(NumFormat::OrdinalText.format(21), "Twenty-First");
        assert_eq!(NumFormat::OrdinalText.format(20), "Twentieth");
        assert_eq!(NumFormat::OrdinalText.format(112), "One Hundred Twelfth");
        assert_eq!(NumFormat::DecimalZero.format(3), "03");
        assert!(!NumFormat::UpperRoman.format(u32::MAX).is_empty());
        assert_eq!(NumFormat::LowerLetter.format(u32::MAX).len(), 30);
    }

    #[test]
    fn arabic_number_formats() {
        // `arabicAlpha`: the hija'i alphabet from alef with hamza, then repeated letters.
        assert_eq!(NumFormat::ArabicAlpha.format(1), "أ");
        assert_eq!(NumFormat::ArabicAlpha.format(2), "ب");
        assert_eq!(NumFormat::ArabicAlpha.format(28), "ي");
        assert_eq!(NumFormat::ArabicAlpha.format(29), "أأ");
        assert_eq!(NumFormat::ArabicAlpha.format(0), "");
        // `arabicAbjad`: the abjad sequence (أ، ب، ج، …), then repeated letters — positional,
        // not additive numerals (MS-OI29500 §17.18.59 (g)).
        assert_eq!(NumFormat::ArabicAbjad.format(1), "أ");
        assert_eq!(NumFormat::ArabicAbjad.format(2), "ب");
        assert_eq!(NumFormat::ArabicAbjad.format(3), "ج");
        assert_eq!(NumFormat::ArabicAbjad.format(11), "ك");
        assert_eq!(NumFormat::ArabicAbjad.format(21), "ش");
        assert_eq!(NumFormat::ArabicAbjad.format(28), "غ");
        assert_eq!(NumFormat::ArabicAbjad.format(29), "أأ");
        assert_eq!(NumFormat::ArabicAbjad.format(0), "");
        // `hindiNumbers`: Devanagari digits (MS-DOCX); user text is never rewritten, only counters.
        assert_eq!(NumFormat::HindiNumbers.format(0), "०");
        assert_eq!(NumFormat::HindiNumbers.format(123), "१२३");
        assert_eq!(NumFormat::HindiNumbers.format(1403), "१४०३");
        // Unknown identifiers survive the round trip and count as decimal.
        assert_eq!(NumFormat::from_ooxml("thaiNumbers"), NumFormat::Custom("thaiNumbers".into()));
        assert_eq!(NumFormat::Custom("thaiNumbers".into()).ooxml(), "thaiNumbers");
        assert_eq!(NumFormat::Custom("thaiNumbers".into()).format(7), "7");
        assert!(NumFormat::Custom("thaiNumbers".into()).is_ordered());
        assert_eq!(NumFormat::from_ooxml("arabicAbjad"), NumFormat::ArabicAbjad);
        assert_eq!(NumFormat::ArabicAbjad.ooxml(), "arabicAbjad");
    }

    #[test]
    fn landscape_swaps() {
        let mut s = SectionProps { margin_left: 50.0, ..Default::default() };
        s.set_landscape(true);
        assert_eq!((s.page_w, s.page_h), (792.0, 612.0));
        assert_eq!(s.margin_top, 50.0);
        s.set_landscape(false);
        assert_eq!(s.margin_left, 50.0);
        assert_eq!((s.page_w, s.page_h), (612.0, 792.0));
    }

    #[test]
    fn columns_split() {
        let s = SectionProps { columns: Columns { count: 2, space: 36.0, ..Default::default() }, ..Default::default() };
        let b = s.column_boxes();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], (0.0, 216.0));
        assert_eq!(b[1], (252.0, 216.0));
    }
}
