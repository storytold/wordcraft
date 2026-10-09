//! WordCraft layout: turns a [`Document`] into pages.
//!
//! - [`para`]: one paragraph → lines (shaping, first-fit line breaking, tabs, list labels,
//!   alignment). Results are memoised per paragraph revision in a [`LayoutCache`].
//! - [`layout`]: sections → pages and columns; tables; keep/widow rules; page breaks; headers
//!   and footers; page-number fields.
//! - [`display`]: a page → draw items (glyph runs, rules, fills, images) for the renderers.
//! - [`hit`]: point ↔ position, caret geometry, line navigation, selection rectangles.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod display;
pub mod fields;
pub mod hit;
pub mod math;
pub mod para;
mod table;

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use wordcraft_doc::numbering::Counters;
use wordcraft_doc::para::{InlineObject, Wrap};
use wordcraft_doc::props::{Border, CharProps, Rgb};
use wordcraft_doc::section::{SectionProps, SectionStart};
use wordcraft_doc::{Block, Blocks, Document, Paragraph, Path, StoryRef};
use wordcraft_geom::Rect;

pub use fields::FieldCtx;
pub use para::{LineEnd, ParaLayout};

/// How the document is viewed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewMode {
    #[default]
    Print,
    /// One page as wide as the window, no page breaks.
    Web,
    /// Draft/Outline: like web, page breaks shown as rules.
    Draft,
}

#[derive(Clone, Debug, Default)]
pub struct LayoutOptions {
    pub view: ViewMode,
    /// Width of the window in Web/Draft views, points.
    pub web_width: f32,
    pub show_hidden: bool,
    /// Check spelling and grammar (squiggles).
    pub proofing: bool,
}

/// Something placed on a page (page coordinates, points, y down).
#[derive(Clone, Debug)]
pub enum Placed {
    /// Lines `l0..l1` of a paragraph; `y` is the top of line `l0`; `x` the column's left edge.
    Lines {
        story: StoryRef,
        path: Path,
        para: Arc<ParaLayout>,
        l0: usize,
        l1: usize,
        x: f32,
        y: f32,
    },
    Fill {
        rect: Rect,
        color: Rgb,
    },
    Rule {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        border: Border,
    },
    Image {
        rect: Rect,
        media: String,
        crop: [f32; 4],
        story: StoryRef,
        path: Path,
        off: usize,
    },
    Shape {
        rect: Rect,
        kind: wordcraft_doc::para::ShapeKind,
        fill: Option<Rgb>,
        stroke: Option<Rgb>,
        stroke_width: f32,
    },
    /// A table cell's area (for hit testing and cell selection).
    Cell {
        rect: Rect,
        table: Path,
        row: usize,
        cell: usize,
        story: StoryRef,
    },
}

impl Placed {
    fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Placed::Lines { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Placed::Fill { rect, .. } | Placed::Image { rect, .. } | Placed::Shape { rect, .. } | Placed::Cell { rect, .. } => {
                rect.x += dx;
                rect.y += dy;
            }
            Placed::Rule { x0, y0, x1, y1, .. } => {
                *x0 += dx;
                *x1 += dx;
                *y0 += dy;
                *y1 += dy;
            }
        }
    }
}

/// One laid-out page.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub w: f32,
    pub h: f32,
    /// Index into `Document::sections()`.
    pub section: usize,
    /// Page number as displayed (after section restarts).
    pub number: u32,
    pub items: Vec<Placed>,
    /// Header / footer items (drawn dimmed while editing the body).
    pub header: Vec<Placed>,
    pub footer: Vec<Placed>,
    /// The body text area (margins).
    pub body: Rect,
    pub header_story: Option<u32>,
    pub footer_story: Option<u32>,
    /// Top-level body block index at the start of the page.
    pub first_block: usize,
}

/// The whole layout.
#[derive(Clone, Debug, Default)]
pub struct DocLayout {
    pub pages: Vec<Page>,
    /// Index: (story, paragraph path) → (page, item index) of each placed piece, in order.
    pub index: HashMap<(StoryRef, Path), Vec<(usize, usize)>>,
    /// Layout time, milliseconds.
    pub ms: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    rev: u64,
    width: u32,
    label: Option<String>,
    page: Option<(u32, u32, u32, u32)>,
    table_chr: u64,
    notes: u64,
    excl: u64,
}

/// Memoised paragraph layouts.
#[derive(Default)]
pub struct LayoutCache {
    paras: HashMap<Key, Arc<ParaLayout>>,
    env: u64,
    used: HashSet<Key>,
    pub hits: u64,
    pub misses: u64,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.paras.len()
    }
    pub fn is_empty(&self) -> bool {
        self.paras.is_empty()
    }
    pub fn clear(&mut self) {
        self.paras.clear();
    }
}

fn hash_of<T: Hash>(t: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn env_hash(doc: &Document, opts: &LayoutOptions) -> u64 {
    let s = serde_json::to_string(&(&doc.styles, &doc.numbering, doc.settings.default_tab, &doc.settings.footnote_format)).unwrap_or_default();
    hash_of(&(s, opts.show_hidden, opts.proofing, wordcraft_proof::user_dictionary().len(), doc.settings.auto_hyphenation))
}

fn has_page_fields(p: &Paragraph) -> bool {
    p.objects.iter().any(|o| match o {
        InlineObject::Field { instr, .. } => {
            matches!(fields::field_name(instr).as_str(), "PAGE" | "NUMPAGES" | "SECTIONPAGES" | "SECTION")
        }
        _ => false,
    })
}

/// Walk state while paginating.
struct Ctx<'a> {
    doc: &'a Document,
    cache: &'a mut LayoutCache,
    opts: &'a LayoutOptions,
    counters: Counters,
    fields: FieldCtx,
    notes_hash: u64,
    numbers: HashMap<u32, Arc<ParaLayout>>,
}

