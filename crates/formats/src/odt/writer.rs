use super::{Align, FBlock, FTable, Fmt, HashMap, Inline, Kind, Para, RevMark, Rgb, VAlign, mime_of, model};
use wordcraft_doc::props::{Border, BorderStyle, Borders, LineSpacing};

// ---------------------------------------------------------------------------------------------
// Export

/// A list style level as written: numbered or not, its label and its start value.
#[derive(Clone, Copy, Default, PartialEq)]
struct ListOut {
    ordered: bool,
    label: Option<model::Label>,
    start: u32,
}

/// WordCraft number format → ODF's `style:num-format` (ODF 1.3 §19.500); formats ODF lacks are
/// written as decimal.
pub(super) fn odf_num_format(f: wordcraft_doc::section::NumFormat) -> &'static str {
    use wordcraft_doc::section::NumFormat as F;
    match f {
        F::LowerRoman => "i",
        F::UpperRoman => "I",
        F::LowerLetter => "a",
        F::UpperLetter => "A",
        F::None => "",
        _ => "1",
    }
}

pub(super) fn x(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            '\u{FFFE}' | '\u{FFFF}' => {}
            c if c.is_control() && !matches!(c, '\t' | '\n') => {}
            _ => o.push(c),
        }
    }
    o
}

pub(super) const NS: &str = concat!(
    "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" ",
    "xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" ",
    "xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" ",
    "xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" ",
    "xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" ",
    "xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" ",
    "xmlns:xlink=\"http://www.w3.org/1999/xlink\" ",
    "xmlns:dc=\"http://purl.org/dc/elements/1.1/\" ",
    "xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" ",
    "xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" ",
    "xmlns:loext=\"urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0\" ",
    "office:version=\"1.3\""
);

pub(super) fn pt(v: f32) -> String {
    let s = format!("{:.2}", v);
    format!("{}pt", s.trim_end_matches('0').trim_end_matches('.'))
}

/// A section that starts a page: the master page it uses and where page numbering restarts.
type SecStart = (String, Option<u32>);

#[derive(Default)]
pub(super) struct Writer {
    /// The custom paragraph styles paragraphs were written in (document style ids).
    pub(super) named: std::collections::BTreeSet<String>,
    /// The custom character styles text was written in.
    pub(super) named_text: std::collections::BTreeSet<String>,
    /// Text boxes written so far (their frames' names).
    boxes: usize,
    /// Notes written so far (their ids), and footnotes and endnotes among them (their numbers).
    notes: usize,
    footnotes: u32,
    endnotes: u32,
    /// Paragraph style that plain paragraphs get: the note style inside a note.
    note_parent: Option<&'static str>,
    /// Prefix of every name it makes (styles, lists, tables, pictures): headers and footers are
    /// written by a second writer whose styles go in `styles.xml`, apart from the body's.
    pub(super) prefix: &'static str,
    /// Automatic text styles: properties XML → name.
    text_styles: Vec<(String, String)>,
    /// Automatic paragraph styles: (parent, properties XML, master page) → name.
    para_styles: Vec<((String, String, String), String)>,
    /// List styles: each level's kind, label and start.
    list_styles: Vec<[ListOut; 9]>,
    /// The `xml:id` of the first list written for each flow list number, so a list that shows
    /// up again continues it.
    list_xml: HashMap<u32, String>,
    /// Table styles: (name, column widths).
    tables: Vec<Vec<f32>>,
    /// The master page (and page-number restart) each table in `tables` starts.
    table_starts: Vec<Option<SecStart>>,
    /// Automatic styles of the columned `text:section`s written.
    pub(super) section_styles: Vec<String>,
    /// The page a section starts, for the next top-level block.
    pub(super) pending_start: Option<SecStart>,
    /// The next top-level list continues the one that ended the previous section.
    pub(super) continue_list: bool,
    /// `xml:id` of the last top-level list written, for `text:continue-list`.
    last_list: Option<String>,
    /// The page the block being written starts.
    master: Option<SecStart>,
    /// Cell styles by (shading, vertical-alignment).
    cell_styles: Vec<String>,
    /// Graphic styles of floating pictures: their `style:graphic-properties` attributes.
    graphic_styles: Vec<String>,
    pub(super) pictures: Vec<(String, std::sync::Arc<Vec<u8>>, String)>,
    /// Formula sub-documents: their directory and their `content.xml`.
    pub(super) formulas: Vec<(String, String)>,
    /// `text:changed-region` elements written so far.
    pub(super) regions: Vec<String>,
    /// The insertion open in the body, and its region id.
    open_ins: Option<(RevMark, String)>,
    /// Style of the paragraph being written.
    cur_style: String,
    /// Styles of the paragraphs joined into the one being written (deleted paragraph marks),
    /// and which of them the text being written belongs to.
    chain: Vec<String>,
    part: usize,
}

/// `style:text-properties` attributes for what only [`Fmt::extra`] holds (underline and strike
/// are written with the rest); `size` is the text's size, which a raised position is a share of.
fn extra_props(x: &wordcraft_doc::props::CharProps, size: Option<f32>) -> String {
    let mut p = String::new();
    let on = |v: Option<bool>| v == Some(true);
    if on(x.caps) {
        p.push_str(" fo:text-transform=\"uppercase\"");
    }
    if on(x.small_caps) {
        p.push_str(" fo:font-variant=\"small-caps\"");
    }
    if on(x.hidden) {
        p.push_str(" text:display=\"none\"");
    }
    if let Some(s) = x.spacing.filter(|s| s.is_finite() && *s != 0.0) {
        p.push_str(&format!(" fo:letter-spacing=\"{}\"", pt(s)));
    }
    if let Some(s) = x.scale.filter(|s| s.is_finite() && *s > 0.0 && *s != 100.0) {
        p.push_str(&format!(" style:text-scale=\"{}%\"", s.round()));
    }
    if let Some(r) = x.position.filter(|r| r.is_finite() && *r != 0.0) {
        let size = size.filter(|s| s.is_finite() && *s >= 1.0).unwrap_or(12.0);
        let pct = r / size * 100.0;
        p.push_str(&format!(" style:text-position=\"{}% 100%\"", (pct * 100.0).round() / 100.0));
    }
    if on(x.outline) {
        p.push_str(" style:text-outline=\"true\"");
    }
    if on(x.shadow) {
        p.push_str(" fo:text-shadow=\"1pt 1pt\"");
    }
    if on(x.emboss) {
        p.push_str(" style:font-relief=\"embossed\"");
    } else if on(x.engrave) {
        p.push_str(" style:font-relief=\"engraved\"");
    }
    p.push_str(&script_attrs(x));
    if let Some(l) = x.lang.as_deref() {
        let mut parts = l.split(['-', '_']);
        let ok = |s: &&str| (1..=8).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric());
        if let Some(lang) = parts.next().filter(ok) {
            p.push_str(&format!(" fo:language=\"{lang}\""));
            if let Some(country) = parts.next().filter(ok) {
                p.push_str(&format!(" fo:country=\"{country}\""));
            }
        }
    }
    p
}

/// `style:text-properties` attributes for character properties `cp` (a style's look).
pub(super) fn props_text(cp: &wordcraft_doc::props::CharProps) -> String {
    use wordcraft_doc::props::{TextColor, Underline, VertAlign};
    let f = Fmt {
        bold: cp.bold == Some(true),
        italic: cp.italic == Some(true),
        underline: cp.underline.is_some_and(|u| u != Underline::None),
        strike: cp.strike == Some(true),
        sup: cp.vert_align == Some(VertAlign::Superscript),
        sub: cp.vert_align == Some(VertAlign::Subscript),
        color: match cp.color {
            Some(TextColor::Rgb(c)) => Some(c),
            _ => None,
        },
        size: cp.size,
        font: cp.font.as_deref().map(std::sync::Arc::from),
        extra: Some(Box::new(cp.clone())),
        ..Default::default()
    };
    text_props(&f)
}

