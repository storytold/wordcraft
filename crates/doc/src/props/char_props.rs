use super::color::{Rgb, TextColor};
use super::para_props::Border;
use super::{Deserialize, Serialize};

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

/// The OpenType ligatures a run asks for ([MS-DOCX] `w14:ligatures`): standard (`liga`),
/// contextual (`clig`), historical (`hlig`) and discretional (`dlig`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Ligatures {
    pub standard: bool,
    pub contextual: bool,
    pub historical: bool,
    pub discretional: bool,
}

impl Ligatures {
    /// Every combination of the four flags.
    pub fn all() -> impl Iterator<Item = Ligatures> {
        (0u8..16).map(|b| Ligatures { standard: b & 1 != 0, contextual: b & 2 != 0, historical: b & 4 != 0, discretional: b & 8 != 0 })
    }
    /// The `w14:ligatures/@w14:val` name: `none`, `all`, or the set flags joined in camelCase
    /// (`standardContextual`).
    pub fn ooxml(self) -> String {
        let on: Vec<&str> =
            [(self.standard, "standard"), (self.contextual, "contextual"), (self.historical, "historical"), (self.discretional, "discretional")]
                .into_iter()
                .filter_map(|(on, n)| on.then_some(n))
                .collect();
        match on.len() {
            0 => "none".into(),
            4 => "all".into(),
            _ => on
                .iter()
                .enumerate()
                .flat_map(|(i, n)| n.chars().enumerate().map(move |(j, c)| if i > 0 && j == 0 { c.to_ascii_uppercase() } else { c }))
                .collect(),
        }
    }
    /// `None` for a name [MS-DOCX] doesn't define.
    pub fn from_ooxml(s: &str) -> Option<Ligatures> {
        Ligatures::all().find(|l| l.ooxml() == s)
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

/// The text size, in points, of a run whose size no style or default sets.
pub const DEFAULT_FONT_SIZE: f32 = 11.0;

/// Character formatting (direct or in a style). `None` = inherit.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CharProps {
    /// Character style id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// The symbol's own font (OOXML `w:sym/@w:font`, ECMA-376 §17.3.3.30), set on a symbol
    /// character from a `w:sym` or Insert › Symbol: it is shown in this font whatever `font` is,
    /// so changing the text's font keeps the symbol. `None` = the run's font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sym_font: Option<String>,
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
    /// Hidden in Web Layout (`w:webHidden`); distinct from `w:vanish` (`hidden`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_hidden: Option<bool>,
    /// Hidden paragraph mark (`w:specVanish`); distinct from hidden run text (`w:vanish`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_vanish: Option<bool>,
    /// Snap characters to the document grid (`w:snapToGrid`). Unset inherits; Word's default is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_to_grid: Option<bool>,
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
    /// OpenType ligatures (`w14:ligatures`). `None` leaves WordCraft's default: standard and
    /// contextual ligatures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ligatures: Option<Ligatures>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emboss: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engrave: Option<bool>,
    /// WordArt text effects: fill, outline, shadow, glow, reflection ([MS-DOCX] `w14:` run
    /// properties). Overlaid effect by effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_effects: Option<Box<crate::wordart::TextEffects>>,
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
    /// East Asian font (Chinese, Japanese and Korean text; OOXML `w:rFonts/@w:eastAsia`). `None` =
    /// inherit, then `font`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_ea: Option<String>,
    /// Complex-script size, points (`w:szCs`). `None` leaves the inherited value unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_cs: Option<f32>,
    /// Complex-script bold / italic (`w:bCs`, `w:iCs`). `None` leaves the inherited value unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold_cs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic_cs: Option<bool>,
    /// BCP 47 language of complex-script text (`w:lang/@w:bidi`, e.g. `fa-IR`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang_bidi: Option<String>,
    /// BCP 47 language of East Asian text (`w:lang/@w:eastAsia`, e.g. `zh-CN`, `ja-JP`, `ko-KR`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang_ea: Option<String>,
    /// East Asian typography: two lines in one and horizontal in vertical (`w:eastAsianLayout`).
    /// Kept for round trip; layout doesn't apply it yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub east_asian_layout: Option<EastAsianLayout>,
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

/// East Asian typography settings of a run (ISO/IEC 29500-1 §17.3.2.10, `w:eastAsianLayout`):
/// *two lines in one* sets the run's text on two sub-lines inside one line (warichu), *horizontal
/// in vertical* turns it 90° to read across a vertical line (tate-chu-yoko). Each attribute is
/// `None` when the file doesn't say.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EastAsianLayout {
    /// East Asian typography run id (`w:id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    /// Two lines in one (`w:combine`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combine: Option<bool>,
    /// Brackets around two lines in one (`w:combineBrackets`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combine_brackets: Option<CombineBrackets>,
    /// Horizontal in vertical (`w:vert`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vert: Option<bool>,
    /// Compress horizontal-in-vertical text to the line height (`w:vertCompress`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vert_compress: Option<bool>,
}

/// Brackets around two lines in one (`ST_CombineBrackets`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CombineBrackets {
    None,
    Round,
    Square,
    Angle,
    Curly,
}

