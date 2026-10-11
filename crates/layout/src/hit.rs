//! Hit testing and caret geometry.

use wordcraft_doc::para::Wrap;
use wordcraft_doc::props::TextDirection;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_geom::{Point, Rect};

use crate::para::ParaLayout;
use crate::{DocLayout, Page, Placed};

/// Caret geometry on a page: a bar from (`x`, `top`) to (`x + width`, `top + height`). It is
/// upright (`width` 0) except in turned text (a table cell's text direction), where it lies
/// across the page (`height` 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub page: usize,
    pub x: f32,
    pub top: f32,
    pub height: f32,
    pub width: f32,
}

/// Result of [`DocLayout::visual_step`].
#[derive(Clone, Debug, PartialEq)]
pub enum VisualStep {
    /// The position one visual step away on the same line.
    Moved(Pos),
    /// Already at that visual end of the line: the line's byte range, whether it is the
    /// paragraph's last line, and whether the paragraph reads right to left.
    Edge { start: usize, stop: usize, last_line: bool, rtl: bool },
}

/// One placed line, resolved to page coordinates.
#[derive(Clone, Debug)]
pub struct LineHit<'a> {
    pub story: StoryRef,
    pub path: &'a Path,
    pub para: &'a ParaLayout,
    pub li: usize,
    pub x: f32,
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

/// A picture, shape or text box as laid out (see [`Placed::Object`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectHit {
    pub page: usize,
    /// Its unrotated frame; `spin` turns it about the centre.
    pub rect: Rect,
    pub spin: wordcraft_geom::Spin,
    pub story: StoryRef,
    pub path: Path,
    pub off: usize,
    pub text_box: Option<u32>,
    pub wrap: Wrap,
    /// Where column- and paragraph-relative offsets start (see [`Placed::Object`]).
    pub origin: Point,
}

impl ObjectHit {
    /// The object's position (its U+FFFC).
    pub fn pos(&self) -> Pos {
        Pos { story: self.story, path: self.path.clone(), off: self.off }
    }
    pub fn floating(&self) -> bool {
        self.wrap != Wrap::Inline
    }
    pub fn behind(&self) -> bool {
        self.wrap == Wrap::BehindText
    }
    /// Page point (`x`, `y`) in the object's own unrotated frame (the point itself unrotated).
    pub fn unspin(&self, x: f32, y: f32) -> Point {
        let (cx, cy) = (self.rect.x + self.rect.w / 2.0, self.rect.y + self.rect.h / 2.0);
        let (u, v) = self.spin.unapply(cx, cy, x, y);
        Point::new(u, v)
    }
    /// The page area it covers: its frame's rotated bounds.
    pub fn bounds(&self) -> Rect {
        self.spin.bounds(self.rect)
    }
}

/// A page's objects (not its header's or footer's), topmost first.
fn objects(page: &Page, index: usize) -> impl Iterator<Item = ObjectHit> + '_ {
    objects_in(&page.items, index)
}

/// A page's objects including its header's and footer's.
fn all_objects(page: &Page, index: usize) -> impl Iterator<Item = ObjectHit> + '_ {
    objects_in(&page.items, index).chain(objects_in(&page.header, index)).chain(objects_in(&page.footer, index))
}

fn objects_in(items: &[Placed], index: usize) -> impl Iterator<Item = ObjectHit> + '_ {
    items.iter().rev().filter_map(move |it| match it {
        Placed::Object { rect, spin, story, path, off, text_box, wrap, origin } => Some(ObjectHit {
            page: index,
            rect: *rect,
            spin: *spin,
            story: *story,
            path: path.clone(),
            off: *off,
            text_box: *text_box,
            wrap: *wrap,
            origin: *origin,
        }),
        _ => None,
    })
}

