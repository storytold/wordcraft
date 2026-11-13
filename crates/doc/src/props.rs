//! Character, paragraph, table and cell properties.
//!
//! Every field is an `Option`: `None` means "inherit" (from the style chain, then document
//! defaults). [`CharProps::overlay`] and friends apply a patch on top of a base.

use serde::{Deserialize, Serialize};

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl<'de> Deserialize<'de> for Rgb {
    /// Reads what it writes (`[r, g, b]`) and also a string [`Rgb::parse`] reads, so the colours in
    /// `CharProps` / `ParaProps` take the `RRGGBB` that `para.borders` and `table.shading` take (#865).
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, SeqAccess, Unexpected, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Rgb;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("`[r, g, b]`, `RRGGBB` or `#RRGGBB`")
            }
            fn visit_str<E: Error>(self, s: &str) -> Result<Rgb, E> {
                Rgb::parse(s).ok_or_else(|| E::invalid_value(Unexpected::Str(s), &self))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Rgb, A::Error> {
                let mut c = [0u8; 3];
                for (i, x) in c.iter_mut().enumerate() {
                    *x = a.next_element()?.ok_or_else(|| A::Error::invalid_length(i, &self))?;
                }
                if a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(A::Error::invalid_length(4, &self));
                }
                Ok(Rgb(c[0], c[1], c[2]))
            }
        }
        d.deserialize_any(V)
    }
}

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
    /// This colour with the DrawingML colour modifier `name` (`lumMod`, `lumOff`, `tint`,
    /// `shade`, `satMod`, ECMA-376 §20.1.2.3) at `v` (1.0 = 100%); other names leave it
    /// unchanged.
    pub fn modified(self, name: &str, v: f64) -> Rgb {
        match name {
            "lumMod" => adjust_lum(self, |l| l * v),
            "lumOff" => adjust_lum(self, |l| l + v),
            "tint" => mix(self, 255.0, v),
            "shade" => mix(self, 0.0, v),
            "satMod" => {
                let (h, sat, l) = to_hsl(self);
                from_hsl(h, (sat * v).clamp(0.0, 1.0), l)
            }
            _ => self,
        }
    }
    /// The colour with hue `h` (0..1 of a turn), saturation `s` and lightness `l` (0..1).
    pub fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
        from_hsl(h, s, l)
    }
    /// This colour with its HSL luminance changed by `f`.
    pub fn with_lum(self, f: impl Fn(f64) -> f64) -> Rgb {
        adjust_lum(self, f)
    }
    pub fn luma(self) -> f32 {
        0.299 * self.0 as f32 + 0.587 * self.1 as f32 + 0.114 * self.2 as f32
    }
}

/// A text colour: automatic (black on light backgrounds, white on dark) or a fixed colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Default)]
pub enum TextColor {
    #[default]
    Auto,
    Rgb(Rgb),
}

impl TextColor {
    /// `auto` or `automatic` (any case), or a colour [`Rgb::parse`] reads.
    pub fn parse(s: &str) -> Option<TextColor> {
        if s.eq_ignore_ascii_case("auto") || s.eq_ignore_ascii_case("automatic") { Some(TextColor::Auto) } else { Rgb::parse(s).map(TextColor::Rgb) }
    }
}

impl<'de> Deserialize<'de> for TextColor {
    /// Reads what it writes (`"Auto"`, `{"Rgb": [r, g, b]}`) and also a string [`TextColor::parse`]
    /// reads, so `styles.modify` and `format.set` take the colours `format.color` takes.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, Unexpected};
        #[derive(Deserialize)]
        enum Written {
            Auto,
            Rgb(Rgb),
        }
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) => {
                TextColor::parse(&s).ok_or_else(|| D::Error::invalid_value(Unexpected::Str(&s), &"`auto`, `RRGGBB` or `#RRGGBB`"))
            }
            v => Ok(match serde_json::from_value(v).map_err(D::Error::custom)? {
                Written::Auto => TextColor::Auto,
                Written::Rgb(c) => TextColor::Rgb(c),
            }),
        }
    }
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

macro_rules! overlay_fields {
    ($dst:ident, $src:ident; $($f:ident),*) => {
        $( if $src.$f.is_some() { $dst.$f = $src.$f.clone(); } )*
    };
}

