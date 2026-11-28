//! Screen readers read the document (#488). The canvas draws pages as pictures, so on its own it
//! is one opaque widget to assistive technology. [`DocumentDescriber`] is the app's mapping of its
//! document model onto the page view's access description:
//!
//! - each body paragraph is a paragraph, a heading (with its outline level) or a list item (with
//!   its list level and marker), holding one text run per laid-out line with its characters'
//!   screen positions, so a reader can follow the caret by character, word and line and point at
//!   text, and a magnifier can keep the caret in view;
//! - tables are table → row → cell, pictures and charts images named by their alt text,
//!   hyperlinks links around their text runs.
//!
//! The framework builds the nodes, caps the tree and describes only the pages on screen and one
//! either side, and only when a screen reader (or a test) reads the node.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use egui::Rect;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::TextDirection;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_layout::para::ClKind;
use wordcraft_layout::{DocLayout, LineEnd, ParaLayout, Placed};

use craft::pages::ui::PlacedPage;
use craft::pages::ui::access::{AccessBuilder, AccessDescriber, AccessNode, AccessPosition, BlockRole, MAX_RUN};

/// Where floating pictures sit on a page, by (paragraph path, byte offset of the anchor character).
type Pictures = HashMap<(Vec<u32>, usize), Rect>;

/// Describes the document the pages are drawn from, and its selection.
pub(crate) struct DocumentDescriber {
    doc: Arc<Document>,
    layout: Arc<DocLayout>,
    /// The anchor and focus the caret is drawn from.
    sel: (Pos, Pos),
    /// The caret and the selection's other end, found while describing: a run's handle exists only once
    /// the builder made it.
    selection: Cell<Option<(AccessPosition, AccessPosition)>>,
}

impl DocumentDescriber {
    /// A describer of `layout`, laid out from `doc`, with the caret and the selection's other end in `sel`. It owns
    /// its snapshot, as the page source does, so the page view can hold it while the body edits the app. While
    /// Insert › Table's preview shows (#3931) the pages and caret are drawn from the preview copy, so the caller
    /// passes that copy's document and selection, not the real document the preview's layout does not belong to.
    pub(crate) fn new(doc: Arc<Document>, layout: Arc<DocLayout>, sel: (Pos, Pos)) -> Self {
        DocumentDescriber { doc, layout, sel, selection: Cell::new(None) }
    }
}

impl AccessDescriber for DocumentDescriber {
    fn describe(&self, placed: &[PlacedPage], out: &mut AccessBuilder) {
        let mut walk = Walk::new(&self.doc, out);
        'pages: for pp in placed {
            let Some(page) = self.layout.pages.get(pp.page) else { continue };
            let scale = crate::canvas::page_screen_scale(pp.rect, page, 1.0);
            // A floating picture's anchor character takes no room on its line: the picture's own rectangle is its bounds.
            let mut pictures = Pictures::new();
            for it in &page.items {
                if let Placed::Image { rect, story: StoryRef::Body, path, off, .. } | Placed::Graphic { rect, story: StoryRef::Body, path, off, .. } =
                    it
                {
                    let min = egui::pos2(pp.rect.min.x + rect.x * scale, pp.rect.min.y + rect.y * scale);
                    pictures.insert((path.0.clone(), *off), Rect::from_min_size(min, egui::vec2(rect.w * scale, rect.h * scale)));
                }
            }
            for it in &page.items {
                let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, turn } = it else { continue };
                let place = LinePlace { x: *x, y: *y, turn: *turn, scale, origin: pp.rect.min };
                if !walk.paragraph(path, para, *l0..*l1, &place, &pictures) {
                    break 'pages;
                }
            }
        }
        let at = |p: &Pos| if p.story == StoryRef::Body { walk.position(&p.path, p.off) } else { None };
        let (anchor, focus) = (at(&self.sel.0), at(&self.sel.1));
        self.selection.set(match (focus, anchor) {
            (Some(focus), Some(anchor)) => Some((focus, anchor)),
            (Some(focus), None) => Some((focus, focus)),
            _ => None,
        });
    }

    fn selection(&self) -> Option<(AccessPosition, AccessPosition)> {
        self.selection.get()
    }
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

/// One text run being gathered.
#[derive(Default)]
struct Seg {
    text: String,
    /// Paragraph x of each character's left edge and its width (points).
    chars: Vec<(f32, f32)>,
    link: Option<String>,
}

