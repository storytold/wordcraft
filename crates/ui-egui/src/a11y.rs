//! Screen readers read the document (#488). The canvas draws pages as pictures, so on its own it
//! is one opaque widget to assistive technology. This module describes the document under it as
//! an AccessKit tree, the way a browser describes a page:
//!
//! - the canvas widget is the **document** node (named after the document), holding the caret
//!   and selection as an AccessKit text selection;
//! - each body paragraph is a **paragraph**, a **heading** (with its outline level) or a **list
//!   item** (with its list level, grouped into lists), holding one **text run** per laid-out line
//!   with its characters' screen positions, so a reader can follow the caret by character, word
//!   and line and point at text;
//! - tables are **table → row → cell**, pictures and charts **images** named by their alt text,
//!   hyperlinks **links** around their text runs; list numbers are **list markers**.
//!
//! Only the pages on screen and one page either side are described, and the tree is capped, so a
//! long document costs no more than a short one. Nothing is built unless a screen reader (or a
//! test) asked egui for AccessKit output; the tree is rebuilt only when the layout, the selection,
//! the described pages or their place on screen changed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::accesskit::{self, Node, NodeId, Role, TextPosition, TextSelection};
use egui::{Id, Rect};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::TextDirection;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_layout::para::{ClKind, Line};
use wordcraft_layout::{DocLayout, LineEnd, ParaLayout, Placed};

/// Most nodes described at once (a dense page has a few hundred).
pub const MAX_NODES: usize = 6000;
/// AccessKit's word starts are `u8`s: a text run holds at most this many characters.
const MAX_RUN: usize = 255;
/// Deepest table nesting described (paths come from files).
const MAX_DEPTH: usize = 16;

/// What was described last frame, and for what.
#[derive(Default)]
pub struct A11yCache {
    key: Option<Key>,
    tree: Tree,
}

struct Key {
    layout: Arc<DocLayout>,
    doc: Id,
    pages: (usize, usize),
    rects: Vec<Rect>,
    sel: (Pos, Pos),
}

impl Key {
    fn same(&self, o: &Key) -> bool {
        Arc::ptr_eq(&self.layout, &o.layout) && self.doc == o.doc && self.pages == o.pages && self.rects == o.rects && self.sel == o.sel
    }
}

/// The described document: its nodes (in creation order), the document node's children and the
/// selection inside it.
#[derive(Default, Clone)]
pub struct Tree {
    pub nodes: Vec<(Id, Node)>,
    pub top: Vec<NodeId>,
    pub selection: Option<TextSelection>,
}

/// The pages to describe: those meeting `view`, and one either side.
pub fn page_window(rects: &[Rect], view: Rect) -> Option<(usize, usize)> {
    let first = rects.iter().position(|r| r.intersects(view))?;
    let last = rects.iter().rposition(|r| r.intersects(view)).unwrap_or(first);
    Some((first.saturating_sub(1), (last + 2).min(rects.len())))
}

/// Describe the document under the canvas widget `doc_id` to AccessKit for this frame.
/// `rects` are the pages' screen rectangles, `view` the visible part of the canvas.
pub fn expose(app: &mut crate::WordApp, ctx: &egui::Context, doc_id: Id, layout: &Arc<DocLayout>, rects: &[Rect], view: Rect) {
    if ctx.accesskit_node_builder(doc_id, |n| n.set_role(Role::Document)).is_none() {
        // No screen reader: describe nothing, keep nothing.
        app.canvas.a11y = A11yCache::default();
        return;
    }
    let pages = page_window(rects, view).unwrap_or((0, 0));
    let sel = (app.session.sel.anchor.clone(), app.session.sel.focus.clone());
    let key = Key { layout: layout.clone(), doc: doc_id, pages, rects: rects.get(pages.0..pages.1).unwrap_or_default().to_vec(), sel };
    if !app.canvas.a11y.key.as_ref().is_some_and(|k| k.same(&key)) {
        app.canvas.a11y.tree = build(&app.session.doc, layout, doc_id, rects, pages, (&key.sel.0, &key.sel.1));
        app.canvas.a11y.key = Some(key);
    }
    let title = app.title_stem();
    let tree = &app.canvas.a11y.tree;
    let ours: HashSet<NodeId> = tree.nodes.iter().map(|(id, _)| id.accesskit_id()).collect();
    for (id, node) in &tree.nodes {
        ctx.accesskit_node_builder(*id, |n| *n = node.clone());
    }
    // egui hangs every new node off the window; ours belong under the document.
    ctx.accesskit_node_builder(egui::accesskit_root_id(), |root| {
        let keep: Vec<NodeId> = root.children().iter().filter(|c| !ours.contains(c)).copied().collect();
        root.set_children(keep);
    });
    ctx.accesskit_node_builder(doc_id, |n| {
        n.set_label(title);
        for c in &tree.top {
            n.push_child(*c);
        }
        match &tree.selection {
            Some(s) => n.set_text_selection(*s),
            None => n.clear_text_selection(),
        }
    });
}

