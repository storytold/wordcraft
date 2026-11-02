//! Screen readers on Windows read the document as one text, the way Word exposes its own. The
//! canvas's document node carries the whole body as an AccessKit `text_document` (a property of
//! the AccessKit fork in the workspace's `[patch.crates-io]`), which AccessKit's Windows adapter
//! serves as one UI Automation text pattern. So a screen reader moves by character, word, line
//! and paragraph through the whole document, not only the pages on screen; finds headings by their
//! style and links, lists and tables as elements; reads the formatting and spelling errors; and
//! echoes typing, as the text isn't read-only (#3174).
//!
//! The text is the body's paragraphs in reading order, table cells included, each ended by `\r`;
//! a list item's starts with its number or bullet and a space, as Word's does. Word starts are
//! where Ctrl+Right stops, and the paragraph mark is a word of its own. It is rebuilt only when
//! the layout changes.
//!
//! Elsewhere the document is described node by node (`crate::a11y`): the other AccessKit adapters
//! don't read `text_document`.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::accesskit::text_document::{Align, Annotation, LineBox, LineStyle, Look, Object, ObjectKind, ParaKind};
use egui::accesskit::{self, Role, TextDocument, TextDocumentRef, TextPosition, TextSelection};
use egui::{Id, Rect};
use wordcraft_doc::para::OBJ;
use wordcraft_doc::props::{Align as DocAlign, LineSpacing, Rgb, Underline, VertAlign};
use wordcraft_doc::resolve::{ResolvedChar, ResolvedPara};
use wordcraft_doc::{Block, Document, InlineObject, Path, Pos, StoryRef};
use wordcraft_geom::Unit;
use wordcraft_layout::display::text_color;
use wordcraft_layout::{DocLayout, ParaLayout, Placed};
use wordcraft_render::RenderOptions;

/// UI Automation's paragraph style ids (`StyleId_*`): Heading 1 to 9 follow `HEADING_1`.
const STYLE_OTHER: i32 = 70000;
const STYLE_HEADING_1: i32 = 70001;
const STYLE_TITLE: i32 = 70010;
const STYLE_SUBTITLE: i32 = 70011;
const STYLE_NORMAL: i32 = 70012;
const STYLE_BULLETED_LIST: i32 = 70015;
const STYLE_NUMBERED_LIST: i32 = 70016;

/// The document as text, and what it was built from.
#[derive(Default)]
pub struct TextCache {
    layout: Option<Arc<DocLayout>>,
    /// Built from Insert › Table's preview copy, which is not the document (#3931).
    previewing: bool,
    /// The session's revision it was built at: positions from an older text are not the
    /// document's.
    rev: u64,
    /// The view it was built for: its words and colours change with it, not with the layout.
    shown: Option<Shown>,
    text: Option<TextDocumentRef>,
    map: Map,
}

/// What the text takes from the view rather than the layout: the interface language (style
/// names), the region's decimal separator (line spacing), and how the page is painted (colours).
#[derive(Clone, Copy, PartialEq)]
struct Shown {
    lang: &'static str,
    decimal: char,
    dark: bool,
    paper: Rgb,
}

/// Describe the document under the canvas widget `doc_id` to AccessKit as one text for this
/// frame, after moving the caret where a screen reader asked. `rects` are the pages' screen
/// rectangles, `view` the visible part of the canvas.
pub fn expose(app: &mut crate::WordApp, ctx: &egui::Context, doc_id: Id, layout: &Arc<DocLayout>, rects: &[Rect], view: Rect) {
    if ctx.accesskit_node_builder(doc_id, |n| n.set_role(Role::Document)).is_none() {
        // No screen reader: describe nothing, keep nothing.
        app.canvas.a11y.text = TextCache::default();
        return;
    }
    // A selection a screen reader set (UI Automation's Select, as NVDA does on every move in
    // browse mode and JAWS on Ctrl+Alt+arrows in a table). Its offsets are those of the text it
    // was given, so it waits while that text is older than the document or is the preview's.
    let wanted = ctx.input(|i| {
        i.accesskit_action_requests(doc_id, accesskit::Action::SetTextSelection)
            .filter_map(|r| match &r.data {
                Some(accesskit::ActionData::SetTextSelection(s)) => Some(*s),
                _ => None,
            })
            .last()
    });
    let cache = &app.canvas.a11y.text;
    if let Some(s) = wanted
        && cache.text.is_some()
        && !cache.previewing
        && cache.rev == app.session.rev()
        && crate::canvas::preview_sel(app).is_none()
        && let Some((anchor, focus)) = cache.map.selection(s.anchor.character_index, s.focus.character_index)
    {
        // A position the document no longer has is refused, and the caret stays.
        let _ = app.run("select.range", serde_json::json!({"anchor": anchor, "focus": focus}));
    }
    let preview = crate::canvas::preview_sel(app).is_some();
    let opts = crate::canvas::page_render_options(app, false);
    let shown = Shown {
        lang: crate::i18n::current().code(),
        decimal: crate::options::decimal(),
        dark: opts.dark,
        paper: opts.paper_of(crate::canvas::page_doc(app)),
    };
    let cache = &app.canvas.a11y.text;
    if !cache.layout.as_ref().is_some_and(|l| Arc::ptr_eq(l, layout)) || cache.previewing != preview || cache.shown != Some(shown) {
        let (text, map) = build(crate::canvas::page_doc(app), layout, &opts);
        app.canvas.a11y.text =
            TextCache { layout: Some(layout.clone()), previewing: preview, rev: app.session.rev(), shown: Some(shown), text: Some(text.into()), map };
    }
    let sel = crate::canvas::preview_sel(app).unwrap_or(&app.session.sel);
    let cache = &app.canvas.a11y.text;
    let selection = cache.map.offset(&sel.anchor).zip(cache.map.offset(&sel.focus));
    let text = cache.text.clone();
    // Each page's rectangle in the node's coordinates (egui points), and points per page point.
    let pages: Vec<f32> = rects
        .iter()
        .zip(&layout.pages)
        .flat_map(|(r, p)| [r.min.x, r.min.y, r.max.x, r.max.y, if p.w > 0.0 { r.width() / p.w } else { 1.0 }])
        .collect();
    let title = app.title_stem();
    let node = doc_id.accesskit_id();
    ctx.accesskit_node_builder(doc_id, |n| {
        // The part of the document on screen, as Word's document element's: the canvas widget's
        // own rectangle is all of its scrolled content, which would make every page "visible".
        n.set_bounds(accesskit::Rect { x0: view.min.x.into(), y0: view.min.y.into(), x1: view.max.x.into(), y1: view.max.y.into() });
        n.set_label(title);
        if let Some(text) = text {
            n.set_text_document(text);
        }
        n.set_page_bounds(pages);
        match selection {
            Some((a, f)) => n.set_text_selection(TextSelection {
                anchor: TextPosition { node, character_index: a },
                focus: TextPosition { node, character_index: f },
            }),
            None => n.clear_text_selection(),
        }
    });
}

