//! Hangul word processor documents, HWPX (`.hwpx`), read-only.
//!
//! Written from the published OWPML schema (KS X 6101, Open Word-Processor Markup Language): a
//! ZIP package whose `mimetype` is `application/hwp+zip`, `Contents/content.hpf` lists the parts
//! (sections in reading order in its spine), `Contents/header.xml` holds the fonts, border fills,
//! character properties (`hh:charPr`) and paragraph properties (`hh:paraPr`) that the sections'
//! paragraphs (`hp:p`), runs (`hp:run`), tables (`hp:tbl`) and cells (`hp:tc`) refer to by id,
//! and `BinData/` the pictures.
//!
//! Read: the same as [`crate::hwp`] — text, character and paragraph formatting, tables with
//! merged cells and cell borders, page setup and columns, pictures, headers, footers, page
//! numbers, footnotes and endnotes, and the text of text boxes. Encrypted packages are refused
//! with a clear error. Bounded: entry sizes, XML depth, nesting, parts and picture bytes are
//! capped, so hostile input gives an error or a best-effort document, never a panic.

use std::collections::HashMap;
use std::io::Cursor;

use wordcraft_doc::para::{Float, InlineObject, LINE_BREAK, NBSP, NoteKind, SOFT_HYPHEN};
use wordcraft_doc::props::{Borders, CharProps, TabStop, TableProps, VAlign};
use wordcraft_doc::section::Columns;
use wordcraft_doc::{Align, Blocks, Document, PartKind, SectionProps, Table};

use crate::hangul::{self, BorderFill, MAX_DEPTH, MAX_MEDIA, MAX_PARTS, PageNum, ParaBuf, ParaShape, RawCell, RawChar, RawLine, RawPara, Story, pt};
use crate::odt::{Attrs, Ev, Pictures, events, get, local, read_entry};

const MIME: &str = "application/hwp+zip";
/// Deepest XML element kept (deeper ones are skipped).
const MAX_XML_DEPTH: usize = 256;
/// Most sections read.
const MAX_SECTIONS: usize = 1000;

type Zip<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

/// Is this a HWPX package (a ZIP whose `mimetype` is `application/hwp+zip`)?
pub fn is_hwpx(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        && zip::ZipArchive::new(Cursor::new(bytes))
            .ok()
            .and_then(|mut z| read_entry(&mut z, "mimetype"))
            .is_some_and(|m| m.trim_ascii() == MIME.as_bytes())
}

/// An XML element with its children.
#[derive(Default)]
struct El {
    /// Local name (no prefix).
    name: String,
    attrs: Attrs,
    kids: Vec<Node>,
}

enum Node {
    El(El),
    Text(String),
}

impl El {
    fn attr(&self, k: &str) -> Option<&str> {
        get(&self.attrs, k)
    }
    fn num(&self, k: &str) -> f64 {
        self.attr(k).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0)
    }
    fn id(&self, k: &str) -> Option<u32> {
        self.attr(k)?.trim().parse().ok()
    }
    fn els(&self) -> impl Iterator<Item = &El> {
        self.kids.iter().filter_map(|n| match n {
            Node::El(e) => Some(e),
            Node::Text(_) => None,
        })
    }
    fn child(&self, name: &str) -> Option<&El> {
        self.els().find(|e| e.name == name)
    }
    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> {
        self.els().filter(move |e| e.name == name)
    }
    /// The first element named `name` at or below this one (depth-first).
    fn find(&self, name: &str) -> Option<&El> {
        if self.name == name {
            return Some(self);
        }
        self.els().find_map(|e| e.find(name))
    }
}

