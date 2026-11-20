//! A new HWPX package from a WordCraft document (one that was not opened from an HWPX, so there
//! is no original to write edits into; see [`crate::write_back`] for those).
//!
//! Element and attribute names are those of Hancom's `hwpx-owpml-model` (`docs/hwp/owpml-reference.md`);
//! what Hancom's viewer needs and how it reads the values was checked by opening the result in
//! the viewer (black box), noted where it matters.
//!
//! Written: the text with its character formatting (fonts, size, bold, italic, underline,
//! strikeout, colour, highlight, super/subscript, width, spacing), paragraph formatting
//! (alignment, indents, spacing, line spacing, page breaks), each section's page setup, and
//! tables (merged cells, column widths, row heights, borders, fill, cell margins, vertical
//! alignment, position in the line), headers and footers (page numbers as automatic numbers),
//! footnotes and endnotes, pictures (their files in `BinData/`), text boxes and rectangles,
//! ellipses and lines, freeforms (as curves and polygons), groups of drawings, numbered and
//! bulleted lists (as HWP's paragraph headings), paragraph style names and comments (as memos).
//! Other drawn shapes are left out and reported; so are charts and diagrams, which the engine
//! turns into pictures before saving.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::BTreeMap;

use wordcraft_doc::para::{Anchor, FloatAlign, NoteKind, ShapeKind, Wrap};
use wordcraft_doc::props::{Border, BorderStyle, Borders, HeightRule, LineSpacing, Underline, VAlign, VMerge, VertAlign};
use wordcraft_doc::resolve::{ResolvedChar, ResolvedPara};
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::styles::TableStyleProps;
use wordcraft_doc::table::Table;
use wordcraft_doc::{Align, Block, Document, InlineObject, Paragraph, SectionProps, TextColor};

use crate::docx::{attr, esc};
use crate::error::{Error, Result};
use crate::header::{FONT_LANGS, LANGS};
use crate::writer::SaveReport;

const NS: &str = concat!(
    r#"xmlns:ha="http://www.hancom.co.kr/hwpml/2011/app" "#,
    r#"xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph" "#,
    r#"xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section" "#,
    r#"xmlns:hc="http://www.hancom.co.kr/hwpml/2011/core" "#,
    r#"xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head" "#,
    r#"xmlns:hpf="http://www.hancom.co.kr/schema/2011/hpf" "#,
    r#"xmlns:dc="http://purl.org/dc/elements/1.1/" "#,
    r#"xmlns:opf="http://www.idpf.org/2007/opf/""#
);
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>";
/// What Hancom writes for most paragraphs.
const PARAGRAPH_ID: &str = "2147483648";
/// Deepest table nesting followed when flattening cells.
const MAX_DEPTH: usize = 32;
/// Deepest nesting of groups written.
const MAX_GROUP_DEPTH: usize = 8;

/// HWPUNIT -> points.
fn hu_pt(v: i64) -> f32 {
    v as f32 / 100.0
}

/// Points -> HWPUNIT (1/100 pt).
pub(crate) fn hu(pt: f32) -> i64 {
    if pt.is_finite() { (f64::from(pt) * 100.0).round().clamp(-1e9, 1e9) as i64 } else { 0 }
}

fn color(c: Option<wordcraft_doc::Rgb>) -> String {
    c.map(|c| format!("#{}", c.hex())).unwrap_or_else(|| "none".into())
}

/// A character shape as written (the key that shares one hh:charPr).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CharKey {
    height: i64,
    /// Font id per language.
    pub(crate) fonts: [usize; 7],
    text_color: String,
    shade: String,
    bold: bool,
    italic: bool,
    underline: Option<&'static str>,
    underline_color: String,
    strike: Option<&'static str>,
    sup: bool,
    sub: bool,
    outline: bool,
    shadow: bool,
    emboss: bool,
    engrave: bool,
    ratio: i64,
    spacing: i64,
}

/// A paragraph shape as written.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ParaKey {
    pub(crate) align: &'static str,
    pub(crate) left: i64,
    pub(crate) right: i64,
    pub(crate) indent: i64,
    pub(crate) before: i64,
    pub(crate) after: i64,
    pub(crate) ls_type: &'static str,
    pub(crate) ls_value: i64,
    pub(crate) keep_next: bool,
    pub(crate) keep_lines: bool,
    pub(crate) page_break_before: bool,
    pub(crate) widow: bool,
    /// The list the paragraph is in: (`hh:heading` type, numbering or bullet id, level from 0).
    pub(crate) heading: (&'static str, usize, u8),
    /// Korean words may break anywhere (`BREAK_WORD`): a Hangul paragraph that said so. Others
    /// keep them whole, Hangul's default.
    pub(crate) break_words: bool,
}

/// No list.
const NO_HEADING: (&str, usize, u8) = ("NONE", 0, 0);

/// One numbered list as written: its levels' (number format, text with `^n`, start, alignment).
type ListDef = Vec<(&'static str, String, u32, &'static str)>;

pub(crate) fn underline_shape(u: Underline) -> Option<&'static str> {
    Some(match u {
        Underline::None => return None,
        Underline::Double => "DOUBLE_SLIM",
        Underline::Thick => "THICK_SLIM",
        Underline::Dotted => "DOT",
        Underline::Dash => "DASH",
        Underline::DotDash => "DASH_DOT",
        Underline::DotDotDash => "DASH_DOT_DOT",
        Underline::Wave => "WAVE",
        Underline::DoubleWave => "DOUBLEWAVE",
        _ => "SOLID",
    })
}

fn align(a: Align) -> &'static str {
    match a {
        Align::Left => "LEFT",
        Align::Center => "CENTER",
        Align::Right => "RIGHT",
        Align::Justify => "JUSTIFY",
        Align::Distribute => "DISTRIBUTE",
    }
}

/// Word's single line is about 1.2 times the font size; HWP's percentages count from the font
/// size itself (the reader maps them the same way: 160% -> 1.6 x size).
const SINGLE_LINE: f32 = 1.2;

/// One side of a border fill: (type, width, colour).
type Side = (&'static str, &'static str, String);

/// A cell's border fill as written: left, right, top, bottom, and the fill colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FillKey {
    sides: [Side; 4],
    fill: Option<String>,
}

/// HWP's line widths, millimetres (the values `width` takes).
const LINE_WIDTHS: [(f32, &str); 16] = [
    (0.1, "0.1 mm"),
    (0.12, "0.12 mm"),
    (0.15, "0.15 mm"),
    (0.2, "0.2 mm"),
    (0.25, "0.25 mm"),
    (0.3, "0.3 mm"),
    (0.4, "0.4 mm"),
    (0.5, "0.5 mm"),
    (0.6, "0.6 mm"),
    (0.7, "0.7 mm"),
    (1.0, "1.0 mm"),
    (1.5, "1.5 mm"),
    (2.0, "2.0 mm"),
    (3.0, "3.0 mm"),
    (4.0, "4.0 mm"),
    (5.0, "5.0 mm"),
];

impl FillKey {
    /// No lines, no fill.
    pub(crate) fn none() -> FillKey {
        FillKey { sides: std::array::from_fn(|_| side(None)), fill: None }
    }
}

/// One hh:borderFill.
pub(crate) fn fill_xml(id: usize, k: &FillKey) -> String {
    let mut o = format!(
        "<hh:borderFill id=\"{id}\" threeD=\"0\" shadow=\"0\" centerLine=\"NONE\" breakCellSeparateLine=\"0\">\
<hh:slash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/><hh:backSlash type=\"NONE\" Crooked=\"0\" isCounter=\"0\"/>"
    );
    for (tag, (kind, width, col)) in ["leftBorder", "rightBorder", "topBorder", "bottomBorder"].iter().zip(&k.sides) {
        o.push_str(&format!("<hh:{tag} type=\"{kind}\" width=\"{width}\" color=\"{col}\"/>"));
    }
    o.push_str("<hh:diagonal type=\"SOLID\" width=\"0.1 mm\" color=\"#000000\"/>");
    if let Some(f) = &k.fill {
        o.push_str(&format!("<hc:fillBrush><hc:winBrush faceColor=\"{f}\" hatchColor=\"#000000\" alpha=\"0\"/></hc:fillBrush>"));
    }
    o.push_str("</hh:borderFill>");
    o
}

fn side(b: Option<Border>) -> Side {
    let Some(b) = b.filter(Border::is_visible) else { return ("NONE", "0.1 mm", "#000000".into()) };
    let kind = match b.style {
        BorderStyle::None => "NONE",
        BorderStyle::Double => "DOUBLE_SLIM",
        BorderStyle::Dotted => "DOT",
        BorderStyle::Dashed => "DASH",
        BorderStyle::Triple | BorderStyle::ThinThickThinSmallGap | BorderStyle::ThinThickThinMediumGap | BorderStyle::ThinThickThinLargeGap => {
            "SLIM_THICK_SLIM"
        }
        BorderStyle::ThinThickSmallGap | BorderStyle::ThinThickMediumGap | BorderStyle::ThinThickLargeGap => "SLIM_THICK",
        BorderStyle::ThickThinSmallGap | BorderStyle::ThickThinMediumGap | BorderStyle::ThickThinLargeGap => "THICK_SLIM",
        BorderStyle::DotDash => "DASH_DOT",
        BorderStyle::Wave => "WAVE",
        BorderStyle::Single | BorderStyle::Thick => "SOLID",
    };
    let mm = if b.width.is_finite() { b.width * 25.4 / 72.0 } else { 0.1 };
    let width = LINE_WIDTHS.iter().min_by(|a, c| (a.0 - mm).abs().total_cmp(&(c.0 - mm).abs())).map(|w| w.1).unwrap_or("0.1 mm");
    (kind, width, color(b.color).replace("none", "#000000"))
}

#[derive(Default)]
pub(crate) struct Tables {
    /// Border fills after the first ("nothing", id 1): id = index + 2.
    pub(crate) fills: Vec<FillKey>,
    /// Font names per language, in order of first use.
    pub(crate) fonts: [Vec<String>; 7],
    chars: BTreeMap<CharKey, usize>,
    pub(crate) char_order: Vec<CharKey>,
    paras: BTreeMap<ParaKey, usize>,
    pub(crate) para_order: Vec<ParaKey>,
    /// Numbered lists (hh:numbering id = index + 1), with the WordCraft list each came from.
    lists: Vec<(u32, ListDef)>,
    /// Bullet characters (hh:bullet id = index + 1).
    bullets: Vec<char>,
    /// Paragraph styles after the Normal style, 바탕글 (hh:style id = index + 1): WordCraft id, name, paraPr, charPr.
    styles: Vec<(String, String, usize, usize)>,
    /// Installed stand-ins the reader put for missing fonts, and the names they replaced
    /// (stand-in -> original when only one font took it; original -> stand-in).
    font_back: BTreeMap<String, String>,
    font_swaps: BTreeMap<String, String>,
    /// [`crate::fonts::hangul_default`] by the run's font, asked once each.
    hangul_defaults: BTreeMap<String, Option<String>>,
    /// Memos written (each one's number and field id; the header then has a memo shape).
    memos: usize,
}

impl Tables {
    fn font(&mut self, lang: usize, name: &str) -> usize {
        let Some(list) = self.fonts.get_mut(lang) else { return 0 };
        match list.iter().position(|f| f == name) {
            Some(i) => i,
            None => {
                list.push(name.to_string());
                list.len() - 1
            }
        }
    }

    fn fill_id(&mut self, key: FillKey) -> usize {
        match self.fills.iter().position(|k| *k == key) {
            Some(i) => i + 2,
            None => {
                self.fills.push(key);
                self.fills.len() + 1
            }
        }
    }

    fn char_id(&mut self, rc: &ResolvedChar) -> usize {
        let mut latin = rc.font.clone();
        // no East Asian font named: the system's Hangul font, unless the run's own font has Hangul
        let mut east = rc
            .font_ea
            .clone()
            .or_else(|| self.hangul_defaults.entry(latin.clone()).or_insert_with(|| crate::fonts::hangul_default(&latin)).clone())
            .unwrap_or_else(|| latin.clone());
        // A run the reader gave stand-ins carries a name of its own in the complex-script slot
        // (unused by Korean and Latin text); only such a run takes original names back. A run
        // that names the stand-in itself (NanumGothic chosen in the document) keeps it.
        let swapped = rc.font_cs != latin && rc.font_cs != east;
        if swapped {
            // the other slot, when both were swapped: by the stand-in's unique original
            if let Some(o) = self.font_back.get(&east) {
                east = o.clone();
            }
            if let Some(o) = self.font_back.get(&latin) {
                latin = o.clone();
            }
        }
        if let Some(sub) = self.font_swaps.get(&rc.font_cs) {
            if *sub == east {
                east = rc.font_cs.clone();
            }
            if *sub == latin {
                latin = rc.font_cs.clone();
            }
        }
        let mut fonts = [0usize; 7];
        for (k, f) in fonts.iter_mut().enumerate() {
            // Hangul, hanja and Japanese text use the East Asian font; the rest the run's.
            let name = if matches!(k, 0 | 2 | 3) { &east } else { &latin };
            *f = self.font(k, name);
        }
        let size = rc.size.max(1.0);
        let key = CharKey {
            height: hu(size),
            fonts,
            text_color: match rc.color {
                TextColor::Rgb(c) => format!("#{}", c.hex()),
                TextColor::Auto => "#000000".into(),
            },
            shade: color(rc.highlight.or(rc.shading)),
            bold: rc.bold,
            italic: rc.italic,
            underline: underline_shape(rc.underline),
            underline_color: rc.underline_color.map(|c| format!("#{}", c.hex())).unwrap_or_else(|| "#000000".into()),
            strike: if rc.double_strike {
                Some("DOUBLE_SLIM")
            } else if rc.strike {
                Some("SOLID")
            } else {
                None
            },
            sup: rc.vert_align == VertAlign::Superscript,
            sub: rc.vert_align == VertAlign::Subscript,
            outline: rc.outline,
            shadow: rc.shadow,
            emboss: rc.emboss,
            engrave: rc.engrave,
            ratio: (rc.scale.round() as i64).clamp(50, 200),
            // character spacing in percent of the font size
            spacing: ((rc.spacing / size * 100.0).round() as i64).clamp(-50, 50),
        };
        self.intern_char(key)
    }

