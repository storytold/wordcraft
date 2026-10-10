//! A small "flow" model every text format maps to and from: paragraphs with a kind (heading,
//! quote, code…), list membership, alignment and formatted inline spans; tables of cells holding
//! blocks. [`to_doc`] and [`from_doc`] convert between it and the full [`Document`].

use std::sync::Arc;

use wordcraft_doc::numbering::{AbstractNum, Level, ListKind, Num, levels_for};
use wordcraft_doc::para::{InlineObject, OBJ, PAGE_BREAK};
use wordcraft_doc::props::{
    Border, BorderStyle, Borders, CellProps, CharProps, NumRef, ParaProps, RowProps, TextColor, Underline, VMerge, VertAlign,
};
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{Align, Block, Blocks, Document, Paragraph, Rgb, Table, para_block};

/// Font used for code spans and code blocks.
pub const MONO_FONT: &str = "Courier New";
/// Style id of code-block paragraphs.
pub const CODE_STYLE: &str = "Code";
/// Nesting limit for tables in tables (hostile input).
pub const MAX_DEPTH: usize = 8;

/// Character formatting the text formats can express.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fmt {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub sup: bool,
    pub sub: bool,
    pub link: Option<String>,
    pub color: Option<Rgb>,
    /// Points.
    pub size: Option<f32>,
    pub font: Option<String>,
    /// Background (highlight / shading).
    pub background: Option<Rgb>,
}

