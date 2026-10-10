//! OpenDocument Text (`.odt`, ODF 1.3).
//!
//! Export writes `mimetype` (stored, first), `META-INF/manifest.xml`, `content.xml` (paragraphs,
//! `text:h` headings with outline levels, spans with automatic text styles, links, bookmarks,
//! line breaks / tabs / `text:s`, nested `text:list`s with generated list styles, tables with
//! header rows, column/row spans and cell shading, pictures as `draw:frame`/`draw:image` with the
//! bytes under `Pictures/`, page breaks), `styles.xml` (Standard, Heading 1–6, Title,
//! Quotations, Preformatted Text, page layout) and `meta.xml`.
//!
//! Import reads the same subset back from `content.xml` / `styles.xml` (style inheritance
//! resolved, both automatic and common styles), skipping notes, annotations, text boxes and
//! tracked-change records.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use quick_xml::events::{BytesStart, Event};
use wordcraft_doc::{Align, Document, Rgb};

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, make_img, mime_of};

/// Largest zip entry we inflate.
const MAX_ENTRY: u64 = 256 << 20;
/// Deepest element nesting handled in content.xml.
const MAX_XML_DEPTH: usize = 512;

// ---------------------------------------------------------------------------------------------
// Export

fn x(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if c.is_control() && !matches!(c, '\t' | '\n') => {}
            _ => o.push(c),
        }
    }
    o
}

const NS: &str = concat!(
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
    "office:version=\"1.3\""
);

fn pt(v: f32) -> String {
    let s = format!("{:.2}", v);
    format!("{}pt", s.trim_end_matches('0').trim_end_matches('.'))
}

#[derive(Default)]
struct Writer {
    /// Automatic text styles: properties XML → name.
    text_styles: Vec<(String, String)>,
    /// Automatic paragraph styles: (parent, properties XML) → name.
    para_styles: Vec<((String, String), String)>,
    /// List styles: level kinds.
    list_styles: Vec<[bool; 9]>,
    /// Table styles: (name, column widths).
    tables: Vec<Vec<f32>>,
    /// Cell styles by shading.
    cell_styles: Vec<Option<Rgb>>,
    pictures: Vec<(String, std::sync::Arc<Vec<u8>>, String)>,
}

