//! Hit testing and caret geometry.

use wordcraft_doc::para::Wrap;
use wordcraft_doc::props::TextDirection;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_geom::{Point, Rect};

use crate::para::{PARA_MARK_WIDTH, ParaLayout};
use crate::{DocLayout, Page, Placed, cell_clip, clipped_cells};

struct EquationPlacement {
    page: usize,
    x: f32,
    baseline: f32,
    math: std::sync::Arc<crate::math::MathLayout>,
    clip: Option<Rect>,
}

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
    slice_lines(&p.items).into_iter().find_map(|(it, clip)| match it {
        Placed::Lines { story, .. } if boxes.contains(story) || *story == crate::DECOR_STORY || !point_in_clip(clip, x, y) => None,
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

/// Whether header/footer part `id` has text near (x, y) on the page (its header or footer
/// lines). Used so an object behind the header text is only grabbed where there's no text,
/// mirroring `text_at` for the body.
fn hf_text_at(p: &Page, id: u32, x: f32, y: f32) -> bool {
    let story = StoryRef::Part(id);
    [&p.header[..], &p.footer[..]]
        .into_iter()
        .flat_map(slice_lines)
        .filter(|(_, clip)| point_in_clip(*clip, x, y))
        .filter_map(|(it, _)| match it {
            Placed::Lines { story: s, para, l0, l1, x: lx, y: ly, .. } if *s == story => Some((para, l0, l1, lx, ly)),
            _ => None,
        })
        .any(|(para, l0, l1, lx, ly)| {
            let (Some(first), Some(last)) = (para.lines.get(*l0), para.lines.get(l1.checked_sub(1).unwrap_or(0))) else { return false };
            let bottom = ly + last.top + last.height - first.top;
            y >= *ly - 2.0 && y <= bottom + 2.0 && x >= *lx - 40.0 && x <= lx + last.right + 40.0
        })
}

/// Pair each physical line item with its own slice's intersected cell clip.
fn slice_lines(items: &[Placed]) -> Vec<(&Placed, Option<Rect>)> {
    let clips = clipped_cells(items);
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let Placed::Lines { story, path, .. } = item else { return None };
            Some((item, cell_clip(&clips, *story, &path.0, index)))
        })
        .collect()
}

fn line_items(page: &Page, story: StoryRef) -> Vec<(&Placed, Option<Rect>)> {
    let slices: &[&[Placed]] = match story {
        StoryRef::Body => &[&page.items],
        StoryRef::Part(_) => &[&page.header, &page.footer, &page.items],
    };
    slices.iter().flat_map(|items| slice_lines(items)).filter(|(item, _)| matches!(item, Placed::Lines { story: s, .. } if *s == story)).collect()
}

fn point_in_clip(clip: Option<Rect>, x: f32, y: f32) -> bool {
    clip.is_none_or(|r| r.w > 0.0 && r.h > 0.0 && r.contains(Point::new(x, y)))
}

/// Intersect screen geometry without discarding a vertical or horizontal caret.
fn clip_geometry(rect: Rect, clip: Option<Rect>) -> Option<Rect> {
    let Some(clip) = clip else { return Some(rect) };
    if clip.w <= 0.0 || clip.h <= 0.0 {
        return None;
    }
    let (x, y) = (rect.x.max(clip.x), rect.y.max(clip.y));
    let (right, bottom) = (rect.right().min(clip.right()), rect.bottom().min(clip.bottom()));
    if right < x || bottom < y || (rect.w > 0.0 && right <= x) || (rect.h > 0.0 && bottom <= y) {
        return None;
    }
    Some(Rect::new(x, y, right - x, bottom - y))
}

