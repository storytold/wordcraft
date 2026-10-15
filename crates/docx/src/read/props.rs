//! `w:rPr`, `w:pPr`, table/row/cell and section property readers.

use std::collections::HashMap;

use wordcraft_doc::para::cap_alt;
use wordcraft_doc::props::{
    Align, Border, BorderStyle, Borders, CellProps, CharProps, HeightRule, Highlight, LineSpacing, NumRef, ParaProps, Rgb, RowProps, TabAlign,
    TabLeader, TabStop, TableFloat, TableLook, TableProps, TextColor, TextDirection, Underline, VAlign, VMerge, VertAlign,
};
use wordcraft_doc::section::{
    Columns, DocGrid, DocGridType, LineNumberRestart, LineNumbering, MAX_CHAR_SPACE, MAX_GRID_PITCH, NotePos, NoteProps, NoteRestart, NumFormat,
    PageVAlign, SectionProps, SectionStart,
};

use super::drawing_color::adjust_lum;
use crate::units::{int, measure, on_off, tw, u32_of};

/// A text frame (`w:framePr` without a drop cap, ECMA-376 §17.3.1.11), sizes and offsets in
/// points (clamped), names kept as OOXML writes them.
fn text_frame(k: &El) -> wordcraft_doc::props::TextFrame {
    let len = |name: &str| tw(k, name).filter(|v| v.is_finite()).map(|v| v.clamp(-1584.0, 1584.0));
    let word = |name: &str, allowed: &[&str], default: &str| k.attr(name).filter(|v| allowed.contains(v)).unwrap_or(default).to_string();
    let align = |name: &str, allowed: &[&str]| k.attr(name).filter(|v| allowed.contains(v)).map(str::to_string);
    wordcraft_doc::props::TextFrame {
        w: len("w:w").filter(|v| *v > 0.0),
        h: len("w:h").map(f32::abs).filter(|v| *v > 0.0),
        h_exact: k.attr("w:hRule") == Some("exact"),
        x: len("w:x").unwrap_or(0.0),
        y: len("w:y").unwrap_or(0.0),
        x_align: align("w:xAlign", &["left", "center", "right", "inside", "outside"]),
        y_align: align("w:yAlign", &["top", "center", "bottom", "inside", "outside"]),
        h_anchor: word("w:hAnchor", &["text", "margin", "page"], "page"),
        v_anchor: word("w:vAnchor", &["text", "margin", "page"], "page"),
        wrap: word("w:wrap", &["around", "auto", "notBeside", "none", "tight", "through"], "auto"),
        h_space: len("w:hSpace").unwrap_or(0.0).max(0.0),
        v_space: len("w:vSpace").unwrap_or(0.0).max(0.0),
    }
}
use crate::xml::El;

/// Context for property reading: theme fonts and colours, and style id aliases.
#[derive(Default)]
pub struct PropCtx {
    pub major_font: String,
    pub minor_font: String,
    /// The theme's complex-script fonts (`a:cs`), which the `…Bidi` theme slots name.
    pub major_cs: String,
    pub minor_cs: String,
    /// The theme's colour scheme (dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink); empty when the
    /// package has none, and then `w:themeColor` falls back to the fixed colour.
    pub theme_colors: Vec<Rgb>,
    /// `w:clrSchemeMapping`: the scheme slot each of text1, background1, text2, background2,
    /// accent1..6, hyperlink and followedHyperlink uses. `None` = the default (same order).
    pub color_map: Option<[usize; 12]>,
    /// Style id renames (e.g. a localized default paragraph style id → `Normal`).
    pub alias: HashMap<String, String>,
    /// The theme's colours (for WordArt effects in theme colours).
    pub theme: Vec<Rgb>,
}

/// A `w:numPr` that gives a level (`w:ilvl`) and no list (`w:numId`): the list is the one the
/// paragraph's style chain uses (ECMA-376 §17.3.1.19). A placeholder while reading, never left in
/// a document (`resolve_list_levels`).
pub(crate) const LEVEL_ONLY: u32 = u32::MAX;

/// The list a level-only `numPr` refers to: the nearest numbering in style `style`'s based-on chain
/// (its own included) that names a list. `None` when no style in the chain has one.
pub(crate) fn chain_list(sheet: &wordcraft_doc::styles::StyleSheet, style: &str) -> Option<u32> {
    sheet.chain(style).iter().rev().filter_map(|s| s.para.numbering).map(|n| n.num).find(|n| *n != LEVEL_ONLY)
}

/// Resolve level-only numbering in `p` against its style chain (`style` when it has none).
pub(crate) fn resolve_level_only(sheet: &wordcraft_doc::styles::StyleSheet, p: &mut ParaProps, style: &str) {
    if let Some(n) = p.numbering.as_mut().filter(|n| n.num == LEVEL_ONLY) {
        match chain_list(sheet, p.style.as_deref().unwrap_or(style)) {
            Some(num) => n.num = num,
            None => p.numbering = None,
        }
    }
}

