use super::xml::{Attrs, Ev, events, get, length, local};
use super::{Align, Arc, Columns, Fmt, HashMap, Kind, Rgb, VAlign, model};
use crate::model::Spacing;
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing, TabAlign, TabLeader, TabStop, Underline};
use wordcraft_doc::section::NumFormat;

// ---------------------------------------------------------------------------------------------
// Import

#[derive(Clone, Debug, Default)]
struct FmtDelta {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    sup: Option<bool>,
    sub: Option<bool>,
    color: Option<Rgb>,
    background: Option<Rgb>,
    size: Option<f32>,
    /// A size in percent of the inherited one (`fo:font-size="130%"`).
    size_pct: Option<f32>,
    font: Option<Arc<str>>,
    code: Option<bool>,
    /// What [`Fmt::extra`] holds, as set here (unset fields leave the inherited ones).
    extra: wordcraft_doc::props::CharProps,
    /// `style:text-position` raising or lowering text at full size: percent of its height.
    raise: Option<f32>,
}

impl FmtDelta {
    /// Whether the properties read so far underline the text.
    fn fmt_underline_on(&self) -> bool {
        self.underline == Some(true)
    }

    fn apply(&self, f: &mut Fmt) {
        macro_rules! set {
            ($($k:ident),*) => { $( if let Some(v) = self.$k.clone() { f.$k = v; } )* };
        }
        set!(bold, italic, underline, strike, sup, sub, code);
        if self.color.is_some() {
            f.color = self.color;
        }
        if self.background.is_some() {
            f.background = self.background;
        }
        if self.size.is_some() {
            f.size = self.size;
        }
        if let Some(p) = self.size_pct {
            f.size = Some((f.size.unwrap_or(12.0) * p / 100.0).clamp(1.0, 1638.0));
        }
        if let Some(fam) = &self.font {
            f.font = Some(fam.clone());
        }
        if self.extra != wordcraft_doc::props::CharProps::default() || self.raise.is_some() {
            let mut x = f.extra.take().map(|b| *b).unwrap_or_default();
            x.overlay(&self.extra);
            if let Some(r) = self.raise {
                let size = f.size.unwrap_or(12.0);
                x.position = Some((r / 100.0 * size).clamp(-1584.0, 1584.0)).filter(|p| *p != 0.0);
            }
            // An explicit "off" only cancels what a parent style set.
            for b in
                [&mut x.double_strike, &mut x.caps, &mut x.small_caps, &mut x.hidden, &mut x.outline, &mut x.shadow, &mut x.emboss, &mut x.engrave]
            {
                if *b == Some(false) {
                    *b = None;
                }
            }
            x.spacing = x.spacing.filter(|s| *s != 0.0);
            x.scale = x.scale.filter(|s| *s != 100.0);
            f.extra = (x != wordcraft_doc::props::CharProps::default()).then(|| Box::new(x));
        }
    }
}

/// The underline `style:text-underline-style`, `-type`, `-width` and `-mode` describe (ODF 1.3
/// §20.390–20.393), when underlined.
fn underline_kind(a: &Attrs) -> Underline {
    let style = get(a, "style:text-underline-style").map(str::trim).unwrap_or("solid");
    let double = get(a, "style:text-underline-type").is_some_and(|t| t.trim() == "double");
    let bold = get(a, "style:text-underline-width").is_some_and(|w| matches!(w.trim(), "bold" | "thick" | "medium"));
    let words = get(a, "style:text-underline-mode").is_some_and(|m| m.trim() == "skip-white-space");
    match style {
        "dotted" => Underline::Dotted,
        "dash" | "long-dash" => Underline::Dash,
        "dot-dash" => Underline::DotDash,
        "dot-dot-dash" => Underline::DotDotDash,
        "wave" if double => Underline::DoubleWave,
        "wave" => Underline::Wave,
        _ if double => Underline::Double,
        _ if bold => Underline::Thick,
        _ if words => Underline::Words,
        _ => Underline::Single,
    }
}

/// The rest of a `style:text-properties` element: what only [`Fmt::extra`] holds.
fn extra_text_properties(a: &Attrs, d: &mut FmtDelta) {
    let underlined = d.fmt_underline_on();
    let x = &mut d.extra;
    if underlined {
        x.underline = Some(underline_kind(a));
        if let Some(c) = get(a, "style:text-underline-color").filter(|c| c.trim() != "font-color").and_then(model::parse_color) {
            x.underline_color = Some(c);
        }
    }
    if let Some(t) = get(a, "style:text-line-through-type") {
        x.double_strike = Some(t.trim() == "double");
    }
    if let Some(t) = get(a, "fo:text-transform") {
        x.caps = Some(t.trim() == "uppercase");
    }
    if let Some(v) = get(a, "fo:font-variant") {
        x.small_caps = Some(v.trim() == "small-caps");
    }
    if let Some(v) = get(a, "text:display") {
        x.hidden = Some(v.trim() == "none");
    }
    if let Some(v) = get(a, "fo:letter-spacing") {
        x.spacing = Some(if v.trim() == "normal" { 0.0 } else { length(v).unwrap_or(0.0).clamp(-1584.0, 1584.0) });
    }
    if let Some(v) = get(a, "style:text-scale")
        .and_then(|v| v.trim().strip_suffix('%'))
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
    {
        x.scale = Some(v.clamp(1.0, 600.0));
    }
    if let Some(v) = get(a, "style:text-outline") {
        x.outline = Some(v.trim() == "true");
    }
    if let Some(v) = get(a, "fo:text-shadow") {
        x.shadow = Some(v.trim() != "none");
    }
    if let Some(v) = get(a, "style:font-relief") {
        (x.emboss, x.engrave) = (Some(v.trim() == "embossed"), Some(v.trim() == "engraved"));
    }
    fn tag(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|v| (1..=8).contains(&v.len()) && v.chars().all(|c| c.is_ascii_alphanumeric()))
    }
    if let Some(l) = tag(get(a, "fo:language")).filter(|l| *l != "none" && *l != "zxx") {
        x.lang = Some(match tag(get(a, "fo:country")).filter(|c| *c != "none") {
            Some(c) => format!("{l}-{c}"),
            None => l.to_string(),
        });
    }
}