macro_rules! drop_same_fields {
    ($dst:ident, $base:ident; $($f:ident),*) => {
        $( if $dst.$f == $base.$f { $dst.$f = None; } )*
    };
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
        drop_same_fields!(self, base; style, font, sym_font, size, bold, italic, underline, underline_color, strike, double_strike, color, highlight,
            shading, border, vert_align, caps, small_caps, hidden, spacing, scale, position, kern, outline, shadow, emboss, engrave, lang, no_proof, rtl,
            cs, font_cs, size_cs, bold_cs, italic_cs, lang_bidi, east_asian_layout, link, ins, del, ligatures);
        self
    }

    /// Apply every `Some` field of `patch` on top of `self`.
    ///
    /// Plain and complex-script properties inherit independently. Commands that intend to
    /// format both scripts must set both values explicitly.
    pub fn overlay(&mut self, patch: &CharProps) {
        overlay_fields!(self, patch; style, font, sym_font, size, bold, italic, underline, underline_color, strike, double_strike, color, highlight,
            shading, border, vert_align, caps, small_caps, hidden, spacing, scale, position, kern, outline, shadow, emboss, engrave, lang, no_proof, rtl,
            cs, font_cs, font_ea, size_cs, bold_cs, italic_cs, lang_bidi, east_asian_layout, link, ins, del, fmt_change, ligatures);
        if let Some(p) = &patch.text_effects {
            self.text_effects.get_or_insert_with(Default::default).overlay(p);
        }
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

/// Two tab positions closer than this (points) are the same stop: `para.tabs` replaces at this
/// tolerance, and the Tabs dialog and the ruler merge and detect no-ops with it.
pub const TAB_POS_EPS: f32 = 0.5;

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

/// A text frame's placement (`w:framePr`, ECMA-376 §17.3.1.11), in points. Offsets apply when
/// the matching alignment is unset; anchors and the wrap keep OOXML's names.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextFrame {
    /// Width; `None` fits the text.
    pub w: Option<f32>,
    /// Height, with `h_exact` exact rather than at least; `None` fits the text.
    pub h: Option<f32>,
    pub h_exact: bool,
    pub x: f32,
    pub y: f32,
    /// `left`, `center`, `right`, `inside`, `outside`.
    pub x_align: Option<String>,
    /// `top`, `center`, `bottom`, `inside`, `outside` (`inline` is no alignment).
    pub y_align: Option<String>,
    /// `text` (the column), `margin` or `page`.
    pub h_anchor: String,
    /// `text` (the paragraph), `margin` or `page`.
    pub v_anchor: String,
    /// `around`, `auto`, `notBeside`, `none`, `tight` or `through`.
    pub wrap: String,
    /// Distance from the text left and right, and above and below.
    pub h_space: f32,
    pub v_space: f32,
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
    /// Left (start) indent in characters (`w:leftChars` / `w:startChars`, stored by Word in
    /// hundredths). A non-zero value wins over `indent_left`; one character is the size of the
    /// paragraph mark's font. `Some(0.0)` cancels an inherited value. Common in East Asian documents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_left_chars: Option<f32>,
    /// Right (end) indent in characters (`w:rightChars` / `w:endChars`), like `indent_left_chars`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_right_chars: Option<f32>,
    /// First-line indent in characters (`w:firstLineChars`; negative = `w:hangingChars`), like
    /// `indent_left_chars`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_first_chars: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_before: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_after: Option<f32>,
    /// HTML-style automatic space before (`w:beforeAutospacing`): 14 pt, none at the top of the
    /// document or of a table cell, and collapsing with the space after the paragraph above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_before_auto: Option<bool>,
    /// HTML-style automatic space after (`w:afterAutospacing`): 14 pt, none at the end of a cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_after_auto: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_spacing: Option<LineSpacing>,
    /// Ignore space before/after between paragraphs of the same style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contextual_spacing: Option<bool>,
    /// `w:mirrorIndents` (ECMA-376 §17.3.1.18): the left and right indents are the inside and
    /// outside ones, so they swap sides on even (left-hand) pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror_indents: Option<bool>,
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
    /// Drop cap "In margin" (`w:dropCap="margin"`): the letter hangs in the left margin and the
    /// text keeps its indent. Unset = dropped into the text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_cap_margin: Option<bool>,
    /// The drop cap's distance from the text, in points (`w:hSpace`). Unset = 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_cap_distance: Option<f32>,
    /// The drop cap letter's font family (the cap run's `w:rFonts`). Unset = the text's font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop_cap_font: Option<String>,
    /// A text frame (`w:framePr` other than a drop cap): the paragraph is placed apart from the
    /// text, which may wrap around it — a footer's page number in a frame, for instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<TextFrame>,
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
    /// `w:snapToGrid`: lines snap to the section's document grid. Unset = on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap_to_grid: Option<bool>,
}

