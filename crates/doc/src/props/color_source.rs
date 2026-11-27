//! The theme colours a file gives text, underline, border and shading colours, kept beside the
//! colours they are shown in.
//!
//! WordprocessingML names such a colour by a colour of the document's theme (`w:themeColor`, a
//! shading's `w:themeFill`) with a tint or a shade, and stores a fixed colour beside it, which
//! only a reader without the theme shows: the theme colour supersedes it (ECMA-376 Part 1, 5th
//! edition, §17.3.2.6 `color`, §17.3.2.40 `u`, §17.3.4 `CT_Border`, §17.3.5 `CT_Shd`). The
//! properties ([`super::CharProps::color`] and the rest) hold the colour shown, which layout and
//! the other formats read. The records here hold the reference and the fixed colour as the file
//! gave them, so saving writes them back and the colour keeps following the theme; each owner
//! keeps its records in one box (`color_sources`), absent for colours without a theme colour.
//!
//! A colour chosen since replaces its record (`set_color` and the like, or a patch that sets the
//! colour). A record also stops counting once its property no longer has the colour it was shown
//! in ([`ColorSource::holds`]), so a colour changed in place is saved as it now is.

use super::color::{Rgb, TextColor};
use super::{Deserialize, Serialize};

/// A colour of the document's theme, as `w:themeColor` and `w:themeFill` name it
/// (`ST_ThemeColor`, ECMA-376 §17.18.97). `none`, no theme colour, isn't one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeColor {
    Dark1,
    Light1,
    Dark2,
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hyperlink,
    FollowedHyperlink,
    Background1,
    Text1,
    Background2,
    Text2,
}

impl ThemeColor {
    pub const ALL: [ThemeColor; 16] = [
        ThemeColor::Dark1,
        ThemeColor::Light1,
        ThemeColor::Dark2,
        ThemeColor::Light2,
        ThemeColor::Accent1,
        ThemeColor::Accent2,
        ThemeColor::Accent3,
        ThemeColor::Accent4,
        ThemeColor::Accent5,
        ThemeColor::Accent6,
        ThemeColor::Hyperlink,
        ThemeColor::FollowedHyperlink,
        ThemeColor::Background1,
        ThemeColor::Text1,
        ThemeColor::Background2,
        ThemeColor::Text2,
    ];

    /// The `ST_ThemeColor` name.
    pub fn ooxml(self) -> &'static str {
        match self {
            ThemeColor::Dark1 => "dark1",
            ThemeColor::Light1 => "light1",
            ThemeColor::Dark2 => "dark2",
            ThemeColor::Light2 => "light2",
            ThemeColor::Accent1 => "accent1",
            ThemeColor::Accent2 => "accent2",
            ThemeColor::Accent3 => "accent3",
            ThemeColor::Accent4 => "accent4",
            ThemeColor::Accent5 => "accent5",
            ThemeColor::Accent6 => "accent6",
            ThemeColor::Hyperlink => "hyperlink",
            ThemeColor::FollowedHyperlink => "followedHyperlink",
            ThemeColor::Background1 => "background1",
            ThemeColor::Text1 => "text1",
            ThemeColor::Background2 => "background2",
            ThemeColor::Text2 => "text2",
        }
    }

    /// The theme colour an `ST_ThemeColor` name gives; `None` for `none` and for a name the
    /// schema doesn't have.
    pub fn from_ooxml(s: &str) -> Option<ThemeColor> {
        ThemeColor::ALL.into_iter().find(|c| c.ooxml() == s)
    }

    /// Its colour in a theme's colour scheme, as an index into [`crate::Settings::theme_colors`]
    /// (dk1, lt1, dk2, lt2, accent1–6, hlink, folHlink). The dark and light colours are scheme
    /// colours themselves; the others go through the document's colour scheme mapping `map`
    /// (`w:clrSchemeMapping`: the scheme colour that text1, background1, text2, background2,
    /// accent1–6, hyperlink and followedHyperlink each stand for), or stand for their own when
    /// there is none.
    pub fn scheme_index(self, map: Option<&[u8; 12]>) -> usize {
        let mapped = |i: usize| map.and_then(|m| m.get(i)).map_or(i, |v| usize::from(*v));
        match self {
            ThemeColor::Dark1 => 0,
            ThemeColor::Light1 => 1,
            ThemeColor::Dark2 => 2,
            ThemeColor::Light2 => 3,
            ThemeColor::Text1 => mapped(0),
            ThemeColor::Background1 => mapped(1),
            ThemeColor::Text2 => mapped(2),
            ThemeColor::Background2 => mapped(3),
            ThemeColor::Accent1 => mapped(4),
            ThemeColor::Accent2 => mapped(5),
            ThemeColor::Accent3 => mapped(6),
            ThemeColor::Accent4 => mapped(7),
            ThemeColor::Accent5 => mapped(8),
            ThemeColor::Accent6 => mapped(9),
            ThemeColor::Hyperlink => mapped(10),
            ThemeColor::FollowedHyperlink => mapped(11),
        }
    }
}