/// Where each body paragraph's text is in the document's text, to turn positions into offsets
/// and back.
#[derive(Default)]
pub struct Map {
    /// Each paragraph: its path, where it starts (with its number or bullet), where its own text
    /// starts, and the UTF-16 offset of each byte of its text from there.
    paras: Vec<(Path, usize, usize, Vec<usize>)>,
    index: HashMap<Path, usize>,
}

impl Map {
    /// The offset of a body position (the caret goes after a list item's number or bullet).
    pub fn offset(&self, pos: &Pos) -> Option<usize> {
        if pos.story != StoryRef::Body {
            return None;
        }
        let (_, _, base, map) = self.paras.get(*self.index.get(&pos.path)?)?;
        Some(base + map.get(pos.off).or(map.last()).copied().unwrap_or(0))
    }

    /// The body position at an offset.
    pub fn pos(&self, off: usize) -> Option<Pos> {
        let k = self.paras.partition_point(|p| p.1 <= off).checked_sub(1)?;
        let (path, _, base, map) = self.paras.get(k)?;
        let u = off.saturating_sub(*base);
        let byte = map.partition_point(|m| *m < u).min(map.len().saturating_sub(1));
        Some(Pos::new(StoryRef::Body, path.clone(), byte))
    }

    /// The body position where a range ending at `off` ends. A range that takes a paragraph's mark
    /// ends with its text, not at the next paragraph's start, which may be in another table cell
    /// (screen readers select a whole cell, mark included).
    pub fn end_pos(&self, off: usize) -> Option<Pos> {
        let k = self.paras.partition_point(|p| p.1 <= off).checked_sub(1)?;
        if k > 0 && self.paras.get(k)?.1 == off {
            let (path, _, _, map) = self.paras.get(k - 1)?;
            return Some(Pos::new(StoryRef::Body, path.clone(), map.len().saturating_sub(1)));
        }
        self.pos(off)
    }

    /// The selection from anchor and focus offsets.
    pub fn selection(&self, anchor: usize, focus: usize) -> Option<(Pos, Pos)> {
        let end = |off: usize| if off > anchor { self.end_pos(off) } else { self.pos(off) };
        Some((self.pos(anchor)?, end(focus)?))
    }
}

/// The document as one text: paragraphs in reading order (table cells included), each ended by
/// `\r`, with their lines (and where they sit on their pages), pages, words, formatting (its
/// colours as `opts` paints them) and spelling errors, and the links, lists and tables in it as
/// elements.
pub fn build(doc: &Document, layout: &DocLayout, opts: &RenderOptions) -> (TextDocument, Map) {
    let paper = opts.paper_of(doc);
    let mut b = Build { doc, layout, opts, paper, s: TextDocument::default(), last_look: None, pages: Vec::new(), map: Map::default() };
    // Each section around its blocks, so a screen reader can say which one the text is in: a
    // section ends with the paragraph that holds its break, the last with the body.
    let mut first = 0;
    for (end, _) in doc.sections() {
        if let Some(part) = doc.body.get(first..=end).filter(|p| !p.is_empty()) {
            let start = b.s.text.len();
            let children = b.blocks(part, first, &[]);
            b.s.objects.push(Object { kind: ObjectKind::Section, range: start..b.s.text.len(), children });
        }
        first = first.max(end.saturating_add(1));
    }
    let Build { mut s, mut pages, mut map, .. } = b;
    // A page starts at the first text placed on it.
    pages.sort_unstable();
    pages.dedup_by_key(|p| p.0);
    s.pages = pages.into_iter().map(|p| p.1).collect();
    s.lines.sort_unstable();
    s.lines.dedup();
    s.boxes.sort_by_key(|b| b.start);
    map.index = map.paras.iter().enumerate().map(|(i, p)| (p.0.clone(), i)).collect();
    (s, map)
}

