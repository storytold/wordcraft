//! Styles: the document defaults as the default style, the default paragraph style as
//! `Standard`, and direct formatting as automatic styles (ODF 1.3 Part 3 §16).
//!
//! LibreOffice keeps the space above, the space below and contextual spacing as one property: a
//! style or paragraph that sets only some of them gets the rest from the default paragraph style,
//! not from its parent. So whenever one is set, all three are written, the missing ones resolved
//! from what the paragraph inherits ([`complete_spacing`]). Tab stops are written as the whole merged list
//! (a child's stops replace its parent's in ODF, they add to them in the model).

use wordcraft_doc::props::{
    Align, Border, BorderStyle, Borders, CharProps, Highlight, LineSpacing, ParaProps, TabAlign, TabLeader, TabStop, TextColor, Underline, VertAlign,
};
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{Document, Paragraph};

use super::{Ctx, Props, attrs, pt};
use crate::odt::x;

/// The name of the default paragraph style in ODF.
pub(super) const STANDARD: &str = "Standard";

/// The model's default paragraph style id (what a paragraph without a style has).
fn default_para_id(doc: &Document) -> &str {
    doc.styles.styles.iter().find(|s| s.kind == StyleKind::Paragraph && s.default).map(|s| s.id.as_str()).unwrap_or("Normal")
}

pub(super) fn is_default_para(doc: &Document, id: &str) -> bool {
    id == "Normal" || id == default_para_id(doc)
}

/// `1.5pt solid #000000`, or `none`.
pub(super) fn border(b: &Border) -> String {
    if !b.is_visible() {
        return "none".into();
    }
    let (style, w) = match b.style {
        BorderStyle::Double | BorderStyle::Triple => ("double", b.width * 3.0),
        BorderStyle::Dotted => ("dotted", b.width),
        BorderStyle::Dashed => ("dashed", b.width),
        BorderStyle::DotDash => ("dot-dash", b.width),
        _ => ("solid", b.width),
    };
    let color = b.color.map(|c| c.hex().to_ascii_lowercase()).unwrap_or_else(|| "000000".into());
    format!("{} {style} #{color}", pt(w.clamp(0.05, 72.0)))
}

/// Borders on the four sides (and their distance from the text as padding).
pub(super) fn borders(b: &Borders, p: &mut Props) {
    for (side, bd, pad) in [
        (b.top, "fo:border-top", "fo:padding-top"),
        (b.bottom, "fo:border-bottom", "fo:padding-bottom"),
        (b.left, "fo:border-left", "fo:padding-left"),
        (b.right, "fo:border-right", "fo:padding-right"),
    ] {
        if let Some(s) = side {
            p.insert(bd, border(&s));
            if s.is_visible() {
                p.insert(pad, pt(s.space.clamp(0.0, 72.0)));
            }
        }
    }
}

fn color(c: wordcraft_doc::Rgb) -> String {
    format!("#{}", c.hex().to_ascii_lowercase())
}

fn on(b: bool, yes: &str, no: &str) -> String {
    if b { yes } else { no }.to_string()
}

/// `en-US` → `fo:language="en" fo:country="US"` (or the asian/complex pair).
fn lang(p: &mut Props, tag: &str, l: &'static str, c: &'static str) {
    let mut it = tag.split(['-', '_']);
    if let Some(a) = it.next().filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())) {
        p.insert(l, a.to_ascii_lowercase());
        if let Some(b) = it.next().filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())) {
            p.insert(c, b.to_ascii_uppercase());
        }
    }
}

