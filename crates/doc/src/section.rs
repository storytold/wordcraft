//! Section properties: page setup, columns, headers/footers, page numbering.

use serde::{Deserialize, Serialize};

use crate::props::Borders;

/// How a section's text sits on its pages, top to bottom (`w:vAlign`, ECMA-376 §17.6.23,
/// ST_VerticalJc). Table cells and text boxes use [`crate::props::VAlign`], which has no
/// Justified.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PageVAlign {
    #[default]
    Top,
    Center,
    /// Justified (`both`): the room left on each page is shared out equally between the page's
    /// paragraphs and tables, so the text runs from the top margin to the bottom one.
    #[serde(alias = "both", alias = "justified")]
    Justify,
    Bottom,
}

impl PageVAlign {
    /// From a command or file value: `top|center|justify|bottom` (also `both`, `justified`,
    /// `middle`); anything else is Top.
    pub fn parse(v: &str) -> PageVAlign {
        match v {
            "center" | "middle" => PageVAlign::Center,
            "justify" | "justified" | "both" => PageVAlign::Justify,
            "bottom" => PageVAlign::Bottom,
            _ => PageVAlign::Top,
        }
    }
}

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

/// A number format (ECMA-376 §17.18.59 ST_NumberFormat). Every value the format defines is
/// kept, so a document saves back the format it was opened with even where WordCraft draws it as
/// plain decimal (see [`NumFormat::format`]).
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
    Hex,
    Chicago,
    IdeographDigital,
    JapaneseCounting,
    Aiueo,
    Iroha,
    DecimalFullWidth,
    DecimalHalfWidth,
    JapaneseLegal,
    JapaneseDigitalTenThousand,
    DecimalEnclosedCircle,
    DecimalFullWidth2,
    AiueoFullWidth,
    IrohaFullWidth,
    Ganada,
    Chosung,
    DecimalEnclosedFullstop,
    DecimalEnclosedParen,
    DecimalEnclosedCircleChinese,
    IdeographEnclosedCircle,
    IdeographTraditional,
    IdeographZodiac,
    IdeographZodiacTraditional,
    TaiwaneseCounting,
    IdeographLegalTraditional,
    TaiwaneseCountingThousand,
    TaiwaneseDigital,
    ChineseCounting,
    ChineseLegalSimplified,
    ChineseCountingThousand,
    KoreanDigital,
    KoreanCounting,
    KoreanLegal,
    KoreanDigital2,
    VietnameseCounting,
    RussianLower,
    RussianUpper,
    NumberInDash,
    Hebrew1,
    Hebrew2,
    ArabicAlpha,
    ArabicAbjad,
    HindiVowels,
    HindiConsonants,
    HindiNumbers,
    HindiCounting,
    ThaiLetters,
    ThaiNumbers,
    ThaiCounting,
    BahtText,
    DollarText,
}

