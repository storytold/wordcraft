//! Character, paragraph, table and cell properties.
//!
//! Every field is an `Option`: `None` means "inherit" (from the style chain, then document
//! defaults). [`CharProps::overlay`] and friends apply a patch on top of a base.

use serde::{Deserialize, Serialize};

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);
    /// `RRGGBB` (no `#`).
    pub fn hex(self) -> String {
        format!("{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
    /// Parse `RRGGBB` or `#RRGGBB`.
    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let p = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
        Some(Rgb(p(0)?, p(2)?, p(4)?))
    }
    pub fn luma(self) -> f32 {
        0.299 * self.0 as f32 + 0.587 * self.1 as f32 + 0.114 * self.2 as f32
    }
}

/// A text colour: automatic (black on light backgrounds, white on dark) or a fixed colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TextColor {
    #[default]
    Auto,
    Rgb(Rgb),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Underline {
    #[default]
    None,
    Single,
    Words,
    Double,
    Thick,
    Dotted,
    Dash,
    DotDash,
    DotDotDash,
    Wave,
    DoubleWave,
}

impl Underline {
    pub const ALL: [Underline; 11] = [
        Underline::None,
        Underline::Single,
        Underline::Words,
        Underline::Double,
        Underline::Thick,
        Underline::Dotted,
        Underline::Dash,
        Underline::DotDash,
        Underline::DotDotDash,
        Underline::Wave,
        Underline::DoubleWave,
    ];
    /// The OOXML `w:u/@w:val` name.
    pub fn ooxml(self) -> &'static str {
        match self {
            Underline::None => "none",
            Underline::Single => "single",
            Underline::Words => "words",
            Underline::Double => "double",
            Underline::Thick => "thick",
            Underline::Dotted => "dotted",
            Underline::Dash => "dash",
            Underline::DotDash => "dotDash",
            Underline::DotDotDash => "dotDotDash",
            Underline::Wave => "wave",
            Underline::DoubleWave => "wavyDouble",
        }
    }
    pub fn from_ooxml(s: &str) -> Underline {
        Underline::ALL.iter().copied().find(|u| u.ooxml() == s).unwrap_or(match s {
            "thickDash" | "dashLong" | "dashedHeavy" | "dashLongHeavy" => Underline::Dash,
            "dottedHeavy" => Underline::Dotted,
            "wavyHeavy" => Underline::Wave,
            "dashDotHeavy" => Underline::DotDash,
            "dashDotDotHeavy" => Underline::DotDotDash,
            _ => Underline::Single,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VertAlign {
    #[default]
    Baseline,
    Superscript,
    Subscript,
}

/// Word's 16 highlighter colours (plus none).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Highlight {
    #[default]
    None,
    Yellow,
    BrightGreen,
    Turquoise,
    Pink,
    Blue,
    Red,
    DarkBlue,
    Teal,
    Green,
    Violet,
    DarkRed,
    DarkYellow,
    Gray50,
    Gray25,
    Black,
    White,
}

impl Highlight {
    pub const ALL: [Highlight; 17] = [
        Highlight::None,
        Highlight::Yellow,
        Highlight::BrightGreen,
        Highlight::Turquoise,
        Highlight::Pink,
        Highlight::Blue,
        Highlight::Red,
        Highlight::DarkBlue,
        Highlight::Teal,
        Highlight::Green,
        Highlight::Violet,
        Highlight::DarkRed,
        Highlight::DarkYellow,
        Highlight::Gray50,
        Highlight::Gray25,
        Highlight::Black,
        Highlight::White,
    ];
    pub fn rgb(self) -> Option<Rgb> {
        Some(match self {
            Highlight::None => return None,
            Highlight::Yellow => Rgb(255, 255, 0),
            Highlight::BrightGreen => Rgb(0, 255, 0),
            Highlight::Turquoise => Rgb(0, 255, 255),
            Highlight::Pink => Rgb(255, 0, 255),
            Highlight::Blue => Rgb(0, 0, 255),
            Highlight::Red => Rgb(255, 0, 0),
            Highlight::DarkBlue => Rgb(0, 0, 128),
            Highlight::Teal => Rgb(0, 128, 128),
            Highlight::Green => Rgb(0, 128, 0),
            Highlight::Violet => Rgb(128, 0, 128),
            Highlight::DarkRed => Rgb(128, 0, 0),
            Highlight::DarkYellow => Rgb(128, 128, 0),
            Highlight::Gray50 => Rgb(128, 128, 128),
            Highlight::Gray25 => Rgb(192, 192, 192),
            Highlight::Black => Rgb(0, 0, 0),
            Highlight::White => Rgb(255, 255, 255),
        })
    }
    /// OOXML `w:highlight/@w:val`.
    pub fn ooxml(self) -> &'static str {
        match self {
            Highlight::None => "none",
            Highlight::Yellow => "yellow",
            Highlight::BrightGreen => "green",
            Highlight::Turquoise => "cyan",
            Highlight::Pink => "magenta",
            Highlight::Blue => "blue",
            Highlight::Red => "red",
            Highlight::DarkBlue => "darkBlue",
            Highlight::Teal => "darkCyan",
            Highlight::Green => "darkGreen",
            Highlight::Violet => "darkMagenta",
            Highlight::DarkRed => "darkRed",
            Highlight::DarkYellow => "darkYellow",
            Highlight::Gray50 => "darkGray",
            Highlight::Gray25 => "lightGray",
            Highlight::Black => "black",
            Highlight::White => "white",
        }
    }
    pub fn from_ooxml(s: &str) -> Highlight {
        Highlight::ALL.iter().copied().find(|h| h.ooxml() == s).unwrap_or(Highlight::None)
    }
    pub fn name(self) -> &'static str {
        match self {
            Highlight::None => "No Color",
            Highlight::Yellow => "Yellow",
            Highlight::BrightGreen => "Bright Green",
            Highlight::Turquoise => "Turquoise",
            Highlight::Pink => "Pink",
            Highlight::Blue => "Blue",
            Highlight::Red => "Red",
            Highlight::DarkBlue => "Dark Blue",
            Highlight::Teal => "Teal",
            Highlight::Green => "Green",
            Highlight::Violet => "Violet",
            Highlight::DarkRed => "Dark Red",
            Highlight::DarkYellow => "Dark Yellow",
            Highlight::Gray50 => "Gray-50%",
            Highlight::Gray25 => "Gray-25%",
            Highlight::Black => "Black",
            Highlight::White => "White",
        }
    }
}