/// Parse XML into a tree; elements past [`MAX_XML_DEPTH`] are dropped. `None` without a root.
fn tree(xml: &[u8]) -> Option<El> {
    let mut stack: Vec<El> = Vec::new();
    let mut root = None;
    let mut skip = 0usize;
    events(xml, |ev| match ev {
        Ev::Start(q, attrs) => {
            if skip > 0 || stack.len() >= MAX_XML_DEPTH {
                skip += 1;
            } else {
                stack.push(El { name: local(&q).to_string(), attrs, kids: Vec::new() });
            }
        }
        Ev::End(_) => {
            if skip > 0 {
                skip -= 1;
            } else if let Some(e) = stack.pop() {
                match stack.last_mut() {
                    Some(parent) => parent.kids.push(Node::El(e)),
                    None => {
                        if root.is_none() {
                            root = Some(e);
                        }
                    }
                }
            }
        }
        Ev::Text(t) => {
            if skip == 0
                && let Some(top) = stack.last_mut()
            {
                top.kids.push(Node::Text(t));
            }
        }
    });
    // Unclosed elements at the end of a truncated file still count.
    while let Some(e) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.kids.push(Node::El(e)),
            None => root = root.or(Some(e)),
        }
    }
    root
}

/// What `header.xml` gives the sections, by id.
#[derive(Default)]
struct Header {
    chars: HashMap<u32, CharProps>,
    paras: HashMap<u32, ParaShape>,
    fills: HashMap<u32, BorderFill>,
}

/// A tab definition (`hh:tabPr`): its stops, from the HwpUnitChar case (HWPUNIT) or else the
/// default branch or plain items (twice that).
fn tab_pr(e: &El) -> Vec<TabStop> {
    let switch = e.child("switch");
    let (src, div) = match (switch.and_then(|s| s.child("case")), switch.and_then(|s| s.child("default"))) {
        (Some(c), _) => (c, 1.0),
        (None, Some(d)) => (d, 2.0),
        (None, None) => (e, 2.0),
    };
    src.children("tabItem")
        .map(|t| {
            let kind = match t.attr("type") {
                Some("RIGHT") => 1,
                Some("CENTER") => 2,
                Some("DECIMAL") => 3,
                _ => 0,
            };
            hangul::tab(pt(t.num("pos") / div), kind, hangul::line_code(t.attr("leader").unwrap_or("NONE")))
        })
        .collect()
}

fn border_fill(e: &El) -> BorderFill {
    let side = |name: &str| {
        let b = e.child(name)?;
        let code = hangul::line_code(b.attr("type").unwrap_or("NONE"));
        Some(BorderFill::side(code, hangul::width_mm_pt(b.attr("width").unwrap_or("0.12")), b.attr("color").and_then(hangul::hex_color)))
    };
    let fill = e.find("winBrush").and_then(|w| w.attr("faceColor")).and_then(hangul::hex_color).filter(|c| *c != wordcraft_doc::Rgb(255, 255, 255));
    BorderFill {
        borders: Borders {
            top: side("topBorder"),
            left: side("leftBorder"),
            bottom: side("bottomBorder"),
            right: side("rightBorder"),
            between: None,
            inside_v: None,
        },
        fill,
    }
}

fn char_pr(e: &El, fonts: &[HashMap<u32, String>; 2]) -> RawChar {
    let refs = e.child("fontRef");
    let face = |g: usize, k: &str| refs.and_then(|r| r.id(k)).and_then(|id| fonts.get(g)?.get(&id).cloned());
    let lang = |name: &str| e.child(name).and_then(|c| c.attr("hangul")).and_then(|v| v.trim().parse::<i32>().ok());
    let has = |name: &str| e.child(name).is_some();
    let typed = |name: &str, k: &str| e.child(name).and_then(|c| c.attr(k)).is_some_and(|v| !v.is_empty() && v != "NONE");
    let underline = e.child("underline").filter(|u| matches!(u.attr("type"), Some("BOTTOM" | "TOP")));
    let strike = e.child("strikeout").and_then(|s| s.attr("shape")).is_some_and(|s| !matches!(s, "" | "NONE" | "3D"));
    RawChar {
        hangul: face(0, "hangul"),
        latin: face(1, "latin"),
        height: e.num("height").clamp(0.0, 1e6) as i32,
        ratio: lang("ratio").unwrap_or(100),
        spacing: lang("spacing").unwrap_or(0),
        bold: has("bold"),
        italic: has("italic"),
        underline: underline.map(|u| hangul::line_code(u.attr("shape").unwrap_or("SOLID")).saturating_sub(1)),
        strike,
        sup: has("supscript"),
        sub: has("subscript"),
        color: e.attr("textColor").and_then(hangul::hex_color),
        shade: e.attr("shadeColor").and_then(hangul::hex_color),
        outline: typed("outline", "type"),
        shadow: typed("shadow", "type"),
        emboss: has("emboss"),
        engrave: has("engrave"),
    }
}