/// Text ink excludes leading-only intersections. Blank and object lines keep their caret band.
fn cell_text_band(para: &ParaLayout, li: usize) -> Option<Rect> {
    let line = para.lines.get(li)?;
    let logical = Rect::new(line.left, line.top, (line.right - line.left).max(0.0), line.height);
    let mut top = f32::INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for ci in line.c0..line.c1 {
        let Some(cluster) = para.clusters.get(ci) else { continue };
        if matches!(cluster.kind, crate::para::ClKind::Object(_)) || !para.rubies.is_empty() || line.hyphen.is_some() {
            return Some(logical);
        }
        let Some(style) = para.styles.get(cluster.style as usize) else { continue };
        let scale = style.size / style.face.get().units_per_em() as f32;
        for gi in cluster.g0..cluster.g1 {
            let Some(glyph) = para.glyphs.get(gi as usize) else { continue };
            let Some((_, low, _, high)) = wordcraft_fonts::math::glyph_bounds(style.face.get(), glyph.gid) else {
                // Bitmap/color glyphs need not have an outline.
                if cluster.kind == crate::para::ClKind::Text {
                    return Some(logical);
                }
                continue;
            };
            let base = line.baseline - style.shift - glyph.dy;
            top = top.min(base - high * scale);
            bottom = bottom.max(base - low * scale);
        }
    }
    if bottom > top { Some(Rect::new(logical.x, top, logical.w, bottom - top)) } else { Some(logical) }
}

/// Every upright line on a page that belongs to `story` (body, or a header/footer part).
/// Turned lines (see [`turned_hit`]) are left out.
pub fn page_lines(page: &Page, story: StoryRef) -> Vec<LineHit<'_>> {
    hit_lines(page, story).into_iter().map(|(line, _)| line).collect()
}

fn hit_lines(page: &Page, story: StoryRef) -> Vec<(LineHit<'_>, Option<Rect>)> {
    let mut v = Vec::new();
    for (it, clip) in line_items(page, story) {
        if let Placed::Lines { story: s, path, para, l0, l1, x, y, turn } = it {
            if turn.is_turned() {
                continue;
            }
            let Some(first) = para.lines.get(*l0) else { continue };
            for li in *l0..*l1 {
                let Some(l) = para.lines.get(li) else { continue };
                let top = y + (l.top - first.top);
                let band = Rect::new(x + l.left, top, (l.right - l.left).max(0.0), l.height);
                if clip_geometry(band, clip).is_none() {
                    continue;
                }
                if clip.is_some()
                    && cell_text_band(para, li).is_some_and(|r| clip_geometry(Rect::new(x + r.x, y + r.y - first.top, r.w, r.h), clip).is_none())
                {
                    continue;
                }
                // Keep intrinsic coordinates for baseline users; only interaction results clip.
                v.push((LineHit { story: *s, path, para, li, x: *x, top, bottom: top + l.height, left: x + l.left, right: x + l.right }, clip));
            }
        }
    }
    v
}

/// Of `lines`, the one nearest the caret moving up (`dir < 0`) or down: the lowest above it or
/// the highest below it.
fn nearest<'a, 'b>(lines: &[&'b LineHit<'a>], dir: i32) -> Option<&'b LineHit<'a>> {
    if dir < 0 { lines.iter().copied().max_by(|a, b| a.top.total_cmp(&b.top)) } else { lines.iter().copied().min_by(|a, b| a.top.total_cmp(&b.top)) }
}

/// The line of `lines` to go to: of the nearest one and the lines of the other cells in its table
/// row, the one across page x `gx` (else the one horizontally nearest it), on a tie the nearer.
fn in_row<'a, 'b>(lines: &[&'b LineHit<'a>], gx: f32, dir: i32) -> Option<&'b LineHit<'a>> {
    let n = nearest(lines, dir)?;
    let off = |l: &LineHit| (l.left - gx).max(gx - l.right).max(0.0);
    lines
        .iter()
        .copied()
        .filter(|l| (l.path == n.path && l.li == n.li) || beside(n.path, l.path))
        .min_by(|a, b| off(a).total_cmp(&off(b)).then(if dir < 0 { b.top.total_cmp(&a.top) } else { a.top.total_cmp(&b.top) }))
}