/// A theme colour with the tint or shade a file gives it: `w:themeTint` and `w:themeShade`, or a
/// shading's `w:themeFillTint` and `w:themeFillShade` (hex bytes, `ST_UcharHexNumber`,
/// ECMA-376 §17.18.98). Both are kept when the file gives both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeRef {
    pub color: ThemeColor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade: Option<u8>,
}

impl ThemeRef {
    /// The colour it names in `palette` (a theme's colours, in [`ThemeColor::scheme_index`]'s
    /// order) through the colour scheme mapping `map`, with its tint, or else its shade, applied
    /// to the colour's HSL luminance: a tint `t` makes it `L·t + (1 − t)`, a shade `s` makes it
    /// `L·s`, and a tint given with a shade is applied alone ([MS-OI29500] §2.1.72, notes to
    /// Part 1 §17.3.2.6). `None` when the palette has no such colour (the document has no theme).
    pub fn resolve(&self, palette: &[Rgb], map: Option<&[u8; 12]>) -> Option<Rgb> {
        let c = *palette.get(self.color.scheme_index(map))?;
        let byte = |v: u8| f64::from(v) / 255.0;
        Some(match (self.tint, self.shade) {
            (Some(t), _) => c.with_lum(|l| l * byte(t) + (1.0 - byte(t))),
            (None, Some(s)) => c.with_lum(|l| l * byte(s)),
            (None, None) => c,
        })
    }
}

/// A text, underline or border colour given as a theme colour: the reference, the fixed colour
/// the file stores beside it, and the colour the property was shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorSource {
    /// The theme colour (`w:themeColor`, `w:themeTint`, `w:themeShade`).
    pub theme: ThemeRef,
    /// The fixed colour stored with it (`w:val` of a text colour, `w:color` of an underline or a
    /// border): `None` when the file gives none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<TextColor>,
    /// The colour the property had when the reference was read or last resolved; a border's or
    /// an underline's automatic colour is `None`. See [`ColorSource::holds`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<TextColor>,
}

impl ColorSource {
    /// The reference still stands for its property, whose colour is `now`: the property has the
    /// colour the reference was shown in. A colour set since, without replacing the record,
    /// differs from it, and is saved as set.
    pub fn holds(&self, now: Option<TextColor>) -> bool {
        self.shown == now
    }
}

/// The shading patterns of `ST_Shd` (ECMA-376 §17.18.78).
pub const SHADING_PATTERNS: [&str; 38] = [
    "nil",
    "clear",
    "solid",
    "horzStripe",
    "vertStripe",
    "reverseDiagStripe",
    "diagStripe",
    "horzCross",
    "diagCross",
    "thinHorzStripe",
    "thinVertStripe",
    "thinReverseDiagStripe",
    "thinDiagStripe",
    "thinHorzCross",
    "thinDiagCross",
    "pct5",
    "pct10",
    "pct12",
    "pct15",
    "pct20",
    "pct25",
    "pct30",
    "pct35",
    "pct37",
    "pct40",
    "pct45",
    "pct50",
    "pct55",
    "pct60",
    "pct62",
    "pct65",
    "pct70",
    "pct75",
    "pct80",
    "pct85",
    "pct87",
    "pct90",
    "pct95",
];

/// A shading (`w:shd`, `CT_Shd`) with a theme colour on its pattern colour, its fill or both, as
/// the file gives it: every attribute, those of a colour the pattern doesn't show included.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShadingSource {
    /// The pattern (`w:val`), one of [`SHADING_PATTERNS`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// The pattern colour (`w:color`) and its theme colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<TextColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_theme: Option<ThemeRef>,
    /// The fill (`w:fill`) and its theme colour (`w:themeFill`, `w:themeFillTint`,
    /// `w:themeFillShade`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<TextColor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_theme: Option<ThemeRef>,
    /// The shading the property had when read or last resolved ([`ColorSource::shown`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shown: Option<Rgb>,
}

impl ShadingSource {
    /// The record still stands for its shading, which is `now` ([`ColorSource::holds`]).
    pub fn holds(&self, now: Option<Rgb>) -> bool {
        self.shown == now
    }
}