impl ParaProps {
    pub fn overlay(&mut self, patch: &ParaProps) {
        overlay_fields!(self, patch; style, align, indent_left, indent_right, indent_first, indent_left_chars, indent_right_chars, indent_first_chars, space_before, space_after, space_before_auto, space_after_auto, line_spacing,
            contextual_spacing, mirror_indents, keep_next, keep_lines, page_break_before, widow_control, outline_level, numbering, tabs, shading, borders,
            suppress_hyphens, suppress_line_numbers, bidi, drop_cap, drop_cap_margin, drop_cap_distance, drop_cap_font, frame, kinsoku, word_wrap, overflow_punct, top_line_punct, auto_space_de, auto_space_dn,
            snap_to_grid);
    }
    pub fn overlaid(mut self, patch: &ParaProps) -> ParaProps {
        self.overlay(patch);
        self
    }
    pub fn is_empty(&self) -> bool {
        *self == ParaProps::default()
    }
    /// After setting indents in points: keep the indents in characters (left, right, first line)
    /// an inherited style may set from winning over them, as Word does (`w:firstLineChars="0"`).
    pub fn cancel_char_indents(&mut self, left: bool, right: bool, first: bool) {
        for (on, chars) in [(left, &mut self.indent_left_chars), (right, &mut self.indent_right_chars), (first, &mut self.indent_first_chars)] {
            if on {
                *chars = Some(0.0);
            }
        }
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

/// Which way a text box's text runs (Shape Format › Text › Text Direction; DrawingML
/// `a:bodyPr/@vert`, ECMA-376 §21.1.10.83 `ST_TextVerticalType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextVert {
    /// Upright, lines stacking downwards (`horz`).
    #[default]
    Horz,
    /// Turned 90° clockwise (`vert`).
    Vert,
    /// Turned 270° (`vert270`).
    Vert270,
    /// Upright letters stacked one above the other (`wordArtVert`).
    Stacked,
}

impl TextVert {
    /// The direction an `ST_TextVerticalType` value names; the East Asian and Mongolian vertical
    /// layouts turn like `vert`, unknown values are horizontal.
    pub fn from_ooxml(v: &str) -> TextVert {
        match v {
            "vert" | "eaVert" | "mongolianVert" => TextVert::Vert,
            "vert270" => TextVert::Vert270,
            "wordArtVert" | "wordArtVertRtl" => TextVert::Stacked,
            _ => TextVert::Horz,
        }
    }
    pub fn ooxml(self) -> &'static str {
        match self {
            TextVert::Horz => "horz",
            TextVert::Vert => "vert",
            TextVert::Vert270 => "vert270",
            TextVert::Stacked => "wordArtVert",
        }
    }
    /// How its lines turn on the page (stacked text stays upright).
    pub fn turn(self) -> TextDirection {
        match self {
            TextVert::Vert => TextDirection::Down,
            TextVert::Vert270 => TextDirection::Up,
            TextVert::Horz | TextVert::Stacked => TextDirection::Horizontal,
        }
    }
}

/// Where a text box's text sits between its top and bottom (Align Text; `a:bodyPr/@anchor`,
/// `ST_TextAnchoringType`): reuses [`VAlign`]. Justified and distributed anchoring read as top
/// and middle.
pub fn valign_from_anchor(v: &str) -> VAlign {
    match v {
        "ctr" | "dist" => VAlign::Center,
        "b" => VAlign::Bottom,
        _ => VAlign::Top,
    }
}