/// Character formatting (direct or in a style). `None` = inherit.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CharProps {
    /// Character style id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<Underline>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline_color: Option<Rgb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub double_strike: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<TextColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Highlight>,
    /// Character shading (background fill).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<Rgb>,
    /// Character border (`w:bdr`); `style: None` = explicitly no border.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vert_align: Option<VertAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_caps: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Character spacing (expanded/condensed), points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f32>,
    /// Horizontal scale, percent (100 = normal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    /// Raised (+) / lowered (−) position, points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<f32>,
    /// Kerning for fonts at or above this size (points); 0 = off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kern: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emboss: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engrave: Option<bool>,
    /// BCP 47 language tag (proofing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_proof: Option<bool>,
    /// Right-to-left run (OOXML `w:rtl`): its characters are complex script and read right to left.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtl: Option<bool>,
    /// Format the whole run with the complex-script properties below (OOXML `w:cs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cs: Option<bool>,
    /// Complex-script font (Arabic, Persian, Hebrew…; OOXML `w:rFonts/@w:cs`). `None` = `font`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_cs: Option<String>,
    /// Complex-script size, points (`w:szCs`). `None` = the same as `size` (see [`CharProps::overlay`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_cs: Option<f32>,
    /// Complex-script bold / italic (`w:bCs`, `w:iCs`). `None` = the same as `bold` / `italic`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold_cs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic_cs: Option<bool>,
    /// BCP 47 language of complex-script text (`w:lang/@w:bidi`, e.g. `fa-IR`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang_bidi: Option<String>,
    /// Hyperlink target: a URL, or `#bookmark` for an internal link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Tracked insertion: index into `Document::revisions`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ins: Option<u32>,
    /// Tracked deletion: index into `Document::revisions`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub del: Option<u32>,
    /// Tracked formatting change (`w:rPrChange`): the formatting before it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<CharProps>>>,
}