fn text_props(f: &Fmt) -> String {
    let mut p = String::new();
    if f.bold {
        p.push_str(" fo:font-weight=\"bold\" style:font-weight-asian=\"bold\" style:font-weight-complex=\"bold\"");
    }
    if f.italic {
        p.push_str(" fo:font-style=\"italic\" style:font-style-asian=\"italic\" style:font-style-complex=\"italic\"");
    }
    if f.underline {
        p.push_str(" style:text-underline-style=\"solid\" style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\"");
    }
    if f.strike {
        p.push_str(" style:text-line-through-style=\"solid\"");
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
    fn text_style(&mut self, f: &Fmt) -> Option<String> {
        let props = text_props(f);
        if props.is_empty() {
            return None;
        }
        if let Some((_, n)) = self.text_styles.iter().find(|(p, _)| *p == props) {
            return Some(n.clone());
        }
        let n = format!("T{}", self.text_styles.len() + 1);
        self.text_styles.push((props, n.clone()));
        Some(n)
    }

    fn para_style(&mut self, p: &Para) -> String {
        let parent = parent_style(p.kind).to_string();
        let mut props = String::new();
        match p.align {
            Some(Align::Center) => props.push_str(" fo:text-align=\"center\""),
            Some(Align::Right) => props.push_str(" fo:text-align=\"end\""),
            Some(Align::Justify) | Some(Align::Distribute) => props.push_str(" fo:text-align=\"justify\""),
            Some(Align::Left) => props.push_str(" fo:text-align=\"start\""),
            None => {}
        }
        if p.page_break {
            props.push_str(" fo:break-before=\"page\"");
        }
        if p.kind == Kind::Rule {
            props.push_str(" fo:border-bottom=\"0.75pt solid #a0a0a0\" fo:padding-bottom=\"1pt\"");
        }
        if props.is_empty() {
            return parent;
        }
        let key = (parent, props);
        if let Some((_, n)) = self.para_styles.iter().find(|(k, _)| *k == key) {
            return n.clone();
        }
        let n = format!("P{}", self.para_styles.len() + 1);
        self.para_styles.push((key, n.clone()));
        n
    }

    fn picture(&mut self, img: &model::Img) -> String {
        let ext = if img.ext == "jpeg" { "jpg".to_string() } else { img.ext.clone() };
        let name = format!("Pictures/image{}.{}", self.pictures.len() + 1, ext);
        self.pictures.push((name.clone(), img.data.clone(), mime_of(&img.ext).to_string()));
        name
    }

    fn inlines(&mut self, inl: &[Inline], out: &mut String) {
        let mut prev_space = true;
        for i in inl {
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
                            '\t' => body.push_str("<text:tab/>"),
                            '\n' => {
                                body.push_str("<text:line-break/>");
                                prev_space = true;
                            }
                            '\u{000C}' | '\u{000E}' => {}
                            c => body.push_str(&x(&c.to_string())),
                        }
                    }
                    if spaces > 0 {
                        flush(&mut body, &mut spaces, prev_space);
                        prev_space = true;
                    }
                    let link = f.link.as_ref();
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
                    out.push_str(&format!(
                        "<draw:frame draw:name=\"Image{n}\" text:anchor-type=\"as-char\" svg:width=\"{}\" svg:height=\"{}\" draw:z-index=\"0\"><draw:image xlink:href=\"{href}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>",
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
            }
        }
    }

    fn para(&mut self, p: &Para, out: &mut String) {
        let style = self.para_style(p);
        match p.kind {
            Kind::Heading(n) => {
                out.push_str(&format!("<text:h text:style-name=\"{style}\" text:outline-level=\"{}\">", n.clamp(1, 6)));
                self.inlines(&p.inlines, out);
                out.push_str("</text:h>");
            }
            _ => {
                out.push_str(&format!("<text:p text:style-name=\"{style}\">"));
                self.inlines(&p.inlines, out);
                out.push_str("</text:p>");
            }
        }
    }

    fn blocks(&mut self, blocks: &[FBlock], out: &mut String, depth: usize) {
        let mut i = 0;
        while let Some(b) = blocks.get(i) {
            match b {
                FBlock::Table(t) => {
                    if depth < model::MAX_DEPTH {
                        self.table(t, out, depth);
                    }
                    i += 1;
                }
                FBlock::Para(p) if p.list.is_some() => {
                    let start = i;
                    let mut kinds = [p.list.map(|l| l.ordered).unwrap_or(false); 9];
                    let mut seen = [false; 9];
                    while let Some(FBlock::Para(q)) = blocks.get(i) {
                        let Some(li) = q.list else { break };
                        let l = li.level.min(8) as usize;
                        if let (Some(s), Some(k)) = (seen.get_mut(l), kinds.get_mut(l))
                            && !*s
                        {
                            *s = true;
                            *k = li.ordered;
                        }
                        i += 1;
                    }
                    self.list_styles.push(kinds);
                    let lname = format!("L{}", self.list_styles.len());
                    // Open lists: per level, whether a list-item is open.
                    let mut stack: Vec<bool> = Vec::new();
                    for q in blocks.get(start..i).unwrap_or(&[]) {
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
                                out.push_str(&format!("<text:list text:style-name=\"{lname}\">"));
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
                        self.para(q, out);
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
                    self.para(p, out);
                    out.push('\n');
                    i += 1;
                }
            }
        }
    }

    fn cell_style(&mut self, shading: Option<Rgb>) -> String {
        let k = match self.cell_styles.iter().position(|s| *s == shading) {
            Some(k) => k,
            None => {
                self.cell_styles.push(shading);
                self.cell_styles.len() - 1
            }
        };
        format!("Cell{}", k + 1)
    }

    fn table(&mut self, t: &FTable, out: &mut String, depth: usize) {
        let cols = t.cols().max(1);
        let widths: Vec<f32> = if t.widths.len() == cols && t.widths.iter().all(|w| w.is_finite() && *w > 0.0) {
            t.widths.clone()
        } else {
            vec![468.0 / cols as f32; cols]
        };
        self.tables.push(widths);
        let tn = self.tables.len();
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
                let st = self.cell_style(c.shading);
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

    fn automatic_styles(&self) -> String {
        let mut s = String::from("<office:automatic-styles>");
        for (props, n) in &self.text_styles {
            s.push_str(&format!("<style:style style:name=\"{n}\" style:family=\"text\"><style:text-properties{props}/></style:style>"));
        }
        for ((parent, props), n) in &self.para_styles {
            s.push_str(&format!(
                "<style:style style:name=\"{n}\" style:family=\"paragraph\" style:parent-style-name=\"{parent}\"><style:paragraph-properties{props}/></style:style>"
            ));
        }
        for (i, kinds) in self.list_styles.iter().enumerate() {
            s.push_str(&format!("<text:list-style style:name=\"L{}\">", i + 1));
            for (l, ordered) in kinds.iter().enumerate() {
                let lv = l + 1;
                let props = format!(
                    "<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"><style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"{}\" fo:text-indent=\"-18pt\" fo:margin-left=\"{}\"/></style:list-level-properties>",
                    pt(36.0 * lv as f32),
                    pt(36.0 * lv as f32)
                );
                if *ordered {
                    let fmt = ["1", "a", "i"].get(l % 3).copied().unwrap_or("1");
                    s.push_str(&format!("<text:list-level-style-number text:level=\"{lv}\" style:num-suffix=\".\" style:num-format=\"{fmt}\">{props}</text:list-level-style-number>"));
                } else {
                    let ch = ["•", "◦", "▪"].get(l % 3).copied().unwrap_or("•");
                    s.push_str(&format!(
                        "<text:list-level-style-bullet text:level=\"{lv}\" text:bullet-char=\"{ch}\">{props}</text:list-level-style-bullet>"
                    ));
                }
            }
            s.push_str("</text:list-style>");
        }
        for (i, widths) in self.tables.iter().enumerate() {
            let tn = i + 1;
            let total: f32 = widths.iter().sum();
            s.push_str(&format!(
                "<style:style style:name=\"Table{tn}\" style:family=\"table\"><style:table-properties style:width=\"{}\" table:align=\"margins\"/></style:style>",
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
        for (i, sh) in self.cell_styles.iter().enumerate() {
            let bg = sh.map(|c| format!(" fo:background-color=\"#{}\"", c.hex().to_ascii_lowercase())).unwrap_or_default();
            s.push_str(&format!(
                "<style:style style:name=\"Cell{}\" style:family=\"table-cell\"><style:table-cell-properties fo:padding=\"4pt\" fo:border=\"0.5pt solid #000000\"{bg}/></style:style>",
                i + 1
            ));
        }
        s.push_str("</office:automatic-styles>");
        s
    }
}

fn styles_xml(sec: &wordcraft_doc::SectionProps, body_font: &str) -> String {
    let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NS}><office:styles>");
    s.push_str(&format!(
        "<style:default-style style:family=\"paragraph\"><style:paragraph-properties fo:margin-bottom=\"8pt\"/><style:text-properties fo:font-family=\"'{}'\" fo:font-size=\"12pt\"/></style:default-style>",
        x(body_font)
    ));
    s.push_str("<style:style style:name=\"Standard\" style:family=\"paragraph\" style:class=\"text\"/>");
    s.push_str("<style:style style:name=\"Heading\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"text\"><style:paragraph-properties fo:margin-top=\"12pt\" fo:margin-bottom=\"4pt\" fo:keep-with-next=\"always\"/><style:text-properties fo:color=\"#0f4761\"/></style:style>");
    let sizes = [20.0, 16.0, 14.0, 12.0, 12.0, 11.0];
    for (i, sz) in sizes.iter().enumerate() {
        let n = i + 1;
        s.push_str(&format!(
            "<style:style style:name=\"Heading_20_{n}\" style:display-name=\"Heading {n}\" style:family=\"paragraph\" style:parent-style-name=\"Heading\" style:next-style-name=\"Standard\" style:default-outline-level=\"{n}\" style:class=\"text\"><style:text-properties fo:font-size=\"{}\"/></style:style>",
            pt(*sz)
        ));
    }
    s.push_str("<style:style style:name=\"Title\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"chapter\"><style:text-properties fo:font-size=\"28pt\"/></style:style>");
    s.push_str("<style:style style:name=\"Quotations\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"html\"><style:paragraph-properties fo:margin-left=\"36pt\" fo:margin-right=\"36pt\"/><style:text-properties fo:font-style=\"italic\" fo:color=\"#404040\"/></style:style>");
    s.push_str(&format!("<style:style style:name=\"Preformatted_20_Text\" style:display-name=\"Preformatted Text\" style:family=\"paragraph\" style:parent-style-name=\"Standard\" style:class=\"html\"><style:paragraph-properties fo:margin-top=\"0pt\" fo:margin-bottom=\"0pt\"/><style:text-properties fo:font-family=\"'{}'\" style:font-family-generic=\"modern\" style:font-pitch=\"fixed\" fo:font-size=\"10pt\"/></style:style>", model::MONO_FONT));
    s.push_str("</office:styles><office:automatic-styles>");
    s.push_str(&format!(
        "<style:page-layout style:name=\"pm1\"><style:page-layout-properties fo:page-width=\"{}\" fo:page-height=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\" fo:margin-left=\"{}\" fo:margin-right=\"{}\" style:print-orientation=\"{}\"/></style:page-layout>",
        pt(sec.page_w),
        pt(sec.page_h),
        pt(sec.margin_top),
        pt(sec.margin_bottom),
        pt(sec.margin_left),
        pt(sec.margin_right),
        if sec.page_w > sec.page_h { "landscape" } else { "portrait" }
    ));
    s.push_str("</office:automatic-styles><office:master-styles><style:master-page style:name=\"Standard\" style:page-layout-name=\"pm1\"/></office:master-styles></office:document-styles>\n");
    s
}

fn meta_xml(m: &Meta) -> String {
    let mut s =
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta {NS}><office:meta><meta:generator>WordCraft</meta:generator>");
    if !m.title.is_empty() {
        s.push_str(&format!("<dc:title>{}</dc:title>", x(&m.title)));
    }
    if !m.subject.is_empty() {
        s.push_str(&format!("<dc:subject>{}</dc:subject>", x(&m.subject)));
    }
    if !m.description.is_empty() {
        s.push_str(&format!("<dc:description>{}</dc:description>", x(&m.description)));
    }
    if !m.author.is_empty() {
        s.push_str(&format!("<meta:initial-creator>{}</meta:initial-creator><dc:creator>{}</dc:creator>", x(&m.author), x(&m.author)));
    }
    for k in m.keywords.split([',', ';']).map(str::trim).filter(|k| !k.is_empty()) {
        s.push_str(&format!("<meta:keyword>{}</meta:keyword>", x(k)));
    }
    if !m.created.is_empty() {
        s.push_str(&format!("<meta:creation-date>{}</meta:creation-date>", x(m.created.trim_end_matches('Z'))));
    }
    s.push_str("</office:meta></office:document-meta>\n");
    s
}

/// Write an `.odt` package.
pub fn export(doc: &Document) -> Result<Vec<u8>, String> {
    let flow = model::from_doc(doc);
    let mut w = Writer::default();
    let mut body = String::new();
    w.blocks(&flow.blocks, &mut body, 0);
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NS}>{}<office:body><office:text>\n{body}</office:text></office:body></office:document-content>\n",
        w.automatic_styles()
    );
    let body_font = doc.styles.default_chr.font.clone().unwrap_or_else(|| wordcraft_doc::styles::BODY_FONT.to_string());
    let styles = styles_xml(&doc.last_section, &body_font);
    let meta = meta_xml(&flow.meta);
    let mut manifest = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">\n<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\n<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>\n<manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>\n",
    );
    for (name, _, mime) in &w.pictures {
        manifest.push_str(&format!("<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{mime}\"/>\n", x(name)));
    }
    manifest.push_str("</manifest:manifest>\n");

    let err = |e: zip::result::ZipError| format!("ODT: {e}");
    let ioerr = |e: std::io::Error| format!("ODT: {e}");
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("mimetype", stored).map_err(err)?;
    z.write_all(b"application/vnd.oasis.opendocument.text").map_err(ioerr)?;
    for (name, data) in [
        ("META-INF/manifest.xml", manifest.as_bytes()),
        ("content.xml", content.as_bytes()),
        ("styles.xml", styles.as_bytes()),
        ("meta.xml", meta.as_bytes()),
    ] {
        z.start_file(name, deflated).map_err(err)?;
        z.write_all(data).map_err(ioerr)?;
    }
    for (name, data, _) in &w.pictures {
        z.start_file(name.as_str(), stored).map_err(err)?;
        z.write_all(data).map_err(ioerr)?;
    }
    let cur = z.finish().map_err(err)?;
    Ok(cur.into_inner())
}

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
    font: Option<String>,
    code: Option<bool>,
}