/// The story of the text (not a text box's) near (x, y): body, footnotes, endnotes.
fn text_at(p: &Page, index: usize, x: f32, y: f32) -> Option<StoryRef> {
    let boxes: Vec<StoryRef> = objects(p, index).filter_map(|o| o.text_box.map(StoryRef::Part)).collect();
    p.items.iter().find_map(|it| match it {
        Placed::Lines { story, .. } if boxes.contains(story) => None,
        Placed::Lines { story, .. } if it.turned_bounds().is_some() => {
            it.turned_bounds().filter(|r| r.expand(2.0).contains(Point::new(x, y))).map(|_| *story)
        }
        Placed::Lines { story, para, l0, l1, x: lx, y: ly, .. } => {
            let first = para.lines.get(*l0)?;
            let last = para.lines.get(l1.checked_sub(1)?)?;
            let bottom = ly + last.top + last.height - first.top;
            (y >= *ly - 2.0 && y <= bottom + 2.0 && x >= *lx - 40.0 && x <= lx + last.right + 40.0).then_some(*story)
        }
        _ => None,
    })
}

fn items_of(page: &Page, story: StoryRef) -> Box<dyn Iterator<Item = &Placed> + '_> {
    match story {
        StoryRef::Body => Box::new(page.items.iter()),
        // Headers and footers, and the text boxes in them, are drawn with the page's header and
        // footer; everything else is in its items.
        StoryRef::Part(_) => Box::new(page.header.iter().chain(page.footer.iter()).chain(page.items.iter())),
    }
}

/// Every upright line on a page that belongs to `story` (body, or a header/footer part).
/// Turned lines (see [`turned_hit`]) are left out.
pub fn page_lines(page: &Page, story: StoryRef) -> Vec<LineHit<'_>> {
    let mut v = Vec::new();
    for it in items_of(page, story) {
        if let Placed::Lines { story: s, path, para, l0, l1, x, y, turn } = it {
            if *s != story || turn.is_turned() {
                continue;
            }
            let Some(first) = para.lines.get(*l0) else { continue };
            for li in *l0..*l1 {
                let Some(l) = para.lines.get(li) else { continue };
                let top = y + (l.top - first.top);
                v.push(LineHit { story: *s, path, para, li, x: *x, top, bottom: top + l.height, left: x + l.left, right: x + l.right });
            }
        }
    }
    v
}

/// The position in turned text (a table cell's text direction) of `story` under page point
/// (x, y), if there is turned text there.
fn turned_hit(page: &Page, story: StoryRef, x: f32, y: f32) -> Option<Pos> {
    items_of(page, story).find_map(|it| {
        let Placed::Lines { story: s, path, para, l0, l1, x: ox, y: oy, turn } = it else { return None };
        if *s != story || !it.turned_bounds()?.expand(2.0).contains(Point::new(x, y)) {
            return None;
        }
        let (u, v) = crate::unturn_point(*turn, *ox, *oy, x, y);
        let first = para.lines.get(*l0)?;
        // The line across whose band v falls, else the nearest.
        let li = (*l0..*l1)
            .filter_map(|li| para.lines.get(li).map(|l| (li, l.top - first.top, l.height)))
            .min_by(|a, b| band_dist(v, a.1, a.2).total_cmp(&band_dist(v, b.1, b.2)))?
            .0;
        Some(Pos { story: *s, path: path.clone(), off: para.off_at_x(li, u) })
    })
}

/// How far `v` is outside the band `top..top + h`.
fn band_dist(v: f32, top: f32, h: f32) -> f32 {
    if v < top {
        top - v
    } else if v > top + h {
        v - top - h
    } else {
        0.0
    }
}

impl DocLayout {
    /// The position nearest to (x, y) on `page`, in `story`.
    pub fn hit(&self, page: usize, x: f32, y: f32, story: StoryRef) -> Option<Pos> {
        let p = self.pages.get(page)?;
        if let Some(pos) = turned_hit(p, story, x, y) {
            return Some(pos);
        }
        let lines = page_lines(p, story);
        let best = lines.iter().min_by(|a, b| score(a, x, y).total_cmp(&score(b, x, y)))?;
        let off = best.para.off_at_x(best.li, x - best.x);
        Some(Pos { story: best.story, path: best.path.clone(), off })
    }