/// An embedded picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Img {
    pub data: Arc<Vec<u8>>,
    /// File extension (`png`, `jpeg`…).
    pub ext: String,
    /// Display size, points.
    pub w: f32,
    pub h: f32,
    pub alt: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String, Fmt),
    Image(Img),
    /// A bookmark (link target).
    Anchor(String),
    /// An equation in the linear format WordCraft keeps them in (`x=(-b±√(b^2-4ac))/2a`). Formats
    /// without equations write `linear` as text.
    Equation {
        linear: String,
        display: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Normal,
    /// 1..=6.
    Heading(u8),
    Title,
    Quote,
    /// One line of a code block.
    Code,
    /// A horizontal rule (empty paragraph with a bottom border).
    Rule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListInfo {
    pub ordered: bool,
    /// 0-based nesting level (0..=8).
    pub level: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Para {
    pub kind: Kind,
    pub list: Option<ListInfo>,
    pub align: Option<Align>,
    pub page_break: bool,
    pub inlines: Vec<Inline>,
}

impl Para {
    pub fn new(kind: Kind) -> Para {
        Para { kind, ..Default::default() }
    }
    /// Append text, merging with the previous span when the formatting matches.
    pub fn push_text(&mut self, s: &str, f: &Fmt) {
        if s.is_empty() {
            return;
        }
        if let Some(Inline::Text(t, pf)) = self.inlines.last_mut()
            && pf == f
        {
            t.push_str(s);
            return;
        }
        self.inlines.push(Inline::Text(s.to_string(), f.clone()));
    }
    /// The plain text (images and anchors left out; equations as their linear text).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for i in &self.inlines {
            match i {
                Inline::Text(t, _) | Inline::Equation { linear: t, .. } => s.push_str(t),
                Inline::Image(_) | Inline::Anchor(_) => {}
            }
        }
        s
    }
    pub fn is_empty(&self) -> bool {
        self.inlines.iter().all(|i| matches!(i, Inline::Text(t, _) if t.is_empty()))
    }

    /// Remove whitespace at the very start and end of the paragraph's text.
    pub fn trim(&mut self) {
        while let Some(Inline::Text(t, _)) = self.inlines.first_mut() {
            let n = t.trim_start_matches([' ', '\t', '\r', '\n']).len();
            let cut = t.len() - n;
            t.replace_range(..cut, "");
            if t.is_empty() {
                self.inlines.remove(0);
            } else {
                break;
            }
        }
        while let Some(Inline::Text(t, _)) = self.inlines.last_mut() {
            let n = t.trim_end_matches([' ', '\t', '\r', '\n']).len();
            t.truncate(n);
            if t.is_empty() {
                self.inlines.pop();
            } else {
                break;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub blocks: Vec<FBlock>,
    pub colspan: u32,
    /// Rows this cell spans (export information; `covered` cells below carry the merge).
    pub rowspan: u32,
    /// Continuation of a vertical merge from the cell above.
    pub covered: bool,
    pub header: bool,
    pub shading: Option<Rgb>,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { blocks: Vec::new(), colspan: 1, rowspan: 1, covered: false, header: false, shading: None }
    }
}

impl Cell {
    /// Text of the cell: paragraphs joined by `\n`.
    pub fn text(&self) -> String {
        let mut v = Vec::new();
        for b in &self.blocks {
            match b {
                FBlock::Para(p) => v.push(p.text()),
                FBlock::Table(t) => v.push(t.rows.iter().flat_map(|r| r.iter().map(Cell::text)).collect::<Vec<_>>().join(" ")),
            }
        }
        v.join("\n")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FTable {
    pub rows: Vec<Vec<Cell>>,
    /// Grid column widths in points (empty = equal widths).
    pub widths: Vec<f32>,
    /// The source table had no borders (HTML default); the document table gets no grid style.
    pub borderless: bool,
}

impl FTable {
    pub fn cols(&self) -> usize {
        self.rows.iter().map(|r| r.iter().map(|c| c.colspan.clamp(1, 63) as usize).sum::<usize>()).max().unwrap_or(0)
    }
    /// After an importer that gives `rowspan` but no placeholder cells (HTML): insert covered
    /// cells into the rows below each spanning cell.
    pub fn insert_covered(&mut self) {
        let nrows = self.rows.len();
        for r in 0..nrows {
            let mut g = 0usize;
            let mut ci = 0usize;
            while let Some(c) = self.rows.get(r).and_then(|row| row.get(ci)) {
                let (span, rs, covered) = (c.colspan.clamp(1, 63) as usize, c.rowspan.clamp(1, 1000) as usize, c.covered);
                if rs > 1 && !covered {
                    for k in 1..rs {
                        let Some(row) = self.rows.get_mut(r + k) else { break };
                        // Find the index in that row at grid column `g`.
                        let mut x = 0usize;
                        let mut at = row.len();
                        for (i, cc) in row.iter().enumerate() {
                            if x >= g {
                                at = i;
                                break;
                            }
                            x += cc.colspan.clamp(1, 63) as usize;
                        }
                        if row.len() < 64 {
                            row.insert(at, Cell { colspan: span as u32, covered: true, ..Default::default() });
                        }
                    }
                }
                g += span;
                ci += 1;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FBlock {
    Para(Para),
    Table(FTable),
}

/// Document properties carried by the text formats.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub description: String,
    pub created: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    pub blocks: Vec<FBlock>,
    pub meta: Meta,
}

/// `inlines` with equations as their linear text (default formatting), for formats that have no
/// equations.
pub fn equations_as_text(inlines: &[Inline]) -> std::borrow::Cow<'_, [Inline]> {
    if !inlines.iter().any(|i| matches!(i, Inline::Equation { .. })) {
        return std::borrow::Cow::Borrowed(inlines);
    }
    let mut p = Para::default();
    for i in inlines {
        match i {
            Inline::Equation { linear, .. } => p.push_text(linear, &Fmt::default()),
            Inline::Text(t, f) => p.push_text(t, f),
            other => p.inlines.push(other.clone()),
        }
    }
    std::borrow::Cow::Owned(p.inlines)
}

// ---------------------------------------------------------------------------------------------
// Flow → Document

/// Text with characters the model reserves (U+FFFC) and stray controls removed.
pub fn clean_text(s: &str) -> String {
    s.chars().filter(|c| *c != OBJ && (!c.is_control() || matches!(c, '\t' | '\n' | '\u{000C}' | '\u{000E}'))).collect()
}

pub fn char_props(f: &Fmt) -> CharProps {
    let on = |b: bool| if b { Some(true) } else { None };
    CharProps {
        bold: on(f.bold),
        italic: on(f.italic),
        underline: if f.underline { Some(Underline::Single) } else { None },
        strike: on(f.strike),
        vert_align: if f.sup {
            Some(VertAlign::Superscript)
        } else if f.sub {
            Some(VertAlign::Subscript)
        } else {
            None
        },
        link: f.link.clone().filter(|l| !l.is_empty()),
        color: f.color.map(TextColor::Rgb),
        size: f.size.filter(|s| s.is_finite() && *s >= 1.0 && *s <= 1638.0),
        font: if f.code { Some(MONO_FONT.to_string()) } else { f.font.clone().filter(|s| !s.is_empty()) },
        shading: f.background,
        ..Default::default()
    }
}

fn code_style() -> Style {
    Style {
        id: CODE_STYLE.into(),
        name: CODE_STYLE.into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        para: ParaProps { space_after: Some(0.0), space_before: Some(0.0), line_spacing: Some(Default::default()), ..Default::default() },
        chr: CharProps { font: Some(MONO_FONT.into()), size: Some(10.0), ..Default::default() },
        quick: false,
        ..Default::default()
    }
}

fn rule_borders() -> Borders {
    Borders { bottom: Some(Border { style: BorderStyle::Single, width: 0.75, color: Some(Rgb(0xA0, 0xA0, 0xA0)), space: 1.0 }), ..Default::default() }
}

struct Builder<'a> {
    doc: &'a mut Document,
}

impl Builder<'_> {
    fn para(&mut self, p: &Para, list_num: Option<u32>) -> Paragraph {
        let mut out = Paragraph::new();
        let style = match p.kind {
            Kind::Heading(n) => Some(format!("Heading{}", n.clamp(1, 6))),
            Kind::Title => Some("Title".into()),
            Kind::Quote => Some("Quote".into()),
            Kind::Code => Some(CODE_STYLE.into()),
            Kind::Rule | Kind::Normal => None,
        };
        out.props.style = style;
        if p.kind == Kind::Rule {
            out.props.borders = Some(rule_borders());
        }
        if let (Some(li), Some(num)) = (p.list, list_num) {
            if out.props.style.is_none() {
                out.props.style = Some("ListParagraph".into());
            }
            out.props.numbering = Some(NumRef { num, level: li.level.min(8) });
        }
        out.props.align = p.align;
        if p.page_break {
            out.props.page_break_before = Some(true);
        }
        for i in &p.inlines {
            let end = out.len();
            match i {
                Inline::Text(t, f) => {
                    let t = clean_text(t);
                    let mut cp = char_props(f);
                    if p.kind == Kind::Code && cp.font.as_deref() == Some(MONO_FONT) {
                        cp.font = None;
                    }
                    let _ = out.insert_text(end, &t, &cp);
                }
                Inline::Image(img) => {
                    let key = self.doc.add_media(img.data.to_vec(), &img.ext);
                    let (w, h) = (finite_or(img.w, 72.0).clamp(1.0, 1584.0), finite_or(img.h, 72.0).clamp(1.0, 1584.0));
                    let obj = InlineObject::Image { media: key, w, h, alt: img.alt.clone(), float: Default::default(), crop: [0.0; 4] };
                    let _ = out.insert_object(end, obj, &CharProps::default());
                }
                Inline::Anchor(name) => {
                    let _ = out.insert_object(end, InlineObject::BookmarkStart { name: name.clone() }, &CharProps::default());
                    let e2 = out.len();
                    let _ = out.insert_object(e2, InlineObject::BookmarkEnd { name: name.clone() }, &CharProps::default());
                }
                Inline::Equation { linear, display } => {
                    let linear = clean_text(linear);
                    if !linear.is_empty() {
                        let _ = out.insert_object(
                            end,
                            InlineObject::Equation { linear, display: *display, math: Default::default() },
                            &CharProps::default(),
                        );
                    }
                }
            }
        }
        out
    }

    /// A new list instance whose levels follow the kinds used in `group` (first use per level).
    fn new_list(&mut self, group: &[ListInfo]) -> u32 {
        let num_levels = levels_for(ListKind::Numbered);
        let bul_levels = levels_for(ListKind::Bullet);
        let mut kinds: [Option<bool>; 9] = [None; 9];
        for li in group {
            if let Some(k) = kinds.get_mut(li.level.min(8) as usize)
                && k.is_none()
            {
                *k = Some(li.ordered);
            }
        }
        // Levels nobody used follow the first level's kind.
        let first = group.first().map(|l| l.ordered).unwrap_or(false);
        let levels: Vec<Level> = (0..9)
            .map(|i| {
                let ordered = kinds.get(i).copied().flatten().unwrap_or(first);
                let src = if ordered { &num_levels } else { &bul_levels };
                src.get(i).cloned().unwrap_or_default()
            })
            .collect();
        let numbering = &mut self.doc.numbering;
        let aid = numbering.abstracts.iter().map(|a| a.id + 1).max().unwrap_or(0);
        numbering.abstracts.push(AbstractNum { id: aid, name: None, levels });
        let nid = numbering.nums.iter().map(|n| n.id + 1).max().unwrap_or(1).max(1);
        numbering.nums.push(Num { id: nid, abstract_id: aid, ..Default::default() });
        nid
    }

    fn blocks(&mut self, blocks: &[FBlock], depth: usize) -> Blocks {
        let mut out = Blocks::new();
        let mut i = 0;
        while let Some(b) = blocks.get(i) {
            match b {
                FBlock::Para(p) if p.list.is_some() => {
                    // A run of consecutive list paragraphs is one list, until a top-level item of
                    // the other kind (bulleted or numbered) starts a new one.
                    let mut j = i;
                    let mut group: Vec<ListInfo> = Vec::new();
                    let mut top: Option<bool> = None;
                    while let Some(FBlock::Para(q)) = blocks.get(j) {
                        let Some(li) = q.list else { break };
                        if li.level == 0 {
                            if top.is_some_and(|o| o != li.ordered) {
                                break;
                            }
                            top = Some(li.ordered);
                        }
                        group.push(li);
                        j += 1;
                    }
                    let num = self.new_list(&group);
                    for q in blocks.get(i..j).unwrap_or(&[]) {
                        if let FBlock::Para(q) = q {
                            let para = self.para(q, Some(num));
                            out.push(para_block(para));
                        }
                    }
                    i = j.max(i + 1);
                    continue;
                }
                FBlock::Para(p) => out.push(para_block(self.para(p, None))),
                FBlock::Table(t) => {
                    if depth >= MAX_DEPTH {
                        // Too deep: flatten into paragraphs.
                        for row in &t.rows {
                            for c in row {
                                out.extend(self.blocks(&c.blocks, depth));
                            }
                        }
                    } else if let Some(tb) = self.table(t, depth) {
                        out.push(Arc::new(Block::Table(tb)));
                    }
                }
            }
            i += 1;
        }
        out
    }

    fn table(&mut self, t: &FTable, depth: usize) -> Option<Table> {
        let rows: Vec<&Vec<Cell>> = t.rows.iter().filter(|r| !r.is_empty()).take(wordcraft_doc::table::MAX_ROWS).collect();
        if rows.is_empty() {
            return None;
        }
        let cols = t.cols().clamp(1, wordcraft_doc::table::MAX_COLS);
        let width = self.doc.last_section.text_width().max(72.0);
        let mut tb = Table::new(rows.len(), cols, width);
        if t.borderless {
            tb.props.style = None;
        }
        if t.widths.len() == cols && t.widths.iter().all(|w| w.is_finite() && *w > 1.0) {
            tb.grid = t.widths.iter().map(|w| w.clamp(6.0, 1584.0)).collect();
        }
        tb.rows.clear();
        for r in rows {
            let mut row = wordcraft_doc::Row { props: RowProps::default(), cells: Vec::new() };
            row.props.header = !r.is_empty() && r.iter().all(|c| c.header);
            let mut g = 0usize;
            for c in r.iter().take(wordcraft_doc::table::MAX_COLS) {
                let span = (c.colspan.clamp(1, 63) as usize).min(cols.saturating_sub(g).max(1));
                let w: f32 = tb.grid.get(g..(g + span).min(tb.grid.len())).map(|s| s.iter().sum()).unwrap_or(72.0);
                g += span;
                let mut blocks = self.blocks(&c.blocks, depth + 1);
                if blocks.is_empty() || matches!(blocks.last().map(|b| &**b), Some(Block::Table(_))) {
                    blocks.push(para_block(Paragraph::new()));
                }
                let vmerge = if c.covered {
                    VMerge::Continue
                } else if c.rowspan > 1 {
                    VMerge::Restart
                } else {
                    VMerge::None
                };
                row.cells.push(wordcraft_doc::Cell {
                    props: CellProps { width: Some(w), span: span as u32, vmerge, shading: c.shading, ..Default::default() },
                    blocks,
                });
            }
            if row.cells.is_empty() {
                row.cells.push(wordcraft_doc::Cell::empty());
            }
            tb.rows.push(row);
        }
        Some(tb)
    }
}

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() { v } else { d }
}

/// Build a document from a flow.
pub fn to_doc(flow: &Flow) -> Document {
    let mut doc = Document::new();
    doc.numbering = Default::default();
    let uses_code = uses_kind(&flow.blocks, Kind::Code, 0);
    if uses_code {
        doc.styles.upsert(code_style());
    }
    let body = Builder { doc: &mut doc }.blocks(&flow.blocks, 0);
    doc.body = body;
    let m = &flow.meta;
    doc.core.title = m.title.clone();
    doc.core.creator = m.author.clone();
    doc.core.subject = m.subject.clone();
    doc.core.keywords = m.keywords.clone();
    doc.core.description = m.description.clone();
    doc.core.created = m.created.clone();
    doc.ensure_nonempty();
    doc
}

fn uses_kind(blocks: &[FBlock], k: Kind, depth: usize) -> bool {
    depth <= MAX_DEPTH
        && blocks.iter().any(|b| match b {
            FBlock::Para(p) => p.kind == k,
            FBlock::Table(t) => t.rows.iter().flatten().any(|c| uses_kind(&c.blocks, k, depth + 1)),
        })
}

// ---------------------------------------------------------------------------------------------
// Document → Flow

/// Is this a monospace family name?
pub fn is_mono(font: &str) -> bool {
    let f = font.to_ascii_lowercase();
    f.contains("courier") || f.contains("mono") || f.contains("consolas") || f.contains("menlo") || f.contains("code") || f == "monaco"
}

fn is_code_style(doc: &Document, id: &str) -> bool {
    let names = |s: &str| {
        let l = s.to_ascii_lowercase().replace([' ', '-', '_'], "");
        l == "code" || l.contains("preformatted") || l == "plaintext" || l == "codeblock" || l == "sourcecode"
    };
    doc.styles.chain(id).iter().any(|s| names(&s.id) || names(&s.name)) || names(id)
}

/// Character formatting from a run's character style chain plus direct formatting (the
/// paragraph style is left out: it is expressed by the paragraph kind).
fn effective(doc: &Document, cp: &CharProps) -> CharProps {
    let mut out = CharProps::default();
    if let Some(cs) = cp.style.as_deref() {
        for s in doc.styles.chain(cs) {
            if s.kind == StyleKind::Character {
                out.overlay(&s.chr);
            }
        }
    }
    let mut direct = cp.clone();
    direct.style = None;
    out.overlay(&direct);
    out
}

fn fmt_of(doc: &Document, cp: &CharProps, in_code: bool) -> Fmt {
    let e = effective(doc, cp);
    let code = !in_code && e.font.as_deref().is_some_and(is_mono);
    Fmt {
        bold: e.bold.unwrap_or(false),
        italic: e.italic.unwrap_or(false),
        underline: e.underline.is_some_and(|u| u != Underline::None) && e.link.is_none(),
        strike: e.strike.unwrap_or(false) || e.double_strike.unwrap_or(false),
        code,
        sup: e.vert_align == Some(VertAlign::Superscript),
        sub: e.vert_align == Some(VertAlign::Subscript),
        link: e.link.clone(),
        color: match e.color {
            Some(TextColor::Rgb(c)) => Some(c),
            _ => None,
        },
        size: e.size,
        font: if code || in_code { None } else { e.font.clone() },
        background: e.highlight.and_then(|h| h.rgb()).or(e.shading),
    }
}

/// Paragraph kind and list membership of a document paragraph.
pub fn para_kind(doc: &Document, p: &Paragraph) -> (Kind, Option<ListInfo>) {
    let rp = doc.styles.resolve_para(&p.props);
    let style = p.props.style.as_deref().unwrap_or("Normal");
    let kind = if let Some(l) = rp.outline_level {
        Kind::Heading((l + 1).min(6))
    } else if doc.styles.chain(style).iter().any(|s| s.id == "Title") {
        Kind::Title
    } else if doc.styles.chain(style).iter().any(|s| s.id == "Quote" || s.id == "IntenseQuote") {
        Kind::Quote
    } else if is_code_style(doc, style) {
        Kind::Code
    } else if p.plain_text().trim().is_empty()
        && p.objects.iter().all(|o| o.is_marker())
        && rp.borders.and_then(|b| b.bottom).is_some_and(|b| b.is_visible())
    {
        Kind::Rule
    } else {
        Kind::Normal
    };
    let list = rp.numbering.and_then(|n| {
        let lv = doc.numbering.level(n.num, n.level)?;
        Some(ListInfo { ordered: !matches!(lv.format, NumFormat::Bullet | NumFormat::None), level: n.level.min(8) })
    });
    (kind, list)
}

fn media_ext(key: &str, data: &[u8]) -> String {
    match image::guess_format(data) {
        Ok(image::ImageFormat::Png) => "png".into(),
        Ok(image::ImageFormat::Jpeg) => "jpeg".into(),
        Ok(image::ImageFormat::Gif) => "gif".into(),
        Ok(image::ImageFormat::WebP) => "webp".into(),
        Ok(image::ImageFormat::Bmp) => "bmp".into(),
        _ => key.rsplit('.').next().filter(|e| e.len() <= 5 && *e != key).unwrap_or("png").to_ascii_lowercase(),
    }
}

/// A document paragraph as flow paragraphs: a page break character inside the text ends the
/// paragraph, and the text after it starts a new one that carries `page_break`.
pub fn flow_paras(doc: &Document, p: &Paragraph) -> Vec<Para> {
    let (kind, list) = para_kind(doc, p);
    let mut done: Vec<Para> = Vec::new();
    let mut out = Para { kind, list, align: p.props.align, page_break: p.props.page_break_before.unwrap_or(false), inlines: Vec::new() };
    let in_code = kind == Kind::Code;
    let mut k = 0usize;
    for (range, props) in p.run_ranges() {
        let Some(text) = p.text.get(range.clone()) else { continue };
        let hidden = props.hidden.unwrap_or(false) || props.del.is_some();
        let f = fmt_of(doc, props, in_code);
        let mut buf = String::new();
        for c in text.chars() {
            if c == PAGE_BREAK {
                if !hidden {
                    out.push_text(&std::mem::take(&mut buf), &f);
                    let next = Para { kind, list: None, align: p.props.align, page_break: true, inlines: Vec::new() };
                    let prev = std::mem::replace(&mut out, next);
                    if prev.inlines.is_empty() {
                        out.page_break |= prev.page_break;
                        out.list = prev.list;
                    } else {
                        done.push(prev);
                    }
                }
                continue;
            }
            if c != OBJ {
                buf.push(c);
                continue;
            }
            let obj = p.objects.get(k);
            k += 1;
            if hidden {
                continue;
            }
            match obj {
                Some(InlineObject::Image { media, w, h, alt, .. }) => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    match doc.media.get(media) {
                        Some(data) => {
                            out.inlines.push(Inline::Image(Img { data: data.clone(), ext: media_ext(media, data), w: *w, h: *h, alt: alt.clone() }))
                        }
                        None if !alt.is_empty() => buf.push_str(alt),
                        None => {}
                    }
                }
                Some(InlineObject::Equation { linear, display, math }) => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    // The linear format is the shared form; an equation known only by its
                    // structure gets it from there.
                    let linear = if linear.is_empty() { wordcraft_doc::math::to_linear(&math.nodes) } else { linear.clone() };
                    out.inlines.push(Inline::Equation { linear, display: *display });
                }
                Some(InlineObject::BookmarkStart { name }) if name != "_GoBack" => {
                    if !buf.is_empty() {
                        out.push_text(&std::mem::take(&mut buf), &f);
                    }
                    out.inlines.push(Inline::Anchor(name.clone()));
                }
                Some(o) => buf.push_str(o.plain_text()),
                None => {}
            }
        }
        if !hidden {
            out.push_text(&buf, &f);
        }
    }
    done.push(out);
    done
}

fn flow_blocks(doc: &Document, bl: &Blocks, depth: usize) -> Vec<FBlock> {
    let mut out = Vec::new();
    for b in bl {
        match &**b {
            Block::Para(p) => out.extend(flow_paras(doc, p).into_iter().map(FBlock::Para)),
            Block::Table(t) if depth < MAX_DEPTH => out.push(FBlock::Table(flow_table(doc, t, depth))),
            Block::Table(t) => {
                for r in &t.rows {
                    for c in &r.cells {
                        out.extend(flow_blocks(doc, &c.blocks, depth));
                    }
                }
            }
        }
    }
    out
}

fn flow_table(doc: &Document, t: &Table, depth: usize) -> FTable {
    let mut rows = Vec::new();
    for (ri, r) in t.rows.iter().enumerate() {
        let header = r.props.header || (ri == 0 && t.props.look.header_row && t.rows.len() > 1);
        let mut cells = Vec::new();
        for (ci, c) in r.cells.iter().enumerate() {
            let mut rowspan = 1;
            if c.props.vmerge == VMerge::Restart {
                let g = t.grid_col(ri, ci);
                for (k, below) in t.rows.iter().enumerate().skip(ri + 1) {
                    let cont = t.cell_at_grid(k, g).and_then(|i| below.cells.get(i)).is_some_and(|bc| bc.props.vmerge == VMerge::Continue);
                    if !cont {
                        break;
                    }
                    rowspan += 1;
                }
            }
            cells.push(Cell {
                blocks: flow_blocks(doc, &c.blocks, depth + 1),
                colspan: c.span() as u32,
                rowspan,
                covered: c.props.vmerge == VMerge::Continue,
                header,
                shading: c.props.shading,
            });
        }
        rows.push(cells);
    }
    FTable { rows, widths: t.grid.clone(), borderless: false }
}

/// The document body as a flow.
pub fn from_doc(doc: &Document) -> Flow {
    let c = &doc.core;
    Flow {
        blocks: flow_blocks(doc, &doc.body, 0),
        meta: Meta {
            title: c.title.clone(),
            author: c.creator.clone(),
            subject: c.subject.clone(),
            keywords: c.keywords.clone(),
            description: c.description.clone(),
            created: c.created.clone(),
        },
    }
}

// ---------------------------------------------------------------------------------------------
// Shared helpers

/// Image extension from bytes, if it's a format we can display.
pub fn sniff_image(data: &[u8]) -> Option<&'static str> {
    match image::guess_format(data).ok()? {
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::Jpeg => Some("jpeg"),
        image::ImageFormat::Gif => Some("gif"),
        image::ImageFormat::WebP => Some("webp"),
        image::ImageFormat::Bmp => Some("bmp"),
        _ => None,
    }
}