impl FmtDelta {
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
        if let Some(fam) = &self.font {
            f.font = Some(fam.clone());
        }
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
    outline: Option<u8>,
    border_bottom: bool,
}

/// Length → points.
fn length(v: &str) -> Option<f32> {
    let v = v.trim();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|x| x.is_finite());
    if let Some(n) = v.strip_suffix("pt") {
        num(n)
    } else if let Some(n) = v.strip_suffix("in") {
        num(n).map(|x| x * 72.0)
    } else if let Some(n) = v.strip_suffix("cm") {
        num(n).map(|x| x * 72.0 / 2.54)
    } else if let Some(n) = v.strip_suffix("mm") {
        num(n).map(|x| x * 72.0 / 25.4)
    } else if let Some(n) = v.strip_suffix("pc") {
        num(n).map(|x| x * 12.0)
    } else if let Some(n) = v.strip_suffix("px") {
        num(n).map(|x| x * 0.75)
    } else {
        num(v)
    }
}

type Attrs = Vec<(String, String)>;

fn attrs_of(e: &BytesStart<'_>) -> Attrs {
    let mut v = Vec::new();
    for a in e.attributes().with_checks(false).flatten().take(128) {
        let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let val = a.unescape_value().map(|c| c.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned());
        v.push((k, val));
    }
    v
}