#[derive(Clone, Debug, Default)]
struct StyleInfo {
    parent: Option<String>,
    display: Option<String>,
    auto: bool,
    fmt: FmtDelta,
    align: Option<Align>,
    page_break: bool,
    /// `fo:keep-with-next`: `Some(true)` for "always", `Some(false)` for "auto".
    keep_next: Option<bool>,
    outline: Option<u8>,
    border_bottom: bool,
    /// `fo:background-color` of a table-cell style's `style:table-cell-properties`.
    cell_background: Option<Rgb>,
    /// `style:vertical-align` of a table-cell style's `style:table-cell-properties`.
    cell_valign: Option<VAlign>,
    /// `fo:border*` of a table-cell style: each side as stated (`none` an explicit no line).
    cell_borders: Option<Borders>,
    /// `fo:padding*` of a table-cell style (top, left, bottom, right), each as stated.
    cell_padding: [Option<f32>; 4],
    /// `style:column-width` of a table-column style's `style:table-column-properties`, in points.
    column_width: Option<f32>,
    /// `style:master-page-name` of a paragraph or table style: its paragraphs (tables) start a
    /// page of that master page. Empty: explicitly none, which stops the lookup up the chain.
    master_page: Option<String>,
    /// `style:page-number` of a paragraph or table style (applied only with a master page): `Some(Some(n))` restarts page numbering at n,
    /// `Some(None)` ("auto") continues it.
    page_number: Option<Option<u32>>,
    /// `style:columns` of a section style.
    columns: Option<Columns>,
    /// Margins, first-line indent and line spacing of `style:paragraph-properties`.
    spacing: Spacing,
    /// `fo:border*` of a paragraph style, each side's padding as its distance from the text.
    para_borders: Option<Borders>,
    /// `fo:background-color` of a paragraph style (`Some(None)`: transparent, stated).
    para_shading: Option<Option<Rgb>>,
    /// `style:graphic-properties` of a graphic style: a frame's placement.
    graphic: Option<super::frames::Graphic>,
    /// `style:tab-stops`: the style's whole set (it replaces the parent's, §17.7).
    tabs: Option<Vec<TabStop>>,
}

/// Most tab stops a paragraph style keeps (Word's limit).
const MAX_TABS: usize = 64;

/// A `style:tab-stop` (ODF 1.3 §17.8), its position from where the style measures; `None` when
/// it has no usable position.
fn tab_stop(a: &Attrs) -> Option<TabStop> {
    let pos = get(a, "style:position").and_then(length).filter(|p| p.is_finite())?.clamp(-MAX_LEN, MAX_LEN);
    let align = match get(a, "style:type").map(str::trim) {
        Some("center") => TabAlign::Center,
        Some("right") => TabAlign::Right,
        Some("char") => TabAlign::Decimal,
        _ => TabAlign::Left,
    };
    let style = get(a, "style:leader-style").map(str::trim).unwrap_or("none");
    let leader = if style == "none" {
        TabLeader::None
    } else {
        match get(a, "style:leader-text").and_then(|t| t.chars().next()) {
            Some('.') => TabLeader::Dot,
            Some('-') => TabLeader::Hyphen,
            Some('_') => TabLeader::Underscore,
            Some('·') => TabLeader::MiddleDot,
            _ if style == "dotted" => TabLeader::Dot,
            _ if matches!(style, "dash" | "long-dash") => TabLeader::Hyphen,
            _ => TabLeader::Underscore,
        }
    };
    Some(TabStop { pos, align, leader })
}

/// Paragraph spacing, indents and line height of a `style:paragraph-properties` element (ODF 1.3
/// §20.204–209, §20.225, §20.317–318) over `sp`. Percentages of the parent's margins aren't
/// followed (left as they were); spacing is clamped to 0..=1584 pt, indents to ±1584 pt.
fn paragraph_spacing(a: &Attrs, sp: &mut Spacing) {
    let len = |k: &str| get(a, k).and_then(length);
    let space = |k: &str| len(k).map(|v| v.clamp(0.0, MAX_LEN));
    let indent = |k: &str| len(k).map(|v| v.clamp(-MAX_LEN, MAX_LEN));
    if let Some(v) = get(a, "fo:margin").and_then(length) {
        sp.before = Some(v.clamp(0.0, MAX_LEN));
        sp.after = sp.before;
        sp.left = Some(v.clamp(-MAX_LEN, MAX_LEN));
        sp.right = sp.left;
    }
    sp.before = space("fo:margin-top").or(sp.before);
    sp.after = space("fo:margin-bottom").or(sp.after);
    sp.left = indent("fo:margin-left").or(sp.left);
    sp.right = indent("fo:margin-right").or(sp.right);
    sp.first = indent("fo:text-indent").or(sp.first);
    if let Some(v) = get(a, "fo:line-height").map(str::trim) {
        if v.eq_ignore_ascii_case("normal") {
            sp.line = Some(LineSpacing::Multiple(1.0));
        } else if let Some(pc) = v.strip_suffix('%').and_then(|n| n.trim().parse::<f32>().ok()).filter(|n| n.is_finite() && *n > 0.0) {
            // Word's range: a sixteenth of a line up to 132 lines.
            sp.line = Some(LineSpacing::Multiple((pc / 100.0).clamp(0.06, 132.0)));
        } else if let Some(h) = length(v).filter(|h| *h > 0.0) {
            sp.line = Some(LineSpacing::Exactly(h.clamp(0.7, MAX_LEN)));
        }
    }
    if let Some(h) = len("style:line-height-at-least").filter(|h| *h >= 0.0) {
        sp.line = Some(LineSpacing::AtLeast(h.clamp(0.7, MAX_LEN)));
    }
    // Leading (extra space between lines) has no WordCraft mode: as a multiple of a 12 pt line.
    if let Some(l) = len("style:line-spacing").filter(|l| *l >= 0.0) {
        sp.line = Some(LineSpacing::Multiple(((LEADING_LINE + l.min(MAX_LEN)) / LEADING_LINE).clamp(0.06, 132.0)));
    }
}