impl NumFormat {
    /// Every format, in the order ST_NumberFormat lists them (which is also the order of the
    /// binary .doc number format codes up to 0x27).
    pub const ALL: [NumFormat; 62] = [
        NumFormat::Decimal,
        NumFormat::UpperRoman,
        NumFormat::LowerRoman,
        NumFormat::UpperLetter,
        NumFormat::LowerLetter,
        NumFormat::Ordinal,
        NumFormat::CardinalText,
        NumFormat::OrdinalText,
        NumFormat::Hex,
        NumFormat::Chicago,
        NumFormat::IdeographDigital,
        NumFormat::JapaneseCounting,
        NumFormat::Aiueo,
        NumFormat::Iroha,
        NumFormat::DecimalFullWidth,
        NumFormat::DecimalHalfWidth,
        NumFormat::JapaneseLegal,
        NumFormat::JapaneseDigitalTenThousand,
        NumFormat::DecimalEnclosedCircle,
        NumFormat::DecimalFullWidth2,
        NumFormat::AiueoFullWidth,
        NumFormat::IrohaFullWidth,
        NumFormat::DecimalZero,
        NumFormat::Bullet,
        NumFormat::Ganada,
        NumFormat::Chosung,
        NumFormat::DecimalEnclosedFullstop,
        NumFormat::DecimalEnclosedParen,
        NumFormat::DecimalEnclosedCircleChinese,
        NumFormat::IdeographEnclosedCircle,
        NumFormat::IdeographTraditional,
        NumFormat::IdeographZodiac,
        NumFormat::IdeographZodiacTraditional,
        NumFormat::TaiwaneseCounting,
        NumFormat::IdeographLegalTraditional,
        NumFormat::TaiwaneseCountingThousand,
        NumFormat::TaiwaneseDigital,
        NumFormat::ChineseCounting,
        NumFormat::ChineseLegalSimplified,
        NumFormat::ChineseCountingThousand,
        NumFormat::KoreanDigital,
        NumFormat::KoreanCounting,
        NumFormat::KoreanLegal,
        NumFormat::KoreanDigital2,
        NumFormat::VietnameseCounting,
        NumFormat::RussianLower,
        NumFormat::RussianUpper,
        NumFormat::None,
        NumFormat::NumberInDash,
        NumFormat::Hebrew1,
        NumFormat::Hebrew2,
        NumFormat::ArabicAlpha,
        NumFormat::ArabicAbjad,
        NumFormat::HindiVowels,
        NumFormat::HindiConsonants,
        NumFormat::HindiNumbers,
        NumFormat::HindiCounting,
        NumFormat::ThaiLetters,
        NumFormat::ThaiNumbers,
        NumFormat::ThaiCounting,
        NumFormat::BahtText,
        NumFormat::DollarText,
    ];

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
            NumFormat::Hex => "hex",
            NumFormat::Chicago => "chicago",
            NumFormat::IdeographDigital => "ideographDigital",
            NumFormat::JapaneseCounting => "japaneseCounting",
            NumFormat::Aiueo => "aiueo",
            NumFormat::Iroha => "iroha",
            NumFormat::DecimalFullWidth => "decimalFullWidth",
            NumFormat::DecimalHalfWidth => "decimalHalfWidth",
            NumFormat::JapaneseLegal => "japaneseLegal",
            NumFormat::JapaneseDigitalTenThousand => "japaneseDigitalTenThousand",
            NumFormat::DecimalEnclosedCircle => "decimalEnclosedCircle",
            NumFormat::DecimalFullWidth2 => "decimalFullWidth2",
            NumFormat::AiueoFullWidth => "aiueoFullWidth",
            NumFormat::IrohaFullWidth => "irohaFullWidth",
            NumFormat::Ganada => "ganada",
            NumFormat::Chosung => "chosung",
            NumFormat::DecimalEnclosedFullstop => "decimalEnclosedFullstop",
            NumFormat::DecimalEnclosedParen => "decimalEnclosedParen",
            NumFormat::DecimalEnclosedCircleChinese => "decimalEnclosedCircleChinese",
            NumFormat::IdeographEnclosedCircle => "ideographEnclosedCircle",
            NumFormat::IdeographTraditional => "ideographTraditional",
            NumFormat::IdeographZodiac => "ideographZodiac",
            NumFormat::IdeographZodiacTraditional => "ideographZodiacTraditional",
            NumFormat::TaiwaneseCounting => "taiwaneseCounting",
            NumFormat::IdeographLegalTraditional => "ideographLegalTraditional",
            NumFormat::TaiwaneseCountingThousand => "taiwaneseCountingThousand",
            NumFormat::TaiwaneseDigital => "taiwaneseDigital",
            NumFormat::ChineseCounting => "chineseCounting",
            NumFormat::ChineseLegalSimplified => "chineseLegalSimplified",
            NumFormat::ChineseCountingThousand => "chineseCountingThousand",
            NumFormat::KoreanDigital => "koreanDigital",
            NumFormat::KoreanCounting => "koreanCounting",
            NumFormat::KoreanLegal => "koreanLegal",
            NumFormat::KoreanDigital2 => "koreanDigital2",
            NumFormat::VietnameseCounting => "vietnameseCounting",
            NumFormat::RussianLower => "russianLower",
            NumFormat::RussianUpper => "russianUpper",
            NumFormat::NumberInDash => "numberInDash",
            NumFormat::Hebrew1 => "hebrew1",
            NumFormat::Hebrew2 => "hebrew2",
            NumFormat::ArabicAlpha => "arabicAlpha",
            NumFormat::ArabicAbjad => "arabicAbjad",
            NumFormat::HindiVowels => "hindiVowels",
            NumFormat::HindiConsonants => "hindiConsonants",
            NumFormat::HindiNumbers => "hindiNumbers",
            NumFormat::HindiCounting => "hindiCounting",
            NumFormat::ThaiLetters => "thaiLetters",
            NumFormat::ThaiNumbers => "thaiNumbers",
            NumFormat::ThaiCounting => "thaiCounting",
            NumFormat::BahtText => "bahtText",
            NumFormat::DollarText => "dollarText",
        }
    }
    /// The format an ST_NumberFormat value names; a value the format doesn't define is decimal
    /// (as Word shows it). So is `custom` (a Word 2010 format string, always written with a
    /// decimal fallback).
    pub fn from_ooxml(s: &str) -> NumFormat {
        NumFormat::ALL.into_iter().find(|f| f.ooxml() == s).unwrap_or(NumFormat::Decimal)
    }
    /// Format `n` (1-based). Formats WordCraft doesn't draw yet (Hebrew, Hindi,
    /// Vietnamese, text amounts) show decimal, as do values past a sequence's end
    /// (the 21st item of ①…⑳).
    pub fn format(self, n: u32) -> String {
        use crate::cjk_numbers as cjk;
        match self {
            NumFormat::DecimalZero => format!("{n:02}"),
            NumFormat::UpperRoman => roman(n).to_uppercase(),
            NumFormat::LowerRoman => roman(n),
            NumFormat::UpperLetter => letters(n).to_uppercase(),
            NumFormat::LowerLetter => letters(n),
            NumFormat::RussianUpper => russian_letters(n).to_uppercase(),
            NumFormat::RussianLower => russian_letters(n),
            NumFormat::ArabicAlpha => repeated_letter(&ARABIC_ALPHA, n),
            NumFormat::ArabicAbjad => repeated_letter(&ARABIC_ABJAD, n),
            NumFormat::Ordinal => format!("{n}{}", ordinal_suffix(n)),
            NumFormat::CardinalText => cardinal_text(n),
            NumFormat::OrdinalText => ordinal_text(n),
            NumFormat::Bullet | NumFormat::None => String::new(),
            NumFormat::Chicago => chicago(n),
            NumFormat::NumberInDash => format!("- {n} -"),
            NumFormat::IdeographDigital | NumFormat::KoreanDigital2 | NumFormat::TaiwaneseDigital => cjk::ideograph_digital(n),
            NumFormat::JapaneseCounting => cjk::japanese_counting(n),
            NumFormat::JapaneseLegal => cjk::japanese_legal(n),
            NumFormat::JapaneseDigitalTenThousand => cjk::japanese_digital_ten_thousand(n),
            NumFormat::Aiueo => cjk::aiueo(n),
            NumFormat::AiueoFullWidth => cjk::aiueo_full_width(n),
            NumFormat::Iroha => cjk::iroha(n),
            NumFormat::IrohaFullWidth => cjk::iroha_full_width(n),
            NumFormat::DecimalFullWidth | NumFormat::DecimalFullWidth2 => cjk::decimal_full_width(n),
            NumFormat::DecimalEnclosedCircle => cjk::decimal_enclosed_circle(n),
            NumFormat::DecimalEnclosedCircleChinese => cjk::decimal_enclosed_circle_chinese(n),
            NumFormat::DecimalEnclosedFullstop => cjk::decimal_enclosed_fullstop(n),
            NumFormat::DecimalEnclosedParen => cjk::decimal_enclosed_paren(n),
            NumFormat::IdeographEnclosedCircle => cjk::ideograph_enclosed_circle(n),
            NumFormat::IdeographTraditional => cjk::ideograph_traditional(n),
            NumFormat::IdeographZodiac => cjk::ideograph_zodiac(n),
            NumFormat::IdeographZodiacTraditional => cjk::ideograph_zodiac_traditional(n),
            NumFormat::IdeographLegalTraditional => cjk::ideograph_legal_traditional(n),
            NumFormat::TaiwaneseCounting => cjk::taiwanese_counting(n),
            NumFormat::TaiwaneseCountingThousand => cjk::taiwanese_counting_thousand(n),
            NumFormat::ChineseCounting => cjk::chinese_counting(n),
            NumFormat::ChineseCountingThousand => cjk::chinese_counting_thousand(n),
            NumFormat::ChineseLegalSimplified => cjk::chinese_legal_simplified(n),
            NumFormat::KoreanCounting => cjk::korean_counting(n),
            NumFormat::KoreanDigital => cjk::korean_digital(n),
            NumFormat::KoreanLegal => cjk::korean_legal(n),
            NumFormat::Ganada => cjk::ganada(n),
            NumFormat::Chosung => cjk::chosung(n),
            NumFormat::Decimal
            | NumFormat::DecimalHalfWidth
            | NumFormat::Hex
            | NumFormat::VietnameseCounting
            | NumFormat::Hebrew1
            | NumFormat::Hebrew2
            | NumFormat::HindiVowels
            | NumFormat::HindiConsonants
            | NumFormat::HindiNumbers
            | NumFormat::HindiCounting
            | NumFormat::ThaiLetters
            | NumFormat::DollarText => n.to_string(),
            NumFormat::ThaiNumbers => crate::thai_numbers::thai_digits(n),
            NumFormat::ThaiCounting => crate::thai_numbers::thai_counting_int(n),
            NumFormat::BahtText => crate::thai_numbers::baht_text_int(n),
        }
    }
}