fn get<'a>(a: &'a Attrs, k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn local(q: &str) -> &str {
    q.rsplit(':').next().unwrap_or(q)
}

enum Ev {
    Start(String, Attrs),
    End(String),
    Text(String),
}

/// Stream XML events (empty elements become start + end). Malformed XML ends the stream.
fn events(xml: &[u8], mut f: impl FnMut(Ev)) {
    let mut r = quick_xml::Reader::from_reader(xml);
    let cfg = r.config_mut();
    cfg.trim_text(false);
    cfg.check_end_names = false;
    cfg.allow_unmatched_ends = true;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => f(Ev::Start(String::from_utf8_lossy(e.name().as_ref()).into_owned(), attrs_of(&e))),
            Ok(Event::Empty(e)) => {
                let n = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                f(Ev::Start(n.clone(), attrs_of(&e)));
                f(Ev::End(n));
            }
            Ok(Event::End(e)) => f(Ev::End(String::from_utf8_lossy(e.name().as_ref()).into_owned())),
            Ok(Event::Text(t)) => f(Ev::Text(t.decode().map(|c| c.into_owned()).unwrap_or_default())),
            Ok(Event::CData(t)) => f(Ev::Text(String::from_utf8_lossy(&t).into_owned())),
            Ok(Event::GeneralRef(r)) => {
                let name = String::from_utf8_lossy(&r).into_owned();
                let s = match name.as_str() {
                    "amp" => "&".to_string(),
                    "lt" => "<".to_string(),
                    "gt" => ">".to_string(),
                    "quot" => "\"".to_string(),
                    "apos" => "'".to_string(),
                    _ => r.resolve_char_ref().ok().flatten().map(|c| c.to_string()).unwrap_or_default(),
                };
                f(Ev::Text(s));
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

#[derive(Default)]
struct Styles {
    map: HashMap<String, StyleInfo>,
    lists: HashMap<String, Vec<bool>>,
}

impl Styles {
    fn read(&mut self, xml: &[u8], auto_section_is_auto: bool) {
        let mut in_auto = false;
        let mut cur: Option<(String, StyleInfo)> = None;
        let mut list: Option<(String, Vec<bool>)> = None;
        events(xml, |ev| match ev {
            Ev::Start(q, a) => match local(&q) {
                "automatic-styles" => in_auto = true,
                "style" if q.starts_with("style:") => {
                    if let Some(name) = get(&a, "style:name") {
                        let info = StyleInfo {
                            parent: get(&a, "style:parent-style-name").map(String::from),
                            display: get(&a, "style:display-name").map(String::from),
                            auto: in_auto && auto_section_is_auto,
                            outline: get(&a, "style:default-outline-level").and_then(|v| v.parse::<u8>().ok()),
                            ..Default::default()
                        };
                        cur = Some((name.to_string(), info));
                    }
                }
                "text-properties" => {
                    if let Some((_, s)) = &mut cur {
                        let d = &mut s.fmt;
                        if let Some(w) = get(&a, "fo:font-weight") {
                            d.bold = Some(w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600));
                        }
                        if let Some(v) = get(&a, "fo:font-style") {
                            d.italic = Some(v == "italic" || v == "oblique");
                        }
                        if let Some(v) = get(&a, "style:text-underline-style") {
                            d.underline = Some(v != "none");
                        }
                        if let Some(v) = get(&a, "style:text-line-through-style") {
                            d.strike = Some(v != "none");
                        }
                        if let Some(v) = get(&a, "style:text-position") {
                            let v = v.trim();
                            let first = v.split_whitespace().next().unwrap_or("");
                            let pct = first.trim_end_matches('%').parse::<f32>().unwrap_or(0.0);
                            d.sup = Some(first == "super" || pct > 0.0);
                            d.sub = Some(first == "sub" || pct < 0.0);
                        }
                        if let Some(c) = get(&a, "fo:color").and_then(model::parse_color) {
                            d.color = Some(c);
                        }
                        if let Some(c) = get(&a, "fo:background-color").and_then(model::parse_color) {
                            d.background = Some(c);
                        }
                        if let Some(sz) = get(&a, "fo:font-size").and_then(length).filter(|v| *v >= 1.0 && *v <= 1638.0) {
                            d.size = Some(sz);
                        }
                        if let Some(fam) = get(&a, "fo:font-family").or_else(|| get(&a, "style:font-name")) {
                            let fam = fam.split(',').next().unwrap_or("").trim().trim_matches(['\'', '"']).to_string();
                            if model::is_mono(&fam) || get(&a, "style:font-pitch") == Some("fixed") {
                                d.code = Some(true);
                            } else if !fam.is_empty() {
                                d.font = Some(fam);
                            }
                        }
                    }
                }
                "paragraph-properties" => {
                    if let Some((_, s)) = &mut cur {
                        s.align = match get(&a, "fo:text-align") {
                            Some("center") => Some(Align::Center),
                            Some("end") | Some("right") => Some(Align::Right),
                            Some("justify") => Some(Align::Justify),
                            Some("start") | Some("left") => Some(Align::Left),
                            _ => s.align,
                        };
                        if get(&a, "fo:break-before") == Some("page") {
                            s.page_break = true;
                        }
                        if get(&a, "fo:border-bottom").is_some_and(|b| b != "none") {
                            s.border_bottom = true;
                        }
                    }
                }
                "list-style" => list = get(&a, "style:name").map(|n| (n.to_string(), vec![false; 10])),
                "list-level-style-number" | "list-level-style-bullet" | "list-level-style-image" => {
                    if let Some((_, v)) = &mut list {
                        let lv = get(&a, "text:level").and_then(|l| l.parse::<usize>().ok()).unwrap_or(1).clamp(1, 10);
                        let ordered = local(&q) == "list-level-style-number" && get(&a, "style:num-format").is_some_and(|f| !f.is_empty());
                        if let Some(slot) = v.get_mut(lv - 1) {
                            *slot = ordered;
                        }
                    }
                }
                _ => {}
            },
            Ev::End(q) => match local(&q) {
                "automatic-styles" => in_auto = false,
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
                _ => {}
            },
            Ev::Text(_) => {}
        });
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
    fn text_fmt(&self, name: &str, base: &Fmt) -> Fmt {
        let mut f = base.clone();
        for (_, s) in self.chain(name).iter().rev() {
            s.fmt.apply(&mut f);
        }
        f
    }

    /// Paragraph kind, alignment, page break, rule and base formatting (automatic styles only).
    fn para(&self, name: &str) -> (Kind, Option<Align>, bool, bool, Fmt) {
        let chain = self.chain(name);
        let mut kind = Kind::Normal;
        for (n, s) in &chain {
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
        }
        let align = chain.iter().find_map(|(_, s)| s.align);
        let pb = chain.iter().take_while(|(_, s)| s.auto).any(|(_, s)| s.page_break);
        let rule = chain.iter().any(|(_, s)| s.border_bottom);
        let mut f = Fmt::default();
        for (_, s) in chain.iter().rev().filter(|(_, s)| s.auto) {
            s.fmt.apply(&mut f);
        }
        (kind, align, pb, rule, f)
    }
}

struct TableB {
    rows: Vec<Vec<Cell>>,
    row: Vec<Cell>,
    /// Covered cells still owed to the previous cell's column span.
    hcover: usize,
}

struct Body<'a> {
    styles: &'a Styles,
    zip: &'a mut dyn FnMut(&str) -> Option<Vec<u8>>,
    containers: Vec<Vec<FBlock>>,
    fmt: Vec<Fmt>,
    para: Option<Para>,
    para_cont: bool,
    /// Open lists: (style name, item open, item used).
    lists: Vec<(Option<String>, bool)>,
    item_used: Vec<bool>,
    /// Lists outside the table cells being read.
    lists_saved: Vec<(Vec<(Option<String>, bool)>, Vec<bool>)>,
    tables: Vec<TableB>,
    cells: Vec<Cell>,
    in_text: bool,
    skip: usize,
    depth: usize,
    frame: Option<(Option<f32>, Option<f32>, Option<Vec<u8>>, String)>,
    in_desc: bool,
}

impl Body<'_> {
    fn container(&mut self) -> &mut Vec<FBlock> {
        if self.containers.is_empty() {
            self.containers.push(Vec::new());
        }
        let n = self.containers.len() - 1;
        &mut self.containers[n]
    }

    fn cur_fmt(&self) -> Fmt {
        self.fmt.last().cloned().unwrap_or_default()
    }

    fn start_para(&mut self, style: Option<&str>, heading: Option<u8>) {
        self.end_para();
        let (mut kind, align, pb, rule, base) = style.map(|s| self.styles.para(s)).unwrap_or((Kind::Normal, None, false, false, Fmt::default()));
        if let Some(h) = heading {
            kind = Kind::Heading(h.clamp(1, 6));
        }
        let mut p = Para { kind, align, page_break: pb, ..Default::default() };
        if rule && kind == Kind::Normal {
            p.kind = Kind::Rule;
        }
        if let Some(lv) = self.lists.len().checked_sub(1)
            && self.lists.last().is_some_and(|l| l.1)
        {
            let style = self.lists.iter().rev().find_map(|l| l.0.clone());
            let ordered = style.and_then(|s| self.styles.lists.get(&s)).and_then(|v| v.get(lv.min(9)).copied()).unwrap_or(false);
            p.list = Some(ListInfo { ordered, level: lv.min(8) as u8 });
            self.para_cont = self.item_used.last().copied().unwrap_or(false);
            if let Some(u) = self.item_used.last_mut() {
                *u = true;
            }
        } else {
            self.para_cont = false;
        }
        self.fmt.push(base);
        self.para = Some(p);
    }

    fn end_para(&mut self) {
        let Some(mut p) = self.para.take() else { return };
        self.fmt.pop();
        if p.kind != Kind::Code {
            p.trim();
        }
        if p.kind == Kind::Rule && !p.is_empty() {
            p.kind = Kind::Normal;
        }
        if matches!(p.kind, Kind::Code | Kind::Heading(_) | Kind::Title) {
            for i in &mut p.inlines {
                if let Inline::Text(_, f) = i {
                    if p.kind == Kind::Code {
                        f.code = false;
                        f.font = None;
                    }
                    f.size = None;
                }
            }
        }
        let cont = std::mem::take(&mut self.para_cont);
        let level = p.list.map(|l| l.level);
        let c = self.container();
        if cont
            && let Some(FBlock::Para(prev)) = c.last_mut()
            && prev.list.is_some()
            && prev.list.map(|l| l.level) == level
        {
            prev.push_text("\n", &Fmt::default());
            prev.inlines.extend(p.inlines);
            return;
        }
        c.push(FBlock::Para(p));
    }

    fn push_text(&mut self, t: &str) {
        let f = self.cur_fmt();
        if self.para.is_none() {
            if t.trim().is_empty() {
                return;
            }
            self.start_para(None, None);
        }
        if let Some(p) = &mut self.para {
            p.push_text(t, &f);
        }
    }

    fn text(&mut self, t: &str) {
        if self.skip > 0 || !self.in_text {
            return;
        }
        if self.in_desc {
            if let Some(fr) = &mut self.frame {
                fr.3.push_str(t);
            }
            return;
        }
        let Some(p) = &self.para else {
            return;
        };
        // Collapse XML whitespace.
        let mut prev_space = match p.inlines.last() {
            Some(Inline::Text(x, _)) => x.ends_with([' ', '\n', '\t']),
            Some(_) => false,
            None => true,
        };
        let mut s = String::with_capacity(t.len());
        for c in t.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r') {
                if !prev_space {
                    s.push(' ');
                    prev_space = true;
                }
            } else {
                s.push(c);
                prev_space = false;
            }
        }
        if !s.is_empty() {
            self.push_text(&s);
        }
    }

    fn start(&mut self, q: &str, a: &Attrs) {
        self.depth += 1;
        if q == "office:text" {
            self.in_text = true;
            return;
        }
        if self.skip > 0 {
            self.skip += 1;
            return;
        }
        if !self.in_text {
            return;
        }
        match local(q) {
            "note"
            | "annotation"
            | "tracked-changes"
            | "text-box"
            | "sequence-decls"
            | "forms"
            | "variable-decls"
            | "user-field-decls"
            | "index-title-template"
            | "table-of-content-source" => {
                self.skip = 1;
            }
            "p" | "h" if !self.in_desc => {
                let h = if local(q) == "h" { Some(get(a, "text:outline-level").and_then(|v| v.parse::<u8>().ok()).unwrap_or(1)) } else { None };
                self.start_para(get(a, "text:style-name"), h);
            }
            "span" => {
                let base = self.cur_fmt();
                let f = match get(a, "text:style-name") {
                    Some(s) => self.styles.text_fmt(s, &base),
                    None => base,
                };
                self.fmt.push(f);
            }
            "a" => {
                let mut f = self.cur_fmt();
                if let Some(h) = get(a, "xlink:href").filter(|h| !h.is_empty()) {
                    f.link = Some(h.to_string());
                }
                if let Some(s) = get(a, "text:style-name") {
                    f = self.styles.text_fmt(s, &f);
                }
                self.fmt.push(f);
            }
            "s" => {
                let n = get(a, "text:c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).clamp(1, 1000);
                self.push_text(&" ".repeat(n));
            }
            "tab" => self.push_text("\t"),
            "line-break" => self.push_text("\n"),
            "bookmark" | "bookmark-start" => {
                if let Some(n) = get(a, "text:name") {
                    if self.para.is_none() {
                        self.start_para(None, None);
                    }
                    if let Some(p) = &mut self.para {
                        p.inlines.push(Inline::Anchor(n.to_string()));
                    }
                }
            }
            "list" => {
                self.end_para();
                if self.lists.len() < 64 {
                    self.lists.push((get(a, "text:style-name").map(String::from), false));
                    self.item_used.push(false);
                } else {
                    self.skip = 1;
                }
            }
            "list-item" | "list-header" => {
                if let Some(l) = self.lists.last_mut() {
                    l.1 = true;
                }
                if let Some(u) = self.item_used.last_mut() {
                    *u = false;
                }
            }
            "table" if q.starts_with("table:") => {
                self.end_para();
                if self.tables.len() >= model::MAX_DEPTH {
                    self.skip = 1;
                    return;
                }
                self.tables.push(TableB { rows: Vec::new(), row: Vec::new(), hcover: 0 });
            }
            "table-row" => {
                if let Some(t) = self.tables.last_mut() {
                    t.row.clear();
                    t.hcover = 0;
                }
            }
            "table-cell" => {
                self.end_para();
                let span = get(a, "table:number-columns-spanned").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1).clamp(1, 63);
                let rows = get(a, "table:number-rows-spanned").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1).clamp(1, 1000);
                let shading = get(a, "table:style-name").and_then(|s| self.styles.map.get(s)).and_then(|s| s.fmt.background);
                if let Some(t) = self.tables.last_mut() {
                    t.hcover = span as usize - 1;
                }
                self.cells.push(Cell { colspan: span, rowspan: rows, shading, ..Default::default() });
                self.containers.push(Vec::new());
                self.lists_saved.push((std::mem::take(&mut self.lists), std::mem::take(&mut self.item_used)));
            }
            "covered-table-cell" => {
                if let Some(t) = self.tables.last_mut() {
                    if t.hcover > 0 {
                        t.hcover -= 1;
                    } else if t.row.len() < 63 {
                        t.row.push(Cell { covered: true, ..Default::default() });
                    }
                }
            }
            "frame" => {
                let w = get(a, "svg:width").and_then(length);
                let h = get(a, "svg:height").and_then(length);
                self.frame = Some((w, h, None, String::new()));
            }
            "image" => {
                if let Some(href) = get(a, "xlink:href")
                    && let Some(fr) = &mut self.frame
                    && fr.2.is_none()
                {
                    let path = href.trim_start_matches("./");
                    fr.2 = (self.zip)(path);
                }
            }
            "desc" | "title" if self.frame.is_some() => self.in_desc = true,
            _ => {}
        }
    }

    fn end(&mut self, q: &str) {
        self.depth = self.depth.saturating_sub(1);
        if q == "office:text" {
            self.end_para();
            self.in_text = false;
            return;
        }
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        if !self.in_text {
            return;
        }
        match local(q) {
            "p" | "h" if !self.in_desc => self.end_para(),
            "span" | "a" => {
                if self.fmt.len() > usize::from(self.para.is_some()) {
                    self.fmt.pop();
                }
            }
            "list" => {
                self.end_para();
                self.lists.pop();
                self.item_used.pop();
            }
            "list-item" | "list-header" => {
                self.end_para();
                if let Some(l) = self.lists.last_mut() {
                    l.1 = false;
                }
            }
            "table-cell" => {
                self.end_para();
                let blocks = if self.containers.len() > 1 { self.containers.pop().unwrap_or_default() } else { Vec::new() };
                (self.lists, self.item_used) = self.lists_saved.pop().unwrap_or_default();
                let mut cell = self.cells.pop().unwrap_or_default();
                cell.blocks = blocks;
                if let Some(t) = self.tables.last_mut()
                    && t.row.len() < 63
                {
                    t.row.push(cell);
                }
            }
            "table-row" => {
                if let Some(t) = self.tables.last_mut() {
                    let r = std::mem::take(&mut t.row);
                    if !r.is_empty() && t.rows.len() < wordcraft_doc::table::MAX_ROWS {
                        t.rows.push(r);
                    }
                }
            }
            "table-header-rows" => {
                if let Some(t) = self.tables.last_mut() {
                    for r in &mut t.rows {
                        for c in r {
                            c.header = true;
                        }
                    }
                }
            }
            "table" if q.starts_with("table:") => {
                self.end_para();
                if let Some(t) = self.tables.pop()
                    && !t.rows.is_empty()
                {
                    self.container().push(FBlock::Table(FTable { rows: t.rows, widths: Vec::new(), borderless: false }));
                }
            }
            "desc" | "title" => self.in_desc = false,
            "frame" => {
                if let Some((w, h, Some(data), alt)) = self.frame.take()
                    && let Some(img) = make_img(data, w, h, alt.trim())
                {
                    if self.para.is_none() {
                        self.start_para(None, None);
                    }
                    if let Some(p) = &mut self.para {
                        p.inlines.push(Inline::Image(img));
                    }
                }
                self.frame = None;
            }
            _ => {}
        }
    }
}