/// Build the description of pages `pages.0..pages.1`.
pub fn build(doc: &Document, layout: &DocLayout, doc_id: Id, rects: &[Rect], pages: (usize, usize), sel: (&Pos, &Pos)) -> Tree {
    let mut b = Builder { doc, doc_id, tree: Tree::default(), index: HashMap::new(), lists: HashMap::new(), offsets: HashMap::new() };
    for pi in pages.0..pages.1 {
        let (Some(page), Some(pr)) = (layout.pages.get(pi), rects.get(pi)) else { continue };
        let scale = crate::canvas::page_screen_scale(*pr, page, 1.0);
        let to_screen = |r: wordcraft_geom::Rect| {
            Rect::from_min_size(egui::pos2(pr.min.x + r.x * scale, pr.min.y + r.y * scale), egui::vec2(r.w * scale, r.h * scale))
        };
        // Where floating pictures sit (their anchor character takes no room on its line).
        let mut pictures: HashMap<(&[u32], usize), Rect> = HashMap::new();
        for it in &page.items {
            if let Placed::Image { rect, story: StoryRef::Body, path, off, .. } | Placed::Graphic { rect, story: StoryRef::Body, path, off, .. } = it
            {
                pictures.insert((path.0.as_slice(), *off), to_screen(*rect));
            }
        }
        for it in &page.items {
            if b.full() {
                break;
            }
            let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, turn } = it else { continue };
            let place = LinePlace { x: *x, y: *y, turn: *turn, scale, origin: pr.min };
            b.paragraph(path, para, *l0..*l1, &place, &pictures);
        }
    }
    let at = |p: &Pos| if p.story == StoryRef::Body { b.position(&p.path, p.off) } else { None };
    b.tree.selection = match (at(sel.0), at(sel.1)) {
        (Some(anchor), Some(focus)) => Some(TextSelection { anchor, focus }),
        (None, Some(focus)) => Some(TextSelection { anchor: focus, focus }),
        _ => None,
    };
    b.tree
}

/// Where a paragraph's lines are placed: the `Placed::Lines` frame and the page on screen.
struct LinePlace {
    x: f32,
    y: f32,
    turn: TextDirection,
    scale: f32,
    origin: egui::Pos2,
}

impl LinePlace {
    fn rect(&self, r: wordcraft_geom::Rect) -> Rect {
        let p = wordcraft_layout::turn_rect(self.turn, self.x, self.y, r);
        let s = self.scale;
        Rect::from_min_size(egui::pos2(self.origin.x + p.x * s, self.origin.y + p.y * s), egui::vec2(p.w * s, p.h * s))
    }
}

struct Builder<'a> {
    doc: &'a Document,
    doc_id: Id,
    tree: Tree,
    index: HashMap<Id, usize>,
    /// Per container: the list being filled and the block index of its last item.
    lists: HashMap<Id, (Id, usize)>,
    /// Per paragraph: (byte offset, text run, character index) of each cluster, in text order.
    offsets: HashMap<Vec<u32>, Vec<(usize, Id, usize)>>,
}