/// Thai Baht words for a floating-point amount (used by the `{ BAHTTEXT … }` field).
/// Returns `None` for non-finite values.
pub fn baht_text_f64(x: f64) -> Option<String> {
    crate::thai_numbers::baht_text_f64(x)
}

/// Parse a decimal token (e.g. `"1.005"`) directly from the field instruction text,
/// rounding to satang (2 d.p.) at the string boundary to avoid f64 precision loss.
pub fn parse_baht_token(s: &str) -> Option<f64> {
    crate::thai_numbers::parse_baht_token(s)
}

/// Chicago Manual of Style note marks: *, †, ‡, §, then doubled (**, ††, …) and so on.
fn chicago(n: u32) -> String {
    const MARKS: [char; 4] = ['*', '\u{2020}', '\u{2021}', '\u{a7}'];
    if n == 0 {
        return String::new();
    }
    let i = (n - 1) as usize;
    let k = (i / 4 + 1).min(10);
    std::iter::repeat_n(MARKS.get(i % 4).copied().unwrap_or('*'), k).collect()
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

/// The 29 letters Word counts Russian lists with: а..я without ё, й, ъ and ь (ы stays).
const RUSSIAN_LETTERS: [char; 29] =
    ['а', 'б', 'в', 'г', 'д', 'е', 'ж', 'з', 'и', 'к', 'л', 'м', 'н', 'о', 'п', 'р', 'с', 'т', 'у', 'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'ы', 'э', 'ю', 'я'];

/// Word-style Russian letters: а..я, аа..яя, ааа.. (repeated letter, as [`letters`]).
pub fn russian_letters(n: u32) -> String {
    if n == 0 {
        return String::new();
    }
    let len = RUSSIAN_LETTERS.len() as u32;
    let n = n.min(len * 30);
    let k = (n - 1) / len + 1;
    let c = RUSSIAN_LETTERS.get(((n - 1) % len) as usize).copied().unwrap_or('а');
    std::iter::repeat_n(c, k as usize).collect()
}

/// `arabicAlpha`: the 28 letters in alphabetical (hijā'ī) order.
const ARABIC_ALPHA: [char; 28] =
    ['أ', 'ب', 'ت', 'ث', 'ج', 'ح', 'خ', 'د', 'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', 'ط', 'ظ', 'ع', 'غ', 'ف', 'ق', 'ك', 'ل', 'م', 'ن', 'ه', 'و', 'ي'];

/// `arabicAbjad`: the 28 letters in abjad order (أبجد هوز حطي كلمن سعفص قرشت ثخذ ضظغ).
const ARABIC_ABJAD: [char; 28] =
    ['أ', 'ب', 'ج', 'د', 'ه', 'و', 'ز', 'ح', 'ط', 'ي', 'ك', 'ل', 'م', 'ن', 'س', 'ع', 'ف', 'ص', 'ق', 'ر', 'ش', 'ت', 'ث', 'خ', 'ذ', 'ض', 'ظ', 'غ'];

/// One letter of `alphabet`, repeated once per pass through it (as [`letters`]: a..z, aa..zz).
fn repeated_letter(alphabet: &[char], n: u32) -> String {
    let Some(&first) = alphabet.first() else { return n.to_string() };
    if n == 0 {
        return String::new();
    }
    let len = alphabet.len() as u32;
    let n = n.min(len * 30);
    let k = (n - 1) / len + 1;
    let c = alphabet.get(((n - 1) % len) as usize).copied().unwrap_or(first);
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

/// Document grid kind (`w:docGrid/@w:type`, ECMA-376 §17.6.5, §17.18.14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DocGridType {
    /// No grid.
    #[default]
    Default,
    /// Line grid only: line heights snap to the line pitch.
    Lines,
    /// Line and character grid: East Asian characters advance by the character pitch.
    LinesAndChars,
    /// Line grid, and all text snaps to the character grid.
    SnapToChars,
}

impl DocGridType {
    pub fn ooxml(self) -> &'static str {
        match self {
            DocGridType::Default => "default",
            DocGridType::Lines => "lines",
            DocGridType::LinesAndChars => "linesAndChars",
            DocGridType::SnapToChars => "snapToChars",
        }
    }
    pub fn from_ooxml(s: &str) -> Option<DocGridType> {
        [DocGridType::Default, DocGridType::Lines, DocGridType::LinesAndChars, DocGridType::SnapToChars].into_iter().find(|t| t.ooxml() == s)
    }
}

/// Smallest and largest line or character pitch the layout uses, points.
pub const MIN_GRID_PITCH: f32 = 1.0;
pub const MAX_GRID_PITCH: f32 = 1584.0;
/// `w:charSpace` bound (4096ths of a point): ±1584 pt.
pub const MAX_CHAR_SPACE: i32 = 1584 * 4096;

/// The document grid (`w:docGrid`): lines per page and characters per line, as pitches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocGrid {
    #[serde(rename = "type")]
    pub kind: DocGridType,
    /// Distance between grid lines, points (`w:linePitch`, twips in the file).
    pub line_pitch: f32,
    /// Added to the default font size to give the character pitch, 4096ths of a point
    /// (`w:charSpace`).
    pub char_space: i32,
}