fn read_entry(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
    let f = z.by_name(name).ok()?;
    if f.size() > MAX_ENTRY {
        return None;
    }
    let mut out = Vec::new();
    f.take(MAX_ENTRY).read_to_end(&mut out).ok()?;
    Some(out)
}

fn read_meta(xml: &[u8]) -> Meta {
    let mut m = Meta::default();
    let mut cur: Option<String> = None;
    let mut keywords: Vec<String> = Vec::new();
    events(xml, |ev| match ev {
        Ev::Start(q, _) => cur = Some(q),
        Ev::End(_) => cur = None,
        Ev::Text(t) => match cur.as_deref() {
            Some("dc:title") => m.title.push_str(&t),
            Some("dc:subject") => m.subject.push_str(&t),
            Some("dc:description") => m.description.push_str(&t),
            Some("meta:initial-creator") => m.author.push_str(&t),
            Some("dc:creator") if m.author.is_empty() => m.author.push_str(&t),
            Some("meta:keyword") => keywords.push(t),
            Some("meta:creation-date") => m.created.push_str(&t),
            _ => {}
        },
    });
    m.keywords = keywords.join(", ");
    if !m.created.is_empty() && !m.created.ends_with('Z') && !m.created.contains('+') {
        let base = m.created.split('.').next().unwrap_or("").to_string();
        m.created = format!("{base}Z");
    }
    m
}