    /// The story whose text is under a point (body, footnotes, endnotes, text boxes; not
    /// headers/footers). A text box wins inside its area, except one behind the text, which only
    /// wins where no body text is.
    pub fn story_at(&self, page: usize, x: f32, y: f32) -> Option<StoryRef> {
        let p = self.pages.get(page)?;
        let text_box = |behind: bool| {
            objects(p, page).find(|o| o.behind() == behind && o.text_box.is_some() && o.rect.contains(Point::new(x, y))).and_then(|o| o.text_box)
        };
        text_box(false).map(StoryRef::Part).or_else(|| text_at(p, page, x, y)).or_else(|| text_box(true).map(StoryRef::Part))
    }

    /// The object a press at (x, y) grabs: a picture or shape anywhere on it, a text box on its
    /// border (within `edge` points either side; inside is its text). Topmost first; objects
    /// behind the text only where there's no text.
    pub fn object_at(&self, page: usize, x: f32, y: f32, edge: f32) -> Option<ObjectHit> {
        let p = self.pages.get(page)?;
        // A rotated object is grabbed on its rotated shape: the point is taken into its frame.
        let grabs = |o: &ObjectHit| {
            let pt = o.unspin(x, y);
            o.rect.expand(edge).contains(pt) && (o.text_box.is_none() || !o.rect.expand(-edge).contains(pt) || o.rect.w.min(o.rect.h) <= edge * 3.0)
        };
        objects(p, page)
            .find(|o| !o.behind() && grabs(o))
            .or_else(|| if text_at(p, page, x, y).is_some() { None } else { objects(p, page).find(|o| o.behind() && grabs(o)) })
    }

    /// The text box in the header or footer at (x, y) on `page`.
    pub fn header_footer_text_box_at(&self, page: usize, x: f32, y: f32) -> Option<u32> {
        let p = self.pages.get(page)?;
        objects_in(&p.header, page)
            .chain(objects_in(&p.footer, page))
            .find(|o| o.text_box.is_some() && o.rect.contains(Point::new(x, y)))
            .and_then(|o| o.text_box)
    }

    /// Where the object at `pos` (its U+FFFC) is laid out.
    pub fn object(&self, pos: &Pos, page_hint: usize) -> Option<ObjectHit> {
        self.find_object(page_hint, |o| o.story == pos.story && o.path == pos.path && o.off == pos.off)
    }

    /// Where the text box showing story `part` is laid out.
    pub fn text_box(&self, part: u32, page_hint: usize) -> Option<ObjectHit> {
        self.find_object(page_hint, |o| o.text_box == Some(part))
    }

    /// The first object matching `f`, looking on `page_hint` first (where the caret is: one page
    /// to scan, not the whole document).
    pub fn find_object(&self, page_hint: usize, f: impl Fn(&ObjectHit) -> bool) -> Option<ObjectHit> {
        let on = |i: usize| self.pages.get(i).and_then(|p| all_objects(p, i).find(|o| f(o)));
        on(page_hint).or_else(|| (0..self.pages.len()).filter(|i| *i != page_hint).find_map(on))
    }

    /// Which header/footer (if any) is at (x, y) on a page — for double-click editing.
    pub fn header_footer_at(&self, page: usize, y: f32) -> Option<(StoryRef, bool)> {
        let p = self.pages.get(page)?;
        if y < p.body.y {
            return p.header_story.map(|id| (StoryRef::Part(id), true));
        }
        if y > p.body.bottom() {
            return p.footer_story.map(|id| (StoryRef::Part(id), false));
        }
        None
    }