/// `fo:text-align` for `a` (ODF 1.3 §20.223).
pub(super) fn align_attr(a: Option<Align>) -> &'static str {
    match a {
        Some(Align::Center) => " fo:text-align=\"center\"",
        Some(Align::Right) => " fo:text-align=\"end\"",
        Some(Align::Justify) | Some(Align::Distribute) => " fo:text-align=\"justify\"",
        Some(Align::Left) => " fo:text-align=\"start\"",
        None => "",
    }
}

/// `fo:language`/`fo:country` (with `suffix`: `-asian`, `-complex` on `style:`) for BCP 47 tag
/// `tag`.
fn lang_attrs(tag: &str, suffix: &str) -> String {
    let mut parts = tag.split(['-', '_']);
    let ok = |s: &&str| (1..=8).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric());
    let ns = if suffix.is_empty() { "fo" } else { "style" };
    let mut p = String::new();
    if let Some(lang) = parts.next().filter(ok) {
        p.push_str(&format!(" {ns}:language{suffix}=\"{lang}\""));
        if let Some(country) = parts.next().filter(ok) {
            p.push_str(&format!(" {ns}:country{suffix}=\"{country}\""));
        }
    }
    p
}

/// The Asian and complex-script attributes of `x`: fonts, complex size, weight, posture and the
/// two languages.
pub(super) fn script_attrs(x: &wordcraft_doc::props::CharProps) -> String {
    let mut p = String::new();
    if let Some(f) = x.font_ea.as_deref() {
        p.push_str(&format!(" style:font-family-asian=\"'{}'\"", super::writer::x(&f.replace('\'', ""))));
    }
    if let Some(f) = x.font_cs.as_deref() {
        p.push_str(&format!(" style:font-family-complex=\"'{}'\"", super::writer::x(&f.replace('\'', ""))));
    }
    if let Some(s) = x.size_cs.filter(|s| s.is_finite()) {
        p.push_str(&format!(" style:font-size-complex=\"{}\"", pt(s)));
    }
    if let Some(b) = x.bold_cs {
        p.push_str(if b { " style:font-weight-complex=\"bold\"" } else { " style:font-weight-complex=\"normal\"" });
    }
    if let Some(i) = x.italic_cs {
        p.push_str(if i { " style:font-style-complex=\"italic\"" } else { " style:font-style-complex=\"normal\"" });
    }
    if let Some(l) = x.lang_ea.as_deref() {
        p.push_str(&lang_attrs(l, "-asian"));
    }
    if let Some(l) = x.lang_bidi.as_deref() {
        p.push_str(&lang_attrs(l, "-complex"));
    }
    p
}

fn text_props(f: &Fmt) -> String {
    let mut p = String::new();
    // Complex-script weight and posture of their own are written with the other script extras.
    let own_cs = |pick: fn(&wordcraft_doc::props::CharProps) -> Option<bool>| f.extra.as_deref().and_then(pick).is_some();
    if f.bold {
        p.push_str(" fo:font-weight=\"bold\" style:font-weight-asian=\"bold\"");
        if !own_cs(|x| x.bold_cs) {
            p.push_str(" style:font-weight-complex=\"bold\"");
        }
    }
    if f.italic {
        p.push_str(" fo:font-style=\"italic\" style:font-style-asian=\"italic\"");
        if !own_cs(|x| x.italic_cs) {
            p.push_str(" style:font-style-complex=\"italic\"");
        }
    }
    let extra = f.extra.as_deref();
    if f.underline {
        use wordcraft_doc::props::Underline as U;
        let (style, kind, width, mode) = match extra.and_then(|e| e.underline) {
            Some(U::Double) => ("solid", "double", "auto", ""),
            Some(U::Thick) => ("solid", "single", "bold", ""),
            Some(U::Dotted) => ("dotted", "single", "auto", ""),
            Some(U::Dash) => ("dash", "single", "auto", ""),
            Some(U::DotDash) => ("dot-dash", "single", "auto", ""),
            Some(U::DotDotDash) => ("dot-dot-dash", "single", "auto", ""),
            Some(U::Wave) => ("wave", "single", "auto", ""),
            Some(U::DoubleWave) => ("wave", "double", "auto", ""),
            Some(U::Words) => ("solid", "single", "auto", " style:text-underline-mode=\"skip-white-space\""),
            _ => ("solid", "single", "auto", ""),
        };
        let color =
            extra.and_then(|e| e.underline_color).map(|c| format!("#{}", c.hex().to_ascii_lowercase())).unwrap_or_else(|| "font-color".into());
        p.push_str(&format!(
            " style:text-underline-style=\"{style}\" style:text-underline-type=\"{kind}\" style:text-underline-width=\"{width}\" style:text-underline-color=\"{color}\"{mode}"
        ));
    }
    if f.strike || extra.is_some_and(|e| e.double_strike == Some(true)) {
        p.push_str(" style:text-line-through-style=\"solid\"");
        if extra.is_some_and(|e| e.double_strike == Some(true)) {
            p.push_str(" style:text-line-through-type=\"double\"");
        }
    }
    if let Some(e) = extra {
        p.push_str(&extra_props(e, f.size));
    }
    if f.sup {
        p.push_str(" style:text-position=\"super 58%\"");
    } else if f.sub {
        p.push_str(" style:text-position=\"sub 58%\"");
    }
    if let Some(c) = f.color {
        p.push_str(&format!(" fo:color=\"#{}\"", c.hex().to_ascii_lowercase()));
    }
    if let Some(c) = f.background {
        p.push_str(&format!(" fo:background-color=\"#{}\"", c.hex().to_ascii_lowercase()));
    }
    if let Some(s) = f.size {
        p.push_str(&format!(" fo:font-size=\"{}\"", pt(s)));
    }
    if f.code {
        p.push_str(&format!(" fo:font-family=\"'{}'\" style:font-family-generic=\"modern\" style:font-pitch=\"fixed\"", model::MONO_FONT));
    } else if let Some(fam) = &f.font {
        p.push_str(&format!(" fo:font-family=\"'{}'\"", x(&fam.replace('\'', ""))));
    }
    p
}

/// `style:paragraph-properties` attributes for the spacing, indents and line spacing `sp` sets
/// (ODF 1.3 §20.204–209, §20.225, §20.317).
pub(super) fn spacing_props(sp: &model::Spacing) -> String {
    let mut p = String::new();
    for (k, v) in [
        ("fo:margin-top", sp.before),
        ("fo:margin-bottom", sp.after),
        ("fo:margin-left", sp.left),
        ("fo:margin-right", sp.right),
        ("fo:text-indent", sp.first),
    ] {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            p.push_str(&format!(" {k}=\"{}\"", pt(v)));
        }
    }
    match sp.line {
        Some(LineSpacing::Multiple(m)) if m.is_finite() => p.push_str(&format!(" fo:line-height=\"{}%\"", (m * 100.0).round().max(1.0))),
        Some(LineSpacing::Exactly(h)) if h.is_finite() => p.push_str(&format!(" fo:line-height=\"{}\"", pt(h))),
        Some(LineSpacing::AtLeast(h)) if h.is_finite() => p.push_str(&format!(" style:line-height-at-least=\"{}\"", pt(h))),
        _ => {}
    }
    p
}

/// The `office:name` of comment `id` written by a writer with names prefixed `prefix`.
fn annotation_name(prefix: &str, id: u32) -> String {
    format!("__Annotation__{prefix}{id}")
}

/// `style:tab-stops` (ODF 1.3 §17.7) for `tabs`, measured from the margin (the package's
/// settings say so); bar and clearing stops have no ODF form and are left out.
pub(super) fn tab_stops(tabs: &[wordcraft_doc::props::TabStop]) -> String {
    use wordcraft_doc::props::{TabAlign, TabLeader};
    let mut s = String::from("<style:tab-stops>");
    for t in tabs.iter().filter(|t| t.pos.is_finite()) {
        let kind = match t.align {
            TabAlign::Left => "",
            TabAlign::Center => " style:type=\"center\"",
            TabAlign::Right => " style:type=\"right\"",
            TabAlign::Decimal => " style:type=\"char\" style:char=\".\"",
            TabAlign::Bar | TabAlign::Clear => continue,
        };
        let leader = match t.leader {
            TabLeader::None => "",
            TabLeader::Dot => " style:leader-style=\"dotted\" style:leader-text=\".\"",
            TabLeader::Hyphen => " style:leader-style=\"solid\" style:leader-text=\"-\"",
            TabLeader::Underscore => " style:leader-style=\"solid\" style:leader-text=\"_\"",
            TabLeader::MiddleDot => " style:leader-style=\"dotted\" style:leader-text=\"·\"",
        };
        s.push_str(&format!("<style:tab-stop style:position=\"{}\"{kind}{leader}/>", pt(t.pos)));
    }
    s.push_str("</style:tab-stops>");
    s
}