/// A paragraph property. `physical`: the package (OWPML 1.4 and later) gives plain and
/// `HwpUnitChar` lengths in HWPUNIT; the `default` branch of a switch, and every length of
/// older packages, are at twice that, like the binary format.
fn para_pr(e: &El, fills: &HashMap<u32, BorderFill>, tabs: &HashMap<u32, Vec<TabStop>>, physical: bool) -> RawPara {
    // The margins and line spacing: the HwpUnitChar case, else the default branch, else plain.
    let switch = e.child("switch");
    let case = switch.and_then(|s| s.child("case")).filter(|c| c.child("margin").is_some() || c.child("lineSpacing").is_some());
    let default = switch.and_then(|s| s.child("default"));
    let (src, div) = match (case, default) {
        (Some(c), _) => (c, if physical { 1.0 } else { 2.0 }),
        (None, Some(d)) => (d, 2.0),
        (None, None) => (e, if physical { 1.0 } else { 2.0 }),
    };
    let len = |name: &str| src.child("margin").and_then(|m| m.child(name)).map_or(0.0, |v| pt(v.num("value") / div));
    let line = src.child("lineSpacing").map_or(RawLine::Percent(160.0), |l| {
        let v = l.num("value");
        match l.attr("type").unwrap_or("PERCENT") {
            "FIXED" => RawLine::Fixed(pt(v / div)),
            "BETWEEN_LINES" => RawLine::Between(pt(v / div)),
            "AT_LEAST" | "MINIMUM" => RawLine::AtLeast(pt(v / div)),
            _ => RawLine::Percent(v as f32),
        }
    });
    let brk = e.child("breakSetting");
    let flag = |k: &str| brk.and_then(|b| b.attr(k)).is_some_and(|v| v == "1" || v == "true");
    RawPara {
        align: match e.child("align").and_then(|a| a.attr("horizontal")).unwrap_or("JUSTIFY") {
            "LEFT" => 1,
            "RIGHT" => 2,
            "CENTER" => 3,
            "DISTRIBUTE" | "DISTRIBUTE_SPACE" => 4,
            _ => 0,
        },
        left: len("left"),
        right: len("right"),
        indent: len("intent"),
        before: len("prev"),
        after: len("next"),
        line,
        keep_next: flag("keepWithNext"),
        keep_lines: flag("keepLines"),
        break_before: flag("pageBreakBefore"),
        widow: flag("widowOrphan"),
        border: e.child("border").and_then(|b| b.id("borderFillIDRef")).and_then(|id| fills.get(&id)).cloned(),
        tabs: e.id("tabPrIDRef").and_then(|id| tabs.get(&id)).cloned().unwrap_or_default(),
    }
}