impl Default for DocGrid {
    /// Word's grid for a new document: no grid, an 18 pt pitch kept for when one is turned on.
    fn default() -> Self {
        DocGrid { kind: DocGridType::Default, line_pitch: 18.0, char_space: 0 }
    }
}

impl DocGrid {
    /// The line pitch, clamped to something layout can use.
    pub fn line_pitch(&self) -> f32 {
        if self.line_pitch.is_finite() { self.line_pitch.clamp(MIN_GRID_PITCH, MAX_GRID_PITCH) } else { 18.0 }
    }
    /// The character pitch for a document whose default font is `font_size` points.
    pub fn char_pitch(&self, font_size: f32) -> f32 {
        let size = if font_size.is_finite() { font_size.clamp(MIN_GRID_PITCH, MAX_GRID_PITCH) } else { 10.5 };
        (size + self.char_space.clamp(-MAX_CHAR_SPACE, MAX_CHAR_SPACE) as f32 / 4096.0).clamp(MIN_GRID_PITCH, MAX_GRID_PITCH)
    }
    /// The `w:charSpace` that gives `pitch` with the default font at `font_size` points.
    pub fn char_space_for(pitch: f32, font_size: f32) -> i32 {
        let pitch = if pitch.is_finite() { pitch.clamp(MIN_GRID_PITCH, MAX_GRID_PITCH) } else { font_size };
        let size = if font_size.is_finite() { font_size.clamp(MIN_GRID_PITCH, MAX_GRID_PITCH) } else { 10.5 };
        (((pitch - size) * 4096.0).round() as i32).clamp(-MAX_CHAR_SPACE, MAX_CHAR_SPACE)
    }
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
    /// The file carries a `w:pgNumType` element that sets nothing else (no start, decimal
    /// format), so saving writes it back rather than dropping it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub page_num_set: bool,
    pub line_numbers: Option<LineNumbering>,
    pub valign: PageVAlign,
    /// Form protection of the section (`w:formProt`, ECMA-376 §17.6.6): `Some` when the file
    /// states it. Kept so saving writes it back; editing does not act on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_prot: Option<bool>,
    pub page_borders: Option<Borders>,
    /// Text direction / bidi section.
    pub rtl: bool,
    /// Which way the section's text runs (`w:textDirection`, ECMA-376 §17.6.20): vertical
    /// sections turn the page's text area (headers and footers stay horizontal).
    #[serde(skip_serializing_if = "is_horizontal")]
    pub text_direction: crate::props::TextDirection,
    /// The file states the horizontal direction (`lrTb`) outright; saving writes it back.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub text_direction_set: bool,
    /// Tracked change of the section's properties (`w:sectPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<crate::props::PropChange<SectionProps>>>,
    /// Document grid (`w:docGrid`); `None` when the file has none (no grid).
    pub doc_grid: Option<DocGrid>,
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
            page_num_set: false,
            line_numbers: None,
            valign: PageVAlign::Top,
            form_prot: None,
            page_borders: None,
            rtl: false,
            text_direction: crate::props::TextDirection::Horizontal,
            text_direction_set: false,
            fmt_change: None,
            doc_grid: None,
            footnote_pr: NoteProps::default(),
            endnote_pr: NoteProps::default(),
        }
    }
}