    /// Pieces of a paragraph: (page, item index).
    fn pieces(&self, story: StoryRef, path: &Path, page_hint: usize) -> Vec<(usize, &Placed)> {
        if story == StoryRef::Body {
            return self
                .index
                .get(&(story, path.clone()))
                .map(|v| v.iter().filter_map(|(pi, ii)| self.pages.get(*pi).and_then(|p| p.items.get(*ii)).map(|it| (*pi, it))).collect())
                .unwrap_or_default();
        }
        // Headers/footers repeat: prefer the hinted page, else the first that shows the part.
        let mut order: Vec<usize> = (0..self.pages.len()).collect();
        if page_hint < order.len() {
            order.retain(|p| *p != page_hint);
            order.insert(0, page_hint);
        }
        for pi in order {
            let mut found = self.pieces_on(pi, story, path);
            if found.is_empty() {
                continue;
            }
            // A note continued over pages has the rest of the paragraph on the pages before or
            // after (a repeated header shows all of it on every page: one page is enough).
            let lines = |v: &[(usize, &Placed)]| {
                v.iter()
                    .filter_map(|(_, it)| if let Placed::Lines { para, l0, l1, .. } = it { Some((*l0, *l1, para.lines.len())) } else { None })
                    .fold((usize::MAX, 0, 0), |(a, b, _), (l0, l1, n)| (a.min(l0), b.max(l1), n))
            };
            let (mut lo, mut hi, n) = lines(&found);
            let mut k = pi;
            while lo > 0 && k > 0 {
                k -= 1;
                let more = self.pieces_on(k, story, path);
                if more.is_empty() {
                    break;
                }
                lo = lo.min(lines(&more).0);
                found.extend(more);
            }
            let mut k = pi;
            while hi < n && k + 1 < self.pages.len() {
                k += 1;
                let more = self.pieces_on(k, story, path);
                if more.is_empty() {
                    break;
                }
                hi = hi.max(lines(&more).1);
                found.extend(more);
            }
            return found;
        }
        Vec::new()
    }

    /// Pieces of a paragraph of story part `story` on page `pi`.
    fn pieces_on(&self, pi: usize, story: StoryRef, path: &Path) -> Vec<(usize, &Placed)> {
        let Some(p) = self.pages.get(pi) else { return Vec::new() };
        items_of(p, story).filter(|it| matches!(it, Placed::Lines { story: s, path: q, .. } if *s == story && q == path)).map(|it| (pi, it)).collect()
    }

    /// Caret geometry for a position.
    pub fn caret(&self, pos: &Pos) -> Option<Caret> {
        self.caret_on(pos, 0)
    }

    pub fn caret_on(&self, pos: &Pos, page_hint: usize) -> Option<Caret> {
        for (pi, it) in self.pieces(pos.story, &pos.path, page_hint) {
            let Placed::Lines { para, l0, l1, x, y, turn, .. } = it else { continue };
            let li = para.line_of(pos.off);
            if li < *l0 || li >= *l1 {
                continue;
            }
            let first = para.lines.get(*l0)?;
            let l = para.lines.get(li)?;
            let cx = para.x_of(li, pos.off)?;
            // In the lines' own frame (turned text turns it onto the page).
            let top = l.top - first.top;
            // Caret spans the text height (ascent + descent), not the full line spacing.
            let (asc, desc) = para.caret_metrics(li);
            let base = top + (l.baseline - l.top);
            let ctop = (base - asc).max(top);
            let r = crate::turn_rect(*turn, *x, *y, Rect::new(cx, ctop, 0.0, (base + desc - ctop).max(4.0)));
            return Some(Caret { page: pi, x: r.x, top: r.y, height: r.h, width: r.w });
        }
        None
    }