fn header(xml: &[u8], physical: bool) -> Header {
    let mut h = Header::default();
    let Some(root) = tree(xml) else { return h };
    let Some(refs) = root.find("refList") else { return h };
    let mut fonts: [HashMap<u32, String>; 2] = Default::default();
    for ff in refs.child("fontfaces").into_iter().flat_map(|f| f.children("fontface")) {
        let g = match ff.attr("lang") {
            Some("HANGUL") => 0,
            Some("LATIN") => 1,
            _ => continue,
        };
        for f in ff.children("font") {
            if let (Some(id), Some(face)) = (f.id("id"), f.attr("face"))
                && let Some(map) = fonts.get_mut(g)
            {
                map.insert(id, face.to_string());
            }
        }
    }
    for bf in refs.child("borderFills").into_iter().flat_map(|b| b.children("borderFill")) {
        if let Some(id) = bf.id("id") {
            h.fills.insert(id, border_fill(bf));
        }
    }
    for c in refs.child("charProperties").into_iter().flat_map(|c| c.children("charPr")) {
        if let Some(id) = c.id("id") {
            h.chars.insert(id, char_pr(c, &fonts).props());
        }
    }
    let mut tabs = HashMap::new();
    for t in refs.child("tabProperties").into_iter().flat_map(|t| t.children("tabPr")) {
        if let Some(id) = t.id("id") {
            tabs.insert(id, tab_pr(t));
        }
    }
    for p in refs.child("paraProperties").into_iter().flat_map(|p| p.children("paraPr")) {
        if let Some(id) = p.id("id") {
            h.paras.insert(id, para_pr(p, &h.fills, &tabs, physical).shape());
        }
    }
    h
}

/// Does the package's `version.xml` say OWPML 1.4 or later (lengths in plain HWPUNIT)?
fn physical_units(xml: &[u8]) -> bool {
    let Some(v) = tree(xml) else { return true };
    let Some((major, minor)) = v.attr("xmlVersion").and_then(|s| s.split_once('.')) else { return true };
    (major.trim().parse::<u32>().unwrap_or(1), minor.trim().parse::<u32>().unwrap_or(4)) >= (1, 4)
}

/// The package's parts: manifest id → path, and the section paths in reading order.
fn package(z: &mut Zip<'_>) -> (HashMap<String, String>, Vec<String>) {
    let mut items = HashMap::new();
    let mut sections = Vec::new();
    if let Some(root) = read_entry(z, "Contents/content.hpf").and_then(|x| tree(&x)) {
        if let Some(m) = root.find("manifest") {
            for it in m.children("item") {
                if let (Some(id), Some(href)) = (it.attr("id"), it.attr("href")) {
                    items.insert(id.to_string(), href.trim_start_matches('/').to_string());
                }
            }
        }
        if let Some(s) = root.find("spine") {
            for r in s.children("itemref") {
                if let Some(href) = r.attr("idref").and_then(|id| items.get(id))
                    && href.to_ascii_lowercase().contains("section")
                {
                    sections.push(href.clone());
                }
            }
        }
    }
    if sections.is_empty() {
        // No spine: the section parts by number.
        let mut found: Vec<(u32, String)> = z
            .file_names()
            .filter_map(|n| {
                let k = n.strip_prefix("Contents/section")?.strip_suffix(".xml")?.parse().ok()?;
                Some((k, n.to_string()))
            })
            .collect();
        found.sort();
        sections = found.into_iter().map(|(_, n)| n).collect();
    }
    sections.truncate(MAX_SECTIONS);
    (items, sections)
}

