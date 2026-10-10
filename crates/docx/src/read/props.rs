//! `w:rPr`, `w:pPr`, table/row/cell and section property readers.

use std::collections::HashMap;

use wordcraft_doc::props::{
    Align, Border, BorderStyle, Borders, CellProps, CharProps, HeightRule, Highlight, LineSpacing, NumRef, ParaProps, Rgb, RowProps, TabAlign,
    TabLeader, TabStop, TableLook, TableProps, TextColor, Underline, VAlign, VMerge, VertAlign,
};
use wordcraft_doc::section::{Columns, LineNumberRestart, LineNumbering, NumFormat, SectionProps, SectionStart};

use crate::units::{int, measure, on_off, tw, u32_of};
use crate::xml::El;

/// Context for property reading: theme fonts and style id aliases.
#[derive(Default)]
pub struct PropCtx {
    pub major_font: String,
    pub minor_font: String,
    /// Style id renames (e.g. a localized default paragraph style id → `Normal`).
    pub alias: HashMap<String, String>,
}

impl PropCtx {
    pub fn style_id(&self, id: &str) -> String {
        self.alias.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    fn theme_font(&self, t: &str) -> Option<String> {
        let f = if t.starts_with("major") { &self.major_font } else { &self.minor_font };
        (!f.is_empty()).then(|| f.clone())
    }

    pub fn rpr(&self, e: &El) -> CharProps {
        let mut c = CharProps::default();
        for k in e.els() {
            match k.name.as_str() {
                "w:rStyle" => c.style = k.attr("w:val").filter(|v| !v.is_empty()).map(|v| self.style_id(v)),
                "w:rFonts" => {
                    c.font = k
                        .attr("w:ascii")
                        .or_else(|| k.attr("w:hAnsi"))
                        .filter(|f| !f.is_empty())
                        .map(str::to_string)
                        .or_else(|| k.attr("w:asciiTheme").or_else(|| k.attr("w:hAnsiTheme")).and_then(|t| self.theme_font(t)))
                        .or_else(|| {
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
                        });
                }
                "w:b" => c.bold = Some(on_off(k)),
                "w:i" => c.italic = Some(on_off(k)),
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
                    c.color = match k.attr("w:val") {
                        Some(v) if v.eq_ignore_ascii_case("auto") => Some(TextColor::Auto),
                        Some(v) => Rgb::parse(v).map(TextColor::Rgb),
                        None => None,
                    }
                }
                "w:spacing" => c.spacing = tw(k, "w:val").map(|v| v.clamp(-1584.0, 1584.0)),
                "w:w" => c.scale = k.attr("w:val").and_then(|v| measure(v.trim_end_matches('%'), 1.0)).map(|v| v.clamp(1.0, 600.0)),
                "w:kern" => c.kern = k.attr("w:val").and_then(|v| measure(v, 2.0)).map(|v| v.clamp(0.0, 1638.0)),
                "w:position" => c.position = k.attr("w:val").and_then(|v| measure(v, 2.0)).map(|v| v.clamp(-1584.0, 1584.0)),
                "w:sz" => c.size = k.attr("w:val").and_then(|v| measure(v, 2.0)).filter(|v| *v > 0.0).map(|v| v.clamp(1.0, 1638.0)),
                "w:highlight" => c.highlight = k.attr("w:val").map(Highlight::from_ooxml),
                "w:u" => {
                    if let Some(v) = k.attr("w:val") {
                        c.underline = Some(Underline::from_ooxml(v));
                    } else {
                        c.underline = Some(Underline::Single);
                    }
                    c.underline_color = k.attr("w:color").and_then(Rgb::parse);
                }
                "w:shd" => c.shading = shd_fill(k),
                "w:vertAlign" => {
                    c.vert_align = k.attr("w:val").map(|v| match v {
                        "superscript" => VertAlign::Superscript,
                        "subscript" => VertAlign::Subscript,
                        _ => VertAlign::Baseline,
                    })
                }
                "w:lang" => c.lang = k.attr("w:val").filter(|v| !v.is_empty() && v.len() < 64).map(str::to_string),
                _ => {}
            }
        }
        c
    }

