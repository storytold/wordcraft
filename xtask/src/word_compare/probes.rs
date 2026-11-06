//! The synthetic probe documents: each isolates one layout rule (fonts, line spacing,
//! alignment, indents, tabs, raised and lowered text, pagination controls, tables, pictures,
//! notes, lists, headers, columns). Text is made-up words from a fixed seed, never real prose.

use super::package::{Parts, inline_picture};

/// One probe: its name, what it checks, and its parts.
pub struct Probe {
    pub name: String,
    pub about: &'static str,
    pub spec: Spec,
}

#[derive(Default)]
pub struct Spec {
    pub body: String,
    pub sect: String,
    pub rpr: String,
    pub ppr: String,
    pub styles: String,
    pub numbering: String,
    pub footnotes: String,
    pub headers: Vec<String>,
    pub pictures: Vec<(u32, u32, [u8; 3])>,
}

impl Spec {
    pub fn parts(&self) -> Parts<'_> {
        Parts {
            body: &self.body,
            sect: &self.sect,
            default_rpr: &self.rpr,
            default_ppr: &self.ppr,
            styles: &self.styles,
            numbering: &self.numbering,
            footnotes: &self.footnotes,
            headers: &self.headers,
            pictures: &self.pictures,
        }
    }
}

/// A4 with 1-inch margins, header and footer 0.5 inch from the edge.
pub const A4: &str = r#"<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;

/// Run properties naming one font for every script, at `half_points`.
pub fn font(name: &str, half_points: u32) -> String {
    format!(
        r#"<w:rFonts w:ascii="{name}" w:hAnsi="{name}" w:eastAsia="{name}" w:cs="{name}"/><w:sz w:val="{half_points}"/><w:szCs w:val="{half_points}"/><w:lang w:val="en-US"/>"#
    )
}

/// Made-up words from a fixed seed: deterministic, pronounceable, of varied length.
pub struct Words(u64);

impl Words {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self, n: u64) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % n
    }

    pub fn word(&mut self) -> String {
        const SYL: [&str; 24] = [
            "ka", "lo", "mi", "ter", "san", "vel", "or", "im", "pa", "ne", "dor", "ust", "re", "ba", "fin", "co", "la", "ti", "gen", "ar", "mus",
            "quel", "by", "the",
        ];
        let n = [1, 1, 2, 2, 2, 3, 3, 4][self.next(8) as usize];
        (0..n).map(|_| SYL[self.next(SYL.len() as u64) as usize]).collect()
    }

    /// `n` words as sentences of 6–16 words.
    pub fn text(&mut self, n: usize) -> String {
        let mut out = String::new();
        let mut left = 0;
        for i in 0..n {
            if left == 0 {
                left = 6 + self.next(11) as usize;
            }
            let mut w = self.word();
            if out.is_empty() || out.ends_with(". ") {
                w = capital(&w);
            }
            out.push_str(&w);
            left -= 1;
            out.push_str(if left == 0 || i + 1 == n {
                ". "
            } else if self.next(9) == 0 {
                ", "
            } else {
                " "
            });
        }
        out.trim_end().to_string()
    }
}