impl Ctx<'_> {
    /// A laid-out line number (cached).
    fn number_para(&mut self, n: u32) -> Arc<ParaLayout> {
        if let Some(p) = self.numbers.get(&n) {
            return p.clone();
        }
        let mut p = Paragraph::with_text(
            &n.to_string(),
            CharProps { size: Some(10.0), color: Some(wordcraft_doc::TextColor::Rgb(Rgb(0x60, 0x60, 0x60))), ..Default::default() },
        );
        p.props.space_after = Some(0.0);
        p.props.space_before = Some(0.0);
        let env = para::ParaEnv {
            doc: self.doc,
            width: 60.0,
            label: None,
            fields: &self.fields,
            show_hidden: false,
            table_chr: None,
            proofing: false,
            exclusions: &[],
        };
        let pl = Arc::new(para::layout_para(&p, &env));
        self.numbers.insert(n, pl.clone());
        pl
    }

    fn para(&mut self, p: &Paragraph, width: f32, table_chr: Option<&CharProps>) -> Arc<ParaLayout> {
        self.para_x(p, width, table_chr, &[])
    }

    fn para_x(&mut self, p: &Paragraph, width: f32, table_chr: Option<&CharProps>, exclusions: &[para::Exclusion]) -> Arc<ParaLayout> {
        let label = p.props.numbering.or_else(|| self.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0).and_then(|n| {
            // Only count paragraphs that will show (list labels advance in order).
            self.counters.next_label(&self.doc.numbering, n.num, n.level)
        });
        let page = if has_page_fields(p) { Some(self.fields.page_key()) } else { None };
        let key = Key {
            rev: p.rev,
            width: width.to_bits(),
            label: label.as_ref().map(|(t, l)| format!("{t}|{}|{}|{:?}", l.indent, l.hanging, l.suffix)),
            page,
            table_chr: table_chr.map(|c| hash_of(&format!("{c:?}"))).unwrap_or(0),
            notes: if p.objects.iter().any(|o| matches!(o, InlineObject::NoteRef { .. })) { self.notes_hash } else { 0 },
            excl: if exclusions.is_empty() { 0 } else { hash_of(&format!("{exclusions:?}")) },
        };
        self.cache.used.insert(key.clone());
        if let Some(pl) = self.cache.paras.get(&key) {
            self.cache.hits += 1;
            return pl.clone();
        }
        self.cache.misses += 1;
        let env = para::ParaEnv {
            doc: self.doc,
            width,
            label,
            fields: &self.fields,
            show_hidden: self.opts.show_hidden,
            table_chr,
            proofing: self.opts.proofing,
            exclusions,
        };
        let pl = Arc::new(para::layout_para(p, &env));
        self.cache.paras.insert(key, pl.clone());
        pl
    }
}

/// Note part ids in document order → numbers (footnotes and endnotes numbered separately).
fn note_numbers(doc: &Document) -> HashMap<u32, u32> {
    let mut m = HashMap::new();
    let (mut f, mut e) = (0u32, 0u32);
    for path in doc.para_paths(StoryRef::Body) {
        if let Some(p) = doc.para(StoryRef::Body, &path) {
            for o in &p.objects {
                if let InlineObject::NoteRef { kind, id, .. } = o {
                    let n = match kind {
                        wordcraft_doc::para::NoteKind::Footnote => {
                            f += 1;
                            f
                        }
                        wordcraft_doc::para::NoteKind::Endnote => {
                            e += 1;
                            e
                        }
                    };
                    m.insert(*id, n);
                }
            }
        }
    }
    m
}

/// Lay out a block list into a free-standing box of `width` (no page breaks): table cells,
/// headers, footers, text boxes. Returns items relative to (0, 0) and the height.
fn layout_box(
    ctx: &mut Ctx,
    story: StoryRef,
    blocks: &Blocks,
    prefix: &[u32],
    width: f32,
    table_chr: Option<&CharProps>,
    depth: usize,
) -> (Vec<Placed>, f32) {
    let mut items = Vec::new();
    let mut y = 0.0f32;
    let mut prev_after = 0.0f32;
    let mut prev_style: Option<(String, bool)> = None;
    for (i, b) in blocks.iter().enumerate() {
        let mut path = prefix.to_vec();
        path.push(i as u32);
        match &**b {
            Block::Para(p) => {
                let pl = ctx.para(p, width, table_chr);
                let ctxl = pl.rp.contextual_spacing;
                let same = prev_style.as_ref().is_some_and(|(s, c)| *s == pl.rp.style && (*c || ctxl));
                let before = if same && ctxl { 0.0 } else { pl.rp.space_before };
                if same && ctxl {
                    y -= prev_after;
                }
                // Word: space between paragraphs is before + after (no collapsing).
                y += if i == 0 { before } else { before.max(0.0) };
                push_para(&mut items, story, &path, &pl, 0, pl.lines.len(), 0.0, y, width);
                y += pl.height + pl.rp.space_after;
                prev_after = if same && ctxl { 0.0 } else { pl.rp.space_after };
                prev_style = Some((pl.rp.style.clone(), ctxl));
            }
            Block::Table(t) => {
                if depth > 8 {
                    continue;
                }
                let tl = table::layout_table(ctx, story, t, &path, width, depth + 1);
                let mut ty = y;
                for row in &tl.rows {
                    for it in &row.items {
                        let mut it = it.clone();
                        it.translate(tl.x, ty);
                        items.push(it);
                    }
                    ty += row.height;
                }
                y = ty;
                prev_after = 0.0;
                prev_style = None;
            }
        }
    }
    (items, y.max(0.0))
}