/// Whether paragraph `l` is in another cell of the table row paragraph `p` is in: where their
/// paths part is a cell index (`[block, row, cell, block, …]`).
fn beside(p: &Path, l: &Path) -> bool {
    p.0.iter().zip(&l.0).position(|(x, y)| x != y).is_some_and(|i| i % 3 == 2)
}

/// Whether paragraph `l` comes between paragraphs `a` and `b` (either way round) in document
/// order, not in another cell of a table row either of them is in (that stands beside it).
fn between(a: &Path, b: &Path, l: &Path) -> bool {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    lo < l && l < hi && !beside(a, l) && !beside(b, l)
}

/// The position in turned text (a table cell's text direction) of `story` under page point
/// (x, y), if there is turned text there.
fn turned_hit(page: &Page, story: StoryRef, x: f32, y: f32) -> Option<Pos> {
    line_items(page, story).into_iter().find_map(|(it, clip)| {
        if !point_in_clip(clip, x, y) {
            return None;
        }
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

/// The position in the turned text of `story` nearest to page point (x, y), wherever it is.
fn nearest_turned(page: &Page, story: StoryRef, x: f32, y: f32) -> Option<Pos> {
    let dist = |r: Rect| band_dist(x, r.x, r.w).hypot(band_dist(y, r.y, r.h));
    let it = line_items(page, story)
        .into_iter()
        .filter(|(_, clip)| point_in_clip(*clip, x, y))
        .map(|(it, _)| it)
        .filter_map(|it| it.turned_bounds().map(|r| (it, dist(r))))
        .min_by(|a, b| a.1.total_cmp(&b.1))?
        .0;
    let Placed::Lines { path, para, l0, l1, x: ox, y: oy, turn, .. } = it else { return None };
    let (u, v) = crate::unturn_point(*turn, *ox, *oy, x, y);
    let first = para.lines.get(*l0)?;
    let li = (*l0..*l1)
        .filter_map(|li| para.lines.get(li).map(|l| (li, l.top - first.top, l.height)))
        .min_by(|a, b| band_dist(v, a.1, a.2).total_cmp(&band_dist(v, b.1, b.2)))?
        .0;
    Some(Pos { story, path: path.clone(), off: para.off_at_x(li, u) })
}

/// The body line on `page` whose band is nearest height `y` (the leftmost on a tie, so in
/// columns the first column wins); only top-level paragraphs' lines when `top_only`.
fn nearest_line_at<'a>(page: &'a Page, y: f32, top_only: bool) -> Option<LineHit<'a>> {
    hit_lines(page, StoryRef::Body)
        .into_iter()
        .filter(|(_, clip)| clip.is_none_or(|r| y >= r.y && y <= r.bottom()))
        .map(|(line, _)| line)
        .filter(|l| !top_only || l.path.0.len() <= 1)
        .min_by(|a, b| band_dist(y, a.top, a.bottom - a.top).total_cmp(&band_dist(y, b.top, b.bottom - b.top)).then(a.left.total_cmp(&b.left)))
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
        let lines = hit_lines(p, story);
        if lines.is_empty() {
            // A page of turned text (a vertical section): the nearest turned line.
            return nearest_turned(p, story, x, y);
        }
        let best =
            lines.iter().filter(|(_, clip)| point_in_clip(*clip, x, y)).min_by(|(a, _), (b, _)| score(a, x, y).total_cmp(&score(b, x, y)))?.0.clone();
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
        self.object_at_in(page, x, y, edge, StoryRef::Body)
    }

    /// The object of `story` a press at (x, y) grabs. Body looks only at the page's own items
    /// (as before); a header/footer part looks only at that part's objects in the page's
    /// header and footer, so a header picture placed over the body never steals body clicks.
    pub fn object_at_in(&self, page: usize, x: f32, y: f32, edge: f32, story: StoryRef) -> Option<ObjectHit> {
        let p = self.pages.get(page)?;
        // A rotated object is grabbed on its rotated shape: the point is taken into its frame.
        let grabs = |o: &ObjectHit| {
            let pt = o.unspin(x, y);
            o.rect.expand(edge).contains(pt) && (o.text_box.is_none() || !o.rect.expand(-edge).contains(pt) || o.rect.w.min(o.rect.h) <= edge * 3.0)
        };
        let cands: Vec<ObjectHit> = match story {
            StoryRef::Body => objects(p, page).collect(),
            StoryRef::Part(_) => all_objects(p, page).filter(|o| o.story == story).collect(),
        };
        let text_here = match story {
            StoryRef::Body => text_at(p, page, x, y).is_some(),
            StoryRef::Part(id) => hf_text_at(p, id, x, y),
        };
        cands
            .iter()
            .find(|o| !o.behind() && grabs(o))
            .or_else(|| if text_here { None } else { cands.iter().find(|o| o.behind() && grabs(o)) })
            .cloned()
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
        let (story, header) = self.header_footer_zone(page, y)?;
        story.map(|s| (s, header))
    }

    /// Whether y on `page` is in its header area (above the body: `Some((_, true))`) or footer
    /// area (below it: `Some((_, false))`), with the story shown there — `None` when the page
    /// has no header/footer yet (double-clicking there creates one, #960).
    pub fn header_footer_zone(&self, page: usize, y: f32) -> Option<(Option<StoryRef>, bool)> {
        let p = self.pages.get(page)?;
        if !y.is_finite() {
            return None;
        }
        if y < p.body.y {
            return Some((p.header_story.map(StoryRef::Part), true));
        }
        if y > p.body.bottom() {
            return Some((p.footer_story.map(StoryRef::Part), false));
        }
        None
    }

    /// Pieces of a paragraph: (page, item index).
    fn pieces(&self, story: StoryRef, path: &Path, page_hint: usize) -> Vec<(usize, &Placed, Option<Rect>)> {
        if story == StoryRef::Body {
            let mut clips = std::collections::HashMap::new();
            return self
                .index
                .get(&(story, path.clone()))
                .map(|v| {
                    v.iter()
                        .filter_map(|(pi, ii)| {
                            let page = self.pages.get(*pi)?;
                            let item = page.items.get(*ii)?;
                            let own = clips.entry(*pi).or_insert_with(|| clipped_cells(&page.items));
                            Some((*pi, item, cell_clip(own, story, &path.0, *ii)))
                        })
                        .collect()
                })
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
            let lines = |v: &[(usize, &Placed, Option<Rect>)]| {
                v.iter()
                    .filter_map(|(_, it, _)| if let Placed::Lines { para, l0, l1, .. } = it { Some((*l0, *l1, para.lines.len())) } else { None })
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
    fn pieces_on(&self, pi: usize, story: StoryRef, path: &Path) -> Vec<(usize, &Placed, Option<Rect>)> {
        let Some(p) = self.pages.get(pi) else { return Vec::new() };
        line_items(p, story)
            .into_iter()
            .filter(|(it, _)| matches!(it, Placed::Lines { path: q, .. } if q == path))
            .map(|(it, clip)| (pi, it, clip))
            .collect()
    }

    /// Caret geometry for a position.
    pub fn caret(&self, pos: &Pos) -> Option<Caret> {
        self.caret_on(pos, 0)
    }

    pub fn caret_on(&self, pos: &Pos, page_hint: usize) -> Option<Caret> {
        for (pi, it, clip) in self.pieces(pos.story, &pos.path, page_hint) {
            let Placed::Lines { para, l0, l1, x, y, turn, .. } = it else { continue };
            let li = para.line_of(pos.off);
            if li < *l0 || li >= *l1 {
                continue;
            }
            let first = para.lines.get(*l0)?;
            let l = para.lines.get(li)?;
            if clip.is_some()
                && cell_text_band(para, li)
                    .is_some_and(|r| clip_geometry(crate::turn_rect(*turn, *x, *y, Rect::new(r.x, r.y - first.top, r.w, r.h)), clip).is_none())
            {
                continue;
            }
            let cx = para.x_of(li, pos.off)?;
            let cx = para.caret_limit(li).map_or(cx, |m| cx.min(m));
            // In the lines' own frame (turned text turns it onto the page).
            let top = l.top - first.top;
            // Caret spans the text height (ascent + descent), not the full line spacing.
            let (asc, desc) = para.caret_metrics(li);
            let base = top + (l.baseline - l.top);
            let ctop = (base - asc).max(top);
            let r = crate::turn_rect(*turn, *x, *y, Rect::new(cx, ctop, 0.0, (base + desc - ctop).max(4.0)));
            let Some(r) = clip_geometry(r, clip) else { continue };
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
        let across: Vec<&LineHit> = cands.iter().copied().filter(|l| gx >= l.left - 2.0 && gx <= l.right + 2.0).collect();
        let target = match nearest(&across, dir) {
            // The nearest line across `gx`, unless lines between the caret's paragraph and that
            // line's in document order come before it on the way: a table row whose cells' text
            // `gx` misses (left of the table, or in the gap between two cells) is the next line,
            // as in Word.
            Some(t) => {
                let skipped: Vec<&LineHit> = cands
                    .iter()
                    .copied()
                    .filter(|l| (if dir < 0 { l.top > t.top } else { l.top < t.top }) && between(&pos.path, t.path, l.path))
                    .collect();
                if skipped.is_empty() { Some(t) } else { in_row(&skipped, gx, dir) }
            }
            None => in_row(&cands, gx, dir),
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
        for (_, it, _) in self.pieces(pos.story, &pos.path, page_hint) {
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

    /// Word's selection bar (#856): when (x, y) is in the page's left margin, left of the body
    /// text column, the start of the body line at that height. Not in the header/footer area,
    /// not beside a table's lines (those are left to the table), nothing on an empty page.
    pub fn selection_bar_at(&self, page: usize, x: f32, y: f32) -> Option<Pos> {
        let p = self.pages.get(page)?;
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || x >= p.body.x || y < p.body.y || y > p.body.bottom() {
            return None;
        }
        let l = nearest_line_at(p, y, false)?;
        if l.path.0.len() > 1 || l.left < p.body.x - 0.5 {
            // A table's line, or text hanging into the margin: not the selection bar.
            return None;
        }
        Some(Pos { story: StoryRef::Body, path: l.path.clone(), off: l.para.lines.get(l.li)?.start })
    }

    /// The start of the top-level body line nearest height `y` on `page` (dragging in the
    /// selection bar extends line by line, wherever the pointer is across).
    pub fn body_line_at(&self, page: usize, y: f32) -> Option<Pos> {
        let p = self.pages.get(page)?;
        if !y.is_finite() {
            return None;
        }
        let l = nearest_line_at(p, y, true)?;
        Some(Pos { story: StoryRef::Body, path: l.path.clone(), off: l.para.lines.get(l.li)?.start })
    }

    /// The whole visual line holding `pos`, trailing spaces and break included: (start, end,
    /// whether it is the paragraph's last line). What a click in the selection bar selects.
    pub fn line_span(&self, pos: &Pos, page_hint: usize) -> Option<(Pos, Pos, bool)> {
        for (_, it, _) in self.pieces(pos.story, &pos.path, page_hint) {
            let Placed::Lines { para, l0, l1, .. } = it else { continue };
            let li = para.line_of(pos.off);
            if li < *l0 || li >= *l1 {
                continue;
            }
            let l = para.lines.get(li)?;
            let next = para.lines.get(li.saturating_add(1));
            let end = next.map_or(l.stop, |n| n.start).max(l.start);
            let mk = |off| Pos { story: pos.story, path: pos.path.clone(), off };
            return Some((mk(l.start), mk(end), next.is_none()));
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
            for (_, it, _) in self.pieces(a.story, &path, page_hint) {
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
            for (pi, it, clip) in self.pieces(a.story, &path, page_hint) {
                let Placed::Lines { para, l0, l1, x, y, turn, .. } = it else { continue };
                let Some(first) = para.lines.get(*l0) else { continue };
                // Rectangles in the lines' own frame, turned onto the page.
                let (ox, oy, turn) = (*x, *y, *turn);
                let mut put = |r: Rect| {
                    if let Some(r) = clip_geometry(crate::turn_rect(turn, ox, oy, r), clip) {
                        out.push((pi, r));
                    }
                };
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
                            let x0 = if l.rtl { ex - PARA_MARK_WIDTH } else { ex };
                            put(Rect::new(x0, top, PARA_MARK_WIDTH, l.height));
                        }
                        continue;
                    }
                    let x0 = para.x_of(li, from).unwrap_or(l.left);
                    let mut x1 = para.x_of(li, to).unwrap_or(x0);
                    // Selection continuing past the paragraph end shows the paragraph mark.
                    let last_line = li + 1 == para.lines.len();
                    if last_line && path != b.path {
                        x1 += PARA_MARK_WIDTH;
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
        for (_, it, _) in self.pieces(pos.story, &pos.path, page_hint) {
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
    /// baseline (page coordinates), and its intrinsic layout. Clip overlays with `equation_clip`.
    pub fn equation_geom(&self, pos: &Pos, page_hint: usize) -> Option<(usize, f32, f32, std::sync::Arc<crate::math::MathLayout>)> {
        let placed = self.equation_placement(pos, page_hint)?;
        let bounds = Rect::new(placed.x, placed.baseline - placed.math.ascent, placed.math.width, placed.math.ascent + placed.math.descent);
        clip_geometry(bounds, placed.clip)?;
        Some((placed.page, placed.x, placed.baseline, placed.math))
    }

    /// Physical cell clip for the same occurrence selected by `equation_geom`, including empty clips.
    /// `None` means no owning cell clip or no placed equation. Visibility is decided by `equation_geom`.
    pub fn equation_clip(&self, pos: &Pos, page_hint: usize) -> Option<Rect> {
        self.equation_placement(pos, page_hint).and_then(|placed| placed.clip)
    }

    fn equation_placement(&self, pos: &Pos, page_hint: usize) -> Option<EquationPlacement> {
        for (pi, it, clip) in self.pieces(pos.story, &pos.path, page_hint) {
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
                return Some(EquationPlacement { page: pi, x: x + cx, baseline: base, math: ml, clip });
            }
        }
        None
    }

    /// The equation under (x, y) on `page` and the caret position inside it nearest the point.
    pub fn equation_hit(&self, page: usize, x: f32, y: f32, story: StoryRef) -> Option<(Pos, wordcraft_doc::math_edit::MathPos)> {
        let p = self.pages.get(page)?;
        for (l, clip) in hit_lines(p, story) {
            if !point_in_clip(clip, x, y) {
                continue;
            }
            let Some(line) = l.para.lines.get(l.li) else { continue };
            let base = l.top + (line.baseline - line.top);
            for k in line.c0..line.c1 {
                let Some(c) = l.para.clusters.get(k) else { continue };
                let crate::para::ClKind::Object(oi) = c.kind else { continue };
                let Some((_, ml)) = l.para.maths.iter().find(|(o, _)| *o == oi) else { continue };
                let x0 = l.x + line.cl_left(k).unwrap_or(0.0);
                if clip_geometry(Rect::new(x0, base - ml.ascent, ml.width, ml.ascent + ml.descent), clip).is_none() {
                    continue;
                }
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
            Placed::Cell { rect, table, row, cell, story, covered: false, .. } if rect.contains(wordcraft_geom::Point::new(x, y)) => {
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