/// Read a HWPX document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("HWPX: not a Hangul document ({e})"))?;
    let mime = read_entry(&mut z, "mimetype");
    if mime.as_deref().is_some_and(|m| m.trim_ascii() != MIME.as_bytes()) {
        return Err("HWPX: not a Hangul document (wrong mimetype)".into());
    }
    if read_entry(&mut z, "META-INF/manifest.xml").is_some_and(|m| m.windows(15).any(|w| w == b"encryption-data")) {
        return Err("HWPX: this document is protected with a password, which WordCraft can't open yet".into());
    }
    let header_xml = read_entry(&mut z, "Contents/header.xml").ok_or("HWPX: the package has no Contents/header.xml")?;
    let physical = read_entry(&mut z, "version.xml").is_none_or(|v| physical_units(&v));
    let hdr = header(&header_xml, physical);
    let (items, paths) = package(&mut z);
    let mut doc = Document::new();
    doc.body.clear();
    let mut pictures = Pictures::new(MAX_MEDIA);
    let mut media: HashMap<String, Option<String>> = HashMap::new();
    let mut sections = Vec::new();
    for path in paths {
        let Some(root) = read_entry(&mut z, &path).and_then(|x| tree(&x)) else { continue };
        let mut s = Sec {
            hdr: &hdr,
            doc: &mut doc,
            zip: &mut z,
            items: &items,
            pictures: &mut pictures,
            media: &mut media,
            sec: hangul::default_page(),
            cols_set: false,
            page_num: None,
        };
        let blocks = s.paras(&root, 0);
        let (mut sec, num) = (s.sec, s.page_num);
        hangul::finish_section(&mut doc, &mut sec, num);
        sections.push((blocks, sec));
    }
    if sections.is_empty() {
        return Err("HWPX: the package has no sections".into());
    }
    hangul::assemble(&mut doc, sections);
    doc.ensure_nonempty();
    Ok(doc)
}

/// The state of reading one section.
struct Sec<'a, 'z> {
    hdr: &'a Header,
    doc: &'a mut Document,
    zip: &'a mut Zip<'z>,
    items: &'a HashMap<String, String>,
    pictures: &'a mut Pictures,
    /// Manifest id → media key (`None`: unreadable).
    media: &'a mut HashMap<String, Option<String>>,
    sec: SectionProps,
    cols_set: bool,
    page_num: Option<PageNum>,
}

impl Sec<'_, '_> {
    /// The paragraphs (`hp:p`) directly in `parent`.
    fn paras(&mut self, parent: &El, depth: usize) -> Blocks {
        let mut story = Story::default();
        for p in parent.children("p") {
            self.paragraph(p, depth, &mut story);
        }
        story.finish()
    }

    /// The paragraphs of a control's `hp:subList`.
    fn sublist(&mut self, e: &El, depth: usize) -> Option<Blocks> {
        Some(self.paras(e.child("subList")?, depth))
    }

    fn paragraph(&mut self, p: &El, depth: usize, story: &mut Story) {
        let mut shape = p.id("paraPrIDRef").and_then(|id| self.hdr.paras.get(&id)).cloned().unwrap_or_else(|| RawPara::default().shape());
        if p.attr("pageBreak") == Some("1") {
            shape.props.page_break_before = Some(true);
        }
        let mut pb = ParaBuf::new(shape);
        let mut split = false;
        let mut after: Vec<Blocks> = Vec::new();
        let mut props = CharProps::default();
        for run in p.children("run") {
            props = run.id("charPrIDRef").and_then(|id| self.hdr.chars.get(&id)).cloned().unwrap_or_default();
            for e in run.els() {
                match e.name.as_str() {
                    "t" => text(e, &mut pb, &props),
                    "secPr" => self.sec_pr(e),
                    "ctrl" => {
                        for c in e.els() {
                            self.control(c, &mut pb, &props, depth);
                        }
                    }
                    "tbl" if depth < MAX_DEPTH => {
                        if let Some(t) = self.table(e, depth + 1) {
                            if !pb.is_empty() {
                                story.para(pb.finish());
                            }
                            story.table(t);
                            split = true;
                        }
                    }
                    "pic" => self.picture(e, &mut pb, &props),
                    "equation" | "linesegarray" => {}
                    // A drawing: its pictures show; a text box's paragraphs follow the paragraph.
                    _ if depth < MAX_DEPTH => {
                        if let Some(pic) = e.find("pic") {
                            self.picture(pic, &mut pb, &props);
                        } else if let Some(dt) = e.find("drawText")
                            && let Some(b) = self.sublist(dt, depth + 1)
                        {
                            after.push(b);
                        }
                    }
                    _ => {}
                }
            }
        }
        pb.set_mark(&props);
        if split && pb.is_empty() {
            story.tail(pb.finish());
        } else {
            story.para(pb.finish());
        }
        for b in after {
            story.extend(b);
        }
    }