    fn intern_char(&mut self, key: CharKey) -> usize {
        if let Some(&id) = self.chars.get(&key) {
            return id;
        }
        let id = self.char_order.len();
        self.chars.insert(key.clone(), id);
        self.char_order.push(key);
        id
    }

    fn para_id(&mut self, rp: &ResolvedPara, font_size: f32) -> usize {
        self.para_id_in(rp, font_size, NO_HEADING)
    }

    fn para_id_in(&mut self, rp: &ResolvedPara, font_size: f32, heading: (&'static str, usize, u8)) -> usize {
        let key = ParaKey { heading, ..para_key(rp, font_size) };
        if let Some(&id) = self.paras.get(&key) {
            return id;
        }
        let id = self.para_order.len();
        self.paras.insert(key.clone(), id);
        self.para_order.push(key);
        id
    }
}

/// A paragraph's shape as HWP writes it.
pub(crate) fn para_key(rp: &ResolvedPara, font_size: f32) -> ParaKey {
    let first = rp.indent_first;
    // HWP's left margin is where the first line starts; a negative indent (a hanging indent) moves
    // the other lines right of it, as DOCX's hanging indent moves the first line left.
    let left = if first < 0.0 { rp.indent_left + first } else { rp.indent_left };
    let (ls_type, ls_value) = match rp.line_spacing {
        LineSpacing::Multiple(m) => ("PERCENT", ((m * SINGLE_LINE * 100.0).round() as i64).clamp(50, 500)),
        // An exact line height as a percentage of the font size: HWP's own form (the reader
        // turns those into exact heights), which Hancom lays out like WordCraft; fixed heights
        // ran lines together in table cells (viewer).
        LineSpacing::Exactly(pt) if font_size > 0.0 && pt.is_finite() => ("PERCENT", ((pt / font_size * 100.0).round() as i64).clamp(50, 500)),
        LineSpacing::Exactly(pt) => ("FIXED", hu(pt).max(100)),
        LineSpacing::AtLeast(pt) => ("AT_LEAST", hu(pt.max(font_size)).max(100)),
    };
    let before = if rp.before_auto { wordcraft_doc::resolve::AUTO_SPACING } else { rp.space_before };
    let after = if rp.after_auto { wordcraft_doc::resolve::AUTO_SPACING } else { rp.space_after };
    ParaKey {
        align: align(rp.align),
        left: hu(left),
        right: hu(rp.indent_right),
        indent: hu(first),
        before: hu(before).max(0),
        after: hu(after).max(0),
        ls_type,
        ls_value,
        keep_next: rp.keep_next,
        keep_lines: rp.keep_lines,
        page_break_before: rp.page_break_before,
        widow: rp.widow_control,
        heading: NO_HEADING,
        break_words: rp.keep_hangul_words == Some(false),
    }
}

/// HWP's number format for a list level's.
fn num_format(f: NumFormat) -> &'static str {
    match f {
        NumFormat::UpperRoman => "ROMAN_CAPITAL",
        NumFormat::LowerRoman => "ROMAN_SMALL",
        NumFormat::UpperLetter => "LATIN_CAPITAL",
        NumFormat::LowerLetter => "LATIN_SMALL",
        NumFormat::Ganada => "HANGUL_SYLLABLE",
        NumFormat::Chosung => "HANGUL_JAMO",
        NumFormat::DecimalEnclosedCircle => "CIRCLED_DIGIT",
        NumFormat::KoreanDigital | NumFormat::KoreanCounting => "HANGUL_PHONETIC",
        NumFormat::IdeographTraditional => "IDEOGRAPH",
        _ => "DIGIT",
    }
}

/// A list level's label text in HWP's form: `%1.%2)` -> `^1.^2)`.
fn level_text(text: &str) -> String {
    let mut o = String::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c == '%' && it.peek().is_some_and(char::is_ascii_digit) {
            o.push('^');
        } else {
            o.push(c);
        }
    }
    o
}

fn b(v: bool) -> u8 {
    u8::from(v)
}

pub(crate) fn char_pr_xml(id: usize, k: &CharKey) -> String {
    let langs = |vals: &dyn Fn(usize) -> String| -> String { LANGS.iter().enumerate().map(|(i, a)| format!(" {a}=\"{}\"", vals(i))).collect() };
    let mut o = format!(
        "<hh:charPr id=\"{id}\" height=\"{}\" textColor=\"{}\" shadeColor=\"{}\" useFontSpace=\"0\" useKerning=\"0\" symMark=\"NONE\" borderFillIDRef=\"1\">",
        k.height, k.text_color, k.shade
    );
    o.push_str(&format!("<hh:fontRef{}/>", langs(&|i| k.fonts.get(i).copied().unwrap_or(0).to_string())));
    o.push_str(&format!("<hh:ratio{}/>", langs(&|_| k.ratio.to_string())));
    o.push_str(&format!("<hh:spacing{}/>", langs(&|_| k.spacing.to_string())));
    o.push_str(&format!("<hh:relSz{}/>", langs(&|_| "100".into())));
    o.push_str(&format!("<hh:offset{}/>", langs(&|_| "0".into())));
    if k.italic {
        o.push_str("<hh:italic/>");
    }
    if k.bold {
        o.push_str("<hh:bold/>");
    }
    match k.underline {
        Some(shape) => o.push_str(&format!("<hh:underline type=\"BOTTOM\" shape=\"{shape}\" color=\"{}\"/>", k.underline_color)),
        None => o.push_str("<hh:underline type=\"NONE\" shape=\"SOLID\" color=\"#000000\"/>"),
    }
    o.push_str(&format!("<hh:strikeout shape=\"{}\" color=\"#000000\"/>", k.strike.unwrap_or("NONE")));
    o.push_str(&format!("<hh:outline type=\"{}\"/>", if k.outline { "SOLID" } else { "NONE" }));
    o.push_str(&format!("<hh:shadow type=\"{}\" color=\"#B2B2B2\" offsetX=\"10\" offsetY=\"10\"/>", if k.shadow { "DROP" } else { "NONE" }));
    if k.emboss {
        o.push_str("<hh:emboss/>");
    }
    if k.engrave {
        o.push_str("<hh:engrave/>");
    }
    if k.sup {
        o.push_str("<hh:supscript/>");
    }
    if k.sub {
        o.push_str("<hh:subscript/>");
    }
    o.push_str("</hh:charPr>");
    o
}

pub(crate) fn para_pr_xml(id: usize, k: &ParaKey) -> String {
    let value = |tag: &str, v: i64| format!("<hc:{tag} value=\"{v}\" unit=\"HWPUNIT\"/>");
    format!(
        "<hh:paraPr id=\"{id}\" tabPrIDRef=\"0\" condense=\"0\" fontLineHeight=\"0\" snapToGrid=\"1\" suppressLineNumbers=\"0\" checked=\"0\">\
<hh:align horizontal=\"{}\" vertical=\"BASELINE\"/>\
<hh:heading type=\"{}\" idRef=\"{}\" level=\"{}\"/>\
<hh:breakSetting breakLatinWord=\"KEEP_WORD\" breakNonLatinWord=\"{}\" widowOrphan=\"{}\" keepWithNext=\"{}\" keepLines=\"{}\" pageBreakBefore=\"{}\" lineWrap=\"BREAK\"/>\
<hh:margin>{}{}{}{}{}</hh:margin>\
<hh:lineSpacing type=\"{}\" value=\"{}\" unit=\"HWPUNIT\"/>\
<hh:border borderFillIDRef=\"1\" offsetLeft=\"0\" offsetRight=\"0\" offsetTop=\"0\" offsetBottom=\"0\" connect=\"0\" ignoreMargin=\"0\"/>\
</hh:paraPr>",
        k.align,
        k.heading.0,
        k.heading.1,
        k.heading.2,
        if k.break_words { "BREAK_WORD" } else { "KEEP_WORD" },
        b(k.widow),
        b(k.keep_next),
        b(k.keep_lines),
        b(k.page_break_before),
        value("intent", k.indent),
        value("left", k.left),
        value("right", k.right),
        value("prev", k.before),
        value("next", k.after),
        k.ls_type,
        k.ls_value
    )
}

/// What an inline object becomes.
enum Piece {
    Text(String),
    /// A run's inner XML (a control or an object) and the height it takes on its line, points.
    Run(String, f32),
}

/// One HWPX paragraph being built.
struct OutPara {
    para_pr: usize,
    /// hh:style id.
    style: usize,
    page_break: bool,
    column_break: bool,
    /// (charPr id, the run's inner XML: an hp:t, a control or an object)
    runs: Vec<(usize, String)>,
    /// charPr of the paragraph mark (for an empty paragraph).
    mark: usize,
    /// A table this paragraph holds (HWP keeps a table in a paragraph's run), as hp:tbl XML.
    table: Option<String>,
    /// Estimated height, points (for table rows: Hancom wants each cell's height).
    height: f32,
}

impl OutPara {
    fn empty(para_pr: usize, mark: usize) -> OutPara {
        OutPara { para_pr, style: 0, page_break: false, column_break: false, runs: Vec::new(), mark, table: None, height: 0.0 }
    }
}

/// Table nesting being written (to stop hostile depths).
struct Builder<'d> {
    doc: &'d Document,
    t: Tables,
    report: SaveReport,
    /// Pictures written: (media key, item id, file name in BinData/, media type).
    media: Vec<(String, String, String, &'static str)>,
    /// Footnotes and endnotes so far.
    notes: [usize; 2],
    /// Drawing objects so far (their z-order and instance ids).
    objects: usize,
    /// Inside a note's own text (its reference mark there is the note's number).
    in_note: bool,
    /// Writing a header or footer: where its area starts (see [`Area`]).
    area: Option<Area>,
    /// What could not be written, by kind.
    left_out: BTreeMap<&'static str, usize>,
    /// Writing into an existing package, whose header has no lists or styles of ours.
    standalone: bool,
    /// Writing a member of a group: where it sits in the group's coordinates (HWPUNIT), the
    /// group's scale onto its frame, and how deep the group is.
    member: Option<Member>,
    memos: Memos,
    /// How deep the blocks being written are, counting tables and the stories (notes, text
    /// boxes, comments) inside them, so a story nested in itself stops at [`MAX_DEPTH`].
    depth_now: usize,
    /// The stories being written, outermost first: a story that holds itself is left out.
    open_stories: Vec<u32>,
    /// Stories (text boxes, notes, headers, footers, comments) written so far: a document whose
    /// text boxes keep naming the same stories would otherwise write them over and over, about
    /// 2^depth times, within the depth limit.
    stories_made: usize,
}

/// Comments being written as Hangul memos: a `MEMO` field around the commented text, holding
/// the memo's paragraphs (viewer-checked, see `docs/hwp/cleanroom.md`).
#[derive(Default)]
struct Memos {
    /// Write comments as memos (a new HWPX); otherwise they are left out.
    write: bool,
    /// Memo fields open in the paragraph being written: (comment id, field id).
    open: Vec<(u32, usize)>,
    /// Writing a memo's own text.
    inside: bool,
}

/// The memo shape every memo uses (hh:memoPr id 1): a light blue box with a grey line.
const MEMO_PROPERTIES: &str = "<hh:memoProperties itemCnt=\"1\"><hh:memoPr id=\"1\" width=\"15591\" lineWidth=\"0.6 mm\" lineType=\"SOLID\" \
lineColor=\"#A9A9A9\" fillColor=\"#E6E6F9\" activeColor=\"#DCEEB2\" memoType=\"NOMAL\"/></hh:memoProperties>";

/// The most stories one file writes (see `Builder::stories_made`): far above a real document's
/// text boxes and notes.
pub(crate) const MAX_STORIES: usize = 10_000;

/// What [`Builder::list_heading`] reports for numbers it can't write in a piece written alone.
/// What a hyperlink loses in the HWPX writer: its destination (the linked text is kept).
pub(crate) const LINKS_LEFT_OUT: &str = "hyperlink destinations (the linked text is kept)";
/// On a cell of a table written alone ([`table_alone`]) whose numbers or bullets were left out:
/// the .hwp writer, which keeps some cells' original records (numbering included), reports the
/// others, and the .hwpx write-back takes the attribute off.
pub(crate) const NUMBERED_CELL: &str = "wcNumbered";
pub(crate) const NUMBERS_LEFT_OUT: &str = "numbered or bulleted paragraphs' numbers and bullets";

/// Width of a memo's text, points (the memo shape's 15591 HWPUNIT less its margins).
const MEMO_WIDTH: f32 = 150.0;

fn field_end(id: usize) -> String {
    format!("<hp:ctrl><hp:fieldEnd beginIDRef=\"{id}\" fieldid=\"{id}\"/></hp:ctrl>")
}

