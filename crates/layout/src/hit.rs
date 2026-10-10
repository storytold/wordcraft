//! Hit testing and caret geometry.

use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_geom::Rect;

use crate::para::ParaLayout;
use crate::{DocLayout, Page, Placed};

/// Caret geometry on a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub page: usize,
    pub x: f32,
    pub top: f32,
    pub height: f32,
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

fn items_of(page: &Page, story: StoryRef) -> Box<dyn Iterator<Item = &Placed> + '_> {
    match story {
        StoryRef::Body => Box::new(page.items.iter()),
        StoryRef::Part(id) => {
            let h = page.header_story == Some(id);
            let f = page.footer_story == Some(id);
            Box::new(page.header.iter().filter(move |_| h).chain(page.footer.iter().filter(move |_| f)).chain(page.items.iter()))
        }
    }
}

/// Every line on a page that belongs to `story` (body, or a header/footer part).
pub fn page_lines(page: &Page, story: StoryRef) -> Vec<LineHit<'_>> {
    let mut v = Vec::new();
    for it in items_of(page, story) {
        if let Placed::Lines { story: s, path, para, l0, l1, x, y } = it {
            if *s != story {
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

impl DocLayout {
    /// The position nearest to (x, y) on `page`, in `story`.
    pub fn hit(&self, page: usize, x: f32, y: f32, story: StoryRef) -> Option<Pos> {
        let p = self.pages.get(page)?;
        let lines = page_lines(p, story);
        let best = lines.iter().min_by(|a, b| score(a, x, y).total_cmp(&score(b, x, y)))?;
        let off = best.para.off_at_x(best.li, x - best.x);
        Some(Pos { story: best.story, path: best.path.clone(), off })
    }

    /// The story whose text is under a point (body, footnotes, endnotes; not headers/footers).
    pub fn story_at(&self, page: usize, x: f32, y: f32) -> Option<StoryRef> {
        let p = self.pages.get(page)?;
        p.items.iter().find_map(|it| match it {
            Placed::Lines { story, para, l0, l1, x: lx, y: ly, .. } => {
                let first = para.lines.get(*l0)?;
                let last = para.lines.get(l1.checked_sub(1)?)?;
                let bottom = ly + last.top + last.height - first.top;
                (y >= *ly - 2.0 && y <= bottom + 2.0 && x >= *lx - 40.0 && x <= lx + last.right + 40.0).then_some(*story)
            }
            _ => None,
        })
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
            let Some(p) = self.pages.get(pi) else { continue };
            let found: Vec<(usize, &Placed)> = items_of(p, story)
                .filter(|it| matches!(it, Placed::Lines { story: s, path: q, .. } if *s == story && q == path))
                .map(|it| (pi, it))
                .collect();
            if !found.is_empty() {
                return found;
            }
        }
        Vec::new()
    }

    /// Caret geometry for a position.
    pub fn caret(&self, pos: &Pos) -> Option<Caret> {
        self.caret_on(pos, 0)
    }

    pub fn caret_on(&self, pos: &Pos, page_hint: usize) -> Option<Caret> {
        for (pi, it) in self.pieces(pos.story, &pos.path, page_hint) {
            let Placed::Lines { para, l0, l1, x, y, .. } = it else { continue };
            let li = para.line_of(pos.off);
            if li < *l0 || li >= *l1 {
                continue;
            }
            let first = para.lines.get(*l0)?;
            let l = para.lines.get(li)?;
            let cx = para.x_of(li, pos.off)?;
            let top = y + (l.top - first.top);
            // Caret spans the text height (ascent + descent), not the full line spacing.
            let (asc, desc) = para.caret_metrics(li);
            let base = top + (l.baseline - l.top);
            let h = (asc + desc).min(l.height.max(asc + desc));
            let ctop = (base - asc).max(top);
            let _ = h;
            return Some(Caret { page: pi, x: x + cx, top: ctop, height: (base + desc - ctop).max(4.0) });
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
                end = para.off_at_x(li, l.right + 1000.0);
            }
            let mk = |off| Pos { story: pos.story, path: pos.path.clone(), off };
            return Some((mk(l.start), mk(end)));
        }
        None
    }

    /// Highlight rectangles for the selection `a..b`, per page.
    pub fn selection_rects(&self, doc: &Document, a: &Pos, b: &Pos, page_hint: usize) -> Vec<(usize, Rect)> {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = Vec::new();
        for path in doc.paths_between(a, b) {
            for (pi, it) in self.pieces(a.story, &path, page_hint) {
                let Placed::Lines { para, l0, l1, x, y, .. } = it else { continue };
                let Some(first) = para.lines.get(*l0) else { continue };
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
                    let top = y + (l.top - first.top);
                    out.push((pi, Rect::new(x + x0, top, (x1 - x0).max(0.0), l.height)));
                }
            }
        }
        out
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