/// Paragraph lines plus their shading and borders.
fn push_para(items: &mut Vec<Placed>, story: StoryRef, path: &[u32], pl: &Arc<ParaLayout>, l0: usize, l1: usize, x: f32, y: f32, width: f32) {
    let (Some(first), Some(last)) = (pl.lines.get(l0), l1.checked_sub(1).and_then(|k| pl.lines.get(k))) else {
        return;
    };
    let h = last.top + last.height - first.top;
    let left = x + pl.rp.indent_left.min(pl.rp.indent_left + pl.rp.indent_first);
    let right = x + width - pl.rp.indent_right;
    if let Some(c) = pl.rp.shading {
        let pad = pl.rp.borders.as_ref().map(|b| b.left.map(|l| l.space).unwrap_or(4.0)).unwrap_or(0.0);
        items.push(Placed::Fill { rect: Rect::new(left - pad, y, right - left + pad * 2.0, h), color: c });
    }
    items.push(Placed::Lines { story, path: Path(path.to_vec()), para: pl.clone(), l0, l1, x, y });
    if let Some(b) = &pl.rp.borders {
        let sp = |o: &Option<Border>| o.map(|b| b.space).unwrap_or(0.0);
        let (lx, rx) = (left - sp(&b.left), right + sp(&b.right));
        let (ty, by) = (y - sp(&b.top), y + h + sp(&b.bottom));
        if let Some(t) = b.top.filter(|_| l0 == 0)
            && t.is_visible()
        {
            items.push(Placed::Rule { x0: lx, y0: ty, x1: rx, y1: ty, border: t });
        }
        if let Some(t) = b.bottom.filter(|_| l1 >= pl.lines.len())
            && t.is_visible()
        {
            items.push(Placed::Rule { x0: lx, y0: by, x1: rx, y1: by, border: t });
        }
        if let Some(t) = b.left.filter(Border::is_visible) {
            items.push(Placed::Rule { x0: lx, y0: ty, x1: lx, y1: by, border: t });
        }
        if let Some(t) = b.right.filter(Border::is_visible) {
            items.push(Placed::Rule { x0: rx, y0: ty, x1: rx, y1: by, border: t });
        }
    }
}

struct PageBuilder<'a> {
    pages: Vec<Page>,
    sect: &'a SectionProps,
    sect_idx: usize,
    col: usize,
    cols: Vec<(f32, f32)>,
    y: f32,
    top: f32,
    bottom: f32,
    number: u32,
    web: bool,
    /// Footnotes waiting for the bottom of the current page: (part id, items, height).
    notes: Vec<(u32, Vec<Placed>, f32)>,
    /// Page bottom before footnotes took space.
    orig_bottom: f32,
    /// The previous paragraph: (style, contextual spacing, space after) for contextual spacing.
    prev: Option<(String, bool, f32)>,
    /// Wrap areas of floating objects on this page: (rect, top-and-bottom only, distance).
    excl: Vec<(Rect, bool, f32)>,
    /// Line numbering counter.
    line_no: u32,
    /// Index of the first body item of the current page (vertical alignment shifts from here).
    page_items_start: usize,
}

/// Gap above the footnote separator and its length.
const NOTE_SEP: f32 = 12.0;