/// The theme colours of a set of borders, side by side with [`super::Borders`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BorderColorSources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub between: Option<ColorSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside_v: Option<ColorSource>,
}

impl BorderColorSources {
    pub fn is_empty(&self) -> bool {
        *self == BorderColorSources::default()
    }

    fn sides_mut(&mut self) -> [&mut Option<ColorSource>; 6] {
        [&mut self.top, &mut self.left, &mut self.bottom, &mut self.right, &mut self.between, &mut self.inside_v]
    }

    fn sides(&self) -> [Option<ColorSource>; 6] {
        [self.top, self.left, self.bottom, self.right, self.between, self.inside_v]
    }

    /// Side by side with [`super::Borders::overlay`]: a side `patch` sets takes its record from
    /// `sources` (none when it has none); the others keep theirs.
    pub fn overlay(&mut self, patch: &super::Borders, sources: &BorderColorSources) {
        let set = [patch.top, patch.left, patch.bottom, patch.right, patch.between, patch.inside_v].map(|s| s.is_some());
        for ((mine, theirs), set) in self.sides_mut().into_iter().zip(sources.sides()).zip(set) {
            if set || theirs.is_some() {
                *mine = theirs;
            }
        }
    }
}

/// The theme colours of a run's properties ([`super::CharProps::color_sources`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CharColorSources {
    /// The text colour's (`w:color`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorSource>,
    /// The underline colour's (`w:u`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<ColorSource>,
    /// The character border's (`w:bdr`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<ColorSource>,
    /// The character shading's (`w:shd`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<ShadingSource>,
}

impl CharColorSources {
    pub fn is_empty(&self) -> bool {
        self.color.is_none() && self.underline.is_none() && self.border.is_none() && self.shading.is_none()
    }
}

/// The theme colours of a paragraph's, a table's, a cell's or a table style part's shading and
/// borders.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BlockColorSources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shading: Option<ShadingSource>,
    #[serde(skip_serializing_if = "BorderColorSources::is_empty")]
    pub borders: BorderColorSources,
}

impl BlockColorSources {
    pub fn is_empty(&self) -> bool {
        self.shading.is_none() && self.borders.is_empty()
    }
}

/// `holder`'s record, `None` when it has none, as `get` picks it out.
pub(crate) fn source<'a, H, T>(holder: &'a Option<Box<H>>, get: impl Fn(&'a H) -> &'a Option<T>) -> Option<&'a T> {
    holder.as_deref().and_then(|h| get(h).as_ref())
}

/// Put `value` in `holder`'s place `get` picks: the box comes when there is something to hold
/// and goes when it holds nothing.
pub(crate) fn put<H: Default, T>(holder: &mut Option<Box<H>>, get: impl Fn(&mut H) -> &mut Option<T>, value: Option<T>, empty: impl Fn(&H) -> bool) {
    match holder {
        Some(h) => {
            *get(h) = value;
            if empty(h) {
                *holder = None;
            }
        }
        None => {
            if value.is_some() {
                let mut h = H::default();
                *get(&mut h) = value;
                *holder = Some(Box::new(h));
            }
        }
    }
}

/// The borders' records of `holder`, the empty set when it has none.
pub(crate) fn borders_of(holder: &Option<Box<BlockColorSources>>) -> BorderColorSources {
    holder.as_deref().map(|h| h.borders).unwrap_or_default()
}

/// Replace the borders' records of `holder` with `borders`.
pub(crate) fn put_borders(holder: &mut Option<Box<BlockColorSources>>, borders: BorderColorSources) {
    match holder {
        Some(h) => {
            h.borders = borders;
            if h.is_empty() {
                *holder = None;
            }
        }
        None if !borders.is_empty() => *holder = Some(Box::new(BlockColorSources { borders, ..BlockColorSources::default() })),
        None => {}
    }
}

/// `edit_color_sources` (or the like) for each owner of theme colour records.
macro_rules! edit_sources {
    ($owner:ty, $field:ident, $edit:ident, $holder:ty) => {
        impl $owner {
            #[doc = concat!("Change the theme colour records of the colours (`", stringify!($field), "`) with `f`: the box comes when they hold one and goes when they hold none.")]
            pub fn $edit(&mut self, f: impl FnOnce(&mut $holder)) {
                let mut held = self.$field.take().map(|b| *b).unwrap_or_default();
                f(&mut held);
                self.$field = (!held.is_empty()).then(|| Box::new(held));
            }
        }
    };
}