/// The `ST_TextAnchoringType` value for `a`.
pub fn anchor_ooxml(a: VAlign) -> &'static str {
    match a {
        VAlign::Top => "t",
        VAlign::Center => "ctr",
        VAlign::Bottom => "b",
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
    /// Right-to-left table (`w:bidiVisual`): the first logical column stands at the right edge,
    /// cell margins and left/right borders mirror, and the alignment and indent are measured from
    /// the right margin.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bidi_visual: bool,
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
    /// Table properties this row overrides (`w:tblPrEx`, ECMA-376 §17.4.61): width, alignment,
    /// indent, borders, shading, layout and cell margins. `Some` when the file has the element.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exceptions: Option<Box<TableProps>>,
    /// Grid columns skipped before the row's first cell (`w:gridBefore`), and the preferred
    /// width of that gap in points (`w:wBefore`).
    #[serde(skip_serializing_if = "is_zero")]
    pub grid_before: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w_before: Option<f32>,
    /// Grid columns left after the row's last cell (`w:gridAfter`), and their width in points
    /// (`w:wAfter`).
    #[serde(skip_serializing_if = "is_zero")]
    pub grid_after: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w_after: Option<f32>,
    /// Tracked formatting change (`w:trPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<RowProps>>>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
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
    /// Vertical alignment (`w:vAlign`); `None` follows the table style, else top. An explicit
    /// `Top` beats the style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valign: Option<VAlign>,
    /// Cell margins override (top, left, bottom, right), points.
    pub margins: Option<[f32; 4]>,
    /// Per-side overrides when `margins` is absent; omitted sides inherit table defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_margins: Option<[Option<f32>; 4]>,
    /// Which way the cell's text runs.
    pub text_direction: TextDirection,
    /// The file states the horizontal direction (`lrTb`) outright; saving writes it back.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub text_direction_set: bool,
    pub no_wrap: bool,
    /// The cell's last paragraph takes no room when it is empty (`w:hideMark`): a row of such
    /// empty cells shrinks to its borders.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hide_mark: bool,
    /// Tracked formatting change (`w:tcPrChange`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fmt_change: Option<Box<PropChange<CellProps>>>,
}

impl CellProps {
    /// Resolve top, left, bottom and right margins against the current table defaults.
    /// A complete override takes precedence over per-side overrides.
    pub fn resolved_margins(&self, defaults: [f32; 4]) -> [f32; 4] {
        if let Some(margins) = self.margins {
            return margins;
        }
        self.partial_margins.map_or(defaults, |sides| std::array::from_fn(|i| sides[i].unwrap_or(defaults[i])))
    }
}

/// `tint` (towards `to` = 255) and `shade` (towards 0): `v` is the share of the original colour kept.
fn mix(c: Rgb, to: f64, v: f64) -> Rgb {
    let f = |x: u8| (x as f64 * v + to * (1.0 - v)).round().clamp(0.0, 255.0) as u8;
    Rgb(f(c.0), f(c.1), f(c.2))
}

