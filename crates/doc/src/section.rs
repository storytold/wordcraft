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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
}

impl NumFormat {
    pub fn ooxml(self) -> &'static str {
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
        ]
        .into_iter()
        .find(|f| f.ooxml() == s)
        .unwrap_or(NumFormat::Decimal)
    }
    /// Format `n` (1-based).
    pub fn format(self, n: u32) -> String {
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
            NumFormat::Bullet | NumFormat::None => String::new(),
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

/// Where footnotes or endnotes are placed (ECMA-376 §17.18.33/§17.18.20, `ST_FtnPos`/`ST_EdnPos`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NotePos {
    /// Footnotes: at the bottom of the page.
    #[default]
    PageBottom,
    /// Footnotes: right below the page's last line of text.
    BeneathText,
    /// Endnotes: at the end of each section.
    SectEnd,
    /// Endnotes: at the end of the document.
    DocEnd,
}

impl NotePos {
    pub fn ooxml(self) -> &'static str {
        match self {
            NotePos::PageBottom => "pageBottom",
            NotePos::BeneathText => "beneathText",
            NotePos::SectEnd => "sectEnd",
            NotePos::DocEnd => "docEnd",
        }
    }
    pub fn from_ooxml(s: &str) -> Option<NotePos> {
        [NotePos::PageBottom, NotePos::BeneathText, NotePos::SectEnd, NotePos::DocEnd].into_iter().find(|p| p.ooxml() == s)
    }
}

/// When note numbering starts again (ECMA-376 §17.18.66, `ST_RestartNumber`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NoteRestart {
    #[default]
    Continuous,
    EachSect,
    EachPage,
}

impl NoteRestart {
    pub fn ooxml(self) -> &'static str {
        match self {
            NoteRestart::Continuous => "continuous",
            NoteRestart::EachSect => "eachSect",
            NoteRestart::EachPage => "eachPage",
        }
    }
    pub fn from_ooxml(s: &str) -> Option<NoteRestart> {
        [NoteRestart::Continuous, NoteRestart::EachSect, NoteRestart::EachPage].into_iter().find(|r| r.ooxml() == s)
    }
}

/// The highest note number "start at" takes (Word's own limit is lower; this keeps hostile files
/// from overflowing the counters).
pub const MAX_NOTE_START: u32 = 32_767;

/// Footnote or endnote options as a document or a section states them (ECMA-376 §17.11.11 and
/// §17.11.4, `w:footnotePr` / `w:endnotePr`): unset fields come from the level above (a
/// section's from the document's, the document's from Word's defaults). The document's number
/// format is `Settings::footnote_format` / `endnote_format`, so its `num_fmt` stays unset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NoteProps {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos: Option<NotePos>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_fmt: Option<NumFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_restart: Option<NoteRestart>,
}

impl NoteProps {
    pub fn is_empty(&self) -> bool {
        *self == NoteProps::default()
    }
}

/// Footnote or endnote options in effect for a section (see [`crate::Document::note_options`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteOptions {
    pub pos: NotePos,
    pub num_fmt: NumFormat,
    /// 1..=[`MAX_NOTE_START`].
    pub num_start: u32,
    pub num_restart: NoteRestart,
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
    /// The section's own footnote and endnote options (unset: the document's).
    #[serde(skip_serializing_if = "NoteProps::is_empty")]
    pub footnote_pr: NoteProps,
    #[serde(skip_serializing_if = "NoteProps::is_empty")]
    pub endnote_pr: NoteProps,
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
            footnote_pr: NoteProps::default(),
            endnote_pr: NoteProps::default(),
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