impl PageBuilder<'_> {
    /// Space footnotes take at the bottom of the page.
    fn notes_h(&self) -> f32 {
        if self.notes.is_empty() { 0.0 } else { NOTE_SEP + self.notes.iter().map(|n| n.2).sum::<f32>() }
    }
    /// Place the collected footnotes at the bottom of the current page.
    fn flush_notes(&mut self) {
        if self.notes.is_empty() {
            return;
        }
        let total = self.notes_h();
        let x = self.sect.margin_left + self.sect.gutter;
        let mut y = self.orig_bottom - total + NOTE_SEP;
        let notes = std::mem::take(&mut self.notes);
        if let Some(pg) = self.pages.last_mut() {
            pg.items.push(Placed::Rule {
                x0: x,
                y0: y - NOTE_SEP / 2.0,
                x1: x + 144.0,
                y1: y - NOTE_SEP / 2.0,
                border: Border { style: wordcraft_doc::props::BorderStyle::Single, width: 0.5, color: None, space: 0.0 },
            });
            for (_, items, h) in notes {
                for mut it in items {
                    it.translate(x, y);
                    pg.items.push(it);
                }
                y += h;
            }
        }
    }
    /// Section vertical alignment: move the page's body down (centre/bottom).
    fn apply_valign(&mut self) {
        let k = match self.sect.valign {
            wordcraft_doc::props::VAlign::Top => return,
            wordcraft_doc::props::VAlign::Center => 0.5,
            wordcraft_doc::props::VAlign::Bottom => 1.0,
        };
        if self.web {
            return;
        }
        let space = (self.orig_bottom - self.notes_h() - self.y).max(0.0) * k;
        let start = self.page_items_start;
        if let Some(pg) = self.pages.last_mut() {
            for it in pg.items.iter_mut().skip(start) {
                it.translate(0.0, space);
            }
        }
    }
    fn new_page(&mut self, first_block: usize, body_top: f32) {
        self.apply_valign();
        self.flush_notes();
        self.excl.clear();
        if self.sect.line_numbers.as_ref().is_some_and(|l| l.restart == wordcraft_doc::section::LineNumberRestart::Page) {
            self.line_no = 0;
        }
        let s = self.sect;
        self.number += 1;
        let (w, h) = if self.web { (s.page_w, f32::MAX / 4.0) } else { (s.page_w, s.page_h) };
        let body = Rect::new(s.margin_left + s.gutter, body_top, s.text_width(), (s.page_h - s.margin_bottom - body_top).max(36.0));
        let mut decor = Vec::new();
        if !self.web {
            // Page borders, measured from the page edge (Word's default), so they surround the
            // header, footer and line numbers.
            if let Some(b) = &s.page_borders {
                let sp = |e: &Option<Border>| e.map(|x| x.space).unwrap_or(24.0);
                let (x0, x1, y0, y1) = (sp(&b.left), s.page_w - sp(&b.right), sp(&b.top), s.page_h - sp(&b.bottom));
                for (e, a, bb) in
                    [(b.top, (x0, y0), (x1, y0)), (b.bottom, (x0, y1), (x1, y1)), (b.left, (x0, y0), (x0, y1)), (b.right, (x1, y0), (x1, y1))]
                {
                    if let Some(e) = e.filter(Border::is_visible) {
                        decor.push(Placed::Rule { x0: a.0, y0: a.1, x1: bb.0, y1: bb.1, border: e });
                    }
                }
            }
            // Lines between columns.
            if s.columns.separator && s.columns.count > 1 {
                let cols = s.column_boxes();
                for w in cols.windows(2) {
                    if let [(ax, aw), (bx, _)] = w {
                        let x = s.margin_left + s.gutter + (ax + aw + bx) / 2.0;
                        decor.push(Placed::Rule { x0: x, y0: body_top, x1: x, y1: s.page_h - s.margin_bottom, border: Border::single(0.5) });
                    }
                }
            }
        }
        self.page_items_start = decor.len();
        self.pages.push(Page { w, h, section: self.sect_idx, number: self.number, body, first_block, items: decor, ..Default::default() });
        self.col = 0;
        self.cols = s.column_boxes();
        self.top = body_top;
        self.bottom = if self.web { f32::MAX / 8.0 } else { s.page_h - s.margin_bottom };
        self.orig_bottom = self.bottom;
        self.y = self.top;
    }
    fn col_x(&self) -> f32 {
        self.sect.margin_left + self.sect.gutter + self.cols.get(self.col).map(|c| c.0).unwrap_or(0.0)
    }
    fn col_w(&self) -> f32 {
        self.cols.get(self.col).map(|c| c.1).unwrap_or_else(|| self.sect.text_width())
    }
    fn at_top(&self) -> bool {
        self.y <= self.top + 0.01
    }
    /// Move to the next column or page.
    fn advance(&mut self, block: usize, body_top: f32) {
        if self.col + 1 < self.cols.len() {
            self.col += 1;
            self.y = self.top;
        } else {
            self.new_page(block, body_top);
        }
    }
    fn page(&mut self) -> Option<&mut Page> {
        self.pages.last_mut()
    }
}