struct Build<'a> {
    doc: &'a Document,
    layout: &'a DocLayout,
    opts: &'a RenderOptions,
    /// The paper under the text, on screen.
    paper: Rgb,
    s: TextDocument,
    last_look: Option<Look>,
    /// (page, where its first text starts).
    pages: Vec<(usize, usize)>,
    map: Map,
}

impl Build<'_> {
    /// The blocks' text, and their elements; the first block is number `first` under `prefix`.
    fn blocks(&mut self, blocks: &[Arc<Block>], first: usize, prefix: &[u32]) -> Vec<Object> {
        let mut out = Vec::new();
        let mut list: Option<Object> = None;
        for (i, b) in blocks.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(u32::try_from(first.saturating_add(i)).unwrap_or(u32::MAX));
            match &**b {
                Block::Para(_) => {
                    let (range, item, links) = self.para(&Path(path));
                    if item {
                        let l = list.get_or_insert_with(|| Object { kind: ObjectKind::List, range: range.clone(), children: Vec::new() });
                        l.range.end = range.end;
                        l.children.push(Object { kind: ObjectKind::ListItem, range, children: links });
                    } else {
                        out.extend(list.take());
                        out.extend(links);
                    }
                }
                Block::Table(t) => {
                    out.extend(list.take());
                    let start = self.s.text.len();
                    let mut cells = Vec::new();
                    for (r, row) in t.rows.iter().enumerate() {
                        // The cell's grid column: a merged cell covers `span()` columns. Its path
                        // stays its index in the row, which is how the document finds it.
                        let mut col = 0usize;
                        for (c, cell) in row.cells.iter().enumerate() {
                            let mut p = path.clone();
                            p.push(u32::try_from(r).unwrap_or(u32::MAX));
                            p.push(u32::try_from(c).unwrap_or(u32::MAX));
                            let cs = self.s.text.len();
                            let children = self.blocks(&cell.blocks, 0, &p);
                            cells.push(Object { kind: ObjectKind::Cell { row: r, col }, range: cs..self.s.text.len(), children });
                            col = col.saturating_add(cell.span());
                        }
                    }
                    let cols = t.rows.iter().map(|r| r.cells.iter().map(|c| c.span()).fold(0usize, usize::saturating_add)).max().unwrap_or(0);
                    out.push(Object { kind: ObjectKind::Table { rows: t.rows.len(), cols }, range: start..self.s.text.len(), children: cells });
                }
            }
        }
        out.extend(list);
        out
    }

    /// One paragraph's text and units: its range, whether it is a list item, and its links and
    /// pictures.
    fn para(&mut self, path: &Path) -> (Range<usize>, bool, Vec<Object>) {
        let (doc, layout) = (self.doc, self.layout);
        let start = self.s.text.len();
        let Some(p) = doc.para(StoryRef::Body, path) else { return (start..start, false, Vec::new()) };
        let pieces: Vec<(usize, &Arc<ParaLayout>, usize, usize, f32, f32)> = layout
            .index
            .get(&(StoryRef::Body, path.clone()))
            .into_iter()
            .flatten()
            .filter_map(|(pg, it)| match layout.pages.get(*pg)?.items.get(*it)? {
                Placed::Lines { para, l0, l1, x, y, .. } => Some((*pg, para, *l0, *l1, *x, *y)),
                _ => None,
            })
            .collect();
        let pl = pieces.first().map(|p| p.1);
        // A list item's number or bullet (symbol-font bullets are private-use characters).
        let label = pl
            .and_then(|pl| pl.label.as_ref())
            .map(|l| l.text.chars().map(|c| if ('\u{E000}'..='\u{F8FF}').contains(&c) { '•' } else { c }).collect::<String>())
            .filter(|t| !t.trim().is_empty());
        let item = label.is_some() && pl.is_some_and(|pl| pl.rp.numbering.is_some_and(|n| n.num != 0));
        let s = &mut self.s;
        if let Some(l) = &label {
            s.text.extend(l.encode_utf16());
            s.text.push(u16::from(b' '));
        }
        let base = s.text.len();
        let map = utf16_offsets(&p.text);
        // A byte of the text; the text's start is the paragraph's, so the label goes with it.
        let at = |byte: usize| if byte == 0 { start } else { base + map.get(byte).copied().unwrap_or_else(|| map.last().copied().unwrap_or(0)) };
        s.text.extend(p.text.encode_utf16());
        s.text.push(u16::from(b'\r'));
        let k = match pl {
            Some(pl) => kind(doc, &pl.rp, label.as_deref()),
            None => ParaKind {
                style_id: STYLE_NORMAL,
                style_name: crate::i18n::builtin_style_name("Normal"),
                align: Align::Left,
                line_spacing: line_spacing(LineSpacing::default()),
            },
        };
        s.paragraphs.push((start, k));
        for (pg, para, l0, l1, x, y) in &pieces {
            let top0 = para.lines.get(*l0).map_or(0.0, |l| l.top);
            for line in para.lines.get(*l0..*l1).unwrap_or_default() {
                s.lines.push(at(line.start));
                let top = y + line.top - top0;
                let mut stops: Vec<(usize, f32)> =
                    (line.c0..line.c1).filter_map(|c| Some((at(para.clusters.get(c)?.start), x + line.xs.get(c - line.c0).copied()?))).collect();
                // The line's end: past its last character (the paragraph mark on the last line).
                let end = if line.stop >= p.text.len() { base + map.last().copied().unwrap_or(0) + 1 } else { at(line.stop) };
                stops.push((end, x + line.xs.last().copied().unwrap_or(0.0)));
                s.boxes.push(LineBox { start: at(line.start), end, page: *pg, top, bottom: top + line.height, stops });
            }
            if let Some(first) = para.lines.get(*l0) {
                self.pages.push((*pg, at(first.start)));
            }
        }
        self.map.paras.push((path.clone(), start, base, map.clone()));
        let mut off = 0;
        loop {
            let next = p.word_end(off);
            if next <= off || next >= p.text.len() {
                break;
            }
            s.words.push(at(next));
            off = next;
        }
        // The paragraph mark is a word of its own, as in Word.
        if !p.text.is_empty() {
            s.words.push(at(p.text.len()));
        }
        if let Some(pl) = pl {
            for cl in &pl.clusters {
                let Some(st) = pl.styles.get(cl.style as usize) else { continue };
                let l = look(&st.rc, self.opts, self.paper);
                if self.last_look.as_ref() != Some(&l) {
                    s.looks.push((at(cl.start), l.clone()));
                    self.last_look = Some(l);
                }
            }
            for (a, b, grammar) in &pl.issues {
                s.annotations.push((at(*a)..at(*b), if *grammar { Annotation::Grammar } else { Annotation::Spelling }));
            }
        }
        // Links: runs with the same target next to each other are one link.
        let mut links: Vec<Object> = Vec::new();
        let mut byte = 0usize;
        for r in &p.runs {
            let end = byte.saturating_add(r.len);
            if let Some(url) = &r.props.link {
                match links.last_mut() {
                    Some(Object { kind: ObjectKind::Link { url: u }, range, .. }) if range.end == at(byte) && u == url => range.end = at(end),
                    _ => links.push(Object { kind: ObjectKind::Link { url: url.clone() }, range: at(byte)..at(end), children: Vec::new() }),
                }
            }
            byte = end;
        }
        // Pictures and charts: elements on their object character, named by their alt text, as
        // screen readers find them in Word (a linked picture is inside its link). Decorative ones
        // add nothing and stay out.
        for off in p.object_offsets() {
            let Some(o @ (InlineObject::Image { .. } | InlineObject::Graphic { .. })) = p.object_at(off) else { continue };
            if o.is_decorative() {
                continue;
            }
            let range = at(off)..at(off.saturating_add(OBJ.len_utf8()));
            let picture = Object { kind: ObjectKind::Image { alt: o.alt_text().to_string() }, range, children: Vec::new() };
            match links.iter_mut().find(|l| l.range.start <= picture.range.start && picture.range.end <= l.range.end) {
                Some(link) => link.children.push(picture),
                None => links.push(picture),
            }
        }
        links.sort_by_key(|o| o.range.start);
        (start..s.text.len(), item, links)
    }
}