    /// The section's page setup (`hp:secPr`).
    fn sec_pr(&mut self, e: &El) {
        let Some(pp) = e.child("pagePr") else { return };
        let m = pp.child("margin");
        let v = |k: &str| m.map_or(0.0, |m| m.num(k));
        let page = hangul::page(
            pp.num("width"),
            pp.num("height"),
            pp.attr("landscape") == Some("NARROWLY"),
            [v("left"), v("right"), v("top"), v("bottom"), v("header"), v("footer"), v("gutter")],
        );
        self.sec = SectionProps { columns: std::mem::take(&mut self.sec.columns), ..page };
    }

    /// A control in `hp:ctrl`.
    fn control(&mut self, c: &El, pb: &mut ParaBuf, props: &CharProps, depth: usize) {
        match c.name.as_str() {
            "colPr" if !self.cols_set => {
                self.cols_set = true;
                let count = c.num("colCount").clamp(1.0, 16.0) as u32;
                if count > 1 {
                    let gap = c.attr("sameGap").map_or(18.0, |_| pt(c.num("sameGap")).clamp(0.0, 200.0));
                    self.sec.columns = Columns { count, space: gap, ..Default::default() };
                }
            }
            "header" | "footer" if depth < MAX_DEPTH => {
                let even = c.attr("applyPageType") == Some("EVEN");
                if let Some(blocks) = self.sublist(c, depth + 1) {
                    hangul::add_header(self.doc, &mut self.sec, c.name == "footer", even, blocks);
                }
            }
            "footNote" | "endNote" if depth < MAX_DEPTH => {
                if let Some(blocks) = self.sublist(c, depth + 1)
                    && self.doc.parts.len() < MAX_PARTS
                {
                    let (kind, part) =
                        if c.name == "footNote" { (NoteKind::Footnote, PartKind::Footnote) } else { (NoteKind::Endnote, PartKind::Endnote) };
                    let id = self.doc.add_part(part, blocks);
                    pb.object(InlineObject::NoteRef { kind, id, custom: String::new() }, props);
                }
            }
            "autoNum" if c.attr("numType") == Some("PAGE") => pb.object(hangul::page_field(), props),
            "pageNum" => {
                let pos = match c.attr("pos").unwrap_or("") {
                    "TOP_LEFT" => 1,
                    "TOP_CENTER" => 2,
                    "TOP_RIGHT" => 3,
                    "BOTTOM_LEFT" => 4,
                    "BOTTOM_CENTER" => 5,
                    "BOTTOM_RIGHT" => 6,
                    "OUTSIDE_TOP" => 7,
                    "OUTSIDE_BOTTOM" => 8,
                    "INSIDE_TOP" => 9,
                    "INSIDE_BOTTOM" => 10,
                    _ => 0,
                };
                self.page_num = Some(PageNum { pos, side: c.attr("sideChar").and_then(|s| s.chars().next()) });
            }
            _ => {}
        }
    }