/// Pixel size of encoded image bytes.
pub fn image_px(data: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(data)).with_guessed_format().ok()?.into_dimensions().ok()
}

/// A picture from bytes with an optional display size (points); the natural size (96 dpi) is
/// used otherwise, shrunk to fit a 6.5" column.
pub fn make_img(data: Vec<u8>, w: Option<f32>, h: Option<f32>, alt: &str) -> Option<Img> {
    let ext = sniff_image(&data)?;
    let (pw, ph) = image_px(&data).map(|(a, b)| (a as f32 * 0.75, b as f32 * 0.75)).unwrap_or((72.0, 72.0));
    let (pw, ph) = (pw.max(1.0), ph.max(1.0));
    let (mut w, mut h) = match (w.filter(|v| v.is_finite() && *v > 0.0), h.filter(|v| v.is_finite() && *v > 0.0)) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * ph / pw),
        (None, Some(h)) => (h * pw / ph, h),
        (None, None) => (pw, ph),
    };
    if w > 468.0 {
        h *= 468.0 / w;
        w = 468.0;
    }
    Some(Img { data: Arc::new(data), ext: ext.to_string(), w: w.clamp(1.0, 1584.0), h: h.clamp(1.0, 1584.0), alt: alt.to_string() })
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let b = [ch.first().copied().unwrap_or(0), ch.get(1).copied().unwrap_or(0), ch.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, v) in idx.iter().enumerate() {
            if i <= ch.len() {
                out.push(B64.get(*v as usize).copied().unwrap_or(b'A') as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Lenient base64 (standard or URL-safe alphabet; whitespace and padding ignored).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Decode a `data:` URI (base64 or percent-encoded).
pub fn data_uri(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.trim().strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    if head.ends_with(";base64") { base64_decode(data) } else { Some(percent_decode(data)) }
}

pub fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%'
            && let Some(h) = b.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(h).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

pub fn mime_of(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// List labels in document order for list paragraphs of a flow ("1.", "a.", "•"), computed by a
/// simple per-level counter (what text exports show).
#[derive(Default)]
pub struct ListCounter {
    counts: [u32; 9],
    ordered: [bool; 9],
}

impl ListCounter {
    pub fn reset(&mut self) {
        *self = ListCounter::default();
    }
    /// Advance for `li` and return its number (1-based; bullets also count).
    pub fn next(&mut self, li: ListInfo) -> u32 {
        let lv = li.level.min(8) as usize;
        if self.ordered.get(lv).copied() != Some(li.ordered)
            && let (Some(o), Some(c)) = (self.ordered.get_mut(lv), self.counts.get_mut(lv))
        {
            *o = li.ordered;
            *c = 0;
        }
        let n = match self.counts.get_mut(lv) {
            Some(c) => {
                *c = c.saturating_add(1);
                *c
            }
            None => 1,
        };
        for d in self.counts.iter_mut().skip(lv + 1) {
            *d = 0;
        }
        n
    }
    pub fn label(&mut self, li: ListInfo) -> String {
        let n = self.next(li);
        if !li.ordered {
            return ["•", "◦", "▪"].get(li.level as usize % 3).copied().unwrap_or("•").to_string();
        }
        match li.level % 3 {
            0 => format!("{n}."),
            1 => format!("{}.", wordcraft_doc::section::letters(n)),
            _ => format!("{}.", wordcraft_doc::section::roman(n)),
        }
    }
}

/// Parse a CSS / HTML colour: `#rgb`, `#rrggbb`, `rgb(r,g,b)` or a basic name.
pub fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        if h.len() == 3 && h.is_ascii() {
            let d = |i: usize| h.get(i..i + 1).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| v * 17);
            return Some(Rgb(d(0)?, d(1)?, d(2)?));
        }
        return Rgb::parse(h);
    }
    if let Some(inner) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")) {
        let v: Vec<u8> = inner
            .trim_end_matches(')')
            .split(',')
            .take(3)
            .filter_map(|x| {
                let x = x.trim();
                if let Some(p) = x.strip_suffix('%') {
                    p.trim().parse::<f32>().ok().map(|f| (f.clamp(0.0, 100.0) * 2.55).round() as u8)
                } else {
                    x.parse::<f32>().ok().map(|f| f.clamp(0.0, 255.0).round() as u8)
                }
            })
            .collect();
        if let [r, g, b] = v[..] {
            return Some(Rgb(r, g, b));
        }
        return None;
    }
    Some(match s.as_str() {
        "black" => Rgb(0, 0, 0),
        "white" => Rgb(255, 255, 255),
        "red" => Rgb(255, 0, 0),
        "green" => Rgb(0, 128, 0),
        "lime" => Rgb(0, 255, 0),
        "blue" => Rgb(0, 0, 255),
        "yellow" => Rgb(255, 255, 0),
        "cyan" | "aqua" => Rgb(0, 255, 255),
        "magenta" | "fuchsia" => Rgb(255, 0, 255),
        "gray" | "grey" => Rgb(128, 128, 128),
        "silver" => Rgb(192, 192, 192),
        "maroon" => Rgb(128, 0, 0),
        "olive" => Rgb(128, 128, 0),
        "navy" => Rgb(0, 0, 128),
        "purple" => Rgb(128, 0, 128),
        "teal" => Rgb(0, 128, 128),
        "orange" => Rgb(255, 165, 0),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            let e = base64_encode(data);
            assert_eq!(base64_decode(&e).unwrap(), data, "{e}");
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert!(base64_decode("@@").is_none());
        assert_eq!(data_uri("data:text/plain,a%20b").unwrap(), b"a b");
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#f00"), Some(Rgb(255, 0, 0)));
        assert_eq!(parse_color("#0080FF"), Some(Rgb(0, 128, 255)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Rgb(1, 2, 3)));
        assert_eq!(parse_color("navy"), Some(Rgb(0, 0, 128)));
        assert_eq!(parse_color("#zz"), None);
        assert_eq!(parse_color("rgb(1)"), None);
    }

    #[test]
    fn list_labels() {
        let mut c = ListCounter::default();
        let o = |level| ListInfo { ordered: true, level };
        assert_eq!(c.label(o(0)), "1.");
        assert_eq!(c.label(o(1)), "a.");
        assert_eq!(c.label(o(1)), "b.");
        assert_eq!(c.label(o(0)), "2.");
        assert_eq!(c.label(ListInfo { ordered: false, level: 0 }), "•");
    }

    #[test]
    fn covered_cells_inserted() {
        let mut t = FTable {
            rows: vec![vec![Cell { rowspan: 2, ..Default::default() }, Cell::default()], vec![Cell::default()]],
            widths: vec![],
            borderless: false,
        };
        t.insert_covered();
        assert_eq!(t.rows[1].len(), 2);
        assert!(t.rows[1][0].covered);
    }
}