fn capital(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// A paragraph with `ppr` (`w:pPr` children) and the given runs.
pub fn p(ppr: &str, runs: &str) -> String {
    if ppr.is_empty() { format!("<w:p>{runs}</w:p>") } else { format!("<w:p><w:pPr>{ppr}</w:pPr>{runs}</w:p>") }
}

/// A run of text with `rpr` (`w:rPr` children); tabs in `text` become `w:tab`.
pub fn r(rpr: &str, text: &str) -> String {
    let rpr = if rpr.is_empty() { String::new() } else { format!("<w:rPr>{rpr}</w:rPr>") };
    let inner: Vec<String> = text.split('\t').map(|t| format!(r#"<w:t xml:space="preserve">{}</w:t>"#, esc(t))).collect();
    format!("<w:r>{rpr}{}</w:r>", inner.join("<w:tab/>"))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// `n` paragraphs of about `words` words each.
fn paras(w: &mut Words, n: usize, words: usize, ppr: &str) -> String {
    (0..n).map(|_| p(ppr, &r("", &w.text(words)))).collect()
}

fn probe(name: impl Into<String>, about: &'static str, spec: Spec) -> Probe {
    Probe { name: name.into(), about, spec }
}

fn base(font_name: &str, half_points: u32, ppr: &str, body: String) -> Spec {
    Spec { body, sect: A4.into(), rpr: font(font_name, half_points), ppr: ppr.into(), ..Default::default() }
}

const SINGLE: &str = r#"<w:spacing w:after="0" w:line="240" w:lineRule="auto"/>"#;

pub fn all() -> Vec<Probe> {
    let mut v = Vec::new();
    fonts(&mut v);
    spacing(&mut v);
    alignment(&mut v);
    raised(&mut v);
    pagination(&mut v);
    tables(&mut v);
    pictures(&mut v);
    notes_lists_headers(&mut v);
    v
}

fn fonts(v: &mut Vec<Probe>) {
    // System fonts on macOS (both sides use the same files) and Office fonts (WordCraft uses
    // metric-compatible substitutes or its fallback).
    for (name, hp, ppr) in [
        ("Times New Roman", 24, SINGLE),
        ("Arial", 22, SINGLE),
        ("Georgia", 22, SINGLE),
        ("Verdana", 20, SINGLE),
        ("Calibri", 22, r#"<w:spacing w:after="160" w:line="259" w:lineRule="auto"/>"#),
        ("Cambria", 22, SINGLE),
        ("Aptos", 24, r#"<w:spacing w:after="160" w:line="278" w:lineRule="auto"/>"#),
    ] {
        let mut w = Words::new(1);
        let key = name.split(' ').next().unwrap_or(name).to_lowercase();
        v.push(probe(
            format!("font-{key}-{}", hp / 2),
            "line breaks, line height and page breaks of plain paragraphs in one font",
            base(name, hp, ppr, paras(&mut w, 14, 110, "")),
        ));
    }
}

fn spacing(v: &mut Vec<Probe>) {
    for (key, sp) in [
        ("single", r#"<w:spacing w:after="0" w:line="240" w:lineRule="auto"/>"#),
        ("1.15", r#"<w:spacing w:after="0" w:line="276" w:lineRule="auto"/>"#),
        ("1.5", r#"<w:spacing w:after="0" w:line="360" w:lineRule="auto"/>"#),
        ("double", r#"<w:spacing w:after="0" w:line="480" w:lineRule="auto"/>"#),
        ("exact-13", r#"<w:spacing w:after="0" w:line="260" w:lineRule="exact"/>"#),
        ("exact-20", r#"<w:spacing w:after="0" w:line="400" w:lineRule="exact"/>"#),
        ("atleast-10", r#"<w:spacing w:after="0" w:line="200" w:lineRule="atLeast"/>"#),
        ("atleast-20", r#"<w:spacing w:after="0" w:line="400" w:lineRule="atLeast"/>"#),
        ("before-after", r#"<w:spacing w:before="240" w:after="120" w:line="240" w:lineRule="auto"/>"#),
        ("contextual", r#"<w:spacing w:before="240" w:after="120" w:line="240" w:lineRule="auto"/><w:contextualSpacing/>"#),
    ] {
        let mut w = Words::new(2);
        v.push(probe(
            format!("spacing-{key}"),
            "line spacing and paragraph spacing (Georgia 11)",
            base("Georgia", 22, sp, paras(&mut w, 12, 90, "")),
        ));
    }
}

fn alignment(v: &mut Vec<Probe>) {
    for (key, ppr) in [
        ("justify", r#"<w:jc w:val="both"/>"#),
        ("center", r#"<w:jc w:val="center"/>"#),
        ("right", r#"<w:jc w:val="right"/>"#),
        ("first-line", r#"<w:ind w:firstLine="720"/>"#),
        ("hanging", r#"<w:ind w:left="720" w:hanging="720"/>"#),
        ("indents", r#"<w:ind w:left="1440" w:right="1440"/><w:jc w:val="both"/>"#),
    ] {
        let mut w = Words::new(3);
        let body = paras(&mut w, 8, 120, ppr);
        v.push(probe(format!("align-{key}"), "alignment and indents (Georgia 11)", base("Georgia", 22, SINGLE, body)));
    }
    let mut w = Words::new(4);
    let tabs = r#"<w:tabs><w:tab w:val="left" w:pos="2160"/><w:tab w:val="center" w:pos="4680"/><w:tab w:val="right" w:leader="dot" w:pos="7920"/><w:tab w:val="decimal" w:pos="9000"/></w:tabs>"#;
    let body: String = (0..30)
        .map(|i| {
            let a = w.word();
            let b = w.word();
            let c = w.text(3);
            p(tabs, &r("", &format!("{a}\t{b}\t{c}\t{}.{}\t{i}", i * 37 % 1000, i * 7 % 100)))
        })
        .collect();
    v.push(probe("tabs", "tab stops: left, centre, right with dot leader, decimal (Georgia 11)", base("Georgia", 22, SINGLE, body)));
    let mut w = Words::new(5);
    let body: String = (0..20).map(|_| p("", &r("", &format!("{}\t{}\t{}", w.word(), w.text(2), w.word())))).collect();
    v.push(probe("tabs-default", "default tab stops every half inch (Georgia 11)", base("Georgia", 22, SINGLE, body)));
}

fn raised(v: &mut Vec<Probe>) {
    for (key, fname, hp) in [("arial", "Arial", 22), ("calibri", "Calibri", 22)] {
        let mut w = Words::new(6);
        let sup = r#"<w:vertAlign w:val="superscript"/>"#;
        let body: String = (0..10)
            .map(|i| {
                let mut runs = String::new();
                for k in 0..12 {
                    runs.push_str(&r("", &format!("{} ", w.text(6))));
                    runs.push_str(&r(sup, &format!("{}", (i * 12 + k) % 40 + 1)));
                    runs.push_str(&r("", " "));
                }
                p("", &runs)
            })
            .collect();
        v.push(probe(format!("superscript-{key}"), "superscript citation numbers inside lines", base(fname, hp, SINGLE, body)));
        // An author line as on a title page: names with small superscript affiliations.
        let mut w = Words::new(7);
        let small = format!("{sup}<w:sz w:val=\"14\"/><w:szCs w:val=\"14\"/>");
        let mut runs = String::new();
        for k in 0..14 {
            runs.push_str(&r("", &format!("{} {}. {}", capital(&w.word()), "M", capital(&w.word()))));
            runs.push_str(&r(&small, &format!("{},{}", k % 5 + 1, k % 3 + 6)));
            runs.push_str(&r("", if k == 13 { "" } else { ", " }));
        }
        let body = format!("{}{}", p(r#"<w:jc w:val="center"/>"#, &runs), paras(&mut w, 6, 120, ""));
        v.push(probe(format!("authors-{key}"), "a centred author line with small superscript affiliations", base(fname, hp, SINGLE, body)));
    }
    let mut w = Words::new(8);
    let body: String = (0..10)
        .map(|_| {
            format!(
                "<w:p>{}{}{}{}{}</w:p>",
                r("", &format!("{} ", w.text(20))),
                r(r#"<w:vertAlign w:val="subscript"/>"#, "2"),
                r("", &format!(" {} ", w.text(10))),
                r(r#"<w:sz w:val="36"/><w:szCs w:val="36"/>"#, &w.word()),
                r("", &format!(" {}", w.text(30)))
            )
        })
        .collect();
    v.push(probe("mixed-sizes", "subscripts and an 18 pt word inside 11 pt lines (Georgia)", base("Georgia", 22, SINGLE, body)));
    let mut w = Words::new(9);
    let body: String = (0..10)
        .map(|_| {
            format!(
                "<w:p>{}{}{}</w:p>",
                r("", &format!("{} ", w.text(20))),
                r(r#"<w:position w:val="6"/>"#, &w.word()),
                r("", &format!(" {}", w.text(25)))
            )
        })
        .collect();
    v.push(probe("raised-position", "text raised 3 pt with w:position (Georgia 11)", base("Georgia", 22, SINGLE, body)));
}

fn heading_style() -> String {
    r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="360" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>"#.into()
}

fn pagination(v: &mut Vec<Probe>) {
    // Paragraph lengths chosen so page ends fall inside paragraphs at different places.
    for (key, ppr) in [("widow-on", ""), ("widow-off", r#"<w:widowControl w:val="0"/>"#), ("keep-lines", "<w:keepLines/>")] {
        let mut w = Words::new(10);
        let body: String =
            [150usize, 95, 210, 60, 180, 140, 75, 230, 120, 90, 160, 200, 50, 170].iter().map(|&n| p(ppr, &r("", &w.text(n)))).collect();
        v.push(probe(
            format!("pagination-{key}"),
            "page breaks inside paragraphs: widow/orphan control and keep lines together (Georgia 11)",
            base("Georgia", 22, SINGLE, body),
        ));
    }
    let mut w = Words::new(11);
    let mut body = String::new();
    for (i, n) in [180usize, 240, 120, 300, 90, 260, 150, 200].iter().enumerate() {
        body.push_str(&p(r#"<w:pStyle w:val="Heading1"/>"#, &r("", &format!("{} {}", i + 1, capital(&w.text(4))))));
        body.push_str(&p("", &r("", &w.text(*n))));
    }
    let mut s = base("Georgia", 22, r#"<w:spacing w:after="120" w:line="240" w:lineRule="auto"/>"#, body);
    s.styles = heading_style();
    v.push(probe("pagination-headings", "headings kept with the next paragraph across page ends (Georgia 11)", s));
    let mut w = Words::new(12);
    let mut body = paras(&mut w, 2, 100, "");
    body.push_str(&p("<w:pageBreakBefore/>", &r("", &w.text(80))));
    body.push_str(&format!("<w:p>{}<w:r><w:br w:type=\"page\"/></w:r>{}</w:p>", r("", &w.text(40)), r("", &w.text(40))));
    body.push_str(&paras(&mut w, 2, 100, ""));
    v.push(probe(
        "pagination-breaks",
        "page break before and a manual page break inside a paragraph (Georgia 11)",
        base("Georgia", 22, r#"<w:spacing w:after="160" w:line="240" w:lineRule="auto"/>"#, body),
    ));
}

fn cell(width: u32, text: &str) -> String {
    format!(r#"<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/></w:tcPr>{}</w:tc>"#, p("", &r("", text)))
}

fn table(rows: &[String], widths: &[u32], header: bool) -> String {
    let grid: String = widths.iter().map(|w| format!(r#"<w:gridCol w:w="{w}"/>"#)).collect();
    let borders = r#"<w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:left w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:right w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="000000"/></w:tblBorders>"#;
    let total: u32 = widths.iter().sum();
    let mut out = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="{total}" w:type="dxa"/>{borders}<w:tblLayout w:type="fixed"/><w:tblLook w:val="04A0"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>"#
    );
    for (i, row) in rows.iter().enumerate() {
        let trpr = if header && i == 0 { "<w:trPr><w:tblHeader/></w:trPr>" } else { "" };
        out.push_str(&format!("<w:tr>{trpr}{row}</w:tr>"));
    }
    out.push_str("</w:tbl>");
    out
}

fn tables(v: &mut Vec<Probe>) {
    let widths = [2000u32, 3000, 4026];
    let mut w = Words::new(13);
    let rows: Vec<String> = (0..12).map(|_| widths.iter().map(|&cw| cell(cw, &w.text((cw / 200) as usize))).collect()).collect();
    let body = format!("{}{}{}", paras(&mut w, 1, 60, ""), table(&rows, &widths, false), paras(&mut w, 2, 80, ""));
    v.push(probe(
        "table-grid",
        "a bordered fixed-width table: cell margins, wrapping in cells, row heights (Georgia 11)",
        base("Georgia", 22, SINGLE, body),
    ));
    let mut w = Words::new(14);
    let mut rows: Vec<String> = vec![widths.iter().map(|&cw| cell(cw, &capital(&w.word()))).collect()];
    rows.extend((0..70).map(|i| widths.iter().map(|&cw| cell(cw, &if i % 7 == 3 { w.text(14) } else { w.text(3) })).collect::<String>()));
    let body = format!("{}{}", table(&rows, &widths, true), paras(&mut w, 1, 40, ""));
    v.push(probe("table-long", "a table across pages with a repeated header row (Georgia 11)", base("Georgia", 22, SINGLE, body)));
}

fn pictures(v: &mut Vec<Probe>) {
    let pics = vec![(40, 24, [70, 120, 200]), (20, 20, [200, 80, 60])];
    for (key, ppr) in [
        ("single", ""),
        ("1.5", r#"<w:spacing w:after="0" w:line="360" w:lineRule="auto"/>"#),
        ("atleast-12", r#"<w:spacing w:after="0" w:line="240" w:lineRule="atLeast"/>"#),
        ("atleast-100", r#"<w:spacing w:after="0" w:line="2000" w:lineRule="atLeast"/>"#),
        ("exact-30", r#"<w:spacing w:after="0" w:line="600" w:lineRule="exact"/>"#),
    ] {
        let mut w = Words::new(15);
        let mut body = String::new();
        for k in 0..4 {
            body.push_str(&p("", &r("", &w.text(40))));
            // A picture alone, then a picture with text, then a small one inside a line.
            body.push_str(&p(ppr, &inline_picture(1, 200.0 + 20.0 * k as f64, 80.0 + 10.0 * k as f64)));
            body.push_str(&p(ppr, &format!("{}{}", inline_picture(1, 120.0, 50.0), r("", &format!(" {}", w.text(12))))));
            body.push_str(&p(
                ppr,
                &format!("{}{}{}", r("", &format!("{} ", w.text(8))), inline_picture(2, 14.0, 14.0), r("", &format!(" {}", w.text(30)))),
            ));
        }
        let mut s = base("Georgia", 22, SINGLE, body);
        s.pictures = pics.clone();
        v.push(probe(format!("picture-{key}"), "inline pictures alone, with text and inside a line, under each line-spacing rule (Georgia 11)", s));
    }
}

fn notes_lists_headers(v: &mut Vec<Probe>) {
    let mut w = Words::new(16);
    let mut body = String::new();
    let mut notes = String::new();
    for id in 1..=12 {
        let ref_run = format!(r#"<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:id="{id}"/></w:r>"#);
        body.push_str(&format!("<w:p>{}{ref_run}{}</w:p>", r("", &format!("{} ", w.text(70))), r("", &format!(" {}", w.text(60)))));
        let len = if id % 5 == 0 { 160 } else { 15 + id * 3 };
        notes.push_str(&format!(
            r#"<w:footnote w:id="{id}"><w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteRef/></w:r>{}</w:p></w:footnote>"#,
            r(r#"<w:sz w:val="18"/><w:szCs w:val="18"/>"#, &format!(" {}", w.text(len as usize)))
        ));
    }
    let mut s = base("Georgia", 22, SINGLE, body);
    s.footnotes = notes;
    v.push(probe("footnotes", "footnotes at page bottoms, a long one continuing on the next page (Georgia 11, notes 9)", s));

    let mut w = Words::new(17);
    let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="multilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/></w:rPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>"#;
    let mut body = String::new();
    for i in 0..30 {
        let (num, lvl) = match i % 6 {
            0..=2 => (1, 0),
            3 => (1, 1),
            _ => (2, 0),
        };
        body.push_str(&p(&format!(r#"<w:numPr><w:ilvl w:val="{lvl}"/><w:numId w:val="{num}"/></w:numPr>"#), &r("", &w.text(12 + (i * 7) % 40))));
    }
    let mut s = base("Georgia", 22, r#"<w:spacing w:after="80" w:line="240" w:lineRule="auto"/>"#, body);
    s.numbering = numbering.into();
    v.push(probe("lists", "numbered, lettered and bulleted lists with hanging indents (Georgia 11)", s));

    let mut w = Words::new(18);
    let header: String = (0..7).map(|_| p(SINGLE, &r("", &w.text(9)))).collect();
    let mut s = base("Georgia", 22, SINGLE, paras(&mut w, 10, 110, ""));
    s.headers = vec![header];
    s.sect = format!(r#"<w:headerReference w:type="default" r:id="rIdH1"/>{A4}"#);
    v.push(probe("header-tall", "a header taller than the top margin pushes the body down (Georgia 11)", s));

    let mut w = Words::new(19);
    let cols = format!(r#"<w:sectPr>{A4}<w:cols w:num="2" w:space="720"/></w:sectPr>"#);
    let mut body = paras(&mut w, 5, 90, "");
    body.push_str(&format!("<w:p><w:pPr>{cols}</w:pPr>{}</w:p>", r("", &w.text(50))));
    body.push_str(&paras(&mut w, 2, 60, ""));
    let mut s = base("Georgia", 22, SINGLE, body);
    s.sect = format!(r#"<w:type w:val="continuous"/>{A4}"#);
    v.push(probe("columns-balanced", "two columns balanced before a continuous section break (Georgia 11)", s));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_deterministic_and_probe_names_unique() {
        assert_eq!(Words::new(3).text(20), Words::new(3).text(20));
        assert_ne!(Words::new(3).text(20), Words::new(4).text(20));
        let all = all();
        let mut names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
    }
}
