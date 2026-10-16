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
//! resolved, both automatic and common styles), skipping notes, annotations, text boxes and drawing shapes.
//!
//! Tracked changes (ODF 1.3 Part 3 §5.5) go both ways: `text:tracked-changes` holds one
//! `text:changed-region` per change; an insertion is the body text between
//! `text:change-start`/`text:change-end`, a deletion keeps the deleted paragraphs in its region
//! and leaves a `text:change` point in the body. They map to `Document::revisions` and the
//! `ins`/`del` marks of runs and paragraph marks; a deleted paragraph mark is a region of
//! several paragraphs. Format changes are read as no change.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use wordcraft_doc::props::VAlign;
use wordcraft_doc::section::{Columns, SectionStart};
use wordcraft_doc::{Align, Document, RevisionKind, Rgb, SectionProps};

use crate::model::{self, Cell, FBlock, FTable, Flow, Fmt, Inline, Kind, ListInfo, Meta, Para, RevMark, make_img, mime_of};

/// Largest zip entry we inflate.
const MAX_ENTRY: u64 = 256 << 20;
/// Deepest element nesting handled in content.xml.
const MAX_XML_DEPTH: usize = 512;
/// Most sections an import makes (page-style changes and column sections); later ones are ignored.
const MAX_SECTIONS: usize = 10_000;
/// Most `text:changed-region`s read (hostile input).
const MAX_REGIONS: usize = 10_000;
/// Most insertions open at once (`text:change-start` without its end yet).
const MAX_OPEN_INSERTIONS: usize = 64;
/// Longest change author or date kept, in characters.
const MAX_INFO_CHARS: usize = 256;

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
    /// Cell styles by (shading, vertical-alignment).
    cell_styles: Vec<(Option<Rgb>, Option<VAlign>)>,
    pictures: Vec<(String, std::sync::Arc<Vec<u8>>, String)>,
    /// `text:changed-region` elements written so far.
    regions: Vec<String>,
    /// The insertion open in the body, and its region id.
    open_ins: Option<(RevMark, String)>,
    /// Style of the paragraph being written.
    cur_style: String,
    /// Styles of the paragraphs joined into the one being written (deleted paragraph marks),
    /// and which of them the text being written belongs to.
    chain: Vec<String>,
    part: usize,
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

    fn close_ins(&mut self, out: &mut String) {
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
        let inl = model::equations_as_text(inl);
        for i in inl.iter() {
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
                Inline::Figure(_) | Inline::Equation { .. } => {}
            }
        }
    }

    /// Paragraph `p`, or with `chain` the paragraphs joined at deleted paragraph marks (`p`
    /// is the first).
    fn para_in(&mut self, p: &Para, chain: Option<&Vec<Para>>, out: &mut String) {
        let paras: &[Para] = chain.map(|c| c.as_slice()).unwrap_or(std::slice::from_ref(p));
        self.chain = chain.map(|c| c.iter().map(|q| self.para_style(q)).collect()).unwrap_or_default();
        self.part = 0;
        let style = self.para_style(p);
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
        let tracked = paras.len() > 1 || p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) if f.ins.is_some() || f.del.is_some()));
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

    fn blocks(&mut self, blocks: &[FBlock], out: &mut String, depth: usize) {
        let joined = join_deleted_marks(blocks);
        let (blocks, chains) = match &joined {
            Some((b, c)) => (b.as_slice(), Some(c)),
            None => (blocks, None),
        };
        let chain = |i: usize| chains.and_then(|c| c.get(&i));
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
                        // A top-level item of the other kind starts a new list.
                        if l == 0 && seen[0] && kinds[0] != li.ordered {
                            break;
                        }
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

    fn cell_style(&mut self, shading: Option<Rgb>, valign: Option<VAlign>) -> String {
        let key = (shading, valign);
        let k = match self.cell_styles.iter().position(|s| *s == key) {
            Some(k) => k,
            None => {
                self.cell_styles.push(key);
                self.cell_styles.len() - 1
            }
        };
        format!("Cell{}", k + 1)
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
                let st = self.cell_style(c.shading, c.valign);
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
        for (i, (sh, va)) in self.cell_styles.iter().enumerate() {
            let bg = sh.map(|c| format!(" fo:background-color=\"#{}\"", c.hex().to_ascii_lowercase())).unwrap_or_default();
            let va_str = match va {
                Some(VAlign::Top) => " style:vertical-align=\"top\"",
                Some(VAlign::Center) => " style:vertical-align=\"middle\"",
                Some(VAlign::Bottom) => " style:vertical-align=\"bottom\"",
                None => "",
            };
            s.push_str(&format!(
                "<style:style style:name=\"Cell{}\" style:family=\"table-cell\"><style:table-cell-properties fo:padding=\"4pt\" fo:border=\"0.5pt solid #000000\"{bg}{va_str}/></style:style>",
                i + 1
            ));
        }
        s.push_str("</office:automatic-styles>");
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
            Item::Inl(Inline::Text(_, f)) => (f.ins.as_ref(), f.del.as_ref()),
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

/// `blocks` with each run of paragraphs joined at tracked deleted paragraph marks (ODF keeps
/// such a deletion inside one body paragraph): the first paragraph stands for the run, and the
/// whole run is returned by its index. `None` when nothing is joined. A deleted mark with no
/// paragraph after it (before a table, at the end) is written as a plain mark.
#[allow(clippy::type_complexity)]
fn join_deleted_marks(blocks: &[FBlock]) -> Option<(Vec<FBlock>, HashMap<usize, Vec<Para>>)> {
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
    // Days since 1970-01-01 → civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Seconds since the Unix epoch: the system clock, or the browser's on the web.
fn now_unix() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok()).unwrap_or(0)
    }
    // `SystemTime::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(target_arch = "wasm32")]
    {
        let ms = js_sys::Date::now();
        if ms.is_finite() && ms > 0.0 { (ms / 1000.0) as i64 } else { 0 }
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
    let flow = model::from_doc_tracked(doc);
    let mut w = Writer::default();
    let mut body = String::new();
    w.blocks(&flow.blocks, &mut body, 0);
    w.close_ins(&mut body);
    let tracked = if w.regions.is_empty() && !doc.settings.track_changes {
        String::new()
    } else {
        format!("<text:tracked-changes text:track-changes=\"{}\">{}</text:tracked-changes>\n", doc.settings.track_changes, w.regions.concat())
    };
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NS}>{}<office:body><office:text>\n{tracked}{body}</office:text></office:body></office:document-content>\n",
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
    /// `fo:background-color` of a table-cell style's `style:table-cell-properties`.
    cell_background: Option<Rgb>,
    /// `style:vertical-align` of a table-cell style's `style:table-cell-properties`.
    cell_valign: Option<VAlign>,
    /// `style:master-page-name` of a paragraph or table style: its paragraphs (tables) start a
    /// page of that master page. Empty: explicitly none, which stops the lookup up the chain.
    master_page: Option<String>,
    /// `style:page-number` of a paragraph or table style (applied only with a master page): `Some(Some(n))` restarts page numbering at n,
    /// `Some(None)` ("auto") continues it.
    page_number: Option<Option<u32>>,
    /// `style:columns` of a section style.
    columns: Option<Columns>,
}