/// Largest spacing or indent read, in points (22 inches, Word's page limit).
const MAX_LEN: f32 = 1584.0;

/// Height of the single line `style:line-spacing` leading is added to (12 pt text).
const LEADING_LINE: f32 = 14.0;

/// `over` laid on `base`: what `over` sets wins.
fn overlay_spacing(base: &mut Spacing, over: &Spacing) {
    base.before = over.before.or(base.before);
    base.after = over.after.or(base.after);
    base.line = over.line.or(base.line);
    base.left = over.left.or(base.left);
    base.right = over.right.or(base.right);
    base.first = over.first.or(base.first);
}

/// The paragraph kind a style name (lower case, `_20_` as spaces) stands for.
fn kind_named(n: &str) -> Option<Kind> {
    if let Some(d) = n.strip_prefix("heading ").and_then(|r| r.trim().parse::<u8>().ok()) {
        Some(Kind::Heading(d.clamp(1, 6)))
    } else {
        match n {
            "title" => Some(Kind::Title),
            "quotations" | "quote" => Some(Kind::Quote),
            "preformatted text" | "source text" | "code" => Some(Kind::Code),
            _ => None,
        }
    }
}

/// How a paragraph style looks ([`Styles::para`]).
#[derive(Clone, Debug, Default)]
pub(super) struct ParaLook {
    pub(super) kind: Kind,
    pub(super) align: Option<Align>,
    pub(super) page_break: bool,
    pub(super) rule: bool,
    pub(super) keep_next: bool,
    pub(super) fmt: Fmt,
    pub(super) spacing: Spacing,
    /// Tab stops, from where the document measures them.
    pub(super) tabs: Option<Vec<TabStop>>,
    /// Borders (when a side shows) and background.
    pub(super) borders: Option<Borders>,
    pub(super) shading: Option<Rgb>,
    /// The custom style (a common style other than Standard and the kinds' styles) the
    /// paragraph is in, by its ODF name; the formatting above is then only what the automatic
    /// styles over it add.
    pub(super) named: Option<String>,
}

/// Where a custom style's formatting comes from besides its own ([`NamedStyle::parent`]).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Parent {
    /// The document defaults (Standard, or no parent).
    Normal,
    /// The style of a paragraph kind.
    Kind(Kind),
    /// Another custom style, by ODF name.
    Named(String),
}

/// A custom paragraph style as a document style: its name, parent and own formatting.
#[derive(Clone, Debug)]
pub(super) struct NamedStyle {
    pub(super) name: String,
    pub(super) parent: Parent,
    pub(super) look: ParaLook,
}

/// Longest style id made, in characters.
const MAX_STYLE_ID: usize = 64;

/// The document style id for an ODF style: its name's letters and digits (LibreOffice's
/// `Text_20_body` is `Textbody`), at most [`MAX_STYLE_ID`] of them.
pub(super) fn style_id(odt: &str) -> String {
    let id: String = odt.replace("_20_", " ").chars().filter(|c| c.is_alphanumeric()).take(MAX_STYLE_ID).collect();
    if id.is_empty() { "Custom".into() } else { id }
}

/// An ODF border (`0.5pt solid #000000`, `none`; ODF 1.3 §20.183): width, style and colour in any
/// order. `none` and `hidden` are an explicit no line; `None` when nothing in it reads.
pub(super) fn parse_border(v: &str) -> Option<Border> {
    let mut b = Border { style: BorderStyle::Single, width: 0.75, color: None, space: 0.0 };
    let mut any = false;
    for t in v.split_whitespace() {
        let t = t.to_ascii_lowercase();
        match t.as_str() {
            "none" | "hidden" => return Some(Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 }),
            "solid" | "groove" | "ridge" | "inset" | "outset" => b.style = BorderStyle::Single,
            "double" => b.style = BorderStyle::Double,
            "dotted" => b.style = BorderStyle::Dotted,
            "dashed" => b.style = BorderStyle::Dashed,
            "dot-dash" | "dot-dot-dash" => b.style = BorderStyle::DotDash,
            "wave" => b.style = BorderStyle::Wave,
            "thin" => b.width = 0.5,
            "medium" => b.width = 1.0,
            "thick" => b.width = 2.0,
            _ => {
                if let Some(c) = model::parse_color(&t) {
                    b.color = Some(c);
                } else if let Some(w) = length(&t).filter(|w| w.is_finite()) {
                    b.width = w.clamp(0.0, 12.0);
                } else {
                    continue;
                }
            }
        }
        any = true;
    }
    any.then_some(b)
}

/// A property element `name` (`q` in full) of the style being read, into `s`: true when it is
/// one (paragraph, table, column, cell, section and graphic properties, tab stops).
fn style_child(name: &str, q: &str, a: &Attrs, s: &mut StyleInfo) -> bool {
    match name {
        "paragraph-properties" => paragraph_properties(a, s),
        "tab-stops" => s.tabs = Some(Vec::new()),
        "tab-stop" => {
            if let Some(t) = s.tabs.as_mut().filter(|t| t.len() < MAX_TABS)
                && let Some(stop) = tab_stop(a)
            {
                t.push(stop);
            }
        }
        "table-properties" => {
            if let Some(v) = get(a, "style:page-number") {
                s.page_number = Some(v.trim().parse::<u32>().ok().map(|n| n.min(32_767)));
            }
        }
        "table-column-properties" => {
            if let Some(w) = get(a, "style:column-width").and_then(length) {
                s.column_width = Some(w);
            }
        }
        "table-cell-properties" => table_cell_properties(a, s),
        "columns" if q.starts_with("style:") => s.columns = Some(columns_of(a)),
        "column-sep" => {
            if let Some(c) = &mut s.columns {
                c.separator = column_sep(a);
            }
        }
        "graphic-properties" => s.graphic = Some(super::frames::Graphic::read(a)),
        _ => return false,
    }
    true
}