/// `style:text-properties` attributes of `c` (set fields only).
pub(super) fn char_props(ctx: &mut Ctx, c: &CharProps, p: &mut Props) {
    if let Some(f) = c.font.as_deref().filter(|f| !f.trim().is_empty()) {
        p.insert("style:font-name", ctx.font(f));
    }
    if let Some(f) = c.font_ea.as_deref().filter(|f| !f.trim().is_empty()) {
        p.insert("style:font-name-asian", ctx.font(f));
    }
    if let Some(f) = c.font_cs.as_deref().filter(|f| !f.trim().is_empty()) {
        p.insert("style:font-name-complex", ctx.font(f));
    }
    if let Some(s) = c.size.filter(|s| s.is_finite() && *s > 0.0) {
        p.insert("fo:font-size", pt(s.min(1638.0)));
        p.insert("style:font-size-asian", pt(s.min(1638.0)));
    }
    if let Some(s) = c.size_cs.filter(|s| s.is_finite() && *s > 0.0) {
        p.insert("style:font-size-complex", pt(s.min(1638.0)));
    }
    if let Some(b) = c.bold {
        p.insert("fo:font-weight", on(b, "bold", "normal"));
        p.insert("style:font-weight-asian", on(b, "bold", "normal"));
    }
    if let Some(b) = c.bold_cs {
        p.insert("style:font-weight-complex", on(b, "bold", "normal"));
    }
    if let Some(i) = c.italic {
        p.insert("fo:font-style", on(i, "italic", "normal"));
        p.insert("style:font-style-asian", on(i, "italic", "normal"));
    }
    if let Some(i) = c.italic_cs {
        p.insert("style:font-style-complex", on(i, "italic", "normal"));
    }
    if let Some(u) = c.underline {
        let (style, ty, width) = match u {
            Underline::None => ("none", None, None),
            Underline::Double => ("solid", Some("double"), None),
            Underline::Thick => ("solid", None, Some("bold")),
            Underline::Dotted => ("dotted", None, None),
            Underline::Dash => ("dash", None, None),
            Underline::DotDash => ("dot-dash", None, None),
            Underline::DotDotDash => ("dot-dot-dash", None, None),
            Underline::Wave => ("wave", None, None),
            Underline::DoubleWave => ("wave", Some("double"), None),
            Underline::Single | Underline::Words => ("solid", None, None),
        };
        p.insert("style:text-underline-style", style.into());
        if style != "none" {
            p.insert("style:text-underline-width", width.unwrap_or("auto").into());
            p.insert("style:text-underline-color", c.underline_color.map(color).unwrap_or_else(|| "font-color".into()));
            p.insert("style:text-underline-mode", on(u == Underline::Words, "skip-white-space", "continuous"));
        }
        if let Some(t) = ty {
            p.insert("style:text-underline-type", t.into());
        }
    }
    if let Some(s) = c.strike {
        p.insert("style:text-line-through-style", on(s, "solid", "none"));
    }
    if c.double_strike == Some(true) {
        p.insert("style:text-line-through-style", "solid".into());
        p.insert("style:text-line-through-type", "double".into());
    }
    match c.color {
        Some(TextColor::Rgb(rgb)) => {
            p.insert("fo:color", color(rgb));
        }
        Some(TextColor::Auto) => {
            p.insert("style:use-window-font-color", "true".into());
        }
        None => {}
    }
    match (c.highlight, c.shading) {
        (Some(h), _) if h != Highlight::None => {
            if let Some(rgb) = h.rgb() {
                p.insert("fo:background-color", color(rgb));
            }
        }
        (_, Some(rgb)) => {
            p.insert("fo:background-color", color(rgb));
        }
        (Some(_), None) => {
            p.insert("fo:background-color", "transparent".into());
        }
        (None, None) => {}
    }
    match c.vert_align {
        Some(VertAlign::Superscript) => {
            p.insert("style:text-position", "super 58%".into());
        }
        Some(VertAlign::Subscript) => {
            p.insert("style:text-position", "sub 58%".into());
        }
        Some(VertAlign::Baseline) | None => {
            if let Some(pos) = c.position.filter(|v| v.is_finite() && *v != 0.0) {
                // A raise in points becomes a share of the font size.
                let size = c.size.or(ctx.doc.styles.default_chr.size).filter(|s| *s > 0.0).unwrap_or(12.0);
                p.insert("style:text-position", format!("{}% 100%", (pos / size * 100.0).clamp(-1000.0, 1000.0).round()));
            } else if c.vert_align.is_some() {
                p.insert("style:text-position", "0% 100%".into());
            }
        }
    }
    if let Some(v) = c.caps {
        p.insert("fo:text-transform", on(v, "uppercase", "none"));
    }
    if let Some(v) = c.small_caps {
        p.insert("fo:font-variant", on(v, "small-caps", "normal"));
    }
    if c.hidden == Some(true) {
        p.insert("text:display", "none".into());
    }
    if let Some(s) = c.spacing.filter(|s| s.is_finite()) {
        p.insert("fo:letter-spacing", if s == 0.0 { "normal".into() } else { pt(s.clamp(-1584.0, 1584.0)) });
    }
    if let Some(s) = c.scale.filter(|s| s.is_finite() && *s > 0.0) {
        p.insert("style:text-scale", format!("{}%", s.clamp(1.0, 600.0).round()));
    }
    if let Some(k) = c.kern {
        p.insert("style:letter-kerning", on(k > 0.0, "true", "false"));
    }
    if let Some(v) = c.outline {
        p.insert("style:text-outline", on(v, "true", "false"));
    }
    if let Some(v) = c.shadow {
        p.insert("fo:text-shadow", on(v, "1pt 1pt", "none"));
    }
    if c.emboss == Some(true) {
        p.insert("style:font-relief", "embossed".into());
    } else if c.engrave == Some(true) {
        p.insert("style:font-relief", "engraved".into());
    } else if c.emboss.is_some() || c.engrave.is_some() {
        p.insert("style:font-relief", "none".into());
    }
    if let Some(l) = c.lang.as_deref() {
        lang(p, l, "fo:language", "fo:country");
    }
    if let Some(l) = c.lang_bidi.as_deref() {
        lang(p, l, "style:language-complex", "style:country-complex");
    }
    // todo: character borders, text effects, East Asian language, no-proof.
}