/// Lay out the whole document.
pub fn layout(doc: &Document, cache: &mut LayoutCache, opts: &LayoutOptions) -> DocLayout {
    let t0 = now_ms();
    let env = env_hash(doc, opts);
    if env != cache.env {
        cache.paras.clear();
        cache.env = env;
    }
    cache.used.clear();
    let notes = note_numbers(doc);
    let notes_hash = hash_of(&{
        let mut v: Vec<_> = notes.iter().map(|(a, b)| (*a, *b)).collect();
        v.sort();
        v
    });
    let mut ctx = Ctx {
        doc,
        cache,
        opts,
        counters: Counters::default(),
        fields: FieldCtx {
            page: 1,
            pages: 1,
            section_pages: 1,
            section: 1,
            notes: Arc::new(notes),
            title: doc.core.title.as_str().into(),
            author: doc.core.creator.as_str().into(),
            ..Default::default()
        },
        notes_hash,
        numbers: HashMap::new(),
    };
    let sections = doc.sections();
    let web = opts.view != ViewMode::Print;
    let default_sect = SectionProps::default();
    let mut web_sect;
    let first_sect = match sections.first() {
        Some((_, s)) => *s,
        None => &default_sect,
    };
    let sect_ref = if web {
        web_sect = first_sect.clone();
        web_sect.page_w = opts.web_width.max(144.0);
        web_sect.margin_left = 18.0;
        web_sect.margin_right = 18.0;
        web_sect.margin_top = 18.0;
        web_sect.gutter = 0.0;
        web_sect.columns = Default::default();
        &web_sect
    } else {
        first_sect
    };
    let mut pb = PageBuilder {
        pages: Vec::new(),
        sect: sect_ref,
        sect_idx: 0,
        col: 0,
        cols: Vec::new(),
        y: 0.0,
        top: 0.0,
        bottom: 0.0,
        number: 0,
        web,
        notes: Vec::new(),
        orig_bottom: 0.0,
        prev: None,
        excl: Vec::new(),
        line_no: 0,
        page_items_start: 0,
    };
    let mut block = 0usize;
    for (si, (end, sect)) in sections.iter().enumerate() {
        let sect: &SectionProps = if web { sect_ref } else { sect };
        pb.sect = sect;
        pb.sect_idx = si;
        let body_top = if web { sect.margin_top } else { body_top_for(&mut ctx, sect, sect.headers.default) };
        let first_top = if web || !sect.title_page { body_top } else { body_top_for(&mut ctx, sect, sect.headers.first) };
        let restart = sect.page_num_start;
        if sect.line_numbers.as_ref().is_some_and(|l| l.restart == wordcraft_doc::section::LineNumberRestart::Section) {
            pb.line_no = 0;
        }
        let start = if si == 0 { SectionStart::NextPage } else { sect.start };
        if web && si > 0 {
            // one long page
        } else if pb.pages.is_empty() || start != SectionStart::Continuous {
            if let Some(n) = restart {
                pb.number = n.saturating_sub(1);
            }
            pb.new_page(block, first_top);
            if !web && matches!(start, SectionStart::EvenPage | SectionStart::OddPage) {
                let want_even = start == SectionStart::EvenPage;
                if pb.number.is_multiple_of(2) != want_even {
                    pb.new_page(block, body_top);
                }
            }
        } else {
            // Continuous: new column layout below the current text.
            pb.cols = sect.column_boxes();
            pb.col = 0;
            pb.top = pb.y;
        }
        let last = (*end).min(doc.body.len().saturating_sub(1));
        while block <= last {
            let Some(b) = doc.body.get(block) else { break };
            match &**b {
                Block::Para(p) => place_para(&mut ctx, &mut pb, p, block, body_top),
                Block::Table(t) => place_table(&mut ctx, &mut pb, t, block, body_top),
            }
            block += 1;
        }
    }
    if pb.pages.is_empty() {
        pb.new_page(0, sect_ref.margin_top);
    }
    // Endnotes after the last paragraph.
    let endnotes: Vec<u32> = {
        let mut ids: Vec<(u32, u32)> = ctx
            .fields
            .notes
            .iter()
            .filter(|(id, _)| doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::Endnote))
            .map(|(a, b)| (*b, *a))
            .collect();
        ids.sort();
        ids.into_iter().map(|(_, id)| id).collect()
    };
    if !endnotes.is_empty() && !web {
        pb.y += 12.0;
        let (x, ry) = (pb.col_x(), pb.y);
        if let Some(pg) = pb.page() {
            pg.items.push(Placed::Rule {
                x0: x,
                y0: ry,
                x1: x + 144.0,
                y1: ry,
                border: Border { style: wordcraft_doc::props::BorderStyle::Single, width: 0.5, color: None, space: 0.0 },
            });
        }
        pb.y += 8.0;
        for id in endnotes {
            let Some(part) = doc.parts.get(&id) else { continue };
            let blocks = part.blocks.clone();
            let (items, h) = layout_box(&mut ctx, StoryRef::Part(id), &blocks, &[], pb.col_w(), None, 0);
            if pb.y + h > pb.bottom && !pb.at_top() {
                pb.advance(block, pb.top);
            }
            let (x, y) = (pb.col_x(), pb.y);
            if let Some(pg) = pb.page() {
                for mut it in items {
                    it.translate(x, y);
                    pg.items.push(it);
                }
            }
            pb.y += h;
        }
    }
    pb.apply_valign();
    pb.flush_notes();
    let mut pages = pb.pages;
    if web && let Some(p) = pages.first_mut() {
        let bottom = p
            .items
            .iter()
            .filter_map(|it| if let Placed::Lines { y, para, l0, l1, .. } = it { item_bottom(*y, para, *l0, *l1) } else { None })
            .fold(0.0f32, f32::max);
        p.h = bottom + 36.0;
    }
    if !web {
        headers_footers(&mut ctx, &mut pages, &sections);
    }
    // Evict paragraph layouts that weren't used this pass if the cache grew large.
    if ctx.cache.paras.len() > 4 * ctx.cache.used.len().max(1024) {
        let used = std::mem::take(&mut ctx.cache.used);
        ctx.cache.paras.retain(|k, _| used.contains(k));
        ctx.cache.used = used;
    }
    let mut index: HashMap<(StoryRef, Path), Vec<(usize, usize)>> = HashMap::new();
    for (pi, p) in pages.iter().enumerate() {
        for (ii, it) in p.items.iter().enumerate() {
            if let Placed::Lines { story, path, .. } = it {
                index.entry((*story, path.clone())).or_default().push((pi, ii));
            }
        }
    }
    DocLayout { pages, index, ms: now_ms() - t0 }
}

fn item_bottom(y: f32, para: &ParaLayout, l0: usize, l1: usize) -> Option<f32> {
    let f = para.lines.get(l0)?;
    let l = para.lines.get(l1.checked_sub(1)?)?;
    Some(y + l.top + l.height - f.top)
}

/// Where body text starts: the top margin, or below the header if it's taller.
fn body_top_for(ctx: &mut Ctx, sect: &SectionProps, header: Option<u32>) -> f32 {
    let Some(id) = header else { return sect.margin_top };
    let Some(part) = ctx.doc.parts.get(&id) else { return sect.margin_top };
    let blocks = part.blocks.clone();
    let (_, h) = layout_box(ctx, StoryRef::Part(id), &blocks, &[], sect.text_width(), None, 0);
    sect.margin_top.max(sect.header + h + 6.0)
}

/// Where a floating object goes: page coordinates for an anchor paragraph whose top is `y0`.
fn float_rect(pb: &PageBuilder, col_x: f32, y0: f32, w: f32, h: f32, float: &wordcraft_doc::para::Float) -> Rect {
    let fx = match float.h_rel {
        wordcraft_doc::para::Anchor::Page => float.x,
        wordcraft_doc::para::Anchor::Margin => pb.sect.margin_left + pb.sect.gutter + float.x,
        _ => col_x + float.x,
    };
    let fy = match float.v_rel {
        wordcraft_doc::para::Anchor::Page => float.y,
        wordcraft_doc::para::Anchor::Margin => pb.top + float.y,
        _ => y0 + float.y,
    };
    Rect::new(fx, fy, w.clamp(1.0, 4000.0), h.clamp(1.0, 4000.0))
}