/// A table-cell style's `style:table-cell-properties` (ODF 1.3 §17.18): background, borders,
/// padding and vertical alignment.
fn table_cell_properties(a: &Attrs, s: &mut StyleInfo) {
    if let Some(c) = get(a, "fo:background-color").and_then(model::parse_color) {
        s.cell_background = Some(c);
    }
    cell_borders_and_padding(a, s);
    if let Some(va) = get(a, "style:vertical-align") {
        s.cell_valign = match va.trim() {
            "top" => Some(VAlign::Top),
            "middle" | "center" => Some(VAlign::Center),
            "bottom" => Some(VAlign::Bottom),
            _ => None,
        };
    }
}

/// A table-cell style's `fo:border*` and `fo:padding*` (a side's own value wins over the
/// shorthand) into `s`.
fn cell_borders_and_padding(a: &Attrs, s: &mut StyleInfo) {
    let all = get(a, "fo:border").and_then(parse_border);
    let side = |k: &str| get(a, k).and_then(parse_border).or(all);
    let (top, left, bottom, right) = (side("fo:border-top"), side("fo:border-left"), side("fo:border-bottom"), side("fo:border-right"));
    if top.is_some() || left.is_some() || bottom.is_some() || right.is_some() {
        s.cell_borders = Some(Borders { top, left, bottom, right, ..Default::default() });
    }
    let pad = |k: &str| get(a, k).and_then(length).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, MAX_LEN));
    let every = pad("fo:padding");
    let sides = [pad("fo:padding-top"), pad("fo:padding-left"), pad("fo:padding-bottom"), pad("fo:padding-right")];
    if every.is_some() || sides.iter().any(Option::is_some) {
        s.cell_padding = sides.map(|v| v.or(every));
    }
}

/// Columns of a `style:columns` element (ODF 1.3 §17.12): count and gap, clamped as the RTF and
/// DOCX readers do; equal widths only (`style:column` widths are not read).
pub(super) fn columns_of(a: &Attrs) -> Columns {
    let mut c = Columns::default();
    if let Some(n) = get(a, "fo:column-count").and_then(|v| v.trim().parse::<u64>().ok()) {
        c.count = n.clamp(1, 45) as u32;
    }
    if let Some(g) = get(a, "fo:column-gap").and_then(length).filter(|v| *v >= 0.0) {
        c.space = g.min(1584.0);
    }
    c
}

/// A `style:column-sep` (ODF 1.3 §17.13) draws a line between columns unless its style is "none".
pub(super) fn column_sep(a: &Attrs) -> bool {
    get(a, "style:style") != Some("none")
}

/// The font family `a` names: `fo:font-family`, else the family of the font face `style:font-name`
/// names (ODF 1.3 §20.194, §16.23), else that name itself.
fn family_of<'a>(a: &'a Attrs, faces: &'a HashMap<String, String>) -> Option<&'a str> {
    get(a, "fo:font-family").or_else(|| get(a, "style:font-name").map(|n| faces.get(n).map_or(n, String::as_str)))
}

fn default_font(a: &Attrs, faces: &HashMap<String, String>) -> Option<String> {
    let family = family_of(a, faces)?;
    let family = family.split(',').next().unwrap_or("").trim().trim_matches(['\'', '"']);
    (!family.is_empty() && !model::is_mono(family)).then(|| family.to_string())
}

fn text_properties(a: &Attrs, style: &mut StyleInfo, faces: &HashMap<String, String>) {
    let fmt = &mut style.fmt;
    let is_bold = |weight: &str| {
        let weight = weight.trim();
        weight.eq_ignore_ascii_case("bold") || weight.parse::<u32>().is_ok_and(|n| n >= 600)
    };
    // Latin attributes take precedence; Asian and complex-script attributes are fallbacks.
    if let Some(weight) = get(a, "fo:font-weight").or_else(|| get(a, "style:font-weight-asian")).or_else(|| get(a, "style:font-weight-complex")) {
        fmt.bold = Some(is_bold(weight));
    }
    let is_italic = |value: &str| value.trim().eq_ignore_ascii_case("italic") || value.trim().eq_ignore_ascii_case("oblique");
    if let Some(value) = get(a, "fo:font-style").or_else(|| get(a, "style:font-style-asian")).or_else(|| get(a, "style:font-style-complex")) {
        fmt.italic = Some(is_italic(value));
    }
    let is_not_none = |value: &str| !value.trim().eq_ignore_ascii_case("none");
    if let Some(value) = get(a, "style:text-underline-style").or_else(|| get(a, "style:text-underline-type")) {
        fmt.underline = Some(is_not_none(value));
    }
    if let Some(value) = get(a, "style:text-line-through-style").or_else(|| get(a, "style:text-line-through-type")) {
        fmt.strike = Some(is_not_none(value));
    }
    if let Some(value) = get(a, "style:text-position") {
        // `super`/`sub` or a percentage raising the text, then its size (§20.374): raised text
        // at full size is a position, smaller text superscript or subscript.
        let mut parts = value.split_whitespace();
        let first = parts.next().unwrap_or("");
        let pct = first.trim_end_matches('%').parse::<f32>().ok().filter(|p| p.is_finite()).unwrap_or(0.0);
        let full = parts.next().and_then(|s| s.trim_end_matches('%').parse::<f32>().ok()).is_some_and(|s| s >= 100.0);
        let keyword = first.eq_ignore_ascii_case("super") || first.eq_ignore_ascii_case("sub");
        if full && !keyword {
            fmt.sup = Some(false);
            fmt.sub = Some(false);
            fmt.raise = Some(pct.clamp(-10_000.0, 10_000.0));
        } else {
            fmt.sup = Some(first.eq_ignore_ascii_case("super") || pct > 0.0);
            fmt.sub = Some(first.eq_ignore_ascii_case("sub") || pct < 0.0);
            fmt.raise = Some(0.0);
        }
    }
    if let Some(color) = get(a, "fo:color").and_then(model::parse_color) {
        fmt.color = Some(color);
    }
    if let Some(color) = get(a, "fo:background-color").and_then(model::parse_color) {
        fmt.background = Some(color);
    }
    if let Some(pct) = get(a, "fo:font-size")
        .and_then(|v| v.trim().strip_suffix('%'))
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|p| p.is_finite() && *p > 0.0)
    {
        fmt.size_pct = Some(pct.min(10_000.0));
    } else if let Some(size) = get(a, "fo:font-size").and_then(length).filter(|size| (1.0..=1638.0).contains(size)) {
        fmt.size = Some(size);
    }
    if let Some(family) = family_of(a, faces) {
        let family = family.split(',').next().unwrap_or("").trim().trim_matches(['\'', '"']).to_string();
        // A monospaced font keeps its name (it is not "code" here, which would be Courier New).
        if !family.is_empty() {
            fmt.font = model::kept_font(&family);
            fmt.code = Some(false);
        }
    }
    extra_text_properties(a, &mut style.fmt);
}