/// An ODF style name (an XML NCName, ODF 1.3 §19.75) for document style id `id`: letters,
/// digits and `-`, `.` kept, anything else as `_xHH_` (its code point in hex), and a leading
/// digit, `-` or `.` escaped too.
pub(super) fn odf_name(id: &str) -> String {
    let mut out = String::new();
    for (i, c) in id.chars().enumerate() {
        let ok = c.is_alphanumeric() || (i > 0 && (c == '-' || c == '.' || c.is_ascii_digit()));
        if ok && !(i == 0 && c.is_ascii_digit()) {
            out.push(c);
        } else {
            out.push_str(&format!("_x{:X}_", c as u32));
        }
    }
    if out.is_empty() { "Custom".into() } else { out }
}

/// `fo:border-*` and `fo:padding-*` attributes for paragraph borders `b`, when a side shows.
pub(super) fn border_attrs(b: Option<Borders>) -> String {
    let mut p = String::new();
    let Some(b) = b.filter(|b| b.any_visible()) else { return p };
    for (side, line) in [("top", b.top), ("left", b.left), ("bottom", b.bottom), ("right", b.right)] {
        let shown = line.filter(|l| l.is_visible());
        p.push_str(&format!(" fo:border-{side}=\"{}\"", odf_border(shown)));
        if let Some(l) = shown {
            p.push_str(&format!(" fo:padding-{side}=\"{}\"", pt(l.space.clamp(0.0, 31.0))));
        }
    }
    p
}

/// Word's default cell margins (top, left, bottom, right), points.
const WORD_CELL_MARGINS: [f32; 4] = [0.0, 5.4, 0.0, 5.4];

/// The cell of `row` that starts at grid column `g`.
fn cell_starting_at(row: &[model::Cell], g: usize) -> Option<&model::Cell> {
    let mut s = 0usize;
    row.iter().find(|x| {
        let here = s == g;
        s += x.colspan.clamp(1, 63) as usize;
        here
    })
}

/// A text box frame's graphic properties for `look`: its line (§20.183 `fo:border`), padding
/// (§20.210 `fo:padding-*`) and fill, both ways (LibreOffice reads `draw:fill`, and fills a
/// frame without it).
fn box_look_attrs(look: &model::BoxLook) -> String {
    let hex = |c: Rgb| format!("#{}", c.hex().to_ascii_lowercase());
    let mut a = match look.line {
        Some((c, w)) if w.is_finite() && w > 0.0 => format!(" fo:border=\"{} solid {}\"", pt(w.min(12.0)), hex(c)),
        _ => " fo:border=\"none\"".to_string(),
    };
    let [l, t, r, b] = look.inset.unwrap_or(DEFAULT_BOX_INSET).map(|v| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 });
    a.push_str(&format!(
        " fo:padding-left=\"{}\" fo:padding-top=\"{}\" fo:padding-right=\"{}\" fo:padding-bottom=\"{}\"",
        pt(l),
        pt(t),
        pt(r),
        pt(b)
    ));
    match look.fill {
        Some(c) => a.push_str(&format!(" fo:background-color=\"{0}\" draw:fill=\"solid\" draw:fill-color=\"{0}\"", hex(c))),
        None => a.push_str(" fo:background-color=\"transparent\" draw:fill=\"none\""),
    }
    a
}

/// A text box's default inner margins (left, top, right, bottom), points: WordCraft's and
/// Word's (0.1" and 0.05").
const DEFAULT_BOX_INSET: [f32; 4] = [7.2, 3.6, 7.2, 3.6];

/// An ODF border value (ODF 1.3 §20.183) for `b`; `none` for no line.
fn odf_border(b: Option<Border>) -> String {
    let Some(b) = b.filter(|b| b.is_visible() && b.width.is_finite()) else { return "none".into() };
    let style = match b.style {
        BorderStyle::Double | BorderStyle::Triple => "double",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Dashed => "dashed",
        BorderStyle::DotDash => "dot-dash",
        _ => "solid",
    };
    let color = b.color.unwrap_or(Rgb(0, 0, 0)).hex().to_ascii_lowercase();
    format!("{} {style} #{color}", pt(b.width.clamp(0.05, 12.0)))
}

/// The `style:table-cell-properties` attributes of cell `c` of row `ri` of `t`, starting at grid
/// column `g0` of `cols`: its padding, shading, vertical alignment and borders. Each edge two
/// cells share is written once, as LibreOffice writes it (every cell its left and bottom, the
/// first row its top, the last column its right), so lines don't double; a side the cell
/// doesn't state gets the table's grid line, or none in a table without lines.
fn cell_props(t: &FTable, ri: usize, g0: usize, c: &model::Cell, cols: usize) -> String {
    let grid = (!t.borderless).then_some(Border { style: BorderStyle::Single, width: 0.5, color: Some(Rgb(0, 0, 0)), space: 0.0 });
    let line = |c: &model::Cell, pick: fn(&Borders) -> Option<Border>| match c.borders.as_ref().and_then(pick) {
        Some(b) => Some(b).filter(|b| b.is_visible()),
        None => grid,
    };
    let row = t.rows.get(ri).map(Vec::as_slice).unwrap_or(&[]);
    let left_cell = g0.checked_sub(1).and_then(|g| {
        let mut s = 0usize;
        row.iter().find(|x| {
            let end = s + x.colspan.clamp(1, 63) as usize;
            let hit = s <= g && g < end;
            s = end;
            hit
        })
    });
    let below = t.rows.get(ri + 1).and_then(|r| cell_starting_at(r, g0));
    let left = line(c, |b| b.left).or_else(|| left_cell.and_then(|l| line(l, |b| b.right)));
    let bottom = line(c, |b| b.bottom).or_else(|| below.and_then(|b| line(b, |x| x.top)));
    let top = if ri == 0 { line(c, |b| b.top) } else { None };
    let right = if g0 + c.colspan.clamp(1, 63) as usize >= cols { line(c, |b| b.right) } else { None };
    let m = c.margins.filter(|m| m.iter().all(|v| v.is_finite())).unwrap_or(WORD_CELL_MARGINS);
    let mut p = format!(
        " fo:padding-top=\"{}\" fo:padding-left=\"{}\" fo:padding-bottom=\"{}\" fo:padding-right=\"{}\"",
        pt(m[0].max(0.0)),
        pt(m[1].max(0.0)),
        pt(m[2].max(0.0)),
        pt(m[3].max(0.0))
    );
    p.push_str(&format!(
        " fo:border-left=\"{}\" fo:border-right=\"{}\" fo:border-top=\"{}\" fo:border-bottom=\"{}\"",
        odf_border(left),
        odf_border(right),
        odf_border(top),
        odf_border(bottom)
    ));
    if let Some(bg) = c.shading {
        p.push_str(&format!(" fo:background-color=\"#{}\"", bg.hex().to_ascii_lowercase()));
    }
    p.push_str(match c.valign {
        Some(VAlign::Top) => " style:vertical-align=\"top\"",
        Some(VAlign::Center) => " style:vertical-align=\"middle\"",
        Some(VAlign::Bottom) => " style:vertical-align=\"bottom\"",
        None => "",
    });
    p
}