/// `c` with its HSL luminance changed by `f`.
fn adjust_lum(c: Rgb, f: impl Fn(f64) -> f64) -> Rgb {
    let (h, s, l) = to_hsl(c);
    from_hsl(h, s, f(l).clamp(0.0, 1.0))
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.0 as f64 / 255.0, c.1 as f64 / 255.0, c.2 as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    if s == 0.0 {
        return Rgb(byte(l), byte(l), byte(l));
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |t: f64| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        byte(v)
    };
    Rgb(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
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

    /// The 16 `w14:ligatures` values of [MS-DOCX], each to its flags and back.
    #[test]
    fn ligature_names() {
        let l = |standard, contextual, historical, discretional| Ligatures { standard, contextual, historical, discretional };
        for (name, flags) in [
            ("none", l(false, false, false, false)),
            ("standard", l(true, false, false, false)),
            ("contextual", l(false, true, false, false)),
            ("historical", l(false, false, true, false)),
            ("discretional", l(false, false, false, true)),
            ("standardContextual", l(true, true, false, false)),
            ("standardHistorical", l(true, false, true, false)),
            ("contextualHistorical", l(false, true, true, false)),
            ("standardDiscretional", l(true, false, false, true)),
            ("contextualDiscretional", l(false, true, false, true)),
            ("historicalDiscretional", l(false, false, true, true)),
            ("standardContextualHistorical", l(true, true, true, false)),
            ("standardContextualDiscretional", l(true, true, false, true)),
            ("standardHistoricalDiscretional", l(true, false, true, true)),
            ("contextualHistoricalDiscretional", l(false, true, true, true)),
            ("all", l(true, true, true, true)),
        ] {
            assert_eq!(Ligatures::from_ooxml(name), Some(flags), "{name}");
            assert_eq!(flags.ooxml(), name);
        }
        assert_eq!(Ligatures::all().count(), 16);
        for bad in ["", "Standard", "contextualStandard", "standardstandard", "allx"] {
            assert_eq!(Ligatures::from_ooxml(bad), None, "{bad:?}");
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

    /// #865: a colour reads as `format.color` takes it (`RRGGBB`, `#RRGGBB`, `auto`), while what
    /// documents and settings already hold reads and writes exactly as before.
    #[test]
    fn text_color_reads_hex_and_keeps_its_serialized_forms() {
        let read = |j: &str| serde_json::from_str::<TextColor>(j);
        let ink = TextColor::Rgb(Rgb(0x1B, 0x16, 0x11));
        assert_eq!(read(r#""1B1611""#).unwrap(), ink);
        assert_eq!(read(r##""#1b1611""##).unwrap(), ink);
        assert_eq!(read(r#""auto""#).unwrap(), TextColor::Auto);
        assert_eq!(read(r#""Automatic""#).unwrap(), TextColor::Auto);
        for (c, j) in [(TextColor::Auto, r#""Auto""#), (ink, r#"{"Rgb":[27,22,17]}"#)] {
            assert_eq!(serde_json::to_string(&c).unwrap(), j, "written as before");
            assert_eq!(read(j).unwrap(), c);
        }
        for bad in [r#""1B161""#, r#""zzzzzz""#, r#""""#, r#"{"Rgb":"1B16"}"#, r#"{"Rgb":[1,2]}"#, r#"{"Hex":[1,2,3]}"#, "7", "[27,22,17]"] {
            assert!(read(bad).is_err(), "{bad}");
        }
        assert_eq!(read(r#"{"Rgb":"1B1611"}"#).unwrap(), ink);
        let p: CharProps = serde_json::from_value(serde_json::json!({"color": "#1B1611", "bold": true})).unwrap();
        assert_eq!((p.color, p.bold), (Some(ink), Some(true)));
    }

    /// #865: an [`Rgb`] reads `RRGGBB` / `#RRGGBB` as well as the `[r, g, b]` it still writes.
    #[test]
    fn rgb_reads_hex_and_keeps_its_serialized_form() {
        let read = |j: &str| serde_json::from_str::<Rgb>(j);
        let ink = Rgb(0x1B, 0x16, 0x11);
        assert_eq!(serde_json::to_string(&ink).unwrap(), "[27,22,17]", "written as before");
        for j in ["[27,22,17]", r#""1B1611""#, r##""#1b1611""##] {
            assert_eq!(read(j).unwrap(), ink, "{j}");
        }
        for bad in ["[27,22]", "[27,22,17,0]", "[27,22,300]", "[-1,0,0]", r#""auto""#, r#""1B161""#, r#""""#, "7", "null", r#"{"r":1}"#] {
            assert!(read(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn props_serde_skip_none() {
        let p = CharProps { bold: Some(true), ..Default::default() };
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, r#"{"bold":true}"#);
        let back: CharProps = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn symbol_font_is_optional_in_json_and_survives_font_changes() {
        // Older documents (no `symFont`) still load.
        let old: CharProps = serde_json::from_str(r#"{"font":"Arial"}"#).unwrap();
        assert_eq!(old.sym_font, None);
        let p = CharProps { font: Some("Arial".into()), sym_font: Some("Wingdings".into()), ..Default::default() };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains(r#""symFont":"Wingdings""#), "{s}");
        assert_eq!(serde_json::from_str::<CharProps>(&s).unwrap(), p);
        // A font change keeps the symbol's own font; Clear Formatting too.
        let changed = p.clone().overlaid(&CharProps { font: Some("Times New Roman".into()), ..Default::default() });
        assert_eq!(changed.sym_font.as_deref(), Some("Wingdings"));
        assert_eq!(p.cleared().sym_font.as_deref(), Some("Wingdings"));
    }
}