edit_sources!(super::CharProps, color_sources, edit_color_sources, CharColorSources);
edit_sources!(super::ParaProps, color_sources, edit_color_sources, BlockColorSources);
edit_sources!(super::TableProps, color_sources, edit_color_sources, BlockColorSources);
edit_sources!(super::CellProps, color_sources, edit_color_sources, BlockColorSources);
edit_sources!(crate::styles::PartFormat, color_sources, edit_color_sources, BlockColorSources);
edit_sources!(crate::section::SectionProps, page_border_sources, edit_page_border_sources, BorderColorSources);

/// The colours a document resolves theme colours through: its theme's colours and its colour
/// scheme mapping ([`ThemeRef::resolve`]).
#[derive(Clone, Copy, Debug)]
pub struct Palette<'a> {
    pub colors: &'a [Rgb],
    pub map: Option<&'a [u8; 12]>,
}

impl Palette<'_> {
    fn resolve(&self, t: &ThemeRef) -> Option<Rgb> {
        t.resolve(self.colors, self.map)
    }
}

impl ColorSource {
    /// Show its property, now `now`, in the colour its theme colour gives under `p`, while the
    /// record still stands for it ([`ColorSource::holds`]): a colour set since stays as set.
    fn refresh(&mut self, now: &mut Option<TextColor>, p: Palette) {
        if self.holds(*now)
            && let Some(c) = p.resolve(&self.theme)
        {
            *now = Some(TextColor::Rgb(c));
            self.shown = *now;
        }
    }

    /// [`ColorSource::refresh`] for a colour without the automatic one (an underline's, a
    /// border's: `None` is automatic there).
    fn refresh_rgb(&mut self, now: &mut Option<Rgb>, p: Palette) {
        let mut shown = now.map(TextColor::Rgb);
        self.refresh(&mut shown, p);
        if let Some(TextColor::Rgb(c)) = shown {
            *now = Some(c);
        }
    }
}

impl ShadingSource {
    /// The shading it shows under `p`, as a file's `w:shd` is read: none for `nil`, else the
    /// fill's theme colour, then (for a solid pattern without a fill) the pattern's, then the
    /// fill, or a solid pattern's colour.
    pub fn resolve(&self, p: Palette) -> Option<Rgb> {
        let pattern = self.pattern.as_deref();
        if pattern == Some("nil") {
            return None;
        }
        let rgb = |c: Option<TextColor>| match c {
            Some(TextColor::Rgb(c)) => Some(c),
            _ => None,
        };
        if let Some(c) = self.fill_theme.as_ref().and_then(|t| p.resolve(t)) {
            return Some(c);
        }
        if pattern == Some("solid")
            && rgb(self.fill).is_none()
            && let Some(c) = self.color_theme.as_ref().and_then(|t| p.resolve(t))
        {
            return Some(c);
        }
        rgb(self.fill).or_else(|| if pattern == Some("solid") { rgb(self.color) } else { None })
    }

    /// [`ColorSource::refresh`] for a shading.
    fn refresh(&mut self, now: &mut Option<Rgb>, p: Palette) {
        if self.holds(*now) {
            *now = self.resolve(p);
            self.shown = *now;
        }
    }
}

impl BorderColorSources {
    fn refresh(&mut self, borders: &mut Option<super::Borders>, p: Palette) {
        let Some(b) = borders else { return };
        let sides = [&mut b.top, &mut b.left, &mut b.bottom, &mut b.right, &mut b.between, &mut b.inside_v];
        for (side, source) in sides.into_iter().zip(self.sides_mut()) {
            if let (Some(border), Some(source)) = (side, source) {
                source.refresh_rgb(&mut border.color, p);
            }
        }
    }
}

impl BlockColorSources {
    fn refresh(&mut self, shading: &mut Option<Rgb>, borders: &mut Option<super::Borders>, p: Palette) {
        if let Some(s) = &mut self.shading {
            s.refresh(shading, p);
        }
        self.borders.refresh(borders, p);
    }
}

impl super::CharProps {
    /// Show the colours that still follow a theme colour in the colours it gives under `p` (the
    /// tracked change's colours before it too).
    pub fn refresh_theme_colors(&mut self, p: Palette) {
        if let Some(s) = self.color_sources.as_deref_mut() {
            if let Some(c) = &mut s.color {
                c.refresh(&mut self.color, p);
            }
            if let Some(c) = &mut s.underline {
                c.refresh_rgb(&mut self.underline_color, p);
            }
            if let (Some(c), Some(b)) = (&mut s.border, &mut self.border) {
                c.refresh_rgb(&mut b.color, p);
            }
            if let Some(c) = &mut s.shading {
                c.refresh(&mut self.shading, p);
            }
        }
        if let Some(ch) = &mut self.fmt_change {
            ch.old.refresh_theme_colors(p);
        }
    }

