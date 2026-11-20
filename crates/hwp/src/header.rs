//! The header part (`hh:head`): fonts, character and paragraph shapes, styles, border fills,
//! numbering, bullets and tab definitions.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::xml::Element;

pub const LANGS: [&str; 7] = ["hangul", "latin", "hanja", "japanese", "other", "symbol", "user"];
/// The same languages as hh:fontface writes them.
pub(crate) const FONT_LANGS: [&str; 7] = ["HANGUL", "LATIN", "HANJA", "JAPANESE", "OTHER", "SYMBOL", "USER"];

/// "1" / "true" (any case, trimmed).
pub fn truthy(v: Option<&str>) -> bool {
    matches!(v.map(|s| s.trim().to_ascii_lowercase()).as_deref(), Some("1" | "true"))
}

/// An integer attribute written as a number (a decimal part is cut off); `default` when missing
/// or not a number.
pub fn num_or(v: Option<&str>, default: i64) -> i64 {
    match v {
        None | Some("") => default,
        Some(s) => match s.trim().parse::<f64>() {
            Ok(f) if f.is_finite() => f.trunc().clamp(i64::MIN as f64, i64::MAX as f64) as i64,
            _ => default,
        },
    }
}

pub fn num(v: Option<&str>) -> i64 {
    num_or(v, 0)
}

/// "#RRGGBB" (or "#AARRGGBB") -> "RRGGBB"; "none", missing or malformed -> None.
pub fn color(v: Option<&str>) -> Option<String> {
    let v = v?.trim();
    let hex = v.strip_prefix('#')?;
    if hex.len() < 6 || !hex.is_ascii() {
        return None;
    }
    let last6 = hex.get(hex.len() - 6..)?;
    if !last6.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(last6.to_ascii_uppercase())
}

#[derive(Clone, Debug, PartialEq)]
pub struct CharPr {
    /// HWPUNIT, 100 = 1 pt.
    pub height: i64,
    pub text_color: Option<String>,
    pub shade_color: Option<String>,
    /// language -> font id
    pub fonts: BTreeMap<&'static str, i64>,
    pub ratio: i64,
    pub spacing: i64,
    pub bold: bool,
    pub italic: bool,
    /// Line shape when underlined.
    pub underline: Option<String>,
    pub underline_color: Option<String>,
    pub strike: Option<String>,
    pub sup: bool,
    pub sub: bool,
    pub outline: bool,
    pub shadow: bool,
    pub emboss: bool,
    pub engrave: bool,
    pub sym_mark: String,
    pub border_fill: i64,
    /// Spaces as wide as the font makes them (`useFontSpace`); otherwise half the character
    /// height (see `convert::Converter::space_rpr`).
    pub font_space: bool,
}