impl CombineBrackets {
    pub const ALL: [CombineBrackets; 5] =
        [CombineBrackets::None, CombineBrackets::Round, CombineBrackets::Square, CombineBrackets::Angle, CombineBrackets::Curly];
    /// OOXML `w:combineBrackets` value.
    pub fn ooxml(self) -> &'static str {
        match self {
            CombineBrackets::None => "none",
            CombineBrackets::Round => "round",
            CombineBrackets::Square => "square",
            CombineBrackets::Angle => "angle",
            CombineBrackets::Curly => "curly",
        }
    }
    /// The value for `s`, `None` for one the schema doesn't have.
    pub fn from_ooxml(s: &str) -> Option<CombineBrackets> {
        CombineBrackets::ALL.into_iter().find(|b| b.ooxml() == s)
    }
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

impl CharProps {
    /// The same formatting outside a hyperlink: no link target, and no "Hyperlink" character
    /// style that came with it. What a paragraph mark or text typed next to a link gets.
    pub fn unlinked(&self) -> CharProps {
        let mut c = self.clone();
        if c.link.take().is_some() && c.style.as_deref() == Some("Hyperlink") {
            c.style = None;
        }
        c
    }

    /// The fields of `self` that differ from `base` (equal ones unset): what `self` adds when
    /// overlaid on `base`.
    pub fn minus(mut self, base: &CharProps) -> CharProps {
        self.minus_text_style(base);
        self.minus_run_visibility(base);
        self.minus_script_style(base);
        self.minus_document_marks(base);
        self
    }

    fn minus_text_style(&mut self, base: &CharProps) {
        drop_same_fields!(self, base; style, font, sym_font, size, bold, italic, underline, underline_color, strike, double_strike, color, highlight,
            shading, border, vert_align, caps, small_caps, hidden, spacing, scale, position, kern, outline, shadow, emboss, engrave, lang, no_proof, rtl);
    }

    fn minus_run_visibility(&mut self, base: &CharProps) {
        drop_same_fields!(self, base; web_hidden, spec_vanish, snap_to_grid);
    }

    fn minus_script_style(&mut self, base: &CharProps) {
        drop_same_fields!(self, base; cs, font_cs, font_ea, size_cs, bold_cs, italic_cs, lang_bidi, lang_ea, east_asian_layout);
    }

    fn minus_document_marks(&mut self, base: &CharProps) {
        drop_same_fields!(self, base; link, ins, del, ligatures);
    }

    /// Apply every `Some` field of `patch` on top of `self`.
    ///
    /// Plain and complex-script properties inherit independently. Commands that intend to
    /// format both scripts must set both values explicitly.
    pub fn overlay(&mut self, patch: &CharProps) {
        self.overlay_text_style(patch);
        self.overlay_run_visibility(patch);
        self.overlay_script_style(patch);
        self.overlay_document_marks(patch);
        if let Some(p) = &patch.text_effects {
            self.text_effects.get_or_insert_with(Default::default).overlay(p);
        }
    }

    fn overlay_text_style(&mut self, patch: &CharProps) {
        overlay_fields!(self, patch; style, font, sym_font, size, bold, italic, underline, underline_color, strike, double_strike, color, highlight,
            shading, border, vert_align, caps, small_caps, hidden, spacing, scale, position, kern, outline, shadow, emboss, engrave, lang, no_proof, rtl);
    }

    fn overlay_run_visibility(&mut self, patch: &CharProps) {
        overlay_fields!(self, patch; web_hidden, spec_vanish, snap_to_grid);
    }

    fn overlay_script_style(&mut self, patch: &CharProps) {
        overlay_fields!(self, patch; cs, font_cs, font_ea, size_cs, bold_cs, italic_cs, lang_bidi, lang_ea, east_asian_layout);
    }

    fn overlay_document_marks(&mut self, patch: &CharProps) {
        overlay_fields!(self, patch; link, ins, del, fmt_change, ligatures);
    }
    pub fn overlaid(mut self, patch: &CharProps) -> CharProps {
        self.overlay(patch);
        self
    }
    pub fn is_empty(&self) -> bool {
        *self == CharProps::default()
    }
    /// The font a character of this run is shown in: the symbol's own font when it has one
    /// ([`Self::sym_font`]), else the run's font.
    pub fn shown_font(&self) -> Option<&str> {
        self.sym_font.as_deref().filter(|f| !f.trim().is_empty()).or(self.font.as_deref())
    }
    /// The same formatting for text typed or painted next to a symbol: without the symbol's own
    /// font.
    pub fn without_symbol(mut self) -> CharProps {
        self.sym_font = None;
        self
    }
    /// Clear direct formatting but keep what isn't "formatting" (links, revisions, character style
    /// is cleared too, as Word's Clear Formatting does).
    pub fn cleared(&self) -> CharProps {
        CharProps {
            link: self.link.clone(),
            ins: self.ins,
            del: self.del,
            fmt_change: self.fmt_change.clone(),
            // A symbol stays the symbol it is.
            sym_font: self.sym_font.clone(),
            ..Default::default()
        }
    }
    /// Just the formatting: without the link, revision marks and tracked formatting change.
    pub fn formatting(&self) -> CharProps {
        CharProps { link: None, ins: None, del: None, fmt_change: None, ..self.clone() }
    }
}