/// UTF-16 offset of each byte of `s` (and of its end).
fn utf16_offsets(s: &str) -> Vec<usize> {
    let mut out = vec![0; s.len() + 1];
    let mut u = 0;
    for (i, c) in s.char_indices() {
        for slot in out.iter_mut().skip(i).take(c.len_utf8()) {
            *slot = u;
        }
        u += c.len_utf16();
    }
    if let Some(last) = out.last_mut() {
        *last = u;
    }
    out
}

/// How a run looks, as UI Automation's text attributes say it. Its colours are those on screen, as
/// Word's are: automatic text black (white on a dark highlight or shading), on its highlight or
/// shading or else the paper, all as a dark page turns them.
fn look(rc: &ResolvedChar, opts: &RenderOptions, paper: Rgb) -> Look {
    let behind = rc.shading.or(rc.highlight);
    let color = opts.ink_text(text_color(&rc.color, behind), behind);
    let background = behind.map_or(paper, |b| opts.ink_bg(b));
    let rgb = |c: Rgb| accesskit::Color { red: c.0, green: c.1, blue: c.2, alpha: 255 };
    Look {
        font: rc.font.clone(),
        size: rc.size,
        weight: if rc.bold { 700 } else { 400 },
        italic: rc.italic,
        underline: match rc.underline {
            Underline::None => None,
            Underline::Single | Underline::Words | Underline::Thick => Some(LineStyle::Single),
            Underline::Double => Some(LineStyle::Double),
            Underline::Dotted => Some(LineStyle::Dotted),
            Underline::Dash | Underline::DotDash | Underline::DotDotDash => Some(LineStyle::Dashed),
            Underline::Wave | Underline::DoubleWave => Some(LineStyle::Wavy),
        },
        strike: if rc.double_strike { Some(LineStyle::Double) } else { rc.strike.then_some(LineStyle::Single) },
        superscript: rc.vert_align == VertAlign::Superscript,
        subscript: rc.vert_align == VertAlign::Subscript,
        color: Some(rgb(color)),
        background: Some(rgb(background)),
    }
}