fn parent_style(k: Kind) -> &'static str {
    match k {
        Kind::Heading(1) => "Heading_20_1",
        Kind::Heading(2) => "Heading_20_2",
        Kind::Heading(3) => "Heading_20_3",
        Kind::Heading(4) => "Heading_20_4",
        Kind::Heading(5) => "Heading_20_5",
        Kind::Heading(_) => "Heading_20_6",
        Kind::Title => "Title",
        Kind::Quote => "Quotations",
        Kind::Code => "Preformatted_20_Text",
        Kind::Normal | Kind::Rule => "Standard",
    }
}

impl Writer {
    /// A writer whose names all start with `prefix`.
    pub(super) fn with_prefix(prefix: &'static str) -> Self {
        Writer { prefix, ..Default::default() }
    }

    fn text_style(&mut self, f: &Fmt) -> Option<String> {
        let mut props = text_props(f);
        // A custom character style: the automatic style's parent (after a separator in the key).
        if let Some(id) = f.extra.as_ref().and_then(|x| x.style.as_deref()) {
            self.named_text.insert(id.to_string());
            props.push('\u{1}');
            props.push_str(&odf_name(id));
        }
        if props.is_empty() {
            return None;
        }
        if let Some((_, n)) = self.text_styles.iter().find(|(p, _)| *p == props) {
            return Some(n.clone());
        }
        let n = format!("{}T{}", self.prefix, self.text_styles.len() + 1);
        self.text_styles.push((props, n.clone()));
        Some(n)
    }

    fn para_style(&mut self, p: &Para) -> String {
        self.para_style_at(p, None)
    }

    /// The paragraph style of `p`; `start` makes it begin a page of that master page (ODF 1.3
    /// §19.471 `style:master-page-name`), restarting page numbering where asked (§20.322).
    fn para_style_at(&mut self, p: &Para, start: Option<&SecStart>) -> String {
        let named = p.extra.as_ref().and_then(|x| x.style.as_deref()).filter(|_| matches!(p.kind, Kind::Normal | Kind::Rule));
        let parent = match (p.kind, self.note_parent, named) {
            // A custom style: written as a common style of the same name (see `custom_styles`).
            (_, _, Some(id)) => {
                self.named.insert(id.to_string());
                odf_name(id)
            }
            (Kind::Normal, Some(n), None) => n.to_string(),
            _ => parent_style(p.kind).to_string(),
        };
        let mut props = String::new();
        match p.align {
            Some(Align::Center) => props.push_str(" fo:text-align=\"center\""),
            Some(Align::Right) => props.push_str(" fo:text-align=\"end\""),
            Some(Align::Justify) | Some(Align::Distribute) => props.push_str(" fo:text-align=\"justify\""),
            Some(Align::Left) => props.push_str(" fo:text-align=\"start\""),
            None => {}
        }
        // The master page starts the page itself: no page break besides.
        if p.page_break && start.is_none() {
            props.push_str(" fo:break-before=\"page\"");
        }
        if let Some(n) = start.and_then(|(_, n)| *n) {
            props.push_str(&format!(" style:page-number=\"{n}\""));
        }

        if p.keep_next {
            props.push_str(" fo:keep-with-next=\"always\"");
        }
        props.push_str(&spacing_props(&p.spacing));
        // Children of the paragraph properties go after a separator in the key.
        if let Some(tabs) = p.extra.as_ref().and_then(|x| x.tabs.as_ref()) {
            props.push('\u{1}');
            props.push_str(&tab_stops(tabs));
        }
        if p.kind == Kind::Rule {
            props.push_str(" fo:border-bottom=\"0.75pt solid #a0a0a0\" fo:padding-bottom=\"1pt\"");
        } else {
            props.push_str(&border_attrs(p.borders));
        }
        if let Some(bg) = p.extra.as_ref().and_then(|x| x.shading) {
            props.push_str(&format!(" fo:background-color=\"#{}\"", bg.hex().to_ascii_lowercase()));
        }
        if let Some(c) = p.extra.as_ref().and_then(|x| x.contextual_spacing) {
            props.push_str(&format!(" style:contextual-spacing=\"{c}\""));
        }
        if p.extra.as_ref().and_then(|x| x.bidi) == Some(true) {
            props.push_str(" style:writing-mode=\"rl-tb\"");
        }
        let master = start.map(|(m, _)| m.clone()).unwrap_or_default();
        if props.is_empty() && master.is_empty() {
            return parent;
        }
        let key = (parent, props, master);
        if let Some((_, n)) = self.para_styles.iter().find(|(k, _)| *k == key) {
            return n.clone();
        }
        let n = format!("{}P{}", self.prefix, self.para_styles.len() + 1);
        self.para_styles.push((key, n.clone()));
        n
    }

    fn picture(&mut self, img: &model::Img) -> String {
        let ext = if img.ext == "jpeg" { "jpg".to_string() } else { img.ext.clone() };
        let name = format!("Pictures/{}image{}.{}", self.prefix, self.pictures.len() + 1, ext);
        self.pictures.push((name.clone(), img.data.clone(), mime_of(&img.ext).to_string()));
        name
    }

    /// A new `text:changed-region` of `kind` by `m` with `body` after its change info; its id.
    fn region(&mut self, kind: &str, m: &RevMark, body: &str) -> String {
        let id = format!("ct{}", self.regions.len() + 1);
        // `dc:date` is required: a change without a date is dated now.
        let date = if m.date.trim().is_empty() { now_iso() } else { m.date.clone() };
        self.regions.push(format!(
            "<text:changed-region text:id=\"{id}\"><text:{kind}><office:change-info><dc:creator>{}</dc:creator><dc:date>{}</dc:date></office:change-info>{body}</text:{kind}></text:changed-region>",
            x(&m.author),
            x(&date)
        ));
        id
    }

    /// Make `m` the insertion open in `st` (closing another one first).
    fn ins_open(&mut self, st: &mut Option<(RevMark, String)>, m: &RevMark, out: &mut String) {
        if st.as_ref().is_some_and(|(o, _)| o == m) {
            return;
        }
        ins_close(st, out);
        let ct = self.region("insertion", m, "");
        out.push_str(&format!("<text:change-start text:change-id=\"{ct}\"/>"));
        *st = Some((m.clone(), ct));
    }

    fn open_ins(&mut self, m: &RevMark, out: &mut String) {
        let mut st = self.open_ins.take();
        self.ins_open(&mut st, m, out);
        self.open_ins = st;
    }

    pub(super) fn close_ins(&mut self, out: &mut String) {
        ins_close(&mut self.open_ins, out);
    }