fn is_horizontal(d: &crate::props::TextDirection) -> bool {
    !d.is_turned()
}

impl SectionProps {
    /// Join the section before this one into it, as when the break between them is deleted:
    /// this section's setup stays, as in Word, and the headers and footers it leaves out (linked
    /// to the previous section) are taken from `before`, so its text keeps showing them.
    pub fn absorb(&mut self, before: &SectionProps) {
        for (own, prev) in [(&mut self.headers, &before.headers), (&mut self.footers, &before.footers)] {
            own.default = own.default.or(prev.default);
            own.first = own.first.or(prev.first);
            own.even = own.even.or(prev.even);
        }
    }
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
    /// The line grid's pitch, points, when the section has a line grid (any grid but "no grid").
    pub fn grid_line_pitch(&self) -> Option<f32> {
        self.doc_grid.filter(|g| g.kind != DocGridType::Default).map(|g| g.line_pitch())
    }
    /// The character grid's pitch, points, when the section has one (`linesAndChars`,
    /// `snapToChars`); `font_size` is the default font size (Normal style).
    pub fn grid_char_pitch(&self, font_size: f32) -> Option<f32> {
        self.doc_grid.filter(|g| matches!(g.kind, DocGridType::LinesAndChars | DocGridType::SnapToChars)).map(|g| g.char_pitch(font_size))
    }
    /// Lines per page the line pitch gives (at least 1).
    pub fn grid_lines_per_page(&self) -> u32 {
        let pitch = self.doc_grid.unwrap_or_default().line_pitch();
        ((self.text_height() / pitch + 0.001).floor() as u32).max(1)
    }
    /// Characters per line the character pitch gives (at least 1).
    pub fn grid_chars_per_line(&self, font_size: f32) -> u32 {
        let pitch = self.doc_grid.unwrap_or_default().char_pitch(font_size);
        ((self.text_width() / pitch + 0.001).floor() as u32).max(1)
    }
    /// Column (x offset from the text area's left, width) for each column, in reading order:
    /// in a right-to-left section (`rtl`, `w:bidi`) the first column stands at the right edge
    /// and the columns (with the space after each) run leftwards.
    pub fn column_boxes(&self) -> Vec<(f32, f32)> {
        let tw = self.text_width();
        let n = self.columns.count.clamp(1, 45) as usize;
        let boxes: Vec<(f32, f32)> = if !self.columns.widths.is_empty() && self.columns.widths.len() == n {
            let mut x = 0.0;
            self.columns
                .widths
                .iter()
                .map(|(w, sp)| {
                    let b = (x, w.max(18.0));
                    x += w + sp;
                    b
                })
                .collect()
        } else {
            let space = self.columns.space.max(0.0);
            let w = ((tw - space * (n as f32 - 1.0)) / n as f32).max(18.0);
            (0..n).map(|i| (i as f32 * (w + space), w)).collect()
        };
        if self.rtl { boxes.into_iter().map(|(x, w)| (tw - x - w, w)).collect() } else { boxes }
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
        assert_eq!([NumFormat::Chicago.format(1), NumFormat::Chicago.format(6)], ["*", "\u{2020}\u{2020}"]);
        assert_eq!(NumFormat::ChineseCounting.format(11), "十一");
        assert_eq!(NumFormat::ChineseCountingThousand.format(101), "一百零一");
        assert_eq!(NumFormat::DecimalEnclosedCircle.format(3), "③");
        assert_eq!(NumFormat::Hebrew1.format(3), "3", "formats not drawn yet show decimal");
        assert_eq!(NumFormat::RussianLower.format(0), "");
        assert_eq!(NumFormat::RussianUpper.format(u32::MAX).chars().count(), 30);
        for f in NumFormat::ALL {
            assert_eq!(NumFormat::from_ooxml(f.ooxml()), f, "{f:?} round-trips");
            assert!(f == NumFormat::Bullet || f == NumFormat::None || !f.format(u32::MAX).is_empty(), "{f:?}");
        }
        assert_eq!(NumFormat::from_ooxml("klingon"), NumFormat::Decimal, "unknown formats are decimal");
    }