impl Default for CharPr {
    fn default() -> Self {
        CharPr {
            height: 1000,
            text_color: Some("000000".into()),
            shade_color: None,
            fonts: BTreeMap::new(),
            ratio: 100,
            spacing: 0,
            bold: false,
            italic: false,
            underline: None,
            underline_color: None,
            strike: None,
            sup: false,
            sub: false,
            outline: false,
            shadow: false,
            emboss: false,
            engrave: false,
            sym_mark: "NONE".into(),
            border_fill: 0,
            font_space: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParaPr {
    pub align: String,
    pub indent: i64,
    pub left: i64,
    pub right: i64,
    pub before: i64,
    pub after: i64,
    pub ls_type: String,
    pub ls_value: i64,
    pub heading_type: String,
    pub heading_ref: i64,
    pub heading_level: i64,
    pub keep_next: bool,
    pub keep_lines: bool,
    pub page_break_before: bool,
    pub widow: bool,
    pub tab_pr: i64,
    pub border_fill: i64,
    /// The margins given in characters (`unit="CHAR"`), by [`ParaPr::CHAR_INDENT`] and the
    /// other bits; see [`ParaPr::in_hwpunit`].
    pub char_units: u8,
    /// The minimum space width (Hancom: 공백 최소값): how far, in percent, spaces may shrink to fit a line (0-75).
    pub condense: i64,
    /// Korean words wrap whole (`breakNonLatinWord="KEEP_WORD"`) rather than between any two
    /// syllables (`BREAK_WORD`).
    pub keep_words: bool,
}

impl ParaPr {
    pub const CHAR_INDENT: u8 = 1;
    pub const CHAR_LEFT: u8 = 2;
    pub const CHAR_RIGHT: u8 = 4;
    pub const CHAR_BEFORE: u8 = 8;
    pub const CHAR_AFTER: u8 = 16;

    /// These properties with the margins given in characters turned into HWPUNIT for a paragraph
    /// whose characters are `font_h` HWPUNIT high: a value of 100 is half a character, the
    /// value times the height over 200 (viewer, measured: 2000 at 12 pt is 120 pt, at 24 pt 240 pt).
    pub fn in_hwpunit(&self, font_h: i64) -> ParaPr {
        let mut pp = self.clone();
        let conv = |v: i64| v.saturating_mul(font_h.max(0)) / 200;
        for (bit, v) in [
            (Self::CHAR_INDENT, &mut pp.indent),
            (Self::CHAR_LEFT, &mut pp.left),
            (Self::CHAR_RIGHT, &mut pp.right),
            (Self::CHAR_BEFORE, &mut pp.before),
            (Self::CHAR_AFTER, &mut pp.after),
        ] {
            if self.char_units & bit != 0 {
                *v = conv(*v);
            }
        }
        pp.char_units = 0;
        pp
    }
}

impl Default for ParaPr {
    fn default() -> Self {
        ParaPr {
            align: "JUSTIFY".into(),
            indent: 0,
            left: 0,
            right: 0,
            before: 0,
            after: 0,
            ls_type: "PERCENT".into(),
            ls_value: 160,
            heading_type: "NONE".into(),
            heading_ref: 0,
            heading_level: 0,
            keep_next: false,
            keep_lines: false,
            page_break_before: false,
            widow: false,
            tab_pr: 0,
            border_fill: 0,
            char_units: 0,
            condense: 0,
            keep_words: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub eng_name: String,
    pub para_pr: i64,
    pub char_pr: i64,
    pub next: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    pub kind: String,
    pub width_mm: f64,
    pub color: Option<String>,
}

impl Default for Border {
    fn default() -> Self {
        Border { kind: "NONE".into(), width_mm: 0.1, color: Some("000000".into()) }
    }
}

impl Border {
    pub fn visible(&self) -> bool {
        !matches!(self.kind.as_str(), "NONE" | "")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub angle: i64,
    pub colors: Vec<String>,
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BorderFill {
    pub left: Border,
    pub right: Border,
    pub top: Border,
    pub bottom: Border,
    pub diagonal: Border,
    /// Bottom-left to top-right.
    pub slash: bool,
    /// Top-left to bottom-right.
    pub back_slash: bool,
    pub fill: Option<String>,
    /// (binary item id, `hc:imgBrush` mode)
    pub image: Option<(String, String)>,
    /// `fill` then holds the average of its colours (DOCX cells have no gradients).
    pub gradient: Option<Gradient>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NumLevel {
    /// 1-based, as written.
    pub level: i64,
    pub fmt: String,
    /// e.g. "^1."
    pub text: String,
    pub start: i64,
    pub char_pr: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Numbering {
    pub id: i64,
    pub start: i64,
    pub levels: BTreeMap<i64, NumLevel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabStop {
    pub pos: i64,
    pub kind: String,
    pub leader: String,
}

#[derive(Clone, Debug, Default)]
pub struct Header {
    /// language -> font id -> face
    pub fonts: BTreeMap<String, BTreeMap<i64, String>>,
    pub char_prs: BTreeMap<i64, CharPr>,
    pub para_prs: BTreeMap<i64, ParaPr>,
    /// In document order (styles are written in this order).
    pub styles: Vec<Style>,
    pub border_fills: BTreeMap<i64, BorderFill>,
    pub numberings: BTreeMap<i64, Numbering>,
    pub bullets: BTreeMap<i64, String>,
    pub tab_prs: BTreeMap<i64, Vec<TabStop>>,
    /// Tab definitions with a right tab at the end of the line (autoTabRight).
    pub auto_tab_right: BTreeSet<i64>,
    pub begin_page: i64,
    /// Where each style id is in `styles`, so a lookup doesn't go through every style (a
    /// header of 100,000 styles). Only `new` fills `styles`, and it keeps this in step.
    style_at: HashMap<i64, usize>,
}

impl Header {
    pub fn new(root: Option<&Element>) -> Header {
        let mut h = Header { begin_page: 1, ..Default::default() };
        let Some(root) = root else { return h };
        if let Some(bn) = root.find("hh:beginNum") {
            h.begin_page = num_or(bn.get("page"), 1).max(1);
        }
        let Some(r) = root.find("hh:refList") else { return h };
        for ff in r.iter("hh:fontface") {
            let lang = ff.get("lang").unwrap_or("").to_ascii_lowercase();
            let faces = ff.findall("hh:font").map(|f| (num(f.get("id")), f.get("face").unwrap_or("").to_string())).collect();
            h.fonts.insert(lang, faces);
        }
        for el in r.iter("hh:borderFill") {
            h.border_fills.insert(num(el.get("id")), border_fill(el));
        }
        for el in r.iter("hh:charPr") {
            h.char_prs.insert(num(el.get("id")), char_pr(el));
        }
        for el in r.iter("hh:tabPr") {
            let id = num(el.get("id"));
            let tabs = el
                .iter("hh:tabItem")
                .map(|t| TabStop {
                    pos: num(t.get("pos")),
                    kind: t.get("type").unwrap_or("LEFT").to_string(),
                    leader: t.get("leader").unwrap_or("NONE").to_string(),
                })
                .collect();
            h.tab_prs.insert(id, tabs);
            if truthy(el.get("autoTabRight")) {
                h.auto_tab_right.insert(id);
            }
        }
        for el in r.iter("hh:numbering") {
            h.numberings.insert(num(el.get("id")), numbering(el));
        }
        for el in r.iter("hh:bullet") {
            let c = el.get("char").filter(|c| !c.is_empty()).unwrap_or("•");
            h.bullets.insert(num(el.get("id")), c.to_string());
        }
        for el in r.iter("hh:paraPr") {
            h.para_prs.insert(num(el.get("id")), para_pr(el));
        }
        // where each style id is in `styles`: a later definition replaces it in place, found
        // without going through every style before it (a header of 100,000 styles)
        let mut style_at = std::mem::take(&mut h.style_at);
        for el in r.iter("hh:style") {
            let id = num(el.get("id"));
            let style = Style {
                id,
                kind: el.get("type").unwrap_or("PARA").to_string(),
                name: el.get("name").unwrap_or("").to_string(),
                eng_name: el.get("engName").unwrap_or("").to_string(),
                para_pr: num(el.get("paraPrIDRef")),
                char_pr: num(el.get("charPrIDRef")),
                next: num_or(el.get("nextStyleIDRef"), id),
            };
            match style_at.get(&id).and_then(|&i| h.styles.get_mut(i)) {
                Some(s) => *s = style, // a later definition replaces it in place
                None => {
                    style_at.insert(id, h.styles.len());
                    h.styles.push(style);
                }
            }
        }
        h.style_at = style_at;
        h
    }

    pub fn char_pr(&self, id: Option<&str>) -> CharPr {
        self.char_prs.get(&num(id)).or_else(|| self.char_prs.get(&0)).cloned().unwrap_or_default()
    }

    pub fn char_pr_id(&self, id: i64) -> CharPr {
        self.char_prs.get(&id).or_else(|| self.char_prs.get(&0)).cloned().unwrap_or_default()
    }

    pub fn para_pr(&self, id: Option<&str>) -> ParaPr {
        self.para_prs.get(&num(id)).or_else(|| self.para_prs.get(&0)).cloned().unwrap_or_default()
    }

    pub fn para_pr_id(&self, id: i64) -> ParaPr {
        self.para_prs.get(&id).or_else(|| self.para_prs.get(&0)).cloned().unwrap_or_default()
    }

    pub fn style(&self, id: i64) -> Option<&Style> {
        self.styles.get(*self.style_at.get(&id)?)
    }

    pub fn font_face(&self, lang: &str, font_id: Option<i64>) -> Option<&str> {
        let f = self.fonts.get(lang)?.get(&font_id?)?;
        (!f.is_empty()).then_some(f.as_str())
    }
}

fn border(el: Option<&Element>) -> Border {
    let Some(el) = el else { return Border::default() };
    let w = el.get("width").unwrap_or("0.1").to_ascii_lowercase().replace("mm", "");
    let width_mm = w.trim().parse::<f64>().ok().filter(|f| f.is_finite()).unwrap_or(0.1);
    Border { kind: el.get("type").unwrap_or("NONE").to_string(), width_mm, color: color(el.get("color")) }
}

fn border_fill(el: &Element) -> BorderFill {
    let mut bf = BorderFill {
        left: border(el.find("hh:leftBorder")),
        right: border(el.find("hh:rightBorder")),
        top: border(el.find("hh:topBorder")),
        bottom: border(el.find("hh:bottomBorder")),
        diagonal: border(el.find("hh:diagonal")),
        ..Default::default()
    };
    bf.slash = el.find("hh:slash").is_some_and(|s| s.get("type").unwrap_or("NONE") != "NONE");
    bf.back_slash = el.find("hh:backSlash").is_some_and(|s| s.get("type").unwrap_or("NONE") != "NONE");
    let fb = el.find("hc:fillBrush");
    if let Some(wb) = fb.and_then(|f| f.find("hc:winBrush")) {
        bf.fill = color(wb.get("faceColor"));
    }
    if let Some(gr) = fb.and_then(|f| f.find("hc:gradation"))
        && bf.fill.is_none()
    {
        let cols: Vec<String> = gr.findall("hc:color").filter_map(|x| color(x.get("value"))).collect();
        if cols.len() >= 2 {
            bf.gradient = Some(Gradient { angle: num(gr.get("angle")), colors: cols.clone(), kind: gr.get("type").unwrap_or("LINEAR").to_string() });
        }
        if !cols.is_empty() {
            bf.fill = Some(average(&cols));
        }
    }
    if let Some(ib) = fb.and_then(|f| f.find("hc:imgBrush"))
        && let Some(id) = ib.find("hc:img").and_then(|i| i.get("binaryItemIDRef")).filter(|s| !s.is_empty())
    {
        bf.image = Some((id.to_string(), ib.get("mode").unwrap_or("TILE").to_string()));
    }
    bf
}

/// The average of "RRGGBB" colours, channel by channel (rounded half to even, as Python does).
fn average(cols: &[String]) -> String {
    let mut out = String::new();
    for i in [0usize, 2, 4] {
        let sum: u32 = cols.iter().filter_map(|c| c.get(i..i + 2)).filter_map(|h| u32::from_str_radix(h, 16).ok()).sum();
        let v = round_half_even(f64::from(sum) / cols.len().max(1) as f64);
        out.push_str(&format!("{v:02X}"));
    }
    out
}

/// Python's round(): halves go to the even neighbour.
pub fn round_half_even(x: f64) -> i64 {
    let r = x.round();
    let r = if (x - x.trunc()).abs() == 0.5 && (r as i64) % 2 != 0 { r - x.signum() } else { r };
    r.clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

fn char_pr(el: &Element) -> CharPr {
    let mut cp = CharPr {
        height: num_or(el.get("height"), 1000),
        text_color: color(el.get("textColor")).or_else(|| Some("000000".into())),
        shade_color: color(el.get("shadeColor")),
        sym_mark: el.get("symMark").unwrap_or("NONE").to_string(),
        border_fill: num(el.get("borderFillIDRef")),
        font_space: truthy(el.get("useFontSpace")),
        ..Default::default()
    };
    if let Some(fr) = el.find("hh:fontRef") {
        cp.fonts = LANGS.iter().filter_map(|l| fr.get(l).map(|v| (*l, num(Some(v))))).collect();
    }
    if let Some(r) = el.find("hh:ratio") {
        let v = num_or(r.get("hangul"), 100);
        cp.ratio = if v == 0 { 100 } else { v };
    }
    if let Some(s) = el.find("hh:spacing") {
        cp.spacing = num(s.get("hangul"));
    }
    cp.bold = el.find("hh:bold").is_some();
    cp.italic = el.find("hh:italic").is_some();
    if let Some(u) = el.find("hh:underline")
        && u.get("type").unwrap_or("NONE") != "NONE"
    {
        cp.underline = Some(u.get("shape").unwrap_or("SOLID").to_string());
        cp.underline_color = color(u.get("color"));
    }
    if let Some(st) = el.find("hh:strikeout") {
        let shape = st.get("shape").unwrap_or("NONE");
        if !matches!(shape, "NONE" | "") {
            cp.strike = Some(shape.to_string());
        }
    }
    cp.sup = el.find("hh:supscript").is_some();
    cp.sub = el.find("hh:subscript").is_some();
    cp.outline = el.find("hh:outline").is_some_and(|o| o.get("type").unwrap_or("NONE") != "NONE");
    cp.shadow = el.find("hh:shadow").is_some_and(|o| o.get("type").unwrap_or("NONE") != "NONE");
    cp.emboss = el.find("hh:emboss").is_some();
    cp.engrave = el.find("hh:engrave").is_some();
    cp
}

fn para_pr(el: &Element) -> ParaPr {
    let mut pp = ParaPr { tab_pr: num(el.get("tabPrIDRef")), condense: num(el.get("condense")).clamp(0, 75), ..Default::default() };
    if let Some(a) = el.find("hh:align") {
        pp.align = a.get("horizontal").unwrap_or("JUSTIFY").to_string();
    }
    if let Some(h) = el.find("hh:heading") {
        pp.heading_type = h.get("type").unwrap_or("NONE").to_string();
        pp.heading_ref = num(h.get("idRef"));
        pp.heading_level = num(h.get("level"));
    }
    if let Some(bs) = el.find("hh:breakSetting") {
        pp.keep_next = truthy(bs.get("keepWithNext"));
        pp.keep_lines = truthy(bs.get("keepLines"));
        pp.keep_words = bs.get("breakNonLatinWord") != Some("BREAK_WORD");
        pp.page_break_before = truthy(bs.get("pageBreakBefore"));
        pp.widow = truthy(bs.get("widowOrphan"));
    }
    if let Some(m) = el.find("hh:margin") {
        let val = |tag: &str| m.find(tag).map(|v| num(v.get("value"))).unwrap_or(0);
        pp.indent = val("hc:intent");
        pp.left = val("hc:left");
        pp.right = val("hc:right");
        pp.before = val("hc:prev");
        pp.after = val("hc:next");
        for (tag, bit) in [
            ("hc:intent", ParaPr::CHAR_INDENT),
            ("hc:left", ParaPr::CHAR_LEFT),
            ("hc:right", ParaPr::CHAR_RIGHT),
            ("hc:prev", ParaPr::CHAR_BEFORE),
            ("hc:next", ParaPr::CHAR_AFTER),
        ] {
            if m.find(tag).and_then(|v| v.get("unit")) == Some("CHAR") {
                pp.char_units |= bit;
            }
        }
    }
    if let Some(ls) = el.find("hh:lineSpacing") {
        pp.ls_type = ls.get("type").unwrap_or("PERCENT").to_string();
        pp.ls_value = num_or(ls.get("value"), 160);
    }
    if let Some(b) = el.find("hh:border") {
        pp.border_fill = num(b.get("borderFillIDRef"));
    }
    pp
}

fn numbering(el: &Element) -> Numbering {
    let mut levels = BTreeMap::new();
    for ph in el.findall("hh:paraHead") {
        let lv = num_or(ph.get("level"), 1);
        let cpr = ph.get("charPrIDRef");
        levels.insert(
            lv,
            NumLevel {
                level: lv,
                fmt: ph.get("numFormat").unwrap_or("DIGIT").to_string(),
                text: ph.text.clone(),
                start: num_or(ph.get("start"), 1),
                char_pr: match cpr {
                    None | Some("") | Some("4294967295") => None,
                    Some(v) => Some(num(Some(v))),
                },
            },
        );
    }
    Numbering { id: num(el.get("id")), start: num_or(el.get("start"), 1), levels }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Margins in characters scale with the paragraph's character height: 100 is half a
    /// character (at 12 pt, 2000 is 120 pt); HWPUNIT margins stay as they are.
    #[test]
    fn char_margins_follow_the_character_height() {
        let xml = r#"<hh:paraPr xmlns:hh="h" xmlns:hc="c" id="1"><hh:margin><hc:intent value="3000" unit="CHAR"/><hc:left value="2000" unit="CHAR"/><hc:right value="500" unit="HWPUNIT"/><hc:prev value="2000" unit="CHAR"/><hc:next value="800" unit="HWPUNIT"/></hh:margin></hh:paraPr>"#;
        let Ok(el) = crate::xml::parse(xml.as_bytes(), false) else { panic!("parse") };
        let pp = para_pr(&el).in_hwpunit(1200);
        assert_eq!((pp.indent, pp.left, pp.right, pp.before, pp.after), (18_000, 12_000, 500, 12_000, 800));
        assert_eq!(para_pr(&el).in_hwpunit(2400).left, 24_000);
    }
}