fn place_para(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, body_top: f32) {
    let width = pb.col_w();
    ctx.fields.page = pb.number;
    // Floating objects anchored here join the page's wrap areas before the text is laid out.
    let col_x = pb.col_x();
    let y0 = pb.y + ctx.doc.styles.resolve_para(&p.props).space_before;
    let mut float_rects: HashMap<usize, Rect> = HashMap::new();
    if !pb.web {
        for (oi, o) in p.objects.iter().enumerate() {
            if let InlineObject::Image { w, h, float, .. } | InlineObject::Shape { w, h, float, .. } = o
                && float.wrap != Wrap::Inline
            {
                let r = float_rect(pb, col_x, y0, *w, *h, float);
                float_rects.insert(oi, r);
                if !matches!(float.wrap, Wrap::BehindText | Wrap::InFrontOfText) {
                    pb.excl.push((r, float.wrap == Wrap::TopAndBottom, float.dist.max(9.0)));
                }
            }
        }
    }
    let rel: Vec<para::Exclusion> = pb
        .excl
        .iter()
        .map(|(r, tb, d)| para::Exclusion {
            top: r.y - d - y0,
            bottom: r.bottom() + d - y0,
            left: r.x - d - col_x,
            right: r.right() + d - col_x,
            top_bottom: *tb,
        })
        .filter(|e| e.bottom > 0.0)
        .collect();
    let pl = ctx.para_x(p, width, None, &rel);
    if pl.rp.page_break_before && !pb.at_top() && !pb.web {
        pb.new_page(block, body_top);
    }
    // Contextual spacing: no space between paragraphs of the same style when either asks for it.
    let mut before = pl.rp.space_before;
    if let Some((style, ctxl, after)) = pb.prev.take()
        && style == pl.rp.style
        && !pb.at_top()
    {
        if ctxl {
            pb.y -= after;
        }
        if pl.rp.contextual_spacing {
            before = 0.0;
        }
    }
    pb.y += before;
    let n = pl.lines.len();
    // Keep lines together: if it doesn't fit but would on an empty column, move it.
    if pl.rp.keep_lines || pl.rp.keep_next {
        let need = pl.height + if pl.rp.keep_next { next_first_line(ctx, block, width) } else { 0.0 };
        if pb.y + need > pb.bottom && !pb.at_top() && need <= pb.bottom - pb.top {
            pb.advance(block, body_top);
            pb.y += pl.rp.space_before.min(0.0);
        }
    }
    let mut l0 = 0;
    while l0 < n {
        // How many lines fit?
        let Some(first) = pl.lines.get(l0) else { break };
        let mut l1 = l0;
        let mut forced_break = None;
        let mut new_notes: Vec<(u32, Vec<Placed>, f32)> = Vec::new();
        while l1 < n {
            let Some(l) = pl.lines.get(l1) else { break };
            // Footnotes referenced on this line go to the bottom of this page.
            let mut line_notes = Vec::new();
            if !pb.web {
                for (ci, id) in &pl.notes {
                    if *ci >= l.c0
                        && *ci < l.c1
                        && !pb.notes.iter().chain(new_notes.iter()).any(|x| x.0 == *id)
                        && let Some(part) = ctx.doc.parts.get(id).filter(|p| p.kind == wordcraft_doc::PartKind::Footnote)
                    {
                        let blocks = part.blocks.clone();
                        let (items, h) = layout_box(ctx, StoryRef::Part(*id), &blocks, &[], pb.sect.text_width(), None, 0);
                        line_notes.push((*id, items, h));
                    }
                }
            }
            let extra: f32 = line_notes.iter().map(|n| n.2).sum::<f32>()
                + if pb.notes.is_empty() && new_notes.is_empty() && !line_notes.is_empty() { NOTE_SEP } else { 0.0 };
            let limit = pb.bottom
                - new_notes.iter().map(|n| n.2).sum::<f32>()
                - if pb.notes.is_empty() && !new_notes.is_empty() { NOTE_SEP } else { 0.0 }
                - extra;
            let bottom = pb.y + (l.top + l.height - first.top);
            if bottom > limit + 0.01 && l1 > l0 {
                break;
            }
            if bottom > limit + 0.01 && l1 == l0 && !pb.at_top() {
                break;
            }
            new_notes.extend(line_notes);
            l1 += 1;
            if matches!(l.end, LineEnd::PageBreak | LineEnd::ColumnBreak) && !pb.web {
                forced_break = Some(l.end);
                break;
            }
        }
        // Widow/orphan control (2-line minimum at either end).
        if forced_break.is_none() && l1 < n && pl.rp.widow_control && n >= 2 {
            if l1 - l0 == 1 && l0 == 0 && !pb.at_top() {
                l1 = l0; // orphan: move the first line to the next page
            } else if n - l1 == 1 && l1 - l0 >= 3 {
                l1 -= 1; // widow: take one more line along
            }
        }
        if l1 == l0 {
            if pb.at_top() {
                l1 = l0 + 1; // can't fit even one line on an empty page: overflow
            } else {
                pb.advance(block, body_top);
                continue;
            }
        }
        // Keep only the notes of the lines that are placed.
        let placed_end = pl.lines.get(l1.saturating_sub(1)).map(|l| l.c1).unwrap_or(0);
        new_notes.retain(|(id, ..)| pl.notes.iter().any(|(ci, nid)| nid == id && *ci < placed_end));
        let before = pb.notes_h();
        pb.notes.extend(new_notes);
        pb.bottom -= pb.notes_h() - before;
        let x = pb.col_x();
        let y = pb.y;
        let mut items = Vec::new();
        push_para(&mut items, StoryRef::Body, &[block as u32], &pl, l0, l1, x, y, width);
        // Floating pictures/shapes anchored in these lines.
        let mut behind = Vec::new();
        if let (Some(fl), Some(ll)) = (pl.lines.get(l0), pl.lines.get(l1.saturating_sub(1))) {
            for k in fl.c0..ll.c1 {
                let Some(c) = pl.clusters.get(k) else { continue };
                let para::ClKind::Object(oi) = c.kind else { continue };
                let Some(obj) = p.objects.get(oi) else { continue };
                let (w, h, float) = match obj {
                    InlineObject::Image { w, h, float, .. } | InlineObject::Shape { w, h, float, .. } if float.wrap != Wrap::Inline => {
                        (*w, *h, *float)
                    }
                    _ => continue,
                };
                let rect = float_rects.get(&oi).copied().unwrap_or_else(|| float_rect(pb, x, y, w, h, &float));
                let it = match obj {
                    InlineObject::Image { media, crop, .. } => {
                        Placed::Image { rect, media: media.clone(), crop: *crop, story: StoryRef::Body, path: Path(vec![block as u32]), off: c.start }
                    }
                    InlineObject::Shape { kind, fill, stroke, stroke_width, .. } => {
                        Placed::Shape { rect, kind: *kind, fill: *fill, stroke: *stroke, stroke_width: *stroke_width }
                    }
                    _ => continue,
                };
                if float.wrap == Wrap::BehindText {
                    behind.push(it);
                } else {
                    items.push(it);
                }
            }
        }
        // Text box contents (inline and floating shapes that own a story).
        if let (Some(fl), Some(ll)) = (pl.lines.get(l0), pl.lines.get(l1.saturating_sub(1))) {
            for li in l0..l1 {
                let Some(line) = pl.lines.get(li) else { continue };
                for k in line.c0..line.c1 {
                    let Some(c) = pl.clusters.get(k) else { continue };
                    let para::ClKind::Object(oi) = c.kind else { continue };
                    let Some(InlineObject::Shape { story: Some(id), w, h, .. }) = p.objects.get(oi) else { continue };
                    let rect = match float_rects.get(&oi) {
                        Some(r) => *r,
                        None => {
                            let cx = x + line.xs.get(k - line.c0).copied().unwrap_or(0.0);
                            Rect::new(cx, y + (line.baseline - fl.top) - c.obj_h, c.adv, c.obj_h)
                        }
                    };
                    let _ = (w, h, ll);
                    let Some(part) = ctx.doc.parts.get(id) else { continue };
                    let blocks = part.blocks.clone();
                    let (inner, _) = layout_box(ctx, StoryRef::Part(*id), &blocks, &[], (rect.w - 14.4).max(12.0), None, 1);
                    for mut it in inner {
                        it.translate(rect.x + 7.2, rect.y + 3.6);
                        items.push(it);
                    }
                }
            }
        }
        // Line numbers in the left margin.
        if let Some(ln) = pb.sect.line_numbers.clone().filter(|_| !pb.web && pl.rp.style != "Header" && p.props.suppress_line_numbers != Some(true))
            && let Some(first) = pl.lines.get(l0)
        {
            for li in l0..l1 {
                let Some(line) = pl.lines.get(li) else { continue };
                pb.line_no += 1;
                let n = pb.line_no + ln.start.saturating_sub(1);
                if ln.count_by > 1 && !n.is_multiple_of(ln.count_by) {
                    continue;
                }
                let num = ctx.number_para(n);
                let w = num.lines.first().and_then(|l| l.xs.last().copied()).unwrap_or(10.0);
                let dist = if ln.distance > 0.0 { ln.distance } else { 18.0 };
                let ly = y + (line.top - first.top) + (line.baseline - line.top) - num.lines.first().map(|l| l.baseline).unwrap_or(10.0);
                items.push(Placed::Lines {
                    story: StoryRef::Part(u32::MAX),
                    path: Path(vec![]),
                    para: num.clone(),
                    l0: 0,
                    l1: 1,
                    x: pb.sect.margin_left + pb.sect.gutter - dist - w,
                    y: ly,
                });
            }
        }
        if let Some(pg) = pb.page() {
            for (i, it) in behind.into_iter().enumerate() {
                pg.items.insert(i.min(pg.items.len()), it);
            }
            pg.items.extend(items);
        }
        if let (Some(f), Some(l)) = (pl.lines.get(l0), pl.lines.get(l1 - 1)) {
            pb.y = y + (l.top + l.height - f.top);
        }
        l0 = l1;
        match forced_break {
            Some(LineEnd::PageBreak) => pb.new_page(block, body_top),
            Some(LineEnd::ColumnBreak) => pb.advance(block, body_top),
            _ if l0 < n => pb.advance(block, body_top),
            _ => {}
        }
    }
    pb.y += pl.rp.space_after;
    pb.prev = Some((pl.rp.style.clone(), pl.rp.contextual_spacing, pl.rp.space_after));
}