    /// Inlines with their insertions as change ranges kept in `st`.
    fn ins_inlines(&mut self, items: &[Item<'_>], st: &mut Option<(RevMark, String)>, out: &mut String) {
        let mut k = 0;
        while k < items.len() {
            let ins = items.get(k).and_then(|i| i.marks().0);
            let mut j = k + 1;
            while j < items.len() && items.get(j).and_then(|i| i.marks().0) == ins {
                j += 1;
            }
            match ins {
                Some(m) => self.ins_open(st, m, out),
                None => ins_close(st, out),
            }
            let seg: Vec<Inline> = items.get(k..j).unwrap_or(&[]).iter().filter_map(|i| i.inline().cloned()).collect();
            self.plain_inlines(&seg, out);
            k = j;
        }
    }

    /// Deleted content: a region holding it (one paragraph per deleted paragraph mark plus
    /// one; text inserted before it was deleted as change ranges inside) and a `text:change`
    /// point in the body.
    fn deletion(&mut self, seg: &[Item<'_>], m: &RevMark, out: &mut String) {
        let mut body = String::new();
        let mut st: Option<(RevMark, String)> = None;
        let mut start = 0;
        let mut first = true;
        loop {
            let end = seg.iter().skip(start).position(|i| matches!(i, Item::Mark(_))).map(|e| start + e);
            if !first {
                self.part += 1;
            }
            first = false;
            let style = self.chain.get(self.part).cloned().unwrap_or_else(|| self.cur_style.clone());
            body.push_str(&format!("<text:p text:style-name=\"{style}\">"));
            self.ins_inlines(seg.get(start..end.unwrap_or(seg.len())).unwrap_or(&[]), &mut st, &mut body);
            // A deleted paragraph mark that had been inserted: the range runs on past it.
            match end.and_then(|e| seg.get(e)).and_then(|i| i.marks().0) {
                Some(ins) => self.ins_open(&mut st, ins, &mut body),
                None => ins_close(&mut st, &mut body),
            }
            body.push_str("</text:p>");
            match end {
                Some(e) => start = e + 1,
                None => break,
            }
        }
        let ct = self.region("deletion", m, &body);
        out.push_str(&format!("<text:change text:change-id=\"{ct}\"/>"));
    }

    /// Paragraph content, with tracked insertions as change ranges and deletions as change points.
    fn items(&mut self, items: &[Item<'_>], out: &mut String) {
        let mut k = 0;
        while k < items.len() {
            let key = items.get(k).map(Item::segment).unwrap_or_default();
            let mut j = k + 1;
            while j < items.len() && items.get(j).map(Item::segment).unwrap_or_default() == key {
                j += 1;
            }
            let seg = items.get(k..j).unwrap_or(&[]);
            match key {
                (_, Some(del)) => self.deletion(seg, del, out),
                (Some(ins), None) => {
                    self.open_ins(ins, out);
                    let inl: Vec<Inline> = seg.iter().filter_map(|i| i.inline().cloned()).collect();
                    self.plain_inlines(&inl, out);
                }
                (None, None) => {
                    self.close_ins(out);
                    let inl: Vec<Inline> = seg.iter().filter_map(|i| i.inline().cloned()).collect();
                    self.plain_inlines(&inl, out);
                }
            }
            k = j;
        }
    }

    fn plain_inlines(&mut self, inl: &[Inline], out: &mut String) {
        let mut prev_space = true;
        // ODF has no ruby: its base text goes through the text path below.
        let inl = model::ruby_as_text(inl);
        let last = inl.len().saturating_sub(1);
        for (idx, i) in inl.iter().enumerate() {
            match i {
                Inline::Text(t, f) => {
                    let mut body = String::new();
                    let mut spaces = 0usize;
                    let flush = |body: &mut String, spaces: &mut usize, prev_space: bool| {
                        if *spaces == 0 {
                            return;
                        }
                        let (lit, extra) = if prev_space { (0, *spaces) } else { (1, *spaces - 1) };
                        if lit == 1 {
                            body.push(' ');
                        }
                        if extra > 0 {
                            body.push_str(&format!("<text:s text:c=\"{extra}\"/>"));
                        }
                        *spaces = 0;
                    };
                    for c in t.chars() {
                        if c == ' ' {
                            spaces += 1;
                            continue;
                        }
                        flush(&mut body, &mut spaces, prev_space);
                        prev_space = false;
                        match c {
                            '\t' => {
                                // The reader collapses a literal space after a tab, so spaces
                                // that follow are written as `<text:s/>`.
                                body.push_str("<text:tab/>");
                                prev_space = true;
                            }
                            '\n' => {
                                body.push_str("<text:line-break/>");
                                prev_space = true;
                            }
                            '\u{000C}' | '\u{000E}' => {}
                            c => body.push_str(&x(&c.to_string())),
                        }
                    }
                    if spaces > 0 {
                        // The reader drops a literal space at the end of a paragraph.
                        flush(&mut body, &mut spaces, prev_space || idx == last);
                        prev_space = true;
                    }
                    // A script, `file:`, UNC or other non-web target is written as plain text, as
                    // the HTML and Markdown exporters do (LibreOffice follows `file:`/UNC hrefs).
                    let link = f.link.as_deref().filter(|l| wordcraft_doc::link::is_safe_href(l));
                    if let Some(l) = link {
                        out.push_str(&format!("<text:a xlink:type=\"simple\" xlink:href=\"{}\">", x(l)));
                    }
                    let fs = Fmt { link: None, ..f.clone() };
                    match self.text_style(&fs) {
                        Some(n) => out.push_str(&format!("<text:span text:style-name=\"{n}\">{body}</text:span>")),
                        None => out.push_str(&body),
                    }
                    if link.is_some() {
                        out.push_str("</text:a>");
                    }
                }
                Inline::Image(img) => {
                    let href = self.picture(img);
                    let n = self.pictures.len();
                    // A floating picture: anchored, in a graphic style that places and wraps it.
                    let (anchor, place) = match img.float.as_deref() {
                        Some(f) => {
                            let (anchor, props, xy) = super::frames::frame_of(f);
                            (anchor, format!(" draw:style-name=\"{}\"{xy}", self.graphic_style(props)))
                        }
                        None => ("as-char", String::new()),
                    };
                    out.push_str(&format!(
                        "<draw:frame draw:name=\"{}Image{n}\"{place} text:anchor-type=\"{anchor}\" svg:width=\"{}\" svg:height=\"{}\" draw:z-index=\"0\"><draw:image xlink:href=\"{href}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>",
                        self.prefix,
                        pt(img.w),
                        pt(img.h)
                    ));
                    if !img.alt.is_empty() {
                        out.push_str(&format!("<svg:desc>{}</svg:desc>", x(&img.alt)));
                    }
                    out.push_str("</draw:frame>");
                    prev_space = false;
                }
                Inline::Anchor(a) => out.push_str(&format!("<text:bookmark text:name=\"{}\"/>", x(a))),
                Inline::Note { endnote, custom, blocks } => {
                    self.note(*endnote, custom, blocks, out);
                    prev_space = false;
                }
                Inline::Comment { id, comment } => {
                    self.comment(*id, comment, out);
                    prev_space = false;
                }
                Inline::TextBox { w, h, float, blocks, look } => {
                    self.text_box((*w, *h), float.as_deref(), look, blocks, out);
                    prev_space = false;
                }
                Inline::CommentEnd(id) => out.push_str(&format!("<office:annotation-end office:name=\"{}\"/>", annotation_name(self.prefix, *id))),
                Inline::Field { instr, result, fmt } => match super::fields::element(instr, &x(result)) {
                    Some(el) => {
                        match self.text_style(&Fmt { link: None, ..fmt.clone() }) {
                            Some(n) => out.push_str(&format!("<text:span text:style-name=\"{n}\">{el}</text:span>")),
                            None => out.push_str(&el),
                        }
                        prev_space = false;
                    }
                    // No ODF field for it: what it shows, as text.
                    None => {
                        self.plain_inlines(&[Inline::Text(result.clone(), fmt.clone())], out);
                        prev_space = result.ends_with(' ');
                    }
                },
                // Read as its base text by `ruby_as_text` above.
                Inline::Ruby { .. } => {}
                Inline::Equation { linear, display } => {
                    self.formula(linear, *display, out);
                    prev_space = false;
                }
                Inline::Figure(_) => {}
            }
        }
    }

    /// A comment (ODF 1.3 §14.1): named, so its `office:annotation-end` closes its range, or for
    /// a reply naming the comment it answers (LibreOffice's `loext:parent-name`); its author,
    /// date, initials, resolved state (`loext:resolved`) and text.
    fn comment(&mut self, id: u32, c: &model::FComment, out: &mut String) {
        let reopen = self.open_ins.as_ref().map(|(m, _)| m.clone());
        self.close_ins(out);
        let mut attrs = match c.parent {
            Some(p) => format!(" loext:parent-name=\"{}\"", annotation_name(self.prefix, p)),
            None => format!(" office:name=\"{}\"", annotation_name(self.prefix, id)),
        };
        if c.resolved {
            attrs.push_str(" loext:resolved=\"true\"");
        }
        let mut head = String::new();
        if !c.author.is_empty() {
            head.push_str(&format!("<dc:creator>{}</dc:creator>", x(&c.author)));
        }
        if !c.date.is_empty() {
            head.push_str(&format!("<dc:date>{}</dc:date>", x(&c.date)));
        }
        if !c.initials.is_empty() {
            head.push_str(&format!("<meta:creator-initials>{}</meta:creator-initials>", x(&c.initials)));
        }
        let saved = (std::mem::take(&mut self.cur_style), std::mem::take(&mut self.chain), self.part, self.note_parent.take());
        let mut body = String::new();
        // An annotation holds paragraphs and lists only: a table's text goes in as paragraphs.
        self.blocks(&model::flatten_tables(&c.blocks), &mut body, 1);
        self.close_ins(&mut body);
        (self.cur_style, self.chain, self.part, self.note_parent) = saved;
        out.push_str(&format!("<office:annotation{attrs}>{head}{body}</office:annotation>"));
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }

    /// A text box (ODF 1.3 §10.4.3 `draw:text-box` in a `draw:frame`) of `size` (points): in line
    /// with the text, or anchored and placed as it floats, with its fill, line and padding.
    fn text_box(&mut self, size: (f32, f32), float: Option<&wordcraft_doc::para::Float>, look: &model::BoxLook, blocks: &[FBlock], out: &mut String) {
        let reopen = self.open_ins.as_ref().map(|(m, _)| m.clone());
        self.close_ins(out);
        let frame = box_look_attrs(look);
        let (anchor, props, xy) = match float {
            Some(f) => super::frames::frame_of(f),
            None => ("as-char", " style:vertical-pos=\"top\" style:vertical-rel=\"baseline\"".to_string(), String::new()),
        };
        let style = self.graphic_style(format!("{props}{frame}"));
        let saved = (std::mem::take(&mut self.cur_style), std::mem::take(&mut self.chain), self.part, self.note_parent.take());
        let mut body = String::new();
        self.blocks(blocks, &mut body, 1);
        self.close_ins(&mut body);
        (self.cur_style, self.chain, self.part, self.note_parent) = saved;
        if body.is_empty() {
            body.push_str("<text:p/>");
        }
        self.boxes += 1;
        let fit = |v: f32| if v.is_finite() { v.clamp(1.0, 1584.0) } else { 72.0 };
        // A box growing to its text has a least height (§20.207 `fo:min-height`), not a height.
        let (height, min) = if look.auto_fit {
            (String::new(), format!(" fo:min-height=\"{}\"", pt(fit(size.1))))
        } else {
            (format!(" svg:height=\"{}\"", pt(fit(size.1))), String::new())
        };
        out.push_str(&format!(
            "<draw:frame draw:name=\"{}Frame{}\" draw:style-name=\"{style}\" text:anchor-type=\"{anchor}\" svg:width=\"{}\"{height}{xy} draw:z-index=\"0\"><draw:text-box{min}>{body}</draw:text-box></draw:frame>",
            self.prefix,
            self.boxes,
            pt(fit(size.0)),
        ));
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }

    /// A footnote or endnote (ODF 1.3 §6.3): its citation (the number LibreOffice shows until it
    /// renumbers, or the custom mark) and its content. The paragraph being written is put aside
    /// meanwhile, and an insertion open around the note ends before it and goes on after it.
    fn note(&mut self, endnote: bool, custom: &str, blocks: &[FBlock], out: &mut String) {
        let reopen = self.open_ins.as_ref().map(|(m, _)| m.clone());
        self.close_ins(out);
        self.notes += 1;
        let id = format!("{}ftn{}", self.prefix, self.notes);
        let (class, parent) = if endnote { ("endnote", "Endnote") } else { ("footnote", "Footnote") };
        let saved = (std::mem::take(&mut self.cur_style), std::mem::take(&mut self.chain), self.part, self.note_parent.replace(parent));
        let mut body = String::new();
        self.blocks(blocks, &mut body, 1);
        self.close_ins(&mut body);
        (self.cur_style, self.chain, self.part, self.note_parent) = saved;
        if body.is_empty() {
            body.push_str(&format!("<text:p text:style-name=\"{parent}\"/>"));
        }
        // The number LibreOffice shows by default (footnotes 1, 2, 3; endnotes i, ii, iii) until it
        // renumbers them itself.
        let number = if endnote {
            self.endnotes = self.endnotes.saturating_add(1);
            wordcraft_doc::section::roman(self.endnotes)
        } else {
            self.footnotes = self.footnotes.saturating_add(1);
            self.footnotes.to_string()
        };
        let (label, citation) = if custom.is_empty() { (String::new(), number) } else { (format!(" text:label=\"{}\"", x(custom)), x(custom)) };
        out.push_str(&format!(
            "<text:note text:id=\"{id}\" text:note-class=\"{class}\"><text:note-citation{label}>{citation}</text:note-citation><text:note-body>{body}</text:note-body></text:note>"
        ));
        if let Some(m) = reopen {
            self.open_ins(&m, out);
        }
    }

    /// Paragraph `p`, or with `chain` the paragraphs joined at deleted paragraph marks (`p`
    /// is the first).
    fn para_in(&mut self, p: &Para, chain: Option<&Vec<Para>>, out: &mut String) {
        let paras: &[Para] = chain.map(Vec::as_slice).unwrap_or(std::slice::from_ref(p));
        let start = self.master.take();
        self.chain = chain.map(|c| c.iter().map(|q| self.para_style(q)).collect()).unwrap_or_default();
        self.part = 0;
        let style = self.para_style_at(p, start.as_ref());
        self.cur_style = style.clone();
        let tag = match p.kind {
            Kind::Heading(n) => {
                out.push_str(&format!("<text:h text:style-name=\"{style}\" text:outline-level=\"{}\">", n.clamp(1, 6)));
                "</text:h>"
            }
            _ => {
                out.push_str(&format!("<text:p text:style-name=\"{style}\">"));
                "</text:p>"
            }
        };
        let tracked = paras.len() > 1
            || p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) | Inline::Ruby { fmt: f, .. } | Inline::Field { fmt: f, .. } if f.ins.is_some() || f.del.is_some()));
        if tracked {
            let mut items = Vec::new();
            for (k, q) in paras.iter().enumerate() {
                items.extend(q.inlines.iter().map(Item::Inl));
                if k + 1 < paras.len() {
                    items.push(Item::Mark(q));
                }
            }
            self.items(&items, out);
        } else {
            self.close_ins(out);
            self.plain_inlines(&p.inlines, out);
        }
        // An inserted paragraph mark: the insertion runs on past the paragraph's end.
        match paras.last().and_then(|q| q.mark_ins.as_deref()) {
            Some(m) => self.open_ins(m, out),
            None => self.close_ins(out),
        }
        out.push_str(tag);
        self.chain.clear();
    }

