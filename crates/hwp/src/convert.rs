//! HWPX -> DOCX conversion (the element tree of [`crate::xml`] to WordprocessingML).
//!
//! The layout rules here were found by comparing Hancom's viewer with WordCraft on real
//! documents; each one says what was observed.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::docx::{CT_CORE, DECL, DocxPackage, W_NS, attr, ct, esc, rel};
use crate::error::Result;
use crate::header::{Border, BorderFill, CharPr, Header, ParaPr, num, num_or, truthy};
use crate::package::Package;
use crate::units::{emu, round_even, tw, tw_f};
use crate::xml::{Element, ORDINAL};

pub(crate) const A4: (i64, i64) = (59528, 84188); // HWPUNIT

pub(crate) const WPS_URI: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";
pub(crate) const FORMS: &[&str] = &["hp:btn", "hp:radioBtn", "hp:checkBtn", "hp:comboBox", "hp:listBox", "hp:edit"];
pub(crate) const SHAPES: &[&str] = &["hp:rect", "hp:ellipse", "hp:polygon", "hp:curve", "hp:arc", "hp:line", "hp:connectLine"];
/// Inline markers inside hp:t that carry no text of their own.
const SILENT_TEXT_MARKS: &[&str] = &["hp:markpenBegin", "hp:markpenEnd", "hp:titleMark", "hp:insertBegin", "hp:insertEnd"];

fn jc(align: &str) -> &'static str {
    match align {
        "LEFT" => "left",
        "RIGHT" => "right",
        "CENTER" => "center",
        "DISTRIBUTE" | "DISTRIBUTE_SPACE" => "distribute",
        _ => "both",
    }
}

fn underline(shape: &str) -> &'static str {
    match shape {
        "DOT" | "CIRCLE" => "dotted",
        "DASH" => "dash",
        "DASH_DOT" => "dotDash",
        "DASH_DOT_DOT" => "dotDotDash",
        "LONG_DASH" => "dashLong",
        "DOUBLE_SLIM" => "double",
        "SLIM_THICK" | "THICK_SLIM" | "SLIM_THICK_SLIM" | "THICK3D" | "THICKREV3D" => "thick",
        "WAVE" => "wave",
        "DOUBLEWAVE" => "wavyDouble",
        _ => "single",
    }
}

fn border_style(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "SOLID" => "single",
        "DOT" | "CIRCLE" => "dotted",
        "DASH" | "LONG_DASH" => "dashed",
        "DASH_DOT" => "dotDash",
        "DASH_DOT_DOT" => "dotDotDash",
        "DOUBLE_SLIM" => "double",
        "SLIM_THICK" => "thinThickSmallGap",
        "THICK_SLIM" => "thickThinSmallGap",
        "SLIM_THICK_SLIM" => "thinThickThinSmallGap",
        "WAVE" => "wave",
        "DOUBLEWAVE" => "doubleWave",
        "THICK3D" | "3D" => "threeDEmboss",
        "THICKREV3D" | "REV3D" => "threeDEngrave",
        _ => return None,
    })
}

fn num_fmt(f: &str) -> &'static str {
    match f {
        "CIRCLED_DIGIT" => "decimalEnclosedCircle",
        "ROMAN_CAPITAL" => "upperRoman",
        "ROMAN_SMALL" => "lowerRoman",
        "LATIN_CAPITAL" | "CIRCLED_LATIN_CAPITAL" => "upperLetter",
        "LATIN_SMALL" | "CIRCLED_LATIN_SMALL" => "lowerLetter",
        "HANGUL_SYLLABLE" | "CIRCLED_HANGUL_SYLLABLE" => "ganada",
        "HANGUL_JAMO" | "CIRCLED_HANGUL_JAMO" => "chosung",
        "HANGUL_PHONETIC" => "koreanDigital",
        "IDEOGRAPH" | "CIRCLED_IDEOGRAPH" => "ideographTraditional",
        "2DIGIT" => "decimalZero",
        _ => "decimal",
    }
}

fn tab_type(t: &str) -> &'static str {
    match t {
        "RIGHT" => "right",
        "CENTER" => "center",
        "DECIMAL" => "decimal",
        _ => "left",
    }
}

fn tab_leader(l: &str) -> &'static str {
    match l {
        "SOLID" => "underscore",
        "DOT" | "DASH_DOT_DOT" | "CIRCLE" => "dot",
        "DASH" | "LONG_DASH" | "DASH_DOT" => "hyphen",
        _ => "none",
    }
}

fn valign(v: &str) -> Option<&'static str> {
    match v {
        "TOP" => Some("top"),
        "CENTER" => Some("center"),
        "BOTTOM" => Some("bottom"),
        _ => None,
    }
}

/// Python's floor division for the few places that divide possibly negative numbers.
pub(crate) fn floor_div(a: i64, b: i64) -> i64 {
    if b == 0 { 0 } else { a.div_euclid(b) - if b < 0 && a.rem_euclid(b) != 0 { 1 } else { 0 } }
}

/// A paragraph whose pPr can still take a sectPr when its section ends.
#[derive(Clone, Debug, Default)]
pub(crate) struct P {
    pub ppr: String,
    pub runs: Vec<String>,
    /// HWPUNIT of line spacing after the last line (see [`Converter::cell`]).
    pub tail: i64,
    /// The page number paragraph added for hp:pageNum.
    pub page_num: bool,
    /// It carries its source mark already ([`Converter::mark_origin`]): told by this, not by a
    /// bookmark's name, since a document's own bookmark may start with "_h" too.
    pub marked: bool,
}

impl P {
    pub fn new(ppr: impl Into<String>, runs: Vec<String>) -> Self {
        P { ppr: ppr.into(), runs, ..Default::default() }
    }

    pub fn render(&self, sect: &str) -> String {
        format!("<w:p><w:pPr>{}{sect}</w:pPr>{}</w:p>", self.ppr, self.runs.concat())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Block {
    P(P),
    /// A rendered w:tbl.
    Table(String),
    /// A bookmark between paragraphs: the source mark of a paragraph shown only as a table.
    Anchor(String),
}

/// A Windows FILETIME (100 ns ticks since 1601-01-01 UTC) as ISO 8601 (`2025-01-02T03:04:05Z`);
/// `None` for 0 or a time before 1970 or after 9999.
fn iso_from_filetime(ft: u64) -> Option<String> {
    let secs = i64::try_from(ft / 10_000_000).ok()? - 11_644_473_600;
    if secs <= 0 {
        return None;
    }
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil date from days since 1970-01-01 (proleptic Gregorian)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y <= 9999).then(|| format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60))
}

/// `s` in pieces that are all spaces (U+0020) or have none.
fn split_spaces(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<bool> = None;
    for (i, c) in s.char_indices() {
        let space = c == ' ';
        if prev.is_some_and(|p| p != space) {
            out.push(s.get(start..i).unwrap_or(""));
            start = i;
        }
        prev = Some(space);
    }
    if start < s.len() {
        out.push(s.get(start..).unwrap_or(""));
    }
    out
}

pub(crate) fn wt(s: &str) -> String {
    format!("<w:t xml:space=\"preserve\">{}</w:t>", esc(s))
}

pub(crate) fn field_runs(instr: &str, rpr: &str, shown: &str) -> Vec<String> {
    vec![
        format!("<w:r>{rpr}<w:fldChar w:fldCharType=\"begin\"/></w:r>"),
        format!("<w:r>{rpr}<w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r>", esc(instr)),
        format!("<w:r>{rpr}<w:fldChar w:fldCharType=\"separate\"/></w:r>"),
        format!("<w:r>{rpr}{}</w:r>", wt(shown)),
        format!("<w:r>{rpr}<w:fldChar w:fldCharType=\"end\"/></w:r>"),
    ]
}

pub(crate) fn border_xml(tag: &str, b: &Border) -> String {
    match border_style(&b.kind).filter(|_| b.visible()) {
        None => format!("<w:{tag} w:val=\"nil\"/>"),
        Some(style) => {
            let sz = round_even(b.width_mm * 72.0 / 25.4 * 8.0).clamp(2, 96);
            format!("<w:{tag} w:val=\"{style}\" w:sz=\"{sz}\" w:space=\"0\" w:color=\"{}\"/>", b.color.as_deref().unwrap_or("auto"))
        }
    }
}

/// A paragraph that takes (almost) no height: separates adjacent tables and closes a section
/// or document that ends with a table, which WordCraft/Word otherwise pad with a full-height
/// empty paragraph.
pub(crate) const TINY_PPR: &str = "<w:spacing w:before=\"0\" w:after=\"0\" w:line=\"20\" w:lineRule=\"exact\"/><w:rPr><w:sz w:val=\"2\"/></w:rPr>";

/// A bullet character as WordCraft can draw it. Bullets in the private-use range are glyphs of
/// symbol fonts, which a font without them can't show: the ones whose glyph Hancom's viewer was
/// seen to draw become that character, the rest "•".
fn bullet_char(ch: char) -> char {
    match ch as u32 {
        0xF0FC => '✓', // drawn as a check mark (2026-11-17)
        0xE000..=0xF8FF => '•',
        _ => ch,
    }
}

/// Does a run show text on its line (text other than spaces, a tab or a break), not counting
/// what is inside a drawing (a text box's own text)? Spaces beside a table on a text line
/// share the table's line in Hancom.
fn shows_text(run: &str) -> bool {
    let mut rest = run;
    let mut outside = String::new();
    while let Some(i) = rest.find("<w:drawing>") {
        outside.push_str(rest.get(..i).unwrap_or(""));
        rest = match rest.get(i..).and_then(|r| r.find("</w:drawing>").map(|j| i + j + "</w:drawing>".len())) {
            Some(end) => rest.get(end..).unwrap_or(""),
            None => "",
        };
    }
    outside.push_str(rest);
    if ["<w:tab/>", "<w:br", "<w:sym ", "<w:fldChar", "<w:instrText"].iter().any(|t| outside.contains(t)) {
        return true;
    }
    // the text of each w:t
    let mut rest = outside.as_str();
    while let Some(i) = rest.find("<w:t") {
        let after = rest.get(i + 4..).unwrap_or("");
        rest = after;
        if !(after.starts_with('>') || after.starts_with(' ')) {
            continue; // w:tab, w:tbl and the like
        }
        let Some(gt) = after.find('>') else { break };
        let body = after.get(gt + 1..).unwrap_or("");
        let text = body.find("</w:t>").and_then(|e| body.get(..e)).unwrap_or("");
        if !text.trim().is_empty() {
            return true;
        }
    }
    false
}

/// An offset attribute; HWPX writes negative ones as unsigned 32-bit numbers.
pub(crate) fn signed32(v: Option<&str>) -> i64 {
    let n = num(v);
    if n >= 1 << 31 { n - (1 << 32) } else { n }
}

/// Smaller outer margins of tables are left out (see [`Converter::table`]).
const SPACER_MIN: i64 = 1000;

/// An empty paragraph exactly `hu` tall.
fn spacer_ppr(hu: i64) -> String {
    format!(
        "<w:spacing w:before=\"0\" w:after=\"0\" w:line=\"{}\" w:lineRule=\"exact\"/><w:rPr><w:sz w:val=\"2\"/></w:rPr>",
        round_even(hu as f64 / 5.0).max(20)
    )
}

// ---- small text scanners (the XML strings here are the converter's own output) ----------

/// The first `key="<digits>"`: (start, end) of the whole match and the digits.
fn find_num_attr<'s>(s: &'s str, key: &str) -> Option<(usize, usize, &'s str)> {
    let pat = format!("{key}=\"");
    let mut from = 0;
    while let Some(i) = s.get(from..).and_then(|r| r.find(&pat)) {
        let start = from + i;
        let ds = start + pat.len();
        let rest = s.get(ds..).unwrap_or("");
        let n = rest.bytes().take_while(u8::is_ascii_digit).count();
        if n > 0 && rest.as_bytes().get(n) == Some(&b'"') {
            return Some((start, ds + n + 1, rest.get(..n).unwrap_or("")));
        }
        from = start + 1; // `pat` starts with an ASCII byte
    }
    None
}

fn splice(s: &str, start: usize, end: usize, with: &str) -> String {
    format!("{}{with}{}", s.get(..start).unwrap_or(""), s.get(end..).unwrap_or(""))
}

/// The text of every `<w:t ...>text</w:t>`.
pub(crate) fn w_t_texts(s: &str) -> String {
    let mut out = String::new();
    let mut from = 0;
    while let Some(i) = s.get(from..).and_then(|r| r.find("<w:t")) {
        let start = from + i;
        from = start + 1;
        let Some(gt) = s.get(start..).and_then(|r| r.find('>')).map(|g| start + g) else { break };
        let rest = s.get(gt + 1..).unwrap_or("");
        let lt = rest.find('<').unwrap_or(rest.len());
        if rest.get(lt..).is_some_and(|r| r.starts_with("</w:t>")) {
            out.push_str(rest.get(..lt).unwrap_or(""));
        }
    }
    out
}

fn no_spaces(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Runs of white space -> one `with`.
fn squeeze_spaces(s: &str, with: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push_str(with);
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// `\x` -> `x` (any character but a line break).
/// `s` split at each `sep` not behind a backslash (the escapes stay; see [`unescape_backslash`]).
pub(crate) fn split_unescaped(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut escaped) = (0, false);
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == sep {
            out.push(s.get(start..i).unwrap_or(""));
            start = i + c.len_utf8();
        }
    }
    out.push(s.get(start..).unwrap_or(""));
    out
}