    /// Whether any colour, or the tracked change's colours before it, follows a theme colour.
    pub fn has_theme_colors(&self) -> bool {
        self.color_sources.is_some() || self.fmt_change.as_ref().is_some_and(|ch| ch.old.has_theme_colors())
    }
}

/// [`super::CharProps::refresh_theme_colors`] for the owners of a shading and borders.
macro_rules! refresh_block {
    ($owner:ty, $shading:ident) => {
        impl $owner {
            /// Show the shading and the borders' colours that still follow a theme colour in the
            /// colours it gives under `p` (the tracked change's before it too).
            pub fn refresh_theme_colors(&mut self, p: Palette) {
                if let Some(s) = self.color_sources.as_deref_mut() {
                    s.refresh(&mut self.$shading, &mut self.borders, p);
                }
                if let Some(ch) = &mut self.fmt_change {
                    ch.old.refresh_theme_colors(p);
                }
            }

            /// Whether the shading or a border, or those before a tracked change, follow a theme
            /// colour.
            pub fn has_theme_colors(&self) -> bool {
                self.color_sources.is_some() || self.fmt_change.as_ref().is_some_and(|ch| ch.old.has_theme_colors())
            }
        }
    };
}

refresh_block!(super::ParaProps, shading);
refresh_block!(super::TableProps, shading);
refresh_block!(super::CellProps, shading);

impl crate::styles::PartFormat {
    /// [`super::CharProps::refresh_theme_colors`] for a table style part.
    pub fn refresh_theme_colors(&mut self, p: Palette) {
        self.chr.refresh_theme_colors(p);
        if let Some(s) = self.color_sources.as_deref_mut() {
            s.refresh(&mut self.fill, &mut self.borders, p);
        }
    }
}

impl crate::section::SectionProps {
    /// [`super::CharProps::refresh_theme_colors`] for the page borders.
    pub fn refresh_theme_colors(&mut self, p: Palette) {
        if let Some(s) = self.page_border_sources.as_deref_mut() {
            s.refresh(&mut self.page_borders, p);
        }
        if let Some(ch) = &mut self.fmt_change {
            ch.old.refresh_theme_colors(p);
        }
    }

    /// Whether a page border, or one before a tracked change, follows a theme colour.
    pub fn has_theme_colors(&self) -> bool {
        self.page_border_sources.is_some() || self.fmt_change.as_ref().is_some_and(|ch| ch.old.has_theme_colors())
    }
}

/// How deep [`crate::Document::refresh_theme_colors`] follows tables in table cells.
const MAX_TABLE_DEPTH: usize = 64;

/// Whether `b` holds a colour that follows a theme colour, in tables `depth` deep.
fn block_has_theme_colors(b: &crate::Block, depth: usize) -> bool {
    match b {
        crate::Block::Para(p) => {
            p.props.has_theme_colors()
                || p.mark.has_theme_colors()
                || p.runs.iter().any(|r| r.props.has_theme_colors())
                || p.section.as_deref().is_some_and(crate::section::SectionProps::has_theme_colors)
        }
        crate::Block::Table(t) => {
            t.props.has_theme_colors()
                || t.rows.iter().any(|r| {
                    r.props.exceptions.as_deref().is_some_and(super::TableProps::has_theme_colors)
                        || r.cells.iter().any(|c| {
                            c.props.has_theme_colors() || (depth < MAX_TABLE_DEPTH && c.blocks.iter().any(|b| block_has_theme_colors(b, depth + 1)))
                        })
                })
        }
    }
}