    pub(super) fn blocks(&mut self, blocks: &[FBlock], out: &mut String, depth: usize) {
        let joined = join_deleted_marks(blocks);
        let (blocks, chains) = match &joined {
            Some((b, c)) => (b.as_slice(), Some(c)),
            None => (blocks, None),
        };
        let chain = |i: usize| chains.and_then(|c| c.get(&i));
        let mut i = 0;
        while let Some(b) = blocks.get(i) {
            // Only the first top-level block of a section starts its page.
            if depth == 0 {
                self.master = self.pending_start.take();
            }
            match b {
                FBlock::Table(t) => {
                    if depth < model::MAX_DEPTH {
                        self.table(t, out, depth);
                    }
                    i += 1;
                }
                FBlock::Para(p) if p.list.is_some() => {
                    let start = i;
                    let id = p.list.map_or(0, |l| l.list);
                    let first = ListOut { ordered: p.list.is_some_and(|l| l.ordered), label: None, start: 1 };
                    let mut kinds = [first; 9];
                    let mut seen = [false; 9];
                    while let Some(FBlock::Para(q)) = blocks.get(i) {
                        let Some(li) = q.list else { break };
                        let l = li.level.min(8) as usize;
                        // An item of another list, or (lists not told apart) a top-level item of
                        // the other kind, starts a new list.
                        if li.list != id || (id == 0 && l == 0 && seen[0] && kinds[0].ordered != li.ordered) {
                            break;
                        }
                        if let (Some(s), Some(k)) = (seen.get_mut(l), kinds.get_mut(l))
                            && !*s
                        {
                            *s = true;
                            *k = ListOut { ordered: li.ordered, label: li.label, start: li.start };
                        }
                        i += 1;
                    }
                    self.list_styles.push(kinds);
                    let lname = format!("{}L{}", self.prefix, self.list_styles.len());
                    let id_of_list = id;
                    // Open lists: per level, whether a list-item is open.
                    let mut stack: Vec<bool> = Vec::new();
                    for (qi, q) in blocks.get(start..i).unwrap_or(&[]).iter().enumerate() {
                        let FBlock::Para(q) = q else { continue };
                        let lv = q.list.map(|l| l.level.min(8) as usize).unwrap_or(0);
                        while stack.len() > lv + 1 {
                            if stack.pop() == Some(true) {
                                out.push_str("</text:list-item>");
                            }
                            out.push_str("</text:list>");
                        }
                        while stack.len() < lv + 1 {
                            if let Some(top) = stack.last_mut()
                                && !*top
                            {
                                out.push_str("<text:list-item>");
                                *top = true;
                            }
                            if stack.is_empty() {
                                let id = format!("{}lst{}", self.prefix, self.list_styles.len());
                                // A list cut by a section boundary goes on numbering: continue-list names the
                                // preceding list by its xml:id, whatever its style name (ODF 1.3 §19.743).
                                let prev = self.last_list.clone();
                                let cont = match prev {
                                    Some(prev) if depth == 0 && start == 0 && std::mem::take(&mut self.continue_list) => {
                                        format!(" text:continue-list=\"{prev}\"")
                                    }
                                    // A list that showed up before goes on with it.
                                    _ => match self.list_xml.get(&id_of_list) {
                                        Some(x) if id_of_list != 0 => format!(" text:continue-list=\"{x}\""),
                                        _ => String::new(),
                                    },
                                };
                                if id_of_list != 0 {
                                    self.list_xml.entry(id_of_list).or_insert_with(|| id.clone());
                                }
                                if depth == 0 {
                                    self.last_list = Some(id.clone());
                                }
                                out.push_str(&format!("<text:list xml:id=\"{id}\" text:style-name=\"{lname}\"{cont}>"));
                            } else {
                                out.push_str("<text:list>");
                            }
                            stack.push(false);
                        }
                        if let Some(top) = stack.last_mut() {
                            if *top {
                                out.push_str("</text:list-item>");
                            }
                            *top = true;
                        }
                        out.push_str("<text:list-item>");
                        self.para_in(q, chain(start + qi), out);
                    }
                    while let Some(open) = stack.pop() {
                        if open {
                            out.push_str("</text:list-item>");
                        }
                        out.push_str("</text:list>");
                    }
                    out.push('\n');
                }
                FBlock::Para(p) => {
                    self.para_in(p, chain(i), out);
                    out.push('\n');
                    i += 1;
                }
            }
        }
    }