    fn table(&mut self, t: &El, depth: usize) -> Option<Table> {
        let m = |e: Option<&El>, k: &str| e.map_or(0.0, |e| pt(e.num(k)).clamp(0.0, 100.0));
        let inm = t.child("inMargin");
        let mut cells = Vec::new();
        for tr in t.children("tr") {
            for tc in tr.children("tc") {
                let addr = tc.child("cellAddr");
                let span = tc.child("cellSpan");
                let size = tc.child("cellSz");
                let n = |e: Option<&El>, k: &str, d: f64| e.map_or(d, |e| if e.attr(k).is_some() { e.num(k) } else { d }).clamp(0.0, 1e6) as usize;
                let sub = tc.child("subList");
                let valign = match sub.and_then(|s| s.attr("vertAlign")) {
                    Some("CENTER") => VAlign::Center,
                    Some("BOTTOM") => VAlign::Bottom,
                    _ => VAlign::Top,
                };
                let cm = tc.child("cellMargin");
                let margins = (tc.attr("hasMargin") == Some("1")).then(|| [m(cm, "top"), m(cm, "left"), m(cm, "bottom"), m(cm, "right")]);
                let fill = tc.id("borderFillIDRef").and_then(|id| self.hdr.fills.get(&id));
                cells.push(RawCell {
                    row: n(addr, "rowAddr", 0.0),
                    col: n(addr, "colAddr", 0.0),
                    rows: n(span, "rowSpan", 1.0),
                    cols: n(span, "colSpan", 1.0),
                    w: size.map_or(0.0, |s| pt(s.num("width"))),
                    h: size.map_or(0.0, |s| pt(s.num("height"))),
                    props: hangul::cell_props(fill, valign, margins),
                    blocks: sub.map(|s| self.paras(s, depth)).unwrap_or_default(),
                });
            }
        }
        let pos = t.child("pos");
        let align = if pos.and_then(|p| p.attr("treatAsChar")) != Some("1") {
            match pos.and_then(|p| p.attr("horzAlign")) {
                Some("CENTER") => Some(Align::Center),
                Some("RIGHT") => Some(Align::Right),
                _ => None,
            }
        } else {
            None
        };
        let props = TableProps {
            cell_margins: Some([m(inm, "top"), m(inm, "left"), m(inm, "bottom"), m(inm, "right")]),
            fixed: true,
            align,
            ..Default::default()
        };
        hangul::table(cells, props)
    }

    /// A picture (`hp:pic`): its image (`hc:img`) shown inline at the picture's size.
    fn picture(&mut self, e: &El, pb: &mut ParaBuf, props: &CharProps) {
        let Some(id) = e.find("img").and_then(|i| i.attr("binaryItemIDRef")) else { return };
        let key = match self.media.get(id) {
            Some(hit) => hit.clone(),
            None => {
                let key = self.load(id);
                self.media.insert(id.to_string(), key.clone());
                key
            }
        };
        let Some(media) = key else { return };
        let sz = e.child("sz").or_else(|| e.child("curSz"));
        let (mut w, mut h) = sz.map_or((0.0, 0.0), |s| (pt(s.num("width")), pt(s.num("height"))));
        if !(w >= 1.0 && h >= 1.0) {
            let px = self.doc.media.get(&media).and_then(|d| crate::model::image_px(d)).unwrap_or((96, 96));
            (w, h) = ((px.0 as f32 * 0.75).clamp(1.0, 2000.0), (px.1 as f32 * 0.75).clamp(1.0, 2000.0));
        }
        pb.object(InlineObject::Image { media, w, h, alt: String::new(), float: Float::default(), crop: [0.0; 4], ole: None }, props);
    }

    fn load(&mut self, id: &str) -> Option<String> {
        let path = self.items.get(id).cloned().unwrap_or_else(|| format!("BinData/{id}"));
        let data = self.pictures.get(self.zip, &path)?;
        let ext = crate::model::sniff_image(&data)?;
        Some(self.doc.add_media(data, ext))
    }
}

/// The text of `hp:t`: its characters and the special characters written as elements.
fn text(t: &El, pb: &mut ParaBuf, props: &CharProps) {
    for n in &t.kids {
        match n {
            Node::Text(s) => {
                let s: String = s.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
                pb.push(&s, props);
            }
            Node::El(e) => match e.name.as_str() {
                "tab" => pb.push_char('\t', props),
                "lineBreak" => pb.push_char(LINE_BREAK, props),
                "nbSpace" => pb.push_char(NBSP, props),
                "fwSpace" => pb.push_char('\u{2007}', props),
                "hyphen" | "softHyphen" => pb.push_char(SOFT_HYPHEN, props),
                _ => {}
            },
        }
    }
}