/// An owner of theme colour records, as [`crate::Document`]'s walk hands it out.
enum ThemeColored<'a> {
    Char(&'a mut super::CharProps),
    Para(&'a mut super::ParaProps),
    Table(&'a mut super::TableProps),
    Cell(&'a mut super::CellProps),
    Part(&'a mut crate::styles::PartFormat),
    Section(&'a mut crate::section::SectionProps),
}

/// Hand `f` each owner in the blocks of a story, the tables `depth` deep; a block without theme
/// colours isn't copied.
fn each_in_blocks(blocks: &mut [std::sync::Arc<crate::Block>], f: &mut dyn FnMut(ThemeColored<'_>), depth: usize) {
    for b in blocks.iter_mut().filter(|b| block_has_theme_colors(b, depth)) {
        match std::sync::Arc::make_mut(b) {
            crate::Block::Para(para) => {
                f(ThemeColored::Para(&mut para.props));
                f(ThemeColored::Char(&mut para.mark));
                for r in &mut para.runs {
                    f(ThemeColored::Char(&mut r.props));
                }
                if let Some(s) = para.section.as_deref_mut() {
                    f(ThemeColored::Section(s));
                }
            }
            crate::Block::Table(t) => {
                f(ThemeColored::Table(&mut t.props));
                for r in &mut t.rows {
                    if let Some(x) = r.props.exceptions.as_deref_mut() {
                        f(ThemeColored::Table(x));
                    }
                    for c in &mut r.cells {
                        f(ThemeColored::Cell(&mut c.props));
                        if depth < MAX_TABLE_DEPTH {
                            each_in_blocks(&mut c.blocks, f, depth + 1);
                        }
                    }
                }
            }
        }
    }
}

/// Hand `f` each owner in a style, its tracked changes' and table style parts' too.
fn each_in_style(st: &mut crate::Style, f: &mut dyn FnMut(ThemeColored<'_>)) {
    f(ThemeColored::Char(&mut st.chr));
    f(ThemeColored::Para(&mut st.para));
    if let Some(ch) = &mut st.chr_change {
        f(ThemeColored::Char(&mut ch.old));
    }
    if let Some(ch) = &mut st.para_change {
        f(ThemeColored::Para(&mut ch.old));
    }
    if let Some(t) = &mut st.table {
        for part in crate::styles::TablePart::ALL {
            let mut p = t.part(part);
            if p.color_sources.is_some() || p.chr.has_theme_colors() {
                f(ThemeColored::Part(&mut p));
                t.set_part(part, p);
            }
        }
    }
}

fn style_has_theme_colors(st: &crate::Style) -> bool {
    st.chr.has_theme_colors()
        || st.para.has_theme_colors()
        || st.chr_change.as_ref().is_some_and(|ch| ch.old.has_theme_colors())
        || st.para_change.as_ref().is_some_and(|ch| ch.old.has_theme_colors())
        || st.table.as_ref().is_some_and(|t| {
            crate::styles::TablePart::ALL.into_iter().any(|part| {
                let f = t.part(part);
                f.color_sources.is_some() || f.chr.has_theme_colors()
            })
        })
}

impl super::CharProps {
    /// Drop the theme colour records (the tracked change's colours before it too): the colours
    /// stay as shown, as fixed colours.
    fn forget_theme_colors(&mut self) {
        self.color_sources = None;
        if let Some(ch) = &mut self.fmt_change {
            ch.old.forget_theme_colors();
        }
    }
}

/// [`super::CharProps::forget_theme_colors`] for the owners of a shading and borders.
macro_rules! forget_block {
    ($($owner:ty),*) => {
        $( impl $owner {
            fn forget_theme_colors(&mut self) {
                self.color_sources = None;
                if let Some(ch) = &mut self.fmt_change {
                    ch.old.forget_theme_colors();
                }
            }
        } )*
    };
}

forget_block!(super::ParaProps, super::TableProps, super::CellProps);

impl crate::section::SectionProps {
    fn forget_theme_colors(&mut self) {
        self.page_border_sources = None;
        if let Some(ch) = &mut self.fmt_change {
            ch.old.forget_theme_colors();
        }
    }
}

impl crate::Document {
    /// Hand `f` every owner of theme colour records: the styles (and their table style parts),
    /// the list levels, every story's paragraphs, tables and cells, and the sections. Only what
    /// holds records is copied.
    fn each_theme_colored(&mut self, f: &mut dyn FnMut(ThemeColored<'_>)) {
        let sheet = &self.styles;
        let styled = sheet.default_chr.has_theme_colors() || sheet.default_para.has_theme_colors() || sheet.styles.iter().any(style_has_theme_colors);
        if styled {
            let sheet = std::sync::Arc::make_mut(&mut self.styles);
            f(ThemeColored::Char(&mut sheet.default_chr));
            f(ThemeColored::Para(&mut sheet.default_para));
            for st in sheet.styles.iter_mut().filter(|st| style_has_theme_colors(st)) {
                each_in_style(st, f);
            }
        }
        let numbering = &self.numbering;
        let mut listed = numbering
            .abstracts
            .iter()
            .flat_map(|a| a.levels.iter())
            .chain(numbering.nums.iter().flat_map(|n| n.level_overrides.iter().map(|(_, l)| l)));
        if listed.any(|l| l.chr.has_theme_colors()) {
            let numbering = std::sync::Arc::make_mut(&mut self.numbering);
            for l in numbering.abstracts.iter_mut().flat_map(|a| a.levels.iter_mut()) {
                f(ThemeColored::Char(&mut l.chr));
            }
            for (_, l) in numbering.nums.iter_mut().flat_map(|n| n.level_overrides.iter_mut()) {
                f(ThemeColored::Char(&mut l.chr));
            }
        }
        each_in_blocks(&mut self.body, f, 0);
        for part in self.parts.values_mut() {
            each_in_blocks(&mut part.blocks, f, 0);
        }
        for bb in &mut self.glossary {
            each_in_blocks(&mut bb.blocks, f, 0);
        }
        for seps in [&mut self.footnote_separators, &mut self.endnote_separators] {
            for blocks in [&mut seps.separator, &mut seps.continuation_separator, &mut seps.continuation_notice].into_iter().flatten() {
                each_in_blocks(blocks, f, 0);
            }
        }
        f(ThemeColored::Section(&mut self.last_section));
    }

    /// After the theme's colours or its colour scheme mapping changed (Design › Themes): every
    /// colour that still follows a theme colour is shown in the colour that theme colour gives
    /// now, its record kept. Colours set since they were read, and fixed colours, stay as they
    /// are, and nothing counts as a formatting change.
    pub fn refresh_theme_colors(&mut self) {
        let colors = self.settings.theme_colors.clone();
        let map = self.settings.theme_color_map;
        let p = Palette { colors: &colors, map: map.as_ref() };
        if let Some(s) = &mut self.settings.page_color_source {
            s.refresh_rgb(&mut self.settings.page_color, p);
        }
        self.each_theme_colored(&mut |owner| match owner {
            ThemeColored::Char(c) => c.refresh_theme_colors(p),
            ThemeColored::Para(c) => c.refresh_theme_colors(p),
            ThemeColored::Table(c) => c.refresh_theme_colors(p),
            ThemeColored::Cell(c) => c.refresh_theme_colors(p),
            ThemeColored::Part(c) => c.refresh_theme_colors(p),
            ThemeColored::Section(c) => c.refresh_theme_colors(p),
        });
    }

    /// Drop every theme colour record, the colours staying as shown: for content brought into
    /// a document whose theme gives theme colours other colours (`Document::adopt`).
    pub fn forget_theme_colors(&mut self) {
        self.settings.page_color_source = None;
        self.each_theme_colored(&mut |owner| match owner {
            ThemeColored::Char(c) => c.forget_theme_colors(),
            ThemeColored::Para(c) => c.forget_theme_colors(),
            ThemeColored::Table(c) => c.forget_theme_colors(),
            ThemeColored::Cell(c) => c.forget_theme_colors(),
            ThemeColored::Part(c) => {
                c.chr.forget_theme_colors();
                c.color_sources = None;
            }
            ThemeColored::Section(c) => c.forget_theme_colors(),
        });
    }

    /// Whether `other` resolves theme colours as this document does: the same theme colours and
    /// colour scheme mapping.
    pub fn same_theme_colors(&self, other: &crate::Document) -> bool {
        self.settings.theme_colors == other.settings.theme_colors && self.settings.theme_color_map == other.settings.theme_color_map
    }
}

impl BorderColorSources {
    /// The records of the sides `new` keeps as `old` has them; a side it changes loses its own
    /// (its colour was chosen with it).
    pub fn kept_by(&self, old: Option<&super::Borders>, new: Option<&super::Borders>) -> BorderColorSources {
        let sides = |b: Option<&super::Borders>| b.map(|b| [b.top, b.left, b.bottom, b.right, b.between, b.inside_v]).unwrap_or_default();
        let (before, after) = (sides(old), sides(new));
        let mut kept = *self;
        for ((source, before), after) in kept.sides_mut().into_iter().zip(before).zip(after) {
            if before != after {
                *source = None;
            }
        }
        kept
    }
}

/// `replace_borders` for the owners of borders.
macro_rules! replace_borders {
    ($owner:ty) => {
        impl $owner {
            /// Set the borders: a side that changes loses its theme colour, a side left as it was
            /// keeps it.
            pub fn replace_borders(&mut self, borders: Option<super::Borders>) {
                let kept = borders_of(&self.color_sources).kept_by(self.borders.as_ref(), borders.as_ref());
                self.borders = borders;
                put_borders(&mut self.color_sources, kept);
            }
        }
    };
}

replace_borders!(super::ParaProps);
replace_borders!(super::TableProps);
replace_borders!(super::CellProps);

impl crate::section::SectionProps {
    /// [`super::ParaProps::replace_borders`] for the page borders.
    pub fn replace_page_borders(&mut self, borders: Option<super::Borders>) {
        let kept = self.page_border_sources.as_deref().map(|s| s.kept_by(self.page_borders.as_ref(), borders.as_ref())).unwrap_or_default();
        self.page_borders = borders;
        self.page_border_sources = (!kept.is_empty()).then(|| Box::new(kept));
    }
}

replace_borders!(crate::styles::PartFormat);

impl crate::styles::PartFormat {
    /// Set the fill as a colour chosen: the theme colour it had goes.
    pub fn set_fill(&mut self, fill: Option<Rgb>) {
        self.fill = fill;
        put(&mut self.color_sources, |s| &mut s.shading, None, BlockColorSources::is_empty);
    }

    /// Set the borders, their colours chosen: the theme colours they had go.
    pub fn set_borders(&mut self, borders: Option<super::Borders>) {
        self.borders = borders;
        put_borders(&mut self.color_sources, BorderColorSources::default());
    }
}

/// A colour that still follows its theme colour, as its authoring compares it: by the record
/// alone, without the colour it resolves to (the theme's to give).
fn authored_color(now: &mut Option<TextColor>, source: &mut Option<ColorSource>) {
    if let Some(s) = source
        && s.holds(*now)
    {
        (*now, s.shown) = (None, None);
    }
}

fn authored_rgb(now: &mut Option<Rgb>, source: &mut Option<ColorSource>) {
    if let Some(s) = source
        && s.holds(now.map(TextColor::Rgb))
    {
        (*now, s.shown) = (None, None);
    }
}

fn authored_shading(now: &mut Option<Rgb>, source: &mut Option<ShadingSource>) {
    if let Some(s) = source
        && s.holds(*now)
    {
        (*now, s.shown) = (None, None);
    }
}

fn authored_borders(borders: &mut Option<super::Borders>, sources: &mut BorderColorSources) {
    if let Some(b) = borders {
        let sides = [&mut b.top, &mut b.left, &mut b.bottom, &mut b.right, &mut b.between, &mut b.inside_v];
        for (side, source) in sides.into_iter().zip(sources.sides_mut()) {
            if let Some(border) = side {
                authored_rgb(&mut border.color, source);
            }
        }
    }
}

impl super::CharProps {
    /// These properties as their authoring compares them: each colour that still follows its
    /// theme colour without the colour it resolves to, so a new theme's colours make no change,
    /// where a colour chosen in its place, one that looks the same included, does.
    pub fn authored(&self) -> super::CharProps {
        let mut c = self.clone();
        if let Some(s) = c.color_sources.as_deref_mut() {
            authored_color(&mut c.color, &mut s.color);
            authored_rgb(&mut c.underline_color, &mut s.underline);
            if let Some(b) = &mut c.border {
                authored_rgb(&mut b.color, &mut s.border);
            }
            authored_shading(&mut c.shading, &mut s.shading);
        }
        if let Some(ch) = &mut c.fmt_change {
            ch.old = ch.old.authored();
        }
        c
    }
}

/// [`super::CharProps::authored`] for the owners of a shading and borders.
macro_rules! authored_block {
    ($owner:ty) => {
        impl $owner {
            /// These properties as their authoring compares them ([`super::CharProps::authored`]).
            pub fn authored(&self) -> $owner {
                let mut c = self.clone();
                if let Some(s) = c.color_sources.as_deref_mut() {
                    authored_shading(&mut c.shading, &mut s.shading);
                    authored_borders(&mut c.borders, &mut s.borders);
                }
                if let Some(ch) = &mut c.fmt_change {
                    ch.old = ch.old.authored();
                }
                c
            }
        }
    };
}

authored_block!(super::ParaProps);
authored_block!(super::TableProps);
authored_block!(super::CellProps);

impl super::RowProps {
    /// These properties as their authoring compares them ([`super::CharProps::authored`]): the
    /// table properties the row overrides.
    pub fn authored(&self) -> super::RowProps {
        let mut r = self.clone();
        if let Some(x) = &mut r.exceptions {
            **x = x.authored();
        }
        if let Some(ch) = &mut r.fmt_change {
            ch.old = ch.old.authored();
        }
        r
    }
}

impl crate::section::SectionProps {
    /// These properties as their authoring compares them ([`super::CharProps::authored`]).
    pub fn authored(&self) -> crate::section::SectionProps {
        let mut s = self.clone();
        if let Some(sources) = s.page_border_sources.as_deref_mut() {
            authored_borders(&mut s.page_borders, sources);
        }
        if let Some(ch) = &mut s.fmt_change {
            ch.old = ch.old.authored();
        }
        s
    }
}