/// Paragraph properties of `pp` (set fields only) into `p` (`style:paragraph-properties`) and
/// `t` (`style:text-properties`). Tab stops, numbering and the style are left to the caller.
pub(super) fn para_props(pp: &ParaProps, p: &mut Props, t: &mut Props) {
    if let Some(a) = pp.align {
        let v = match a {
            Align::Left => "start",
            Align::Center => "center",
            Align::Right => "end",
            Align::Justify | Align::Distribute => "justify",
        };
        p.insert("fo:text-align", v.into());
        if a == Align::Distribute {
            p.insert("fo:text-align-last", "justify".into());
        }
    }
    let len = |v: f32| pt(v.clamp(-1584.0, 1584.0));
    for (v, k) in [(pp.indent_left, "fo:margin-left"), (pp.indent_right, "fo:margin-right"), (pp.indent_first, "fo:text-indent")] {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            p.insert(k, len(v));
        }
    }
    for (v, k) in [(pp.space_before, "fo:margin-top"), (pp.space_after, "fo:margin-bottom")] {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            p.insert(k, pt(v.clamp(0.0, 1584.0)));
        }
    }
    if let Some(v) = pp.contextual_spacing {
        p.insert("style:contextual-spacing", on(v, "true", "false"));
    }
    match pp.line_spacing {
        Some(LineSpacing::Multiple(m)) if m.is_finite() && m > 0.0 => {
            p.insert("fo:line-height", format!("{}%", (m * 100.0).clamp(1.0, 13200.0).round()));
        }
        Some(LineSpacing::AtLeast(v)) if v.is_finite() => {
            p.insert("style:line-height-at-least", pt(v.clamp(0.0, 1584.0)));
        }
        Some(LineSpacing::Exactly(v)) if v.is_finite() => {
            p.insert("fo:line-height", pt(v.abs().clamp(0.0, 1584.0)));
        }
        _ => {}
    }
    if let Some(v) = pp.keep_next {
        p.insert("fo:keep-with-next", on(v, "always", "auto"));
    }
    if let Some(v) = pp.keep_lines {
        p.insert("fo:keep-together", on(v, "always", "auto"));
    }
    if let Some(v) = pp.page_break_before {
        p.insert("fo:break-before", on(v, "page", "auto"));
    }
    if let Some(v) = pp.widow_control {
        p.insert("fo:widows", on(v, "2", "0"));
        p.insert("fo:orphans", on(v, "2", "0"));
    }
    if let Some(c) = pp.shading {
        p.insert("fo:background-color", color(c));
    }
    if let Some(b) = &pp.borders {
        borders(b, p);
    }
    if let Some(v) = pp.bidi {
        p.insert("style:writing-mode", on(v, "rl-tb", "lr-tb"));
    }
    if let Some(v) = pp.suppress_line_numbers {
        p.insert("text:number-lines", on(!v, "true", "false"));
    }
    if let Some(v) = pp.snap_to_grid {
        p.insert("style:snap-to-layout-grid", on(v, "true", "false"));
    }
    if let Some(v) = pp.suppress_hyphens {
        t.insert("fo:hyphenate", on(!v, "true", "false"));
    }
    // todo: indents in characters, borders between paragraphs, drop caps, text frames
    // (`ParaProps::frame`), East Asian line breaking.
}