/// A paragraph's line spacing as WordCraft shows it: a multiple as the Line Spacing menu and the
/// Paragraph dialog do (`1.15`, with the region's decimal separator), points as a table row's
/// height does (`At least 12 pt`).
fn line_spacing(spacing: LineSpacing) -> String {
    let points = |pt: f32| crate::options::length_in(Unit::Points, pt);
    match spacing {
        // A multiple written as points are: two decimals at most, trailing zeros dropped.
        LineSpacing::Multiple(m) => Unit::Points.number(m, crate::options::decimal()),
        LineSpacing::AtLeast(pt) => format!("{} {}", tl!("At least"), points(pt)),
        LineSpacing::Exactly(pt) => format!("{} {}", tl!("Exactly"), points(pt)),
    }
}

/// What a paragraph is: a heading by its outline level, a bulleted or numbered list item by its
/// label, otherwise its style; with the style's name as the interface shows it, the alignment seen
/// on the page and the line spacing.
fn kind(doc: &Document, rp: &ResolvedPara, label: Option<&str>) -> ParaKind {
    let style_id = match (rp.outline_level.filter(|l| *l < 9), rp.numbering.filter(|n| n.num != 0).and(label)) {
        (Some(l), _) => STYLE_HEADING_1 + i32::from(l),
        (None, Some(label)) if !label.chars().any(char::is_alphanumeric) => STYLE_BULLETED_LIST,
        (None, Some(_)) => STYLE_NUMBERED_LIST,
        (None, None) => match rp.style.as_str() {
            "Title" => STYLE_TITLE,
            "Subtitle" => STYLE_SUBTITLE,
            "" | "Normal" => STYLE_NORMAL,
            _ => STYLE_OTHER,
        },
    };
    ParaKind {
        style_id,
        style_name: doc.styles.get(&rp.style).map_or_else(|| rp.style.clone(), crate::i18n::style_name),
        align: match rp.align.visual(rp.bidi) {
            DocAlign::Left => Align::Left,
            DocAlign::Center => Align::Center,
            DocAlign::Right => Align::Right,
            DocAlign::Justify | DocAlign::Distribute => Align::Justify,
        },
        line_spacing: line_spacing(rp.line_spacing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::{Node, NodeId};
    use serde_json::json;

    /// A heading, a paragraph with a link, a bulleted item and a 2 × 2 table.
    fn sample() -> crate::WordApp {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        for (cmd, p) in [
            ("text.insert", json!({"text": "Title here"})),
            ("para.style", json!({"style": "Heading1"})),
            ("text.newParagraph", json!({})),
            ("para.style", json!({"style": "Normal"})),
            ("text.insert", json!({"text": "Read this "})),
            ("insert.link", json!({"url": "https://example.com", "text": "link"})),
            ("text.newParagraph", json!({})),
            ("text.insert", json!({"text": "First item"})),
            ("para.bullets", json!({})),
            ("text.newParagraph", json!({})),
            ("para.bullets", json!({})),
            ("insert.table", json!({"rows": 2, "cols": 2})),
            ("text.insert", json!({"text": "Cell A"})),
        ] {
            app.run(cmd, p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        }
        app
    }

    /// Every element in the text, depth first.
    fn every(objects: &[Object]) -> Vec<&Object> {
        objects.iter().flat_map(|o| std::iter::once(o).chain(every(&o.children))).collect()
    }

    /// One frame of `app` in `ctx` with AccessKit on, the document described as text, and
    /// `events`: what AccessKit was told.
    fn frame(app: &mut crate::WordApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Option<accesskit::TreeUpdate> {
        ctx.enable_accesskit();
        app.canvas.a11y_as_text = true;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            time: Some(ctx.input(|i| i.time) + 0.1),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        out.platform_output.accesskit_update.take()
    }

    /// The document node after a few frames.
    fn document(app: &mut crate::WordApp, ctx: &egui::Context) -> (NodeId, Node) {
        let mut update = None;
        for _ in 0..4 {
            update = frame(app, ctx, Vec::new()).or(update);
        }
        let update = update.expect("AccessKit output");
        update.nodes.into_iter().find(|(_, n)| n.role() == Role::Document).expect("a document node")
    }

    /// A frame in which a screen reader selects `anchor..focus` of the document's text.
    fn request(app: &mut crate::WordApp, ctx: &egui::Context, node: NodeId, anchor: usize, focus: usize) {
        let at = |character_index| TextPosition { node, character_index };
        let request = accesskit::ActionRequest {
            action: accesskit::Action::SetTextSelection,
            target_tree: accesskit::TreeId::ROOT,
            target_node: node,
            data: Some(accesskit::ActionData::SetTextSelection(TextSelection { anchor: at(anchor), focus: at(focus) })),
        };
        frame(app, ctx, vec![egui::Event::AccessKitActionRequest(request)]);
    }

    fn selection(app: &crate::WordApp) -> (Pos, Pos) {
        (app.session.sel.anchor.clone(), app.session.sel.focus.clone())
    }

    /// The canvas's document node carries the whole document as one text: the heading, the
    /// paragraph and the list item by their style ids, the link, the list and the table as
    /// elements, words where Ctrl+Right stops, and the caret as an offset into it.
    #[test]
    fn the_document_is_one_text_with_its_elements() {
        let mut app = sample();
        // The caret: in "Read this ", after "Read".
        app.run("select.text", json!({"text": "Read"})).unwrap();
        app.run("caret.right", json!({})).unwrap();
        let (id, node) = document(&mut app, &egui::Context::default());
        let doc = node.text_document().expect("the document as text");
        let text = String::from_utf16_lossy(&doc.text);
        assert!(text.starts_with("Title here\rRead this link\r"), "{text:?}");
        assert!(text.contains("First item\r") && text.contains("Cell A\r"), "{text:?}");
        // Not described node by node as well.
        assert!(node.children().is_empty(), "{:?}", node.children());

        let styles: Vec<i32> = doc.paragraphs.iter().map(|p| p.1.style_id).collect();
        assert_eq!(styles.get(..3), Some(&[STYLE_HEADING_1, STYLE_NORMAL, STYLE_BULLETED_LIST][..]), "{styles:?}");

        // "Read this link\r" starts at 11: words after "Read " and "this ", and its mark.
        for w in [16, 21, 25] {
            assert!(doc.words.contains(&w), "word at {w}: {:?}", doc.words);
        }
        let link = every(&doc.objects).into_iter().find(|o| matches!(o.kind, ObjectKind::Link { .. })).expect("a link");
        assert_eq!(link.range, 21..25);
        assert_eq!(link.kind, ObjectKind::Link { url: "https://example.com".into() });
        let list = every(&doc.objects).into_iter().find(|o| o.kind == ObjectKind::List).expect("a list");
        assert_eq!(list.children.iter().map(|c| c.kind.clone()).collect::<Vec<_>>(), vec![ObjectKind::ListItem]);
        let table = every(&doc.objects).into_iter().find(|o| o.kind == ObjectKind::Table { rows: 2, cols: 2 }).expect("a 2 × 2 table");
        assert_eq!(table.children.len(), 4);
        let cell = table.children.first().expect("a cell");
        assert_eq!(cell.kind, ObjectKind::Cell { row: 0, col: 0 });
        assert_eq!(String::from_utf16_lossy(doc.text.get(cell.range.clone()).unwrap_or_default()), "Cell A\r");

        assert!(!doc.lines.is_empty() && !doc.boxes.is_empty());
        let pages = node.page_bounds().unwrap_or_default();
        assert!(!pages.is_empty() && pages.len() % 5 == 0, "{pages:?}");

        let s = node.text_selection().expect("the caret");
        assert_eq!((s.anchor.node, s.anchor.character_index, s.focus.node, s.focus.character_index), (id, 15, id, 15));
    }

    /// A screen reader moves the caret and the selection by offsets into the text, as NVDA does
    /// on every move in browse mode.
    #[test]
    fn a_screen_reader_moves_the_caret() {
        let mut app = sample();
        let ctx = egui::Context::default();
        let (id, _) = document(&mut app, &ctx);
        request(&mut app, &ctx, id, 21, 25);
        assert_eq!(selection(&app), (Pos::body(1, 10), Pos::body(1, 14)));
        request(&mut app, &ctx, id, 0, 0);
        assert_eq!(selection(&app), (Pos::body(0, 0), Pos::body(0, 0)));
    }

    /// After an edit, a request still in the old text's offsets waits: it would land elsewhere.
    #[test]
    fn a_request_on_an_older_text_waits() {
        let mut app = sample();
        let ctx = egui::Context::default();
        let (id, _) = document(&mut app, &ctx);
        // "!Read this link": the old text's 21..25 would now be " lin".
        app.run("select.text", json!({"text": "Read"})).unwrap();
        app.run("caret.left", json!({})).unwrap();
        app.run("text.insert", json!({"text": "!"})).unwrap();
        let before = selection(&app);
        assert_eq!(before, (Pos::body(1, 1), Pos::body(1, 1)));
        request(&mut app, &ctx, id, 21, 25);
        assert_eq!(selection(&app), before);
        // Once the text has the edit, the link is one further on.
        document(&mut app, &ctx);
        request(&mut app, &ctx, id, 22, 26);
        assert_eq!(selection(&app), (Pos::body(1, 11), Pos::body(1, 15)));
    }

    /// A whole cell, its mark included (as screen readers select one), ends in that cell, not at
    /// the next cell's start.
    #[test]
    fn a_whole_cell_ends_in_its_cell() {
        let mut app = sample();
        let (_, node) = document(&mut app, &egui::Context::default());
        let doc = node.text_document().expect("the document as text");
        let cell = every(&doc.objects)
            .into_iter()
            .find_map(|o| match o.kind {
                ObjectKind::Table { .. } => o.children.first().cloned(),
                _ => None,
            })
            .expect("a cell");
        let (anchor, focus) = app.canvas.a11y.text.map.selection(cell.range.start, cell.range.end).expect("a selection");
        assert_eq!(anchor.path, focus.path);
        assert_eq!((anchor.off, focus.off), (0, "Cell A".len()));
    }

    /// CodeRabbit on #4951: a merged cell covers several grid columns, so the cells after it are
    /// numbered by grid column and the table counts the grid's columns. Their text still maps
    /// back to the document, which knows a cell by its place in the row.
    #[test]
    fn merged_cells_take_their_grid_columns() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("document.setText", json!({"text": "Intro"})).unwrap();
        app.run("caret.docEnd", json!({})).unwrap();
        app.run("insert.table", json!({"rows": 2, "cols": 3})).unwrap();
        for (i, t) in ["A1", "B1", "C1", "A2", "B2", "C2"].iter().enumerate() {
            if i > 0 {
                app.run("text.tab", json!({})).unwrap();
            }
            app.run("text.insert", json!({"text": t})).unwrap();
        }
        // A1 and B1 merged.
        let at = |c: u32| serde_json::to_value(Pos::new(StoryRef::Body, Path(vec![1, 0, c, 0]), 0)).unwrap();
        app.run("select.range", json!({"anchor": at(0), "focus": at(1)})).unwrap();
        app.run("table.merge", json!({})).unwrap();
        let (_, node) = document(&mut app, &egui::Context::default());
        let doc = node.text_document().expect("the document as text");
        let table = every(&doc.objects).into_iter().find(|o| matches!(o.kind, ObjectKind::Table { .. })).expect("a table");
        assert_eq!(table.kind, ObjectKind::Table { rows: 2, cols: 3 });
        let cells: Vec<ObjectKind> = table.children.iter().map(|c| c.kind.clone()).collect();
        let cell = |row, col| ObjectKind::Cell { row, col };
        assert_eq!(cells, vec![cell(0, 0), cell(0, 2), cell(1, 0), cell(1, 1), cell(1, 2)]);
        let c1 = table.children.get(1).expect("the cell after the merge");
        assert_eq!(String::from_utf16_lossy(doc.text.get(c1.range.clone()).unwrap_or_default()), "C1\r");
        let (anchor, _) = app.canvas.a11y.text.map.selection(c1.range.start, c1.range.start).expect("a position");
        assert_eq!(anchor.path, Path(vec![1, 0, 1, 0]));
    }

    /// What NVDA+F reads: the style's name as the interface shows it, the line spacing as the
    /// Line Spacing menu writes it, and the colours on screen, which a dark page turns.
    #[test]
    fn formatting_reads_as_the_interface_and_the_screen_show_it() {
        let mut app = sample();
        app.run("select.text", json!({"text": "Read"})).unwrap();
        app.run("para.lineSpacing", json!({"value": 1.15})).unwrap();
        app.run("format.color", json!({"color": "C00000"})).unwrap();
        app.run("select.text", json!({"text": "Title here"})).unwrap();
        app.run("para.lineSpacing", json!({"atLeast": 14})).unwrap();
        app.run("select.text", json!({"text": "this"})).unwrap();
        app.run("format.highlight", json!({"color": "yellow"})).unwrap();
        let ctx = egui::Context::default();
        let (_, node) = document(&mut app, &ctx);
        let doc = node.text_document().expect("the document as text");
        let para = |i: usize| doc.paragraphs.get(i).map(|p| p.1.clone()).expect("a paragraph");
        assert_eq!((para(0).style_name.as_str(), para(0).line_spacing.as_str()), ("Heading 1", "At least 14 pt"));
        assert_eq!((para(1).style_name.as_str(), para(1).line_spacing.as_str()), ("Normal", "1.15"));
        // In another interface language, the interface's names (the app sets its own language
        // each frame, so the text is built here directly).
        crate::i18n::set_current(crate::i18n::fixture_lang(
            "x-fixture",
            "\tNormal\tNormal-FX\n\tHeading {n}\tHeading-FX {n}\n\tAt least\tAt-least-FX\n",
        ));
        let layout = app.session.layout();
        let (built, _) = build(&app.session.doc, &layout, &crate::canvas::page_render_options(&app, false));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let names: Vec<(&str, &str)> = built.paragraphs.iter().take(2).map(|p| (p.1.style_name.as_str(), p.1.line_spacing.as_str())).collect();
        assert_eq!(names, [("Heading-FX 1", "At-least-FX 14 pt"), ("Normal-FX", "1.15")]);
        let rgb = |red, green, blue| Some(accesskit::Color { red, green, blue, alpha: 255 });
        let look = |off: usize| doc.looks.iter().rev().find(|l| l.0 <= off).map(|l| l.1.clone()).expect("a look");
        // "Read this link\r" starts at 11: "Read" red on the paper, "this" on its highlight.
        assert_eq!((look(11).color, look(11).background), (rgb(0xC0, 0, 0), rgb(0xFF, 0xFF, 0xFF)));
        assert_eq!(look(16).background, rgb(0xFF, 0xFF, 0x00));
        // Automatic text is black on white (the heading has its style's colour).
        let text = String::from_utf16_lossy(&doc.text);
        let item = text.find("First item").map(|b| text[..b].encode_utf16().count()).expect("the list item");
        assert_eq!((look(item).color, look(item).background), (rgb(0, 0, 0), rgb(0xFF, 0xFF, 0xFF)));
        assert_eq!(look(0).color, rgb(0x0F, 0x47, 0x61));

        // A dark page: the text is rebuilt with its colours though the layout is the same.
        app.run("view.darkMode", json!({})).unwrap();
        let (_, node) = document(&mut app, &ctx);
        let doc = node.text_document().expect("the document as text");
        let dark = doc.looks.iter().rev().find(|l| l.0 <= item).map(|l| l.1.clone()).expect("a look");
        let opts = crate::canvas::page_render_options(&app, false);
        let paper = opts.paper_of(&app.session.doc);
        assert_eq!(dark.background, rgb(paper.0, paper.1, paper.2));
        // Automatic text turns light on the dark paper.
        assert!(paper.luma() < 100.0 && dark.color.is_some_and(|c| c.red > 0xC0), "{dark:?} on {paper:?}");
    }

    /// Each section is an element around its text, so a screen reader can say which section the
    /// text is in; what is inside it keeps its own elements.
    #[test]
    fn sections_are_elements_around_their_text() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("document.setText", json!({"text": "One"})).unwrap();
        app.run("caret.docEnd", json!({})).unwrap();
        app.run("layout.break", json!({"kind": "nextPage"})).unwrap();
        app.run("text.insert", json!({"text": "Two "})).unwrap();
        app.run("insert.link", json!({"url": "https://example.com", "text": "link"})).unwrap();
        let (_, node) = document(&mut app, &egui::Context::default());
        let doc = node.text_document().expect("the document as text");
        let text = |o: &Object| String::from_utf16_lossy(doc.text.get(o.range.clone()).unwrap_or_default());
        let sections: Vec<(ObjectKind, String)> = doc.objects.iter().map(|o| (o.kind.clone(), text(o))).collect();
        assert_eq!(sections, [(ObjectKind::Section, "One\r".to_string()), (ObjectKind::Section, "Two link\r".to_string())]);
        let inside: Vec<String> = doc.objects.get(1).map(|o| o.children.iter().map(text).collect()).unwrap_or_default();
        assert_eq!(inside, ["link"]);
        // The second section's text maps back to its paragraph.
        let start = doc.objects.get(1).map(|o| o.range.start).unwrap_or_default();
        assert_eq!(app.canvas.a11y.text.map.pos(start).map(|p| p.path), Some(Path(vec![1])));
    }

    /// A picture is an element on its object character, named by its alt text, as screen readers
    /// find graphics in Word; a decorative one isn't.
    #[test]
    fn pictures_are_elements_named_by_their_alt_text() {
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let data = wordcraft_engine::cmd::insert::base64_encode(&png);
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("text.insert", json!({"text": "Before "})).unwrap();
        app.run("insert.picture", json!({"data": data, "width": 80})).unwrap();
        app.run("object.altText", json!({"text": "A red bar"})).unwrap();
        app.run("caret.docEnd", json!({})).unwrap();
        app.run("text.insert", json!({"text": " and "})).unwrap();
        app.run("insert.picture", json!({"data": data, "width": 80})).unwrap();
        app.run("object.altText", json!({"decorative": true})).unwrap();
        app.run("caret.docEnd", json!({})).unwrap();
        app.run("text.insert", json!({"text": " after"})).unwrap();
        let (_, node) = document(&mut app, &egui::Context::default());
        let doc = node.text_document().expect("the document as text");
        let pictures: Vec<(ObjectKind, String)> = every(&doc.objects)
            .into_iter()
            .filter(|o| matches!(o.kind, ObjectKind::Image { .. }))
            .map(|o| (o.kind.clone(), String::from_utf16_lossy(doc.text.get(o.range.clone()).unwrap_or_default())))
            .collect();
        assert_eq!(pictures, [(ObjectKind::Image { alt: "A red bar".into() }, "\u{FFFC}".to_string())], "the decorative picture isn't one");
    }

    /// A misspelt word is a spelling error in the text, over the word alone, as NVDA's W and
    /// NVDA+F read it.
    #[test]
    fn spelling_errors_are_marked_in_the_text() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        app.run("document.setText", json!({"text": "A beautifull day and anothr one."})).unwrap();
        let (_, node) = document(&mut app, &egui::Context::default());
        let doc = node.text_document().expect("the document as text");
        let text = String::from_utf16_lossy(&doc.text);
        let marked: Vec<(String, Annotation)> =
            doc.annotations.iter().map(|(r, k)| (String::from_utf16_lossy(doc.text.get(r.clone()).unwrap_or_default()), *k)).collect();
        assert_eq!(marked, [("beautifull".to_string(), Annotation::Spelling), ("anothr".to_string(), Annotation::Spelling)], "{text:?}");
    }

    #[test]
    fn utf16_offsets_of_each_byte() {
        assert_eq!(utf16_offsets("a\u{1F600}b"), vec![0, 1, 1, 1, 1, 3, 4]);
        assert_eq!(utf16_offsets(""), vec![0]);
    }
}