/// A paragraph style's `style:paragraph-properties` (ODF 1.3 §17.6) into `s`.
fn paragraph_properties(a: &Attrs, s: &mut StyleInfo) {
    s.align = match get(a, "fo:text-align").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("center") => Some(Align::Center),
        Some("end") | Some("right") => Some(Align::Right),
        Some("justify") => Some(Align::Justify),
        Some("start") | Some("left") => Some(Align::Left),
        _ => s.align,
    };
    if get(a, "fo:break-before").is_some_and(|b| b.trim().eq_ignore_ascii_case("page")) {
        s.page_break = true;
    }
    match get(a, "fo:keep-with-next") {
        Some("always") => s.keep_next = Some(true),
        Some("auto") => s.keep_next = Some(false),
        _ => {}
    }
    if let Some(v) = get(a, "style:page-number") {
        s.page_number = Some(v.trim().parse::<u32>().ok().map(|n| n.min(32_767)));
    }
    if get(a, "fo:border-bottom").is_some_and(|b| !b.trim().eq_ignore_ascii_case("none")) {
        s.border_bottom = true;
    }
    paragraph_spacing(a, &mut s.spacing);
    if let Some(b) = border_sides(a) {
        s.para_borders = Some(b);
    }
    if let Some(bg) = get(a, "fo:background-color") {
        s.para_shading = Some(if bg.trim() == "transparent" { None } else { model::parse_color(bg) });
    }
}

/// `fo:border`, `fo:border-top`… of `a` (a side's own value wins over the shorthand), each side's
/// `fo:padding*` as its distance from the text; `None` when none is stated.
fn border_sides(a: &Attrs) -> Option<Borders> {
    let all = get(a, "fo:border").and_then(parse_border);
    let pad_all = get(a, "fo:padding").and_then(length);
    let side = |b: &str, p: &str| {
        let mut line = get(a, b).and_then(parse_border).or(all)?;
        if let Some(sp) = get(a, p).and_then(length).or(pad_all).filter(|v| v.is_finite()) {
            line.space = sp.clamp(0.0, 31.0);
        }
        Some(line)
    };
    let b = Borders {
        top: side("fo:border-top", "fo:padding-top"),
        left: side("fo:border-left", "fo:padding-left"),
        bottom: side("fo:border-bottom", "fo:padding-bottom"),
        right: side("fo:border-right", "fo:padding-right"),
        ..Default::default()
    };
    (b.top.is_some() || b.left.is_some() || b.bottom.is_some() || b.right.is_some()).then_some(b)
}

/// A list style's level (ODF 1.3 §16.31–16.33): numbered or not, its label and start value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ListLevel {
    pub(super) ordered: bool,
    pub(super) label: Option<model::Label>,
    pub(super) start: u32,
}

/// ODF number format (`style:num-format`, ODF 1.3 §19.500) → WordCraft's; an empty format shows
/// no number.
fn num_format(f: &str) -> NumFormat {
    match f.trim() {
        "" => NumFormat::None,
        "i" => NumFormat::LowerRoman,
        "I" => NumFormat::UpperRoman,
        "a" => NumFormat::LowerLetter,
        "A" => NumFormat::UpperLetter,
        _ => NumFormat::Decimal,
    }
}

/// A level's `style:list-level-properties` (its label's alignment, `fo:text-align`) or
/// `style:list-level-label-alignment` (its indent, `fo:margin-left`, and the label's hanging,
/// `fo:text-indent`), ODF 1.3 §17.19–17.20, into `slot`'s label.
fn level_position(slot: &mut ListLevel, local: &str, a: &Attrs) {
    let Some(label) = slot.label.as_mut() else { return };
    if local == "list-level-properties" {
        label.align = match get(a, "fo:text-align").map(str::trim) {
            Some("end" | "right") => Some(Align::Right),
            Some("center") => Some(Align::Center),
            Some("start" | "left") => Some(Align::Left),
            _ => label.align,
        };
        return;
    }
    let len = |k: &str| get(a, k).and_then(length).filter(|v| v.is_finite());
    if let Some(m) = len("fo:margin-left") {
        label.indent = model::Label::twips(m.clamp(-MAX_LEN, MAX_LEN));
        label.hanging = model::Label::twips((-len("fo:text-indent").unwrap_or(0.0)).clamp(-MAX_LEN, MAX_LEN));
    }
}

/// A `text:list-level-style-number`/`-bullet`/`-image` element's level.
fn list_level(local: &str, a: &Attrs) -> ListLevel {
    let shown = get(a, "text:display-levels").and_then(|v| v.trim().parse::<u8>().ok()).unwrap_or(1).clamp(1, 9);
    let start = get(a, "text:start-value").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(1).clamp(0, 32_767);
    let affix = |k: &str| model::Affix::new(get(a, k).unwrap_or(""));
    match local {
        "list-level-style-number" => {
            let format = num_format(get(a, "style:num-format").unwrap_or(""));
            let ordered = format != NumFormat::None;
            let label = model::Label {
                format,
                prefix: affix("style:num-prefix"),
                suffix: affix("style:num-suffix"),
                shown,
                indent: None,
                hanging: None,
                align: None,
            };
            ListLevel { ordered, label: ordered.then_some(label), start }
        }
        _ => {
            // A picture bullet shows as the default bullet.
            let bullet = get(a, "text:bullet-char").and_then(|b| b.chars().find(|c| !c.is_control() && !c.is_whitespace()));
            ListLevel { ordered: false, label: Some(model::Label::bulleted(bullet)), start: 1 }
        }
    }
}