/// Height of the first line of the block after `block` (for keep-with-next).
fn next_first_line(ctx: &mut Ctx, block: usize, width: f32) -> f32 {
    match ctx.doc.body.get(block + 1).map(|b| &**b) {
        Some(Block::Para(p)) => {
            // Don't advance list counters for a lookahead: lay out without the label.
            let rp = ctx.doc.styles.resolve_para(&p.props);
            let env = para::ParaEnv {
                doc: ctx.doc,
                width,
                label: None,
                fields: &ctx.fields,
                show_hidden: ctx.opts.show_hidden,
                table_chr: None,
                proofing: false,
                exclusions: &[],
            };
            let _ = rp;
            let pl = para::layout_para(p, &env);
            pl.rp.space_before + pl.lines.first().map(|l| l.height).unwrap_or(0.0)
        }
        Some(Block::Table(_)) => 24.0,
        None => 0.0,
    }
}

fn place_table(ctx: &mut Ctx, pb: &mut PageBuilder, t: &wordcraft_doc::Table, block: usize, body_top: f32) {
    pb.prev = None;
    let width = pb.col_w();
    let tl = table::layout_table(ctx, StoryRef::Body, t, &[block as u32], width, 1);
    let header_rows: Vec<usize> = (0..t.rows.len()).take_while(|r| t.rows.get(*r).is_some_and(|row| row.props.header)).collect();
    let place = |pb: &mut PageBuilder, row: &table::RowLayout| {
        let (x, y) = (pb.col_x() + tl.x, pb.y);
        if let Some(pg) = pb.page() {
            for it in &row.items {
                let mut it = it.clone();
                it.translate(x, y);
                pg.items.push(it);
            }
        }
        pb.y += row.height;
    };
    for (ri, row) in tl.rows.iter().enumerate() {
        let splittable = !header_rows.contains(&ri) && !t.rows.get(ri).is_some_and(|r| r.props.cant_split);
        let mut rest: Option<table::RowLayout> = None;
        // Each pass places the part of the row that fits, then breaks the page.
        for _ in 0..1000 {
            let cur = rest.as_ref().unwrap_or(row);
            if pb.y + cur.height <= pb.bottom + 0.01 {
                break;
            }
            let split = if splittable { table::split_row(cur, pb.bottom - pb.y) } else { None };
            match split {
                Some((a, b)) => {
                    place(pb, &a);
                    rest = Some(b);
                }
                None if pb.at_top() => break,
                None => {}
            }
            pb.advance(block, body_top);
            // Repeat header rows.
            if !header_rows.contains(&ri) {
                for hr in &header_rows {
                    if let Some(h) = tl.rows.get(*hr) {
                        place(pb, h);
                    }
                }
            }
        }
        place(pb, rest.as_ref().unwrap_or(row));
    }
}