/// What a line holds, in reading order.
enum Piece {
    Run(Seg),
    /// A picture's alt text and its bounds on screen, when known.
    Image(String, Option<Rect>),
}

/// One page walk: the nodes made so far by document position.
struct Walk<'a, 'o> {
    doc: &'a Document,
    out: &'o mut AccessBuilder,
    paras: HashMap<Vec<u32>, AccessNode>,
    tables: HashMap<Vec<u32>, AccessNode>,
    rows: HashMap<Vec<u32>, AccessNode>,
    cells: HashMap<Vec<u32>, AccessNode>,
    /// The lines described: a repeated table header row shows the same lines again.
    done: HashSet<(Vec<u32>, usize)>,
    /// Per paragraph: (byte offset, text run, character index) of each cluster, in text order.
    offsets: HashMap<Vec<u32>, Vec<(usize, AccessNode, usize)>>,
}

impl<'a, 'o> Walk<'a, 'o> {
    fn new(doc: &'a Document, out: &'o mut AccessBuilder) -> Self {
        Walk {
            doc,
            out,
            paras: HashMap::new(),
            tables: HashMap::new(),
            rows: HashMap::new(),
            cells: HashMap::new(),
            done: HashSet::new(),
            offsets: HashMap::new(),
        }
    }

    /// The node blocks at `path` sit in: `Some(None)` for the document, a table cell (made on first use,
    /// with its row and table) otherwise; `None` for a path that cannot be described.
    fn container(&mut self, path: &[u32]) -> Option<Option<AccessNode>> {
        if path.len() <= 1 {
            return Some(None);
        }
        if path.len() < 4 {
            return None;
        }
        let n = path.len();
        let (tpath, rpath, cpath) = (path.get(..n - 3)?, path.get(..n - 2)?, path.get(..n - 1)?);
        if let Some(cell) = self.cells.get(cpath) {
            return Some(Some(*cell));
        }
        let table = match self.tables.get(tpath) {
            Some(t) => *t,
            None => {
                let parent = self.container(tpath)?;
                let t = self.out.block(parent, BlockRole::Table)?;
                self.tables.insert(tpath.to_vec(), t);
                t
            }
        };
        let row = match self.rows.get(rpath) {
            Some(r) => *r,
            None => {
                let r = self.out.block(Some(table), BlockRole::Row)?;
                self.rows.insert(rpath.to_vec(), r);
                r
            }
        };
        let cell = self.out.block(Some(row), BlockRole::Cell)?;
        self.cells.insert(cpath.to_vec(), cell);
        Some(Some(cell))
    }

    /// The paragraph at `path`, made on first sight (later pieces of it add lines).
    fn para_node(&mut self, path: &Path, para: &ParaLayout) -> Option<AccessNode> {
        if let Some(p) = self.paras.get(path.0.as_slice()) {
            return Some(*p);
        }
        let parent = self.container(&path.0)?;
        let heading = para.rp.outline_level.filter(|l| *l < 9);
        let list = para.rp.numbering.filter(|n| n.num != 0 && para.label.is_some());
        let role = match (heading, list) {
            (Some(l), _) => BlockRole::Heading(u8::try_from(l as usize + 1).unwrap_or(9)),
            (None, Some(num)) => {
                let marker = para
                    .label
                    .as_ref()
                    .map(|l| {
                        // Symbol-font bullets are private-use characters: say them as bullets.
                        l.text.chars().map(|c| if ('\u{E000}'..='\u{F8FF}').contains(&c) { '•' } else { c }).collect::<String>()
                    })
                    .unwrap_or_default();
                BlockRole::list_item(u8::try_from(num.level as usize).unwrap_or(8).min(8), marker)
            }
            (None, None) => BlockRole::Paragraph,
        };
        let node = self.out.block(parent, role)?;
        self.paras.insert(path.0.clone(), node);
        Some(node)
    }