/// Most font faces kept from `office:font-face-decls` (hostile input).
const MAX_FACES: usize = 10_000;

#[derive(Default)]
pub(super) struct Styles {
    map: HashMap<String, StyleInfo>,
    /// Font faces (`style:font-face`): name → family.
    faces: HashMap<String, String>,
    /// The levels of `text:outline-style` (heading numbering, ODF 1.3 §16.34), when there is one.
    pub(super) outline: Option<Vec<ListLevel>>,
    pub(super) lists: HashMap<String, Vec<ListLevel>>,
    /// Font of `style:default-style` for paragraphs (the document default, ODF 1.3 §16.4).
    default_font: Option<String>,
    /// Spacing of `style:default-style` for paragraphs.
    default_spacing: Spacing,
    /// Tab stops are measured from the paragraph's indent (LibreOffice's `TabsRelativeToIndent`
    /// setting in `settings.xml`, on unless it says otherwise).
    pub(super) tabs_from_indent: bool,
    /// The default tab interval (`style:tab-stop-distance` of the default style), points.
    pub(super) default_tab: Option<f32>,
    /// Pair kerning of `style:default-style` for paragraphs (`style:letter-kerning`, §20.317;
    /// off unless it says so).
    pub(super) default_kern: bool,
}

impl Styles {
    pub(super) fn read(&mut self, xml: &[u8], auto_section_is_auto: bool) {
        let mut in_auto = false;
        let mut cur: Option<(String, StyleInfo)> = None;
        let mut in_default = false;
        let mut list: Option<(String, Vec<ListLevel>)> = None;
        // The list or outline level being read.
        let mut level_at: Option<usize> = None;
        let mut outline_at: Option<usize> = None;
        events(xml, |ev| match ev {
            Ev::Start(q, a) => match local(&q) {
                "automatic-styles" => in_auto = true,
                "style" if q.starts_with("style:") => {
                    if let Some(name) = get(&a, "style:name") {
                        let info = StyleInfo {
                            parent: get(&a, "style:parent-style-name").map(String::from),
                            display: get(&a, "style:display-name").map(String::from),
                            auto: in_auto && auto_section_is_auto,
                            outline: get(&a, "style:default-outline-level").and_then(|v| v.trim().parse::<u8>().ok()),
                            master_page: get(&a, "style:master-page-name").map(String::from),
                            ..Default::default()
                        };
                        cur = Some((name.to_string(), info));
                    }
                }
                "default-style" if q.starts_with("style:") => in_default = get(&a, "style:family") == Some("paragraph"),
                "text-properties" if in_default && cur.is_none() => {
                    self.default_font = default_font(&a, &self.faces);
                    self.default_kern = get(&a, "style:letter-kerning").is_some_and(|k| k.trim() == "true");
                }
                "paragraph-properties" if in_default && cur.is_none() => {
                    paragraph_spacing(&a, &mut self.default_spacing);
                    if let Some(d) = get(&a, "style:tab-stop-distance").and_then(length).filter(|d| d.is_finite() && *d > 0.0) {
                        self.default_tab = Some(d.clamp(1.0, MAX_LEN));
                    }
                }
                "text-properties" => {
                    if let Some((_, s)) = &mut cur {
                        text_properties(&a, s, &self.faces);
                    }
                }
                name if cur.as_mut().is_some_and(|(_, s)| style_child(name, &q, &a, s)) => {}
                "font-face" if q.starts_with("style:") && self.faces.len() < MAX_FACES => {
                    if let (Some(n), Some(f)) = (get(&a, "style:name"), get(&a, "svg:font-family")) {
                        let family = f.split(',').next().unwrap_or("").trim().trim_matches(['\'', '"']);
                        if !family.is_empty() && family.len() <= model::MAX_FONT_NAME {
                            self.faces.insert(n.to_string(), family.to_string());
                        }
                    }
                }
                "list-style" => list = get(&a, "style:name").map(|n| (n.to_string(), vec![ListLevel::default(); 10])),
                "outline-style" => self.outline = Some(vec![ListLevel::default(); 10]),
                "outline-level-style" => {
                    let lv = get(&a, "text:level").and_then(|l| l.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 10);
                    if let Some(slot) = self.outline.as_mut().and_then(|o| o.get_mut(lv - 1)) {
                        *slot = list_level("list-level-style-number", &a);
                    }
                    outline_at = Some(lv - 1);
                }
                "list-level-style-number" | "list-level-style-bullet" | "list-level-style-image" => {
                    if let Some((_, v)) = &mut list {
                        let lv = get(&a, "text:level").and_then(|l| l.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, 10);
                        if let Some(slot) = v.get_mut(lv - 1) {
                            *slot = list_level(local(&q), &a);
                        }
                        level_at = Some(lv - 1);
                    }
                }
                "list-level-properties" | "list-level-label-alignment" => {
                    let slot = match (&mut list, level_at) {
                        (Some((_, v)), Some(i)) => v.get_mut(i),
                        _ => self.outline.as_mut().zip(outline_at).and_then(|(o, i)| o.get_mut(i)),
                    };
                    if let Some(slot) = slot {
                        level_position(slot, local(&q), &a);
                    }
                }
                _ => {}
            },
            Ev::End(q) => match local(&q) {
                "automatic-styles" => in_auto = false,
                "default-style" if q.starts_with("style:") => in_default = false,
                "style" if q.starts_with("style:") => {
                    if let Some((n, s)) = cur.take()
                        && self.map.len() < 100_000
                    {
                        self.map.insert(n, s);
                    }
                }
                "list-style" => {
                    if let Some((n, v)) = list.take() {
                        self.lists.insert(n, v);
                    }
                }
                "list-level-style-number" | "list-level-style-bullet" | "list-level-style-image" => level_at = None,
                "outline-level-style" => outline_at = None,
                _ => {}
            },
            Ev::Text(_) => {}
        });
    }