/// One text run being gathered.
#[derive(Default)]
struct Run {
    text: String,
    lens: Vec<u8>,
    /// Paragraph x of each character's left edge and its width (points).
    xs: Vec<(f32, f32)>,
    link: Option<String>,
}

impl Builder<'_> {
    fn full(&self) -> bool {
        self.tree.nodes.len() >= MAX_NODES
    }

    /// Add a node under `parent` (the document when `None`). False when it exists or the tree is full.
    fn add(&mut self, id: Id, parent: Option<Id>, node: Node) -> bool {
        if self.index.contains_key(&id) || self.full() {
            return false;
        }
        match parent {
            Some(p) => match self.index.get(&p).and_then(|&i| self.tree.nodes.get_mut(i)) {
                Some((_, pn)) => pn.push_child(id.accesskit_id()),
                None => return false,
            },
            None => self.tree.top.push(id.accesskit_id()),
        }
        self.index.insert(id, self.tree.nodes.len());
        self.tree.nodes.push((id, node));
        true
    }

    fn node_mut(&mut self, id: Id) -> Option<&mut Node> {
        let i = *self.index.get(&id)?;
        self.tree.nodes.get_mut(i).map(|(_, n)| n)
    }

    /// The node blocks at `path` sit in: the document, or a table cell (made on first use, with
    /// its row and table).
    fn container(&mut self, path: &[u32]) -> Option<Id> {
        if path.len() <= 1 {
            return Some(self.doc_id);
        }
        if path.len() < 4 || path.len() > MAX_DEPTH * 3 + 1 {
            return None;
        }
        let n = path.len();
        let (tpath, rpath, cpath) = (path.get(..n - 3)?, path.get(..n - 2)?, path.get(..n - 1)?);
        let (row, col) = (*path.get(n - 3)? as usize, *path.get(n - 2)? as usize);
        let (tid, rid, cid) = (self.doc_id.with(("table", tpath)), self.doc_id.with(("row", rpath)), self.doc_id.with(("cell", cpath)));
        if !self.index.contains_key(&tid) {
            let parent = self.container(tpath)?;
            let mut t = Node::new(Role::Table);
            if let Some(tb) = self.doc.block(StoryRef::Body, &Path(tpath.to_vec())).and_then(|b| b.as_table()) {
                t.set_row_count(tb.rows.len());
                t.set_column_count(tb.grid.len());
            }
            self.add(tid, (parent != self.doc_id).then_some(parent), t);
        }
        if !self.index.contains_key(&rid) {
            let mut r = Node::new(Role::Row);
            r.set_row_index(row);
            self.add(rid, Some(tid), r);
        }
        if !self.index.contains_key(&cid) {
            let mut c = Node::new(Role::Cell);
            c.set_row_index(row);
            c.set_column_index(col);
            self.add(cid, Some(rid), c);
        }
        self.index.contains_key(&cid).then_some(cid)
    }

    /// The paragraph at `path`, made on first sight (later pieces of it add lines).
    fn para_node(&mut self, path: &Path, para: &ParaLayout) -> Option<Id> {
        let id = self.doc_id.with(("para", path.0.as_slice()));
        if self.index.contains_key(&id) {
            return Some(id);
        }
        let container = self.container(&path.0)?;
        let parent = (container != self.doc_id).then_some(container);
        let block = path.last();
        let heading = para.rp.outline_level.filter(|l| *l < 9);
        let list = para.rp.numbering.filter(|n| n.num != 0 && para.label.is_some());
        let node = match (heading, list) {
            (Some(l), _) => {
                let mut n = Node::new(Role::Heading);
                n.set_level(l as usize + 1);
                n
            }
            (None, Some(num)) => {
                let mut n = Node::new(Role::ListItem);
                n.set_level(num.level as usize + 1);
                n
            }
            (None, None) => Node::new(Role::Paragraph),
        };
        let parent = if list.is_some() && heading.is_none() {
            // Consecutive list paragraphs share a list.
            let lid = match self.lists.get(&container) {
                Some((lid, last)) if last + 1 == block && self.index.contains_key(lid) => *lid,
                _ => {
                    let lid = container.with(("list", block));
                    if !self.add(lid, parent, Node::new(Role::List)) {
                        return None;
                    }
                    lid
                }
            };
            self.lists.insert(container, (lid, block));
            Some(lid)
        } else {
            parent
        };
        if !self.add(id, parent, node) {
            return None;
        }
        if let Some(label) = para.label.as_ref().filter(|l| !l.text.trim().is_empty()) {
            let mut m = Node::new(Role::ListMarker);
            // Symbol-font bullets are private-use characters: say them as bullets.
            m.set_value(label.text.chars().map(|c| if ('\u{E000}'..='\u{F8FF}').contains(&c) { '•' } else { c }).collect::<String>());
            self.add(id.with("marker"), Some(id), m);
        }
        Some(id)
    }

    /// Lines `lines` of the paragraph at `path`: text runs (split at links, pictures and the
    /// run length limit), links and pictures.
    fn paragraph(
        &mut self,
        path: &Path,
        para: &ParaLayout,
        lines: std::ops::Range<usize>,
        at: &LinePlace,
        pictures: &HashMap<(&[u32], usize), Rect>,
    ) {
        let Some(pid) = self.para_node(path, para) else { return };
        let Some(dp) = self.doc.para(StoryRef::Body, path) else { return };
        let links = links(dp);
        let first_top = para.lines.get(lines.start).map_or(0.0, |l| l.top);
        for li in lines {
            let Some(line) = para.lines.get(li) else { break };
            if self.full() || self.index.contains_key(&pid.with(("run", li, 0usize))) {
                // A repeated table header row shows the same lines again.
                continue;
            }
            let mut lb = LineBuild { pid, li, seg: 0, prev: None, top: line.top - first_top, run: Run::default(), map: Vec::new() };
            for k in line.c0..line.c1 {
                let Some(cl) = para.clusters.get(k) else { break };
                lb.map.push((cl.start, lb.seg, lb.run.lens.len()));
                let shown = para.shown.binary_search_by_key(&k, |s| s.0).ok().and_then(|i| para.shown.get(i)).map(|s| s.1.as_str());
                let text = match (cl.kind, shown) {
                    (_, Some(s)) => s,
                    (ClKind::Text | ClKind::Space, None) => dp.text.get(cl.start..cl.end).unwrap_or(""),
                    (ClKind::Tab, None) => "\t",
                    (ClKind::LineBreak, None) => "\n",
                    (ClKind::Object(_), None) => {
                        let picture = match dp.object_at(cl.start) {
                            Some(InlineObject::Image { alt, .. } | InlineObject::Graphic { alt, .. }) => Some(alt.clone()),
                            _ => None,
                        };
                        if let Some(alt) = picture {
                            if !lb.run.lens.is_empty() {
                                self.flush(&mut lb, line, at);
                            }
                            let rect = pictures.get(&(path.0.as_slice(), cl.start)).copied().or_else(|| {
                                let (x0, x1) = (line.cl_left(k)?, line.cl_right(k)?);
                                Some(at.rect(wordcraft_geom::Rect::new(x0.min(x1), lb.top, (x1 - x0).abs(), line.height)))
                            });
                            let mut n = Node::new(Role::Image);
                            if !alt.trim().is_empty() {
                                n.set_label(alt);
                            }
                            if let Some(r) = rect {
                                n.set_bounds(bounds(r));
                            }
                            self.add(pid.with(("image", cl.start)), Some(pid), n);
                        }
                        ""
                    }
                    (ClKind::PageBreak | ClKind::ColumnBreak | ClKind::Marker, None) => "",
                };
                if text.is_empty() {
                    continue;
                }
                let link = links.iter().find(|(r, _)| r.contains(&cl.start)).map(|(_, u)| u.clone());
                if (link != lb.run.link || lb.run.lens.len() >= MAX_RUN) && !lb.run.lens.is_empty() {
                    self.flush(&mut lb, line, at);
                    if let Some(m) = lb.map.last_mut() {
                        *m = (cl.start, lb.seg, 0);
                    }
                }
                lb.run.link = link;
                let (x0, x1) = (line.cl_left(k).unwrap_or(0.0), line.cl_right(k).unwrap_or(0.0));
                push_char(&mut lb.run, text, x0.min(x1), (x1 - x0).abs());
            }
            if line.end == LineEnd::Para {
                // The paragraph mark ends the run, as AccessKit expects of a paragraph's end.
                if lb.run.link.is_some() && !lb.run.lens.is_empty() {
                    self.flush(&mut lb, line, at);
                }
                if lb.run.lens.len() >= MAX_RUN {
                    self.flush(&mut lb, line, at);
                }
                lb.run.link = None;
                lb.map.push((para.text_len, lb.seg, lb.run.lens.len()));
                let x = line.visual_end_x();
                push_char(&mut lb.run, "\n", x, 0.0);
            }
            if !lb.run.lens.is_empty() || lb.prev.is_none() {
                self.flush(&mut lb, line, at);
            }
            // Offsets that fell into an empty last segment point at the end of the last run.
            let end = lb.prev;
            let entries = self.offsets.entry(path.0.clone()).or_default();
            for (off, seg, idx) in lb.map {
                let (seg, idx) = if seg >= lb.seg { end.unwrap_or((seg, idx)) } else { (seg, idx) };
                entries.push((off, pid.with(("run", li, seg)), idx));
            }
        }
    }

    /// Emit the run being gathered as a text run node (inside a link node when it is linked).
    fn flush(&mut self, lb: &mut LineBuild, line: &Line, at: &LinePlace) {
        let run = std::mem::take(&mut lb.run);
        if run.lens.is_empty() && lb.prev.is_some() {
            return;
        }
        let id = lb.pid.with(("run", lb.li, lb.seg));
        let mut n = Node::new(Role::TextRun);
        let x0 = run.xs.iter().map(|c| c.0).fold(f32::INFINITY, f32::min);
        let x1 = run.xs.iter().map(|c| c.0 + c.1).fold(f32::NEG_INFINITY, f32::max);
        let (x0, x1) = if x0.is_finite() && x1.is_finite() { (x0, x1) } else { (line.left, line.left) };
        let r = at.rect(wordcraft_geom::Rect::new(x0, lb.top, (x1 - x0).max(0.0), line.height));
        n.set_bounds(bounds(r));
        let rtl = line.rtl || !line.vis.is_empty();
        n.set_text_direction(if line.rtl { accesskit::TextDirection::RightToLeft } else { accesskit::TextDirection::LeftToRight });
        if at.turn == TextDirection::Horizontal && !rtl {
            n.set_character_positions(run.xs.iter().map(|c| (c.0 - x0) * at.scale).collect::<Vec<f32>>());
            n.set_character_widths(run.xs.iter().map(|c| c.1 * at.scale).collect::<Vec<f32>>());
        }
        n.set_word_starts(word_starts(&run.text, &run.lens));
        n.set_value(run.text);
        n.set_character_lengths(run.lens.clone());
        if let Some((pseg, _)) = lb.prev {
            let pid = lb.pid.with(("run", lb.li, pseg));
            n.set_previous_on_line(pid.accesskit_id());
            if let Some(p) = self.node_mut(pid) {
                p.set_next_on_line(id.accesskit_id());
            }
        }
        let parent = match &run.link {
            Some(url) => {
                let lid = lb.pid.with(("link", lb.li, lb.seg));
                let mut l = Node::new(Role::Link);
                l.set_url(url.clone());
                l.set_bounds(bounds(r));
                self.add(lid, Some(lb.pid), l);
                lid
            }
            None => lb.pid,
        };
        if self.add(id, Some(parent), n) {
            lb.prev = Some((lb.seg, run.lens.len()));
        }
        lb.seg += 1;
    }

    /// The text position of byte `off` of the paragraph at `path`, when it is described.
    fn position(&self, path: &Path, off: usize) -> Option<TextPosition> {
        let entries = self.offsets.get(&path.0)?;
        let i = entries.partition_point(|e| e.0 < off);
        let (_, run, idx) = entries.get(i).or(entries.last())?;
        self.index.contains_key(run).then(|| TextPosition { node: run.accesskit_id(), character_index: *idx })
    }
}