/// Fill in the space above, the space below and contextual spacing from `base` (the properties
/// the paragraph or style inherits) when `p` sets only some of them (see the module docs).
pub(super) fn complete_spacing(base: &ParaProps, p: &mut Props) {
    const KEYS: [&str; 3] = ["fo:margin-top", "fo:margin-bottom", "style:contextual-spacing"];
    if !KEYS.iter().any(|k| p.contains_key(k)) {
        return;
    }
    let values = [
        pt(base.space_before.filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, 1584.0)),
        pt(base.space_after.filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, 1584.0)),
        on(base.contextual_spacing.unwrap_or(false), "true", "false"),
    ];
    for (k, v) in KEYS.into_iter().zip(values) {
        p.entry(k).or_insert(v);
    }
}

/// Tab stops `base` with `add` merged in (a stop at the same place replaces the inherited one,
/// a clear stop removes it), in position order.
pub(super) fn merged_tabs(base: &[TabStop], add: &[TabStop]) -> Vec<TabStop> {
    let mut v: Vec<TabStop> = base.iter().filter(|t| !add.iter().any(|a| (a.pos - t.pos).abs() < 0.5)).copied().collect();
    v.extend(add.iter().copied());
    v.retain(|t| t.align != TabAlign::Clear && t.pos.is_finite());
    v.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    v
}

/// `style:tab-stops`. Positions are from the text column's edge: `TabsRelativeToIndent` is off
/// in `settings.xml`.
pub(super) fn tabs_xml(tabs: &[TabStop]) -> String {
    let mut s = String::from("<style:tab-stops>");
    for t in tabs.iter().take(64) {
        let ty = match t.align {
            TabAlign::Right => " style:type=\"right\"",
            TabAlign::Center => " style:type=\"center\"",
            TabAlign::Decimal => " style:type=\"char\" style:char=\".\"",
            _ => "",
        };
        let lead = match t.leader {
            TabLeader::None => String::new(),
            l => {
                let c = l.char().unwrap_or('.');
                let style = if l == TabLeader::Underscore { "solid" } else { "dotted" };
                format!(" style:leader-style=\"{style}\" style:leader-text=\"{}\"", x(&c.to_string()))
            }
        };
        s.push_str(&format!("<style:tab-stop style:position=\"{}\"{ty}{lead}/>", pt(t.pos.clamp(0.0, 1584.0))));
    }
    s.push_str("</style:tab-stops>");
    s
}

/// `<style:paragraph-properties …>tabs</…>` and `<style:text-properties …/>`, empty parts left out.
fn props_xml(pp: &Props, tabs: Option<&[TabStop]>, tp: &Props) -> String {
    let mut s = String::new();
    if !pp.is_empty() || tabs.is_some() {
        s.push_str(&format!("<style:paragraph-properties{}>{}</style:paragraph-properties>", attrs(pp), tabs.map(tabs_xml).unwrap_or_default()));
    }
    if !tp.is_empty() {
        s.push_str(&format!("<style:text-properties{}/>", attrs(tp)));
    }
    s
}

/// Properties a paragraph of style `id` inherits (document defaults and the style chain).
fn inherited(doc: &Document, id: Option<&str>) -> ParaProps {
    match id {
        Some(id) => doc.styles.para_style_props(id).0,
        None => doc.styles.default_para.clone(),
    }
}