/// An ISO 8601 time (`2025-01-02T03:04:05Z`; no zone is taken as UTC) as a Windows FILETIME:
/// 100 ns ticks since 1601-01-01 UTC, what a memo's field keeps (viewer-checked).
fn filetime(iso: &str) -> Option<u64> {
    let num = |a: usize, b: usize| iso.get(a..b).and_then(|t| t.parse::<i64>().ok());
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi) = (num(11, 13).unwrap_or(0), num(14, 16).unwrap_or(0));
    let sec = num(17, 19).unwrap_or(0);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // days from 1970-01-01 (proleptic Gregorian)
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    // 1601-01-01 is 134774 days before 1970-01-01
    let secs = (days + 134_774) * 86_400 + h * 3600 + mi * 60 + sec;
    u64::try_from(secs).ok()?.checked_mul(10_000_000)
}

/// `s` for a slash-separated field command: `\`, `/` and `;` behind a backslash.
fn field_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars().filter(|c| !c.is_control()) {
        if matches!(c, '\\' | '/' | ';') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// A drawing inside a group (see [`Builder::member`]). Groups are written unscaled (their own
/// size is their shown size): a member's place and size are in shown HWPUNIT, its own sizes
/// (points, in the group's coordinates) multiplied by `sx` x `sy`. Hancom scales a member's size
/// by the group's scale but not its offset (viewer, measured), so a scaled group would need
/// offsets in shown units while DrawingML's are in the group's own; unscaled, both agree.
#[derive(Clone, Copy, Debug)]
struct Member {
    x: i64,
    y: i64,
    sx: f64,
    sy: f64,
    level: usize,
}

/// The image format of picture bytes: (extension, media type).
pub(crate) fn image_kind(data: &[u8]) -> (&'static str, &'static str) {
    if data.starts_with(b"\x89PNG") {
        ("png", "image/png")
    } else if data.starts_with(&[0xFF, 0xD8]) {
        ("jpg", "image/jpeg")
    } else if data.starts_with(b"GIF8") {
        ("gif", "image/gif")
    } else if data.starts_with(b"BM") {
        ("bmp", "image/bmp")
    } else if data.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || data.get(40..44) == Some(b" EMF") {
        ("emf", "image/x-emf")
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        ("webp", "image/webp")
    } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        ("tif", "image/tiff")
    } else {
        ("png", "image/png")
    }
}

/// A header's or footer's area on the page, points: Hancom places the objects in a header or
/// footer from the area's top-left corner whatever they are anchored to (viewer, measured).
#[derive(Clone, Copy, Debug)]
struct Area {
    /// The left margin (where the area starts).
    left: f32,
    /// The area's top, from the paper's top.
    top: f32,
    /// The body's top, from the paper's top.
    body_top: f32,
}

/// Where an object sits: (textWrap, hp:pos, hp:outMargin).
fn placement_xml(f: &wordcraft_doc::para::Float, area: Option<Area>) -> (&'static str, String, String) {
    let inline = f.wrap == Wrap::Inline;
    let wrap = match f.wrap {
        Wrap::Square | Wrap::Tight | Wrap::Through => "SQUARE",
        Wrap::BehindText => "BEHIND_TEXT",
        Wrap::InFrontOfText => "IN_FRONT_OF_TEXT",
        Wrap::Inline | Wrap::TopAndBottom => "TOP_AND_BOTTOM",
    };
    let h_rel = match f.h_rel {
        Anchor::Page => "PAPER",
        Anchor::Margin | Anchor::LeftMargin | Anchor::RightMargin => "PAGE",
        Anchor::Column => "COLUMN",
        _ => "PARA",
    };
    let v_rel = match f.v_rel {
        Anchor::Page => "PAPER",
        Anchor::Margin | Anchor::TopMargin => "PAGE",
        _ => "PARA",
    };
    let align = |a: Option<FloatAlign>, start: &'static str, end: &'static str| match a {
        Some(FloatAlign::Center) => "CENTER",
        Some(FloatAlign::End) => end,
        Some(FloatAlign::Inside) => "INSIDE",
        Some(FloatAlign::Outside) => "OUTSIDE",
        _ => start,
    };
    let (mut x, mut y) = (f.x, f.y);
    if let Some(a) = area {
        if f.h_rel == Anchor::Page {
            x -= a.left;
        }
        y = match f.v_rel {
            Anchor::Page => y - a.top,
            Anchor::Margin | Anchor::TopMargin => y + a.body_top - a.top,
            _ => y,
        };
    }
    // Hancom's offset places the object's outer margin, where DOCX's places the object.
    x -= f.dist.max(0.0);
    y -= f.dist_top.max(0.0);
    let pos = format!(
        "<hp:pos treatAsChar=\"{}\" affectLSpacing=\"0\" flowWithText=\"{}\" allowOverlap=\"{}\" holdAnchorAndSO=\"0\" vertRelTo=\"{v_rel}\" horzRelTo=\"{h_rel}\" vertAlign=\"{}\" horzAlign=\"{}\" vertOffset=\"{}\" horzOffset=\"{}\"/>",
        b(inline),
        // an object placed on the page keeps its place (flowing with the text would hold it
        // inside the body area)
        b(inline || v_rel == "PARA"),
        b(!inline),
        align(f.v_align, "TOP", "BOTTOM"),
        align(f.h_align, "LEFT", "RIGHT"),
        if inline || f.v_align.is_some() { 0 } else { hu(y) },
        if inline || f.h_align.is_some() { 0 } else { hu(x) }
    );
    let margin = format!(
        "<hp:outMargin left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\"/>",
        hu(f.dist).max(0),
        hu(f.dist).max(0),
        hu(f.dist_top).max(0),
        hu(f.dist_bottom).max(0)
    );
    (wrap, pos, margin)
}