    /// `w:pPr` (without `w:sectPr`; `w:rPr` is the paragraph mark, returned separately).
    pub fn ppr(&self, e: &El) -> (ParaProps, CharProps) {
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
                "w:suppressAutoHyphens" => p.suppress_hyphens = Some(on_off(k)),
                "w:suppressLineNumbers" => p.suppress_line_numbers = Some(on_off(k)),
                "w:bidi" => p.bidi = Some(on_off(k)),
                "w:framePr" => {
                    if matches!(k.attr("w:dropCap"), Some("drop") | Some("margin")) {
                        p.drop_cap = Some(k.attr("w:lines").and_then(u32_of).unwrap_or(3).clamp(1, 10) as u8);
                    }
                }
                "w:numPr" => {
                    let num = k.child_val("w:numId").and_then(u32_of);
                    let level = k.child_val("w:ilvl").and_then(u32_of).unwrap_or(0).min(8) as u8;
                    if let Some(num) = num {
                        p.numbering = Some(NumRef { num, level });
                    }
                }
                "w:pBdr" => p.borders = Some(borders(k)),
                "w:shd" => p.shading = shd_fill(k),
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
                    if let Some(line) = k.attr("w:line") {
                        let rule = k.attr("w:lineRule").unwrap_or("auto");
                        p.line_spacing = match rule {
                            "exact" => measure(line, 20.0).map(|v| LineSpacing::Exactly(v.abs())),
                            "atLeast" => measure(line, 20.0).map(|v| LineSpacing::AtLeast(v.abs())),
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
                }
                "w:jc" => p.align = k.attr("w:val").map(align),
                "w:outlineLvl" => p.outline_level = k.attr("w:val").and_then(u32_of).map(|v| v.min(9) as u8),
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

/// `w:shd/@w:fill` (a solid fill colour; `auto` = none).
pub fn shd_fill(e: &El) -> Option<Rgb> {
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
fn margins(e: &El) -> [f32; 4] {
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
                "w:shd" => t.shading = shd_fill(k),
                "w:tblLayout" => t.fixed = k.attr("w:type") == Some("fixed"),
                "w:tblCellMar" => t.cell_margins = Some(margins(k)),
                "w:tblLook" => t.look = look(k),
                "w:tblCaption" => t.caption = k.attr("w:val").map(str::to_string),
                "w:tblDescription" if t.caption.is_none() => t.caption = k.attr("w:val").map(str::to_string),
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
                r.height_rule = match k.attr("w:hRule") {
                    Some("exact") => HeightRule::Exact,
                    Some("auto") => HeightRule::Auto,
                    _ => HeightRule::AtLeast,
                };
            }
            "w:tblHeader" => r.header = on_off(k),
            "w:cantSplit" => r.cant_split = on_off(k),
            _ => {}
        }
    }
    r
}

/// `w:tcPr`; returns the props and whether the cell is a legacy `hMerge` continuation.
pub fn tcpr(e: &El) -> (CellProps, bool) {
    let mut c = CellProps { span: 1, ..Default::default() };
    let mut hcont = false;
    for k in e.els() {
        match k.name.as_str() {
            "w:tcW" => {
                if matches!(k.attr("w:type"), None | Some("dxa")) {
                    c.width = tw(k, "w:w").filter(|v| *v > 0.0);
                }
            }
            "w:gridSpan" => c.span = k.attr("w:val").and_then(u32_of).unwrap_or(1).clamp(1, 63),
            "w:hMerge" => hcont = k.attr("w:val") != Some("restart"),
            "w:vMerge" => c.vmerge = if k.attr("w:val") == Some("restart") { VMerge::Restart } else { VMerge::Continue },
            "w:tcBorders" => c.borders = Some(borders(k)),
            "w:shd" => c.shading = shd_fill(k),
            "w:noWrap" => c.no_wrap = on_off(k),
            "w:tcMar" => c.margins = Some(margins(k)),
            "w:textDirection" => c.vertical_text = !matches!(k.attr("w:val"), None | Some("lrTb") | Some("lrTbV") | Some("tb")),
            "w:vAlign" => {
                c.valign = match k.attr("w:val") {
                    Some("center") => VAlign::Center,
                    Some("bottom") => VAlign::Bottom,
                    _ => VAlign::Top,
                }
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
            "w:titlePg" => s.title_page = on_off(k),
            "w:bidi" => s.rtl = on_off(k),
            "w:pgNumType" => {
                s.page_num_start = k.attr("w:start").and_then(u32_of);
                if let Some(f) = k.attr("w:fmt") {
                    s.page_num_format = NumFormat::from_ooxml(f);
                }
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
                    Some("center") | Some("both") => VAlign::Center,
                    Some("bottom") => VAlign::Bottom,
                    _ => VAlign::Top,
                }
            }
            "w:pgBorders" => s.page_borders = Some(borders(k)),
            _ => {}
        }
    }
    (s, refs)
}