/// Parse an `.odt` package into the flow model.
pub fn parse(bytes: &[u8]) -> Result<Flow, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("ODT: not a valid package ({e})"))?;
    if let Some(mt) = read_entry(&mut z, "mimetype") {
        let mt = String::from_utf8_lossy(&mt);
        if !mt.trim().starts_with("application/vnd.oasis.opendocument.text") {
            return Err(format!("ODT: not a text document ({})", mt.trim()));
        }
    }
    let content = read_entry(&mut z, "content.xml").ok_or("ODT: content.xml is missing")?;
    let mut styles = Styles::default();
    if let Some(s) = read_entry(&mut z, "styles.xml") {
        styles.read(&s, false);
    }
    styles.read(&content, true);
    let meta = read_entry(&mut z, "meta.xml").map(|m| read_meta(&m)).unwrap_or_default();
    let mut total = 0u64;
    let mut fetch = |path: &str| -> Option<Vec<u8>> {
        let d = read_entry(&mut z, path)?;
        total += d.len() as u64;
        (total <= MAX_ENTRY).then_some(d)
    };
    let mut body = Body {
        styles: &styles,
        zip: &mut fetch,
        containers: vec![Vec::new()],
        fmt: Vec::new(),
        para: None,
        para_cont: false,
        lists: Vec::new(),
        lists_saved: Vec::new(),
        item_used: Vec::new(),
        tables: Vec::new(),
        cells: Vec::new(),
        in_text: false,
        skip: 0,
        depth: 0,
        frame: None,
        in_desc: false,
    };
    events(&content, |ev| match ev {
        Ev::Start(q, a) => {
            if body.depth < MAX_XML_DEPTH {
                body.start(&q, &a);
            } else {
                body.depth += 1;
            }
        }
        Ev::End(q) => {
            if body.depth <= MAX_XML_DEPTH {
                body.end(&q);
            } else {
                body.depth -= 1;
            }
        }
        Ev::Text(t) => body.text(&t),
    });
    body.end_para();
    while body.containers.len() > 1 {
        let c = body.containers.pop().unwrap_or_default();
        body.container().extend(c);
    }
    let mut blocks = body.containers.pop().unwrap_or_default();
    for t in body.tables.drain(..) {
        if !t.rows.is_empty() {
            blocks.push(FBlock::Table(FTable { rows: t.rows, widths: Vec::new(), borderless: false }));
        }
    }
    if blocks.is_empty() {
        blocks.push(FBlock::Para(Para::default()));
    }
    Ok(Flow { blocks, meta })
}

/// Parse an `.odt` package into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    Ok(model::to_doc(&parse(bytes)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(length("1in"), Some(72.0));
        assert_eq!(length("12pt"), Some(12.0));
        assert!((length("2.54cm").unwrap() - 72.0).abs() < 0.01);
        assert_eq!(length("x"), None);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"not a zip").is_err());
    }
}