/// A line being turned into runs.
struct LineBuild {
    pid: Id,
    li: usize,
    /// The segment (run) number being gathered.
    seg: usize,
    /// The last run emitted on this line and its length in characters.
    prev: Option<(usize, usize)>,
    /// The line's top in the frame of its `Placed::Lines`.
    top: f32,
    run: Run,
    /// (byte offset, segment, character index) of each cluster.
    map: Vec<(usize, usize, usize)>,
}

fn push_char(run: &mut Run, text: &str, x: f32, w: f32) {
    // One character per cluster; a cluster longer than AccessKit's byte count keeps its first char.
    let text = if text.len() > usize::from(u8::MAX) { text.get(..text.chars().next().map_or(0, char::len_utf8)).unwrap_or("") } else { text };
    let Ok(len) = u8::try_from(text.len()) else { return };
    if len == 0 {
        return;
    }
    run.text.push_str(text);
    run.lens.push(len);
    let (x, w) = (if x.is_finite() { x } else { 0.0 }, if w.is_finite() { w } else { 0.0 });
    run.xs.push((x, w));
}

/// Characters where a word starts (after one that isn't part of a word).
fn word_starts(text: &str, lens: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut at, mut after_gap) = (0usize, true);
    for (i, len) in lens.iter().enumerate() {
        let word = text.get(at..).and_then(|s| s.chars().next()).is_some_and(|c| c.is_alphanumeric() || c == '_');
        if word && after_gap && i > 0 {
            out.push(u8::try_from(i).unwrap_or(u8::MAX));
        }
        after_gap = !word;
        at += usize::from(*len);
    }
    out
}