    /// An equation as an embedded formula object (ODF 1.3 §10.4.3): a frame in line with the text
    /// pointing at a sub-document whose `content.xml` is the equation's MathML, its frame about
    /// the size the formula takes (LibreOffice scales the formula to the frame).
    fn formula(&mut self, linear: &str, display: bool, out: &mut String) {
        let n = self.formulas.len() + 1;
        let name = format!("Object {}{n}", self.prefix);
        let mathml = crate::mathml::linear_to_mathml(linear, display);
        self.formulas.push((name.clone(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{mathml}\n")));
        let (w, h) = super::formula_size::size(linear);
        // Centred on the line, as LibreOffice sets its own formulas.
        let style = self.graphic_style(" style:vertical-pos=\"middle\" style:vertical-rel=\"text\"".into());
        out.push_str(&format!(
            "<draw:frame draw:name=\"{}Formula{n}\" draw:style-name=\"{style}\" text:anchor-type=\"as-char\" svg:width=\"{}\" svg:height=\"{}\" draw:z-index=\"0\"><draw:object xlink:href=\"./{}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/><svg:desc>{}</svg:desc></draw:frame>",
            self.prefix,
            pt(w),
            pt(h),
            x(&name),
            x(linear)
        ));
    }

    /// The automatic graphic style of a frame placed by `props`.
    fn graphic_style(&mut self, props: String) -> String {
        let k = match self.graphic_styles.iter().position(|s| *s == props) {
            Some(k) => k,
            None => {
                self.graphic_styles.push(props);
                self.graphic_styles.len() - 1
            }
        };
        format!("{}fr{}", self.prefix, k + 1)
    }

    /// The automatic style of a cell with `props` (its `style:table-cell-properties` attributes).
    fn cell_style(&mut self, props: String) -> String {
        let k = match self.cell_styles.iter().position(|s| *s == props) {
            Some(k) => k,
            None => {
                self.cell_styles.push(props);
                self.cell_styles.len() - 1
            }
        };
        format!("{}Cell{}", self.prefix, k + 1)
    }

    fn table(&mut self, t: &FTable, out: &mut String, depth: usize) {
        self.close_ins(out);
        let cols = t.cols().max(1);
        let widths: Vec<f32> = if t.widths.len() == cols && t.widths.iter().all(|w| w.is_finite() && *w > 0.0) {
            t.widths.clone()
        } else {
            vec![468.0 / cols as f32; cols]
        };
        self.tables.push(widths);
        self.table_starts.push(self.master.take());
        let tn = format!("{}{}", self.prefix, self.tables.len());
        out.push_str(&format!("<table:table table:name=\"Table{tn}\" table:style-name=\"Table{tn}\">"));
        for c in 0..cols {
            out.push_str(&format!("<table:table-column table:style-name=\"Table{tn}.C{}\"/>", c + 1));
        }
        let header_rows = t.rows.iter().take_while(|r| !r.is_empty() && r.iter().all(|c| c.header)).count();
        for (ri, row) in t.rows.iter().enumerate() {
            if ri == 0 && header_rows > 0 {
                out.push_str("<table:table-header-rows>");
            }
            out.push_str("<table:table-row>");
            let mut g = 0usize;
            for c in row {
                let span = c.colspan.clamp(1, 63) as usize;
                g += span;
                if c.covered {
                    for _ in 0..span {
                        out.push_str("<table:covered-table-cell/>");
                    }
                    continue;
                }
                let st = self.cell_style(cell_props(t, ri, g - span, c, cols));
                let mut attrs = format!(" table:style-name=\"{st}\" office:value-type=\"string\"");
                if span > 1 {
                    attrs.push_str(&format!(" table:number-columns-spanned=\"{span}\""));
                }
                if c.rowspan > 1 {
                    attrs.push_str(&format!(" table:number-rows-spanned=\"{}\"", c.rowspan));
                }
                out.push_str(&format!("<table:table-cell{attrs}>"));
                if c.blocks.is_empty() {
                    out.push_str("<text:p text:style-name=\"Standard\"/>");
                }
                self.blocks(&c.blocks, out, depth + 1);
                self.close_ins(out);
                out.push_str("</table:table-cell>");
                for _ in 1..span {
                    out.push_str("<table:covered-table-cell/>");
                }
            }
            for _ in g..cols {
                out.push_str("<table:table-cell office:value-type=\"string\"><text:p/></table:table-cell>");
            }
            out.push_str("</table:table-row>");
            if ri + 1 == header_rows {
                out.push_str("</table:table-header-rows>");
            }
        }
        out.push_str("</table:table>\n");
    }

    pub(super) fn automatic_styles(&self) -> String {
        format!("<office:automatic-styles>{}</office:automatic-styles>", self.style_defs())
    }

    /// The automatic styles made so far, without their `office:automatic-styles` element.
    pub(super) fn style_defs(&self) -> String {
        let mut s = String::new();
        for (props, n) in &self.text_styles {
            let (props, parent) = match props.split_once('\u{1}') {
                Some((p, parent)) => (p, format!(" style:parent-style-name=\"{parent}\"")),
                None => (props.as_str(), String::new()),
            };
            s.push_str(&format!("<style:style style:name=\"{n}\" style:family=\"text\"{parent}><style:text-properties{props}/></style:style>"));
        }
        for ((parent, props, master), n) in &self.para_styles {
            let master = if master.is_empty() { String::new() } else { format!(" style:master-page-name=\"{}\"", x(master)) };
            let (attrs, children) = props.split_once('\u{1}').unwrap_or((props, ""));
            s.push_str(&format!(
                "<style:style style:name=\"{n}\" style:family=\"paragraph\" style:parent-style-name=\"{parent}\"{master}><style:paragraph-properties{attrs}>{children}</style:paragraph-properties></style:style>"
            ));
        }
        for (i, kinds) in self.list_styles.iter().enumerate() {
            s.push_str(&format!("<text:list-style style:name=\"{}L{}\">", self.prefix, i + 1));
            for (l, out) in kinds.iter().enumerate() {
                let ordered = &out.ordered;
                let lv = l + 1;
                // The level's own place, else WordCraft's default: 36 pt a level, hanging 18 pt.
                let (indent, hanging) = out.label.and_then(|lb| lb.position()).unwrap_or((36.0 * lv as f32, 18.0));
                let align = match out.label.and_then(|lb| lb.align) {
                    Some(Align::Right) => " fo:text-align=\"end\"",
                    Some(Align::Center) => " fo:text-align=\"center\"",
                    _ => "",
                };
                let props = format!(
                    "<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"{align}><style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"{}\" fo:text-indent=\"{}\" fo:margin-left=\"{}\"/></style:list-level-properties>",
                    pt(indent),
                    pt(-hanging),
                    pt(indent)
                );
                if *ordered {
                    let fmt = ["1", "a", "i"].get(l % 3).copied().unwrap_or("1");
                    // The level's own label, else WordCraft's default for the level.
                    let attrs = match out.label {
                        Some(lb) if lb.format != wordcraft_doc::section::NumFormat::Bullet => {
                            let mut a = format!(" style:num-format=\"{}\"", odf_num_format(lb.format));
                            for (k, v) in [("style:num-prefix", lb.prefix.as_str()), ("style:num-suffix", lb.suffix.as_str())] {
                                if !v.is_empty() {
                                    a.push_str(&format!(" {k}=\"{}\"", x(v)));
                                }
                            }
                            if lb.shown > 1 {
                                a.push_str(&format!(" text:display-levels=\"{}\"", lb.shown.min(lv as u8)));
                            }
                            a
                        }
                        _ => format!(" style:num-suffix=\".\" style:num-format=\"{fmt}\""),
                    };
                    let start = if out.start != 1 { format!(" text:start-value=\"{}\"", out.start) } else { String::new() };
                    s.push_str(&format!("<text:list-level-style-number text:level=\"{lv}\"{attrs}{start}>{props}</text:list-level-style-number>"));
                } else {
                    let own = out.label.and_then(|lb| lb.bullet()).map(|c| c.to_string());
                    let ch = own.as_deref().unwrap_or_else(|| ["•", "◦", "▪"].get(l % 3).copied().unwrap_or("•"));
                    let ch = x(ch);
                    s.push_str(&format!(
                        "<text:list-level-style-bullet text:level=\"{lv}\" text:bullet-char=\"{ch}\">{props}</text:list-level-style-bullet>"
                    ));
                }
            }
            s.push_str("</text:list-style>");
        }
        for (i, widths) in self.tables.iter().enumerate() {
            let tn = format!("{}{}", self.prefix, i + 1);
            let total: f32 = widths.iter().sum();
            let (master, number) = match self.table_starts.get(i).and_then(Option::as_ref) {
                Some((m, n)) => {
                    (format!(" style:master-page-name=\"{}\"", x(m)), n.map(|n| format!(" style:page-number=\"{n}\"")).unwrap_or_default())
                }
                None => (String::new(), String::new()),
            };
            s.push_str(&format!(
                "<style:style style:name=\"Table{tn}\" style:family=\"table\"{master}><style:table-properties style:width=\"{}\" table:align=\"margins\"{number}/></style:style>",
                pt(total)
            ));
            for (c, w) in widths.iter().enumerate() {
                s.push_str(&format!(
                    "<style:style style:name=\"Table{tn}.C{}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{}\"/></style:style>",
                    c + 1,
                    pt(*w)
                ));
            }
        }
        for (i, props) in self.graphic_styles.iter().enumerate() {
            s.push_str(&format!(
                "<style:style style:name=\"{}fr{}\" style:family=\"graphic\"><style:graphic-properties{props}/></style:style>",
                self.prefix,
                i + 1
            ));
        }
        for (i, props) in self.cell_styles.iter().enumerate() {
            s.push_str(&format!(
                "<style:style style:name=\"{}Cell{}\" style:family=\"table-cell\"><style:table-cell-properties{props}/></style:style>",
                self.prefix,
                i + 1
            ));
        }
        for st in &self.section_styles {
            s.push_str(st);
        }
        s
    }
}

/// One piece of a paragraph being written: an inline, or the deleted mark of a paragraph
/// joined to the next one.
enum Item<'a> {
    Inl(&'a Inline),
    Mark(&'a Para),
}

impl Item<'_> {
    /// Its tracked insertion and deletion.
    fn marks(&self) -> (Option<&RevMark>, Option<&RevMark>) {
        match self {
            Item::Inl(Inline::Text(_, f) | Inline::Ruby { fmt: f, .. } | Inline::Field { fmt: f, .. }) => (f.ins.as_ref(), f.del.as_ref()),
            Item::Inl(_) => (None, None),
            Item::Mark(p) => (p.mark_ins.as_deref(), p.mark_del.as_deref()),
        }
    }

    /// What a run of content is written as: deleted content is one region whatever was
    /// inserted inside it, so only its deletion counts.
    fn segment(&self) -> (Option<&RevMark>, Option<&RevMark>) {
        match self.marks() {
            (_, Some(del)) => (None, Some(del)),
            k => k,
        }
    }

    fn inline(&self) -> Option<&Inline> {
        match self {
            Item::Inl(i) => Some(i),
            Item::Mark(_) => None,
        }
    }
}

/// Close the insertion open in `st`.
fn ins_close(st: &mut Option<(RevMark, String)>, out: &mut String) {
    if let Some((_, ct)) = st.take() {
        out.push_str(&format!("<text:change-end text:change-id=\"{ct}\"/>"));
    }
}

/// The blocks after joining at deleted marks, and each joined run's paragraphs by the first one's index.
type JoinedMarks = (Vec<FBlock>, HashMap<usize, Vec<Para>>);

/// `blocks` with each run of paragraphs joined at tracked deleted paragraph marks (ODF keeps
/// such a deletion inside one body paragraph): the first paragraph stands for the run, and the
/// whole run is returned by its index. `None` when nothing is joined. A deleted mark with no
/// paragraph after it (before a table, at the end) is written as a plain mark.
fn join_deleted_marks(blocks: &[FBlock]) -> Option<JoinedMarks> {
    if !blocks.iter().any(|b| matches!(b, FBlock::Para(p) if p.mark_del.is_some())) {
        return None;
    }
    let mut out = Vec::with_capacity(blocks.len());
    let mut chains = HashMap::new();
    let mut i = 0;
    while let Some(b) = blocks.get(i) {
        i += 1;
        let FBlock::Para(p) = b else {
            out.push(b.clone());
            continue;
        };
        let mut chain = vec![p.clone()];
        while chain.last().is_some_and(|q| q.mark_del.is_some())
            && let Some(FBlock::Para(q)) = blocks.get(i)
        {
            chain.push(q.clone());
            i += 1;
        }
        if chain.len() > 1 {
            chains.insert(out.len(), chain);
        }
        out.push(FBlock::Para(p.clone()));
    }
    Some((out, chains))
}

/// Now, as an `xsd:dateTime` (UTC).
fn now_iso() -> String {
    let t = now_unix();
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Reuse the crate's 400-year-cycle converter (chrono within a cycle, cycles carried in the
    // year) so a clock past chrono's range keeps the proleptic Gregorian date the old hand-rolled
    // code gave, never the epoch. `{y:04}` rather than `%Y`, which writes a `+` before year 10000.
    let (y, m, d) = crate::sheet::civil(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Seconds since the Unix epoch: the system clock, or the browser's on the web.
fn now_unix() -> i64 {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
    }
    // `SystemTime::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let ms = js_sys::Date::now();
        if ms.is_finite() && ms > 0.0 { (ms / 1000.0) as i64 } else { 0 }
    }
}