impl PropCtx {
    pub fn style_id(&self, id: &str) -> String {
        self.alias.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    fn theme_font(&self, t: &str) -> Option<String> {
        let (latin, cs) = if t.starts_with("major") { (&self.major_font, &self.major_cs) } else { (&self.minor_font, &self.minor_cs) };
        // Measured in Word: `minorBidi`/`majorBidi` name the complex-script font, and an empty one
        // names nothing, so the font beside it or the inherited one applies.
        let f = if t.ends_with("Bidi") { cs } else { latin };
        (!f.is_empty()).then(|| f.clone())
    }

    /// The theme colour `e` names in `attr` (`w:themeColor`, `w:themeFill`), through the colour
    /// scheme mapping, with its `tint` and `shade` (hex bytes) applied to the HSL luminance as
    /// Word does. When given it supersedes the fixed colour; `None` leaves the fixed one.
    fn theme_color(&self, e: &El, attr: &str, tint: &str, shade: &str) -> Option<Rgb> {
        let logical = |i: usize| self.color_map.and_then(|m| m.get(i).copied()).unwrap_or(i);
        let slot = match e.attr(attr)? {
            "dark1" => 0,
            "light1" => 1,
            "dark2" => 2,
            "light2" => 3,
            "text1" => logical(0),
            "background1" => logical(1),
            "text2" => logical(2),
            "background2" => logical(3),
            "accent1" => logical(4),
            "accent2" => logical(5),
            "accent3" => logical(6),
            "accent4" => logical(7),
            "accent5" => logical(8),
            "accent6" => logical(9),
            "hyperlink" => logical(10),
            "followedHyperlink" => logical(11),
            _ => return None,
        };
        let mut c = *self.theme_colors.get(slot)?;
        let byte = |a: &str| e.attr(a).and_then(|v| u8::from_str_radix(v.trim(), 16).ok()).map(|v| f64::from(v) / 255.0);
        if let Some(t) = byte(tint) {
            c = adjust_lum(c, |l| l * t + (1.0 - t));
        }
        if let Some(s) = byte(shade) {
            c = adjust_lum(c, |l| l * s);
        }
        Some(c)
    }

    /// `w:shd`: its fill colour (a theme fill first), or a solid pattern's colour.
    pub fn shd_fill(&self, e: &El) -> Option<Rgb> {
        if let Some(c) = self.theme_color(e, "w:themeFill", "w:themeFillTint", "w:themeFillShade") {
            return Some(c);
        }
        // A solid pattern without a fill is painted in its pattern colour, a theme one first.
        if e.attr("w:val") == Some("solid")
            && e.attr("w:fill").and_then(Rgb::parse).is_none()
            && let Some(c) = self.theme_color(e, "w:themeColor", "w:themeTint", "w:themeShade")
        {
            return Some(c);
        }
        shd_fill(e)
    }

    pub fn rpr(&self, e: &El) -> CharProps {
        let mut c = CharProps::default();
        for k in e.els() {
            match k.name.as_str() {
                "w:rStyle" => c.style = k.attr("w:val").filter(|v| !v.is_empty()).map(|v| self.style_id(v)),
                "w:rFonts" => {
                    // A theme font supersedes the fixed font beside it (`w:asciiTheme` over
                    // `w:ascii`, `w:hAnsiTheme` over `w:hAnsi`).
                    let named = |a: &str| k.attr(a).filter(|f| !f.is_empty()).map(str::to_string);
                    let themed = |a: &str| k.attr(a).and_then(|t| self.theme_font(t));
                    c.font =
                        themed("w:asciiTheme").or_else(|| named("w:ascii")).or_else(|| themed("w:hAnsiTheme")).or_else(|| named("w:hAnsi")).or_else(
                            || {
                                // `w:eastAsia` and `w:cs` name fonts for other scripts; they stand in for the
                                // Latin font only when the run is hinted as that script. Otherwise the Latin
                                // font is inherited (a Normal style with only `w:eastAsia`/`w:cs` keeps the
                                // `docDefaults` font, as in Word).
                                match k.attr("w:hint") {
                                    Some("eastAsia") => k.attr("w:eastAsia"),
                                    Some("cs") => k.attr("w:cs"),
                                    _ => None,
                                }
                                .filter(|f| !f.is_empty())
                                .map(str::to_string)
                            },
                        );
                    // The complex-script font (Persian, Arabic, Hebrew text). A `w:cstheme` font is
                    // left to inherit: the theme's per-script fonts aren't read.
                    c.font_cs = k.attr("w:cs").filter(|f| !f.is_empty() && f.len() < 256).map(str::to_string);
                    // The East Asian font (Chinese, Japanese, Korean text). A `w:eastAsiaTheme` font
                    // is left to inherit, like `w:cstheme`.
                    c.font_ea = k.attr("w:eastAsia").filter(|f| !f.is_empty() && f.len() < 256).map(str::to_string);
                }
                "w:b" => c.bold = Some(on_off(k)),
                "w:bCs" => c.bold_cs = Some(on_off(k)),
                "w:i" => c.italic = Some(on_off(k)),
                "w:iCs" => c.italic_cs = Some(on_off(k)),
                "w:cs" => c.cs = Some(on_off(k)),
                "w:caps" => c.caps = Some(on_off(k)),
                "w:smallCaps" => c.small_caps = Some(on_off(k)),
                "w:strike" => c.strike = Some(on_off(k)),
                "w:dstrike" => c.double_strike = Some(on_off(k)),
                "w:outline" => c.outline = Some(on_off(k)),
                "w:shadow" => c.shadow = Some(on_off(k)),
                "w:emboss" => c.emboss = Some(on_off(k)),
                "w:imprint" => c.engrave = Some(on_off(k)),
                "w:noProof" => c.no_proof = Some(on_off(k)),
                "w:vanish" => c.hidden = Some(on_off(k)),
                "w:rtl" => c.rtl = Some(on_off(k)),
                "w:color" => {
                    c.color = match (self.theme_color(k, "w:themeColor", "w:themeTint", "w:themeShade"), k.attr("w:val")) {
                        (Some(t), _) => Some(TextColor::Rgb(t)),
                        (None, Some(v)) if v.eq_ignore_ascii_case("auto") => Some(TextColor::Auto),
                        (None, Some(v)) => Rgb::parse(v).map(TextColor::Rgb),
                        (None, None) => None,
                    }
                }
                "w:spacing" => c.spacing = tw(k, "w:val").map(|v| v.clamp(-1584.0, 1584.0)),
                "w:w" => c.scale = k.attr("w:val").and_then(|v| measure(v.trim_end_matches('%'), 1.0)).map(|v| v.clamp(1.0, 600.0)),
                "w:kern" => c.kern = k.attr("w:val").and_then(|v| measure(v, 2.0)).map(|v| v.clamp(0.0, 1638.0)),
                "w:position" => c.position = k.attr("w:val").and_then(|v| measure(v, 2.0)).map(|v| v.clamp(-1584.0, 1584.0)),
                "w:sz" => c.size = k.attr("w:val").and_then(|v| measure(v, 2.0)).filter(|v| *v > 0.0).map(|v| v.clamp(1.0, 1638.0)),
                "w:szCs" => c.size_cs = k.attr("w:val").and_then(|v| measure(v, 2.0)).filter(|v| *v > 0.0).map(|v| v.clamp(1.0, 1638.0)),
                "w:highlight" => c.highlight = k.attr("w:val").map(Highlight::from_ooxml),
                "w:u" => {
                    if let Some(v) = k.attr("w:val") {
                        c.underline = Some(Underline::from_ooxml(v));
                    } else {
                        c.underline = Some(Underline::Single);
                    }
                    c.underline_color =
                        self.theme_color(k, "w:themeColor", "w:themeTint", "w:themeShade").or_else(|| k.attr("w:color").and_then(Rgb::parse));
                }
                "w:bdr" => c.border = Some(border(k)),
                "w:shd" => c.shading = self.shd_fill(k),
                "w:vertAlign" => {
                    c.vert_align = k.attr("w:val").map(|v| match v {
                        "superscript" => VertAlign::Superscript,
                        "subscript" => VertAlign::Subscript,
                        _ => VertAlign::Baseline,
                    })
                }
                "w:lang" => {
                    c.lang = k.attr("w:val").filter(|v| !v.is_empty() && v.len() < 64).map(str::to_string);
                    c.lang_bidi = k.attr("w:bidi").filter(|v| !v.is_empty() && v.len() < 64).map(str::to_string);
                }
                n if n.starts_with("w14:") => {
                    let fx = c.text_effects.get_or_insert_with(Default::default);
                    super::wordart::read(k, &self.theme, fx);
                    if fx.is_empty() {
                        c.text_effects = None;
                    }
                }
                _ => {}
            }
        }
        c
    }

    /// `w:pPr` (without `w:sectPr`; `w:rPr` is the paragraph mark, returned separately).
    pub fn ppr(&self, e: &El) -> (ParaProps, CharProps) {
        // (`w:numPr` with a level and no list sets `LEVEL_ONLY`, resolved by the caller.)
        let mut p = ParaProps::default();
        let mut mark = CharProps::default();
        for k in e.els() {
            match k.name.as_str() {
                "w:pStyle" => p.style = k.attr("w:val").filter(|v| !v.is_empty()).map(|v| self.style_id(v)),
                "w:keepNext" => p.keep_next = Some(on_off(k)),
                "w:keepLines" => p.keep_lines = Some(on_off(k)),
                "w:pageBreakBefore" => p.page_break_before = Some(on_off(k)),
                "w:widowControl" => p.widow_control = Some(on_off(k)),
                "w:contextualSpacing" => p.contextual_spacing = Some(on_off(k)),
                "w:mirrorIndents" => p.mirror_indents = Some(on_off(k)),
                "w:suppressAutoHyphens" => p.suppress_hyphens = Some(on_off(k)),
                "w:suppressLineNumbers" => p.suppress_line_numbers = Some(on_off(k)),
                "w:bidi" => p.bidi = Some(on_off(k)),
                "w:kinsoku" => p.kinsoku = Some(on_off(k)),
                "w:wordWrap" => p.word_wrap = Some(on_off(k)),
                "w:overflowPunct" => p.overflow_punct = Some(on_off(k)),
                "w:topLinePunct" => p.top_line_punct = Some(on_off(k)),
                "w:autoSpaceDE" => p.auto_space_de = Some(on_off(k)),
                "w:autoSpaceDN" => p.auto_space_dn = Some(on_off(k)),
                "w:snapToGrid" => p.snap_to_grid = Some(on_off(k)),
                "w:framePr" => {
                    if matches!(k.attr("w:dropCap"), Some("drop") | Some("margin")) {
                        p.drop_cap = Some(k.attr("w:lines").and_then(u32_of).unwrap_or(3).clamp(1, 10) as u8);
                    } else {
                        p.frame = Some(text_frame(k));
                    }
                }
                "w:numPr" => {
                    let num = k.child_val("w:numId").and_then(u32_of);
                    let ilvl = k.child_val("w:ilvl").and_then(u32_of);
                    let level = ilvl.unwrap_or(0).min(8) as u8;
                    match (num, ilvl) {
                        (Some(num), _) => p.numbering = Some(NumRef { num, level }),
                        // A level without a list: the list comes from the based-on style, as in
                        // a Heading 3 that only says "level 3" of Heading 2's list. Resolved once
                        // the styles are read (`resolve_list_levels`).
                        (None, Some(_)) => p.numbering = Some(NumRef { num: LEVEL_ONLY, level }),
                        (None, None) => {}
                    }
                }
                "w:pBdr" => p.borders = Some(borders(k)),
                "w:shd" => p.shading = self.shd_fill(k),
                "w:tabs" => {
                    let tabs: Vec<TabStop> = k.children("w:tab").take(64).filter_map(tab_stop).collect();
                    p.tabs = Some(tabs);
                }
                "w:spacing" => {
                    if let Some(v) = tw(k, "w:before") {
                        p.space_before = Some(v.max(0.0));
                    }
                    if let Some(v) = tw(k, "w:after") {
                        p.space_after = Some(v.max(0.0));
                    }
                    // HTML-style automatic spacing; "0" switches an inherited one off.
                    let auto = |name| k.attr(name).map(|v| matches!(v.trim(), "1" | "true" | "on"));
                    if let Some(a) = auto("w:beforeAutospacing") {
                        p.space_before_auto = Some(a);
                    }
                    if let Some(a) = auto("w:afterAutospacing") {
                        p.space_after_auto = Some(a);
                    }
                    if let Some(line) = k.attr("w:line") {
                        // Word reads the rule case-insensitively (`atleast` is seen in the wild).
                        let rule = k.attr("w:lineRule").unwrap_or("auto").to_ascii_lowercase();
                        p.line_spacing = match rule.as_str() {
                            "exact" => measure(line, 20.0).map(|v| LineSpacing::Exactly(v.abs())),
                            "atleast" => measure(line, 20.0).map(|v| LineSpacing::AtLeast(v.abs())),
                            _ => measure(line, 240.0).map(|v| LineSpacing::Multiple(v.abs().clamp(0.06, 132.0))),
                        };
                    }
                }
                "w:ind" => {
                    if let Some(v) = tw(k, "w:left").or_else(|| tw(k, "w:start")) {
                        p.indent_left = Some(v);
                    }
                    if let Some(v) = tw(k, "w:right").or_else(|| tw(k, "w:end")) {
                        p.indent_right = Some(v);
                    }
                    if let Some(v) = tw(k, "w:hanging") {
                        p.indent_first = Some(-v);
                    } else if let Some(v) = tw(k, "w:firstLine") {
                        p.indent_first = Some(v);
                    }
                    // Character units (hundredths of a character), which win when non-zero.
                    let chars = |a: &str| k.attr(a).and_then(int).map(|v| v.clamp(-158_400, 158_400) as f32 / 100.0);
                    if let Some(v) = chars("w:leftChars").or_else(|| chars("w:startChars")) {
                        p.indent_left_chars = Some(v);
                    }
                    if let Some(v) = chars("w:rightChars").or_else(|| chars("w:endChars")) {
                        p.indent_right_chars = Some(v);
                    }
                    match (chars("w:hangingChars"), chars("w:firstLineChars")) {
                        (Some(h), f) if h != 0.0 || f.is_none() => p.indent_first_chars = Some(-h),
                        (_, Some(f)) => p.indent_first_chars = Some(f),
                        _ => {}
                    }
                }
                "w:jc" => p.align = k.attr("w:val").map(align),
                "w:outlineLvl" => p.outline_level = k.attr("w:val").and_then(u32_of).map(|v| v.min(9) as u8),
                // [MS-DOCX] "Collapsed by default" (an on/off with `w:val`; `w15:val` seen too).
                "w15:collapsed" => {
                    p.collapsed = Some(k.attr("w:val").or_else(|| k.attr("w15:val")).is_none_or(|v| !matches!(v.trim(), "0" | "false" | "off")))
                }
                "w:rPr" => mark = self.rpr(k),
                _ => {}
            }
        }
        (p, mark)
    }
}

pub fn align(v: &str) -> Align {
    match v {
        "center" => Align::Center,
        "right" | "end" => Align::Right,
        "both" | "justify" | "lowKashida" | "mediumKashida" | "highKashida" => Align::Justify,
        "distribute" | "thaiDistribute" => Align::Distribute,
        _ => Align::Left,
    }
}

fn tab_stop(t: &El) -> Option<TabStop> {
    let pos = tw(t, "w:pos")?;
    let align = match t.attr("w:val").unwrap_or("left") {
        "center" => TabAlign::Center,
        "right" | "end" => TabAlign::Right,
        "decimal" => TabAlign::Decimal,
        "bar" => TabAlign::Bar,
        "clear" => TabAlign::Clear,
        _ => TabAlign::Left,
    };
    let leader = match t.attr("w:leader").unwrap_or("none") {
        "dot" => TabLeader::Dot,
        "hyphen" => TabLeader::Hyphen,
        "underscore" | "heavy" => TabLeader::Underscore,
        "middleDot" => TabLeader::MiddleDot,
        _ => TabLeader::None,
    };
    Some(TabStop { pos, align, leader })
}

/// `w:shd/@w:fill` (a solid fill colour; `auto` = none), ignoring theme fills ([`PropCtx::shd_fill`]).
fn shd_fill(e: &El) -> Option<Rgb> {
    let fill = e.attr("w:fill").and_then(Rgb::parse);
    if fill.is_some() {
        return fill;
    }
    // A solid pattern uses the pattern colour.
    if e.attr("w:val") == Some("solid") {
        return e.attr("w:color").and_then(Rgb::parse);
    }
    None
}

pub fn border(e: &El) -> Border {
    let style = BorderStyle::from_ooxml(e.attr("w:val").unwrap_or("single"));
    let width = e.attr("w:sz").and_then(|v| measure(v, 8.0)).unwrap_or(0.5).clamp(0.0, 12.0);
    let color = e.attr("w:color").and_then(Rgb::parse);
    let space = e.attr("w:space").and_then(|v| measure(v, 1.0)).unwrap_or(0.0).clamp(0.0, 31.0);
    Border { style, width, color, space }
}

/// `w:pBdr`, `w:tblBorders`, `w:tcBorders`, `w:pgBorders`.
pub fn borders(e: &El) -> Borders {
    let mut b = Borders::default();
    for k in e.els() {
        match k.local() {
            "top" => b.top = Some(border(k)),
            "left" | "start" => b.left = Some(border(k)),
            "bottom" => b.bottom = Some(border(k)),
            "right" | "end" => b.right = Some(border(k)),
            "between" | "insideH" => b.between = Some(border(k)),
            "insideV" => b.inside_v = Some(border(k)),
            _ => {}
        }
    }
    b
}

/// Margins element (`w:tblCellMar`, `w:tcMar`) → [top, left, bottom, right].
pub fn margins(e: &El) -> [f32; 4] {
    let get = |names: &[&str]| names.iter().find_map(|n| e.child(n)).and_then(|c| tw(c, "w:w")).unwrap_or(0.0).clamp(0.0, 1584.0);
    [get(&["w:top"]), get(&["w:left", "w:start"]), get(&["w:bottom"]), get(&["w:right", "w:end"])]
}

impl PropCtx {
    pub fn tblpr(&self, e: &El) -> TableProps {
        let mut t = TableProps::default();
        for k in e.els() {
            match k.name.as_str() {
                "w:tblStyle" => t.style = k.attr("w:val").filter(|v| !v.is_empty()).map(|v| self.style_id(v)),
                "w:tblW" => {
                    let ty = k.attr("w:type").unwrap_or("dxa");
                    let w = k.attr("w:w").unwrap_or("0");
                    match ty {
                        "pct" => {
                            t.width_pct = if let Some(p) = w.strip_suffix('%') { measure(p, 1.0) } else { measure(w, 50.0) }
                                .map(|v| v.clamp(0.0, 1000.0))
                                .filter(|v| *v > 0.0)
                        }
                        "auto" | "nil" => {}
                        _ => t.width = measure(w, 20.0).filter(|v| *v > 0.0),
                    }
                }
                "w:jc" => t.align = k.attr("w:val").map(align),
                "w:tblInd" => t.indent = tw(k, "w:w"),
                "w:tblBorders" => t.borders = Some(borders(k)),
                "w:shd" => t.shading = self.shd_fill(k),
                "w:tblLayout" => t.fixed = k.attr("w:type") == Some("fixed"),
                "w:bidiVisual" => t.bidi_visual = on_off(k),
                "w:tblCellMar" => t.cell_margins = Some(margins(k)),
                "w:tblLook" => t.look = look(k),
                "w:tblpPr" => t.float = Some(table_float(k)),
                "w:tblOverlap" => {
                    if let Some(f) = t.float.as_mut() {
                        f.overlap = k.attr("w:val") != Some("never");
                    }
                }
                "w:tblCaption" => t.caption = k.attr("w:val").map(cap_alt),
                "w:tblDescription" => t.description = k.attr("w:val").map(cap_alt),
                _ => {}
            }
        }
        t
    }
}

/// `w:tblLook` in either the attribute form or the legacy hex bitmask form.
pub fn look(e: &El) -> TableLook {
    let mut l = TableLook::default();
    if let Some(v) = e.attr("w:val") {
        let bits = u32::from_str_radix(v.trim(), 16).unwrap_or(0x04A0);
        l.header_row = bits & 0x0020 != 0;
        l.total_row = bits & 0x0040 != 0;
        l.first_column = bits & 0x0080 != 0;
        l.last_column = bits & 0x0100 != 0;
        l.banded_rows = bits & 0x0200 == 0;
        l.banded_columns = bits & 0x0400 == 0;
    }
    let b = |name: &str| e.attr(name).map(|v| !matches!(v, "0" | "false" | "off"));
    if let Some(v) = b("w:firstRow") {
        l.header_row = v;
    }
    if let Some(v) = b("w:lastRow") {
        l.total_row = v;
    }
    if let Some(v) = b("w:firstColumn") {
        l.first_column = v;
    }
    if let Some(v) = b("w:lastColumn") {
        l.last_column = v;
    }
    if let Some(v) = b("w:noHBand") {
        l.banded_rows = !v;
    }
    if let Some(v) = b("w:noVBand") {
        l.banded_columns = !v;
    }
    l
}

pub fn trpr(e: &El) -> RowProps {
    let mut r = RowProps::default();
    for k in e.els() {
        match k.name.as_str() {
            "w:trHeight" => {
                r.height = tw(k, "w:val").map(|v| v.abs());
                r.height_rule = match k.attr("w:hRule").map(str::to_ascii_lowercase).as_deref() {
                    Some("exact") => HeightRule::Exact,
                    Some("auto") => HeightRule::Auto,
                    _ => HeightRule::AtLeast,
                };
            }
            "w:tblHeader" => r.header = on_off(k),
            "w:cantSplit" => r.cant_split = on_off(k),
            "w:gridBefore" => r.grid_before = k.attr("w:val").and_then(u32_of).unwrap_or(0).min(63),
            "w:gridAfter" => r.grid_after = k.attr("w:val").and_then(u32_of).unwrap_or(0).min(63),
            "w:wBefore" => r.w_before = gap_width(k),
            "w:wAfter" => r.w_after = gap_width(k),
            _ => {}
        }
    }
    r
}

/// `w:wBefore`/`w:wAfter` in points: only a width in twips (`dxa`) is kept.
fn gap_width(k: &El) -> Option<f32> {
    match k.attr("w:type") {
        None | Some("dxa") => tw(k, "w:w").filter(|v| *v >= 0.0).map(|v| v.min(crate::units::MAX_LEN_PT)),
        _ => None,
    }
}

/// `w:tcPr`; returns the props and whether the cell is a legacy `hMerge` continuation.
pub fn tcpr(pc: &PropCtx, e: &El) -> (CellProps, bool) {
    let mut c = CellProps { span: 1, ..Default::default() };
    let mut hcont = false;
    for k in e.els() {
        match k.name.as_str() {
            "w:tcW" => match k.attr("w:type") {
                None | Some("dxa") => c.width = tw(k, "w:w").filter(|v| *v > 0.0),
                Some("pct") => {
                    let w = k.attr("w:w").unwrap_or("0");
                    c.width_pct = if let Some(p) = w.strip_suffix('%') { measure(p, 1.0) } else { measure(w, 50.0) }
                        .map(|v| v.clamp(0.0, 100.0))
                        .filter(|v| *v > 0.0);
                }
                _ => {}
            },
            "w:gridSpan" => c.span = k.attr("w:val").and_then(u32_of).unwrap_or(1).clamp(1, 63),
            "w:hMerge" => hcont = k.attr("w:val") != Some("restart"),
            "w:vMerge" => c.vmerge = if k.attr("w:val") == Some("restart") { VMerge::Restart } else { VMerge::Continue },
            "w:tcBorders" => c.borders = Some(borders(k)),
            "w:shd" => c.shading = pc.shd_fill(k),
            "w:noWrap" => c.no_wrap = on_off(k),
            "w:hideMark" => c.hide_mark = on_off(k),
            "w:tcMar" => c.margins = Some(margins(k)),
            "w:textDirection" => {
                c.text_direction = TextDirection::from_ooxml(k.attr("w:val").unwrap_or(""));
                c.text_direction_set = k.attr("w:val") == Some("lrTb");
            }
            "w:vAlign" => {
                c.valign = Some(match k.attr("w:val") {
                    Some("center") => VAlign::Center,
                    Some("bottom") => VAlign::Bottom,
                    _ => VAlign::Top,
                })
            }
            _ => {}
        }
    }
    (c, hcont)
}

/// A header/footer reference found in a `w:sectPr`: (is_footer, type, relationship id).
pub struct HfRef {
    pub footer: bool,
    pub kind: String,
    pub rid: String,
}

/// `w:sectPr` (header/footer references are returned for the caller to resolve).
pub fn sectpr(e: &El) -> (SectionProps, Vec<HfRef>) {
    let mut s = SectionProps::default();
    let mut refs = Vec::new();
    for k in e.els() {
        match k.name.as_str() {
            "w:headerReference" | "w:footerReference" => {
                if let Some(rid) = k.attr("r:id") {
                    refs.push(HfRef {
                        footer: k.name == "w:footerReference",
                        kind: k.attr("w:type").unwrap_or("default").to_string(),
                        rid: rid.to_string(),
                    });
                }
            }
            "w:type" => {
                s.start = match k.attr("w:val") {
                    Some("continuous") => SectionStart::Continuous,
                    Some("evenPage") => SectionStart::EvenPage,
                    Some("oddPage") => SectionStart::OddPage,
                    Some("nextColumn") => SectionStart::NextColumn,
                    _ => SectionStart::NextPage,
                }
            }
            "w:pgSz" => {
                if let Some(w) = tw(k, "w:w").filter(|v| *v >= 36.0) {
                    s.page_w = w;
                }
                if let Some(h) = tw(k, "w:h").filter(|v| *v >= 36.0) {
                    s.page_h = h;
                }
                s.landscape = match k.attr("w:orient") {
                    Some("landscape") => true,
                    Some(_) => false,
                    None => s.page_w > s.page_h,
                };
            }
            "w:pgMar" => {
                let m = |n: &str, d: f32| tw(k, n).map(f32::abs).unwrap_or(d).min(s.page_w.max(s.page_h));
                s.margin_top = m("w:top", s.margin_top);
                s.margin_bottom = m("w:bottom", s.margin_bottom);
                s.margin_left = m("w:left", s.margin_left);
                s.margin_right = m("w:right", s.margin_right);
                s.header = m("w:header", s.header);
                s.footer = m("w:footer", s.footer);
                s.gutter = m("w:gutter", s.gutter);
            }
            "w:cols" => {
                let mut c = Columns { count: k.attr("w:num").and_then(u32_of).unwrap_or(1).clamp(1, 45), ..Columns::default() };
                if let Some(sp) = tw(k, "w:space") {
                    c.space = sp.max(0.0);
                }
                c.separator = k.attr("w:sep").is_some_and(|v| !matches!(v, "0" | "false" | "off"));
                let equal = k.attr("w:equalWidth").is_none_or(|v| !matches!(v, "0" | "false" | "off"));
                if !equal {
                    c.widths = k
                        .children("w:col")
                        .take(45)
                        .map(|col| (tw(col, "w:w").unwrap_or(0.0).max(0.0), tw(col, "w:space").unwrap_or(0.0).max(0.0)))
                        .collect();
                    if !c.widths.is_empty() {
                        c.count = c.widths.len() as u32;
                    }
                }
                s.columns = c;
            }
            "w:footnotePr" => s.footnote_pr = note_pr(k, true),
            "w:endnotePr" => s.endnote_pr = note_pr(k, true),
            "w:formProt" => s.form_prot = Some(on_off(k)),
            "w:titlePg" => s.title_page = on_off(k),
            "w:bidi" => s.rtl = on_off(k),
            "w:textDirection" => {
                s.text_direction = wordcraft_doc::props::TextDirection::from_ooxml(k.attr("w:val").unwrap_or(""));
                s.text_direction_set = k.attr("w:val") == Some("lrTb");
            }
            "w:pgNumType" => {
                s.page_num_start = k.attr("w:start").and_then(u32_of);
                if let Some(f) = k.attr("w:fmt") {
                    s.page_num_format = NumFormat::from_ooxml(f);
                }
                s.page_num_set = s.page_num_start.is_none() && s.page_num_format == NumFormat::Decimal;
            }
            "w:lnNumType" => {
                let mut l = LineNumbering::default();
                if let Some(v) = k.attr("w:countBy").and_then(u32_of) {
                    l.count_by = v.clamp(1, 100);
                }
                if let Some(v) = k.attr("w:start").and_then(int) {
                    l.start = (v.clamp(0, 32_767) + 1) as u32;
                }
                l.distance = tw(k, "w:distance").unwrap_or(0.0).max(0.0);
                l.restart = match k.attr("w:restart") {
                    Some("newSection") => LineNumberRestart::Section,
                    Some("continuous") => LineNumberRestart::Continuous,
                    _ => LineNumberRestart::Page,
                };
                s.line_numbers = Some(l);
            }
            "w:vAlign" => {
                s.valign = match k.attr("w:val") {
                    Some("center") => PageVAlign::Center,
                    Some("both") => PageVAlign::Justify,
                    Some("bottom") => PageVAlign::Bottom,
                    _ => PageVAlign::Top,
                }
            }
            "w:pgBorders" => s.page_borders = Some(borders(k)),
            "w:docGrid" => {
                let mut g = DocGrid::default();
                if let Some(t) = k.attr("w:type").and_then(DocGridType::from_ooxml) {
                    g.kind = t;
                }
                if let Some(p) = tw(k, "w:linePitch").filter(|v| v.is_finite() && *v > 0.0) {
                    g.line_pitch = p.min(MAX_GRID_PITCH);
                }
                if let Some(c) = k.attr("w:charSpace").and_then(int) {
                    g.char_space = c.clamp(-(MAX_CHAR_SPACE as i64), MAX_CHAR_SPACE as i64) as i32;
                }
                s.doc_grid = Some(g);
            }
            _ => {}
        }
    }
    (s, refs)
}

/// `w:footnotePr` / `w:endnotePr` (ECMA-376 §17.11): placement, number format (only kept when
/// `fmt`: the document's goes to its own setting), start and restart. A start out of range is
/// clamped; unknown values are left unset.
pub fn note_pr(e: &El, fmt: bool) -> NoteProps {
    NoteProps {
        pos: e.child_val("w:pos").and_then(NotePos::from_ooxml),
        num_fmt: if fmt { e.child_val("w:numFmt").map(NumFormat::from_ooxml) } else { None },
        num_start: e.child_val("w:numStart").and_then(int).map(|v| v.clamp(1, wordcraft_doc::section::MAX_NOTE_START as i64) as u32),
        num_restart: e.child_val("w:numRestart").and_then(NoteRestart::from_ooxml),
    }
}

/// `w:tblpPr`: a floating table's anchors, offsets or alignments and distances from text.
fn table_float(e: &El) -> TableFloat {
    use wordcraft_doc::para::{Anchor, FloatAlign};
    let pt = |n: &str| e.attr(n).and_then(|v| measure(v, 20.0)).map(|v| v.clamp(-31_680.0, 31_680.0)).unwrap_or(0.0);
    let dist = |n: &str| pt(n).clamp(0.0, 1584.0);
    let rel = |n: &str, text: Anchor| match e.attr(n) {
        Some("page") => Anchor::Page,
        Some("margin") => Anchor::Margin,
        _ => text,
    };
    let align = |n: &str| e.attr(n).and_then(FloatAlign::from_ooxml);
    TableFloat {
        h_rel: rel("w:horzAnchor", Anchor::Column),
        v_rel: rel("w:vertAnchor", Anchor::Paragraph),
        x: pt("w:tblpX"),
        y: pt("w:tblpY"),
        h_align: align("w:tblpXSpec"),
        v_align: align("w:tblpYSpec"),
        dist: [dist("w:leftFromText"), dist("w:topFromText"), dist("w:rightFromText"), dist("w:bottomFromText")],
        overlap: true,
    }
}