/// The corners of a preset shape drawn as a polygon in its `w` x `h` frame (HWPUNIT): triangles,
/// the diamond, block arrows (shaft half the height, head as long as half the height), the
/// five-pointed star and the heart (its outline sampled). `None` for other kinds.
fn polygon_points(kind: ShapeKind, w: i64, h: i64) -> Option<Vec<(i64, i64)>> {
    let (wf, hf) = (w as f64, h as f64);
    let scale = |pts: &[(f64, f64)]| pts.iter().map(|&(x, y)| ((x * wf).round() as i64, (y * hf).round() as i64)).collect();
    // a right-pointing arrow in unit coordinates of a frame `len` long and `thick` high
    let arrow = |len: f64, thick: f64| {
        let head = (thick / 2.0 / len).clamp(0.0, 1.0);
        [(0.0, 0.25), (1.0 - head, 0.25), (1.0 - head, 0.0), (1.0, 0.5), (1.0 - head, 1.0), (1.0 - head, 0.75), (0.0, 0.75)]
    };
    Some(match kind {
        ShapeKind::Triangle => scale(&[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        ShapeKind::RightTriangle => scale(&[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        ShapeKind::Diamond => scale(&[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]),
        ShapeKind::Arrow => scale(&arrow(wf, hf)),
        ShapeKind::ArrowLeft => scale(&arrow(wf, hf).map(|(x, y)| (1.0 - x, y))),
        ShapeKind::ArrowUp => scale(&arrow(hf, wf).map(|(x, y)| (y, 1.0 - x))),
        ShapeKind::ArrowDown => scale(&arrow(hf, wf).map(|(x, y)| (y, x))),
        ShapeKind::Star => {
            let pts: Vec<(f64, f64)> = (0..10)
                .map(|k| {
                    let r = if k % 2 == 0 { 0.5 } else { 0.5 * 0.382 };
                    let a = std::f64::consts::PI * (k as f64) / 5.0 - std::f64::consts::FRAC_PI_2;
                    (0.5 + r * a.cos(), 0.5 + r * a.sin() + 0.05)
                })
                .collect();
            scale(&pts)
        }
        ShapeKind::Heart => {
            let pts: Vec<(f64, f64)> = (0..32)
                .map(|k| {
                    let t = std::f64::consts::TAU * (k as f64) / 32.0;
                    let x = 16.0 * t.sin().powi(3);
                    let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                    (0.5 + x / 34.0, 0.5 - (y + 2.5) / 31.0)
                })
                .collect();
            scale(&pts)
        }
        _ => return None,
    })
}

/// The size and turn of a drawing object (offset, orgSz, curSz, flip, rotationInfo,
/// renderingInfo): `w` x `h` in its own coordinates shown at `cur`, inside a
/// group when `member` is given: its offset in the group goes into the translation (as Hancom
/// writes members).
fn frame_xml(w: i64, h: i64, cur: (i64, i64), f: &wordcraft_doc::para::Float, member: Option<Member>) -> String {
    let id = "e1=\"1\" e2=\"0\" e3=\"0\" e4=\"0\" e5=\"1\" e6=\"0\"";
    let angle = if f.rot.is_finite() { (f.rot.round() as i64).rem_euclid(360) } else { 0 };
    let (x, y) = member.map(|m| (m.x, m.y)).unwrap_or((0, 0));
    let (sx, sy) = (cur.0 as f64 / w.max(1) as f64, cur.1 as f64 / h.max(1) as f64);
    format!(
        "<hp:offset x=\"{x}\" y=\"{y}\"/><hp:orgSz width=\"{w}\" height=\"{h}\"/><hp:curSz width=\"{}\" height=\"{}\"/>\
<hp:flip horizontal=\"{}\" vertical=\"{}\"/><hp:rotationInfo angle=\"{angle}\" centerX=\"{}\" centerY=\"{}\" rotateimage=\"1\"/>\
<hp:renderingInfo><hc:transMatrix e1=\"1\" e2=\"0\" e3=\"{x}\" e4=\"0\" e5=\"1\" e6=\"{y}\"/><hc:scaMatrix e1=\"{}\" e2=\"0\" e3=\"0\" e4=\"0\" e5=\"{}\" e6=\"0\"/><hc:rotMatrix {id}/></hp:renderingInfo>",
        cur.0,
        cur.1,
        b(f.flip_h),
        b(f.flip_v),
        w / 2,
        h / 2,
        scale(sx),
        scale(sy)
    )
}

/// A scale factor as Hancom writes it (up to six decimals, "1" for one).
fn scale(v: f64) -> String {
    let v = if v.is_finite() && v > 0.0 { v } else { 1.0 };
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Points of a freeform's paths in a `w` x `h` frame (HWPUNIT), each closed or not.
fn freeform_paths(f: &wordcraft_doc::freeform::Freeform, w: i64, h: i64) -> Vec<(Vec<(i64, i64)>, bool)> {
    let sx = if f.w > 0.0 { w as f64 / f64::from(f.w) } else { 100.0 };
    let sy = if f.h > 0.0 { h as f64 / f64::from(f.h) } else { 100.0 };
    f.paths
        .iter()
        .take(256)
        .filter(|p| p.pts.len() >= 2)
        .map(|p| {
            (p.pts.iter().take(10_000).map(|[x, y]| ((f64::from(*x) * sx).round() as i64, (f64::from(*y) * sy).round() as i64)).collect(), p.closed)
        })
        .collect()
}

fn sz_xml(w: i64, h: i64) -> String {
    format!("<hp:sz width=\"{w}\" widthRelTo=\"ABSOLUTE\" height=\"{h}\" heightRelTo=\"ABSOLUTE\" protect=\"0\"/>")
}

fn sub_list(paras: &[OutPara], valign: &str, w: i64, h: i64) -> String {
    let body: String = paras.iter().map(|p| para_xml(p, None)).collect();
    format!(
        "<hp:subList id=\"\" textDirection=\"HORIZONTAL\" lineWrap=\"BREAK\" vertAlign=\"{valign}\" linkListIDRef=\"0\" linkListNextIDRef=\"0\" textWidth=\"{w}\" textHeight=\"{h}\" hasTextRef=\"0\" hasNumRef=\"0\">{body}</hp:subList>"
    )
}

/// An automatic number (page, total pages, note number) as a control.
fn auto_num(kind: &str, num: usize, suffix: &str) -> String {
    format!(
        "<hp:ctrl><hp:autoNum num=\"{num}\" numType=\"{kind}\"><hp:autoNumFormat type=\"DIGIT\" userChar=\"\" prefixChar=\"\" suffixChar=\"{suffix}\" supscript=\"0\"/></hp:autoNum></hp:ctrl>"
    )
}

/// Word's default cell margins (top, left, bottom, right), points.
const DEFAULT_CELL_MARGINS: [f32; 4] = [0.0, 5.4, 0.0, 5.4];

fn valign(v: Option<VAlign>) -> &'static str {
    match v.unwrap_or_default() {
        VAlign::Top => "TOP",
        VAlign::Center => "CENTER",
        VAlign::Bottom => "BOTTOM",
    }
}

/// Height of a paragraph's text set `width` points wide, roughly (full-width characters one em,
/// others 0.55 em): Hancom wants each cell's height; it grows rows to fit their text.
fn text_height(text: &str, size: f32, line: f32, width: f32) -> f32 {
    let em: f32 = text.chars().map(|c| if c as u32 >= 0x1100 { size } else { size * 0.55 }).sum();
    let lines = (em / width.max(size)).ceil().max(1.0) + text.matches(wordcraft_doc::para::LINE_BREAK).count() as f32;
    lines * line
}

impl Builder<'_> {
    /// The text of one WordCraft paragraph as HWPX paragraphs (a page or column break starts a
    /// new one, as HWP marks breaks on the paragraph that follows them).
    fn paragraph(&mut self, p: &Paragraph, out: &mut Vec<OutPara>, ts: Option<&TableStyleProps>, width: f32) {
        let style = p.props.style.as_deref();
        let mut rp = self.doc.styles.resolve_para_in(&p.props, ts.map(|s| &s.para));
        let mark_rc = self.doc.styles.resolve_char_in(style, ts.map(|s| &s.chr), &p.mark);
        let heading = self.list_heading(p, &mut rp);
        let para_pr = self.t.para_id_in(&rp, mark_rc.size, heading);
        let mark = self.t.char_id(&mark_rc);
        let style_id = self.style_id(style);
        let size = p.runs.first().map(|r| self.doc.styles.resolve_char_in(style, ts.map(|s| &s.chr), &r.props).size).unwrap_or(mark_rc.size);
        let line = match rp.line_spacing {
            LineSpacing::Multiple(m) => size * SINGLE_LINE * m,
            LineSpacing::Exactly(pt) => pt,
            LineSpacing::AtLeast(pt) => pt.max(size * SINGLE_LINE),
        };
        let indents = rp.indent_left.max(0.0) + rp.indent_right.max(0.0);
        let mut cur = OutPara { style: style_id, ..OutPara::empty(para_pr, mark) };
        cur.height = text_height(&p.text, size, line, width - indents) + rp.space_before.max(0.0) + rp.space_after.max(0.0);
        let mut objects = p.objects.iter();
        let mut pos = 0usize;
        let mut link: Option<&String> = None; // the hyperlink of the run before
        for run in &p.runs {
            let end = pos.saturating_add(run.len).min(p.text.len());
            let text = p.text.get(pos..end).unwrap_or("");
            pos = end;
            // a hyperlink's destination isn't written yet (its text is): each link is reported
            if run.props.link.is_some() && run.props.link.as_ref() != link {
                self.missing(LINKS_LEFT_OUT);
            }
            link = run.props.link.as_ref();
            if run.props.del.is_some() {
                // a tracked deletion: the text as it will read; its objects are passed over too,
                // or the next object would take the deleted one's place
                for _ in text.chars().filter(|&c| c == wordcraft_doc::para::OBJ) {
                    objects.next();
                }
                continue;
            }
            let id = self.t.char_id(&self.doc.styles.resolve_char_in(style, ts.map(|s| &s.chr), &run.props));
            let mut inner = String::new();
            for ch in text.chars() {
                match ch {
                    '\t' => inner.push_str("<hp:tab/>"),
                    wordcraft_doc::para::LINE_BREAK => inner.push_str("<hp:lineBreak/>"),
                    wordcraft_doc::para::NBSP => inner.push_str("<hp:nbSpace/>"),
                    wordcraft_doc::para::NB_HYPHEN => inner.push('-'),
                    wordcraft_doc::para::SOFT_HYPHEN => {}
                    wordcraft_doc::para::OBJ => {
                        let Some(o) = objects.next() else { continue };
                        match self.object(o, width) {
                            Some(Piece::Text(t)) => inner.push_str(&esc(&t)),
                            Some(Piece::Run(xml, height)) => {
                                if !inner.is_empty() {
                                    cur.runs.push((id, format!("<hp:t>{}</hp:t>", std::mem::take(&mut inner))));
                                }
                                cur.runs.push((id, xml));
                                cur.height = cur.height.max(height);
                            }
                            None => {}
                        }
                    }
                    wordcraft_doc::para::PAGE_BREAK | wordcraft_doc::para::COLUMN_BREAK => {
                        if !inner.is_empty() {
                            cur.runs.push((id, format!("<hp:t>{}</hp:t>", std::mem::take(&mut inner))));
                        }
                        // the rest of the paragraph: same style, no second list number
                        let rest = if heading == NO_HEADING { para_pr } else { self.t.para_id(&rp, mark_rc.size) };
                        let mut next = OutPara { style: style_id, ..OutPara::empty(rest, mark) };
                        next.page_break = ch == wordcraft_doc::para::PAGE_BREAK;
                        next.column_break = ch == wordcraft_doc::para::COLUMN_BREAK;
                        out.push(std::mem::replace(&mut cur, next));
                    }
                    c if (c as u32) < 0x20 => {}
                    c => inner.push_str(&esc(&c.to_string())),
                }
            }
            if !inner.is_empty() {
                cur.runs.push((id, format!("<hp:t>{inner}</hp:t>")));
            }
        }
        out.push(cur);
    }

    /// The list `p` is in, as HWP's paragraph heading, with the list level's indents put into
    /// `rp` where WordCraft's layout puts them (unless the paragraph or its style sets its own).
    /// Hancom draws the number itself, at the start of the first line.
    fn list_heading(&mut self, p: &Paragraph, rp: &mut ResolvedPara) -> (&'static str, usize, u8) {
        let numbering = &self.doc.numbering;
        let Some(n) = p.props.numbering.or(rp.numbering).filter(|n| n.num != 0 && numbering.shows_label(n.num, n.level)) else {
            return NO_HEADING;
        };
        if self.standalone {
            // a piece written alone (a table written again) can't add numbering definitions to
            // the file's header yet: its numbers are left out, and said so
            self.missing(NUMBERS_LEFT_OUT);
            return NO_HEADING;
        }
        let level = n.level.min(8);
        let Some(lvl) = numbering.level(n.num, level) else { return NO_HEADING };
        let (style_left, style_first) = if p.props.numbering.is_some() {
            (false, false)
        } else {
            self.doc.styles.indents_over_list(p.props.style.as_deref().unwrap_or("Normal"))
        };
        if !(p.props.indent_left.is_some() || style_left) {
            rp.indent_left = lvl.indent;
        }
        if !(p.props.indent_first.is_some() || style_first) {
            rp.indent_first = -lvl.hanging;
        }
        if lvl.format == NumFormat::Bullet {
            let ch = lvl.text.chars().next().filter(|c| !(0xE000..=0xF8FF).contains(&(*c as u32))).unwrap_or('•');
            let id = match self.t.bullets.iter().position(|c| *c == ch) {
                Some(i) => i + 1,
                None => {
                    self.t.bullets.push(ch);
                    self.t.bullets.len()
                }
            };
            return ("BULLET", id, level);
        }
        let id = match self.t.lists.iter().position(|(num, _)| *num == n.num) {
            Some(i) => i + 1,
            None => {
                let starts = numbering.num(n.num).map(|x| x.start_overrides.clone()).unwrap_or_default();
                let def: ListDef = (0..9u8)
                    .map(|k| {
                        let l = numbering.level(n.num, k).cloned().unwrap_or_default();
                        let start = starts.iter().find(|(lv, _)| *lv == k).map_or(l.start, |(_, s)| *s);
                        (num_format(l.format), level_text(&l.text), start, align(l.align))
                    })
                    .collect();
                self.t.lists.push((n.num, def));
                self.t.lists.len()
            }
        };
        ("NUMBER", id, level)
    }

    /// The hh:style id of paragraph style `id` (바탕글, 0, for Normal and anything unknown).
    fn style_id(&mut self, id: Option<&str>) -> usize {
        let Some(id) = id.filter(|id| !self.standalone && *id != "Normal") else { return 0 };
        let Some(st) = self.doc.styles.get(id).filter(|s| s.kind == wordcraft_doc::StyleKind::Paragraph) else { return 0 };
        if let Some(i) = self.t.styles.iter().position(|s| s.0 == id) {
            return i + 1;
        }
        let props = wordcraft_doc::ParaProps { style: Some(id.to_string()), ..Default::default() };
        let rp = self.doc.styles.resolve_para(&props);
        let rc = self.doc.styles.resolve_char(Some(id), &wordcraft_doc::CharProps::default());
        let para = self.t.para_id(&rp, rc.size);
        let chr = self.t.char_id(&rc);
        let name = if st.name.is_empty() { id.to_string() } else { st.name.clone() };
        self.t.styles.push((id.to_string(), name, para, chr));
        self.t.styles.len()
    }

    fn missing(&mut self, what: &'static str) {
        *self.left_out.entry(what).or_insert(0) += 1;
    }

    /// The paragraphs of story `id` (a note, a text box, a header or footer), `width` wide.
    fn story(&mut self, id: u32, width: f32) -> Vec<OutPara> {
        let mut out = Vec::new();
        let depth = self.depth_now + 1;
        if self.open_stories.contains(&id) || depth >= MAX_DEPTH {
            self.missing("text boxes or notes nested in themselves or too deep");
        } else if self.stories_made >= MAX_STORIES {
            self.missing("text boxes or notes past the most one file writes");
        } else if let Some(part) = self.doc.parts.get(&id) {
            self.stories_made += 1;
            let blocks = part.blocks.clone();
            self.open_stories.push(id);
            self.blocks(&blocks, &mut out, None, width, depth);
            self.open_stories.pop();
        }
        if out.is_empty() {
            out.push(OutPara::empty(0, 0));
        }
        out
    }

    /// An inline object at its place in the text: text of its own, a run's XML (with the height it
    /// takes on the line, points), or nothing.
    fn object(&mut self, o: &InlineObject, width: f32) -> Option<Piece> {
        match o {
            InlineObject::Field { instr, result, .. } => {
                let name = instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
                match name.as_str() {
                    "PAGE" => Some(Piece::Run(auto_num("PAGE", 1, ""), 0.0)),
                    "NUMPAGES" => Some(Piece::Run(auto_num("TOTAL_PAGE", 1, ""), 0.0)),
                    // HWPX's total-page number counts the document; a section's page count has
                    // no number of its own: its shown value is written, and said so
                    "SECTIONPAGES" => {
                        self.missing("section page counts (written as the value they showed)");
                        Some(Piece::Text(result.clone()))
                    }
                    _ => Some(Piece::Text(result.clone())),
                }
            }
            InlineObject::Equation { linear, .. } => Some(Piece::Text(linear.clone())),
            InlineObject::NoteRef { kind, id, .. } => {
                if self.in_note {
                    return None; // the note's own mark: its number comes from the autoNum below
                }
                let k = usize::from(*kind == NoteKind::Endnote);
                let n = self.notes.get(k).copied().unwrap_or(0) + 1;
                if let Some(c) = self.notes.get_mut(k) {
                    *c = n;
                }
                self.in_note = true;
                let mut paras = self.story(*id, width);
                self.in_note = false;
                let (tag, num) = if k == 0 { ("footNote", "FOOTNOTE") } else { ("endNote", "ENDNOTE") };
                if let Some(first) = paras.first_mut() {
                    first.runs.insert(0, (first.mark, auto_num(num, n, ")")));
                }
                let xml = format!(
                    "<hp:ctrl><hp:{tag} number=\"{n}\" suffixChar=\")\" instId=\"{}\">{}</hp:{tag}></hp:ctrl>",
                    1_000_000 + self.notes.iter().sum::<usize>(),
                    sub_list(&paras, "TOP", hu(width), 0)
                );
                Some(Piece::Run(xml, 0.0))
            }
            InlineObject::Image { media, w, h, float, crop, fx, .. } => {
                let Some(data) = self.doc.media.get(media) else {
                    self.missing("pictures without their image data");
                    return None;
                };
                // Hancom's brightness, contrast and transparency aren't read, and how they map to
                // WordCraft's isn't known: written as none, and said so
                if !fx.is_identity() {
                    self.missing("picture brightness, contrast or transparency");
                }
                let item = match self.media.iter().find(|m| m.0 == *media) {
                    Some(m) => m.1.clone(),
                    None => {
                        let (ext, mime) = image_kind(data);
                        let n = self.media.len() + 1;
                        self.media.push((media.clone(), format!("image{n}"), format!("image{n}.{ext}"), mime));
                        format!("image{n}")
                    }
                };
                self.objects += 1;
                let (wd, ht) = self.dims(*w, *h);
                // the picture's own size: its pixels at 96 dpi (75 HWPUNIT each); the crop is in it
                let (pw, ph) = crate::docx::pixel_size(data).map(|(a, c)| (i64::from(a) * 75, i64::from(c) * 75)).unwrap_or((wd, ht));
                let f = |v: f32| f64::from(if v.is_finite() { v.clamp(0.0, 0.99) } else { 0.0 });
                let [cl, ct, cr, cb] = crop.map(f);
                let clip = |full: i64, a: f64, b2: f64| ((full as f64 * a).round() as i64, (full as f64 * (1.0 - b2)).round() as i64);
                let ((l, r), (t, bt)) = (clip(pw, cl, cr), clip(ph, ct, cb));
                let (wrap, tail) = self.placement_tail(wd, ht, float);
                let z = self.objects;
                let xml = format!(
                    "<hp:pic id=\"\" zOrder=\"{z}\" numberingType=\"PICTURE\" textWrap=\"{wrap}\" textFlow=\"BOTH_SIDES\" lock=\"0\" href=\"\" groupLevel=\"{}\" instid=\"{}\" reverse=\"0\">\
{}<hp:imgRect><hc:pt0 x=\"0\" y=\"0\"/><hc:pt1 x=\"{wd}\" y=\"0\"/><hc:pt2 x=\"{wd}\" y=\"{ht}\"/><hc:pt3 x=\"0\" y=\"{ht}\"/></hp:imgRect>\
<hp:imgClip left=\"{l}\" right=\"{r}\" top=\"{t}\" bottom=\"{bt}\"/><hp:inMargin left=\"0\" right=\"0\" top=\"0\" bottom=\"0\"/>\
<hp:imgDim dimwidth=\"{pw}\" dimheight=\"{ph}\"/><hc:img binaryItemIDRef=\"{item}\" bright=\"0\" contrast=\"0\" effect=\"REAL_PIC\" alpha=\"0\"/>{tail}</hp:pic>",
                    self.member.map_or(0, |m| m.level),
                    2_000_000 + z,
                    self.frame(wd, ht, float)
                );
                Some(Piece::Run(xml, if float.wrap == Wrap::Inline { *h } else { 0.0 }))
            }
            InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, story, freeform, .. } => {
                self.shape(*kind, (*w, *h), fill.as_ref(), (*stroke, *stroke_width), float, (*story, freeform.as_deref()))
            }
            InlineObject::Graphic { .. } => {
                self.missing("charts or diagrams");
                None
            }
            InlineObject::Group { w, h, float, ch_w, ch_h, children, .. } => self.group((*w, *h), (*ch_w, *ch_h), children, float, width),
            InlineObject::CommentStart { id } => self.memo(*id),
            InlineObject::CommentEnd { id } => {
                let i = self.memos.open.iter().position(|m| m.0 == *id)?;
                let (_, field) = self.memos.open.remove(i);
                Some(Piece::Run(field_end(field), 0.0))
            }
            _ => None, // bookmarks, content controls, field ranges: their text stays
        }
    }

    /// The start of comment `id`'s range as a memo field holding the comment's text, its replies
    /// after it (Hangul memos have none: each as a line "↳ author" and its text). Comments in
    /// notes, headers and footers or inside another memo are left out.
    fn memo(&mut self, id: u32) -> Option<Piece> {
        if !self.memos.write {
            return None;
        }
        let c = self.doc.comments.get(&id)?;
        if c.parent.is_some() {
            return None; // written in its memo
        }
        if self.memos.inside {
            self.missing("comments inside a comment");
            return None;
        }
        self.t.memos += 1;
        let n = self.t.memos;
        self.memos.inside = true;
        let mut paras = self.story(c.part, MEMO_WIDTH);
        let replies: Vec<&wordcraft_doc::Comment> = self.doc.comments.values().filter(|r| r.parent == Some(id)).collect();
        for r in replies {
            let (para_pr, mark) = paras.first().map_or((0, 0), |p| (p.para_pr, p.mark));
            let mut head = OutPara::empty(para_pr, mark);
            head.runs.push((mark, format!("<hp:t>{}</hp:t>", esc(&format!("\u{21b3} {}", r.author)))));
            paras.push(head);
            paras.extend(self.story(r.part, MEMO_WIDTH));
        }
        self.memos.inside = false;
        let t = filetime(c.date_utc.as_deref().unwrap_or(&c.date)).unwrap_or(0);
        let command = format!("MEMO/65535/{n}/{}/{}/{}/\\;;", t & 0xFFFF_FFFF, t >> 32, field_text(&c.author));
        self.memos.open.push((id, n));
        Some(Piece::Run(
            format!(
                "<hp:ctrl><hp:fieldBegin id=\"{n}\" type=\"MEMO\" name=\"\" editable=\"1\" dirty=\"0\" zorder=\"0\" fieldid=\"{n}\">\
<hp:parameters cnt=\"1\" name=\"\"><hp:stringParam name=\"Command\">{}</hp:stringParam></hp:parameters>{}</hp:fieldBegin></hp:ctrl>",
                esc(&command),
                sub_list(&paras, "TOP", 0, 0)
            ),
            0.0,
        ))
    }

    /// Close the memo fields still open at the end of `out`'s last paragraph: a memo's range
    /// may run over paragraphs (viewer-checked) but not out of its story or section.
    fn close_memos(&mut self, out: &mut [OutPara]) {
        let Some(last) = out.last_mut() else { return };
        for (_, field) in self.memos.open.drain(..) {
            last.runs.push((last.mark, field_end(field)));
        }
    }

    fn shape(
        &mut self,
        kind: ShapeKind,
        (w, h): (f32, f32),
        fill: Option<&wordcraft_doc::fill::Fill>,
        (stroke, stroke_width): (Option<wordcraft_doc::Rgb>, f32),
        float: &wordcraft_doc::para::Float,
        (story, freeform): (Option<u32>, Option<&wordcraft_doc::freeform::Freeform>),
    ) -> Option<Piece> {
        let (wd, ht) = self.dims(w, h);
        let paths = freeform.map(|f| freeform_paths(f, wd, ht)).unwrap_or_default();
        if paths.len() > 1 && self.member.is_none_or(|m| m.level < MAX_GROUP_DEPTH) {
            // several paths: a group of one curve or polygon each, over the same frame
            let mut members = String::new();
            let level = self.member.map_or(0, |m| m.level) + 1;
            let saved = self.member.replace(Member { x: 0, y: 0, sx: 1.0, sy: 1.0, level });
            let at = saved.map(|m| (m.x, m.y)).unwrap_or((0, 0));
            for path in &paths {
                if let Some(Piece::Run(xml, _)) = self.path_shape(path, (wd, ht), fill, (stroke, stroke_width), float) {
                    members.push_str(&xml);
                }
            }
            self.member = saved.map(|m| Member { x: at.0, y: at.1, ..m });
            let piece = self.container(members, (wd, ht), float);
            self.member = saved;
            return Some(piece);
        }
        if let Some(path) = paths.first() {
            // at the deepest group level there's no group to hold the other paths: the first is
            // written, the others are said to be left out
            if paths.len() > 1 {
                self.missing("freeform paths past the first, in groups nested too deep");
            }
            return self.path_shape(path, (wd, ht), fill, (stroke, stroke_width), float);
        }
        let points = polygon_points(kind, wd, ht);
        let tag = match kind {
            _ if story.is_some() => "rect",
            ShapeKind::Rectangle | ShapeKind::RoundedRectangle => "rect",
            // (an OR symbol's spokes are not drawn)
            ShapeKind::Ellipse | ShapeKind::FlowchartOr => "ellipse",
            ShapeKind::Line | ShapeKind::StraightConnector | ShapeKind::ElbowConnector | ShapeKind::CurvedConnector => "line",
            _ if points.is_some() => "polygon",
            _ => {
                self.missing("drawn shapes");
                return None;
            }
        };
        self.objects += 1;
        let z = self.objects;
        let line = Self::line_shape(stroke, stroke_width);
        let brush = match fill {
            Some(wordcraft_doc::fill::Fill::Solid { color, .. }) => {
                format!("<hc:fillBrush><hc:winBrush faceColor=\"#{}\" hatchColor=\"#000000\" alpha=\"0\"/></hc:fillBrush>", color.hex())
            }
            Some(wordcraft_doc::fill::Fill::Gradient(g)) if g.stops.len() >= 2 => {
                let angle = match g.kind {
                    wordcraft_doc::fill::GradientKind::Linear { angle, .. } if angle.is_finite() => angle.round() as i64,
                    _ => 0,
                };
                let cols: String = g.stops.iter().take(16).map(|s| format!("<hc:color value=\"#{}\"/>", s.color.hex())).collect();
                format!(
                    "<hc:fillBrush><hc:gradation type=\"LINEAR\" angle=\"{angle}\" centerX=\"0\" centerY=\"0\" step=\"255\" colorNum=\"{}\" stepCenter=\"50\" alpha=\"0\">{cols}</hc:gradation></hc:fillBrush>",
                    g.stops.len().min(16)
                )
            }
            _ => String::new(),
        };
        let mut text = String::new();
        let mut height = 0.0;
        if let Some(id) = story {
            let body = self.doc.parts.get(&id).map(|p| p.body).unwrap_or_default();
            let [il, it, ir, ib] = body.insets_pt();
            // the story's own objects are placed in its text, not as members of a group the box
            // is in: the box itself keeps its group coordinates
            let member = self.member.take();
            let paras = self.story(id, w - il - ir);
            self.member = member;
            text = format!(
                "<hp:drawText lastWidth=\"{wd}\" name=\"\" editable=\"0\"><hp:textMargin left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\"/>{}</hp:drawText>",
                hu(il),
                hu(ir),
                hu(it),
                hu(ib),
                sub_list(&paras, valign(Some(body.anchor)), hu(w - il - ir).max(0), hu(h - it - ib).max(0))
            );
        }
        let corners = format!("<hc:pt0 x=\"0\" y=\"0\"/><hc:pt1 x=\"{wd}\" y=\"0\"/><hc:pt2 x=\"{wd}\" y=\"{ht}\"/><hc:pt3 x=\"0\" y=\"{ht}\"/>");
        let (geometry, attrs) = match tag {
            "rect" => (corners, format!(" ratio=\"{}\"", if kind == ShapeKind::RoundedRectangle { 20 } else { 0 })),
            "ellipse" => (
                format!(
                    "<hc:center x=\"{}\" y=\"{}\"/><hc:ax1 x=\"{wd}\" y=\"{}\"/><hc:ax2 x=\"{}\" y=\"0\"/><hc:start1 x=\"0\" y=\"0\"/><hc:start2 x=\"0\" y=\"0\"/><hc:end1 x=\"0\" y=\"0\"/><hc:end2 x=\"0\" y=\"0\"/>",
                    wd / 2,
                    ht / 2,
                    ht / 2,
                    wd / 2
                ),
                " intervalDirty=\"0\" hasArcPr=\"0\" arcType=\"NORMAL\"".to_string(),
            ),
            "polygon" => (points.unwrap_or_default().iter().map(|(x, y)| format!("<hc:pt x=\"{x}\" y=\"{y}\"/>")).collect(), String::new()),
            _ => (format!("<hc:startPt x=\"0\" y=\"0\"/><hc:endPt x=\"{wd}\" y=\"{ht}\"/>"), " isReverseHV=\"0\"".to_string()),
        };
        if float.wrap == Wrap::Inline && self.member.is_none() {
            height = h;
        }
        let (wrap, tail) = self.placement_tail(wd, ht, float);
        let xml = format!(
            "<hp:{tag} id=\"\" zOrder=\"{z}\" numberingType=\"NONE\" textWrap=\"{wrap}\" textFlow=\"BOTH_SIDES\" lock=\"0\" href=\"\" groupLevel=\"{}\" instid=\"{}\"{attrs}>\
{}{line}{brush}{text}{geometry}{tail}</hp:{tag}>",
            self.member.map_or(0, |m| m.level),
            3_000_000 + z,
            self.frame(wd, ht, float)
        );
        Some(Piece::Run(xml, height))
    }

    /// An object's frame elements: its own, or as a member of the group being written.
    fn frame(&self, w: i64, h: i64, float: &wordcraft_doc::para::Float) -> String {
        frame_xml(w, h, (w, h), float, self.member)
    }

    /// An object's size `w` x `h` points as shown, HWPUNIT: scaled by the groups around it.
    fn dims(&self, w: f32, h: f32) -> (i64, i64) {
        let (sx, sy) = self.member.map(|m| (m.sx, m.sy)).unwrap_or((1.0, 1.0));
        let v = |pt: f32, s: f64| ((hu(pt) as f64 * s).round() as i64).max(0);
        // one side may be 0 (a level or upright line)
        match (v(w, sx), v(h, sy)) {
            (0, 0) => (1, 1),
            wh => wh,
        }
    }

    /// The textWrap value and the elements after an object's content (hp:sz, hp:pos,
    /// hp:outMargin); a group's member has none of them.
    fn placement_tail(&self, w: i64, h: i64, float: &wordcraft_doc::para::Float) -> (&'static str, String) {
        if self.member.is_some() {
            return ("IN_FRONT_OF_TEXT", String::new());
        }
        let (wrap, pos, margin) = placement_xml(float, self.area);
        (wrap, format!("{}{pos}{margin}", sz_xml(w, h)))
    }

    /// The outline of a drawn shape (hp:lineShape).
    fn line_shape(stroke: Option<wordcraft_doc::Rgb>, stroke_width: f32) -> String {
        let (color, width, style) = match stroke {
            Some(c) => (c.hex(), hu(stroke_width).max(10), "SOLID"),
            None => ("000000".to_string(), 0, "NONE"),
        };
        format!(
            "<hp:lineShape color=\"#{color}\" width=\"{width}\" style=\"{style}\" endCap=\"FLAT\" headStyle=\"NORMAL\" tailStyle=\"NORMAL\" headfill=\"1\" tailfill=\"1\" headSz=\"MEDIUM_MEDIUM\" tailSz=\"MEDIUM_MEDIUM\" outlineStyle=\"NORMAL\" alpha=\"0\"/>"
        )
    }

    /// One freeform path: a polygon when closed (its first point repeated at the end), else a
    /// curve of straight segments.
    fn path_shape(
        &mut self,
        (pts, closed): &(Vec<(i64, i64)>, bool),
        (wd, ht): (i64, i64),
        fill: Option<&wordcraft_doc::fill::Fill>,
        (stroke, stroke_width): (Option<wordcraft_doc::Rgb>, f32),
        float: &wordcraft_doc::para::Float,
    ) -> Option<Piece> {
        let (Some(&first), true) = (pts.first(), pts.len() >= 2) else { return None };
        self.objects += 1;
        let z = self.objects;
        let line = Self::line_shape(stroke, stroke_width);
        let (tag, geometry) = if *closed {
            let mut g: String = pts.iter().map(|(x, y)| format!("<hc:pt x=\"{x}\" y=\"{y}\"/>")).collect();
            if pts.last() != Some(&first) {
                g.push_str(&format!("<hc:pt x=\"{}\" y=\"{}\"/>", first.0, first.1));
            }
            ("polygon", g)
        } else {
            let g: String = pts
                .windows(2)
                .filter_map(|s| {
                    Some(format!(
                        "<hp:seg type=\"LINE\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
                        s.first()?.0,
                        s.first()?.1,
                        s.get(1)?.0,
                        s.get(1)?.1
                    ))
                })
                .collect();
            ("curve", g)
        };
        let brush = match fill {
            Some(wordcraft_doc::fill::Fill::Solid { color, .. }) if *closed => {
                format!("<hc:fillBrush><hc:winBrush faceColor=\"#{}\" hatchColor=\"#000000\" alpha=\"0\"/></hc:fillBrush>", color.hex())
            }
            _ => String::new(),
        };
        let (wrap, tail) = self.placement_tail(wd, ht, float);
        let height = if float.wrap == Wrap::Inline && self.member.is_none() { hu_pt(ht) } else { 0.0 };
        let xml = format!(
            "<hp:{tag} id=\"\" zOrder=\"{z}\" numberingType=\"NONE\" textWrap=\"{wrap}\" textFlow=\"BOTH_SIDES\" lock=\"0\" href=\"\" groupLevel=\"{}\" instid=\"{}\">\
{}{line}{brush}{geometry}{tail}</hp:{tag}>",
            self.member.map_or(0, |m| m.level),
            3_000_000 + z,
            self.frame(wd, ht, float)
        );
        Some(Piece::Run(xml, height))
    }

    /// A group of drawings: its members placed in its own coordinates (`ch_w` x `ch_h`), shown
    /// at `w` x `h`.
    fn group(
        &mut self,
        (w, h): (f32, f32),
        (ch_w, ch_h): (f32, f32),
        children: &[wordcraft_doc::para::GroupChild],
        float: &wordcraft_doc::para::Float,
        width: f32,
    ) -> Option<Piece> {
        let level = self.member.map_or(0, |m| m.level) + 1;
        if level > MAX_GROUP_DEPTH {
            self.missing("groups nested too deep");
            return None;
        }
        let (wd, ht) = self.dims(w, h);
        // the members' coordinates onto the group's shown size
        let ratio = |shown: i64, space: f32, own: f32| shown as f64 / (hu(if space > 0.0 { space } else { own }) as f64).max(1.0);
        let (sx, sy) = (ratio(wd, ch_w, w), ratio(ht, ch_h, h));
        let saved = self.member;
        let mut members = String::new();
        for c in children.iter().take(wordcraft_doc::para::MAX_GROUP_CHILDREN) {
            let at = |v: f32, s: f64| (hu(v) as f64 * s).round() as i64;
            self.member = Some(Member { x: at(c.x, sx), y: at(c.y, sy), sx, sy, level });
            if let Some(Piece::Run(xml, _)) = self.object(&c.obj, width) {
                members.push_str(&xml);
            }
        }
        self.member = saved;
        if members.is_empty() {
            return None;
        }
        Some(self.container(members, (wd, ht), float))
    }

    /// An hp:container holding `members`, `size` HWPUNIT (unscaled: see [`Member`]).
    fn container(&mut self, members: String, cur: (i64, i64), float: &wordcraft_doc::para::Float) -> Piece {
        self.objects += 1;
        let z = self.objects;
        let frame = self.frame(cur.0, cur.1, float);
        let (wrap, tail) = self.placement_tail(cur.0, cur.1, float);
        let height = if float.wrap == Wrap::Inline && self.member.is_none() { hu_pt(cur.1) } else { 0.0 };
        let xml = format!(
            "<hp:container id=\"\" zOrder=\"{z}\" numberingType=\"NONE\" textWrap=\"{wrap}\" textFlow=\"BOTH_SIDES\" lock=\"0\" href=\"\" groupLevel=\"{}\" instid=\"{}\">{frame}{members}{tail}</hp:container>",
            self.member.map_or(0, |m| m.level),
            3_000_000 + z
        );
        Piece::Run(xml, height)
    }

    /// The section's headers and footers (and a first page without them), as controls.
    fn headers_footers(&mut self, sp: &SectionProps) -> String {
        let width = sp.text_width();
        let even_odd = self.doc.settings.even_odd_headers;
        let mut o = String::new();
        let mut hide = [false, false];
        for (k, (tag, set, area)) in
            [("header", &sp.headers, sp.margin_top - sp.header), ("footer", &sp.footers, sp.margin_bottom - sp.footer)].into_iter().enumerate()
        {
            let top = if k == 0 { sp.header } else { sp.page_h - sp.margin_bottom };
            self.area = Some(Area { left: sp.margin_left, top, body_top: sp.margin_top });
            let pages: Vec<(Option<u32>, &str)> =
                if even_odd && set.even.is_some() { vec![(set.default, "ODD"), (set.even, "EVEN")] } else { vec![(set.default, "BOTH")] };
            for (id, apply) in pages {
                let Some(id) = id else { continue };
                let paras = self.story(id, width);
                o.push_str(&format!(
                    "<hp:ctrl><hp:{tag} id=\"\" applyPageType=\"{apply}\">{}</hp:{tag}></hp:ctrl>",
                    sub_list(&paras, "TOP", hu(width), hu(area).max(0))
                ));
            }
            self.area = None;
            if sp.title_page {
                let first_empty = set.first.is_none_or(|id| {
                    self.doc.parts.get(&id).is_none_or(|p| p.blocks.iter().all(|b| matches!(&**b, Block::Para(q) if q.text.trim().is_empty())))
                });
                // HWPX has no first-page header of its own: a blank one hides the default there;
                // one with content isn't written, with or without a default, and is said so
                if !first_empty {
                    self.missing("first-page headers or footers of their own");
                } else if set.default.is_some()
                    && let Some(h) = hide.get_mut(k)
                {
                    *h = true;
                }
            }
        }
        if hide.iter().any(|h| *h) {
            o.push_str(&format!(
                "<hp:ctrl><hp:pageHiding hideHeader=\"{}\" hideFooter=\"{}\" hideMasterPage=\"0\" hideBorder=\"0\" hideFill=\"0\" hidePageNum=\"0\"/></hp:ctrl>",
                b(hide[0]),
                b(hide[1])
            ));
        }
        o
    }

    /// Blocks `width` points wide (the text column or a cell).
    fn blocks(&mut self, blocks: &[std::sync::Arc<Block>], out: &mut Vec<OutPara>, ts: Option<&TableStyleProps>, width: f32, depth: usize) {
        // a nested story (cell, note, text box) closes the memos it opens; the body's run on
        let outer_memos = (depth > 0).then(|| std::mem::take(&mut self.memos.open));
        let from = out.len();
        let outer_depth = std::mem::replace(&mut self.depth_now, depth);
        for b in blocks {
            match &**b {
                Block::Para(p) => self.paragraph(p, out, ts, width),
                Block::Table(t) => {
                    if depth >= MAX_DEPTH {
                        // the depth limit stays; what it leaves out is said
                        self.missing("tables nested too deep");
                        continue;
                    }
                    let (xml, height) = self.table(t, depth);
                    let mut holder = OutPara::empty(0, 0);
                    holder.table = Some(xml);
                    holder.height = height;
                    out.push(holder);
                }
            }
        }
        self.depth_now = outer_depth;
        if let Some(outer) = outer_memos {
            if let Some(mine) = out.get_mut(from..) {
                self.close_memos(mine);
            }
            self.memos.open = outer;
        }
    }

    /// A table as hp:tbl, and its height (points, estimated).
    fn table(&mut self, t: &Table, depth: usize) -> (String, f32) {
        let doc = self.doc;
        let style = t.props.style.as_deref().and_then(|id| doc.styles.table_style(id));
        let mut tb: Borders = style.as_ref().and_then(|s| s.parts.borders).unwrap_or_default();
        if let Some(own) = t.props.borders {
            tb.overlay(&own);
        }
        let def_m = t.props.cell_margins.or(style.as_ref().and_then(|s| s.parts.cell_margins)).unwrap_or(DEFAULT_CELL_MARGINS);
        let base_fill = t.props.shading.or(style.as_ref().and_then(|s| s.parts.fill));
        let header_fill = style.as_ref().and_then(|s| s.parts.header_fill).filter(|_| t.props.look.header_row);
        // the grid: each cell's first column and span
        let placed: Vec<Vec<(usize, usize)>> = t
            .rows
            .iter()
            .map(|row| {
                let mut g = row.first_grid();
                row.cells
                    .iter()
                    .map(|c| {
                        let at = (g, c.span());
                        g = g.saturating_add(c.span());
                        at
                    })
                    .collect()
            })
            .collect();
        let ncols = placed.iter().filter_map(|r| r.last().map(|(g, s)| g + s)).max().unwrap_or(0).max(t.grid.len()).max(1);
        let widths: Vec<f32> = (0..ncols).map(|i| t.grid.get(i).copied().filter(|w| w.is_finite() && *w > 0.0).unwrap_or(36.0)).collect();
        let span_w = |g: usize, s: usize| -> f32 { widths.get(g..(g + s).min(widths.len())).map(|w| w.iter().sum()).unwrap_or(36.0) };
        let continues = |ri: usize, g: usize| {
            t.rows
                .get(ri)
                .zip(placed.get(ri))
                .is_some_and(|(row, at)| row.cells.iter().zip(at).any(|(c, (cg, _))| *cg == g && c.props.vmerge == VMerge::Continue))
        };
        let nrows = t.rows.len();
        // each cell's paragraphs, then the row heights they need
        struct Out {
            ri: usize,
            g: usize,
            span: usize,
            rows: usize,
            fill: usize,
            margins: [f32; 4],
            valign: &'static str,
            header: bool,
            paras: String,
            /// Its numbers or bullets were left out (see [`NUMBERED_CELL`]).
            numbered: bool,
        }
        let mut cells: Vec<Out> = Vec::new();
        let row_h: Vec<f32> = t
            .rows
            .iter()
            .map(|r| {
                if r.props.height_rule == HeightRule::Auto {
                    return 0.0;
                }
                // DOCX row heights leave out the cells' top and bottom margins (Word adds them);
                // HWP cell heights include them.
                let pad = r.cells.iter().map(|c| c.props.resolved_margins(def_m)).map(|m| m[0] + m[2]).fold(0.0f32, f32::max);
                r.props.height.filter(|h| h.is_finite()).unwrap_or(0.0).max(0.0) + pad
            })
            .collect();
        // The rows' heights with the text's estimated need: only the table's size. A cell's height
        // is its row's own: Hancom grows a row to fit its text when it lays the table out (its own
        // files hold cells only their margins tall around many lines). The estimate can be several
        // times too tall, and as a cell's height it came back as the row's height on reopening.
        let mut row_need = row_h.clone();
        for (ri, row) in t.rows.iter().enumerate() {
            for (cell, &(g, span)) in row.cells.iter().zip(placed.get(ri).map(Vec::as_slice).unwrap_or(&[])) {
                if cell.props.vmerge == VMerge::Continue {
                    continue;
                }
                // A continued cell joins the cell above it, also when that one doesn't start a
                // merge (a row inserted inside a merge): skipped otherwise, it would leave a hole.
                let mut rows = 1;
                while ri + rows < nrows && continues(ri + rows, g) {
                    rows += 1;
                }
                let cb = cell.props.borders.unwrap_or_default();
                let last = ri + rows >= nrows;
                let edge = |own: Option<Border>, outer: bool, ob: Option<Border>, ib: Option<Border>| own.or(if outer { ob } else { ib });
                let sides = [
                    side(edge(cb.left, g == row.first_grid(), tb.left, tb.inside_v)),
                    side(edge(cb.right, g + span >= ncols, tb.right, tb.inside_v)),
                    side(edge(cb.top, ri == 0, tb.top, tb.between)),
                    side(edge(cb.bottom, last, tb.bottom, tb.between)),
                ];
                let fill = cell.props.shading.or(if ri == 0 { header_fill } else { None }).or(base_fill);
                let fill = self.t.fill_id(FillKey { sides, fill: fill.map(|c| format!("#{}", c.hex())) });
                let margins = cell.props.resolved_margins(def_m);
                let w = span_w(g, span);
                let mut paras = Vec::new();
                let numbers_before = self.left_out.get(NUMBERS_LEFT_OUT).copied().unwrap_or(0);
                self.blocks(&cell.blocks, &mut paras, style.as_ref(), w - margins[1] - margins[3], depth + 1);
                let numbered = self.left_out.get(NUMBERS_LEFT_OUT).copied().unwrap_or(0) > numbers_before;
                if paras.is_empty() {
                    paras.push(OutPara::empty(0, 0));
                }
                let need = paras.iter().map(|p| p.height).sum::<f32>() + margins[0] + margins[2];
                if rows == 1
                    && let Some(h) = row_need.get_mut(ri)
                {
                    *h = h.max(need);
                }
                let xml: String = paras.iter().map(|p| para_xml(p, None)).collect();
                cells.push(Out {
                    ri,
                    g,
                    span,
                    rows,
                    fill,
                    margins,
                    valign: valign(cell.props.valign),
                    header: row.props.header,
                    paras: xml,
                    numbered,
                });
            }
        }
        let mut rows_xml = vec![String::new(); nrows];
        for c in &cells {
            let w = span_w(c.g, c.span);
            let h: f32 = row_h.get(c.ri..(c.ri + c.rows).min(nrows)).map(|r| r.iter().sum()).unwrap_or(0.0).max(c.margins[0] + c.margins[2] + 1.0);
            let [mt, ml, mb, mr] = c.margins;
            if let Some(r) = rows_xml.get_mut(c.ri) {
                let mark = if c.numbered && self.standalone { format!(" {NUMBERED_CELL}=\"1\"") } else { String::new() };
                r.push_str(&format!(
                    "<hp:tc{mark} name=\"\" header=\"{}\" hasMargin=\"1\" protect=\"0\" editable=\"0\" dirty=\"0\" borderFillIDRef=\"{}\">\
<hp:subList id=\"\" textDirection=\"HORIZONTAL\" lineWrap=\"BREAK\" vertAlign=\"{}\" linkListIDRef=\"0\" linkListNextIDRef=\"0\" textWidth=\"{}\" textHeight=\"{}\" hasTextRef=\"0\" hasNumRef=\"0\">{}</hp:subList>\
<hp:cellAddr colAddr=\"{}\" rowAddr=\"{}\"/><hp:cellSpan colSpan=\"{}\" rowSpan=\"{}\"/><hp:cellSz width=\"{}\" height=\"{}\"/>\
<hp:cellMargin left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\"/></hp:tc>",
                    b(c.header),
                    c.fill,
                    c.valign,
                    hu(w - ml - mr).max(0),
                    hu(h - mt - mb).max(0),
                    c.paras,
                    c.g,
                    c.ri,
                    c.span,
                    c.rows,
                    hu(w),
                    hu(h),
                    hu(ml),
                    hu(mr),
                    hu(mt),
                    hu(mb)
                ));
            }
        }
        let total_w: f32 = widths.iter().sum();
        let total_h: f32 = row_need.iter().sum();
        let (ha, offset) = match t.props.align {
            Some(Align::Center) => ("CENTER", 0.0),
            Some(Align::Right) => ("RIGHT", 0.0),
            _ => ("LEFT", t.props.indent.filter(|i| i.is_finite()).unwrap_or(0.0)),
        };
        let repeat = t.rows.first().is_some_and(|r| r.props.header);
        let rows: String = rows_xml.iter().map(|r| format!("<hp:tr>{r}</hp:tr>")).collect();
        let [mt, ml, mb, mr] = def_m;
        let xml = format!(
            "<hp:tbl id=\"\" zOrder=\"0\" numberingType=\"TABLE\" textWrap=\"TOP_AND_BOTTOM\" textFlow=\"BOTH_SIDES\" lock=\"0\" pageBreak=\"CELL\" repeatHeader=\"{}\" rowCnt=\"{nrows}\" colCnt=\"{ncols}\" cellSpacing=\"0\" borderFillIDRef=\"1\" noAdjust=\"0\">\
<hp:sz width=\"{}\" widthRelTo=\"ABSOLUTE\" height=\"{}\" heightRelTo=\"ABSOLUTE\" protect=\"0\"/>\
<hp:pos treatAsChar=\"0\" affectLSpacing=\"0\" flowWithText=\"1\" allowOverlap=\"0\" holdAnchorAndSO=\"0\" vertRelTo=\"PARA\" horzRelTo=\"COLUMN\" vertAlign=\"TOP\" horzAlign=\"{ha}\" vertOffset=\"0\" horzOffset=\"{}\"/>\
<hp:outMargin left=\"0\" right=\"0\" top=\"0\" bottom=\"0\"/><hp:inMargin left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\"/>{rows}</hp:tbl>",
            b(repeat),
            hu(total_w),
            hu(total_h),
            hu(offset),
            hu(ml),
            hu(mr),
            hu(mt),
            hu(mb)
        );
        (xml, total_h)
    }
}

/// One paragraph's XML; `sec_pr` (the section's settings) rides in the first run of a section's
/// first paragraph.
fn para_xml(p: &OutPara, sec_pr: Option<&str>) -> String {
    let mut o = format!(
        "<hp:p id=\"{PARAGRAPH_ID}\" paraPrIDRef=\"{}\" styleIDRef=\"{}\" pageBreak=\"{}\" columnBreak=\"{}\" merged=\"0\">",
        p.para_pr,
        p.style,
        b(p.page_break),
        b(p.column_break)
    );
    let first_char = p.runs.first().map(|r| r.0).unwrap_or(p.mark);
    if let Some(sec) = sec_pr {
        o.push_str(&format!("<hp:run charPrIDRef=\"{first_char}\">{sec}</hp:run>"));
    }
    if let Some(tbl) = &p.table {
        o.push_str(&format!("<hp:run charPrIDRef=\"{}\">{tbl}</hp:run>", p.mark));
    } else if p.runs.is_empty() {
        o.push_str(&format!("<hp:run charPrIDRef=\"{}\"/>", p.mark));
    }
    for (id, inner) in &p.runs {
        o.push_str(&format!("<hp:run charPrIDRef=\"{id}\">{inner}</hp:run>"));
    }
    o.push_str("</hp:p>");
    o
}

/// The section's footnote and endnote shapes: numbers 1) 2) 3) counted through the document,
/// footnotes under each column's text and endnotes at the document's end, and a solid 0.12 mm
/// separator as long as WordCraft draws it (144 pt). Without them Hancom draws no separator.
const NOTE_SHAPES: &str = concat!(
    "<hp:footNotePr><hp:autoNumFormat type=\"DIGIT\" userChar=\"\" prefixChar=\"\" suffixChar=\")\" supscript=\"0\"/>",
    "<hp:noteLine length=\"14400\" type=\"SOLID\" width=\"0.12 mm\" color=\"#000000\"/>",
    "<hp:noteSpacing betweenNotes=\"283\" belowLine=\"567\" aboveLine=\"850\"/>",
    "<hp:numbering type=\"CONTINUOUS\" newNum=\"1\"/><hp:placement place=\"EACH_COLUMN\" beneathText=\"0\"/></hp:footNotePr>",
    "<hp:endNotePr><hp:autoNumFormat type=\"DIGIT\" userChar=\"\" prefixChar=\"\" suffixChar=\")\" supscript=\"0\"/>",
    "<hp:noteLine length=\"14400\" type=\"SOLID\" width=\"0.12 mm\" color=\"#000000\"/>",
    "<hp:noteSpacing betweenNotes=\"0\" belowLine=\"567\" aboveLine=\"850\"/>",
    "<hp:numbering type=\"CONTINUOUS\" newNum=\"1\"/><hp:placement place=\"END_OF_DOCUMENT\" beneathText=\"0\"/></hp:endNotePr>",
);

fn sec_pr_xml(sp: &SectionProps, memo_shape: bool) -> String {
    // HWP keeps a portrait sheet's size and turns it with `landscape` (NARROWLY = landscape).
    let (w, h) = (sp.page_w.min(sp.page_h), sp.page_w.max(sp.page_h));
    let landscape = if sp.landscape { "NARROWLY" } else { "WIDELY" };
    // DOCX measures the body from the page edge; HWP from the header/footer area, which starts
    // at its top/bottom margin (the reader maps them the same way).
    let top = sp.header.max(0.0).min(sp.margin_top);
    let bottom = sp.footer.max(0.0).min(sp.margin_bottom);
    format!(
        "<hp:secPr id=\"\" textDirection=\"HORIZONTAL\" spaceColumns=\"{}\" tabStop=\"8000\" tabStopVal=\"4000\" tabStopUnit=\"HWPUNIT\" outlineShapeIDRef=\"0\" memoShapeIDRef=\"{}\" textVerticalWidthHead=\"0\" masterPageCnt=\"0\">\
<hp:grid lineGrid=\"0\" charGrid=\"0\" wonggojiFormat=\"0\"/>\
<hp:startNum pageStartsOn=\"BOTH\" page=\"{}\" pic=\"0\" tbl=\"0\" equation=\"0\"/>\
<hp:visibility hideFirstHeader=\"0\" hideFirstFooter=\"0\" hideFirstMasterPage=\"0\" border=\"SHOW_ALL\" fill=\"SHOW_ALL\" hideFirstPageNum=\"0\" hideFirstEmptyLine=\"0\" showLineNumber=\"0\"/>\
<hp:lineNumberShape restartType=\"0\" countBy=\"0\" distance=\"0\" startNumber=\"0\"/>\
<hp:pagePr landscape=\"{landscape}\" width=\"{}\" height=\"{}\" gutterType=\"LEFT_ONLY\">\
<hp:margin header=\"{}\" footer=\"{}\" gutter=\"{}\" left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\"/></hp:pagePr>\
{NOTE_SHAPES}</hp:secPr>{}",
        hu(sp.columns.space),
        u8::from(memo_shape),
        sp.page_num_start.unwrap_or(0),
        hu(w),
        hu(h),
        hu(sp.margin_top - top).max(0),
        hu(sp.margin_bottom - bottom).max(0),
        hu(sp.gutter).max(0),
        hu(sp.margin_left).max(0),
        hu(sp.margin_right).max(0),
        hu(top),
        hu(bottom),
        col_pr_xml(sp)
    )
}

/// The section's columns, as a control (HWP changes the number of columns in a paragraph; a
/// new section always starts a new page).
fn col_pr_xml(sp: &SectionProps) -> String {
    format!(
        "<hp:ctrl><hp:colPr id=\"\" type=\"NEWSPAPER\" layout=\"LEFT\" colCount=\"{}\" sameSz=\"1\" sameGap=\"{}\"/></hp:ctrl>",
        sp.columns.count.clamp(1, 255),
        hu(sp.columns.space).max(0)
    )
}

fn section_xml(paras: &[OutPara], sp: &SectionProps, controls: &str, memo_shape: bool) -> String {
    let mut o = format!("{DECL}<hs:sec {NS}>");
    let sec = sec_pr_xml(sp, memo_shape) + controls;
    for (i, p) in paras.iter().enumerate() {
        o.push_str(&para_xml(p, (i == 0).then_some(sec.as_str())));
    }
    o.push_str("</hs:sec>");
    o
}

fn header_xml(t: &Tables, sections: usize) -> String {
    let mut o = format!("{DECL}<hh:head {NS} version=\"1.4\" secCnt=\"{sections}\">");
    o.push_str("<hh:beginNum page=\"1\" footnote=\"1\" endnote=\"1\" pic=\"1\" tbl=\"1\" equation=\"1\"/><hh:refList>");
    o.push_str("<hh:fontfaces itemCnt=\"7\">");
    for (k, lang) in FONT_LANGS.iter().enumerate() {
        let fonts = t.fonts.get(k).map(Vec::as_slice).unwrap_or(&[]);
        o.push_str(&format!("<hh:fontface lang=\"{lang}\" fontCnt=\"{}\">", fonts.len()));
        for (i, f) in fonts.iter().enumerate() {
            o.push_str(&format!("<hh:font id=\"{i}\" face={} type=\"TTF\" isEmbedded=\"0\"/>", attr(f)));
        }
        o.push_str("</hh:fontface>");
    }
    o.push_str("</hh:fontfaces>");
    // borderFill 1: no lines, no fill (what characters and paragraphs refer to); then the cells'
    o.push_str(&format!("<hh:borderFills itemCnt=\"{}\">", t.fills.len() + 1));
    for (i, k) in std::iter::once(&FillKey::none()).chain(&t.fills).enumerate() {
        o.push_str(&fill_xml(i + 1, k));
    }
    o.push_str("</hh:borderFills>");
    o.push_str(&format!("<hh:charProperties itemCnt=\"{}\">", t.char_order.len()));
    for (i, k) in t.char_order.iter().enumerate() {
        o.push_str(&char_pr_xml(i, k));
    }
    o.push_str("</hh:charProperties>");
    o.push_str("<hh:tabProperties itemCnt=\"1\"><hh:tabPr id=\"0\" autoTabLeft=\"0\" autoTabRight=\"0\"/></hh:tabProperties>");
    o.push_str(&lists_xml(t));
    o.push_str(&format!("<hh:paraProperties itemCnt=\"{}\">", t.para_order.len()));
    for (i, k) in t.para_order.iter().enumerate() {
        o.push_str(&para_pr_xml(i, k));
    }
    o.push_str("</hh:paraProperties>");
    o.push_str(&styles_xml(t));
    if t.memos > 0 {
        o.push_str(MEMO_PROPERTIES);
    }
    o.push_str("</hh:refList></hh:head>");
    o
}

/// A paraHead's attributes after `level` (the number sits at the start of the first line; the
/// paragraph's own indents place it).
const PARA_HEAD: &str = "useInstWidth=\"0\" autoIndent=\"0\" widthAdjust=\"0\" textOffsetType=\"PERCENT\" textOffset=\"50\"";

/// hh:numberings and hh:bullets.
fn lists_xml(t: &Tables) -> String {
    let mut o = String::new();
    if !t.lists.is_empty() {
        o.push_str(&format!("<hh:numberings itemCnt=\"{}\">", t.lists.len()));
        for (i, (_, levels)) in t.lists.iter().enumerate() {
            o.push_str(&format!("<hh:numbering id=\"{}\" start=\"0\">", i + 1));
            for (k, (fmt, text, start, al)) in levels.iter().enumerate() {
                o.push_str(&format!(
                    "<hh:paraHead start=\"{start}\" level=\"{}\" align=\"{al}\" {PARA_HEAD} numFormat=\"{fmt}\" charPrIDRef=\"4294967295\" checkable=\"0\">{}</hh:paraHead>",
                    k + 1,
                    esc(text)
                ));
            }
            o.push_str("</hh:numbering>");
        }
        o.push_str("</hh:numberings>");
    }
    if !t.bullets.is_empty() {
        o.push_str(&format!("<hh:bullets itemCnt=\"{}\">", t.bullets.len()));
        for (i, c) in t.bullets.iter().enumerate() {
            o.push_str(&format!(
                "<hh:bullet id=\"{}\" char={} useImage=\"0\"><hh:paraHead level=\"0\" align=\"LEFT\" {PARA_HEAD} numFormat=\"DIGIT\" charPrIDRef=\"4294967295\" checkable=\"0\"/></hh:bullet>",
                i + 1,
                attr(&c.to_string())
            ));
        }
        o.push_str("</hh:bullets>");
    }
    o
}

/// hh:styles: 바탕글 (Normal), then the paragraph styles the text uses, under their names.
fn styles_xml(t: &Tables) -> String {
    let style = |id: usize, name: &str, eng: &str, para: usize, chr: usize| {
        format!(
            "<hh:style id=\"{id}\" type=\"PARA\" name={} engName={} paraPrIDRef=\"{para}\" charPrIDRef=\"{chr}\" nextStyleIDRef=\"{id}\" langID=\"1042\" lockForm=\"0\"/>",
            attr(name),
            attr(eng)
        )
    };
    let mut o = format!("<hh:styles itemCnt=\"{}\">", t.styles.len() + 1);
    o.push_str(&style(0, "바탕글", "Normal", 0, 0));
    for (i, (_, name, para, chr)) in t.styles.iter().enumerate() {
        o.push_str(&style(i + 1, name, name, *para, *chr));
    }
    o.push_str("</hh:styles>");
    o
}

fn package_xml(title: &str, sections: usize, media: &[(String, String, String, &'static str)]) -> (String, String, String) {
    let container = format!(
        "{DECL}<ocf:container xmlns:ocf=\"urn:oasis:names:tc:opendocument:xmlns:container\" xmlns:hpf=\"http://www.hancom.co.kr/schema/2011/hpf\"><ocf:rootfiles><ocf:rootfile full-path=\"Contents/content.hpf\" media-type=\"application/hwpml-package+xml\"/></ocf:rootfiles></ocf:container>"
    );
    let mut items = String::from("<opf:item id=\"header\" href=\"Contents/header.xml\" media-type=\"application/xml\"/>");
    let mut spine = String::from("<opf:itemref idref=\"header\" linear=\"yes\"/>");
    for (_, id, name, mime) in media {
        items.push_str(&format!("<opf:item id=\"{id}\" href=\"BinData/{name}\" media-type=\"{mime}\" isEmbeded=\"1\"/>"));
    }
    for i in 0..sections {
        items.push_str(&format!("<opf:item id=\"section{i}\" href=\"Contents/section{i}.xml\" media-type=\"application/xml\"/>"));
        spine.push_str(&format!("<opf:itemref idref=\"section{i}\" linear=\"yes\"/>"));
    }
    let hpf = format!(
        "{DECL}<opf:package {NS} version=\"\" unique-identifier=\"\" id=\"\"><opf:metadata><opf:title>{}</opf:title><opf:language>ko</opf:language></opf:metadata><opf:manifest>{items}</opf:manifest><opf:spine>{spine}</opf:spine></opf:package>",
        esc(title)
    );
    let version = format!(
        "{DECL}<hv:HCFVersion xmlns:hv=\"http://www.hancom.co.kr/hwpml/2011/version\" tagetApplication=\"WORDPROCESSOR\" major=\"5\" minor=\"1\" micro=\"0\" buildNumber=\"1\" os=\"1\" xmlVersion=\"1.4\" application=\"WordCraft\" appVersion=\"{}\"/>",
        env!("CARGO_PKG_VERSION")
    );
    (container, hpf, version)
}

/// The fonts the reader swapped for installed stand-ins, original -> stand-in.
fn font_swaps(doc: &Document) -> BTreeMap<String, String> {
    doc.custom_props.iter().find(|p| p.name == crate::fonts::PROPERTY).map(|p| crate::fonts::swaps(&p.value)).unwrap_or_default()
}

/// The original names of the fonts the reader swapped for installed stand-ins (see
/// [`crate::fonts`]), by stand-in.
fn font_back(doc: &Document) -> BTreeMap<String, String> {
    doc.custom_props.iter().find(|p| p.name == crate::fonts::PROPERTY).map(|p| crate::fonts::originals(&p.value)).unwrap_or_default()
}

/// The pictures a written piece of XML refers to: (media key, item id, file name, media type).
pub(crate) type PieceMedia = Vec<(String, String, String, &'static str)>;

/// A table of `doc` as hp:tbl XML for an existing HWPX, with the shapes it uses (ids from 0 for
/// characters and paragraphs, from 1 for border fills; see `restyle::HeaderEdit::add_generated`)
/// and the pictures in it (items from "image1", to be renamed into the package's).
/// A drawing, text box or group of `doc` as XML for an existing HWPX, with the shapes its text
/// uses and the pictures in it (as for [`table_alone`]). `None` for what this writer leaves out.
pub(crate) fn object_alone(doc: &Document, o: &InlineObject, n: usize) -> Option<(String, Tables, SaveReport, PieceMedia)> {
    let mut bld = Builder {
        doc,
        t: Tables { font_back: font_back(doc), font_swaps: font_swaps(doc), ..Tables::default() },
        report: SaveReport::default(),
        media: Vec::new(),
        notes: [0, 0],
        objects: n,
        in_note: false,
        area: None,
        left_out: BTreeMap::new(),
        standalone: true,
        member: None,
        memos: Memos::default(),
        depth_now: 0,
        open_stories: Vec::new(),
        stories_made: 0,
    };
    let base_p = doc.styles.resolve_para(&wordcraft_doc::ParaProps::default());
    let base_c = doc.styles.resolve_char(None, &wordcraft_doc::CharProps::default());
    bld.t.para_id(&base_p, base_c.size);
    bld.t.char_id(&base_c);
    let Some(Piece::Run(xml, _)) = bld.object(o, 451.0) else { return None };
    // members of a group (or text in a box) this writer left out are reported, as for tables
    for (what, n) in &bld.left_out {
        bld.report.skipped.push(format!("{n} {what} in an edited object left out"));
    }
    Some((xml, bld.t, bld.report, bld.media))
}

pub(crate) fn table_alone(doc: &Document, t: &Table) -> (String, Tables, SaveReport, PieceMedia) {
    let mut bld = Builder {
        doc,
        t: Tables { font_back: font_back(doc), font_swaps: font_swaps(doc), ..Tables::default() },
        report: SaveReport::default(),
        media: Vec::new(),
        notes: [0, 0],
        objects: 0,
        in_note: false,
        area: None,
        left_out: BTreeMap::new(),
        standalone: true,
        member: None,
        memos: Memos::default(),
        depth_now: 0,
        open_stories: Vec::new(),
        stories_made: 0,
    };
    let base_p = doc.styles.resolve_para(&wordcraft_doc::ParaProps::default());
    let base_c = doc.styles.resolve_char(None, &wordcraft_doc::CharProps::default());
    bld.t.para_id(&base_p, base_c.size);
    bld.t.char_id(&base_c);
    let (xml, _) = bld.table(t, 1);
    for (what, n) in &bld.left_out {
        bld.report.skipped.push(format!("{n} {what} in an edited table left out"));
    }
    (xml, bld.t, bld.report, bld.media)
}

/// A picture of `doc` as hp:pic XML for an existing HWPX, its image being the package item
/// `item` (the `n`-th object added: its z-order and instance id). With the image's file
/// extension and media type; `None` without its image data.
/// A picture's XML alone (a new or rewritten picture of a write-back), its type, and what of it
/// isn't written (for the save's report).
pub(crate) fn picture_alone(doc: &Document, o: &InlineObject, item: &str, n: usize) -> Option<(String, &'static str, &'static str, Vec<String>)> {
    let InlineObject::Image { media, .. } = o else { return None };
    let (ext, mime) = image_kind(doc.media.get(media)?);
    let mut bld = Builder {
        doc,
        t: Tables::default(),
        report: SaveReport::default(),
        media: Vec::new(),
        notes: [0, 0],
        objects: n,
        in_note: false,
        area: None,
        left_out: BTreeMap::new(),
        standalone: true,
        member: None,
        memos: Memos::default(),
        depth_now: 0,
        open_stories: Vec::new(),
        stories_made: 0,
    };
    let Some(Piece::Run(xml, _)) = bld.object(o, 0.0) else { return None };
    let left_out = bld.left_out.keys().map(|what| format!("a picture's {what} not written")).collect();
    Some((xml.replacen("binaryItemIDRef=\"image1\"", &format!("binaryItemIDRef=\"{}\"", crate::docx::esc(item)), 1), ext, mime, left_out))
}

/// A new HWPX package holding `doc`, and what it could not hold yet.
pub fn from_document(doc: &Document) -> Result<(Vec<u8>, SaveReport)> {
    package(doc, true)
}

/// [`from_document`], with comments as memos when `memos` (a new .hwp, made from this package,
/// does not write them yet).
pub(crate) fn package(doc: &Document, memos: bool) -> Result<(Vec<u8>, SaveReport)> {
    let mut bld = Builder {
        doc,
        t: Tables { font_back: font_back(doc), font_swaps: font_swaps(doc), ..Tables::default() },
        report: SaveReport::default(),
        media: Vec::new(),
        notes: [0, 0],
        objects: 0,
        in_note: false,
        area: None,
        left_out: BTreeMap::new(),
        standalone: false,
        member: None,
        memos: Memos { write: memos, ..Memos::default() },
        depth_now: 0,
        open_stories: Vec::new(),
        stories_made: 0,
    };
    // the default shapes come first (ids 0), from the Normal style
    let base_p = doc.styles.resolve_para(&wordcraft_doc::ParaProps::default());
    let base_c = doc.styles.resolve_char(None, &wordcraft_doc::CharProps::default());
    bld.t.para_id(&base_p, base_c.size);
    bld.t.char_id(&base_c);
    // the body, split into sections where a paragraph ends one
    let mut sections: Vec<(Vec<OutPara>, &SectionProps)> = Vec::new();
    let mut cur: Vec<OutPara> = Vec::new();
    let secs = doc.sections();
    for (cur_index, blk) in doc.body.iter().enumerate() {
        let width = secs.iter().find(|(i, _)| *i >= cur_index).map(|(_, sp)| sp.text_width()).unwrap_or(451.0);
        bld.blocks(std::slice::from_ref(blk), &mut cur, None, width, 0);
        if let Block::Para(p) = &**blk
            && let Some(sp) = &p.section
        {
            // A memo can't run out of an HWPX section, but a continuous section after this one
            // joins it (below): its memos stay open across the change of columns.
            let next = secs.iter().find(|(i, _)| *i > cur_index).map_or(&doc.last_section, |(_, n)| *n);
            if next.start != wordcraft_doc::section::SectionStart::Continuous {
                bld.close_memos(&mut cur);
            }
            sections.push((std::mem::take(&mut cur), sp));
        }
    }
    bld.close_memos(&mut cur);
    if !cur.is_empty() || sections.is_empty() {
        sections.push((cur, &doc.last_section));
    }
    for (paras, _) in sections.iter_mut() {
        if paras.is_empty() {
            paras.push(OutPara::empty(0, 0));
        }
    }
    // A continuous section (other columns further down the same page) stays in the HWP section
    // before it: HWP starts every section on a new page, and changes columns with a control in
    // the paragraph where they change.
    let mut grouped: Vec<(Vec<OutPara>, &SectionProps)> = Vec::new();
    for (mut paras, sp) in sections {
        match grouped.last_mut() {
            Some(last) if sp.start == wordcraft_doc::section::SectionStart::Continuous => {
                if let Some(first) = paras.first_mut() {
                    first.runs.insert(0, (first.mark, col_pr_xml(sp)));
                }
                last.0.extend(paras);
            }
            _ => grouped.push((paras, sp)),
        }
    }
    let sections = grouped;
    let controls: Vec<String> = sections.iter().map(|(_, sp)| bld.headers_footers(sp)).collect();
    let comments = doc.comments.values().filter(|c| c.parent.is_none()).count();
    if !memos && comments > 0 {
        bld.missing("comments");
    } else if comments > bld.t.memos {
        // no range in the body text to hang the memo on
        *bld.left_out.entry("comments").or_insert(0) += comments - bld.t.memos;
    }
    for (what, n) in &bld.left_out {
        bld.report.skipped.push(format!("{n} {what} left out"));
    }
    let memo_shape = bld.t.memos > 0;
    let section_xmls: Vec<String> = sections.iter().zip(&controls).map(|((p, sp), c)| section_xml(p, sp, c, memo_shape)).collect();
    let header = header_xml(&bld.t, section_xmls.len());
    let (container, hpf, version) = package_xml(&doc.core.title, section_xmls.len(), &bld.media);

    let out = {
        let mut z = craft_zip::Writer::new();
        let (stored, deflated) = (craft_zip::Compression::Stored, craft_zip::Compression::Deflated);
        let mut add = |name: &str, data: &[u8], how| -> Result<()> { z.add(name, data, how).map_err(Error::format) };
        // the container wants "mimetype" first and stored
        add("mimetype", b"application/hwp+zip", stored)?;
        add("version.xml", version.as_bytes(), deflated)?;
        add("META-INF/container.xml", container.as_bytes(), deflated)?;
        add("Contents/content.hpf", hpf.as_bytes(), deflated)?;
        add("Contents/header.xml", header.as_bytes(), deflated)?;
        for (i, s) in section_xmls.iter().enumerate() {
            add(&format!("Contents/section{i}.xml"), s.as_bytes(), deflated)?;
        }
        for (key, _, name, _) in &bld.media {
            if let Some(data) = doc.media.get(key) {
                add(&format!("BinData/{name}"), data, deflated)?;
            }
        }
        z.finish().map_err(Error::format)?
    };
    Ok((out, bld.report))
}