    /// The position one line up (`dir < 0`) or down from `pos`, keeping `goal_x` (page x).
    pub fn vertical(&self, pos: &Pos, goal_x: Option<f32>, dir: i32, page_hint: usize) -> Option<(Pos, f32)> {
        let c = self.caret_on(pos, page_hint)?;
        let gx = goal_x.unwrap_or(c.x);
        let page = self.pages.get(c.page)?;
        let lines = page_lines(page, pos.story);
        let cur_mid = c.top + c.height / 2.0;
        let cands: Vec<&LineHit> = lines
            .iter()
            .filter(|l| {
                let mid = (l.top + l.bottom) / 2.0;
                if dir < 0 {
                    mid < cur_mid - 1.0 && l.bottom <= c.top + c.height * 0.5
                } else {
                    mid > cur_mid + 1.0 && l.top >= c.top + c.height * 0.5
                }
            })
            .collect();
        let target = if dir < 0 {
            cands
                .iter()
                .filter(|l| gx >= l.left - 2.0 && gx <= l.right + 2.0)
                .max_by(|a, b| a.top.total_cmp(&b.top))
                .or_else(|| cands.iter().max_by(|a, b| a.top.total_cmp(&b.top)))
        } else {
            cands
                .iter()
                .filter(|l| gx >= l.left - 2.0 && gx <= l.right + 2.0)
                .min_by(|a, b| a.top.total_cmp(&b.top))
                .or_else(|| cands.iter().min_by(|a, b| a.top.total_cmp(&b.top)))
        };
        if let Some(l) = target {
            let off = l.para.off_at_x(l.li, gx - l.x);
            return Some((Pos { story: l.story, path: l.path.clone(), off }, gx));
        }
        // Next/previous page.
        let np = if dir < 0 { c.page.checked_sub(1)? } else { c.page + 1 };
        let p = self.pages.get(np)?;
        let lines = page_lines(p, pos.story);
        let l = if dir < 0 { lines.iter().max_by(|a, b| a.top.total_cmp(&b.top))? } else { lines.iter().min_by(|a, b| a.top.total_cmp(&b.top))? };
        let off = l.para.off_at_x(l.li, gx - l.x);
        Some((Pos { story: l.story, path: l.path.clone(), off }, gx))
    }

    /// Start and end of the visual line holding `pos` (Home / End).
    pub fn line_bounds(&self, pos: &Pos, page_hint: usize) -> Option<(Pos, Pos)> {
        for (_, it) in self.pieces(pos.story, &pos.path, page_hint) {
            let Placed::Lines { para, l0, l1, .. } = it else { continue };
            let li = para.line_of(pos.off);
            if li < *l0 || li >= *l1 {
                continue;
            }
            let l = para.lines.get(li)?;
            let mut end = l.stop;
            // Don't put the caret after a trailing break on that line, nor past wrap spaces.
            if matches!(l.end, crate::LineEnd::LineBreak | crate::LineEnd::PageBreak | crate::LineEnd::ColumnBreak) && end > l.start {
                end = para.clusters.get(l.c1.saturating_sub(1)).map(|c| c.start).unwrap_or(end);
            } else if l.end == crate::LineEnd::Wrap {
                // Before the wrapped line's trailing space: its last reachable offset, in logical
                // order (the far edge is the left one in right-to-left text).
                end = para.line_offsets(li).into_iter().max().unwrap_or(end);
            }
            let mk = |off| Pos { story: pos.story, path: pos.path.clone(), off };
            return Some((mk(l.start), mk(end)));
        }
        None
    }

    /// Column (block) selection: the piece of every line from the line holding `a` to the line
    /// holding `b` (document order) that lies between page x `left` and `right`. One `(start,
    /// end)` pair per line, both in the same paragraph; lines shorter than `left` give an empty
    /// pair. At most `max` lines.
    pub fn column_segments(&self, doc: &Document, a: &Pos, b: &Pos, left: f32, right: f32, page_hint: usize, max: usize) -> Vec<(Pos, Pos)> {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let (left, right) = if left <= right { (left, right) } else { (right, left) };
        let mut out = Vec::new();
        if a.story != b.story || !left.is_finite() || !right.is_finite() {
            return out;
        }
        for path in doc.paths_between(a, b) {
            for (_, it) in self.pieces(a.story, &path, page_hint) {
                let Placed::Lines { para, l0, l1, x, .. } = it else { continue };
                let first = if path == a.path { para.line_of(a.off) } else { 0 };
                let last = if path == b.path { para.line_of(b.off) } else { usize::MAX };
                for li in *l0..*l1 {
                    if li < first || li > last {
                        continue;
                    }
                    if out.len() >= max {
                        return out;
                    }
                    let o0 = para.off_at_x(li, left - x);
                    let o1 = para.off_at_x(li, right - x);
                    let (o0, o1) = if o0 <= o1 { (o0, o1) } else { (o1, o0) };
                    let mk = |off| Pos { story: a.story, path: path.clone(), off };
                    out.push((mk(o0), mk(o1)));
                }
            }
        }
        out
    }