    /// Lines `lines` of the paragraph at `path`: text runs (split at links, pictures and the run length
    /// limit), links and pictures. False when the tree is full and the walk should stop.
    fn paragraph(&mut self, path: &Path, para: &ParaLayout, lines: Range<usize>, at: &LinePlace, pictures: &Pictures) -> bool {
        let Some(pid) = self.para_node(path, para) else { return true };
        let Some(dp) = self.doc.para(StoryRef::Body, path) else { return true };
        let links = links(dp);
        let first_top = para.lines.get(lines.start).map_or(0.0, |l| l.top);
        for li in lines {
            let Some(line) = para.lines.get(li) else { break };
            if !self.done.insert((path.0.clone(), li)) {
                continue;
            }
            let top = line.top - first_top;
            let mut pieces: Vec<Piece> = Vec::new();
            let mut runs = 0usize;
            let mut cur = Seg::default();
            // (byte offset, run number, character index) of each cluster.
            let mut map: Vec<(usize, usize, usize)> = Vec::new();
            // Emit the run being gathered.
            let flush = |cur: &mut Seg, pieces: &mut Vec<Piece>, runs: &mut usize, force: bool| {
                if cur.chars.is_empty() && !force {
                    return;
                }
                pieces.push(Piece::Run(std::mem::take(cur)));
                *runs += 1;
            };
            for k in line.c0..line.c1 {
                let Some(cl) = para.clusters.get(k) else { break };
                let shown = para.shown.binary_search_by_key(&k, |s| s.0).ok().and_then(|i| para.shown.get(i)).map(|s| s.1.as_str());
                let text = match (cl.kind, shown) {
                    (_, Some(s)) => s,
                    (ClKind::Text | ClKind::Space, None) => dp.text.get(cl.start..cl.end).unwrap_or(""),
                    (ClKind::Tab, None) => "\t",
                    (ClKind::LineBreak, None) => "\n",
                    (ClKind::Object(_), None) => {
                        if let Some(InlineObject::Image { alt, .. } | InlineObject::Graphic { alt, .. }) = dp.object_at(cl.start) {
                            flush(&mut cur, &mut pieces, &mut runs, false);
                            let rect = pictures.get(&(path.0.clone(), cl.start)).copied().or_else(|| {
                                let (x0, x1) = (line.cl_left(k)?, line.cl_right(k)?);
                                Some(at.rect(wordcraft_geom::Rect::new(x0.min(x1), top, (x1 - x0).abs(), line.height)))
                            });
                            pieces.push(Piece::Image(alt.clone(), rect));
                        }
                        ""
                    }
                    (ClKind::PageBreak | ClKind::ColumnBreak | ClKind::Marker, None) => "",
                };
                if text.is_empty() {
                    map.push((cl.start, runs, cur.chars.len()));
                    continue;
                }
                // Every run has a node of its own, so `position` can name any character in it: the limit
                // splits runs here rather than leaving it to the builder.
                let text: String = text.chars().take(MAX_RUN).collect();
                let count = text.chars().count();
                let link = links.iter().find(|(r, _)| r.contains(&cl.start)).map(|(_, u)| u.clone());
                if (link != cur.link || cur.chars.len() + count > MAX_RUN) && !cur.chars.is_empty() {
                    flush(&mut cur, &mut pieces, &mut runs, false);
                }
                cur.link = link;
                map.push((cl.start, runs, cur.chars.len()));
                // A line break ends its line as the paragraph mark does, and takes the same room.
                let (x, w) = if cl.kind == ClKind::LineBreak {
                    line.mark_box()
                } else {
                    let (x0, x1) = (line.cl_left(k).unwrap_or(0.0), line.cl_right(k).unwrap_or(0.0));
                    (x0.min(x1), (x1 - x0).abs())
                };
                push_chars(&mut cur, &text, x, w);
            }
            if line.end == LineEnd::Para {
                // The paragraph mark ends the run, as AccessKit expects of a paragraph's end. It takes the
                // room a selection shows it with: magnifiers keep the caret in view by the box of the
                // character after it, which on an empty line is this mark (#1119).
                if (cur.link.is_some() || cur.chars.len() >= MAX_RUN) && !cur.chars.is_empty() {
                    flush(&mut cur, &mut pieces, &mut runs, false);
                }
                cur.link = None;
                map.push((para.text_len, runs, cur.chars.len()));
                let (x, w) = line.mark_box();
                push_chars(&mut cur, "\n", x, w);
            }
            // An empty line still has one (empty) run for the caret to sit in.
            let first_run = runs == 0;
            flush(&mut cur, &mut pieces, &mut runs, first_run);

            let horizontal = at.turn == TextDirection::Horizontal && !(line.rtl || !line.vis.is_empty());
            let line_rect = at.rect(wordcraft_geom::Rect::new(line.left, top, 0.0, line.height));
            let mut nodes: Vec<(AccessNode, usize)> = Vec::new();
            for piece in pieces {
                match piece {
                    Piece::Image(alt, rect) => {
                        // PLAN: assumed `AccessBuilder::image(&mut self, parent: Option<AccessNode>, alt: impl Into<String>, bounds: Rect) -> Option<AccessNode>` (chrome-w7)
                        let made = match rect {
                            Some(r) => self.out.image(Some(pid), alt, r),
                            None => self.out.block(Some(pid), BlockRole::Image(alt)),
                        };
                        if made.is_none() {
                            return false;
                        }
                    }
                    Piece::Run(seg) => {
                        let parent = match &seg.link {
                            Some(url) => match self.out.block(Some(pid), BlockRole::Link(url.clone())) {
                                Some(l) => l,
                                None => return false,
                            },
                            None => pid,
                        };
                        // Turned and bidirectional text keeps no per-character boxes.
                        let chars: Vec<(f32, f32)> = if horizontal {
                            seg.chars
                                .iter()
                                .map(|&(x, w)| {
                                    let r = at.rect(wordcraft_geom::Rect::new(x, top, w, line.height));
                                    (r.min.x, r.width())
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                        let Some(node) = self.out.line(parent, &seg.text, &chars, line_rect) else { return false };
                        nodes.push((node, seg.chars.len()));
                    }
                }
            }
            // Offsets that fell into an empty last run point at the end of the last run.
            let entries = self.offsets.entry(path.0.clone()).or_default();
            for (off, run, idx) in map {
                if let Some(&(node, len)) = nodes.get(run).or(nodes.last()) {
                    entries.push((off, node, if run < nodes.len() { idx } else { len }));
                }
            }
        }
        true
    }

    /// The text position of byte `off` of the paragraph at `path`, when it is described.
    fn position(&self, path: &Path, off: usize) -> Option<AccessPosition> {
        let entries = self.offsets.get(&path.0)?;
        let i = entries.partition_point(|e| e.0 < off);
        let (_, run, idx) = entries.get(i).or(entries.last())?;
        Some(AccessPosition::new(*run, *idx))
    }
}

/// Adds `text`'s characters to the run, the cluster's box shared between them.
fn push_chars(run: &mut Seg, text: &str, x: f32, w: f32) {
    let n = text.chars().count();
    if n == 0 {
        return;
    }
    let (x, w) = (if x.is_finite() { x } else { 0.0 }, if w.is_finite() { w } else { 0.0 });
    let each = w / n as f32;
    for (i, c) in text.chars().enumerate() {
        run.text.push(c);
        run.chars.push((x + each * i as f32, each));
    }
}

/// The paragraph's hyperlinked byte ranges and their targets.
fn links(p: &wordcraft_doc::Paragraph) -> Vec<(Range<usize>, String)> {
    let mut out: Vec<(Range<usize>, String)> = Vec::new();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::pages::ui::access::MAX_NODES;
    use craft::ui::runtime::access::Role;
    use craft::ui::testing::Fixture;
    use craft::ui::testing::access::{AccessTree, NodeKey};
    use egui::accesskit;
    use serde_json::json;

    fn role(r: accesskit::Role) -> Role {
        Role::Node(r)
    }

    /// The app shown in a fixture as the real window shows it, with Insert › Table's preview set in every
    /// pass when `preview` is given, as the grid does while its square is hovered.
    fn shown(app: &Rc<RefCell<crate::WordApp>>, preview: Option<(usize, usize)>) -> Fixture {
        let shared = app.clone();
        let mut f = Fixture::new(move |ui| {
            let mut app = shared.borrow_mut();
            if let Some(p) = preview {
                app.canvas.table_preview = Some(p);
            }
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        });
        f.room(egui::vec2(1440.0, 900.0));
        f.settle();
        f
    }

    /// The text of the runs below `key`, in order.
    fn text(tree: &AccessTree, key: NodeKey, out: &mut String) {
        if tree.role_of(key) == Some(role(accesskit::Role::TextRun)) {
            out.push_str(tree.value_of(key).unwrap_or(""));
        }
        for c in tree.children_of(key) {
            text(tree, c, out);
        }
    }

    fn texts(tree: &AccessTree, r: accesskit::Role) -> Vec<String> {
        tree.with_role(role(r))
            .into_iter()
            .map(|k| {
                let mut s = String::new();
                text(tree, k, &mut s);
                s
            })
            .collect()
    }

    /// The document's selection as the describer reports it for `app`'s whole layout: the character
    /// offsets of the caret and of the other end.
    fn selection_offsets(app: &crate::WordApp) -> Option<(usize, usize)> {
        let doc = crate::canvas::page_doc(app);
        let layout = Arc::new(wordcraft_layout::layout(doc, &mut wordcraft_layout::LayoutCache::new(), &Default::default()));
        let sel = crate::canvas::preview_sel(app).unwrap_or(app.session.sel_ends());
        let describer = DocumentDescriber::new(Arc::new(doc.clone()), layout.clone(), (sel.anchor.clone(), sel.focus.clone()));
        let placed: Vec<PlacedPage> = layout
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| PlacedPage::new(i, Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(p.w, p.h)), 1.0, (0.0, p.h)))
            .collect();
        let mut out = AccessBuilder::new();
        describer.describe(&placed, &mut out);
        describer.selection().map(|(caret, other)| (caret.offset, other.offset))
    }

    /// #3931 review: the page shows a previewed table while Insert › Table's grid is hovered, so the
    /// description must be of that page's table, not the real document's block at the preview's path. In
    /// the preview copy path `[0]` is a 2 × 2 table; in the real document it is the paragraph "Lanterns".
    #[test]
    fn the_preview_table_is_described_from_the_previewed_page() {
        let app = Rc::new(RefCell::new(crate::WordApp::new(
            wordcraft_engine::Session::new(wordcraft_doc::Document::from_text("Lanterns\nKites")),
            Default::default(),
        )));
        let f = shown(&app, Some((2, 2)));
        let tree = f.tree();
        assert_eq!(tree.with_role(role(accesskit::Role::Table)).len(), 1, "the previewed page has one table");
        assert_eq!(tree.with_role(role(accesskit::Role::Row)).len(), 2, "the 2 × 2 preview's rows");
        assert_eq!(tree.with_role(role(accesskit::Role::Cell)).len(), 4, "the 2 × 2 preview's cells");
        // The preview copy has the table at the caret and the real paragraphs after it, so "Lanterns" is
        // still described, below the table, never inside it where the real paragraph's path now holds the
        // table.
        let mut in_table = String::new();
        text(tree, tree.with_role(role(accesskit::Role::Table))[0], &mut in_table);
        assert!(!in_table.contains("Lanterns"), "the real paragraph is not at the preview's path: {in_table:?}");
    }

    /// #488: the canvas describes the document: a heading, a paragraph, a list item in a list, a table's
    /// rows and cells and a link, with the text in text runs and the caret at its paragraph and character.
    #[test]
    fn the_canvas_describes_the_document_to_screen_readers() {
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        for (cmd, p) in [
            ("text.insert", json!({"text": "Title here"})),
            ("para.style", json!({"style": "Heading1"})),
            ("text.newParagraph", json!({})),
            ("text.insert", json!({"text": "Section here"})),
            ("para.style", json!({"style": "Heading2"})),
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
        assert_eq!(selection_offsets(&app), Some((4, 4)), "the caret is after \"Read\", with no selection");

        let app = Rc::new(RefCell::new(app));
        let f = shown(&app, None);
        let tree = f.tree();

        // AccessKit's levels are zero-based (#3173): Heading 1 is level 0, Heading 2 level 1, which the Windows
        // adapter reports to UI Automation as levels 1 and 2.
        // In tree order: `with_role` lists the nodes in egui's update order, which follows its id map, not the page.
        let headings: Vec<NodeKey> = f.by_role(role(accesskit::Role::Heading)).iter().map(|t| t.node).collect();
        // PLAN: assumed `AccessTree::level_of(&self, NodeKey) -> Option<usize>` and `url_of(&self, NodeKey) -> Option<&str>` (chrome-w7)
        assert_eq!(headings.iter().map(|h| tree.level_of(*h)).collect::<Vec<_>>(), [Some(0), Some(1)]);
        let heading_texts: Vec<String> = headings
            .iter()
            .map(|h| {
                let mut s = String::new();
                text(tree, *h, &mut s);
                s
            })
            .collect();
        assert_eq!(heading_texts, ["Title here\n", "Section here\n"]);
        assert!(texts(tree, accesskit::Role::Paragraph).contains(&"Read this link\n".to_string()), "{:?}", texts(tree, accesskit::Role::Paragraph));
        assert_eq!(texts(tree, accesskit::Role::Link), ["link"]);
        let links = tree.with_role(role(accesskit::Role::Link));
        assert_eq!(tree.url_of(links[0]), Some("https://example.com"));
        assert_eq!(tree.with_role(role(accesskit::Role::List)).len(), 1, "one list");
        let items = tree.with_role(role(accesskit::Role::ListItem));
        assert_eq!(tree.level_of(items[0]), Some(0), "a first-level list item");
        assert_eq!(texts(tree, accesskit::Role::ListItem), ["First item\n"]);
        assert_eq!(tree.with_role(role(accesskit::Role::ListMarker)).len(), 1, "the bullet");
        assert_eq!(tree.with_role(role(accesskit::Role::Table)).len(), 1);
        assert_eq!(tree.with_role(role(accesskit::Role::Row)).len(), 2);
        assert_eq!(tree.with_role(role(accesskit::Role::Cell)).len(), 4);
        // The cells come in the order the nodes were made, which is not a reading order the framework
        // promises; the typed cell is among them with its text and paragraph mark.
        assert!(texts(tree, accesskit::Role::Cell).contains(&"Cell A\n".to_string()), "{:?}", texts(tree, accesskit::Role::Cell));
        for run in tree.with_role(role(accesskit::Role::TextRun)) {
            assert!(tree.bounds_of(run).is_some(), "{:?}", tree.value_of(run));
        }
    }

    /// #1119: a magnifier keeps the caret in view by the box of the character after it. On an empty line
    /// that character is the line's end mark — the paragraph mark after Enter, a line break between two
    /// Shift+Enter breaks — and it takes room where the caret is, so the view follows the caret onto it.
    #[test]
    fn a_magnifier_finds_the_caret_on_an_empty_line() {
        let app = Rc::new(RefCell::new(crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default())));
        {
            let mut a = app.borrow_mut();
            a.run("text.insert", json!({"text": "First line"})).unwrap();
            a.run("text.newParagraph", json!({})).unwrap();
        }
        assert_end_mark_at_caret(&app, "Enter");
        {
            let mut a = app.borrow_mut();
            for cmd in ["text.lineBreak", "text.lineBreak", "caret.left"] {
                a.run(cmd, json!({})).unwrap_or_else(|e| panic!("{cmd}: {e}"));
            }
        }
        assert_end_mark_at_caret(&app, "Shift+Enter");
    }

    /// An end-mark run of the empty line the caret is on exists, with room, starting where the caret is
    /// drawn and on its line.
    fn assert_end_mark_at_caret(app: &Rc<RefCell<crate::WordApp>>, step: &str) {
        let f = shown(app, None);
        let tree = f.tree();
        let (caret, height) = crate::canvas::caret_screen(&mut app.borrow_mut()).expect("the caret on screen");
        let found = tree.with_role(role(accesskit::Role::TextRun)).into_iter().filter(|k| tree.value_of(*k) == Some("\n")).any(|k| {
            let Some(b) = tree.bounds_of(k) else { return false };
            b.width() > 0.0 && (b.min.x - caret.x).abs() < 0.5 && b.min.y <= caret.y + 0.5 && caret.y + height <= b.max.y + 0.5
        });
        assert!(found, "{step}: an end mark with room starts at the caret {caret:?}");
    }

    /// Hostile layouts and junk geometry don't panic and the description stays bounded.
    #[test]
    fn junk_geometry_is_harmless() {
        let doc = wordcraft_doc::Document::from_text(&"word ".repeat(4000));
        let layout = wordcraft_layout::layout(&doc, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
        let n = layout.pages.len();
        let p = Pos::body(0, 1_000_000);
        let describer = DocumentDescriber::new(Arc::new(doc), Arc::new(layout), (p.clone(), p));
        let rect = Rect::from_min_size(egui::pos2(f32::NAN, 0.0), egui::vec2(f32::INFINITY, 1.0));
        let placed: Vec<PlacedPage> = (0..n + 5).map(|i| PlacedPage::new(i, rect, f64::NAN, (f32::NAN, f32::INFINITY))).collect();
        let mut out = AccessBuilder::new();
        describer.describe(&placed, &mut out);
        assert!(!out.is_empty() && out.len() <= MAX_NODES);
        assert!(describer.selection().is_some(), "an offset past the end lands at the end");
    }
}