/// Columns of a `style:columns` element (ODF 1.3 §17.12): count and gap, clamped as the RTF and
/// DOCX readers do; equal widths only (`style:column` widths are not read).
fn columns_of(a: &Attrs) -> Columns {
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
fn column_sep(a: &Attrs) -> bool {
    get(a, "style:style") != Some("none")
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
        let val =
            a.normalized_value(XmlVersion::Explicit1_0).map(|c| c.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned());
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
                            master_page: get(&a, "style:master-page-name").map(String::from),
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
                "table-properties" => {
                    if let Some((_, s)) = &mut cur
                        && let Some(v) = get(&a, "style:page-number")
                    {
                        s.page_number = Some(v.trim().parse::<u32>().ok().map(|n| n.min(32_767)));
                    }
                }
                "table-cell-properties" => {
                    if let Some((_, s)) = &mut cur {
                        if let Some(c) = get(&a, "fo:background-color").and_then(model::parse_color) {
                            s.cell_background = Some(c);
                        }
                        if let Some(va) = get(&a, "style:vertical-align") {
                            s.cell_valign = match va.trim() {
                                "top" => Some(VAlign::Top),
                                "middle" | "center" => Some(VAlign::Center),
                                "bottom" => Some(VAlign::Bottom),
                                _ => None,
                            };
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
                        if let Some(v) = get(&a, "style:page-number") {
                            s.page_number = Some(v.trim().parse::<u32>().ok().map(|n| n.min(32_767)));
                        }
                        if get(&a, "fo:border-bottom").is_some_and(|b| b != "none") {
                            s.border_bottom = true;
                        }
                    }
                }
                "columns" if q.starts_with("style:") => {
                    if let Some((_, s)) = &mut cur {
                        s.columns = Some(columns_of(&a));
                    }
                }
                "column-sep" => {
                    if let Some((_, s)) = &mut cur
                        && let Some(c) = &mut s.columns
                    {
                        c.separator = column_sep(&a);
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

    /// The page a paragraph style starts (`style:master-page-name` and `style:page-number`,
    /// ODF 1.3 §16.2 and §17.6): the nearest master page and page-number setting in its chain.
    fn page_start<'a>(&'a self, name: &'a str) -> (Option<&'a str>, Option<Option<u32>>) {
        let chain = self.chain(name);
        let master = chain.iter().find_map(|(_, s)| s.master_page.as_deref()).filter(|m| !m.is_empty());
        (master, chain.iter().find_map(|(_, s)| s.page_number))
    }

    /// Columns of a section style (whole chain).
    fn columns(&self, name: &str) -> Option<Columns> {
        self.chain(name).iter().find_map(|(_, s)| s.columns.clone())
    }

    /// Background colour of a table-cell style (whole chain).
    fn cell_background(&self, name: &str) -> Option<Rgb> {
        self.chain(name).iter().find_map(|(_, s)| s.cell_background)
    }

    /// Vertical alignment of a table-cell style (whole chain).
    fn cell_valign(&self, name: &str) -> Option<VAlign> {
        self.chain(name).iter().find_map(|(_, s)| s.cell_valign)
    }

    /// Paragraph kind, alignment, page break, rule and base formatting. The text formatting of
    /// every style in the chain applies (automatic and common, ODF 1.3 §16.2 inheritance), except
    /// the style that gives the paragraph its kind and that style's ancestors: their formatting
    /// is the kind's own look (a heading's size and colour), not direct formatting. `Standard`,
    /// the default paragraph style, holds document defaults and is left out too.
    fn para(&self, name: &str) -> (Kind, Option<Align>, bool, bool, Fmt) {
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
        let align = chain.iter().find_map(|(_, s)| s.align);
        let pb = chain.iter().take_while(|(_, s)| s.auto).any(|(_, s)| s.page_break);
        let rule = chain.iter().any(|(_, s)| s.border_bottom);
        let mut f = Fmt::default();
        for (_, s) in chain.iter().take(kind_at).rev().filter(|(n, s)| s.auto || *n != "Standard") {
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

/// A `text:changed-region` read from `text:tracked-changes`.
#[derive(Default)]
struct Region {
    kind: Option<RevisionKind>,
    /// Sequence number: tells regions by the same author at the same time apart.
    id: u32,
    author: String,
    date: String,
    /// A deletion's deleted paragraphs.
    blocks: Vec<FBlock>,
}

impl Region {
    fn mark(&self) -> RevMark {
        RevMark { author: self.author.trim().to_string(), date: self.date.trim().to_string(), id: Some(self.id) }
    }
}

/// Reader state saved while a deletion's content is read into its own container.
struct DelState {
    containers: usize,
    tables: usize,
    cells: usize,
    lists_saved: usize,
    fmt: usize,
    lists: Vec<(Option<String>, bool)>,
    item_used: Vec<bool>,
    /// Insertions open inside the deleted content (text inserted, then deleted).
    ins: Vec<(String, RevMark)>,
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
    /// The master pages' page setups.
    pages: &'a Pages,
    /// The current master page's setup (a column section's page, without its columns).
    page: SectionProps,
    /// The open section; where its blocks start in the body; how many sections have ended.
    sec: SectionProps,
    sec_from: usize,
    sections: usize,
    /// Element depth of the open columned `text:section`.
    cols_at: Option<usize>,
    /// Inside `text:tracked-changes`.
    in_tc: bool,
    /// `text:track-changes` of the document's tracked changes (default on), when it has them.
    track: Option<bool>,
    /// Changed regions read, by id.
    regions: HashMap<String, Region>,
    /// Regions seen (capped at [`MAX_REGIONS`]).
    region_count: usize,
    /// The region being read.
    region: Option<(String, Region)>,
    /// Depth of the `office:change-info` being read, and the field its text goes to.
    info_at: Option<usize>,
    info_field: Option<bool>,
    /// Reading a deletion's content.
    del: Option<DelState>,
    /// Insertions open in the body: (region id, revision).
    active_ins: Vec<(String, RevMark)>,
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

    /// Reading the body itself, not a table cell or a frame.
    fn in_body(&self) -> bool {
        self.containers.len() == 1 && self.tables.is_empty() && self.frame.is_none()
    }

    /// End the open section before what comes next, which starts section `next`. An open section
    /// with nothing in it yet takes `next`'s place instead. Only in the body, and at most
    /// [`MAX_SECTIONS`]. True when what comes next follows a section break.
    fn break_section(&mut self, next: SectionProps) -> bool {
        if !self.in_body() {
            return false;
        }
        let Some(body) = self.containers.first_mut() else { return false };
        if body.len() <= self.sec_from {
            let restart = self.sec.page_num_start;
            self.sec = next;
            self.sec.page_num_start = self.sec.page_num_start.or(restart);
            return self.sections > 0;
        }
        if self.sections + 1 >= MAX_SECTIONS {
            return false;
        }
        // A section ends on a paragraph: after a table, an empty one (as Word writes it).
        if !matches!(body.last(), Some(FBlock::Para(p)) if p.section.is_none()) {
            body.push(FBlock::Para(Para::default()));
        }
        if let Some(FBlock::Para(p)) = body.last_mut() {
            p.section = Some(Box::new(std::mem::replace(&mut self.sec, next)));
        }
        self.sec_from = body.len();
        self.sections += 1;
        true
    }

    /// A body paragraph or table of style `name` that names a master page starts a new page with
    /// that page's setup and, when the style says so, page numbering restarted (ODF 1.3 applies
    /// `style:page-number` only with a master page). The section that starts there, if any.
    ///
    /// Rare cases left simple: a master-page paragraph first inside a columned `text:section`
    /// replaces that (still empty) column section, so the columns are lost; a columned section
    /// nested in another is read as part of the outer one.
    fn page_section(&mut self, name: &str) -> Option<SectionProps> {
        if !self.in_body() {
            return None;
        }
        let (master, number) = self.styles.page_start(name);
        let restart = number.flatten();
        let page = master.and_then(|m| self.pages.masters.get(m))?.clone();
        self.page = page.clone();
        Some(SectionProps { start: SectionStart::NextPage, page_num_start: restart, ..page })
    }

    fn start_para(&mut self, style: Option<&str>, heading: Option<u8>) {
        self.end_para();
        let (mut kind, align, mut pb, rule, base) = style.map(|s| self.styles.para(s)).unwrap_or((Kind::Normal, None, false, false, Fmt::default()));
        // The section break starts the new page; no page break besides.
        if let Some(next) = style.and_then(|s| self.page_section(s))
            && self.break_section(next)
        {
            pb = false;
        }
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
        // A paragraph ending inside an open insertion has an inserted mark. (Paragraphs that
        // `place_deletion` splits off keep only the marks read inside their region.)
        if p.mark_ins.is_none() {
            p.mark_ins = self.open_insertion().map(Box::new);
        }
        self.finish_para(p);
    }

    /// The innermost insertion open where text is being read (the body, or deleted content).
    fn open_insertion(&self) -> Option<RevMark> {
        let open = match &self.del {
            Some(d) => &d.ins,
            None => &self.active_ins,
        };
        open.last().map(|(_, m)| m.clone())
    }

    /// Add a finished paragraph to the current container.
    fn finish_para(&mut self, mut p: Para) {
        // Spaces next to tracked text are part of a change.
        let tracked = self.del.is_some()
            || p.mark_ins.is_some()
            || p.mark_del.is_some()
            || p.inlines.iter().any(|i| matches!(i, Inline::Text(_, f) if f.ins.is_some() || f.del.is_some()));
        if p.kind != Kind::Code && !tracked {
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
        let mut f = self.cur_fmt();
        f.ins = self.open_insertion();
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
        if self.info_at.is_some() {
            if let (Some(field), Some((_, r))) = (self.info_field, &mut self.region) {
                let s = if field { &mut r.author } else { &mut r.date };
                let room = MAX_INFO_CHARS.saturating_sub(s.chars().count());
                s.extend(t.chars().take(room));
            }
            return;
        }
        if self.in_tc && self.del.is_none() {
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
        if self.info_at.is_some() {
            self.info_field = match local(q) {
                "creator" => Some(true),
                "date" => Some(false),
                _ => None,
            };
            return;
        }
        if self.in_tc {
            if self.del.is_none() {
                self.tracked_start(q, a);
                return;
            }
            // Inside deleted content: only insertions (text inserted, then deleted) nest.
            if matches!(local(q), "change-start" | "change-end") {
                let regions = &self.regions;
                if let Some(d) = &mut self.del {
                    change_mark(regions, &mut d.ins, local(q) == "change-start", a);
                }
                return;
            }
            if matches!(local(q), "tracked-changes" | "changed-region" | "insertion" | "deletion" | "format-change" | "change") {
                self.skip = 1;
                return;
            }
            if local(q) == "change-info" && self.region.is_some() {
                self.info_at = Some(self.depth);
                return;
            }
        }
        match local(q) {
            "tracked-changes" => {
                if self.para.is_none() && self.tables.is_empty() && self.cells.is_empty() && self.frame.is_none() {
                    self.in_tc = true;
                    self.track = Some(get(a, "text:track-changes") != Some("false"));
                } else {
                    self.skip = 1;
                }
            }
            "change" => {
                if let Some(id) = get(a, "text:change-id")
                    && self.regions.get(id).is_some_and(|r| r.kind == Some(RevisionKind::Delete))
                    && let Some(r) = self.regions.remove(id)
                {
                    let mark = r.mark();
                    self.place_deletion(r.blocks, mark);
                }
            }
            "change-start" | "change-end" => change_mark(&self.regions, &mut self.active_ins, local(q) == "change-start", a),
            "note"
            | "annotation"
            | "text-box"
            | "sequence-decls"
            | "forms"
            | "variable-decls"
            | "user-field-decls"
            | "index-title-template"
            | "table-of-content-source" => {
                self.skip = 1;
            }
            // Drawing shapes (ODF 1.3 §10.3), with their text, until shapes are read (#1654).
            // Groups are walked, so the pictures in them still are.
            "rect" | "line" | "polyline" | "polygon" | "regular-polygon" | "path" | "circle" | "ellipse" | "connector" | "caption" | "measure"
            | "custom-shape"
                if q.starts_with("draw:") =>
            {
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
                // A body table whose style names a master page starts that page (ODF 1.3 §19.509).
                if let Some(next) = get(a, "table:style-name").and_then(|s| self.page_section(s)) {
                    self.break_section(next);
                }
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
                let shading = get(a, "table:style-name").and_then(|s| self.styles.cell_background(s));
                let valign = get(a, "table:style-name").and_then(|s| self.styles.cell_valign(s));
                if let Some(t) = self.tables.last_mut() {
                    t.hcover = span as usize - 1;
                }
                self.cells.push(Cell { colspan: span, rowspan: rows, shading, valign, ..Default::default() });
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
            // A columned section (ODF 1.3 §5.4) is a continuous section with its columns.
            "section" if q.starts_with("text:") => {
                self.end_para();
                if self.cols_at.is_none()
                    && self.in_body()
                    && let Some(columns) = get(a, "text:style-name").and_then(|s| self.styles.columns(s)).filter(|c| c.count > 1)
                {
                    let next = SectionProps { start: SectionStart::Continuous, page_num_start: None, columns, ..self.page.clone() };
                    self.break_section(next);
                    self.cols_at = Some(self.depth);
                }
            }
            _ => {}
        }
    }

    /// An element inside `text:tracked-changes`, outside deleted content.
    fn tracked_start(&mut self, q: &str, a: &Attrs) {
        match local(q) {
            "changed-region" if self.region.is_none() && self.region_count < MAX_REGIONS => {
                let id = get(a, "text:id").unwrap_or_default().to_string();
                let r = Region { id: self.region_count as u32, ..Default::default() };
                self.region_count += 1;
                self.region = Some((id, r));
            }
            "insertion" | "deletion" | "format-change" if self.region.as_ref().is_some_and(|(_, r)| r.kind.is_none()) => {
                let kind = match local(q) {
                    "insertion" => RevisionKind::Insert,
                    "deletion" => RevisionKind::Delete,
                    _ => RevisionKind::Format,
                };
                if let Some((_, r)) = &mut self.region {
                    r.kind = Some(kind);
                }
                if kind == RevisionKind::Delete {
                    self.containers.push(Vec::new());
                    self.del = Some(DelState {
                        containers: self.containers.len(),
                        tables: self.tables.len(),
                        cells: self.cells.len(),
                        lists_saved: self.lists_saved.len(),
                        fmt: self.fmt.len(),
                        lists: std::mem::take(&mut self.lists),
                        item_used: std::mem::take(&mut self.item_used),
                        ins: Vec::new(),
                    });
                }
            }
            "change-info" if self.region.is_some() => self.info_at = Some(self.depth),
            _ => self.skip = 1,
        }
    }

    /// End a deletion's content: its paragraphs go to the region being read.
    fn close_deletion(&mut self) {
        if self.del.is_none() {
            return;
        }
        self.end_para();
        let Some(d) = self.del.take() else { return };
        let base = d.containers.saturating_sub(1).min(self.containers.len());
        let blocks: Vec<FBlock> = self.containers.split_off(base).into_iter().flatten().filter(|b| matches!(b, FBlock::Para(_))).collect();
        if self.containers.is_empty() {
            self.containers.push(Vec::new());
        }
        self.tables.truncate(d.tables);
        self.cells.truncate(d.cells);
        self.lists_saved.truncate(d.lists_saved);
        self.fmt.truncate(d.fmt);
        self.lists = d.lists;
        self.item_used = d.item_used;
        self.frame = None;
        self.in_desc = false;
        self.para_cont = false;
        if let Some((_, r)) = &mut self.region {
            r.blocks = blocks;
        }
    }

    /// Put a deletion's paragraphs back at a `text:change` point, as deleted text. Inside a
    /// paragraph, each deleted paragraph mark splits it; between paragraphs, each deleted
    /// paragraph stands alone with its mark deleted.
    fn place_deletion(&mut self, blocks: Vec<FBlock>, mark: RevMark) {
        let mut paras: Vec<Para> = Vec::new();
        for b in blocks {
            let FBlock::Para(mut p) = b else { continue };
            // Pictures and other objects carry no revision mark: leave them out.
            p.inlines.retain(|i| matches!(i, Inline::Text(..) | Inline::Anchor(_)));
            for i in &mut p.inlines {
                if let Inline::Text(_, f) = i {
                    f.del = Some(mark.clone());
                }
            }
            paras.push(p);
        }
        let append = |dst: &mut Para, src: Vec<Inline>| {
            for i in src {
                match i {
                    Inline::Text(t, f) => dst.push_text(&t, &f),
                    other => dst.inlines.push(other),
                }
            }
        };
        let Some(mut cur) = self.para.take() else {
            self.para_cont = false;
            for mut p in paras {
                p.mark_del = Some(Box::new(mark.clone()));
                self.finish_para(p);
            }
            return;
        };
        let n = paras.len();
        let mut it = paras.into_iter();
        let mut first_mark_ins = None;
        if let Some(first) = it.next() {
            first_mark_ins = first.mark_ins;
            append(&mut cur, first.inlines);
        }
        if n < 2 {
            self.para = Some(cur);
            return;
        }
        let mut rest = Para { kind: cur.kind, list: cur.list, align: cur.align, ..Default::default() };
        // The host paragraph now ends at the first deleted paragraph's mark.
        cur.mark_del = Some(Box::new(mark.clone()));
        cur.mark_ins = first_mark_ins;
        self.finish_para(cur);
        for (k, mut p) in it.enumerate() {
            if k + 2 == n {
                // The last deleted paragraph runs on into the rest of the body paragraph.
                rest.kind = p.kind;
                rest.align = p.align.or(rest.align);
                append(&mut rest, p.inlines);
            } else {
                p.mark_del = Some(Box::new(mark.clone()));
                self.finish_para(p);
            }
        }
        self.para = Some(rest);
    }

    fn end(&mut self, q: &str) {
        self.depth = self.depth.saturating_sub(1);
        if q == "office:text" {
            if self.del.is_some() {
                // Unclosed deleted content (hostile): drop it.
                self.close_deletion();
            }
            self.in_tc = false;
            self.region = None;
            self.info_at = None;
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
        if let Some(d) = self.info_at {
            if self.depth < d {
                self.info_at = None;
            }
            self.info_field = None;
            return;
        }
        if self.in_tc {
            match local(q) {
                "deletion" if self.del.is_some() => {
                    self.close_deletion();
                    return;
                }
                "changed-region" => {
                    // A deletion left open (hostile) ends with its region.
                    self.close_deletion();
                    if let Some((id, r)) = self.region.take()
                        && r.kind.is_some()
                    {
                        // The first region with an id wins.
                        self.regions.entry(id).or_insert(r);
                    }
                    return;
                }
                "tracked-changes" => {
                    self.close_deletion();
                    self.in_tc = false;
                    self.region = None;
                    return;
                }
                _ if self.del.is_some() => {}
                _ => return,
            }
        }
        match local(q) {
            "p" | "h" if !self.in_desc => self.end_para(),
            "span" | "a" if self.fmt.len() > usize::from(self.para.is_some()) => {
                self.fmt.pop();
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
            "section" if q.starts_with("text:") && self.cols_at == Some(self.depth + 1) => {
                self.end_para();
                self.cols_at = None;
                let next = SectionProps { start: SectionStart::Continuous, page_num_start: None, ..self.page.clone() };
                self.break_section(next);
            }
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

/// A `text:change-start` (`start`) or `text:change-end` (attributes `a`) among the insertions
/// `open`: unknown ids, ends without a start and starts past [`MAX_OPEN_INSERTIONS`] are ignored.
fn change_mark(regions: &HashMap<String, Region>, open: &mut Vec<(String, RevMark)>, start: bool, a: &Attrs) {
    let Some(id) = get(a, "text:change-id") else { return };
    if !start {
        if let Some(k) = open.iter().position(|(o, _)| o == id) {
            open.remove(k);
        }
        return;
    }
    if let Some(r) = regions.get(id).filter(|r| r.kind == Some(RevisionKind::Insert))
        && open.len() < MAX_OPEN_INSERTIONS
        && !open.iter().any(|(o, _)| o == id)
    {
        open.push((id.to_string(), r.mark()));
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

/// The pictures a package's frames point at, within one byte budget. Each entry is inflated
/// at most once, and only when the size it declares fits in what is left; the bytes inflated
/// (even from an entry bigger than it declares) and every later copy are charged, so repeated
/// references and lying entries can't go past the budget, and nothing is inflated after it.
struct Pictures {
    /// Entry name → its bytes (`None`: missing, unreadable or over the budget; not retried).
    seen: HashMap<String, Option<Vec<u8>>>,
    /// Bytes still allowed.
    room: u64,
}

impl Pictures {
    fn new(budget: u64) -> Self {
        Pictures { seen: HashMap::new(), room: budget }
    }

    /// The bytes of entry `name`, or `None` when it can't be read or doesn't fit.
    fn get(&mut self, z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
        if let Some(hit) = self.seen.get(name) {
            let d = hit.as_ref().filter(|d| d.len() as u64 <= self.room)?;
            self.room = self.room.saturating_sub(d.len() as u64);
            return Some(d.clone());
        }
        let d = self.inflate(z, name);
        self.seen.insert(name.to_string(), d.clone());
        d
    }

    /// Inflate entry `name`, charging the bytes inflated; `None` when it doesn't fit or fails.
    fn inflate(&mut self, z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<Vec<u8>> {
        let room = self.room;
        if room == 0 {
            return None;
        }
        let f = z.by_name(name).ok()?;
        if f.size() > room {
            return None;
        }
        let mut out = Vec::new();
        let read = f.take(room.saturating_add(1)).read_to_end(&mut out);
        self.room = room.saturating_sub(out.len() as u64);
        (read.is_ok() && out.len() as u64 <= room).then_some(out)
    }
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
    parse_full(bytes).map(|(flow, _)| flow)
}

/// [`parse`], with the last section's properties (the others are on the paragraphs that end them).
fn parse_full(bytes: &[u8]) -> Result<(Flow, SectionProps), String> {
    parse_tracked(bytes).map(|(f, last, _)| (f, last))
}

/// [`parse_full`], plus whether change tracking is on (`None`: the document has no tracked changes).
fn parse_tracked(bytes: &[u8]) -> Result<(Flow, SectionProps, Option<bool>), String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("ODT: not a valid package ({e})"))?;
    if let Some(mt) = read_entry(&mut z, "mimetype") {
        let mt = String::from_utf8_lossy(&mt);
        if !mt.trim().starts_with("application/vnd.oasis.opendocument.text") {
            return Err(format!("ODT: not a text document ({})", mt.trim()));
        }
    }
    let content = read_entry(&mut z, "content.xml").ok_or("ODT: content.xml is missing")?;
    let mut styles = Styles::default();
    let mut pages = Pages::default();
    if let Some(s) = read_entry(&mut z, "styles.xml") {
        styles.read(&s, false);
        pages = read_pages(&s);
    }
    let page = pages.first.clone().unwrap_or_default();
    styles.read(&content, true);
    let meta = read_entry(&mut z, "meta.xml").map(|m| read_meta(&m)).unwrap_or_default();
    let mut pictures = Pictures::new(MAX_ENTRY);
    let mut fetch = |path: &str| -> Option<Vec<u8>> {
        // A frame may point at any entry, as often as it likes: `Pictures` keeps that bounded.
        pictures.get(&mut z, path)
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
        pages: &pages,
        sec: page.clone(),
        page,
        sec_from: 0,
        sections: 0,
        cols_at: None,
        in_tc: false,
        track: None,
        regions: HashMap::new(),
        region_count: 0,
        region: None,
        info_at: None,
        info_field: None,
        del: None,
        active_ins: Vec::new(),
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
    // An open section with nothing in it (after a column section at the end) isn't one: the
    // section before it ends the document.
    let mut last = body.sec;
    if blocks.len() <= body.sec_from
        && let Some(FBlock::Para(p)) = blocks.last_mut()
        && let Some(s) = p.section.take()
    {
        last = *s;
    }
    if blocks.is_empty() {
        blocks.push(FBlock::Para(Para::default()));
    }
    let track = body.track;
    Ok((Flow { blocks, meta }, last, track))
}

/// Parse an `.odt` package into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    let (flow, last, track) = parse_tracked(bytes)?;
    let mut doc = model::to_doc(&flow);
    doc.last_section = last;
    if let Some(t) = track {
        doc.settings.track_changes = t;
    }
    Ok(doc)
}

/// The page setup of each master page in `styles.xml` (ODF 1.3 §16.9), and of the first one,
/// which pages use until a paragraph names another.
#[derive(Default)]
struct Pages {
    masters: HashMap<String, SectionProps>,
    first: Option<SectionProps>,
}

fn read_pages(xml: &[u8]) -> Pages {
    let mut layouts: HashMap<String, (Attrs, Option<Columns>)> = HashMap::new();
    let mut cur: Option<String> = None;
    let mut masters: Vec<(String, String)> = Vec::new();
    events(xml, |ev| match ev {
        Ev::Start(q, a) => match local(&q) {
            "page-layout" if q.starts_with("style:") => cur = get(&a, "style:name").map(String::from),
            "page-layout-properties" => {
                if let Some(n) = &cur
                    && layouts.len() < MAX_SECTIONS
                {
                    layouts.insert(n.clone(), (a, None));
                }
            }
            "columns" if q.starts_with("style:") => {
                if let Some(l) = cur.as_ref().and_then(|n| layouts.get_mut(n)) {
                    l.1 = Some(columns_of(&a));
                }
            }
            "column-sep" => {
                if let Some(c) = cur.as_ref().and_then(|n| layouts.get_mut(n)).and_then(|l| l.1.as_mut()) {
                    c.separator = column_sep(&a);
                }
            }
            "master-page" if masters.len() < MAX_SECTIONS => {
                if let (Some(n), Some(l)) = (get(&a, "style:name"), get(&a, "style:page-layout-name")) {
                    masters.push((n.to_string(), l.to_string()));
                }
            }
            _ => {}
        },
        Ev::End(q) => {
            if local(&q) == "page-layout" {
                cur = None;
            }
        }
        Ev::Text(_) => {}
    });
    let mut pages = Pages::default();
    for (name, layout) in masters {
        let Some((a, cols)) = layouts.get(&layout) else { continue };
        let mut sec = SectionProps::default();
        apply_layout(a, &mut sec);
        // A one-column layout keeps the default columns (and their gap).
        if let Some(c) = cols.as_ref().filter(|c| c.count > 1) {
            sec.columns = c.clone();
        }
        if pages.first.is_none() {
            pages.first = Some(sec.clone());
        }
        pages.masters.entry(name).or_insert(sec);
    }
    pages
}

/// Replace `sec` with the page setup of the first master page in `styles.xml`, if any.
#[cfg(test)]
fn apply_page_layout(xml: &[u8], sec: &mut SectionProps) {
    if let Some(first) = read_pages(xml).first {
        *sec = first;
    }
}

/// A `style:page-layout-properties` element's page size, orientation and margins into `sec`.
fn apply_layout(a: &Attrs, sec: &mut SectionProps) {
    let len = |k: &str| get(a, k).and_then(length).filter(|v| *v >= 0.0);
    // Like the DOCX reader: a page is at least half an inch; at most 22 inches (Word's limit), so a hostile
    // `1e30pt` can't reach layout.
    let page = |k: &str| len(k).filter(|v| *v >= 36.0).map(|v| v.min(1584.0));
    if let (Some(w), Some(h)) = (page("fo:page-width"), page("fo:page-height")) {
        sec.page_w = w;
        sec.page_h = h;
    }
    sec.landscape = match get(a, "style:print-orientation") {
        Some("landscape") => true,
        Some("portrait") => false,
        _ => sec.page_w > sec.page_h,
    };
    for (k, slot) in [
        ("fo:margin-top", &mut sec.margin_top),
        ("fo:margin-bottom", &mut sec.margin_bottom),
        ("fo:margin-left", &mut sec.margin_left),
        ("fo:margin-right", &mut sec.margin_right),
    ] {
        if let Some(v) = len(k) {
            // No margin is larger than the page's longer side (as `w:pgMar` in the DOCX reader).
            *slot = v.min(sec.page_w.max(sec.page_h));
        }
    }
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

    /// A package holding `entries`, deflated.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            z.start_file(*name, o).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    /// `bytes` (a package) with the size its central directory declares for `name` set to `size`.
    fn declare_size(mut bytes: Vec<u8>, name: &str, size: u32) -> Vec<u8> {
        let at = (0..bytes.len())
            .find(|&i| bytes[i..].starts_with(b"PK\x01\x02") && bytes.get(i + 46..).is_some_and(|r| r.starts_with(name.as_bytes())))
            .unwrap();
        bytes[at + 24..at + 28].copy_from_slice(&size.to_le_bytes());
        bytes
    }

    fn archive(bytes: &[u8]) -> zip::ZipArchive<Cursor<&[u8]>> {
        zip::ZipArchive::new(Cursor::new(bytes)).unwrap()
    }

    #[test]
    fn pictures_are_inflated_once_and_every_use_is_charged() {
        // Every frame on a picture inflated it again, even once the budget was used up.
        let a = zip_of(&[("p.png", &[1u8; 300])]);
        let b = zip_of(&[("p.png", &[2u8; 300]), ("q.png", &[3u8; 10])]);
        let (mut za, mut zb) = (archive(&a), archive(&b));
        let mut pics = Pictures::new(1000);
        assert_eq!(pics.get(&mut za, "p.png"), Some(vec![1u8; 300]));
        // Kept from the first read: the entry isn't read again (another package's would differ).
        assert_eq!(pics.get(&mut zb, "p.png"), Some(vec![1u8; 300]));
        assert_eq!(pics.get(&mut zb, "p.png"), Some(vec![1u8; 300]));
        assert_eq!(pics.room, 100);
        // A fourth copy doesn't fit; an entry that couldn't be read isn't looked up again.
        assert_eq!(pics.get(&mut za, "p.png"), None);
        assert_eq!(pics.get(&mut za, "q.png"), None);
        assert_eq!(pics.get(&mut zb, "q.png"), None);
        assert_eq!(pics.room, 100);
    }

    #[test]
    fn pictures_over_the_budget_are_not_inflated() {
        // An entry declaring more than is left is refused before inflating, and costs nothing.
        let pkg = zip_of(&[("big.png", &[0u8; 1200]), ("ok.png", &[5u8; 300])]);
        let mut z = archive(&pkg);
        let mut pics = Pictures::new(1000);
        assert_eq!(pics.get(&mut z, "big.png"), None);
        assert_eq!(pics.room, 1000);
        assert_eq!(pics.get(&mut z, "ok.png").map(|d| d.len()), Some(300));
        // One inflating to more than it declares stops a byte past the budget and uses it up:
        // nothing is inflated after it.
        let liar = declare_size(zip_of(&[("liar.png", &[0u8; 5000]), ("ok.png", &[5u8; 10])]), "liar.png", 10);
        let mut z = archive(&liar);
        let mut pics = Pictures::new(1000);
        assert_eq!(pics.get(&mut z, "liar.png"), None);
        assert_eq!(pics.room, 0);
        assert_eq!(pics.get(&mut z, "ok.png"), None);
    }

    #[test]
    fn frames_sharing_a_picture_all_show_it() {
        let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 300]].concat();
        let frames = r#"<draw:frame svg:width="1in" svg:height="1in"><draw:image xlink:href="Pictures/a.png"/></draw:frame>"#.repeat(3);
        let content = format!(
            r#"<office:document-content {NS_T}><office:body><office:text><text:p>{frames}</text:p></office:text></office:body></office:document-content>"#
        );
        let pkg = zip_of(&[("content.xml", content.as_bytes()), ("Pictures/a.png", &png)]);
        let flow = parse(&pkg).unwrap();
        let imgs: Vec<&model::Img> =
            paras(&flow).into_iter().flat_map(|p| &p.inlines).filter_map(|i| if let Inline::Image(img) = i { Some(img) } else { None }).collect();
        assert_eq!(imgs.len(), 3);
        assert!(imgs.iter().all(|i| i.data.as_slice() == png.as_slice()));
        assert_eq!(import(&pkg).unwrap().media.len(), 1);
    }

    const NS_T: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#;

    /// A package with `content.xml` and, when given, `styles.xml`.
    fn package(content: &str, styles: Option<&str>) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in
            [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content)].into_iter().chain(styles.map(|s| ("styles.xml", s)))
        {
            z.start_file(name, o).unwrap();
            z.write_all(data.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    fn paras(f: &Flow) -> Vec<&Para> {
        f.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None }).collect()
    }

    #[test]
    fn common_paragraph_styles_give_text_formatting() {
        // #436: a common paragraph style's text properties reach its paragraphs, directly,
        // through a derived common style and through an automatic style's parent.
        let props = r##"<style:text-properties fo:font-weight="bold" fo:color="#ff0000" fo:font-size="24pt"/>"##;
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Emphasis" style:family="paragraph" style:parent-style-name="Standard">{props}</style:style><style:style style:name="Derived" style:family="paragraph" style:parent-style-name="Emphasis"/></office:styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="Bridge" style:family="paragraph" style:parent-style-name="Emphasis"/></office:automatic-styles><office:body><office:text><text:p>Plain</text:p><text:p text:style-name="Emphasis">Common</text:p><text:p text:style-name="Derived">Derived</text:p><text:p text:style-name="Bridge">Bridge</text:p></office:text></office:body></office:document-content>"#
        );
        let f = parse(&package(&content, Some(&styles))).unwrap();
        let ps = paras(&f);
        assert_eq!(ps.len(), 4);
        let fmt = |p: &Para| p.inlines.iter().find_map(|i| if let Inline::Text(_, f) = i { Some(f.clone()) } else { None }).unwrap();
        assert_eq!(fmt(ps[0]), Fmt::default());
        for p in &ps[1..] {
            let f = fmt(p);
            assert!(f.bold && f.color == Some(Rgb(255, 0, 0)) && f.size == Some(24.0), "{}: {f:?}", p.text());
        }
    }

    #[test]
    fn text_after_a_drawing_shape_stays_in_its_paragraph() {
        // #1654: a shape's own text:p closed the paragraph it is anchored in, and the text after
        // the shape was dropped. Shapes are skipped like text boxes; the paragraph keeps its text.
        let draw =
            r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;
        let content = format!(
            r#"<office:document-content {NS_T} {draw}><office:body><office:text><text:p text:style-name="Heading"><draw:custom-shape svg:width="20mm" svg:height="20mm"><text:p>In the shape</text:p><draw:enhanced-geometry draw:type="rectangle"/></draw:custom-shape>Before <draw:rect><text:p/></draw:rect>middle <draw:g><draw:ellipse><text:p>In the group</text:p></draw:ellipse></draw:g>after</text:p><text:p>Next</text:p></office:text></office:body></office:document-content>"#
        );
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Heading" style:family="paragraph"><style:text-properties fo:font-weight="bold"/></style:style></office:styles></office:document-styles>"#
        );
        let f = parse(&package(&content, Some(&styles))).unwrap();
        let ps = paras(&f);
        assert_eq!(ps.iter().map(|p| p.text()).collect::<Vec<_>>(), ["Before middle after", "Next"]);
        assert!(ps[0].inlines.iter().all(|i| !matches!(i, Inline::Text(_, f) if !f.bold)), "{:?}", ps[0].inlines);
    }

    #[test]
    fn page_size_orientation_and_margins_round_trip() {
        // #810: the page setup written on export is read back on import.
        let mut doc = Document::new();
        doc.last_section.page_w = 841.0;
        doc.last_section.page_h = 595.0;
        doc.last_section.landscape = true;
        doc.last_section.margin_left = 36.0;
        doc.last_section.margin_top = 54.0;
        let back = import(&export(&doc).unwrap()).unwrap();
        let s = &back.last_section;
        assert!(s.landscape);
        assert_eq!((s.page_w, s.page_h), (841.0, 595.0));
        assert_eq!((s.margin_left, s.margin_top), (36.0, 54.0));
    }

    #[test]
    fn page_layout_follows_the_first_master_page() {
        let xml = br#"<office:document-styles><office:automatic-styles>
            <style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout>
            <style:page-layout style:name="pm2">
            <style:page-layout-properties fo:page-width="10in" fo:page-height="5in" fo:margin-left="1in" style:print-orientation="landscape"/>
            </style:page-layout>
            </office:automatic-styles><office:master-styles>
            <style:master-page style:name="Standard" style:page-layout-name="pm2"/>
            <style:master-page style:name="Other" style:page-layout-name="pm1"/>
            </office:master-styles></office:document-styles>"#;
        let mut sec = wordcraft_doc::SectionProps::default();
        apply_page_layout(xml, &mut sec);
        assert!(sec.landscape);
        assert_eq!((sec.page_w, sec.page_h, sec.margin_left, sec.margin_top), (720.0, 360.0, 72.0, 72.0));
    }

    #[test]
    fn hostile_page_layout_values_are_clamped() {
        let styles = |w: &str, h: &str, m: &str| {
            format!(
                r#"<office:document-styles><office:automatic-styles><style:page-layout style:name="pm1">
                <style:page-layout-properties fo:page-width="{w}" fo:page-height="{h}" fo:margin-top="{m}"
                fo:margin-bottom="{m}" fo:margin-left="{m}" fo:margin-right="{m}"/></style:page-layout>
                </office:automatic-styles><office:master-styles>
                <style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
                </office:document-styles>"#
            )
        };
        // Huge values: the page is capped at 22 inches and margins at the page's longer side.
        let mut sec = wordcraft_doc::SectionProps::default();
        apply_page_layout(styles("1e30pt", "1e30in", "1e30pt").as_bytes(), &mut sec);
        assert_eq!((sec.page_w, sec.page_h), (1584.0, 1584.0));
        assert_eq!((sec.margin_top, sec.margin_left), (1584.0, 1584.0));
        // Negative, zero, tiny and non-numeric values leave the defaults alone.
        let def = wordcraft_doc::SectionProps::default();
        for (w, h, m) in [("-10in", "-5in", "-1in"), ("0pt", "0pt", "NaNpt"), ("1pt", "2pt", "infpt"), ("NaN", "inf", "x")] {
            let mut sec = wordcraft_doc::SectionProps::default();
            apply_page_layout(styles(w, h, m).as_bytes(), &mut sec);
            assert_eq!((sec.page_w, sec.page_h), (def.page_w, def.page_h), "{w} {h}");
            assert_eq!((sec.margin_top, sec.margin_right), (def.margin_top, def.margin_right), "{m}");
            assert!(sec.page_w.is_finite() && sec.margin_left.is_finite());
        }
    }

    #[test]
    fn table_cell_style_background_is_shading() {
        // #437: `fo:background-color` on `style:table-cell-properties` is the cell's shading.
        let content = format!(
            r##"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="RedCell" style:family="table-cell"><style:table-cell-properties fo:background-color="#ff0000"/></style:style></office:automatic-styles><office:body><office:text><table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell table:style-name="RedCell"><text:p>Red</text:p></table:table-cell><table:table-cell><text:p>Plain</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"##
        );
        let f = parse(&package(&content, None)).unwrap();
        let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
        let row: Vec<_> = t.rows[0].iter().map(|c| (c.text(), c.shading)).collect();
        assert_eq!(row, vec![("Red".to_string(), Some(Rgb(255, 0, 0))), ("Plain".to_string(), None)]);
    }

    #[test]
    fn table_cell_style_vertical_alignment() {
        let content = format!(
            r##"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="MidCell" style:family="table-cell"><style:table-cell-properties style:vertical-align="middle"/></style:style><style:style style:name="BotCell" style:family="table-cell"><style:table-cell-properties style:vertical-align="bottom"/></style:style></office:automatic-styles><office:body><office:text><table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell table:style-name="MidCell"><text:p>Mid</text:p></table:table-cell><table:table-cell table:style-name="BotCell"><text:p>Bot</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>"##
        );
        let f = parse(&package(&content, None)).unwrap();
        let Some(FBlock::Table(t)) = f.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table") };
        assert_eq!(t.rows[0][0].valign, Some(VAlign::Center));
        assert_eq!(t.rows[0][1].valign, Some(VAlign::Bottom));
        let doc = crate::model::to_doc(&f);
        let bytes = export(&doc).unwrap();
        let f2 = parse(&bytes).unwrap();
        let Some(FBlock::Table(t2)) = f2.blocks.iter().find(|b| matches!(b, FBlock::Table(_))) else { panic!("no table in export") };
        assert_eq!(t2.rows[0][0].valign, Some(VAlign::Center));
        assert_eq!(t2.rows[0][1].valign, Some(VAlign::Bottom));
    }

    /// #1379: three master pages (A4 upright; A4 wide with page numbering restarting at 1; A4
    /// upright again with numbering continuing) and a two-column `text:section` in between.
    fn sections_package() -> Vec<u8> {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:automatic-styles>
            <style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" style:print-orientation="portrait" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2.5cm" fo:margin-right="2.5cm"/></style:page-layout>
            <style:page-layout style:name="pW"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" style:print-orientation="landscape" fo:margin-top="1.5cm" fo:margin-bottom="1.5cm" fo:margin-left="1.5cm" fo:margin-right="1.5cm"/></style:page-layout>
            </office:automatic-styles><office:master-styles>
            <style:master-page style:name="Upright" style:page-layout-name="pU"/>
            <style:master-page style:name="Wide" style:page-layout-name="pW"/>
            <style:master-page style:name="Closing" style:page-layout-name="pU"/>
            </office:master-styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles>
            <style:style style:name="PStart" style:family="paragraph" style:master-page-name="Upright"/>
            <style:style style:name="PWide" style:family="paragraph" style:master-page-name="Wide"><style:paragraph-properties fo:break-before="page" style:page-number="1"/></style:style>
            <style:style style:name="PClose" style:family="paragraph" style:master-page-name="Closing"><style:paragraph-properties fo:break-before="page" style:page-number="auto"/></style:style>
            <style:style style:name="Two" style:family="section"><style:section-properties><style:columns fo:column-count="2" fo:column-gap="1cm"><style:column-sep style:width="0.02cm" style:style="solid"/></style:columns></style:section-properties></style:style>
            </office:automatic-styles><office:body><office:text>
            <text:p text:style-name="PStart">Opening words</text:p><text:p>More opening words</text:p>
            <text:h text:style-name="PWide" text:outline-level="1">Wide part</text:h>
            <table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell><text:p>A</text:p></table:table-cell><table:table-cell><text:p>B</text:p></table:table-cell></table:table-row></table:table>
            <text:section text:name="Cols" text:style-name="Two"><text:p>Left column text</text:p><text:p>Right column text</text:p></text:section>
            <text:p text:style-name="PClose">Closing words</text:p><text:p>Last line</text:p>
            </office:text></office:body></office:document-content>"#
        );
        package(&content, Some(&styles))
    }

    #[test]
    fn master_pages_and_column_sections_become_sections() {
        use wordcraft_doc::section::SectionStart;
        let doc = import(&sections_package()).unwrap();
        let secs = doc.sections();
        let summary: Vec<_> =
            secs.iter().map(|(_, s)| (s.start, s.landscape, s.page_w.round(), s.page_h.round(), s.columns.count, s.page_num_start)).collect();
        assert_eq!(
            summary,
            [
                (SectionStart::NextPage, false, 595.0, 842.0, 1, None),
                (SectionStart::NextPage, true, 842.0, 595.0, 1, Some(1)),
                (SectionStart::Continuous, true, 842.0, 595.0, 2, None),
                (SectionStart::NextPage, false, 595.0, 842.0, 1, None),
            ]
        );
        // Margins come from each section's page layout; the columns from the section style.
        assert!((secs[0].1.margin_left - 70.87).abs() < 0.01 && (secs[1].1.margin_left - 42.52).abs() < 0.01);
        let cols = &secs[2].1.columns;
        assert!((cols.space - 28.35).abs() < 0.01 && cols.separator, "{cols:?}");
        // Each section ends on its own last paragraph; the wide one ends after its table.
        let text = |i: usize| match doc.body.get(i).map(|b| &**b) {
            Some(wordcraft_doc::Block::Para(p)) => p.plain_text().to_string(),
            _ => String::from("<not a paragraph>"),
        };
        assert_eq!(text(secs[0].0), "More opening words");
        assert!(matches!(doc.body.get(secs[1].0 - 1).map(|b| &**b), Some(wordcraft_doc::Block::Table(_))));
        assert_eq!(text(secs[2].0), "Right column text");
        assert_eq!(text(secs[3].0), "Last line");
    }

    #[test]
    fn one_master_page_stays_one_section() {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pU"/></office:master-styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="Standard"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">One</text:p><text:p>Two</text:p></office:text></office:body></office:document-content>"#
        );
        let doc = import(&package(&content, Some(&styles))).unwrap();
        assert_eq!(doc.sections().len(), 1);
        assert_eq!(doc.last_section.page_w.round(), 595.0);
    }

    #[test]
    fn hostile_sections_are_capped_and_clamped() {
        // 20,000 master-page paragraphs give at most 10,000 sections; absurd column counts and
        // gaps are clamped; a column section at the very end leaves no empty section after it.
        let mut body = String::new();
        for _ in 0..20_000 {
            body.push_str(r#"<text:p text:style-name="PM">x</text:p>"#);
        }
        body.push_str(r#"<text:section text:style-name="Many"><text:p>cols</text:p></text:section>"#);
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="PM" style:family="paragraph" style:master-page-name="Standard"/><style:style style:name="Many" style:family="section"><style:section-properties><style:columns fo:column-count="99999999999" fo:column-gap="1e30in"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
        );
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pU"/></office:master-styles></office:document-styles>"#
        );
        let doc = import(&package(&content, Some(&styles))).unwrap();
        assert_eq!(doc.sections().len(), 10_000);
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="Many" style:family="section"><style:section-properties><style:columns fo:column-count="1000" fo:column-gap="1e30in"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text><text:p>before</text:p><text:section text:style-name="Many"><text:p>cols</text:p></text:section></office:text></office:body></office:document-content>"#
        );
        let doc = import(&package(&content, None)).unwrap();
        let secs = doc.sections();
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[1].1.columns.count, 45);
        assert!(secs[1].1.columns.space <= 1584.0);
    }

    #[test]
    fn master_page_on_a_table_style_starts_a_section() {
        // Review of #1379: a table style can name a master page too (ODF 1.3 §19.509), with its
        // page-number restart on `style:table-properties`. An empty name on a child style means
        // "no page style here", and a page-number restart without a master page starts nothing.
        use wordcraft_doc::section::SectionStart;
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:styles><style:style style:name="WideParent" style:family="paragraph" style:master-page-name="Wide"/></office:styles><office:automatic-styles>
            <style:page-layout style:name="pU"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm"><style:columns fo:column-count="1" fo:column-gap="0cm"/></style:page-layout-properties></style:page-layout>
            <style:page-layout style:name="pW"><style:page-layout-properties fo:page-width="29.7cm" fo:page-height="21cm" style:print-orientation="landscape"/></style:page-layout>
            </office:automatic-styles><office:master-styles>
            <style:master-page style:name="Upright" style:page-layout-name="pU"/><style:master-page style:name="Wide" style:page-layout-name="pW"/>
            </office:master-styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles>
            <style:style style:name="NoPage" style:family="paragraph" style:parent-style-name="WideParent" style:master-page-name=""/>
            <style:style style:name="Restart" style:family="paragraph"><style:paragraph-properties fo:break-before="page" style:page-number="3"/></style:style>
            <style:style style:name="WideTable" style:family="table" style:master-page-name="Wide"><style:table-properties style:page-number="1"/></style:style>
            </office:automatic-styles><office:body><office:text>
            <text:p>Intro</text:p><text:p text:style-name="NoPage">Not a new page style</text:p><text:p text:style-name="Restart">Page break only</text:p>
            <table:table table:style-name="WideTable"><table:table-column/><table:table-row><table:table-cell><text:p>Cell</text:p></table:table-cell></table:table-row></table:table>
            <text:p>After</text:p>
            </office:text></office:body></office:document-content>"#
        );
        let doc = import(&package(&content, Some(&styles))).unwrap();
        let secs = doc.sections();
        let summary: Vec<_> = secs.iter().map(|(i, s)| (*i, s.start, s.landscape, s.page_num_start)).collect();
        assert_eq!(summary, [(2, SectionStart::NextPage, false, None), (4, SectionStart::NextPage, true, Some(1))]);
        assert!(matches!(doc.body.get(3).map(|b| &**b), Some(wordcraft_doc::Block::Table(_))));
        // A one-column page layout keeps the default column gap.
        assert_eq!(secs[0].1.columns, wordcraft_doc::section::Columns::default());
    }

    // --- Tracked changes (ODF 1.3 Part 3 §5.5) -------------------------------------------------

    const NS_TC: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/""#;

    /// An `.odt` whose body holds `regions` in `text:tracked-changes` and then `body`.
    fn tracked(regions: &str, body: &str) -> Vec<u8> {
        let content = format!(
            r#"<office:document-content {NS_TC}><office:body><office:text><text:tracked-changes text:track-changes="true">{regions}</text:tracked-changes>{body}</office:text></office:body></office:document-content>"#
        );
        package(&content, None)
    }

    fn region(id: &str, kind: &str, author: &str, date: &str, content: &str) -> String {
        format!(
            r#"<text:changed-region text:id="{id}"><text:{kind}><office:change-info><dc:creator>{author}</dc:creator><dc:date>{date}</dc:date><text:p>a remark on the change</text:p></office:change-info>{content}</text:{kind}></text:changed-region>"#
        )
    }

    /// Each body paragraph as (text, revision kind) runs, plus its mark's revision kind.
    type RevRuns = Vec<(Vec<(String, Option<wordcraft_doc::RevisionKind>)>, Option<wordcraft_doc::RevisionKind>)>;

    fn rev_runs(doc: &Document) -> RevRuns {
        let kind = |i: Option<u32>| i.and_then(|i| doc.revisions.get(i as usize)).map(|r| r.kind);
        doc.body
            .iter()
            .filter_map(|b| if let wordcraft_doc::Block::Para(p) = &**b { Some(p) } else { None })
            .map(|p| {
                let runs = p.run_ranges().map(|(r, cp)| (p.text[r].to_string(), kind(cp.del).or(kind(cp.ins)))).collect();
                (runs, kind(p.mark.del).or(kind(p.mark.ins)))
            })
            .collect()
    }

    fn insertion_and_deletion() -> Vec<u8> {
        let regions = region("ct1", "insertion", "Alice", "2024-03-01T10:00:00", "")
            + &region("ct2", "deletion", "Bob", "2024-03-02T11:30:00", "<text:p>old<text:s/></text:p>");
        tracked(
            &regions,
            r#"<text:p>Keep <text:change-start text:change-id="ct1"/>new <text:change-end text:change-id="ct1"/><text:change text:change-id="ct2"/>words.</text:p>"#,
        )
    }

    #[test]
    fn tracked_insertion_and_deletion_import_with_authors_and_dates() {
        use wordcraft_doc::RevisionKind::{Delete, Insert};
        let doc = import(&insertion_and_deletion()).unwrap();
        let s = |v: &str| v.to_string();
        assert_eq!(rev_runs(&doc), vec![(vec![(s("Keep "), None), (s("new "), Some(Insert)), (s("old "), Some(Delete)), (s("words."), None)], None)]);
        let revs: Vec<_> = doc.revisions.iter().map(|r| (r.kind, r.author.as_str(), r.date.as_str())).collect();
        assert_eq!(revs, vec![(Insert, "Alice", "2024-03-01T10:00:00"), (Delete, "Bob", "2024-03-02T11:30:00")]);
        assert!(doc.settings.track_changes, "text:track-changes=\"true\" turns tracking on");
    }

    #[test]
    fn tracked_deletion_across_paragraphs_keeps_the_deleted_paragraph_mark() {
        use wordcraft_doc::RevisionKind::Delete;
        // "First tail¶head rest." with "tail¶head " deleted: the body holds the merged paragraph.
        let regions = region("d1", "deletion", "Bob", "2024-03-02T11:30:00", "<text:p>tail</text:p><text:p>head<text:s/></text:p>");
        let doc = import(&tracked(&regions, r#"<text:p>First <text:change text:change-id="d1"/>rest.</text:p>"#)).unwrap();
        let s = |v: &str| v.to_string();
        assert_eq!(
            rev_runs(&doc),
            vec![(vec![(s("First "), None), (s("tail"), Some(Delete))], Some(Delete)), (vec![(s("head "), Some(Delete)), (s("rest."), None)], None),]
        );
    }

    #[test]
    fn tracked_changes_round_trip_through_export() {
        let regions = region("ct1", "insertion", "Alice", "2024-03-01T10:00:00", "")
            + &region("d1", "deletion", "Bob", "2024-03-02T11:30:00", "<text:p>tail</text:p><text:p></text:p><text:p>head<text:s/></text:p>");
        let body = r#"<text:p>First <text:change text:change-id="d1"/>rest <text:change-start text:change-id="ct1"/>added</text:p><text:p>whole new<text:change-end text:change-id="ct1"/></text:p><text:p>End.</text:p>"#;
        let doc = import(&tracked(&regions, body)).unwrap();
        let back = import(&export(&doc).unwrap()).unwrap();
        assert_eq!(rev_runs(&back), rev_runs(&doc));
        let revs = |d: &Document| d.revisions.iter().map(|r| (r.kind, r.author.clone(), r.date.clone())).collect::<Vec<_>>();
        assert_eq!(revs(&back), revs(&doc));
        assert_eq!(back.settings.track_changes, doc.settings.track_changes);
        // The inserted paragraph mark survives: "added¶" is part of the insertion.
        assert_eq!(rev_runs(&doc)[2].1, Some(wordcraft_doc::RevisionKind::Insert));
    }

    #[test]
    fn hostile_tracked_changes_are_bounded() {
        // Unknown and duplicate ids, a change-end without a start, nested regions, a deletion
        // inside a deletion, a change mark inside the deleted text and tracked changes inside a paragraph.
        let regions = region("d1", "deletion", "B", "x", r#"<text:p>gone<text:change text:change-id="d1"/></text:p>"#)
            + r#"<text:changed-region text:id="n"><text:changed-region text:id="m"><text:deletion><text:p>inner</text:p></text:deletion></text:changed-region></text:changed-region>"#
            + &region("dd", "deletion", "B", "x", "<text:deletion><text:p>deeper</text:p></text:deletion><text:p>ok</text:p>")
            + &region("d1", "insertion", "C", "y", "");
        let body = r#"<text:p>a<text:change-end text:change-id="zz"/><text:change text:change-id="d1"/><text:change text:change-id="d1"/><text:change-start text:change-id="nope"/>b<text:change text:change-id="m"/><text:change text:change-id="dd"/><text:tracked-changes><text:changed-region text:id="q"/></text:tracked-changes>c</text:p>"#;
        let doc = import(&tracked(&regions, body)).unwrap();
        let text: String = rev_runs(&doc).iter().flat_map(|(r, _)| r.iter().map(|(t, _)| t.clone())).collect();
        assert_eq!(text.matches("gone").count(), 1, "a deletion is placed once: {text}");
        assert!(!text.contains("deeper"), "{text}");
        assert!(text.starts_with('a') && text.ends_with('c'), "{text}");

        // A huge number of regions is capped.
        let many: String = (0..20_000).map(|i| format!(r#"<text:changed-region text:id="r{i}"><text:insertion/></text:changed-region>"#)).collect();
        let starts: String = (0..20_000).map(|i| format!(r#"<text:change-start text:change-id="r{i}"/>x"#)).collect();
        let doc = import(&tracked(&many, &format!("<text:p>{starts}</text:p>"))).unwrap();
        assert!(doc.revisions.len() <= 10_000, "{}", doc.revisions.len());
    }

    /// Every paragraph (body and table cells, in order) as (text, inserted, deleted) runs plus
    /// its mark's (inserted, deleted).
    type Marks = Vec<(Vec<(String, bool, bool)>, (bool, bool))>;

    fn marks(doc: &Document) -> Marks {
        fn walk(bl: &wordcraft_doc::Blocks, out: &mut Marks) {
            for b in bl {
                match &**b {
                    wordcraft_doc::Block::Para(p) => out.push((
                        p.run_ranges().map(|(r, cp)| (p.text[r].to_string(), cp.ins.is_some(), cp.del.is_some())).collect(),
                        (p.mark.ins.is_some(), p.mark.del.is_some()),
                    )),
                    wordcraft_doc::Block::Table(t) => {
                        for r in &t.rows {
                            for c in &r.cells {
                                walk(&c.blocks, out);
                            }
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&doc.body, &mut out);
        out
    }

    #[test]
    fn inserted_then_deleted_text_keeps_both_marks() {
        // "new" was inserted by Alice, then Bob deleted "old new": the deletion's content holds
        // the insertion as a change range.
        let regions = region("i1", "insertion", "Alice", "2024-03-01T10:00:00", "")
            + &region(
                "d1",
                "deletion",
                "Bob",
                "2024-03-02T11:30:00",
                r#"<text:p>old <text:change-start text:change-id="i1"/>new<text:change-end text:change-id="i1"/></text:p>"#,
            );
        let doc = import(&tracked(&regions, r#"<text:p>Keep <text:change text:change-id="d1"/>end.</text:p>"#)).unwrap();
        let s = |v: &str| v.to_string();
        let want =
            vec![(vec![(s("Keep "), false, false), (s("old "), false, true), (s("new"), true, true), (s("end."), false, false)], (false, false))];
        assert_eq!(marks(&doc), want);
        let back = import(&export(&doc).unwrap()).unwrap();
        assert_eq!(marks(&back), want, "both marks survive a save");
        assert_eq!(back.revisions.len(), doc.revisions.len(), "one deletion stays one region and one revision: {:?}", back.revisions);
    }

    #[test]
    fn a_deleted_mark_inside_an_insertion_is_not_inserted() {
        // "A" inserted by Alice, then a paragraph break Bob deleted, then "C" inserted by Alice:
        // the insertion range runs across the deletion point, but the deleted mark wasn't inserted.
        let regions = region("i1", "insertion", "Alice", "2024-03-01T10:00:00", "")
            + &region("d1", "deletion", "Bob", "2024-03-02T11:30:00", "<text:p></text:p><text:p></text:p>");
        let body =
            r#"<text:p><text:change-start text:change-id="i1"/>A<text:change text:change-id="d1"/>C<text:change-end text:change-id="i1"/></text:p>"#;
        let doc = import(&tracked(&regions, body)).unwrap();
        let s = |v: &str| v.to_string();
        let want = vec![(vec![(s("A"), true, false)], (false, true)), (vec![(s("C"), true, false)], (false, false))];
        assert_eq!(marks(&doc), want);
        let back = import(&export(&doc).unwrap()).unwrap();
        assert_eq!(marks(&back), want, "after a save");
    }

    #[test]
    fn insertions_end_before_a_table_and_inside_a_cell() {
        let regions = region("i1", "insertion", "Alice", "2024-03-01T10:00:00", "") + &region("i2", "insertion", "Bob", "2024-03-02T10:00:00", "");
        let table = r#"<table:table><table:table-column/><table:table-row><table:table-cell><text:p>Cell <text:change-start text:change-id="i2"/>added</text:p><text:change-end text:change-id="i2"/></table:table-cell></table:table-row></table:table>"#;
        let body = format!(
            r#"<text:p>Start</text:p><text:p><text:change-start text:change-id="i1"/>New paragraph</text:p><text:change-end text:change-id="i1"/>{table}<text:p>End.</text:p>"#
        );
        let content = format!(
            r#"<office:document-content {NS_TC} xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:text><text:tracked-changes>{regions}</text:tracked-changes>{body}</office:text></office:body></office:document-content>"#
        );
        let doc = import(&package(&content, None)).unwrap();
        let s = |v: &str| v.to_string();
        let want = vec![
            (vec![(s("Start"), false, false)], (false, false)),
            (vec![(s("New paragraph"), true, false)], (true, false)),
            (vec![(s("Cell "), false, false), (s("added"), true, false)], (true, false)),
            (vec![(s("End."), false, false)], (false, false)),
        ];
        assert_eq!(marks(&doc), want);
        let back = import(&export(&doc).unwrap()).unwrap();
        assert_eq!(marks(&back), want);
        let authors = |d: &Document| d.revisions.iter().map(|r| r.author.clone()).collect::<Vec<_>>();
        assert_eq!(authors(&back), authors(&doc));
    }

    #[test]
    fn exported_changes_always_have_a_date_and_keep_paragraph_separators() {
        // A change without a date gets one on save (`dc:date` is required); a U+2029 in deleted
        // text is text, not a paragraph break.
        let regions = r#"<text:changed-region text:id="d1"><text:deletion><office:change-info><dc:creator>Bob</dc:creator></office:change-info><text:p>a&#x2029;b</text:p></text:deletion></text:changed-region>"#;
        let doc = import(&tracked(regions, r#"<text:p>x<text:change text:change-id="d1"/>y</text:p>"#)).unwrap();
        assert_eq!(doc.revisions.first().map(|r| r.date.as_str()), Some(""));
        let back = import(&export(&doc).unwrap()).unwrap();
        assert!(back.revisions.first().is_some_and(|r| r.date.len() >= 19 && r.date.starts_with("20")), "{:?}", back.revisions);
        assert_eq!(marks(&back), marks(&doc));
        assert_eq!(marks(&back).len(), 1);
    }
}