    /// Highlight rectangles for the selection `a..b`, per page.
    pub fn selection_rects(&self, doc: &Document, a: &Pos, b: &Pos, page_hint: usize) -> Vec<(usize, Rect)> {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = Vec::new();
        for path in doc.paths_between(a, b) {
            for (pi, it) in self.pieces(a.story, &path, page_hint) {
                let Placed::Lines { para, l0, l1, x, y, turn, .. } = it else { continue };
                let Some(first) = para.lines.get(*l0) else { continue };
                // Rectangles in the lines' own frame, turned onto the page.
                let (ox, oy, turn) = (*x, *y, *turn);
                let mut put = |r: Rect| out.push((pi, crate::turn_rect(turn, ox, oy, r)));
                for li in *l0..*l1 {
                    let Some(l) = para.lines.get(li) else { continue };
                    let from = if path == a.path { a.off.max(l.start) } else { l.start };
                    let to = if path == b.path { b.off.min(l.stop) } else { l.stop };
                    if path == a.path && a.off > l.stop {
                        continue;
                    }
                    if path == b.path && b.off < l.start {
                        continue;
                    }
                    if from > to {
                        continue;
                    }
                    let top = l.top - first.top;
                    if !l.vis.is_empty() {
                        // Bidirectional line: the selected text can be several visual pieces.
                        for (x0, x1) in para.x_spans(li, from, to) {
                            put(Rect::new(x0, top, (x1 - x0).max(0.0), l.height));
                        }
                        // Selection continuing past the paragraph end shows the paragraph mark.
                        if li + 1 == para.lines.len() && path != b.path {
                            let ex = l.end_x();
                            let x0 = if l.rtl { ex - 5.0 } else { ex };
                            put(Rect::new(x0, top, 5.0, l.height));
                        }
                        continue;
                    }
                    let x0 = para.x_of(li, from).unwrap_or(l.left);
                    let mut x1 = para.x_of(li, to).unwrap_or(x0);
                    // Selection continuing past the paragraph end shows the paragraph mark.
                    let last_line = li + 1 == para.lines.len();
                    if last_line && path != b.path {
                        x1 += 5.0;
                    }
                    if path == b.path && to >= l.stop && li + 1 < para.lines.len() && b.off > l.stop {
                        x1 = para.x_of(li, l.stop).unwrap_or(x1);
                    }
                    if x1 <= x0 && !(last_line && path != b.path) {
                        continue;
                    }
                    put(Rect::new(x0, top, (x1 - x0).max(0.0), l.height));
                }
            }
        }
        out
    }

    /// One step left or right from `pos` in visual order on its line (arrow keys in
    /// bidirectional text). `Edge` when `pos` is already at that end of the line.
    pub fn visual_step(&self, pos: &Pos, left: bool, page_hint: usize) -> Option<VisualStep> {
        for (_, it) in self.pieces(pos.story, &pos.path, page_hint) {
            let Placed::Lines { para, l0, l1, .. } = it else { continue };
            let li = para.line_of(pos.off);
            if li < *l0 || li >= *l1 {
                continue;
            }
            let l = para.lines.get(li)?;
            let here = para.x_of(li, pos.off)?;
            let mut best: Option<(usize, f32)> = None;
            for off in para.line_offsets(li) {
                let Some(x) = para.x_of(li, off) else { continue };
                let ahead = if left { x < here - 0.01 } else { x > here + 0.01 };
                let d = (x - here).abs();
                if ahead && best.is_none_or(|b| d < b.1) {
                    best = Some((off, d));
                }
            }
            return Some(match best {
                Some((off, _)) => VisualStep::Moved(Pos { story: pos.story, path: pos.path.clone(), off }),
                None => VisualStep::Edge { start: l.start, stop: l.stop, last_line: li + 1 == para.lines.len(), rtl: l.rtl },
            });
        }
        None
    }