    #[test]
    fn russian_letters_follow_word() {
        // Word's own labels for a russianLower list: no ё, й, ъ or ь, then doubled letters.
        let labels: Vec<String> = (1..=31).map(|n| NumFormat::RussianLower.format(n)).collect();
        assert_eq!(labels.join(" "), "а б в г д е ж з и к л м н о п р с т у ф х ц ч ш щ ы э ю я аа бб");
        assert_eq!(NumFormat::RussianLower.format(59), "ааа");
        assert_eq!(NumFormat::RussianLower.format(100), "нннн");
        assert_eq!(NumFormat::RussianUpper.format(26), "Ы");
        assert_eq!(NumFormat::RussianUpper.format(36), "ЖЖ");
    }

    #[test]
    fn arabic_letters_count_in_both_orders() {
        let alpha: Vec<String> = (1..=5).map(|n| NumFormat::ArabicAlpha.format(n)).collect();
        assert_eq!(alpha.join(" "), "أ ب ت ث ج");
        let abjad: Vec<String> = (1..=5).map(|n| NumFormat::ArabicAbjad.format(n)).collect();
        assert_eq!(abjad.join(" "), "أ ب ج د ه");
        assert_eq!(NumFormat::ArabicAlpha.format(28), "ي");
        assert_eq!(NumFormat::ArabicAbjad.format(28), "غ");
        // Past the 28th letter they double, as Latin and Russian letters do.
        assert_eq!(NumFormat::ArabicAlpha.format(29), "أأ");
        assert_eq!(NumFormat::ArabicAbjad.format(30), "بب");
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
    fn grid_pitches_are_clamped() {
        let mut s = SectionProps::default();
        assert_eq!(s.grid_line_pitch(), None);
        s.doc_grid = Some(DocGrid { kind: DocGridType::Lines, line_pitch: 0.0, char_space: i32::MIN });
        assert_eq!(s.grid_line_pitch(), Some(MIN_GRID_PITCH));
        assert_eq!(s.grid_char_pitch(10.5), None);
        s.doc_grid = Some(DocGrid { kind: DocGridType::LinesAndChars, line_pitch: f32::NAN, char_space: i32::MIN });
        assert_eq!(s.grid_line_pitch(), Some(18.0));
        assert_eq!(s.grid_char_pitch(10.5), Some(MIN_GRID_PITCH));
        // 648 pt of text height at an 18 pt pitch: 36 lines; 468 pt / 13 pt = 36 characters.
        s.doc_grid = Some(DocGrid { kind: DocGridType::LinesAndChars, line_pitch: 18.0, char_space: DocGrid::char_space_for(13.0, 10.5) });
        assert_eq!(s.grid_lines_per_page(), 36);
        assert_eq!(s.grid_chars_per_line(10.5), 36);
    }

    #[test]
    fn columns_split() {
        let s = SectionProps { columns: Columns { count: 2, space: 36.0, ..Default::default() }, ..Default::default() };
        let b = s.column_boxes();
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], (0.0, 216.0));
        assert_eq!(b[1], (252.0, 216.0));
    }

    #[test]
    fn rtl_columns_run_right_to_left() {
        let s = SectionProps { rtl: true, columns: Columns { count: 2, space: 36.0, ..Default::default() }, ..Default::default() };
        assert_eq!(s.column_boxes(), vec![(252.0, 216.0), (0.0, 216.0)]);
        let s = SectionProps {
            rtl: true,
            columns: Columns { count: 2, widths: vec![(100.0, 20.0), (200.0, 0.0)], ..Default::default() },
            ..Default::default()
        };
        // Text width 468: the first column hugs the right edge, the second sits 20pt to its left.
        assert_eq!(s.column_boxes(), vec![(368.0, 100.0), (148.0, 200.0)]);
    }
}