/// A field command's text with its escapes undone: a backslash before a separator (`;`, `/`) or
/// another backslash. Any other backslash is the text's own, as in a Windows path written as it
/// is ("E:\Work\other.hwp" stays so; "E:\\Work\\other.hwp" becomes it).
pub(crate) fn unescape_backslash(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (c, it.peek()) {
            ('\\', Some(&n)) if matches!(n, ';' | '/' | '\\') => {
                out.push(n);
                it.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// `^1` -> `%1` (HWP level placeholders to DOCX ones).
pub(crate) fn level_placeholders(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (c, it.peek()) {
            ('^', Some(&d)) if d.is_ascii_digit() => {
                out.push('%');
                out.push(d);
                it.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// Every wrapping element (`<wp:wrapSquare .../>` and the like) -> `<wp:wrapNone/>`.
pub(crate) fn no_wrap(s: &str) -> String {
    const KINDS: [&str; 4] = ["TopAndBottom", "Square", "Tight", "Through"];
    let mut out = String::new();
    let mut from = 0;
    while let Some(i) = s.get(from..).and_then(|r| r.find("<wp:wrap")) {
        let start = from + i;
        let after = start + "<wp:wrap".len();
        let tail = s.get(after..).unwrap_or("");
        let end = KINDS.iter().find(|k| tail.starts_with(*k)).and_then(|k| wrap_end(s, after + k.len()));
        match end {
            Some(end) => {
                out.push_str(s.get(from..start).unwrap_or(""));
                out.push_str("<wp:wrapNone/>");
                from = end;
            }
            None => {
                out.push_str(s.get(from..after).unwrap_or(""));
                from = after;
            }
        }
    }
    out.push_str(s.get(from..).unwrap_or(""));
    out
}

/// End of a wrapping element whose name ends at `at`: `.../>` or `...>...</wp:wrapX>`.
fn wrap_end(s: &str, at: usize) -> Option<usize> {
    let rest = s.get(at..)?;
    for (j, c) in rest.char_indices() {
        if rest.get(j..).is_some_and(|r| r.starts_with("/>")) {
            return Some(at + j + 2);
        }
        if c == '>' {
            let inner = rest.get(j + 1..)?;
            let line = inner.split('\n').next().unwrap_or("");
            let close = line.find("</wp:wrap")?;
            let name = line.get(close + "</wp:wrap".len()..)?;
            let n = name.chars().take_while(|c| c.is_alphanumeric() || *c == '_').map(char::len_utf8).sum::<usize>();
            if n > 0 && name.get(n..).is_some_and(|r| r.starts_with('>')) {
                return Some(at + j + 1 + close + "</wp:wrap".len() + n + 1);
            }
            return None;
        }
    }
    None
}

/// Hancom (and Word) add one paragraph's space after to the next one's space before; WordCraft
/// keeps only the larger of the two (measured: 20pt + 20pt -> 20pt). Moving the space after
/// into the next paragraph's space before gives the sum in both.
fn merge_spacing(blocks: &mut [Block]) {
    for i in 1..blocks.len() {
        let (head, tail) = blocks.split_at_mut(i);
        let (Some(Block::P(a)), Some(Block::P(b))) = (head.last_mut(), tail.first_mut()) else { continue };
        let Some((a0, a1, av)) = find_num_attr(&a.ppr, "w:after") else { continue };
        let Some((b0, b1, bv)) = find_num_attr(&b.ppr, "w:before") else { continue };
        if av == "0" {
            continue;
        }
        let total = av.parse::<i64>().unwrap_or(0).saturating_add(bv.parse::<i64>().unwrap_or(0));
        a.ppr = splice(&a.ppr, a0, a1, "w:after=\"0\"");
        b.ppr = splice(&b.ppr, b0, b1, &format!("w:before=\"{total}\""));
    }
}

pub(crate) fn render_blocks(blocks: &mut [Block], need_final_p: bool) -> String {
    merge_spacing(blocks);
    let mut out = String::new();
    let mut prev_table = false;
    for b in blocks.iter() {
        match b {
            Block::Anchor(xml) => {
                if prev_table {
                    // the separator goes first, so the mark sits right before its table
                    out.push_str(&P::new(TINY_PPR, vec![]).render(""));
                    prev_table = false;
                }
                out.push_str(xml);
            }
            Block::Table(xml) => {
                if prev_table {
                    // Adjacent tables would merge into one in Word; keep them apart.
                    out.push_str(&P::new(TINY_PPR, vec![]).render(""));
                }
                out.push_str(xml);
                prev_table = true;
            }
            Block::P(p) => {
                out.push_str(&p.render(""));
                prev_table = false;
            }
        }
    }
    if need_final_p && (blocks.is_empty() || prev_table) {
        out.push_str("<w:p/>");
    }
    out
}

/// Unicode circled number for 1..50, else the number in parentheses.
fn circled(n: i64) -> String {
    let c = match n {
        1..=20 => char::from_u32(0x2460 + (n - 1) as u32),
        21..=35 => char::from_u32(0x3251 + (n - 21) as u32),
        36..=50 => char::from_u32(0x32B1 + (n - 36) as u32),
        _ => None,
    };
    c.map(String::from).unwrap_or_else(|| format!("({n})"))
}

/// Percent-encode as Python's `urllib.parse.quote(s, safe)`.
fn quote(s: &str, safe: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "_.-~".contains(c) || safe.contains(c) {
            o.push(c);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// The most page background tiles laid one by one; more are drawn as one tiling fill.
const MAX_BACKGROUND_TILES: i64 = 10_000;

/// The largest page size or margin read from a file, in HWPUNIT (about 2,000 inches). Real pages
/// are far smaller; the bound keeps every sum of page lengths in range.
const MAX_PAGE_LENGTH: i64 = 14_400_000;

#[derive(Clone, Debug)]
pub(crate) struct Margins {
    pub left: i64,
    pub right: i64,
    pub top: i64,
    pub bottom: i64,
    pub header: i64,
    pub footer: i64,
    pub gutter: i64,
}

impl Default for Margins {
    fn default() -> Self {
        Margins { left: 8504, right: 8504, top: 5668, bottom: 4252, header: 4252, footer: 4252, gutter: 0 }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PageBorder {
    pub bf: BorderFill,
    pub paper: bool,
    pub area: String,
    /// left, right, top, bottom
    pub off: [i64; 4],
}

#[derive(Clone, Debug)]
pub(crate) struct Hf {
    pub kind: &'static str,
    pub which: &'static str,
    pub name: String,
    /// None: the same part as another reference (see first_page_parts).
    pub blocks: Option<Vec<Block>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HideFirst {
    pub header: bool,
    pub footer: bool,
    pub pagenum: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Section {
    pub width: i64,
    pub height: i64,
    pub landscape: bool,
    pub vertical: bool,
    pub margin: Margins,
    pub start_page: i64,
    pub outline: Option<i64>,
    pub tab_stop: i64,
    /// (pos, formatType, sideChar)
    pub page_num: Option<(String, String, String)>,
    pub hf: Vec<Hf>,
    /// ((kind, which), rId)
    pub refs: Vec<((&'static str, &'static str), String)>,
    pub have_secpr: bool,
    pub title_pg: bool,
    /// Page of the current paragraph (Hancom's layout).
    pub page_index: i64,
    pub hide_first: Option<HideFirst>,
    /// The kind (header/footer) the page number paragraph went to.
    pub page_num_kind: Option<&'static str>,
    pub page_border: Option<PageBorder>,
    /// The master page (Hancom: 바탕쪽): its paragraphs.
    pub master: Option<Element>,
}

impl Default for Section {
    fn default() -> Self {
        Section {
            width: A4.0,
            height: A4.1,
            landscape: false,
            vertical: false,
            margin: Margins::default(),
            start_page: 0,
            outline: None,
            tab_stop: 8000,
            page_num: None,
            hf: Vec::new(),
            refs: Vec::new(),
            have_secpr: false,
            title_pg: false,
            page_index: 0,
            hide_first: None,
            page_num_kind: None,
            page_border: None,
            master: None,
        }
    }
}

fn set_ref(refs: &mut Vec<((&'static str, &'static str), String)>, key: (&'static str, &'static str), rid: String) {
    match refs.iter_mut().find(|(k, _)| *k == key) {
        Some(r) => r.1 = rid,
        None => refs.push((key, rid)),
    }
}

/// What a conversion did: counts of what was converted and of what was not.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub stats: BTreeMap<String, i64>,
    pub unsupported: BTreeMap<String, i64>,
    /// Whether Korean words wrap whole (KEEP_WORD, `true`) or between any syllables
    /// (BREAK_WORD), by the origin bookmark each converted paragraph starts with (see
    /// `Converter::mark_origin`). DOCX has no such setting, so the reader of the package can't
    /// see it: [`apply_keep_words`] puts it on the paragraphs of the document read from it.
    pub keep_words: BTreeMap<String, bool>,
}

/// Puts each paragraph's Korean word wrapping from `report` (see [`Report::keep_words`]) on the
/// paragraphs of `doc`, the document read from the converted package: a paragraph takes the
/// setting of the first origin bookmark in it.
pub fn apply_keep_words(doc: &mut wordcraft_doc::Document, report: &Report) {
    use wordcraft_doc::{InlineObject, StoryRef};
    if report.keep_words.is_empty() {
        return;
    }
    let stories: Vec<StoryRef> = std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|&id| StoryRef::Part(id))).collect();
    for story in stories {
        for path in doc.para_paths(story) {
            let keep = doc.para(story, &path).and_then(|p| {
                p.objects_with_offsets().find_map(|(_, o)| match o {
                    InlineObject::BookmarkStart { name } => report.keep_words.get(name).copied(),
                    _ => None,
                })
            });
            if let Some(keep) = keep
                && let Ok(p) = doc.para_mut(story, &path)
            {
                p.props.keep_hangul_words = Some(keep);
            }
        }
    }
}

/// A table cell being converted.
#[derive(Clone, Debug)]
pub(crate) struct CellInfo<'e> {
    pub el: &'e Element,
    pub row: i64,
    pub col: i64,
    pub rs: i64,
    pub cs: i64,
    pub w: i64,
    pub h: i64,
    pub bf: Option<BorderFill>,
    /// top, left, bottom, right
    pub tmar: Option<[i64; 4]>,
}

pub(crate) struct Converter<'a> {
    pub pkg: &'a Package,
    pub h: Header,
    /// Missing fonts swapped for installed stand-ins (see [`crate::fonts`]), and the header's
    /// own names (language -> id -> name).
    font_subs: BTreeMap<String, String>,
    font_names: BTreeMap<String, BTreeMap<i64, String>>,
    pub out: DocxPackage,
    /// The part relationships go to (document, a header, notes...).
    pub part: String,
    pub stats: BTreeMap<String, i64>,
    pub unsupported: BTreeMap<String, i64>,
    pub sec: Section,
    num_ids: Vec<((&'static str, i64), i64)>,
    footnotes: Vec<(i64, Vec<Block>)>,
    endnotes: Vec<(i64, Vec<Block>)>,
    note_kind: Option<&'static str>,
    note_ref_done: bool,
    fields: HashMap<String, bool>,
    page_last: bool,
    object_numbers: BTreeMap<String, i64>,
    /// Twips, inside a table cell (None: the body).
    pub text_width: Option<i64>,
    /// Column count and gap of the current region.
    pub columns: (i64, i64),
    memos: HashMap<String, Element>,
    memo_fields: HashMap<String, i64>,
    /// Comments made from memos: author, when (ISO 8601, UTC), paragraphs.
    comments: Vec<(String, Option<String>, String)>,
    /// Layout-cache position of the next top-level paragraph.
    para_top: Option<i64>,
    /// ... of the table being converted.
    table_top: Option<i64>,
    /// The alignment of the paragraph holding the table being converted: a table on a text line
    /// sits where that paragraph aligns it.
    table_para_align: Option<String>,
    click_fields: HashMap<String, String>,
    click_open: Option<String>,
    click_close: Option<String>,
    in_delete: bool,
    pub pic_n: i64,
    bm_n: i64,
    /// [`Report::keep_words`] as the paragraphs are marked.
    keep_words: BTreeMap<String, bool>,
    sec_index: usize,
    hf_n: i64,
    even_odd: bool,
    inherited_refs: Vec<((&'static str, &'static str), String)>,
    /// The last header/footer of each kind a section made: its part and its own blocks (before
    /// the section's master page and background went in), for a later section that inherits it
    /// but needs one of its own to add its master page or background to.
    inherited_hf: HashMap<(&'static str, &'static str), (String, Vec<Block>)>,
    rpr_cache: HashMap<i64, String>,
    /// The character shape each cached rPr was made from, and the rPr of its spaces when they
    /// need other spacing (see [`Converter::space_rpr`]).
    rpr_ids: HashMap<String, i64>,
    /// The character shape of the run being written: the shape its spaces are measured by.
    /// Two shapes can give the same rPr (one using the font's space, one not), so the shape
    /// isn't told from the rPr while a run is written.
    run_shape: Option<i64>,
    space_rprs: HashMap<(i64, i64), Option<String>>,
    depth: usize,
    /// Table cells the file doesn't hold that may still be made, for the whole document: the
    /// parts of merged cells below their first row and the empty cells of declared rows and
    /// columns (hostile files: [`MAX_MADE_CELLS`]).
    made_cells_left: i64,
}

/// Most table cells made up during one conversion (merged-cell rows and empty cells a table
/// declares but doesn't hold). A table may declare 10,000 rows of 64 columns with one cell in
/// them; repeated, that is millions of cells from a small file. Past this, declared rows without
/// cells are left out and counted in the stats.
pub(crate) const MAX_MADE_CELLS: i64 = 200_000;

/// How one paragraph's lines are laid out.
pub(crate) struct ParaCtx {
    pp: ParaPr,
    style_id: Option<String>,
    /// HWPUNIT height the line spacing is measured from.
    font_h: i64,
    page_break: bool,
    /// The paragraph mark's run properties (its last run's).
    mark_rpr: String,
    /// No line spacing after the (one) line: a cell's or page's last.
    trim: bool,
    text_only: bool,
    /// Its tables go on the text line, in text boxes.
    inline_tables: bool,
}

/// A paragraph being converted: the pieces made so far.
#[derive(Default)]
pub(crate) struct ParaOut {
    blocks: Vec<Block>,
    cap_above: Vec<Block>,
    cap_below: Vec<Block>,
    runs: Vec<String>,
    /// Nothing written yet (the page break goes on the first piece).
    first: bool,
    click_start: Option<usize>,
    para_top: Option<i64>,
}

/// Hancom sizes each line by its own tallest character, so a short bigger run (a citation
/// "[1]") lifts one line only; use the height that carries most text.
fn main_height(runs_el: &[&Element], heights: &[i64], max_h: i64) -> i64 {
    let mut weight: Vec<(i64, i64)> = Vec::new();
    for (r, hgt) in runs_el.iter().zip(heights.iter()) {
        let n: i64 =
            r.findall("hp:t").map(|t| (t.text.chars().count() + t.children.iter().map(|c| c.tail.chars().count()).sum::<usize>()) as i64).sum();
        match weight.iter_mut().find(|(k, _)| k == hgt) {
            Some(w) => w.1 += n,
            None => weight.push((*hgt, n)),
        }
    }
    if weight.iter().any(|(_, w)| *w > 0)
        && let Some(&(main, _)) = weight.iter().max_by_key(|(k, w)| (*w, *k))
        && max_h * 5 <= main * 6
    {
        return main; // much bigger text keeps its room (exact lines clip)
    }
    max_h
}

/// Deepest nesting of tables, text boxes and notes followed (hostile files).
const MAX_NESTING: usize = 64;
const DOC_PART: &str = "word/document.xml";

impl<'a> Converter<'a> {
    pub fn new(pkg: &'a Package) -> Result<Self> {
        let head = match &pkg.header_name {
            Some(n) => Some(pkg.xml(n)?),
            None => None,
        };
        let mut out = DocxPackage::default();
        out.part(DOC_PART);
        let mut h = Header::new(head.as_ref());
        let font_names = h.fonts.clone();
        let font_subs = crate::fonts::substitute_all(&mut h.fonts);
        Ok(Converter {
            pkg,
            h,
            font_subs,
            font_names,
            out,
            part: DOC_PART.to_string(),
            stats: BTreeMap::new(),
            unsupported: BTreeMap::new(),
            sec: Section::default(),
            num_ids: Vec::new(),
            footnotes: Vec::new(),
            endnotes: Vec::new(),
            note_kind: None,
            note_ref_done: false,
            fields: HashMap::new(),
            page_last: false,
            object_numbers: BTreeMap::new(),
            text_width: None,
            columns: (1, 0),
            memos: HashMap::new(),
            memo_fields: HashMap::new(),
            comments: Vec::new(),
            para_top: None,
            table_top: None,
            table_para_align: None,
            click_fields: HashMap::new(),
            click_open: None,
            click_close: None,
            in_delete: false,
            pic_n: 0,
            bm_n: 0,
            keep_words: BTreeMap::new(),
            sec_index: 0,
            hf_n: 0,
            even_odd: false,
            inherited_refs: Vec::new(),
            inherited_hf: HashMap::new(),
            rpr_cache: HashMap::new(),
            rpr_ids: HashMap::new(),
            run_shape: None,
            space_rprs: HashMap::new(),
            depth: 0,
            made_cells_left: MAX_MADE_CELLS,
        })
    }

    pub fn stat(&mut self, key: &str) {
        *self.stats.entry(key.to_string()).or_insert(0) += 1;
    }

    pub fn missing(&mut self, key: &str) {
        *self.unsupported.entry(key.to_string()).or_insert(0) += 1;
    }

    pub fn add_rel(&mut self, kind: &str, target: &str) -> String {
        let part = self.part.clone();
        self.out.part(&part).add_rel(&rel(kind), target, false)
    }

    // ==== entry =====================================================================
    pub fn convert(mut self) -> Result<(Vec<u8>, Report)> {
        let mut body: Vec<String> = Vec::new();
        let names = self.pkg.section_names.clone();
        for (i, name) in names.iter().enumerate() {
            self.sec_index = i;
            self.sec =
                if i > 0 { Section { outline: self.sec.outline, tab_stop: self.sec.tab_stop, ..Default::default() } } else { Section::default() };
            let mut root = self.pkg.xml(name)?;
            self.memos = root.findall("hp:memo").filter_map(|m| Some((m.get("id")?.to_string(), m.find("hp:subList")?.clone()))).collect();
            let mut blocks: Vec<Block> = Vec::new();
            // Column regions: (index into blocks where the region starts, (count, gap)). Hancom
            // can change the column count mid-section (a two-column table of contents); DOCX
            // needs a continuous section break for each change.
            let mut regions: Vec<(usize, (i64, i64))> = vec![(0, (1, 0))];
            let (mut last_pos, mut region_col) = (-1i64, 0i64);
            let idx: Vec<usize> = root.children.iter().enumerate().filter(|(_, c)| c.tag == "hp:p").map(|(k, _)| k).collect();
            let starts: Vec<Option<i64>> = idx
                .iter()
                .map(|&k| {
                    root.children.get(k).and_then(|p| p.find("hp:linesegarray")).and_then(|s| s.children.first()).map(|s0| num(s0.get("vertpos")))
                })
                .collect();
            for (j, &k) in idx.iter().enumerate() {
                let Some(p) = root.children.get(k) else { continue };
                // Last paragraph on its page (or column): the next one restarts the cache.
                let segs_len = p.find("hp:linesegarray").map(|s| s.children.len());
                let first_vert = p.find("hp:linesegarray").and_then(|s| s.children.first()).map(|s| num(s.get("vertpos")));
                let last_vert = p.find("hp:linesegarray").and_then(|s| s.children.last()).map(|s| num(s.get("vertpos")));
                let nxt = if j + 1 < idx.len() { starts.get(j + 1).copied().flatten() } else { Some(0) };
                self.para_top = starts.get(j).copied().flatten();
                let col = direct_col_pr(p);
                if let Some(col) = col {
                    if blocks.is_empty() {
                        regions[0] = (0, col);
                    } else if regions.last().map(|r| r.1) != Some(col) {
                        regions.push((blocks.len(), col));
                    }
                }
                self.columns = regions.last().map(|r| r.1).unwrap_or((1, 0));
                // (One column only, and not before a new column region: the cache restarts at
                // each column and region too, and WordCraft balances columns its own way.)
                let next_has_col = idx.get(j + 1).and_then(|&n| root.children.get(n)).map(|q| direct_col_pr(q).is_some()).unwrap_or(false);
                self.page_last =
                    segs_len == Some(1) && matches!((nxt, first_vert), (Some(n), Some(f)) if n <= f) && self.columns.0 == 1 && !next_has_col;
                // Hancom's layout cache restarts line positions at each page, so a drop in
                // vertpos marks a new page (an explicit page break without a cache, too). It
                // restarts at each column and each new column region as well: in a region of n
                // columns every n-th restart is a page, the others are column breaks, written as
                // such so the columns split where Hancom split them.
                let mut set_column_break = None;
                match (first_vert, last_vert) {
                    (Some(first), Some(last)) => {
                        if col.is_some() {
                            // A new column definition starts again in the first column; Hancom
                            // ignores a column break on that paragraph (viewer).
                            region_col = 0;
                            set_column_break = Some("0");
                        } else if 0 <= first && first < last_pos {
                            region_col += 1;
                            let cols = self.columns.0;
                            if cols > 1 && region_col % cols != 0 && !truthy(p.get("pageBreak")) {
                                set_column_break = Some("1");
                            } else {
                                self.sec.page_index += 1;
                            }
                        }
                        last_pos = last;
                    }
                    _ => {
                        if truthy(p.get("pageBreak")) && !blocks.is_empty() {
                            self.sec.page_index += 1;
                        }
                    }
                }
                if let Some(v) = set_column_break
                    && let Some(pm) = root.children.get_mut(k)
                {
                    pm.set("columnBreak", v);
                }
                let Some(p) = root.children.get(k) else { continue };
                let made = self.paragraph(p, false);
                blocks.extend(made);
            }
            self.finish_section();
            self.stat("sections");
            let n_regions = regions.len();
            for r in 0..n_regions {
                let Some(&(start, (cols, gap))) = regions.get(r) else { continue };
                let end = regions.get(r + 1).map(|x| x.0).unwrap_or(blocks.len()).min(blocks.len());
                let start = start.min(end);
                let part = blocks.get_mut(start..end).unwrap_or_default();
                // A column region that starts on a new page: the page break is the section
                // break's (a page break inside a continuous break is not honoured).
                let mut new_page = false;
                if r > 0
                    && let Some(Block::P(p0)) = part.first_mut()
                    && p0.ppr.contains("<w:pageBreakBefore/>")
                {
                    p0.ppr = p0.ppr.replacen("<w:pageBreakBefore/>", "", 1);
                    new_page = true;
                }
                let sect = self.sect_xml(cols, gap, r == 0, new_page);
                if r == n_regions - 1 && i == names.len() - 1 {
                    // The document's last sectPr is a child of the body.
                    body.push(render_blocks(part, false));
                    if matches!(part.last(), Some(Block::Table(_))) {
                        body.push(P::new(TINY_PPR, vec![]).render(""));
                    }
                    body.push(sect);
                } else if let Some((Block::P(lastp), rest)) = {
                    // spacing between the last two paragraphs is merged as in the last region
                    // (rendering `rest` alone would leave the last paragraph out of it)
                    merge_spacing(part);
                    part.split_last_mut()
                } {
                    // Any other sectPr rides on the last paragraph of its region.
                    body.push(render_blocks(rest, false));
                    body.push(lastp.render(&sect));
                } else {
                    let empty = part.is_empty();
                    body.push(render_blocks(part, false));
                    body.push(P::new(if empty { "" } else { TINY_PPR }, vec![]).render(&sect));
                }
            }
        }
        let doc = format!("{DECL}<w:document {W_NS}><w:body>{}</w:body></w:document>", body.concat());
        self.out.put(DOC_PART, &doc, Some(&ct("document.main")));
        self.write_styles();
        self.write_numbering();
        self.write_notes();
        self.write_settings();
        self.write_core();
        if !self.font_subs.is_empty() {
            // kept so a new HWPX written from the document puts the original names back
            self.out.custom.push((crate::fonts::PROPERTY.into(), crate::fonts::to_json(&self.font_subs)));
            self.stats.insert("font substitutions".into(), self.font_subs.len() as i64);
        }
        let report = Report { stats: self.stats.clone(), unsupported: self.unsupported.clone(), keep_words: std::mem::take(&mut self.keep_words) };
        Ok((self.out.finish()?, report))
    }

    // ==== blocks / paragraphs ===========================================================
    pub fn blocks<'e>(&mut self, paras: impl Iterator<Item = &'e Element>, in_cell: bool) -> Vec<Block> {
        let paras: Vec<&Element> = paras.collect();
        if self.depth >= MAX_NESTING {
            self.missing("nesting too deep");
            return Vec::new();
        }
        self.depth += 1;
        let mut out = Vec::new();
        let n = paras.len();
        for (i, p) in paras.into_iter().enumerate() {
            out.extend(self.paragraph(p, in_cell && i + 1 == n));
        }
        self.depth -= 1;
        out
    }

    fn style_height(&self, style_id: Option<&str>) -> i64 {
        let cp = self.h.style(num(style_id)).map(|s| s.char_pr).unwrap_or(0);
        self.h.char_pr_id(cp).height
    }

    pub fn paragraph(&mut self, p: &Element, cell_last: bool) -> Vec<Block> {
        self.stat("paragraphs");
        let page_last = std::mem::replace(&mut self.page_last, false); // nested (cell) paragraphs: false
        let runs_el: Vec<&Element> = p.findall("hp:run").collect();
        let ctx = self.para_layout(p, &runs_el, cell_last, page_last);
        let mut acc = ParaOut { para_top: self.para_top.take(), first: true, ..Default::default() };
        if truthy(p.get("columnBreak")) {
            acc.runs.push("<w:r><w:br w:type=\"column\"/></w:r>".into());
        }
        let outer_shape = self.run_shape;
        for run in &runs_el {
            let rpr = self.rpr(run.get("charPrIDRef"));
            let shape = num(run.get("charPrIDRef"));
            for child in &run.children {
                // set for each child: a story inside one (a text box) sets its own
                self.run_shape = Some(shape);
                self.para_child(child, &rpr, &ctx, &mut acc);
            }
        }
        self.run_shape = outer_shape;
        let mut blocks = std::mem::take(&mut acc.blocks);
        if !acc.runs.is_empty() || blocks.is_empty() {
            let ppr = self.ppr(&ctx, ctx.page_break && acc.first);
            blocks.push(Block::P(P::new(ppr, std::mem::take(&mut acc.runs))));
        }
        if cell_last
            && ctx.text_only
            && !ctx.trim
            && let Some(Block::P(last)) = blocks.last_mut()
        {
            let v = ctx.pp.ls_value;
            last.tail = match ctx.pp.ls_type.as_str() {
                "PERCENT" => floor_div(ctx.font_h * (v - 100).max(0), 100),
                "BETWEEN_LINES" => v.max(0),
                _ => 0,
            };
        }
        self.mark_origin(p, &mut blocks, ctx.pp.keep_words);
        let mut out = acc.cap_above;
        out.extend(blocks);
        out.extend(acc.cap_below);
        out
    }

    /// How a paragraph's lines are laid out (see [`ParaCtx`]).
    fn para_layout(&mut self, p: &Element, runs_el: &[&Element], cell_last: bool, page_last: bool) -> ParaCtx {
        // The section's page, given in its first paragraph, is read first: the tables below are
        // measured against its line width (a later read of the same settings does nothing).
        if let Some(sp) = runs_el.iter().flat_map(|r| r.children.iter()).find(|c| c.tag == "hp:secPr") {
            self.sec_pr(sp);
        }
        let pp = self.h.para_pr(p.get("paraPrIDRef"));
        let style_id = p.get("styleIDRef").map(str::to_string);
        let mut heights: Vec<i64> = runs_el.iter().map(|r| self.h.char_pr(r.get("charPrIDRef")).height).collect();
        if heights.is_empty() {
            heights.push(self.style_height(style_id.as_deref()));
        }
        let max_h = heights.iter().copied().max().unwrap_or(1000);
        let mut font_h = main_height(runs_el, &heights, max_h);
        let mark_rpr = runs_el.last().map(|r| self.rpr(r.get("charPrIDRef"))).unwrap_or_default();
        // Hancom lays text lines at exactly font * percent; WordCraft would stretch a line to
        // the font's natural height, so text-only paragraphs get an exact line height.
        // Paragraphs with objects keep a minimum height so pictures are never clipped. Inside
        // a cell, a one-line last paragraph is exactly one font height tall: Hancom adds no
        // line spacing after a cell's last line (its layout cache, hp:lineseg). Tables set
        // "like a character" that share their line with text or other tables (boxes for a
        // member number) stay on the line inside inline text boxes; a DOCX table always starts
        // a block of its own.
        let tables: Vec<&Element> = runs_el.iter().flat_map(|r| r.children.iter()).filter(|c| c.tag == "hp:tbl").collect();
        let mut inline_tables = !tables.is_empty()
            && tables.iter().all(|t| t.find("hp:pos").is_some_and(|pos| truthy(pos.get("treatAsChar"))))
            && (tables.len() > 1 || runs_el.iter().flat_map(|r| r.findall("hp:t")).any(|t| !t.text.trim().is_empty()));
        if inline_tables {
            // Only tables that share a line: wider ones go one under the other in Hancom too.
            let widths: i64 = tables.iter().map(|t| t.find("hp:sz").map(|s| num(s.get("width"))).unwrap_or(0)).sum();
            inline_tables = tw(widths) <= self.line_width();
        }
        let text_only = !inline_tables
            && !runs_el.iter().flat_map(|r| r.children.iter()).any(|c| {
                c.tag == "hp:pic"
                    || SHAPES.contains(&c.tag.as_str())
                    || FORMS.contains(&c.tag.as_str())
                    || matches!(c.tag.as_str(), "hp:container" | "hp:equation" | "hp:ole")
            });
        let segs = p.find("hp:linesegarray");
        let segs_len = segs.map(|s| s.children.len());
        if segs_len == Some(1) {
            // One line holds every character, the tallest included, and a bullet or number in
            // its own character shape: Hancom's layout cache has the result.
            font_h = max_h;
            let cached = segs.and_then(|s| s.children.first()).map(|s| num(s.get("vertsize"))).unwrap_or(0);
            if text_only && font_h <= cached && cached <= floor_div(font_h * 3, 2) {
                font_h = cached;
            }
        }
        // The line spacing after a page's last line takes no room in Hancom either.
        let mut one_line = segs_len == Some(1);
        if segs.is_none()
            && cell_last
            && let Some(text_width) = self.text_width.filter(|w| *w != 0)
        {
            // No layout cache (some generators leave it out): one line if the text is clearly
            // narrower than the cell (full-width characters 1 em, others 0.6 em).
            let text: String = runs_el
                .iter()
                .flat_map(|r| r.findall("hp:t"))
                .map(|t| format!("{}{}", t.text, t.children.iter().map(|c| c.tail.as_str()).collect::<String>()))
                .collect();
            let em = font_h as f64 / 5.0;
            let width: f64 = text.chars().map(|ch| if ch as u32 >= 0x1100 { em } else { em * 0.6 }).sum();
            one_line = !text.contains('\n') && width <= text_width as f64 * 0.9;
        }
        let trim = (cell_last || page_last) && text_only && one_line;
        ParaCtx { pp, style_id, font_h, page_break: truthy(p.get("pageBreak")), mark_rpr, trim, text_only, inline_tables }
    }

    /// One child of a paragraph's run.
    fn para_child(&mut self, child: &Element, rpr: &str, ctx: &ParaCtx, acc: &mut ParaOut) {
        let tag = child.tag.as_str();
        match tag {
            "hp:t" => {
                // spaces opening the paragraph (an indent typed as spaces) keep their full width:
                // Hangul shrinks spaces only to fit a line, which these hardly ever decide
                let leading = !acc.runs.iter().any(|r| r.contains("<w:t"));
                let t = self.text(child, rpr, (ctx.pp.condense, leading));
                acc.runs.extend(t);
            }
            "hp:ctrl" => {
                let c = self.ctrl(child, rpr);
                acc.runs.extend(c);
                if self.click_open.take().is_some() {
                    acc.click_start = Some(acc.runs.len());
                }
                if let Some(guide) = self.click_close.take() {
                    // An unfilled input field (Hancom: 누름틀) shows its guide text only while editing in Hancom (the
                    // viewer shows nothing); hide it, keeping the text.
                    if let Some(tail) = acc.click_start.and_then(|cs| acc.runs.get_mut(cs..))
                        && no_spaces(&unescape_xml(&w_t_texts(&tail.concat()))) == no_spaces(&guide)
                    {
                        let hidden = hide_runs(tail);
                        tail.clone_from_slice(&hidden);
                    }
                    acc.click_start = None;
                }
            }
            "hp:secPr" => self.sec_pr(child),
            "hp:tbl" => self.para_table(child, rpr, ctx, acc),
            "hp:equation" => acc.runs.push(self.equation(child, rpr)),
            "hp:compose" => {
                let c = self.compose(child, rpr);
                acc.runs.extend(c);
            }
            _ if tag == "hp:pic" || SHAPES.contains(&tag) || tag == "hp:container" => {
                let d = self.drawing(child, rpr);
                acc.runs.extend(d);
                // The caption of an object on a text line: its own paragraphs above or below
                // that line.
                if child.find("hp:pos").is_some_and(|x| truthy(x.get("treatAsChar")))
                    && let Some((side, cap)) = self.caption(child)
                    && !cap.is_empty()
                {
                    if side == "TOP" { acc.cap_above.extend(cap) } else { acc.cap_below.extend(cap) }
                }
            }
            _ if FORMS.contains(&tag) => {
                let f = self.form(child, rpr);
                acc.runs.extend(f);
            }
            _ => self.missing(tag),
        }
    }

    fn para_table(&mut self, child: &Element, rpr: &str, ctx: &ParaCtx, acc: &mut ParaOut) {
        let pos = child.find("hp:pos");
        let over = matches!(child.get("textWrap"), Some("BEHIND_TEXT" | "IN_FRONT_OF_TEXT")) && pos.is_some_and(|x| !truthy(x.get("treatAsChar")));
        if ctx.inline_tables || over {
            let r = self.inline_table(child, rpr);
            acc.runs.extend(r);
            return;
        }
        self.table_top = acc.para_top.take(); // only the first table's is known
        self.table_para_align = Some(ctx.pp.align.clone());
        let items = self.table(child, false);
        if items.is_empty() {
            return;
        }
        // flush the text before the table
        if !acc.runs.is_empty() {
            let page_break = ctx.page_break && acc.first;
            // Floating objects alone before the table are placed from their paragraph's top,
            // which in Hancom is the table's line: a paragraph of (almost) no height keeps them
            // there instead of a line higher.
            let floating_only = acc.runs.iter().any(|r| r.contains("<wp:anchor ")) && acc.runs.iter().all(|r| !shows_text(r));
            let ppr = if floating_only && !page_break && !ctx.pp.page_break_before { TINY_PPR.to_string() } else { self.ppr(ctx, page_break) };
            acc.blocks.push(Block::P(P::new(ppr, std::mem::take(&mut acc.runs))));
            acc.first = false;
        }
        if acc.first && (ctx.page_break || ctx.pp.page_break_before) {
            // The paragraph starts with a table: a table cannot carry the page break, so a
            // paragraph of (almost) no height does.
            acc.blocks.push(Block::P(P::new(format!("<w:pageBreakBefore/>{TINY_PPR}"), vec![])));
        }
        acc.blocks.extend(items);
        acc.first = false;
    }

    /// Hidden bookmarks naming the source paragraph ("_hp_<section>_<ordinal>" on its first
    /// DOCX paragraph, "_hq_..." on further pieces, "_ht_..." before a paragraph shown only as
    /// a table, which WordCraft moves into the table), so a document edited in WordCraft can be
    /// written back into the original. WordCraft keeps bookmarks when it saves; it drops
    /// w14:paraId. Each mark's paragraph wraps Korean words as `keep_words` says
    /// ([`Report::keep_words`]).
    fn mark_origin(&mut self, p: &Element, blocks: &mut Vec<Block>, keep_words: bool) {
        let Some(ordinal) = p.get(ORDINAL) else { return };
        let ordinal = ordinal.to_string();
        let mut first = true;
        for b in blocks.iter_mut() {
            if let Block::P(bp) = b {
                // (a caption's paragraph inside the blocks of its table carries its own mark)
                if !bp.marked {
                    let name = format!("_{}_{}_{}", if first { "hp" } else { "hq" }, self.sec_index, ordinal);
                    let bm = self.bookmark(&name);
                    bp.runs.insert(0, bm);
                    bp.marked = true;
                    self.keep_words.insert(name, keep_words);
                    first = false;
                }
            }
        }
        if first && !blocks.is_empty() {
            let bm = self.bookmark(&format!("_ht_{}_{}", self.sec_index, ordinal));
            blocks.insert(0, Block::Anchor(bm));
        }
    }

    fn bookmark(&mut self, name: &str) -> String {
        self.bm_n += 1;
        format!("<w:bookmarkStart w:id=\"{n}\" w:name=\"{name}\"/><w:bookmarkEnd w:id=\"{n}\"/>", n = self.bm_n)
    }

    // ==== text ===========================================================================
    /// The runs of `t` in `rpr`, in a paragraph whose spaces may shrink `condense` percent;
    /// `leading`: nothing of the paragraph's text came before.
    fn text(&mut self, t: &Element, rpr: &str, (condense, leading): (i64, bool)) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();
        let space_rpr = self.space_rpr(rpr, condense);
        let lead_rpr = if leading { self.space_rpr(rpr, 0) } else { None };
        let mut leading = leading;
        let mut runs: Vec<String> = Vec::new();
        // text, its spaces in runs of their own when they need other spacing
        let mut push_text = |parts: &mut Vec<String>, runs: &mut Vec<String>, s: &str| {
            let mut plain = String::new();
            for piece in split_spaces(s) {
                let spaces = piece.starts_with(' ');
                let srpr = if spaces && leading { lead_rpr.as_ref() } else { space_rpr.as_ref() };
                leading &= spaces;
                match srpr.filter(|_| spaces) {
                    Some(srpr) => {
                        if !plain.is_empty() {
                            parts.push(wt(&std::mem::take(&mut plain)));
                        }
                        if !parts.is_empty() {
                            runs.push(format!("<w:r>{rpr}{}</w:r>", parts.concat()));
                            parts.clear();
                        }
                        runs.push(format!("<w:r>{srpr}{}</w:r>", wt(piece)));
                    }
                    None => plain.push_str(piece),
                }
            }
            if !plain.is_empty() {
                parts.push(wt(&plain));
            }
        };
        if !t.text.is_empty() && !self.in_delete {
            push_text(&mut parts, &mut runs, &t.text);
        }
        for c in &t.children {
            let tag = c.tag.as_str();
            if self.in_delete && tag != "hp:deleteEnd" {
                // inside a tracked deletion
            } else {
                match tag {
                    "hp:tab" => parts.push("<w:tab/>".into()),
                    "hp:lineBreak" => parts.push("<w:br/>".into()),
                    "hp:nbSpace" => parts.push(wt("\u{a0}")),
                    "hp:fwSpace" => parts.push(wt("\u{3000}")),
                    "hp:hypen" => parts.push(wt("-")),
                    "hp:deleteBegin" => self.in_delete = true,
                    "hp:deleteEnd" => self.in_delete = false,
                    _ if SILENT_TEXT_MARKS.contains(&tag) => {}
                    _ => self.missing(tag),
                }
            }
            if !c.tail.is_empty() && !self.in_delete {
                push_text(&mut parts, &mut runs, &c.tail);
            }
        }
        if !parts.is_empty() {
            runs.push(format!("<w:r>{rpr}{}</w:r>", parts.concat()));
        }
        runs
    }

    /// Overlapped characters (Hancom: 글자 겹치기), used in documents mostly for circled numbers. Circled
    /// numbers above 18 come as a tens glyph (U+F0288 + tens) and a units glyph (U+F0292 +
    /// units) from Hancom's private-use range; inferred from consecutive clause numbers in real
    /// documents and checked against the Hancom Viewer.
    fn compose(&mut self, el: &Element, rpr: &str) -> Vec<String> {
        let mut text = el.get("composeText").unwrap_or("").to_string();
        let codes: Vec<u32> = text.chars().map(|c| c as u32).collect();
        let private = |c: u32| (0xE000..=0xF8FF).contains(&c) || c >= 0xF0000;
        if let [a, b] = codes.as_slice()
            && (0xF0288..=0xF0291).contains(a)
            && (0xF0292..=0xF029B).contains(b)
            && el.get("circleType").unwrap_or("CHAR") == "CHAR"
        {
            text = circled(i64::from(a - 0xF0288) * 10 + i64::from(b - 0xF0292));
            return if text.is_empty() || self.in_delete { vec![] } else { vec![format!("<w:r>{rpr}{}</w:r>", wt(&text))] };
        }
        if codes.iter().any(|&c| private(c)) {
            self.missing("hp:compose (unknown glyphs)");
            text = text.chars().filter(|c| !private(*c as u32)).collect();
        }
        if text.is_empty() || self.in_delete { vec![] } else { vec![format!("<w:r>{rpr}{}</w:r>", wt(&text))] }
    }

    pub fn rpr(&mut self, cp_id: Option<&str>) -> String {
        let key = num(cp_id);
        if let Some(r) = self.rpr_cache.get(&key).cloned() {
            // two shapes can give the same rPr with different space widths: the rPr now stands
            // for this run's shape (`space_rpr` is asked right after, for the same run)
            self.rpr_ids.insert(r.clone(), key);
            return r;
        }
        let inner = self.rpr_inner(&self.h.char_pr_id(key));
        let r = if inner.is_empty() { String::new() } else { format!("<w:rPr>{inner}</w:rPr>") };
        self.rpr_cache.insert(key, r.clone());
        self.rpr_ids.insert(r.clone(), key);
        r
    }

    /// The rPr for the spaces of text in `rpr` (one [`Converter::rpr`] made), when they need
    /// other spacing than its other characters. Hangul draws a space half the character height
    /// wide, scaled by the width ratio, with the character spacing taken from that half width
    /// (`useFontSpace` off, as in every sample; viewer, measured at 20 pt: 10, 9 at 90 %, 9 at
    /// -10 %, 8 at both), while WordCraft gives it the font's own space and the run's spacing in
    /// points. The difference goes into the spaces' spacing.
    ///
    /// A paragraph's minimum space width (Hancom: 공백 최소값; `condense`, percent) lets Hangul shrink spaces that much to fit
    /// a line (viewer, measured: a line 8.2 % of its spaces too long stays whole from 9 on, not
    /// at 8). WordCraft's spaces do not shrink, so they get the shrunk width: lines then break
    /// where Hangul breaks them, and justified lines stretch the spaces back.
    fn space_rpr(&mut self, rpr: &str, condense: i64) -> Option<String> {
        let key = match self.run_shape {
            Some(k) => k,
            None => *self.rpr_ids.get(rpr)?,
        };
        if let Some(r) = self.space_rprs.get(&(key, condense)) {
            return r.clone();
        }
        let cp = self.h.char_pr_id(key);
        let r = self.space_spacing(&cp, condense).map(|tw| {
            let inner = self.rpr_inner_spaced(&cp, Some(tw));
            format!("<w:rPr>{inner}</w:rPr>")
        });
        self.space_rprs.insert((key, condense), r.clone());
        r
    }

    /// The spacing (twips) that makes a space of `cp` as wide as Hangul lays it out, shrunk by
    /// `condense` percent, if that differs from what its other characters get.
    fn space_spacing(&self, cp: &CharPr, condense: i64) -> Option<i64> {
        if cp.font_space {
            return None;
        }
        let family =
            self.h.font_face("latin", cp.fonts.get("latin").copied()).or_else(|| self.h.font_face("hangul", cp.fonts.get("hangul").copied()))?;
        let face = wordcraft_fonts::FontDb::global().face(family, if cp.bold { "Bold" } else { "Regular" });
        let upem = face.units_per_em();
        if upem <= 0.0 {
            return None;
        }
        let own = face.advance(face.glyph_for(' ')) / upem;
        let (size, ratio, spacing) = (cp.height as f64 / 100.0, cp.ratio as f64 / 100.0, cp.spacing as f64 / 100.0);
        let shrink = 1.0 - condense.clamp(0, 75) as f64 / 100.0;
        let points = size * ((0.5 * ratio + 0.5 * spacing) * shrink - own * ratio);
        let tw = round_even(points * 20.0);
        let usual = round_even(cp.spacing as f64 * cp.height as f64 / 500.0);
        (tw != usual).then_some(tw)
    }

    fn rpr_inner(&self, cp: &CharPr) -> String {
        self.rpr_inner_spaced(cp, None)
    }

    /// [`Converter::rpr_inner`] with the character spacing `spacing` (twips) instead of the
    /// shape's own.
    fn rpr_inner_spaced(&self, cp: &CharPr, spacing: Option<i64>) -> String {
        let latin = self.h.font_face("latin", cp.fonts.get("latin").copied());
        let hangul = self.h.font_face("hangul", cp.fonts.get("hangul").copied());
        let mut o = String::new();
        if latin.is_some() || hangul.is_some() {
            let a = latin.or(hangul).unwrap_or("");
            let e = hangul.or(latin).unwrap_or("");
            // A swapped font's own name rides in the complex-script slot (unused by Korean and
            // Latin text), so a new HWPX gets the original back (see crate::fonts).
            let own = |lang: &str| cp.fonts.get(lang).and_then(|id| self.font_names.get(lang)?.get(id)).map(String::as_str);
            let cs = match (own("hangul"), own("latin")) {
                (Some(o), _) if Some(o) != hangul => o,
                (_, Some(o)) if Some(o) != latin => o,
                _ => a,
            };
            o.push_str(&format!("<w:rFonts w:ascii={} w:hAnsi={} w:eastAsia={} w:cs={}/>", attr(a), attr(a), attr(e), attr(cs)));
        }
        if cp.bold {
            o.push_str("<w:b/><w:bCs/>");
        }
        if cp.italic {
            o.push_str("<w:i/><w:iCs/>");
        }
        if let Some(s) = &cp.strike {
            o.push_str(if s == "DOUBLE_SLIM" { "<w:dstrike/>" } else { "<w:strike/>" });
        }
        if cp.outline {
            o.push_str("<w:outline/>");
        }
        if cp.shadow {
            o.push_str("<w:shadow/>");
        }
        if cp.emboss {
            o.push_str("<w:emboss/>");
        }
        if cp.engrave {
            o.push_str("<w:imprint/>");
        }
        if let Some(c) = &cp.text_color {
            o.push_str(&format!("<w:color w:val=\"{c}\"/>"));
        }
        let spacing = spacing.unwrap_or_else(|| round_even(cp.spacing as f64 * cp.height as f64 / 500.0));
        if spacing != 0 {
            o.push_str(&format!("<w:spacing w:val=\"{spacing}\"/>"));
        }
        if cp.ratio != 100 {
            o.push_str(&format!("<w:w w:val=\"{}\"/>", cp.ratio.clamp(1, 600)));
        }
        let hp = round_even(cp.height as f64 / 50.0).max(2);
        o.push_str(&format!("<w:sz w:val=\"{hp}\"/><w:szCs w:val=\"{hp}\"/>"));
        if let Some(u) = &cp.underline {
            let col = cp.underline_color.as_ref().map(|c| format!(" w:color=\"{c}\"")).unwrap_or_default();
            o.push_str(&format!("<w:u w:val=\"{}\"{col}/>", underline(u)));
        }
        if let Some(s) = cp.shade_color.as_deref().filter(|s| *s != "FFFFFF") {
            o.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{s}\"/>"));
        }
        if cp.sup {
            o.push_str("<w:vertAlign w:val=\"superscript\"/>");
        } else if cp.sub {
            o.push_str("<w:vertAlign w:val=\"subscript\"/>");
        }
        if !matches!(cp.sym_mark.as_str(), "NONE" | "") {
            o.push_str("<w:em w:val=\"dot\"/>");
        }
        o
    }

    /// A body paragraph's pPr; text-only paragraphs get exact line heights (see paragraph()).
    fn ppr(&mut self, ctx: &ParaCtx, page_break: bool) -> String {
        let mut o = String::new();
        if let Some(sid) = ctx.style_id.as_deref()
            && self.h.style(num(Some(sid))).is_some()
        {
            o.push_str(&format!("<w:pStyle w:val=\"S{}\"/>", num(Some(sid))));
        }
        o.push_str(&self.ppr_inner(&ctx.pp, ctx.font_h, page_break, true, ctx.trim, ctx.text_only));
        o.push_str(&ctx.mark_rpr);
        o
    }

    fn ppr_inner(&mut self, pp: &ParaPr, font_h: i64, page_break: bool, numbering: bool, trim: bool, exact: bool) -> String {
        let pp = &pp.in_hwpunit(font_h);
        let mut o = String::new();
        if pp.keep_next {
            o.push_str("<w:keepNext/>");
        }
        if pp.keep_lines {
            o.push_str("<w:keepLines/>");
        }
        if page_break || pp.page_break_before {
            o.push_str("<w:pageBreakBefore/>");
        }
        o.push_str(if pp.widow { "<w:widowControl/>" } else { "<w:widowControl w:val=\"0\"/>" });
        if numbering {
            o.push_str(&self.num_pr(pp));
        }
        if pp.border_fill != 0
            && let Some(bf) = self.h.border_fills.get(&pp.border_fill)
        {
            let sides: String = [("top", &bf.top), ("left", &bf.left), ("bottom", &bf.bottom), ("right", &bf.right)]
                .iter()
                .filter(|(_, b)| b.visible())
                .map(|(t, b)| border_xml(t, b))
                .collect();
            if !sides.is_empty() {
                o.push_str(&format!("<w:pBdr>{sides}</w:pBdr>"));
            }
            if let Some(f) = bf.fill.as_deref().filter(|f| *f != "FFFFFF") {
                o.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{f}\"/>"));
            }
        }
        let tabs = self.h.tab_prs.get(&pp.tab_pr).cloned().unwrap_or_default();
        let mut items: Vec<String> = tabs
            .iter()
            .filter(|t| t.pos > 0)
            .map(|t| format!("<w:tab w:val=\"{}\" w:leader=\"{}\" w:pos=\"{}\"/>", tab_type(&t.kind), tab_leader(&t.leader), tw(t.pos)))
            .collect();
        if self.h.auto_tab_right.contains(&pp.tab_pr) {
            // The automatic tab at the right end (Hancom: 오른쪽 끝 자동 탭): a right tab where the line ends (DOCX tab positions count from
            // the text column's left edge).
            let end = self.line_width() - tw(pp.right);
            if end > 0 && tabs.iter().all(|t| tw(t.pos) < end) {
                items.push(format!("<w:tab w:val=\"right\" w:leader=\"none\" w:pos=\"{end}\"/>"));
            }
        }
        if !items.is_empty() {
            o.push_str(&format!("<w:tabs>{}</w:tabs>", items.concat()));
        }
        // Line spacing: HWP percentages are relative to the font size (measured: 10pt at 160%
        // -> 16pt pitch). exact=true for text-only paragraphs, see paragraph().
        let v = pp.ls_value;
        let flow = if exact { "exact" } else { "atLeast" };
        let (line, rule) = match pp.ls_type.as_str() {
            "FIXED" => (tw(v), "exact"),
            "AT_LEAST" => (tw(v), "atLeast"),
            _ if trim => (tw(font_h), "exact"),
            "BETWEEN_LINES" => (tw(font_h + v), flow),
            _ => (tw_f(font_h as f64 * v as f64 / 100.0), flow),
        };
        o.push_str(&format!(
            "<w:spacing w:before=\"{}\" w:after=\"{}\" w:line=\"{}\" w:lineRule=\"{rule}\"/>",
            tw(pp.before).max(0),
            tw(pp.after).max(0),
            line.max(20)
        ));
        let (mut left, right, ind) = (tw(pp.left), tw(pp.right), tw(pp.indent));
        let mut extra = String::new();
        if ind > 0 {
            extra = format!(" w:firstLine=\"{ind}\"");
        } else if ind < 0 {
            left += -ind;
            extra = format!(" w:hanging=\"{}\"", -ind);
        }
        o.push_str(&format!("<w:ind w:left=\"{left}\" w:right=\"{right}\"{extra}/>"));
        o.push_str(&format!("<w:jc w:val=\"{}\"/>", jc(&pp.align)));
        // (Korean word wrapping has no DOCX equivalent: it goes by Report::keep_words)
        if pp.heading_type == "OUTLINE" {
            o.push_str(&format!("<w:outlineLvl w:val=\"{}\"/>", pp.heading_level.clamp(0, 8)));
        }
        o
    }

    // ==== numbering ======================================================================
    fn num_pr(&mut self, pp: &ParaPr) -> String {
        let key = match pp.heading_type.as_str() {
            "OUTLINE" => match self.sec.outline {
                Some(o) if self.h.numberings.contains_key(&o) => ("num", o),
                _ => return String::new(),
            },
            "NUMBER" if self.h.numberings.contains_key(&pp.heading_ref) => ("num", pp.heading_ref),
            "BULLET" if self.h.bullets.contains_key(&pp.heading_ref) => ("bul", pp.heading_ref),
            _ => return String::new(),
        };
        let id = match self.num_ids.iter().find(|(k, _)| *k == key) {
            Some((_, id)) => *id,
            None => {
                let id = self.num_ids.len() as i64 + 1;
                self.num_ids.push((key, id));
                id
            }
        };
        format!("<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{id}\"/></w:numPr>", pp.heading_level.clamp(0, 8))
    }

    fn write_numbering(&mut self) {
        if self.num_ids.is_empty() {
            return;
        }
        let (mut abstracts, mut nums) = (String::new(), String::new());
        for ((kind, r), nid) in self.num_ids.clone() {
            let mut lvls = String::new();
            for i in 0..9i64 {
                let body = if kind == "bul" {
                    let ch = bullet_char(self.h.bullets.get(&r).and_then(|c| c.chars().next()).unwrap_or('•'));
                    format!(
                        "<w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:suff w:val=\"space\"/><w:lvlText w:val={}/><w:lvlJc w:val=\"left\"/>",
                        attr(&ch.to_string())
                    )
                } else {
                    let lv = self.h.numberings.get(&r).and_then(|n| n.levels.get(&(i + 1)));
                    let fmt = lv.map(|l| num_fmt(&l.fmt)).unwrap_or("decimal");
                    let text = lv.map(|l| l.text.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("^{}.", i + 1));
                    let text = level_placeholders(&text.replace('%', "%%"));
                    let start = lv.map(|l| l.start).filter(|s| *s != 0).unwrap_or(1);
                    format!(
                        "<w:start w:val=\"{start}\"/><w:numFmt w:val=\"{fmt}\"/><w:suff w:val=\"space\"/><w:lvlText w:val={}/><w:lvlJc w:val=\"left\"/>",
                        attr(&text)
                    )
                };
                lvls.push_str(&format!("<w:lvl w:ilvl=\"{i}\">{body}</w:lvl>"));
            }
            abstracts.push_str(&format!("<w:abstractNum w:abstractNumId=\"{nid}\"><w:multiLevelType w:val=\"multilevel\"/>{lvls}</w:abstractNum>"));
            nums.push_str(&format!("<w:num w:numId=\"{nid}\"><w:abstractNumId w:val=\"{nid}\"/></w:num>"));
        }
        let xml = format!("{DECL}<w:numbering {W_NS}>{abstracts}{nums}</w:numbering>");
        self.out.put("word/numbering.xml", &xml, Some(&ct("numbering")));
        self.out.part(DOC_PART).add_rel(&rel("numbering"), "numbering.xml", false);
    }

    // ==== controls =======================================================================
    fn ctrl(&mut self, el: &Element, rpr: &str) -> Vec<String> {
        let mut out = Vec::new();
        for c in &el.children {
            let tag = c.tag.as_str();
            match tag {
                "hp:colPr" => {} // handled per column region, see direct_col_pr()
                "hp:header" | "hp:footer" => self.header_footer(c, if tag == "hp:header" { "header" } else { "footer" }),
                "hp:footNote" | "hp:endNote" => {
                    let r = self.note(c, if tag == "hp:footNote" { "footnote" } else { "endnote" });
                    out.push(r);
                }
                "hp:fieldBegin" => {
                    let r = self.field_begin(c, rpr);
                    out.extend(r);
                }
                "hp:fieldEnd" => {
                    for k in [c.get("beginIDRef").unwrap_or(""), c.get("fieldid").unwrap_or("")] {
                        if let Some(g) = self.click_fields.remove(k) {
                            self.click_close = Some(g);
                        }
                        if let Some(cid) = self.memo_fields.remove(k) {
                            out.push(format!("<w:commentRangeEnd w:id=\"{cid}\"/><w:r><w:commentReference w:id=\"{cid}\"/></w:r>"));
                        }
                    }
                    let a = self.fields.remove(c.get("beginIDRef").unwrap_or("")).unwrap_or(false);
                    let b = a || self.fields.remove(c.get("fieldid").unwrap_or("")).unwrap_or(false);
                    if b {
                        out.push(format!("<w:r>{rpr}<w:fldChar w:fldCharType=\"end\"/></w:r>"));
                    }
                }
                "hp:bookmark" => {
                    self.bm_n += 1;
                    let raw = c.get("name").map(str::to_string).unwrap_or_else(|| format!("bm{}", self.bm_n));
                    let name: String = squeeze_spaces(&raw, "_").chars().take(40).collect();
                    out.push(format!("<w:bookmarkStart w:id=\"{n}\" w:name={}/><w:bookmarkEnd w:id=\"{n}\"/>", attr(&name), n = self.bm_n));
                }
                "hp:autoNum" => {
                    let r = self.auto_num(c, rpr);
                    out.extend(r);
                }
                "hp:pageNum" => {
                    self.sec.page_num = Some((
                        c.get("pos").unwrap_or("BOTTOM_CENTER").to_string(),
                        c.get("formatType").unwrap_or("DIGIT").to_string(),
                        c.get("sideChar").unwrap_or("").to_string(),
                    ));
                }
                "hp:newNum" if c.get("numType") == Some("PAGE") && self.on_first_page() => {
                    self.sec.start_page = num_or(c.get("num"), 1).max(1);
                }
                "hp:pageHiding" if self.on_first_page() => {
                    self.sec.hide_first = Some(HideFirst {
                        header: truthy(c.get("hideHeader")),
                        footer: truthy(c.get("hideFooter")),
                        pagenum: truthy(c.get("hidePageNum")),
                    });
                }
                _ => self.missing(tag),
            }
        }
        out
    }

    /// Page controls on a section's first page can be expressed in DOCX (titlePg, pgNumType).
    fn on_first_page(&self) -> bool {
        self.part == DOC_PART && self.sec.page_index == 0
    }

    fn auto_num(&mut self, el: &Element, rpr: &str) -> Vec<String> {
        let t = el.get("numType").unwrap_or("");
        match t {
            "PAGE" => field_runs("PAGE", rpr, "1"),
            "TOTAL_PAGE" => field_runs("NUMPAGES", rpr, "1"),
            "FOOTNOTE" | "ENDNOTE" if self.note_kind.is_some() => {
                self.note_ref_done = true;
                let tag = if self.note_kind == Some("footnote") { "footnoteRef" } else { "endnoteRef" };
                vec![format!("<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:{tag}/></w:r>")]
            }
            "PICTURE" | "TABLE" | "EQUATION" => {
                // Caption numbers count each kind through the document (written as text).
                let n = self.object_numbers.entry(t.to_string()).or_insert(0);
                *n += 1;
                vec![format!("<w:r>{rpr}{}</w:r>", wt(&n.to_string()))]
            }
            _ => {
                self.missing(&format!("autoNum:{t}"));
                vec![]
            }
        }
    }

    fn field_begin(&mut self, el: &Element, rpr: &str) -> Vec<String> {
        let ids: Vec<String> = [el.get("id"), el.get("fieldid")].into_iter().flatten().filter(|k| !k.is_empty()).map(str::to_string).collect();
        let url = if el.get("type") == Some("HYPERLINK") { hyperlink_url(el) } else { None };
        for k in &ids {
            self.fields.insert(k.clone(), url.is_some());
        }
        if el.get("type") == Some("CLICK_HERE") {
            let guide = click_here_guide(el);
            if !guide.is_empty() {
                for k in &ids {
                    self.click_fields.insert(k.clone(), guide.clone());
                }
                self.click_open = Some(guide);
            }
        }
        if let Some(memo) = self.memo_comment(el) {
            for k in &ids {
                self.memo_fields.insert(k.clone(), memo);
            }
            return vec![format!("<w:commentRangeStart w:id=\"{memo}\"/>")];
        }
        let Some(url) = url else {
            if el.get("type") != Some("CLICK_HERE") {
                // an input field keeps its text; nothing is lost
                self.missing(&format!("field:{}", el.get("type").unwrap_or("")));
            }
            return vec![];
        };
        self.stat("hyperlinks");
        vec![
            format!("<w:r>{rpr}<w:fldChar w:fldCharType=\"begin\"/></w:r>"),
            format!("<w:r>{rpr}<w:instrText xml:space=\"preserve\"> HYPERLINK \"{}\" </w:instrText></w:r>", esc(&url)),
            format!("<w:r>{rpr}<w:fldChar w:fldCharType=\"separate\"/></w:r>"),
        ]
    }

    /// A memo field (Hancom: 메모; "MEMO/<shape>/<number>/.../<author>/...") -> a DOCX comment id; the memo's
    /// paragraphs (hp:memo, or in HWPX the field's hp:subList) become the comment.
    fn memo_comment(&mut self, el: &Element) -> Option<i64> {
        let cmd = el.iter("hp:stringParam").find(|sp| sp.get("name") == Some("Command")).map(|sp| sp.text.clone()).unwrap_or_default();
        let parts = split_unescaped(&cmd, '/');
        if !matches!(el.get("type"), Some("MEMO" | "UNKNOWN")) || !cmd.starts_with("MEMO/") || parts.len() < 3 {
            return None;
        }
        // HWP keeps the memo's paragraphs apart (hp:memo, by number); HWPX inside the field
        let sub = parts.get(2).and_then(|k| self.memos.get(*k)).or_else(|| el.find("hp:subList")).cloned();
        let saved = std::mem::replace(&mut self.part, "word/comments.xml".to_string());
        let mut blocks = match &sub {
            Some(s) => self.blocks(s.findall("hp:p"), true),
            None => vec![],
        };
        self.part = saved;
        let author = parts.get(5).map(|a| unescape_backslash(a).trim().to_string()).unwrap_or_default();
        let cid = self.comments.len() as i64;
        let xml = render_blocks(&mut blocks, true);
        // the memo's time: a FILETIME in two halves (low, high), UTC (viewer-checked)
        let half = |k: usize| parts.get(k).and_then(|v| v.trim().parse::<u32>().ok());
        let when = half(3).zip(half(4)).and_then(|(lo, hi)| iso_from_filetime((u64::from(hi) << 32) | u64::from(lo)));
        self.comments.push((if author.is_empty() { "HWP".into() } else { author }, when, xml));
        self.stat("memos");
        Some(cid)
    }

    fn note(&mut self, el: &Element, kind: &'static str) -> String {
        let nid = if kind == "footnote" { self.footnotes.len() } else { self.endnotes.len() } as i64 + 1;
        let saved_part = std::mem::replace(&mut self.part, format!("word/{kind}s.xml"));
        let saved = (self.note_kind, self.note_ref_done);
        self.note_kind = Some(kind);
        self.note_ref_done = false;
        let mut blocks = match el.find("hp:subList") {
            Some(sub) => self.blocks(sub.findall("hp:p"), false),
            None => vec![],
        };
        if !self.note_ref_done {
            let r = format!("<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:{kind}Ref/></w:r>");
            match blocks.first_mut() {
                Some(Block::P(p0)) => p0.runs.insert(0, r),
                _ => blocks.insert(0, Block::P(P::new("", vec![r]))),
            }
        }
        self.part = saved_part;
        (self.note_kind, self.note_ref_done) = saved;
        if kind == "footnote" {
            self.footnotes.push((nid, blocks))
        } else {
            self.endnotes.push((nid, blocks))
        }
        self.stat(&format!("{kind}s"));
        format!("<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:{kind}Reference w:id=\"{nid}\"/></w:r>")
    }

    fn header_footer(&mut self, el: &Element, kind: &'static str) {
        self.hf_n += 1;
        let name = format!("word/{kind}{}.xml", self.hf_n);
        let saved = std::mem::replace(&mut self.part, name.clone());
        self.out.part(&name);
        let blocks = match el.find("hp:subList") {
            Some(sub) => self.blocks(sub.findall("hp:p"), false),
            None => vec![],
        };
        self.part = saved;
        let apply = el.get("applyPageType").unwrap_or("BOTH");
        let which = if apply == "EVEN" { "even" } else { "default" };
        if apply == "EVEN" {
            self.even_odd = true;
        }
        self.sec.hf.push(Hf { kind, which, name, blocks: Some(blocks) });
        self.stat(&format!("{kind}s"));
    }

    // ==== section ========================================================================
    fn sec_pr(&mut self, el: &Element) {
        if self.sec.have_secpr {
            return;
        }
        self.sec.have_secpr = true;
        if el.get("outlineShapeIDRef").is_some() {
            self.sec.outline = Some(num(el.get("outlineShapeIDRef")));
        }
        if el.get("tabStop").is_some_and(|s| !s.is_empty()) {
            self.sec.tab_stop = num_or(el.get("tabStop"), 8000);
        }
        self.sec.vertical = el.get("textDirection").filter(|s| !s.is_empty()).unwrap_or("HORIZONTAL").starts_with("VERTICAL");
        if let Some(pg) = el.find("hp:pagePr") {
            self.sec.width = num_or(pg.get("width"), A4.0).clamp(0, MAX_PAGE_LENGTH);
            self.sec.height = num_or(pg.get("height"), A4.1).clamp(0, MAX_PAGE_LENGTH);
            // WIDELY is the usual (portrait) value; NARROWLY turns the sheet sideways.
            self.sec.landscape = pg.get("landscape") == Some("NARROWLY");
            if let Some(m) = pg.find("hp:margin") {
                let mm = &mut self.sec.margin;
                for (k, slot) in [
                    ("left", &mut mm.left),
                    ("right", &mut mm.right),
                    ("top", &mut mm.top),
                    ("bottom", &mut mm.bottom),
                    ("header", &mut mm.header),
                    ("footer", &mut mm.footer),
                    ("gutter", &mut mm.gutter),
                ] {
                    if m.get(k).is_some() {
                        // bounded, so the sums the page geometry takes of them can't overflow
                        *slot = num(m.get(k)).clamp(0, MAX_PAGE_LENGTH);
                    }
                }
            }
        }
        if let Some(sub) = el.findall("hp:masterPage").find(|x| x.get("type").unwrap_or("BOTH") == "BOTH").and_then(|mp| mp.find("hp:subList")) {
            self.sec.master = Some(sub.clone());
        }
        if let Some(pbf) = el.findall("hp:pageBorderFill").find(|x| x.get("type").unwrap_or("BOTH") == "BOTH")
            && let Some(bf) = self.h.border_fills.get(&num(pbf.get("borderFillIDRef")))
        {
            let off = pbf.find("hp:offset");
            let o = |k: &str| off.map(|x| num(x.get(k))).unwrap_or(0);
            self.sec.page_border = Some(PageBorder {
                bf: bf.clone(),
                paper: pbf.get("textBorder") == Some("PAPER"),
                area: pbf.get("fillArea").unwrap_or("PAPER").to_string(),
                off: [o("left"), o("right"), o("top"), o("bottom")],
            });
        }
        match el.find("hp:startNum") {
            Some(sn) if num(sn.get("page")) > 0 => self.sec.start_page = num(sn.get("page")),
            _ => {
                if self.stats.get("sections").copied().unwrap_or(0) == 0 && self.h.begin_page > 1 {
                    self.sec.start_page = self.h.begin_page;
                }
            }
        }
    }

    fn finish_section(&mut self) {
        if self.sec.page_num.as_ref().is_some_and(|(pos, _, _)| !matches!(pos.as_str(), "NONE" | "")) {
            self.add_page_number();
        }
        self.sec.title_pg = self.sec.hide_first.is_some_and(|h| h.header || h.footer || h.pagenum);
        let has_default_header = self.sec.hf.iter().any(|h| h.kind == "header" && h.which == "default");
        if (self.has_background() || self.sec.master.is_some()) && !has_default_header {
            let inherits = self.inherited_refs.iter().any(|(k, _)| *k == ("header", "default"));
            match self.inherited_hf.get(&("header", "default")).cloned() {
                // the earlier section's header continues here: this section takes a copy of its
                // own (with the copied pictures' relationships) to add its master page to
                Some((from, blocks)) if inherits => {
                    self.hf_n += 1;
                    let name = format!("word/header{}.xml", self.hf_n);
                    if let Some(rels) = self.out.parts.iter().find(|(n, _)| *n == from).map(|(_, p)| p.rels.clone()) {
                        self.out.part(&name).rels = rels;
                    }
                    self.sec.hf.push(Hf { kind: "header", which: "default", name, blocks: Some(blocks) });
                }
                _ if inherits => self.stat("master pages or backgrounds not shown (inherited header)"),
                _ => {
                    self.hf_n += 1;
                    self.sec.hf.push(Hf { kind: "header", which: "default", name: format!("word/header{}.xml", self.hf_n), blocks: Some(vec![]) });
                }
            }
        }
        // this section's own headers and footers, as later sections may inherit them
        for h in &self.sec.hf {
            if let Some(blocks) = &h.blocks {
                self.inherited_hf.insert((h.kind, h.which), (h.name.clone(), blocks.clone()));
            }
        }
        if let Some(mut master) = self.sec.master.take() {
            // The master page (Hancom: 바탕쪽; objects on every page, mostly placed on the paper): DOCX has no master page,
            // so its content rides in the default header, where it repeats likewise. Its "page"
            // positions count from the paper (offsets from the sheet's edges match the
            // viewer), and it lies behind everything and takes no room:
            // wrapNone + behindDoc, in paragraphs of (almost) no height.
            set_paper_positions(&mut master, 0);
            let mut hf = std::mem::take(&mut self.sec.hf);
            for h in hf.iter_mut() {
                if h.kind == "header"
                    && h.which == "default"
                    && let Some(blocks) = h.blocks.as_mut()
                {
                    let saved = std::mem::replace(&mut self.part, h.name.clone());
                    self.out.part(&h.name);
                    let mut made = self.blocks(master.findall("hp:p"), true);
                    self.part = saved;
                    for b in made.iter_mut() {
                        if let Block::P(bp) = b {
                            bp.ppr = TINY_PPR.to_string();
                            for r in bp.runs.iter_mut() {
                                *r = no_wrap(&r.replace("behindDoc=\"0\"", "behindDoc=\"1\""));
                            }
                        }
                    }
                    made.append(blocks);
                    *blocks = made;
                    self.stat("master pages");
                }
            }
            self.sec.hf = hf;
            self.sec.master = Some(master);
        }
        if self.sec.title_pg {
            self.first_page_parts();
        }
        let mut hf = std::mem::take(&mut self.sec.hf);
        for h in hf.iter_mut() {
            if h.kind == "header"
                && let Some(blocks) = h.blocks.as_mut()
                && let Some(bg) = self.page_background(&h.name)
            {
                blocks.insert(0, Block::P(P::new(TINY_PPR, vec![bg])));
            }
        }
        for h in hf.iter_mut() {
            let rel_kind = if h.kind == "header" { "header" } else { "footer" };
            let target = h.name.split_once('/').map(|(_, t)| t).unwrap_or(&h.name).to_string();
            let Some(blocks) = h.blocks.as_mut() else {
                // the same part as another reference (see first_page_parts)
                let rid = self.out.part(DOC_PART).add_rel(&rel(rel_kind), &target, false);
                set_ref(&mut self.sec.refs, (h.kind, h.which), rid);
                continue;
            };
            let tag = if h.kind == "header" { "hdr" } else { "ftr" };
            let xml = format!("{DECL}<w:{tag} {W_NS}>{}</w:{tag}>", render_blocks(blocks, true));
            self.out.put(&h.name, &xml, Some(&ct(rel_kind)));
            let rid = self.out.part(DOC_PART).add_rel(&rel(rel_kind), &target, false);
            set_ref(&mut self.sec.refs, (h.kind, h.which), rid);
        }
        self.sec.hf = hf;
        for (k, v) in self.sec.refs.clone() {
            set_ref(&mut self.inherited_refs, k, v);
        }
    }

    /// sectPr for one column region of an HWP section (the first one starts a new page).
    fn sect_xml(&self, cols: i64, gap: i64, first: bool, new_page: bool) -> String {
        let s = &self.sec;
        let mut refs_sorted = s.refs.clone();
        refs_sorted.sort_by(|a, b| (a.0.0 != "header", a.0.1).cmp(&(b.0.0 != "header", b.0.1)));
        let refs: String = refs_sorted.iter().map(|((k, w), rid)| format!("<w:{k}Reference w:type=\"{w}\" r:id=\"{rid}\"/>")).collect();
        let (mut w, mut h) = (s.width, s.height);
        let mut orient = "";
        if s.landscape {
            (w, h) = (w.max(h), w.min(h));
            orient = " w:orient=\"landscape\"";
        }
        let mut m = s.margin.clone();
        if s.vertical {
            // Hancom turns the margins with the text (a line starts at the "left" margin, at
            // the top of the sheet); DOCX margins stay on the sheet.
            let o = s.margin.clone();
            m = Margins { top: o.left, right: o.top + o.header, bottom: o.right, left: o.bottom + o.footer, header: 0, footer: 0, gutter: o.gutter };
        }
        let pg = format!("<w:pgSz w:w=\"{}\" w:h=\"{}\"{orient}/>", tw(w), tw(h));
        // HWP measures the body from top+header (and bottom+footer); the header itself starts
        // at the top margin.
        let mar = format!(
            "<w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"{}\" w:footer=\"{}\" w:gutter=\"{}\"/>",
            tw(m.top + m.header),
            tw(m.right),
            tw(m.bottom + m.footer),
            tw(m.left),
            tw(m.top),
            tw(m.bottom),
            tw(m.gutter)
        );
        let pgnum = if s.start_page != 0 && first { format!("<w:pgNumType w:start=\"{}\"/>", s.start_page) } else { String::new() };
        let col = if cols > 1 { format!("<w:cols w:num=\"{cols}\" w:space=\"{}\"/>", tw(gap)) } else { "<w:cols w:space=\"720\"/>".to_string() };
        let title = if s.title_pg && first { "<w:titlePg/>" } else { "" };
        let kind = if first || new_page { "nextPage" } else { "continuous" };
        let tdir = if s.vertical { "<w:textDirection w:val=\"tbRl\"/>" } else { "" };
        format!("<w:sectPr>{refs}<w:type w:val=\"{kind}\"/>{pg}{mar}{}{pgnum}{col}{title}{tdir}</w:sectPr>", page_borders(s))
    }

    fn has_background(&self) -> bool {
        let Some(pb) = &self.sec.page_border else { return false };
        let filled = pb.bf.image.is_some() || pb.bf.fill.as_deref().is_some_and(|f| !matches!(f, "FFFFFF" | "none"));
        filled || frame_border(&self.sec).is_some()
    }

    fn page_frame(&mut self) -> String {
        let Some(b) = frame_border(&self.sec).cloned() else { return String::new() };
        let Some(pb) = &self.sec.page_border else { return String::new() };
        let (m, off) = (self.sec.margin.clone(), pb.off);
        let s = &self.sec;
        let (w, h) = if s.landscape { (s.width.max(s.height), s.width.min(s.height)) } else { (s.width, s.height) };
        let (x, y) = (m.left - off[0], m.top + m.header - off[2]);
        let fw = w - m.left - m.right + off[0] + off[1];
        let fh = h - m.top - m.header - m.bottom - m.footer + off[2] + off[3];
        let dash = match b.kind.as_str() {
            "DOT" | "CIRCLE" => "sysDot",
            "DASH" => "dash",
            "DASH_DOT" => "dashDot",
            "DASH_DOT_DOT" => "sysDashDotDot",
            "LONG_DASH" => "lgDash",
            _ => "solid",
        };
        let line = format!(
            "<a:ln w=\"{}\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill><a:prstDash val=\"{dash}\"/></a:ln>",
            round_even(b.width_mm * 36000.0),
            b.color.as_deref().unwrap_or("000000")
        );
        self.background_box("page", x, y, fw, fh, "<a:noFill/>").replace("<a:ln><a:noFill/></a:ln>", &line)
    }

    /// The page background (Hancom: 쪽 배경; colour or picture) as a behind-text box anchored in the header: DOCX has one
    /// background for the whole document only.
    fn page_background(&mut self, header: &str) -> Option<String> {
        if !self.has_background() {
            return None;
        }
        let bf = self.sec.page_border.as_ref()?.bf.clone();
        let frame = self.page_frame();
        if bf.image.is_none() && !bf.fill.as_deref().is_some_and(|f| !matches!(f, "FFFFFF" | "none")) {
            return (!frame.is_empty()).then_some(frame);
        }
        let fill = self.page_fill(header).unwrap_or_default();
        let both = fill + &frame;
        (!both.is_empty()).then_some(both)
    }

    fn page_fill(&mut self, header: &str) -> Option<String> {
        let pb = self.sec.page_border.clone()?;
        let bf = &pb.bf;
        let s = &self.sec;
        let (mut w, mut h) = if s.landscape { (s.width.max(s.height), s.width.min(s.height)) } else { (s.width, s.height) };
        let (mut rel_from, x, y) = ("page", 0i64, 0i64);
        if pb.area != "PAPER" {
            // the fill covers the page or the border (`fillArea` PAGE or BORDER), not the
            // paper: here the body area
            let m = &s.margin;
            rel_from = "margin";
            w = w - m.left - m.right;
            h = h - m.top - m.header - m.bottom - m.footer;
        }
        if bf.image.is_none() {
            if let Some(g) = &bf.gradient {
                // A shape can show the gradient itself (same mapping as drawn shapes).
                let fill = crate::drawing::gradient_fill(&g.kind, g.angle, &g.colors);
                return Some(self.background_box(rel_from, x, y, w, h, &fill));
            }
            let fill = format!("<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>", bf.fill.as_deref().unwrap_or("FFFFFF"));
            return Some(self.background_box(rel_from, x, y, w, h, &fill));
        }
        let (id, mode) = bf.image.clone()?;
        let (name, data) = self.pkg.binary(&id)?;
        let ext = ext_of(name);
        let media = self.out.add_media(name, data, &ext);
        let rid = self.out.part(header).add_rel(&rel("image"), &media, false);
        let px = crate::docx::pixel_size(data).filter(|(a, b)| *a > 0 && *b > 0);
        let Some((pw, ph)) = px.filter(|_| mode.starts_with("TILE")) else {
            let fill = format!("<a:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>");
            return Some(self.background_box(rel_from, x, y, w, h, &fill));
        };
        // Tiles at 96 dpi (Hancom's size for a picture without resolution), laid one by one:
        // DrawingML tiling scales by the picture's resolution, which readers guess differently.
        // Tiles at the far edges are cropped to the area.
        let (tw_, th_) = (i64::from(pw) * 75, i64::from(ph) * 75);
        // Too many tiles to lay one by one (a tiny picture over a page): one box with a tiling
        // fill covers the area instead, at the size the reader gives the picture, and says so.
        let tiles = |len: i64, tile: i64| if tile > 0 { (len.max(0) + tile - 1) / tile } else { 0 };
        if tiles(w, tw_).saturating_mul(tiles(h, th_)) > MAX_BACKGROUND_TILES {
            self.missing("page background tiles at the picture's own size (tiled by the reader instead)");
            let fill = format!(
                "<a:blipFill><a:blip r:embed=\"{rid}\"/><a:tile tx=\"0\" ty=\"0\" sx=\"100000\" sy=\"100000\" flip=\"none\" algn=\"tl\"/></a:blipFill>"
            );
            return Some(self.background_box(rel_from, x, y, w, h, &fill));
        }
        let mut runs = String::new();
        let mut ty = 0i64;
        let mut guard = 0;
        while ty < h && guard < MAX_BACKGROUND_TILES {
            let mut tx = 0i64;
            while tx < w && guard < MAX_BACKGROUND_TILES {
                guard += 1;
                let (cw, ch) = (tw_.min(w - tx), th_.min(h - ty));
                let crop = format!(
                    "<a:srcRect r=\"{}\" b=\"{}\"/>",
                    round_even((1.0 - cw as f64 / tw_ as f64) * 100000.0),
                    round_even((1.0 - ch as f64 / th_ as f64) * 100000.0)
                );
                let fill = format!("<a:blipFill><a:blip r:embed=\"{rid}\"/>{crop}<a:stretch><a:fillRect/></a:stretch></a:blipFill>");
                runs.push_str(&self.background_box(rel_from, x + tx, y + ty, cw, ch, &fill));
                tx += tw_;
            }
            ty += th_;
        }
        Some(runs)
    }

    fn background_box(&mut self, rel_from: &str, x: i64, y: i64, w: i64, h: i64, fill: &str) -> String {
        self.pic_n += 1;
        format!(
            "<w:r><w:drawing><wp:anchor distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\" simplePos=\"0\" relativeHeight=\"0\" behindDoc=\"1\" locked=\"1\" layoutInCell=\"1\" allowOverlap=\"1\"><wp:simplePos x=\"0\" y=\"0\"/><wp:positionH relativeFrom=\"{rel_from}\"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom=\"{rel_from}\"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx=\"{}\" cy=\"{}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:wrapNone/><wp:docPr id=\"{n}\" name=\"Page background {n}\"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri=\"{WPS_URI}\"><wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>{fill}<a:ln><a:noFill/></a:ln></wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>",
            emu(x),
            emu(y),
            emu(w),
            emu(h),
            emu(w),
            emu(h),
            n = self.pic_n
        )
    }

    fn add_page_number(&mut self) {
        let Some((pos, fmt, side)) = self.sec.page_num.clone() else { return };
        let kind: &'static str = if pos.contains("BOTTOM") { "footer" } else { "header" };
        let jcv = if pos.ends_with("LEFT") {
            "left"
        } else if pos.ends_with("RIGHT") || pos.contains("SIDE") {
            "right"
        } else {
            "center"
        };
        let switch = match fmt.as_str() {
            "ROMAN_CAPITAL" => " \\* ROMAN",
            "ROMAN_SMALL" => " \\* roman",
            "LATIN_CAPITAL" => " \\* ALPHABETIC",
            "LATIN_SMALL" => " \\* alphabetic",
            _ => "",
        };
        let rpr = self.rpr(Some("0"));
        let mut runs = Vec::new();
        if !side.is_empty() {
            runs.push(format!("<w:r>{rpr}{}</w:r>", wt(&format!("{side} "))));
        }
        runs.extend(field_runs(&format!("PAGE{switch}"), &rpr, "1"));
        if !side.is_empty() {
            runs.push(format!("<w:r>{rpr}{}</w:r>", wt(&format!(" {side}"))));
        }
        let para = P { ppr: format!("<w:jc w:val=\"{jcv}\"/>"), runs, tail: 0, page_num: true, marked: false };
        self.sec.page_num_kind = Some(kind);
        for h in self.sec.hf.iter_mut() {
            if h.kind == kind && h.which == "default" {
                if let Some(b) = h.blocks.as_mut() {
                    b.push(Block::P(para));
                }
                return;
            }
        }
        if self.inherited_refs.iter().any(|(k, _)| *k == (kind, "default")) {
            // An earlier section's header/footer continues here. If it shows a page number it
            // does here too; if not (text only), this section takes a copy of its own, with the
            // copy's relationships, to add the number to.
            let Some((from, mut blocks)) = self.inherited_hf.get(&(kind, "default")).cloned() else { return };
            if blocks.iter().any(|b| matches!(b, Block::P(p) if p.page_num)) {
                return;
            }
            blocks.push(Block::P(para));
            self.hf_n += 1;
            let name = format!("word/{kind}{}.xml", self.hf_n);
            if let Some(rels) = self.out.parts.iter().find(|(n, _)| *n == from).map(|(_, p)| p.rels.clone()) {
                self.out.part(&name).rels = rels;
            }
            self.sec.hf.push(Hf { kind, which: "default", name, blocks: Some(blocks) });
            return;
        }
        self.hf_n += 1;
        self.sec.hf.push(Hf { kind, which: "default", name: format!("word/{kind}{}.xml", self.hf_n), blocks: Some(vec![Block::P(para)]) });
    }

    /// First-page header/footer for hp:pageHiding: empty where hidden, else the default.
    fn first_page_parts(&mut self) {
        let Some(hide) = self.sec.hide_first else { return };
        for kind in ["header", "footer"] {
            let default = self.sec.hf.iter().find(|h| h.kind == kind && h.which == "default").map(|h| (h.name.clone(), h.blocks.clone()));
            // without one of its own, the section shows an earlier section's (with that one's
            // page number, when it has one)
            let shown = default.clone().or_else(|| {
                self.inherited_refs
                    .iter()
                    .any(|(k, _)| *k == (kind, "default"))
                    .then(|| self.inherited_hf.get(&(kind, "default")).cloned().map(|(name, blocks)| (name, Some(blocks))))
                    .flatten()
            });
            let has_pn = self.sec.page_num_kind == Some(kind);
            let hidden = if kind == "header" { hide.header } else { hide.footer };
            let blocks = if hidden {
                vec![]
            } else if hide.pagenum && has_pn && shown.is_some() {
                shown
                    .as_ref()
                    .and_then(|(_, b)| b.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|b| !matches!(b, Block::P(p) if p.page_num))
                    .collect()
            } else if let Some((name, _)) = default {
                self.sec.hf.push(Hf { kind, which: "first", name, blocks: None });
                continue;
            } else if let Some((_, rid)) = self.inherited_refs.iter().find(|(k, _)| *k == (kind, "default")).cloned() {
                set_ref(&mut self.sec.refs, (kind, "first"), rid);
                continue;
            } else {
                continue;
            };
            self.hf_n += 1;
            let name = format!("word/{kind}{}.xml", self.hf_n);
            // the copied blocks' pictures and links refer to the default part's relationships
            // (rId…): the new part gets the same ones
            if let Some(rels) = shown.as_ref().and_then(|(d, _)| self.out.parts.iter().find(|(n, _)| n == d)).map(|(_, p)| p.rels.clone()) {
                self.out.part(&name).rels = rels;
            }
            self.sec.hf.push(Hf { kind, which: "first", name, blocks: Some(blocks) });
        }
    }

    // ==== forms and boxed tables ===========================================================
    /// A form object (Hancom: 양식 개체) as it prints: a box of the object's size showing its caption or text (a radio
    /// button or check box mark in front, a frame around fields and buttons).
    fn form(&mut self, el: &Element, rpr: &str) -> Vec<String> {
        let sz = el.find("hp:sz");
        let (w, h) = sz.map(|s| (num(s.get("width")), num(s.get("height")))).unwrap_or((0, 0));
        let on = !matches!(el.get("value").unwrap_or("0"), "0" | "");
        let mark = match el.tag.as_str() {
            "hp:radioBtn" => {
                if on {
                    "◉ "
                } else {
                    "○ "
                }
            }
            "hp:checkBtn" => {
                if on {
                    "☑ "
                } else {
                    "☐ "
                }
            }
            _ => "",
        };
        let caption = el.get("caption").filter(|s| !s.is_empty()).or_else(|| el.get("text")).unwrap_or("");
        let text = format!("{mark}{caption}{}", if el.tag == "hp:comboBox" { " ▾" } else { "" });
        let frpr = if el.get("charPrIDRef").is_some() { self.rpr(el.get("charPrIDRef")) } else { rpr.to_string() };
        let boxed = !matches!(el.tag.as_str(), "hp:radioBtn" | "hp:checkBtn") && el.get("drawFrame").unwrap_or("1") != "0";
        let jcv = if el.tag == "hp:btn" { "center" } else { "left" };
        let para =
            P::new(format!("<w:spacing w:before=\"0\" w:after=\"0\"/><w:jc w:val=\"{jcv}\"/>"), vec![format!("<w:r>{frpr}{}</w:r>", wt(&text))]);
        let fill = if el.tag == "hp:btn" { "<a:solidFill><a:srgbClr val=\"E0E0E0\"/></a:solidFill>" } else { "<a:noFill/>" };
        let line = if boxed { "<a:ln w=\"6350\"><a:solidFill><a:srgbClr val=\"808080\"/></a:solidFill></a:ln>" } else { "<a:ln><a:noFill/></a:ln>" };
        self.stat("form objects");
        self.pic_n += 1;
        let graphic = format!(
            "<a:graphic><a:graphicData uri=\"{WPS_URI}\"><wps:wsp><wps:cNvSpPr txBox=\"1\"/><wps:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>{fill}{line}</wps:spPr><wps:txbx><w:txbxContent>{}</w:txbxContent></wps:txbx><wps:bodyPr lIns=\"18000\" tIns=\"0\" rIns=\"18000\" bIns=\"0\" anchor=\"ctr\" wrap=\"square\"><a:noAutofit/></wps:bodyPr></wps:wsp></a:graphicData></a:graphic>",
            emu(w),
            emu(h),
            para.render("")
        );
        let place = self.placement(el);
        vec![self.frame(&graphic, rpr, &place, w as f64, h as f64)]
    }

    /// A table on a text line, or behind or in front of the text: an invisible text box
    /// (inline, or anchored without wrapping) holding the table. A DOCX floating table always
    /// pushes the text aside.
    fn inline_table(&mut self, el: &Element, rpr: &str) -> Vec<String> {
        let mut blocks = self.table(el, true);
        if blocks.is_empty() {
            return vec![];
        }
        let sz = el.find("hp:sz");
        let (w, h) = sz.map(|s| (num(s.get("width")), num(s.get("height")))).unwrap_or((0, 0));
        self.pic_n += 1;
        let mut content = render_blocks(&mut blocks, false);
        if matches!(blocks.last(), Some(Block::Table(_))) {
            content.push_str(&P::new(TINY_PPR, vec![]).render(""));
        }
        let graphic = format!(
            "<a:graphic><a:graphicData uri=\"{WPS_URI}\"><wps:wsp><wps:cNvSpPr txBox=\"1\"/><wps:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></wps:spPr><wps:txbx><w:txbxContent>{content}</w:txbxContent></wps:txbx><wps:bodyPr lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" wrap=\"square\"><a:noAutofit/></wps:bodyPr></wps:wsp></a:graphicData></a:graphic>",
            emu(w),
            emu(h)
        );
        let place = self.placement(el);
        vec![self.frame(&graphic, rpr, &place, w as f64, h as f64)]
    }

    // ==== tables ===========================================================================
    fn table(&mut self, el: &Element, boxed: bool) -> Vec<Block> {
        self.stat("tables");
        let mut top = self.table_top.take(); // before the cells' paragraphs run
        if self.depth >= MAX_NESTING {
            self.missing("nesting too deep");
            return vec![];
        }
        let mut cells: Vec<CellInfo> = Vec::new();
        for tr in el.findall("hp:tr") {
            for tc in tr.findall("hp:tc") {
                let (a, sp, sz) = (tc.find("hp:cellAddr"), tc.find("hp:cellSpan"), tc.find("hp:cellSz"));
                cells.push(CellInfo {
                    el: tc,
                    row: a.map(|a| num(a.get("rowAddr"))).unwrap_or(0).clamp(0, MAX_ROWS),
                    col: a.map(|a| num(a.get("colAddr"))).unwrap_or(0).clamp(0, MAX_COLS),
                    rs: sp.map(|s| num_or(s.get("rowSpan"), 1).max(1)).unwrap_or(1).min(MAX_ROWS),
                    cs: sp.map(|s| num_or(s.get("colSpan"), 1).max(1)).unwrap_or(1).min(MAX_COLS),
                    w: sz.map(|s| num(s.get("width"))).unwrap_or(0),
                    h: sz.map(|s| num(s.get("height"))).unwrap_or(0),
                    bf: None,
                    tmar: None,
                });
            }
        }
        if cells.is_empty() {
            return vec![];
        }
        let rows = num(el.get("rowCnt")).max(cells.iter().map(|c| c.row + c.rs).max().unwrap_or(0)).clamp(0, MAX_ROWS);
        let cols = num(el.get("colCnt")).max(cells.iter().map(|c| c.col + c.cs).max().unwrap_or(0)).clamp(0, MAX_COLS);
        let sz = el.find("hp:sz");
        let table_w = sz.map(|s| num(s.get("width"))).unwrap_or(0);
        let widths = col_widths(&cells, cols, table_w);
        let im = el.find("hp:inMargin");
        let tmar = im.map(|im| [num(im.get("top")), num(im.get("left")), num(im.get("bottom")), num(im.get("right"))]);
        for i in 0..cells.len() {
            let bf = cells.get(i).and_then(|c| self.cell_border_fill(c, el));
            if let Some(c) = cells.get_mut(i) {
                c.bf = bf;
                c.tmar = tmar;
            }
        }
        let origin: HashMap<(i64, i64), usize> = cells.iter().enumerate().map(|(i, c)| ((c.row, c.col), i)).collect();
        let mut covered: HashMap<(i64, i64), usize> = HashMap::new();
        for (i, c) in cells.iter().enumerate() {
            for r in (c.row + 1)..(c.row + c.rs).min(MAX_ROWS) {
                if self.made_cells_left <= 0 {
                    break;
                }
                self.made_cells_left -= 1;
                covered.insert((r, c.col), i);
            }
        }
        let cell_mar = match im {
            Some(im) => format!(
                "<w:tblCellMar>{}</w:tblCellMar>",
                ["top", "left", "bottom", "right"]
                    .iter()
                    .map(|k| format!("<w:{k} w:w=\"{}\" w:type=\"dxa\"/>", tw(num(im.get(k)))))
                    .collect::<String>()
            ),
            None => String::new(),
        };
        let pos = el.find("hp:pos");
        let para_align = self.table_para_align.take(); // before the cells' paragraphs run
        let (jcv, tblp, ind) = match pos.filter(|p| !truthy(p.get("treatAsChar")) && !boxed) {
            Some(pos) => self.table_position(el, pos, &widths, sz),
            // A table on a text line goes where its paragraph aligns it (a right-aligned one at
            // the right of the cell or column), as in Hancom's viewer. DOCX tables don't follow
            // their paragraph's alignment, so it becomes the table's.
            None if !boxed => match para_align.as_deref() {
                Some("RIGHT") => ("right".to_string(), String::new(), String::new()),
                Some("CENTER") => ("center".to_string(), String::new(), String::new()),
                _ => Default::default(),
            },
            None => Default::default(),
        };
        let repeat = truthy(el.get("repeatHeader"));
        let mut row_data = self.table_rows(&cells, &origin, &covered, &widths, rows, cols);
        // Hancom keeps a table that is a hair taller than the page body on one page (its line
        // runs into the bottom margin); WordCraft would push a sliver onto a new page. Measured
        // on full-page forms: up to 0.5mm over. Trim the tallest row to fit, keeping 700
        // HWPUNIT (2.5mm) free: the smallest slack that kept WordCraft from spilling the last
        // row of such forms (it rounds rows and draws borders a bit taller).
        let total: i64 = row_data.iter().map(|r| r.2).sum();
        // A table further down the page has only the rest of the body (from the layout
        // cache's position of its paragraph, plus its own offset from that paragraph).
        if let (Some(t), Some(pos)) = (top, pos)
            && pos.get("vertRelTo").unwrap_or("PARA") == "PARA"
        {
            top = Some(t + if !truthy(pos.get("treatAsChar")) { signed32(pos.get("vertOffset")).max(0) } else { 0 });
        }
        let limit = self.body_height() - 700 - top.unwrap_or(0);
        fit_rows(&mut row_data, total, limit);
        let mut rows_xml = String::new();
        for (tcs, height, _, is_header) in &row_data {
            let mut trpr = String::new();
            if *height != 0 || (repeat && *is_header) {
                trpr = format!(
                    "<w:trPr>{}{}</w:trPr>",
                    if *height != 0 { format!("<w:trHeight w:val=\"{}\" w:hRule=\"atLeast\"/>", tw(*height)) } else { String::new() },
                    if repeat && *is_header { "<w:tblHeader/>" } else { "" }
                );
            }
            rows_xml.push_str(&format!("<w:tr>{trpr}{}</w:tr>", tcs.concat()));
        }
        let grid: String = widths.iter().map(|w| format!("<w:gridCol w:w=\"{w}\"/>")).collect();
        let tblpr = format!(
            "<w:tblPr>{tblp}<w:tblW w:w=\"{}\" w:type=\"dxa\"/>{}{ind}<w:tblLayout w:type=\"fixed\"/>{cell_mar}</w:tblPr>",
            widths.iter().sum::<i64>(),
            if jcv.is_empty() { String::new() } else { format!("<w:jc w:val=\"{jcv}\"/>") }
        );
        let xml = format!("<w:tbl>{tblpr}<w:tblGrid>{grid}</w:tblGrid>{rows_xml}</w:tbl>");
        let cap = self.caption(el);
        let mut out: Vec<Block> = match cap {
            Some(("TOP", mut c)) => {
                c.push(Block::Table(xml));
                c
            }
            Some((_, c)) => {
                let mut v = vec![Block::Table(xml)];
                v.extend(c);
                v
            }
            None => vec![Block::Table(xml)],
        };
        if let Some(pos) = pos.filter(|p| !boxed && tblp.is_empty() && !truthy(p.get("treatAsChar"))) {
            // A table in the text flow takes its outer margins (and a downward offset) above
            // and below it (layout cache: next line = offset + top + height + bottom).
            let om = el.find("hp:outMargin");
            let above = signed32(pos.get("vertOffset")).max(0) + om.map(|o| num(o.get("top"))).unwrap_or(0);
            let below = om.map(|o| num(o.get("bottom"))).unwrap_or(0);
            let body = self.body_height();
            if (SPACER_MIN..body).contains(&above) {
                out.insert(0, Block::P(P::new(spacer_ppr(above), vec![])));
            }
            if (SPACER_MIN..body).contains(&below) {
                out.push(Block::P(P::new(spacer_ppr(below), vec![])));
            }
        }
        out
    }

    /// Each row: (cells XML, content height for trHeight, full row height, header row).
    fn table_rows(
        &mut self,
        cells: &[CellInfo],
        origin: &HashMap<(i64, i64), usize>,
        covered: &HashMap<(i64, i64), usize>,
        widths: &[i64],
        rows: i64,
        cols: i64,
    ) -> Vec<RowData> {
        let mut row_data: Vec<RowData> = Vec::new();
        let held: HashSet<i64> = origin.keys().chain(covered.keys()).map(|&(r, _)| r).collect();
        self.depth += 1;
        for r in 0..rows {
            // out of made-up cells: a declared row the file holds no cell of is left out
            if self.made_cells_left <= 0 && !held.contains(&r) {
                self.stat("table rows left out (too many empty cells)");
                continue;
            }
            let (mut tcs, mut c, mut height, mut vpad, mut rule, mut is_header) = (Vec::new(), 0i64, 0i64, 0i64, 0i64, false);
            while c < cols {
                if let Some(&i) = origin.get(&(r, c)) {
                    let Some(cell) = cells.get(i).cloned() else { break };
                    let (xml, cell_pad) = self.cell(&cell, widths, if cell.rs > 1 { Some("restart") } else { None });
                    tcs.push(xml);
                    if cell.rs == 1 {
                        height = height.max(cell.h);
                        vpad = vpad.max(cell_pad);
                        rule = rule.max(cell_rule(&cell, r == rows - 1));
                    }
                    is_header = is_header || truthy(cell.el.get("header"));
                    c += cell.cs;
                } else if let Some(&i) = covered.get(&(r, c)) {
                    let Some(cell) = cells.get(i).cloned() else { break };
                    tcs.push(self.cell(&cell, widths, Some("continue")).0);
                    c += cell.cs;
                } else {
                    if self.made_cells_left <= 0 {
                        // Out of made-up cells: a gap before a cell the file holds becomes one
                        // empty cell spanning it, so that cell keeps its text and its place (at
                        // most one per real cell); with none after it, the row stays short.
                        let Some(next) = (c + 1..cols).find(|&k| origin.contains_key(&(r, k)) || covered.contains_key(&(r, k))) else { break };
                        let span = next - c;
                        let w: i64 = (c..next).map(|k| widths.get(k as usize).copied().unwrap_or(20)).sum();
                        tcs.push(format!("<w:tc><w:tcPr><w:tcW w:w=\"{w}\" w:type=\"dxa\"/><w:gridSpan w:val=\"{span}\"/></w:tcPr><w:p/></w:tc>"));
                        c = next;
                        continue;
                    }
                    self.made_cells_left -= 1;
                    tcs.push(format!(
                        "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/></w:tcPr><w:p/></w:tc>",
                        widths.get(c as usize).copied().unwrap_or(20)
                    ));
                    c += 1;
                }
            }
            // HWP cell heights include the cell's top/bottom margins and borders, but WordCraft
            // adds both on top of w:trHeight (measured), so give it the content height only.
            row_data.push((tcs, (height - vpad - rule).max(0), height, is_header));
        }
        self.depth -= 1;
        row_data
    }

    /// Where a table in the text flow sits: (w:jc, w:tblpPr of a floating table, w:tblInd).
    fn table_position(&mut self, el: &Element, pos: &Element, widths: &[i64], sz: Option<&Element>) -> (String, String, String) {
        let (mut jcv, mut tblp, mut ind) = (String::new(), String::new(), String::new());
        let wrap = el.get("textWrap").unwrap_or("TOP_AND_BOTTOM");
        if wrap != "TOP_AND_BOTTOM" || matches!(pos.get("vertRelTo"), Some("PAPER" | "PAGE")) {
            // A table placed on the page (text beside it, or a paper/page position)
            // floats. DOCX lets text flow beside any floating table, so a "top and bottom"
            // table relative to its paragraph stays in the text flow instead.
            let p = self.placement(el);
            let tw_w: i64 = widths.iter().sum::<i64>() * 5;
            let sz_h = sz.map(|s| num(s.get("height"))).unwrap_or(0);
            let (x, y) = self.offsets(&p, tw_w as f64, sz_h as f64);
            let d = p.dist;
            let anchor = |f: &str| match f {
                "page" => "page",
                "margin" => "margin",
                _ => "text",
            };
            tblp = format!(
                "<w:tblpPr w:leftFromText=\"{}\" w:rightFromText=\"{}\" w:topFromText=\"{}\" w:bottomFromText=\"{}\" w:vertAnchor=\"{}\" w:horzAnchor=\"{}\" w:tblpX=\"{}\" w:tblpY=\"{}\"/>",
                floor_div(d[0], 635),
                floor_div(d[1], 635),
                floor_div(d[2], 635),
                floor_div(d[3], 635),
                anchor(&p.v_from),
                anchor(&p.h_from),
                tw(x),
                tw(y)
            );
            self.stat("floating tables");
        } else {
            jcv = match pos.get("horzAlign").unwrap_or("") {
                "CENTER" => "center",
                "RIGHT" => "right",
                _ => "",
            }
            .to_string();
            if jcv.is_empty() && num(pos.get("horzOffset")) != 0 {
                ind = format!("<w:tblInd w:w=\"{}\" w:type=\"dxa\"/>", tw(num(pos.get("horzOffset"))));
            }
        }
        (jcv, tblp, ind)
    }

    /// Twips from the left to the right edge of the text column being converted.
    pub fn line_width(&self) -> i64 {
        if let Some(w) = self.text_width {
            return w;
        }
        let (s, m) = (&self.sec, &self.sec.margin);
        let page_w = if s.landscape { s.width.max(s.height) } else { s.width };
        let (cols, gap) = self.columns;
        tw_f((page_w - m.left - m.right - gap * (cols - 1)) as f64 / cols.max(1) as f64)
    }

    /// HWPUNIT height of the current section's body (page minus margins, header, footer).
    pub fn body_height(&self) -> i64 {
        let (s, m) = (&self.sec, &self.sec.margin);
        let page_h = if s.landscape { s.width.min(s.height) } else { s.height };
        page_h - m.top - m.header - m.bottom - m.footer
    }

    /// The cell's own border/fill, overlaid by any hp:cellzone covering it. A zone sets only
    /// what it defines (borders whose type is not NONE, diagonals, fill); measured on a Hancom
    /// document whose header-cell diagonal comes from a zone.
    fn cell_border_fill(&self, c: &CellInfo, tbl: &Element) -> Option<BorderFill> {
        let mut bf = self.h.border_fills.get(&num(c.el.get("borderFillIDRef"))).cloned();
        if let Some(zl) = tbl.find("hp:cellzoneList") {
            for z in zl.findall("hp:cellzone") {
                let inside = num(z.get("startRowAddr")) <= c.row
                    && c.row <= num(z.get("endRowAddr"))
                    && num(z.get("startColAddr")) <= c.col
                    && c.col <= num(z.get("endColAddr"));
                if !inside {
                    continue;
                }
                let Some(zb) = self.h.border_fills.get(&num(z.get("borderFillIDRef"))) else { continue };
                let base = bf.clone().unwrap_or_default();
                let mut n = base.clone();
                if zb.left.visible() {
                    n.left = zb.left.clone();
                }
                if zb.right.visible() {
                    n.right = zb.right.clone();
                }
                if zb.top.visible() {
                    n.top = zb.top.clone();
                }
                if zb.bottom.visible() {
                    n.bottom = zb.bottom.clone();
                }
                n.slash = base.slash || zb.slash;
                n.back_slash = base.back_slash || zb.back_slash;
                n.diagonal = if zb.slash || zb.back_slash { zb.diagonal.clone() } else { base.diagonal.clone() };
                n.fill = zb.fill.clone().or(base.fill.clone());
                bf = Some(n);
            }
        }
        bf
    }

    /// (w:tc XML, the cell's top + bottom margin as written, HWPUNIT)
    fn cell(&mut self, c: &CellInfo, widths: &[i64], vmerge: Option<&str>) -> (String, i64) {
        let tc = c.el;
        let lo = (c.col.max(0) as usize).min(widths.len());
        let hi = ((c.col + c.cs).max(0) as usize).min(widths.len()).max(lo);
        let width: i64 = widths.get(lo..hi).map(|s| s.iter().sum()).unwrap_or(0);
        let mut o = format!("<w:tcW w:w=\"{width}\" w:type=\"dxa\"/>");
        if c.cs > 1 {
            o.push_str(&format!("<w:gridSpan w:val=\"{}\"/>", c.cs));
        }
        match vmerge {
            Some("restart") => o.push_str("<w:vMerge w:val=\"restart\"/>"),
            Some("continue") => o.push_str("<w:vMerge/>"),
            _ => {}
        }
        if let Some(bf) = &c.bf {
            let diag = format!(
                "{}{}",
                if bf.back_slash { border_xml("tl2br", &bf.diagonal) } else { String::new() },
                if bf.slash { border_xml("tr2bl", &bf.diagonal) } else { String::new() }
            );
            o.push_str(&format!(
                "<w:tcBorders>{}{}{}{}{diag}</w:tcBorders>",
                border_xml("top", &bf.top),
                border_xml("left", &bf.left),
                border_xml("bottom", &bf.bottom),
                border_xml("right", &bf.right)
            ));
            if let Some(f) = bf.fill.as_deref().filter(|f| *f != "FFFFFF") {
                o.push_str(&format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{f}\"/>"));
            }
        }
        let cm = tc.find("hp:cellMargin");
        let own = cm.is_some() && truthy(tc.get("hasMargin"));
        // top, left, bottom, right
        let mut mar = match (own, cm, c.tmar) {
            (true, Some(cm), _) => [num(cm.get("top")), num(cm.get("left")), num(cm.get("bottom")), num(cm.get("right"))],
            (_, _, Some(t)) => t,
            _ => [0, 0, 0, 0],
        };
        let sub = tc.find("hp:subList");
        let mut content = "<w:p/>".to_string();
        let mut blocks: Vec<Block> = Vec::new();
        if vmerge != Some("continue") {
            let saved = self.text_width.replace((width - tw(mar[1] + mar[3])).max(1));
            blocks = match sub {
                Some(s) => self.blocks(s.findall("hp:p"), true),
                None => vec![],
            };
            self.text_width = saved;
            content = render_blocks(&mut blocks, true);
        }
        // Hancom adds no line spacing after a cell's last line; DOCX can't drop it from one
        // line, so a multi-line last paragraph gives it back through the bottom margin.
        let tail = match blocks.last() {
            Some(Block::P(p)) => p.tail,
            _ => 0,
        };
        if tail != 0 {
            mar[2] = (mar[2] - tail).max(0);
        }
        if own || tail != 0 {
            o.push_str(&format!(
                "<w:tcMar><w:top w:w=\"{}\" w:type=\"dxa\"/><w:left w:w=\"{}\" w:type=\"dxa\"/><w:bottom w:w=\"{}\" w:type=\"dxa\"/><w:right w:w=\"{}\" w:type=\"dxa\"/></w:tcMar>",
                tw(mar[0]),
                tw(mar[1]),
                tw(mar[2]),
                tw(mar[3])
            ));
        }
        if let Some(v) = sub.and_then(|s| s.get("vertAlign")).and_then(valign) {
            o.push_str(&format!("<w:vAlign w:val=\"{v}\"/>"));
        }
        (format!("<w:tc><w:tcPr>{o}</w:tcPr>{content}</w:tc>"), mar[0] + mar[2])
    }

    pub fn caption(&mut self, el: &Element) -> Option<(&'static str, Vec<Block>)> {
        let cap = el.find("hp:caption")?;
        let blocks = match cap.find("hp:subList") {
            Some(sub) => self.blocks(sub.findall("hp:p"), false),
            None => vec![],
        };
        Some((if cap.get("side") == Some("TOP") { "TOP" } else { "BOTTOM" }, blocks))
    }

    fn equation(&mut self, el: &Element, rpr: &str) -> String {
        let script = el.find("hp:script").map(|s| s.text.trim().to_string()).unwrap_or_default();
        self.stat("equations (as text)");
        if script.is_empty() {
            return String::new();
        }
        let r = if rpr.is_empty() { "<w:rPr><w:i/></w:rPr>".to_string() } else { rpr.replace("</w:rPr>", "<w:i/></w:rPr>") };
        format!("<w:r>{r}{}</w:r>", wt(&script))
    }

    // ==== package parts ====================================================================
    fn write_styles(&mut self) {
        let base = self.h.style(0).cloned();
        let base_cp = self.h.char_pr_id(base.as_ref().map(|s| s.char_pr).unwrap_or(0));
        let base_pp = self.h.para_pr_id(base.as_ref().map(|s| s.para_pr).unwrap_or(0));
        let mut styles: Vec<String> = Vec::new();
        for st in self.h.styles.clone() {
            let sid = format!("S{}", st.id);
            let name = if !st.name.is_empty() {
                st.name.clone()
            } else if !st.eng_name.is_empty() {
                st.eng_name.clone()
            } else {
                sid.clone()
            };
            let default = if st.id == 0 { " w:default=\"1\"" } else { "" };
            if st.kind == "CHAR" {
                styles.push(format!(
                    "<w:style w:type=\"character\" w:styleId=\"{sid}\"><w:name w:val={}/><w:rPr>{}</w:rPr></w:style>",
                    attr(&name),
                    self.rpr_inner(&self.h.char_pr_id(st.char_pr))
                ));
                continue;
            }
            let cp = self.h.char_pr_id(st.char_pr);
            let pp = self.h.para_pr_id(st.para_pr);
            let nxt = if self.h.style(st.next).is_some() { format!("<w:next w:val=\"S{}\"/>", st.next) } else { String::new() };
            let ppr = self.ppr_inner(&pp, cp.height, false, false, false, false);
            styles.push(format!(
                "<w:style w:type=\"paragraph\"{default} w:styleId=\"{sid}\"><w:name w:val={}/>{nxt}<w:qFormat/><w:pPr>{ppr}</w:pPr><w:rPr>{}</w:rPr></w:style>",
                attr(&name),
                self.rpr_inner(&cp)
            ));
        }
        if self.h.style(0).is_none() {
            styles.insert(0, "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>".into());
        }
        let base_ppr = self.ppr_inner(&base_pp, base_cp.height, false, false, false, false);
        let xml = format!(
            "{DECL}<w:styles {W_NS}><w:docDefaults><w:rPrDefault><w:rPr>{}<w:lang w:val=\"ko-KR\" w:eastAsia=\"ko-KR\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr>{base_ppr}</w:pPr></w:pPrDefault></w:docDefaults>{}</w:styles>",
            self.rpr_inner(&base_cp),
            styles.concat()
        );
        self.out.put("word/styles.xml", &xml, Some(&ct("styles")));
        self.out.part(DOC_PART).add_rel(&rel("styles"), "styles.xml", false);
    }

    fn write_notes(&mut self) {
        for kind in ["footnote", "endnote"] {
            let mut notes = std::mem::take(if kind == "footnote" { &mut self.footnotes } else { &mut self.endnotes });
            if notes.is_empty() {
                continue;
            }
            let mut body = format!(
                "<w:{kind} w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:{kind}><w:{kind} w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:{kind}>"
            );
            for (nid, blocks) in notes.iter_mut() {
                body.push_str(&format!("<w:{kind} w:id=\"{nid}\">{}</w:{kind}>", render_blocks(blocks, true)));
            }
            self.out.put(&format!("word/{kind}s.xml"), &format!("{DECL}<w:{kind}s {W_NS}>{body}</w:{kind}s>"), Some(&ct(&format!("{kind}s"))));
            self.out.part(DOC_PART).add_rel(&rel(&format!("{kind}s")), &format!("{kind}s.xml"), false);
            if kind == "footnote" { self.footnotes = notes } else { self.endnotes = notes }
        }
        if !self.comments.is_empty() {
            let body: String = self
                .comments
                .iter()
                .enumerate()
                .map(|(i, (author, when, xml))| {
                    let initials: String = author.chars().take(2).collect();
                    let date = when.as_deref().map(|d| format!(" w:date={}", attr(d))).unwrap_or_default();
                    format!("<w:comment w:id=\"{i}\" w:author={} w:initials={}{date}>{xml}</w:comment>", attr(author), attr(&initials))
                })
                .collect();
            self.out.put("word/comments.xml", &format!("{DECL}<w:comments {W_NS}>{body}</w:comments>"), Some(&ct("comments")));
            self.out.part(DOC_PART).add_rel(&rel("comments"), "comments.xml", false);
        }
    }

    fn write_settings(&mut self) {
        let mut o = format!("<w:defaultTabStop w:val=\"{}\"/>", tw(self.sec.tab_stop).max(1));
        if self.even_odd {
            o.push_str("<w:evenAndOddHeaders/>");
        }
        if !self.footnotes.is_empty() {
            o.push_str("<w:footnotePr><w:footnote w:id=\"-1\"/><w:footnote w:id=\"0\"/></w:footnotePr>");
        }
        if !self.endnotes.is_empty() {
            o.push_str("<w:endnotePr><w:endnote w:id=\"-1\"/><w:endnote w:id=\"0\"/></w:endnotePr>");
        }
        o.push_str(
            "<w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat>",
        );
        self.out.put("word/settings.xml", &format!("{DECL}<w:settings {W_NS}>{o}</w:settings>"), Some(&ct("settings")));
        self.out.part(DOC_PART).add_rel(&rel("settings"), "settings.xml", false);
    }

    fn write_core(&mut self) {
        let get = |k: &str| self.pkg.metadata.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        let (title, creator) = (get("title"), get("creator"));
        let xml = format!(
            "{DECL}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:title>{}</dc:title><dc:creator>{}</dc:creator></cp:coreProperties>",
            esc(&title),
            esc(&creator)
        );
        self.out.put("docProps/core.xml", &xml, Some(CT_CORE));
    }
}

/// The most table rows read: the document model's own limit, so a table WordCraft writes (up to
/// that many rows) reads back whole. Cells a file declares but doesn't hold are bounded apart
/// (`MAX_MADE_CELLS`).
const MAX_ROWS: i64 = wordcraft_doc::table::MAX_ROWS as i64;
const MAX_COLS: i64 = 64;

/// (count, gap) of a column definition placed directly in this paragraph.
pub(crate) fn direct_col_pr(p: &Element) -> Option<(i64, i64)> {
    for r in p.findall("hp:run") {
        for ctl in r.findall("hp:ctrl") {
            if let Some(c) = ctl.find("hp:colPr") {
                return Some((num_or(c.get("colCount"), 1).clamp(1, 255), num(c.get("sameGap"))));
            }
        }
    }
    None
}

/// The page border (Hancom: 쪽 테두리) -> w:pgBorders, measured from the paper edge (body-based ones are drawn in the
/// header, see [`Converter::page_frame`]).
fn page_borders(s: &Section) -> String {
    let Some(pb) = s.page_border.as_ref().filter(|pb| pb.paper) else { return String::new() };
    let sides: String =
        [("top", &pb.bf.top, pb.off[2]), ("left", &pb.bf.left, pb.off[0]), ("bottom", &pb.bf.bottom, pb.off[3]), ("right", &pb.bf.right, pb.off[1])]
            .iter()
            .filter(|(_, b, _)| b.visible())
            .map(|(side, b, off)| border_xml(side, b).replace("w:space=\"0\"", &format!("w:space=\"{}\"", round_even(*off as f64 / 100.0).min(31))))
            .collect();
    if sides.is_empty() {
        return String::new();
    }
    format!("<w:pgBorders w:offsetFrom=\"page\">{sides}</w:pgBorders>")
}

/// The line of a page border measured from the body, or None. WordCraft does not draw
/// w:pgBorders measured from the text, and a paper-based border can sit at most 31pt from the
/// edge, so such a border becomes a box in the header.
fn frame_border(s: &Section) -> Option<&Border> {
    let pb = s.page_border.as_ref().filter(|pb| !pb.paper)?;
    [&pb.bf.top, &pb.bf.left, &pb.bf.bottom, &pb.bf.right].into_iter().find(|b| b.visible())
}

/// Master pages count "page" positions from the paper.
fn set_paper_positions(el: &mut Element, depth: usize) {
    if depth > 256 {
        return;
    }
    if el.tag == "hp:pos" {
        for k in ["horzRelTo", "vertRelTo"] {
            if el.get(k) == Some("PAGE") {
                el.set(k, "PAPER");
            }
        }
    }
    for c in el.children.iter_mut() {
        set_paper_positions(c, depth + 1);
    }
}

fn col_widths(cells: &[CellInfo], cols: i64, table_w: i64) -> Vec<i64> {
    let n = cols.max(0) as usize;
    let mut w: Vec<Option<i64>> = vec![None; n];
    for c in cells {
        if c.cs == 1
            && c.col < cols
            && c.w > 0
            && let Some(slot) = w.get_mut(c.col as usize)
            && slot.is_none()
        {
            *slot = Some(c.w);
        }
    }
    let mut changed = true;
    let mut guard = 0;
    while changed && guard < 10_000 {
        changed = false;
        guard += 1;
        for c in cells {
            let span = (c.col.max(0) as usize)..((c.col + c.cs).min(cols).max(0) as usize);
            let unknown: Vec<usize> = span.clone().filter(|i| w.get(*i).is_some_and(|x| x.is_none())).collect();
            if unknown.len() == 1 && c.w > 0 {
                let known: i64 = span.filter_map(|i| w.get(i).copied().flatten()).sum();
                if let Some(slot) = unknown.first().and_then(|&u| w.get_mut(u)) {
                    *slot = Some((c.w - known).max(200));
                    changed = true;
                }
            }
        }
    }
    let rest: Vec<usize> = w.iter().enumerate().filter(|(_, x)| x.is_none()).map(|(i, _)| i).collect();
    if !rest.is_empty() {
        let known: i64 = w.iter().flatten().sum();
        let each = if table_w != 0 { floor_div(table_w - known, rest.len() as i64).max(2000) } else { 5000 };
        for i in rest {
            if let Some(slot) = w.get_mut(i) {
                *slot = Some(each);
            }
        }
    }
    w.into_iter().map(|x| tw(x.unwrap_or(0)).max(20)).collect()
}

type RowData = (Vec<String>, i64, i64, bool);

/// Trim the first of the tallest rows by what the table is over `limit` (see table()).
fn fit_rows(row_data: &mut [RowData], total: i64, limit: i64) {
    if !(0 < total - limit && (total - limit) as f64 <= total as f64 * 0.05) {
        return;
    }
    // the first of the tallest rows, as Python's max()
    let mut best: Option<usize> = None;
    for (i, rd) in row_data.iter().enumerate() {
        if best.and_then(|b| row_data.get(b)).is_none_or(|b| rd.1 > b.1) {
            best = Some(i);
        }
    }
    if let Some(rd) = best.and_then(|b| row_data.get_mut(b)) {
        rd.1 = (rd.1 - (total - limit)).max(0);
    }
}

/// HWPUNIT thickness of the borders WordCraft adds to a row: top (+ bottom on the last row).
fn cell_rule(c: &CellInfo, last_row: bool) -> i64 {
    let Some(bf) = &c.bf else { return 0 };
    let mut sides = vec![&bf.top];
    if last_row {
        sides.push(&bf.bottom);
    }
    let mm: f64 = sides.iter().filter(|b| border_style(&b.kind).is_some()).map(|b| b.width_mm).sum();
    round_even(mm * 7200.0 / 25.4)
}

/// The guide text of an input field (Direction): a parameter in HWPX, inside the command string
/// ("...Direction:wstring:<n>:<text>...") in HWP.
fn click_here_guide(el: &Element) -> String {
    if let Some(sp) = el.iter("hp:stringParam").find(|sp| sp.get("name") == Some("Direction")) {
        return sp.text.trim().to_string();
    }
    const KEY: &str = "Direction:wstring:";
    for sp in el.iter("hp:stringParam") {
        let s = sp.text.as_str();
        let mut from = 0;
        while let Some(i) = s.get(from..).and_then(|r| r.find(KEY)) {
            let ds = from + i + KEY.len();
            from = from + i + 1;
            let rest = s.get(ds..).unwrap_or("");
            let n = rest.bytes().take_while(u8::is_ascii_digit).count();
            if n > 0 && rest.as_bytes().get(n) == Some(&b':') {
                let count: usize = rest.get(..n).and_then(|d| d.parse().ok()).unwrap_or(0);
                let text = rest.get(n + 1..).unwrap_or("");
                return text.chars().take(count).collect::<String>().trim().to_string();
            }
        }
    }
    String::new()
}

fn hide_runs(runs: &[String]) -> Vec<String> {
    runs.iter()
        .map(|r| {
            if !r.contains("<w:t") {
                r.clone()
            } else if r.contains("<w:rPr>") {
                let at = ["<w:color", "<w:spacing", "<w:w ", "<w:sz"].iter().filter_map(|t| r.find(t)).min();
                match at.filter(|a| *a > 0) {
                    Some(a) => format!("{}<w:vanish/>{}", r.get(..a).unwrap_or(""), r.get(a..).unwrap_or("")),
                    None => r.replacen("<w:rPr>", "<w:rPr><w:vanish/>", 1),
                }
            } else {
                r.replacen("<w:r>", "<w:r><w:rPr><w:vanish/></w:rPr>", 1)
            }
        })
        .collect()
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn safe_url(url: &str) -> bool {
    let l = url.to_ascii_lowercase();
    ["http:", "https:", "mailto:", "ftp:"].iter().any(|p| l.starts_with(p))
}

/// A link to another document (by extension).
fn doc_link(url: &str) -> bool {
    const EXTS: &[&str] = &["hwp", "hwpx", "hwt", "doc", "docx", "odt", "rtf", "pdf", "txt", "xls", "xlsx", "ppt", "pptx", "htm", "html"];
    let l = url.to_ascii_lowercase();
    l.rsplit_once('.').is_some_and(|(_, e)| EXTS.contains(&e))
}

fn hyperlink_url(el: &Element) -> Option<String> {
    for sp in el.iter("hp:stringParam") {
        if !matches!(sp.get("name"), Some("Command" | "Path")) {
            continue;
        }
        // up to the first ';' not escaped with a backslash
        let raw = &sp.text;
        let mut first = String::new();
        let mut prev_backslash = false;
        for ch in raw.chars() {
            if ch == ';' && !prev_backslash {
                break;
            }
            prev_backslash = ch == '\\';
            first.push(ch);
        }
        let url = unescape_backslash(&first).trim().to_string();
        if safe_url(&url) && !url.contains('"') {
            return Some(url);
        }
        if doc_link(&url) && !url.contains('"') {
            // A link to another document (".\other.hwp", "E:\Work\other.hwp").
            let path = url.replace('\\', "/");
            let b = path.as_bytes();
            if b.len() >= 3 && b[0].is_ascii_alphabetic() && &b[1..3] == b":/" {
                return Some(format!("file:///{}", quote(&path, "/:")));
            }
            return Some(quote(path.strip_prefix("./").unwrap_or(&path), "/"));
        }
    }
    None
}

pub(crate) fn ext_of(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() && !e.is_empty() => e.to_string(),
        _ => "png".to_string(),
    }
}

#[cfg(test)]
mod space_tests {
    use super::split_spaces;

    /// The FILETIME a memo keeps (the HWP sample's, 2013-06-26 09:14:43 UTC) reads as ISO 8601.
    #[test]
    fn memo_times_read_as_iso() {
        assert_eq!(super::iso_from_filetime((30_306_893u64 << 32) | 2_552_988_672).as_deref(), Some("2013-06-26T09:14:43Z"));
        assert_eq!(super::iso_from_filetime(0), None);
    }

    /// A level line has no height: its shown width stays (the original size, 100 x 100, is only
    /// the space its end points are given in), as level lines in Hancom's files have it.
    #[test]
    fn level_lines_keep_their_width() {
        let mut line = crate::xml::Element::new("hp:line");
        for (tag, w, h) in [("hp:curSz", "22189", "0"), ("hp:orgSz", "100", "100")] {
            let mut sz = crate::xml::Element::new(tag);
            sz.set("width", w);
            sz.set("height", h);
            line.children.push(sz);
        }
        assert_eq!(crate::drawing::size_of(&line), (22189, 0));
        line.tag = "hp:rect".into();
        assert_eq!(crate::drawing::size_of(&line), (100, 100));
    }

    #[test]
    fn spaces_split_off_their_text() {
        assert_eq!(split_spaces("  가나 다라  끝"), ["  ", "가나", " ", "다라", "  ", "끝"]);
        assert_eq!(split_spaces("가나"), ["가나"]);
        assert!(split_spaces("").is_empty());
    }
}