fn headers_footers(ctx: &mut Ctx, pages: &mut [Page], sections: &[(usize, &SectionProps)]) {
    let total = pages.len() as u32;
    // Pages per section.
    let mut per_section: HashMap<usize, u32> = HashMap::new();
    for p in pages.iter() {
        *per_section.entry(p.section).or_default() += 1;
    }
    let even_odd = ctx.doc.settings.even_odd_headers;
    let mut first_of_section: HashSet<usize> = HashSet::new();
    let mut seen = HashSet::new();
    for (i, p) in pages.iter().enumerate() {
        if seen.insert(p.section) {
            first_of_section.insert(i);
        }
    }
    for (i, page) in pages.iter_mut().enumerate() {
        let Some((_, sect)) = sections.get(page.section) else { continue };
        // Inherit header/footer references from earlier sections (Word's "link to previous").
        let pick = |get: &dyn Fn(&SectionProps) -> wordcraft_doc::section::HeaderSet| -> Option<u32> {
            let mut set = get(sect);
            for k in (0..page.section).rev() {
                if set.default.is_some() && set.first.is_some() && set.even.is_some() {
                    break;
                }
                if let Some((_, s)) = sections.get(k) {
                    let o = get(s);
                    set.default = set.default.or(o.default);
                    set.first = set.first.or(o.first);
                    set.even = set.even.or(o.even);
                }
            }
            if sect.title_page && first_of_section.contains(&i) {
                return set.first;
            }
            if even_odd && page.number % 2 == 0 {
                return set.even.or(set.default);
            }
            set.default
        };
        let hid = pick(&|s: &SectionProps| s.headers);
        let fid = pick(&|s: &SectionProps| s.footers);
        ctx.fields.page = page.number;
        ctx.fields.page_format = sect.page_num_format;
        ctx.fields.pages = total;
        ctx.fields.section_pages = per_section.get(&page.section).copied().unwrap_or(1);
        ctx.fields.section = page.section as u32 + 1;
        let w = sect.text_width();
        let x = sect.margin_left + sect.gutter;
        if let Some(id) = hid
            && let Some(part) = ctx.doc.parts.get(&id)
        {
            let blocks = part.blocks.clone();
            let (mut items, _) = layout_box(ctx, StoryRef::Part(id), &blocks, &[], w, None, 0);
            for it in &mut items {
                it.translate(x, sect.header);
            }
            page.header = items;
            page.header_story = Some(id);
        }
        if let Some(id) = fid
            && let Some(part) = ctx.doc.parts.get(&id)
        {
            let blocks = part.blocks.clone();
            let (mut items, h) = layout_box(ctx, StoryRef::Part(id), &blocks, &[], w, None, 0);
            for it in &mut items {
                it.translate(x, sect.page_h - sect.footer - h);
            }
            page.footer = items;
            page.footer_story = Some(id);
        }
    }
}

/// Is an object floating (not laid out inline)?
pub fn is_floating(o: &InlineObject) -> bool {
    match o {
        InlineObject::Image { float, .. } | InlineObject::Shape { float, .. } => float.wrap != Wrap::Inline,
        _ => false,
    }
}

/// Milliseconds since an arbitrary epoch (monotonic on native; 0 on wasm).
pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

#[cfg(test)]
mod tests;