    /// Where the equation at `pos` (its U+FFFC) is drawn: page, x of its left edge and its
    /// baseline (page coordinates), and its layout.
    pub fn equation_geom(&self, pos: &Pos, page_hint: usize) -> Option<(usize, f32, f32, std::sync::Arc<crate::math::MathLayout>)> {
        for (pi, it) in self.pieces(pos.story, &pos.path, page_hint) {
            // Equations in turned text aren't edited in place.
            let Placed::Lines { para, l0, l1, x, y, turn: TextDirection::Horizontal, .. } = it else { continue };
            let li = para.line_of(pos.off);
            if li < *l0 || li >= *l1 {
                continue;
            }
            let first = para.lines.get(*l0)?;
            let l = para.lines.get(li)?;
            for k in l.c0..l.c1 {
                let Some(c) = para.clusters.get(k) else { continue };
                let crate::para::ClKind::Object(oi) = c.kind else { continue };
                if c.start != pos.off {
                    continue;
                }
                let ml = para.maths.iter().find(|(o, _)| *o == oi)?.1.clone();
                let cx = l.cl_left(k)?;
                let base = y + (l.baseline - first.top);
                return Some((pi, x + cx, base, ml));
            }
        }
        None
    }

    /// The equation under (x, y) on `page` and the caret position inside it nearest the point.
    pub fn equation_hit(&self, page: usize, x: f32, y: f32, story: StoryRef) -> Option<(Pos, wordcraft_doc::math_edit::MathPos)> {
        let p = self.pages.get(page)?;
        for l in page_lines(p, story) {
            let Some(line) = l.para.lines.get(l.li) else { continue };
            let base = l.top + (line.baseline - line.top);
            for k in line.c0..line.c1 {
                let Some(c) = l.para.clusters.get(k) else { continue };
                let crate::para::ClKind::Object(oi) = c.kind else { continue };
                let Some((_, ml)) = l.para.maths.iter().find(|(o, _)| *o == oi) else { continue };
                let x0 = l.x + line.cl_left(k).unwrap_or(0.0);
                let pad = 2.0;
                if x < x0 - pad || x > x0 + ml.width + pad || y < base - ml.ascent - pad || y > base + ml.descent + pad {
                    continue;
                }
                let slot = ml.hit(x - x0, base - y)?;
                let pos = Pos { story: l.story, path: l.path.clone(), off: c.start };
                return Some((pos, wordcraft_doc::math_edit::MathPos { path: slot.path.clone(), off: slot.off }));
            }
        }
        None
    }

    /// Page index for a body position (first page showing its line).
    pub fn page_of(&self, pos: &Pos) -> Option<usize> {
        self.caret(pos).map(|c| c.page)
    }

    /// Table cell under a point: (story, table path, row, cell).
    pub fn cell_at(&self, page: usize, x: f32, y: f32) -> Option<(StoryRef, Path, usize, usize)> {
        let p = self.pages.get(page)?;
        p.items.iter().rev().find_map(|it| match it {
            Placed::Cell { rect, table, row, cell, story } if rect.contains(wordcraft_geom::Point::new(x, y)) => {
                Some((*story, table.clone(), *row, *cell))
            }
            _ => None,
        })
    }
}

fn score(l: &LineHit, x: f32, y: f32) -> f32 {
    let dy = if y < l.top {
        l.top - y
    } else if y > l.bottom {
        y - l.bottom
    } else {
        0.0
    };
    let dx = if x < l.left - 4.0 {
        l.left - 4.0 - x
    } else if x > l.right + 4.0 {
        x - l.right - 4.0
    } else {
        0.0
    };
    dy * 4.0 + dx
}