/// The default paragraph style `st` as `Standard`.
fn standard(ctx: &mut Ctx, st: &Style) -> String {
    let doc = ctx.doc;
    let mut head = format!(" style:name=\"{STANDARD}\" style:display-name=\"{}\" style:family=\"paragraph\"", x(&st.name));
    if let Some(l) = st.para.outline_level.filter(|l| *l < 9) {
        head.push_str(&format!(" style:default-outline-level=\"{}\"", l + 1));
    }
    let mut pp = Props::new();
    let mut tp = Props::new();
    para_props(&st.para, &mut pp, &mut tp);
    complete_spacing(&doc.styles.default_para, &mut pp);
    let tabs = st.para.tabs.is_some().then(|| merged_tabs(&[], doc.styles.para_style_props(&st.id).0.tabs.as_deref().unwrap_or(&[])));
    char_props(ctx, &st.chr, &mut tp);
    format!("<style:style{head}>{}</style:style>", props_xml(&pp, tabs.as_deref(), &tp))
}

/// The contents of `office:styles`: the default style and `Standard`.
pub(super) fn common(ctx: &mut Ctx) -> String {
    let doc = ctx.doc;
    let mut pp = Props::new();
    let mut tp = Props::new();
    para_props(&doc.styles.default_para, &mut pp, &mut tp);
    complete_spacing(&doc.styles.default_para, &mut pp);
    // Widow and orphan control is on unless a document turns it off (as the model resolves it);
    // ODF's default is off.
    if doc.styles.default_para.widow_control.is_none() {
        pp.insert("fo:widows", "2".into());
        pp.insert("fo:orphans", "2".into());
    }
    if doc.settings.default_tab.is_finite() && doc.settings.default_tab > 0.0 {
        pp.insert("style:tab-stop-distance", pt(doc.settings.default_tab.min(1584.0)));
    }
    char_props(ctx, &doc.styles.default_chr, &mut tp);
    tp.insert("fo:hyphenate", on(doc.settings.auto_hyphenation, "true", "false"));
    let default_tabs = doc.styles.default_para.tabs.as_deref().map(|t| merged_tabs(&[], t));
    let mut s = format!("<style:default-style style:family=\"paragraph\">{}</style:default-style>", props_xml(&pp, default_tabs.as_deref(), &tp));
    match doc.styles.styles.iter().find(|st| st.kind == StyleKind::Paragraph && is_default_para(doc, &st.id)) {
        Some(st) => s.push_str(&standard(ctx, st)),
        None => s.push_str(&format!("<style:style style:name=\"{STANDARD}\" style:family=\"paragraph\" style:class=\"text\"/>")),
    }
    s
}

/// The automatic style of paragraph `p` (`Standard` when it has no direct formatting): its
/// direct formatting and `extra` paragraph properties (page breaks of split paragraphs). `mark`
/// writes the paragraph mark's formatting too (an empty paragraph is as high as its mark).
pub(super) fn paragraph(ctx: &mut Ctx, p: &Paragraph, extra: &Props, mark: bool) -> String {
    let doc = ctx.doc;
    let mut pp = Props::new();
    let mut tp = Props::new();
    para_props(&p.props, &mut pp, &mut tp);
    let base = inherited(doc, Some(default_para_id(doc)));
    complete_spacing(&base, &mut pp);
    let tabs = p.props.tabs.as_deref().map(|t| merged_tabs(base.tabs.as_deref().unwrap_or(&[]), t));
    pp.extend(extra.iter().map(|(k, v)| (*k, v.clone())));
    if mark {
        char_props(ctx, &p.mark.formatting(), &mut tp);
    }
    if pp.is_empty() && tabs.is_none() && tp.is_empty() {
        return STANDARD.to_string();
    }
    ctx.auto_style("paragraph", &format!(" style:parent-style-name=\"{STANDARD}\""), &props_xml(&pp, tabs.as_deref(), &tp))
}

/// The automatic text style of a run with formatting `c`, or none.
pub(super) fn text(ctx: &mut Ctx, c: &CharProps) -> Option<String> {
    let mut tp = Props::new();
    char_props(ctx, &c.formatting(), &mut tp);
    (!tp.is_empty()).then(|| ctx.auto_style("text", "", &format!("<style:text-properties{}/>", attrs(&tp))))
}