    /// The document's default font: the Standard paragraph style's, else the default style's.
    pub(super) fn doc_font(&self) -> Option<String> {
        self.map.get("Standard").filter(|s| !s.auto).and_then(|s| s.fmt.font.as_deref().map(str::to_string)).or_else(|| self.default_font.clone())
    }

    /// The document's default paragraph spacing: the default style's, with the Standard
    /// paragraph style's over it, on ODF's initial values (no margins, single lines).
    pub(super) fn doc_spacing(&self) -> Spacing {
        let mut sp = Spacing {
            before: Some(0.0),
            after: Some(0.0),
            line: Some(LineSpacing::Multiple(1.0)),
            left: Some(0.0),
            right: Some(0.0),
            first: Some(0.0),
        };
        overlay_spacing(&mut sp, &self.default_spacing);
        if let Some(s) = self.map.get("Standard").filter(|s| !s.auto) {
            overlay_spacing(&mut sp, &s.spacing);
        }
        sp
    }

    fn chain<'a>(&'a self, name: &'a str) -> Vec<(&'a str, &'a StyleInfo)> {
        let mut out = Vec::new();
        let mut cur = Some(name);
        while let Some(n) = cur {
            if out.len() >= 16 {
                break;
            }
            let Some(s) = self.map.get(n) else { break };
            if out.iter().any(|(m, _): &(&str, &StyleInfo)| *m == n) {
                break;
            }
            out.push((n, s));
            cur = s.parent.as_deref();
        }
        out
    }

    /// Text formatting of a text style (whole chain).
    pub(super) fn text_fmt(&self, name: &str, base: &Fmt) -> Fmt {
        let mut f = base.clone();
        for (_, s) in self.chain(name).iter().rev() {
            s.fmt.apply(&mut f);
        }
        f
    }

    /// The page a paragraph style starts (`style:master-page-name` and `style:page-number`,
    /// ODF 1.3 §16.2 and §17.6): the nearest master page and page-number setting in its chain.
    pub(super) fn page_start<'a>(&'a self, name: &'a str) -> (Option<&'a str>, Option<Option<u32>>) {
        let chain = self.chain(name);
        let master = chain.iter().find_map(|(_, s)| s.master_page.as_deref()).filter(|m| !m.is_empty());
        (master, chain.iter().find_map(|(_, s)| s.page_number))
    }

    /// Columns of a section style (whole chain).
    pub(super) fn columns(&self, name: &str) -> Option<Columns> {
        self.chain(name).iter().find_map(|(_, s)| s.columns.clone())
    }

    /// Width in points of a table-column style (whole chain).
    pub(super) fn column_width(&self, name: &str) -> Option<f32> {
        self.chain(name).iter().find_map(|(_, s)| s.column_width)
    }

    /// Background colour of a table-cell style (whole chain).
    pub(super) fn cell_background(&self, name: &str) -> Option<Rgb> {
        self.chain(name).iter().find_map(|(_, s)| s.cell_background)
    }

    /// Borders of a table-cell style (whole chain).
    pub(super) fn cell_borders(&self, name: &str) -> Option<Borders> {
        self.chain(name).iter().find_map(|(_, s)| s.cell_borders)
    }

    /// Padding of a table-cell style (top, left, bottom, right; whole chain), when it gives all
    /// four sides; missing sides of a partial one are 0, as LibreOffice reads them.
    pub(super) fn cell_padding(&self, name: &str) -> Option<[f32; 4]> {
        let p = self.chain(name).iter().map(|(_, s)| s.cell_padding).find(|p| p.iter().any(Option::is_some))?;
        Some(p.map(|v| v.unwrap_or(0.0)))
    }

    /// A graphic style's frame placement (whole chain).
    pub(super) fn graphic(&self, name: &str) -> super::frames::Graphic {
        self.chain(name).iter().filter_map(|(_, s)| s.graphic.as_ref()).fold(super::frames::Graphic::default(), |g, p| g.or(p))
    }

    /// Vertical alignment of a table-cell style (whole chain).
    pub(super) fn cell_valign(&self, name: &str) -> Option<VAlign> {
        self.chain(name).iter().find_map(|(_, s)| s.cell_valign)
    }

    /// Paragraph kind, alignment, page break, rule, keep-with-next and base formatting. The text formatting of
    /// every style in the chain applies (automatic and common, ODF 1.3 §16.2 inheritance), except
    /// the style that gives the paragraph its kind and that style's ancestors: their formatting
    /// is the kind's own look (a heading's size and colour), not direct formatting. `Standard`,
    /// the default paragraph style, holds document defaults and is left out too.
    /// The look the file gives each style that maps to a paragraph kind (Heading 1–6, Title,
    /// Quotations, Preformatted Text): its text formatting, spacing, alignment and keep with
    /// next, over the whole chain but Standard (the document defaults). A style named for the
    /// kind wins over one only displayed so.
    pub(super) fn kind_looks(&self) -> Vec<(Kind, ParaLook)> {
        let mut found: Vec<(Kind, bool, &str)> = Vec::new();
        for (n, s) in self.map.iter().filter(|(_, s)| !s.auto) {
            let own = kind_named(&n.to_ascii_lowercase().replace("_20_", " "));
            let shown = s.display.as_deref().and_then(|d| kind_named(&d.to_ascii_lowercase()));
            let Some((kind, canonical)) = own.map(|k| (k, true)).or(shown.map(|k| (k, false))) else { continue };
            match found.iter_mut().find(|(k, ..)| *k == kind) {
                Some(slot) if !slot.1 && canonical => *slot = (kind, canonical, n),
                Some(slot) if slot.1 == canonical && n.as_str() < slot.2 => *slot = (kind, canonical, n),
                Some(_) => {}
                None => found.push((kind, canonical, n)),
            }
        }
        found
            .into_iter()
            .map(|(kind, _, name)| {
                let chain: Vec<_> = self.chain(name).into_iter().filter(|(n, _)| *n != "Standard").collect();
                let mut fmt = Fmt::default();
                let mut spacing = Spacing::default();
                for (_, s) in chain.iter().rev() {
                    s.fmt.apply(&mut fmt);
                    overlay_spacing(&mut spacing, &s.spacing);
                }
                let align = chain.iter().find_map(|(_, s)| s.align);
                let keep_next = chain.iter().find_map(|(_, s)| s.keep_next).unwrap_or(false);
                (kind, ParaLook { kind, align, keep_next, fmt, spacing, ..Default::default() })
            })
            .collect()
    }

    /// The custom style `odt` names (a common paragraph style other than Standard and the
    /// kinds' styles), with its parent and own formatting; `None` for any other style.
    /// Whether `odt` names a custom style ([`Self::named_style`]), without building it.
    pub(super) fn is_named(&self, odt: &str) -> bool {
        let Some(s) = self.map.get(odt).filter(|s| !s.auto && odt != "Standard") else { return false };
        let lower = |n: &str| n.to_ascii_lowercase().replace("_20_", " ");
        // The kinds' styles, and the note text styles (WordCraft's Footnote Text and Endnote
        // Text stand for them), are no custom styles.
        let special = |n: &str| kind_named(n).is_some() || matches!(n, "footnote" | "endnote");
        !(special(&lower(odt)) || s.display.as_deref().is_some_and(|d| special(&lower(d))))
    }

    pub(super) fn named_style(&self, odt: &str) -> Option<NamedStyle> {
        if !self.is_named(odt) {
            return None;
        }
        let s = self.map.get(odt)?;
        let lower = |n: &str| n.to_ascii_lowercase().replace("_20_", " ");
        let parent = match s.parent.as_deref() {
            None | Some("Standard") => Parent::Normal,
            Some(p) => match self.map.get(p) {
                None => Parent::Normal,
                Some(ps) => match kind_named(&lower(p)).or_else(|| ps.display.as_deref().and_then(|d| kind_named(&lower(d)))) {
                    Some(k) => Parent::Kind(k),
                    None if ps.auto => Parent::Normal,
                    None => Parent::Named(p.to_string()),
                },
            },
        };
        let mut fmt = Fmt::default();
        s.fmt.apply(&mut fmt);
        let look = ParaLook {
            align: s.align,
            keep_next: s.keep_next == Some(true),
            fmt,
            spacing: s.spacing,
            tabs: s.tabs.clone(),
            borders: s.para_borders.filter(|b| b.any_visible()),
            shading: s.para_shading.flatten(),
            ..Default::default()
        };
        let name = s.display.clone().unwrap_or_else(|| odt.replace("_20_", " "));
        Some(NamedStyle { name: name.chars().take(super::super::model::MAX_FONT_NAME).collect(), parent, look })
    }

    pub(super) fn para(&self, name: &str) -> ParaLook {
        let chain = self.chain(name);
        let mut kind = Kind::Normal;
        let mut kind_at = chain.len();
        for (i, (n, s)) in chain.iter().enumerate() {
            kind_at = i;
            let dn = s.display.as_deref().unwrap_or(n).to_ascii_lowercase().replace("_20_", " ");
            let nn = n.to_ascii_lowercase().replace("_20_", " ");
            for cand in [dn.as_str(), nn.as_str()] {
                if let Some(d) = cand.strip_prefix("heading ").and_then(|r| r.trim().parse::<u8>().ok()) {
                    kind = Kind::Heading(d.clamp(1, 6));
                } else if cand == "title" {
                    kind = Kind::Title;
                } else if cand == "quotations" || cand == "quote" {
                    kind = Kind::Quote;
                } else if cand == "preformatted text" || cand == "source text" || cand == "code" {
                    kind = Kind::Code;
                }
            }
            if kind != Kind::Normal {
                break;
            }
            if let Some(o) = s.outline.filter(|o| (1..=10).contains(o)) {
                kind = Kind::Heading(o.min(6));
                break;
            }
            kind_at = chain.len();
        }
        // The nearest custom style: its formatting stays in it; the automatic styles over it give
        // the paragraph's own.
        let named_at = chain.iter().take(kind_at).position(|(n, _)| self.is_named(n));
        let named = named_at.and_then(|i| chain.get(i)).map(|(n, _)| n.to_string());
        let kind_at = named_at.unwrap_or(kind_at);
        let align = if named.is_some() { chain.iter().take(kind_at).find_map(|(_, s)| s.align) } else { chain.iter().find_map(|(_, s)| s.align) };
        let pb = chain.iter().take_while(|(_, s)| s.auto).any(|(_, s)| s.page_break);
        let rule = chain.iter().any(|(_, s)| s.border_bottom);
        let mut f = Fmt::default();
        let mut spacing = Spacing::default();
        for (_, s) in chain.iter().take(kind_at).rev().filter(|(n, s)| s.auto || *n != "Standard") {
            s.fmt.apply(&mut f);
            overlay_spacing(&mut spacing, &s.spacing);
        }
        // Like the text formatting, the kind style's own `fo:keep-with-next` (a heading's) is the kind's look, not direct formatting.
        let keep_next = chain.iter().take(kind_at).find_map(|(_, s)| s.keep_next).unwrap_or(false);
        // A style's tab stops replace its parent's; the kind's style and Standard give none.
        let tabs = chain.iter().take(kind_at).filter(|(n, s)| s.auto || *n != "Standard").find_map(|(_, s)| s.tabs.clone());
        // Borders and background: the nearest in the chain, the kind's style and Standard aside.
        let own = || chain.iter().take(kind_at).filter(|(n, s)| s.auto || *n != "Standard");
        let borders = own().find_map(|(_, s)| s.para_borders).filter(|b| b.any_visible());
        let shading = own().find_map(|(_, s)| s.para_shading).flatten();
        ParaLook { kind, align, page_break: pb, rule, keep_next, fmt: f, spacing, tabs, borders, shading, named }
    }
}