/// The paragraph's hyperlinked byte ranges and their targets.
fn links(p: &wordcraft_doc::Paragraph) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    let mut at = 0usize;
    for r in &p.runs {
        let end = at.saturating_add(r.len);
        if let Some(url) = &r.props.link {
            match out.last_mut() {
                Some((range, u)) if range.end == at && u == url => range.end = end,
                _ => out.push((at..end, url.clone())),
            }
        }
        at = end;
    }
    out
}

fn bounds(r: Rect) -> accesskit::Rect {
    accesskit::Rect { x0: r.min.x.into(), y0: r.min.y.into(), x1: r.max.x.into(), y1: r.max.y.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// #488: with AccessKit on, the canvas describes the document: a heading with its level, a
    /// paragraph, a list item in a list, a table's rows and cells and a link, with the text in
    /// text runs and the caret at its paragraph and character.
    #[test]
    fn the_canvas_describes_the_document_to_screen_readers() {
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
        // The caret: in "Read this ", after "Read".
        app.run("select.text", json!({"text": "Read"})).unwrap();
        app.run("caret.right", json!({})).unwrap();
        let caret = app.session.sel.focus.clone();
        assert!(caret.path.0.len() == 1 && caret.off == 4, "{caret:?}");

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut update = None;
        for i in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))),
                time: Some(i as f64 * 0.1),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
            update = out.platform_output.accesskit_update.take();
        }
        let update = update.expect("AccessKit output");
        let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();

        // A tree: every node has at most one parent.
        let mut parents: HashMap<NodeId, usize> = HashMap::new();
        for n in nodes.values() {
            for c in n.children() {
                *parents.entry(*c).or_default() += 1;
            }
        }
        assert!(parents.values().all(|n| *n == 1), "a node with two parents");

        let (doc_id, doc) = nodes.iter().find(|(_, n)| n.role() == Role::Document).expect("a document node");
        // Text of a node's runs, depth first.
        fn text(nodes: &HashMap<NodeId, &Node>, id: NodeId, out: &mut String) {
            let Some(n) = nodes.get(&id) else { return };
            if n.role() == Role::TextRun {
                out.push_str(n.value().unwrap_or(""));
            }
            for c in n.children() {
                text(nodes, *c, out);
            }
        }
        let mut all = Vec::new();
        fn walk(nodes: &HashMap<NodeId, &Node>, id: NodeId, depth: usize, out: &mut Vec<(NodeId, Role, usize)>) {
            let Some(n) = nodes.get(&id) else { return };
            out.push((id, n.role(), depth));
            for c in n.children() {
                walk(nodes, *c, depth + 1, out);
            }
        }
        walk(&nodes, *doc_id, 0, &mut all);
        let of = |role: Role| all.iter().filter(|(_, r, _)| *r == role).map(|(id, _, _)| *id).collect::<Vec<_>>();
        let texts = |role: Role| {
            of(role)
                .into_iter()
                .map(|id| {
                    let mut s = String::new();
                    text(&nodes, id, &mut s);
                    s
                })
                .collect::<Vec<_>>()
        };

        let headings = of(Role::Heading);
        assert_eq!(headings.len(), 1);
        assert_eq!(nodes[&headings[0]].level(), Some(1));
        assert_eq!(texts(Role::Heading), ["Title here\n"]);
        assert!(texts(Role::Paragraph).contains(&"Read this link\n".to_string()), "{:?}", texts(Role::Paragraph));
        assert_eq!(texts(Role::Link), ["link"]);
        assert_eq!(nodes[&of(Role::Link)[0]].url(), Some("https://example.com"));
        assert_eq!(of(Role::List).len(), 1, "one list");
        assert_eq!(texts(Role::ListItem), ["First item\n"]);
        assert_eq!(nodes[&of(Role::ListItem)[0]].level(), Some(1));
        assert_eq!(of(Role::ListMarker).len(), 1, "the bullet");
        assert_eq!(of(Role::Table).len(), 1);
        assert_eq!(of(Role::Row).len(), 2);
        assert_eq!(of(Role::Cell).len(), 4);
        assert_eq!(texts(Role::Cell)[0], "Cell A\n");

        // Text runs carry one length and one position per character.
        for id in of(Role::TextRun) {
            let n = nodes[&id];
            let v = n.value().unwrap_or("");
            assert_eq!(n.character_lengths().iter().map(|l| usize::from(*l)).sum::<usize>(), v.len(), "{v:?}");
            if let Some(p) = n.character_positions() {
                assert_eq!(p.len(), n.character_lengths().len());
            }
            assert!(n.bounds().is_some());
        }

        // The caret: the selection is on the document node, in "Read this "'s run, after "Read".
        let s = doc.text_selection().expect("the caret");
        assert_eq!(s.anchor, s.focus);
        assert_eq!(s.focus.character_index, 4);
        let run = nodes[&s.focus.node];
        assert_eq!(run.role(), Role::TextRun);
        assert_eq!(run.value(), Some("Read this "));
    }

    /// Hostile layouts and junk geometry don't panic and the tree stays bounded.
    #[test]
    fn junk_geometry_is_harmless() {
        let doc = wordcraft_doc::Document::from_text(&"word ".repeat(4000));
        let layout = wordcraft_layout::layout(&doc, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
        let n = layout.pages.len();
        let rects = vec![Rect::from_min_size(egui::pos2(f32::NAN, 0.0), egui::vec2(f32::INFINITY, 1.0)); n];
        let p = Pos::body(0, 1_000_000);
        let t = build(&doc, &layout, Id::new("doc"), &rects, (0, n + 5), (&p, &p));
        assert!(!t.nodes.is_empty() && t.nodes.len() <= MAX_NODES);
        assert!(t.selection.is_some(), "an offset past the end lands at the end");
        assert!(page_window(&rects, Rect::NOTHING).is_none());
    }
}