/// A tracked formatting change (ECMA-376 §17.13.5: `w:rPrChange`, `w:pPrChange`,
/// `w:tblPrChange`, `w:trPrChange`, `w:tcPrChange`, `w:sectPrChange`): the properties as they
/// were before the change, and the revision (author, date) that made it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropChange<T> {
    /// Index into `Document::revisions`.
    pub rev: u32,
    /// The properties before the change (formatting only: no revision or link marks).
    pub old: T,
}

impl<T> PropChange<T> {
    pub fn boxed(rev: u32, old: T) -> Option<Box<PropChange<T>>> {
        Some(Box::new(PropChange { rev, old }))
    }
}

/// A tracked change of a paragraph's list numbering (`w:numberingChange`, kept for round trip):
/// `original` is the number as it was shown before the change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NumChange {
    /// Index into `Document::revisions`.
    pub rev: u32,
    pub original: String,
}

macro_rules! overlay_fields {
    ($dst:ident, $src:ident; $($f:ident),*) => {
        $( if $src.$f.is_some() { $dst.$f = $src.$f.clone(); } )*
    };
}

impl CharProps {
    /// Apply every `Some` field of `patch` on top of `self`.
    ///
    /// Size, bold and italic come in pairs with their complex-script values (Word's own buttons
    /// set both): a patch that sets the plain value and leaves the complex-script one `None` sets
    /// both, so a run's size or bold isn't overridden for its Persian/Arabic characters by an
    /// inherited `w:szCs` / `w:bCs`. The complex-script font inherits on its own (Word often
    /// sets only the Latin font and lets complex-script text keep the style's).
    pub fn overlay(&mut self, patch: &CharProps) {
        overlay_fields!(self, patch; style, font, size, bold, italic, underline, underline_color, strike, double_strike, color, highlight,
            shading, border, vert_align, caps, small_caps, hidden, spacing, scale, position, kern, outline, shadow, emboss, engrave, lang, no_proof, rtl,
            cs, font_cs, size_cs, bold_cs, italic_cs, lang_bidi, link, ins, del, fmt_change);
        if patch.size.is_some() && patch.size_cs.is_none() {
            self.size_cs = None;
        }
        if patch.bold.is_some() && patch.bold_cs.is_none() {
            self.bold_cs = None;
        }
        if patch.italic.is_some() && patch.italic_cs.is_none() {
            self.italic_cs = None;
        }
    }
    pub fn overlaid(mut self, patch: &CharProps) -> CharProps {
        self.overlay(patch);
        self
    }
    pub fn is_empty(&self) -> bool {
        *self == CharProps::default()
    }
    /// Clear direct formatting but keep what isn't "formatting" (links, revisions, character style
    /// is cleared too, as Word's Clear Formatting does).
    pub fn cleared(&self) -> CharProps {
        CharProps { link: self.link.clone(), ins: self.ins, del: self.del, fmt_change: self.fmt_change.clone(), ..Default::default() }
    }
    /// Just the formatting: without the link, revision marks and tracked formatting change.
    pub fn formatting(&self) -> CharProps {
        CharProps { link: None, ins: None, del: None, fmt_change: None, ..self.clone() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
    Distribute,
}

impl Align {
    /// Alignment is logical (`Left` is the start edge, ISO 29500 `start`): the alignment as seen
    /// on the page, or the logical one for a visual choice, in a paragraph that reads right to
    /// left when `rtl` (Left and Right swap; the swap is its own inverse).
    pub fn visual(self, rtl: bool) -> Align {
        match self {
            Align::Left if rtl => Align::Right,
            Align::Right if rtl => Align::Left,
            a => a,
        }
    }
}

/// Line spacing rule.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "rule", content = "value")]
pub enum LineSpacing {
    /// Multiple of single spacing (1.0 = single, 1.15, 1.5, 2.0 = double).
    Multiple(f32),
    /// At least this many points.
    AtLeast(f32),
    /// Exactly this many points.
    Exactly(f32),
}

impl Default for LineSpacing {
    fn default() -> Self {
        LineSpacing::Multiple(1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TabAlign {
    #[default]
    Left,
    Center,
    Right,
    Decimal,
    Bar,
    /// Clears an inherited stop at this position.
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TabLeader {
    #[default]
    None,
    Dot,
    Hyphen,
    Underscore,
    MiddleDot,
}

impl TabLeader {
    pub fn char(self) -> Option<char> {
        match self {
            TabLeader::None => None,
            TabLeader::Dot => Some('.'),
            TabLeader::Hyphen => Some('-'),
            TabLeader::Underscore => Some('_'),
            TabLeader::MiddleDot => Some('·'),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabStop {
    /// Points from the paragraph's left indent origin (the text column's left edge).
    pub pos: f32,
    #[serde(default)]
    pub align: TabAlign,
    #[serde(default)]
    pub leader: TabLeader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BorderStyle {
    #[default]
    None,
    Single,
    Double,
    Dotted,
    Dashed,
    Thick,
    Triple,
    DotDash,
    Wave,
}

impl BorderStyle {
    pub fn ooxml(self) -> &'static str {
        match self {
            BorderStyle::None => "nil",
            BorderStyle::Single => "single",
            BorderStyle::Double => "double",
            BorderStyle::Dotted => "dotted",
            BorderStyle::Dashed => "dashed",
            BorderStyle::Thick => "thick",
            BorderStyle::Triple => "triple",
            BorderStyle::DotDash => "dotDash",
            BorderStyle::Wave => "wave",
        }
    }
    pub fn from_ooxml(s: &str) -> BorderStyle {
        match s {
            "nil" | "none" => BorderStyle::None,
            "double" => BorderStyle::Double,
            "dotted" => BorderStyle::Dotted,
            "dashed" | "dashSmallGap" => BorderStyle::Dashed,
            "thick" => BorderStyle::Thick,
            "triple" => BorderStyle::Triple,
            "dotDash" | "dotDotDash" => BorderStyle::DotDash,
            "wave" | "doubleWave" => BorderStyle::Wave,
            _ => BorderStyle::Single,
        }
    }
}

/// One border line.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Border {
    pub style: BorderStyle,
    /// Line width, points.
    pub width: f32,
    pub color: Option<Rgb>,
    /// Distance from the text, points.
    pub space: f32,
}

impl Border {
    pub fn single(width: f32) -> Border {
        Border { style: BorderStyle::Single, width, color: None, space: 0.0 }
    }
    pub fn is_visible(&self) -> bool {
        self.style != BorderStyle::None && self.width > 0.0
    }
}

/// Borders on the four sides (plus inner borders for tables and runs of bordered paragraphs).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Borders {
    pub top: Option<Border>,
    pub left: Option<Border>,
    pub bottom: Option<Border>,
    pub right: Option<Border>,
    /// Between paragraphs / inside horizontal (tables).
    pub between: Option<Border>,
    /// Inside vertical (tables).
    pub inside_v: Option<Border>,
}

impl Borders {
    pub fn all(b: Border) -> Borders {
        Borders { top: Some(b), left: Some(b), bottom: Some(b), right: Some(b), between: Some(b), inside_v: Some(b) }
    }
    pub fn box_(b: Border) -> Borders {
        Borders { top: Some(b), left: Some(b), bottom: Some(b), right: Some(b), between: None, inside_v: None }
    }
    pub fn any_visible(&self) -> bool {
        [self.top, self.left, self.bottom, self.right, self.between, self.inside_v].iter().flatten().any(Border::is_visible)
    }
    /// Apply every `Some` side of `patch` on top of `self`.
    pub fn overlay(&mut self, patch: &Borders) {
        if patch.top.is_some() {
            self.top = patch.top;
        }
        if patch.left.is_some() {
            self.left = patch.left;
        }
        if patch.bottom.is_some() {
            self.bottom = patch.bottom;
        }
        if patch.right.is_some() {
            self.right = patch.right;
        }
        if patch.between.is_some() {
            self.between = patch.between;
        }
        if patch.inside_v.is_some() {
            self.inside_v = patch.inside_v;
        }
    }
}

/// A reference to a numbering definition and level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NumRef {
    /// `Numbering::nums` id; 0 removes numbering inherited from a style.
    pub num: u32,
    pub level: u8,
}

/// Paragraph formatting. `None` = inherit.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ParaProps {
    /// Paragraph style id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// Left indent, points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_right: Option<f32>,
    /// First-line indent, points; negative = hanging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_first: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_before: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_after: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_spacing: Option<LineSpacing>,
    /// Ignore space before/after between paragraphs of the same style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contextual_spacing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_next: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_lines: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_break_before: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widow_control: Option<bool>,
    /// 0..=8 = Level 1..9; 9 = body text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numbering: Option<NumRef>,
    /// Tab stops (replaces the inherited list; `TabAlign::Clear` stops remove inherited ones).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tabs: Option<Vec<TabStop>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<Rgb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub borders: Option<Borders>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_hyphens: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_line_numbers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bidi: Option<bool>,
    /// Drop cap: lines to drop (0 = none) — applies to the first character(s).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_cap: Option<u8>,
    /// Asian typography (`w:kinsoku`): East Asian line-breaking rules — no line starts with
    /// closing punctuation such as 、。」 or ends with opening punctuation such as 「（.
    /// Unset = on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinsoku: Option<bool>,
    /// `w:wordWrap`: Latin words wrap whole; `false` lets them break at any character (Word's
    /// "Allow Latin text to wrap in the middle of a word"). Unset = on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<bool>,
    /// `w:overflowPunct`: punctuation may hang past the line end. Unset = on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overflow_punct: Option<bool>,
    /// `w:topLinePunct`: compress punctuation at the start of a line. Unset = off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_line_punct: Option<bool>,
    /// `w:autoSpaceDE`: automatic space between Asian and Latin text. Unset = on.
    #[serde(rename = "autoSpaceDE", skip_serializing_if = "Option::is_none")]
    pub auto_space_de: Option<bool>,
    /// `w:autoSpaceDN`: automatic space between Asian text and numbers. Unset = on.
    #[serde(rename = "autoSpaceDN", skip_serializing_if = "Option::is_none")]
    pub auto_space_dn: Option<bool>,
    /// Tracked change of the list numbering (`w:numberingChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_change: Option<Box<NumChange>>,
    /// Tracked formatting change (`w:pPrChange`): the paragraph properties before it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<ParaProps>>>,
}

impl ParaProps {
    pub fn overlay(&mut self, patch: &ParaProps) {
        overlay_fields!(self, patch; style, align, indent_left, indent_right, indent_first, space_before, space_after, line_spacing,
            contextual_spacing, keep_next, keep_lines, page_break_before, widow_control, outline_level, numbering, tabs, shading, borders,
            suppress_hyphens, suppress_line_numbers, bidi, drop_cap, kinsoku, word_wrap, overflow_punct, top_line_punct, auto_space_de, auto_space_dn);
    }
    pub fn overlaid(mut self, patch: &ParaProps) -> ParaProps {
        self.overlay(patch);
        self
    }
    pub fn is_empty(&self) -> bool {
        *self == ParaProps::default()
    }
    /// Just the formatting: without the tracked changes.
    pub fn formatting(&self) -> ParaProps {
        ParaProps { num_change: None, fmt_change: None, ..self.clone() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

/// Which way a table cell's text runs (`w:textDirection`, ECMA-376 §17.4.72).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextDirection {
    /// Left to right, lines stacking downwards.
    #[default]
    Horizontal,
    /// Turned 90° clockwise: lines read top to bottom and stack from right to left (`tbRl`).
    Down,
    /// Turned 90° counter-clockwise: lines read bottom to top and stack from left to right (`btLr`).
    Up,
}

impl TextDirection {
    /// The direction an OOXML `ST_TextDirection` value names (transitional and strict names).
    /// The East Asian vertical layouts turn like `Down`; unknown values are horizontal.
    pub fn from_ooxml(v: &str) -> TextDirection {
        match v {
            "tbRl" | "tbRlV" | "tbLrV" | "rl" | "rlV" | "lrV" => TextDirection::Down,
            "btLr" | "lr" => TextDirection::Up,
            _ => TextDirection::Horizontal,
        }
    }
    /// The transitional OOXML name.
    pub fn ooxml(self) -> &'static str {
        match self {
            TextDirection::Horizontal => "lrTb",
            TextDirection::Down => "tbRl",
            TextDirection::Up => "btLr",
        }
    }
    /// Word's Text Direction button: horizontal → down → up → horizontal.
    pub fn next(self) -> TextDirection {
        match self {
            TextDirection::Horizontal => TextDirection::Down,
            TextDirection::Down => TextDirection::Up,
            TextDirection::Up => TextDirection::Horizontal,
        }
    }
    pub fn is_turned(self) -> bool {
        self != TextDirection::Horizontal
    }
}

/// Table-wide properties.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TableProps {
    pub style: Option<String>,
    /// Preferred width: points (`Some`) or automatic (`None`).
    pub width: Option<f32>,
    /// Preferred width as a percentage of the text column (overrides `width`).
    pub width_pct: Option<f32>,
    pub align: Option<Align>,
    /// Indent from the left margin, points.
    pub indent: Option<f32>,
    pub borders: Option<Borders>,
    /// Default cell margins (top, left, bottom, right), points.
    pub cell_margins: Option<[f32; 4]>,
    /// Fixed column widths (no auto-fit).
    pub fixed: bool,
    /// Table style options ("look"): header row, total row, banded rows, first column, last
    /// column, banded columns.
    pub look: TableLook,
    pub shading: Option<Rgb>,
    /// Alternative text: the title (`w:tblCaption`).
    pub caption: Option<String>,
    /// Alternative text: the description (`w:tblDescription`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Floating placement (`w:tblpPr`); `None` for a table in the text flow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub float: Option<TableFloat>,
    /// Tracked formatting change (`w:tblPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<TableProps>>>,
}

/// Where a floating table sits; the text after it wraps around it.
#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TableFloat {
    /// What `x` is measured from: the column (`Column`), the margin or the page.
    pub h_rel: crate::para::Anchor,
    /// What `y` is measured from: the text where the table stands (`Paragraph`), the margin or
    /// the page.
    pub v_rel: crate::para::Anchor,
    /// Offsets, points (used when the matching alignment is `None`).
    pub x: f32,
    pub y: f32,
    pub h_align: Option<crate::para::FloatAlign>,
    pub v_align: Option<crate::para::FloatAlign>,
    /// Distance from surrounding text: left, top, right, bottom (points).
    pub dist: [f32; 4],
    /// Whether it may overlap other floating tables (`w:tblOverlap`); Word's default is yes.
    pub overlap: bool,
}

impl TableFloat {
    /// The distances from text, finite and clamped (left, top, right, bottom).
    pub fn dist_from_text(&self) -> [f32; 4] {
        self.dist.map(|v| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TableLook {
    pub header_row: bool,
    pub total_row: bool,
    pub banded_rows: bool,
    pub first_column: bool,
    pub last_column: bool,
    pub banded_columns: bool,
}

impl Default for TableLook {
    fn default() -> Self {
        TableLook { header_row: true, total_row: false, banded_rows: true, first_column: true, last_column: false, banded_columns: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HeightRule {
    #[default]
    Auto,
    AtLeast,
    Exact,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RowProps {
    /// Points (meaning depends on `height_rule`).
    pub height: Option<f32>,
    pub height_rule: HeightRule,
    /// Repeat as a header row at the top of each page.
    pub header: bool,
    /// Don't let the row break across pages.
    pub cant_split: bool,
    /// Tracked formatting change (`w:trPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<RowProps>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VMerge {
    #[default]
    None,
    /// First cell of a vertically merged range.
    Restart,
    /// Continues the merge from the cell above.
    Continue,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CellProps {
    /// Preferred width, points.
    pub width: Option<f32>,
    /// Preferred width as a percentage of the table's (`w:tcW w:type="pct"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_pct: Option<f32>,
    /// Number of grid columns spanned (1 = no horizontal merge).
    pub span: u32,
    pub vmerge: VMerge,
    pub shading: Option<Rgb>,
    pub borders: Option<Borders>,
    pub valign: VAlign,
    /// Cell margins override (top, left, bottom, right), points.
    pub margins: Option<[f32; 4]>,
    /// Which way the cell's text runs.
    pub text_direction: TextDirection,
    pub no_wrap: bool,
    /// Tracked formatting change (`w:tcPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<CellProps>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hex() {
        assert_eq!(Rgb(255, 0, 16).hex(), "FF0010");
        assert_eq!(Rgb::parse("#FF0010"), Some(Rgb(255, 0, 16)));
        assert_eq!(Rgb::parse("zzzzzz"), None);
        assert_eq!(Rgb::parse("é1234"), None);
    }

    #[test]
    fn overlay_keeps_unset() {
        let base = CharProps { bold: Some(true), size: Some(12.0), ..Default::default() };
        let patch = CharProps { size: Some(14.0), italic: Some(true), ..Default::default() };
        let r = base.overlaid(&patch);
        assert_eq!(r.bold, Some(true));
        assert_eq!(r.size, Some(14.0));
        assert_eq!(r.italic, Some(true));
    }

    #[test]
    fn borders_overlay_per_side() {
        let thin = Border::single(0.5);
        let thick = Border::single(2.0);
        let nil = Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 };
        let mut b = Borders::all(thin);
        b.overlay(&Borders { top: Some(thick), ..Default::default() });
        assert_eq!(b.top, Some(thick));
        assert_eq!(b.bottom, Some(thin));
        b.overlay(&Borders::box_(nil));
        assert_eq!(b.left, Some(nil)); // explicit nil overrides that side
        assert_eq!(b.between, Some(thin)); // untouched sides survive
        b.overlay(&Borders::default());
        assert_eq!(b.left, Some(nil)); // empty patch changes nothing
    }

    #[test]
    fn ooxml_names_round_trip() {
        for u in Underline::ALL {
            assert_eq!(Underline::from_ooxml(u.ooxml()), u);
        }
        for h in Highlight::ALL {
            assert_eq!(Highlight::from_ooxml(h.ooxml()), h);
        }
    }

    #[test]
    fn text_direction_names() {
        let mut d = TextDirection::Horizontal;
        for _ in 0..3 {
            assert_eq!(TextDirection::from_ooxml(d.ooxml()), d);
            d = d.next();
        }
        assert_eq!(d, TextDirection::Horizontal, "the button cycles through three directions");
        // Strict names, East Asian vertical layouts and junk.
        assert_eq!(TextDirection::from_ooxml("rl"), TextDirection::Down);
        assert_eq!(TextDirection::from_ooxml("lr"), TextDirection::Up);
        assert_eq!(TextDirection::from_ooxml("tbRlV"), TextDirection::Down);
        assert_eq!(TextDirection::from_ooxml("tb"), TextDirection::Horizontal);
        assert_eq!(TextDirection::from_ooxml("lrTbV"), TextDirection::Horizontal);
        assert_eq!(TextDirection::from_ooxml("é?"), TextDirection::Horizontal);
    }

    #[test]
    fn props_serde_skip_none() {
        let p = CharProps { bold: Some(true), ..Default::default() };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, r#"{"bold":true}"#);
        let back: CharProps = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }
}
