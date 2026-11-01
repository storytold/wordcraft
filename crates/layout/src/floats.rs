use super::ctx::Ctx;
use super::page::{PageBuilder, PageFrame};
use super::placed::Placed;
use super::table_flow::footer_table_wraps;
use super::{
    Anchor, Arc, CharProps, Document, Float, FloatAlign, HashMap, HideDeleted, InlineObject, Level, LineEnd, ParaLayout, Paragraph, Path, Point,
    Rect, SectionProps, Spin, StoryRef, Wrap, WrapText, contour, display, para, textbox,
};

/// Which of `p`'s objects (by index) the layout leaves out ([`para::left_out`]): anchored in hidden
/// text unless `show_hidden`, in a tracked deletion with `hide_deleted`, or a hidden drawing
/// (`Float::hidden`, whatever is shown). `table_chr` is the
/// cell's table-style character formatting ([`para::CellText::chr`]), resolved as the paragraph is
/// laid out with (`StyleSheet::resolve_char_in`).
pub(crate) fn left_out_objects(
    doc: &Document,
    p: &Paragraph,
    table_chr: Option<&CharProps>,
    show_hidden: bool,
    hide_deleted: HideDeleted,
) -> impl Fn(usize) -> bool {
    let mut left = Vec::new();
    // No text left out with hidden text shown and markup on (hidden drawings still are, below).
    if (hide_deleted.any() || !show_hidden) && !p.objects.is_empty() {
        let style = p.props.style.as_deref();
        let is_left = |c: &CharProps| {
            let rc = doc.styles.resolve_char_in(style, table_chr, c);
            para::left_out(doc, &rc, show_hidden, hide_deleted)
        };
        // Runs and objects are both in order: one walk finds each object's run (past the last run,
        // the paragraph mark, as `props_of_char` gives), resolving a run's formatting only once.
        let mut runs = p.run_ranges().peekable();
        let mut last: Option<(usize, bool)> = None;
        for off in p.object_offsets() {
            while runs.next_if(|(r, _)| r.end <= off).is_some() {}
            let l = match runs.peek() {
                Some((r, c)) => match last {
                    Some((end, l)) if end == r.end => l,
                    _ => {
                        let l = is_left(c);
                        last = Some((r.end, l));
                        l
                    }
                },
                None => is_left(&p.mark),
            };
            left.push(l);
        }
    }
    // A hidden drawing is left out whatever is shown (Word shows it in no view).
    move |k| left.get(k).copied().unwrap_or(false) || p.objects.get(k).and_then(InlineObject::frame).is_some_and(|(_, _, f)| f.hidden)
}

/// A floating (not inline) picture, chart or shape: its size and placement.
pub(crate) fn floating(o: &InlineObject) -> Option<(f32, f32, &Float)> {
    o.frame().filter(|(_, _, float)| float.wrap != Wrap::Inline)
}

/// Where a floating object goes, in the coordinates of the box `frame` places on the page.
/// `col` is the column (or cell) left edge and width, `y0` the anchor paragraph's top. Without a
/// frame (table cells, text boxes, notes) positions relative to the page or margins fall back
/// to the box itself.
pub(crate) fn float_rect(frame: Option<PageFrame>, col: (f32, f32), y0: f32, w: f32, h: f32, float: &Float) -> Rect {
    // Thinner than a point stays thin (a 0.25 pt rule, a flat line): Word keeps the size.
    let (w, h) = (wordcraft_geom::finite(w).clamp(0.0, 4000.0), wordcraft_geom::finite(h).clamp(0.0, 4000.0));
    // Each axis: the reference area's start and extent, in page coordinates. Across, as laid out
    // before an even page is mirrored (less the shift it then gets).
    let page_x = |f: &PageFrame| {
        let (s, even) = (f.sect, f.even());
        let (left, right, shift) = f.sides();
        let (start, extent) = match float.h_rel {
            Anchor::Page => (0.0, s.page_w),
            Anchor::Margin => (left, s.text_width()),
            Anchor::LeftMargin => (0.0, left),
            Anchor::RightMargin => (s.page_w - right, right),
            Anchor::InsideMargin if even => (s.page_w - right, right),
            Anchor::OutsideMargin if !even => (s.page_w - right, right),
            Anchor::InsideMargin | Anchor::OutsideMargin => (0.0, left),
            _ => return None,
        };
        Some((start - shift, extent))
    };
    let page_y = |s: &SectionProps| match float.v_rel {
        Anchor::Page => Some((0.0, s.page_h)),
        Anchor::Margin => Some((s.margin_top, s.page_h - s.margin_top - s.margin_bottom)),
        Anchor::TopMargin | Anchor::InsideMargin => Some((0.0, s.margin_top)),
        Anchor::BottomMargin | Anchor::OutsideMargin => Some((s.page_h - s.margin_bottom, s.margin_bottom)),
        _ => None,
    };
    let hx = frame.and_then(|f| page_x(&f).map(|(a, e)| (a - f.origin.0, e))).unwrap_or(col);
    let vy = frame.and_then(|f| page_y(f.sect).map(|(a, e)| (a - f.origin.1, e))).unwrap_or((y0, 0.0));
    let h_align = PageFrame::h_align(frame.as_ref(), float.h_align);
    // Without a frame (a cell) `vy` has no height to align in: the object starts at the anchor
    // line instead of being pushed up by half its height or more.
    let v_align = if frame.is_none() { None } else { float.v_align };
    Rect::new(align_in(hx, float.x, w, h_align), align_in(vy, float.y, h, v_align), w, h)
}

/// Where an object of `size` starts on one axis of `area` (start, extent): at `offset` from the
/// area's start, or aligned in it when `align` is set.
pub(crate) fn align_in((start, extent): (f32, f32), offset: f32, size: f32, align: Option<FloatAlign>) -> f32 {
    match align {
        None => start + if offset.is_finite() { offset.clamp(-31_680.0, 31_680.0) } else { 0.0 },
        Some(FloatAlign::Start | FloatAlign::Inside) => start,
        Some(FloatAlign::Center) => start + (extent - size) / 2.0,
        Some(FloatAlign::End | FloatAlign::Outside) => start + extent - size,
    }
}

/// An area text keeps clear of: its bounds, whether text may only go above and below it, and
/// (Tight and Through wrapping) the outline text follows inside it, the sides text may go on,
/// and whether it comes from a floating table (inclusive line-bottom contact, #3977).
pub(crate) type WrapArea = (Rect, bool, Option<Arc<contour::Contour>>, WrapText, bool);

/// Wrap areas a page starts with (#2048): by page index, the floating objects (block, object
/// index) placed above the top of their anchor paragraph, with text before it on the page, and
/// their areas. Layout places a float only when it reaches its anchor, so the text above that
/// was laid out without it: the pass is done again with these known from the page's start.
pub(crate) type EarlyFloats = HashMap<usize, Vec<(usize, usize, WrapArea)>>;

/// Layout passes made again at most for [`EarlyFloats`] (a float moving with the text it pushes
/// down may move again). The pass after the last one keeps each float clear of the text laid
/// out before it ([`settle_early`]).
pub(crate) const MAX_REWRAP_PASSES: usize = 3;

/// Whether the text kept clear of `seed` is kept clear of `area` too: the same table-contact
/// rule and wrap side, `area` inside `seed`, and no outline on `seed` other than `area`'s own
/// in the same place.
fn covers(seed: &WrapArea, area: &WrapArea) -> bool {
    let (s, r) = (seed.0, area.0);
    let inside = r.x >= s.x - 0.5 && r.y >= s.y - 0.5 && r.right() <= s.right() + 0.5 && r.bottom() <= s.bottom() + 0.5;
    let same_place = (r.x - s.x).abs() < 0.5 && (r.y - s.y).abs() < 0.5 && (r.w - s.w).abs() < 0.5 && (r.h - s.h).abs() < 0.5;
    let outline = seed.2.is_none() || (same_place && seed.2 == area.2);
    seed.4 == area.4 && (seed.3 == area.3 || seed.1) && (seed.1 || !area.1) && inside && outline
}

/// In a pass no other follows ([`MAX_REWRAP_PASSES`]), a float at `r` with wrap area `area`
/// placed above the top of its paragraph (`top`), beside text laid out before it, that the page
/// did not start with room for: it moves into the room the page started with for it (`seed`), or
/// else down to its paragraph's top, so no text laid out before it runs under it. `wrap` gives
/// the wrap area of a rectangle.
fn settle_early(
    r: Rect,
    area: Option<WrapArea>,
    seed: Option<&WrapArea>,
    top: f32,
    wrap: impl Fn(Rect) -> Option<WrapArea>,
) -> (Rect, Option<WrapArea>) {
    let Some(a) = area.as_ref().filter(|a| a.0.y < top - 0.5) else { return (r, area) };
    if seed.is_some_and(|s| covers(s, a)) {
        return (r, area);
    }
    if let Some(s) = seed {
        let moved = Rect { x: r.x + (s.0.x - a.0.x), y: r.y + (s.0.y - a.0.y), ..r };
        let moved_area = wrap(moved);
        if moved_area.as_ref().is_some_and(|m| covers(s, m)) {
            return (moved, moved_area);
        }
    }
    let down = Rect { y: r.y + (top - a.0.y), ..r };
    (down, wrap(down))
}

/// The early floats a pass found, `found`, are all among those the pass started with, `early`,
/// each inside the area the page started with for it.
pub(crate) fn early_covered(found: &EarlyFloats, early: &EarlyFloats) -> bool {
    found.iter().all(|(page, v)| {
        v.iter().all(|(b, k, area)| early.get(page).is_some_and(|w| w.iter().any(|(eb, ek, seed)| (eb, ek) == (b, k) && covers(seed, area))))
    })
}

/// Adds the early floats a pass found to those pages start with (#2048). Nothing is taken away:
/// a float whose wrap area pushes its own anchor onto the next page has no settled place (back
/// on the first page, its anchor fits there again), so the first page keeps the room it was
/// given and the float goes on the next, rather than the passes swapping between the two. A
/// float found somewhere else on the same page keeps text clear of both places (one plain
/// rectangle around them), so the areas only grow.
pub(crate) fn merge_early(early: &mut EarlyFloats, found: EarlyFloats) {
    for (page, v) in found {
        let have = early.entry(page).or_default();
        for (b, k, area) in v {
            match have.iter_mut().find(|(eb, ek, _)| (*eb, *ek) == (b, k)) {
                Some((_, _, seed)) if !covers(seed, &area) => {
                    let beside = seed.3 == area.3 && !seed.1 && !area.1;
                    *seed = (seed.0.union(&area.0), !beside, None, area.3, area.4);
                }
                Some(_) => {}
                None => have.push((b, k, area)),
            }
        }
    }
}

/// The area text keeps clear of around a floating object at `r` (its rectangle grown by the
/// distances from text; Tight and Through: around `outline`, unit coordinates across `r`, when
/// it has one), and whether text may only go above and below it. `None` when text flows over
/// or under the object.
pub(crate) fn wrap_area(r: Rect, float: &Float, outline: Option<&[(f32, f32)]>) -> Option<WrapArea> {
    if matches!(float.wrap, Wrap::Inline | Wrap::BehindText | Wrap::InFrontOfText) {
        return None;
    }
    if matches!(float.wrap, Wrap::Tight | Wrap::Through)
        && let Some((area, c)) = outline.and_then(|o| contour::contour_area(r, float, o))
    {
        return Some((area, false, Some(Arc::new(c)), float.wrap_text, false));
    }
    let d = |v: f32| if v.is_finite() { v.clamp(0.0, 1584.0) } else { 0.0 };
    let (side, top, bottom) = (d(float.dist), d(float.dist_top), d(float.dist_bottom));
    // A rotated object keeps text clear of its rotated bounds.
    let r = float.spin().bounds(r);
    Some((Rect::new(r.x - side, r.y - top, r.w + side * 2.0, r.h + top + bottom), float.wrap == Wrap::TopAndBottom, None, float.wrap_text, false))
}

/// Wrap areas relative to a paragraph whose column starts at `x0` and whose top is `y0`.
pub(crate) fn rel_exclusions(excl: &[WrapArea], x0: f32, y0: f32) -> Vec<para::Exclusion> {
    excl.iter()
        .map(|(r, tb, c, side, table)| para::Exclusion {
            top: r.y - y0,
            bottom: r.bottom() - y0,
            left: r.x - x0,
            right: r.right() - x0,
            top_bottom: *tb,
            contour: c.clone(),
            side: *side,
            table: *table,
        })
        .filter(|e| e.bottom > 0.0)
        .collect()
}

/// The items drawing floating object `o` at `rect` (a group: its members), turned by its own
/// spin inside `outer` (its group's, about `outer`'s centre point).
fn float_items(o: &InlineObject, rect: Rect, outer: (Spin, Point), story: StoryRef, path: &[u32], off: usize) -> Vec<Placed> {
    let own = o.frame().map(|(_, _, f)| f.spin()).unwrap_or_default();
    let (ospin, oc) = outer;
    let rect = carried(rect, ospin, oc);
    let spin = own.within(ospin);
    match o {
        InlineObject::Image { media, crop, .. } => {
            vec![Placed::Image { rect, media: media.clone(), crop: *crop, story, path: Path(path.to_vec()), off, spin }]
        }
        InlineObject::Shape { kind, fill, stroke, stroke_width, effects, freeform, extra, .. } => {
            let object = wordcraft_doc::Pos { story, path: Path(path.to_vec()), off };
            vec![Placed::Shape {
                rect,
                kind: *kind,
                fill: fill.clone(),
                stroke: *stroke,
                stroke_width: *stroke_width,
                effects: *effects,
                freeform: freeform.clone(),
                spin,
                extra: extra.clone(),
                object: Some(object),
                alt: shape_figure_alt(o).unwrap_or_default().to_string(),
            }]
        }
        InlineObject::Graphic { graphic, .. } => {
            vec![Placed::Graphic { rect, graphic: graphic.clone(), story, path: Path(path.to_vec()), off, spin }]
        }
        InlineObject::Group { .. } => {
            let centre = Point::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            match o.canvas_style() {
                // A Drawing Canvas: its background and outline, then its members clipped to its
                // frame, all turned together: the members sit in the unturned frame and the clip
                // carries the turn (drawn as the turn around the clip).
                Some(c) => {
                    let members = group_members(o, rect)
                        .into_iter()
                        .flat_map(|(r, m)| float_items(m, r, (Spin::default(), centre), story, path, off))
                        .collect();
                    vec![
                        Placed::Shape {
                            rect,
                            kind: wordcraft_doc::para::ShapeKind::Rectangle,
                            fill: c.fill.clone(),
                            stroke: c.stroke,
                            stroke_width: c.stroke_width,
                            effects: Default::default(),
                            freeform: None,
                            spin,
                            extra: Default::default(),
                            object: Some(wordcraft_doc::Pos { story, path: Path(path.to_vec()), off }),
                            alt: String::new(),
                        },
                        Placed::Clip { rect, spin, items: members },
                    ]
                }
                None => group_members(o, rect).into_iter().flat_map(|(r, c)| float_items(c, r, (spin, centre), story, path, off)).collect(),
            }
        }
        _ => Vec::new(),
    }
}

/// The description a shape is tagged with as a figure in tagged PDF: its own, when it has one,
/// isn't marked decorative and shows no text. A shape without one is an artifact screen readers
/// skip, and a text box's text is read as text, so its description isn't used.
pub fn shape_figure_alt(o: &InlineObject) -> Option<&str> {
    match o {
        InlineObject::Shape { story: None, float, .. } if !float.alt.decorative && !float.alt.text.trim().is_empty() => Some(&float.alt.text),
        _ => None,
    }
}

/// A group's members, each with its rectangle when the group is at `rect`.
/// `rect` moved (not turned) to where `spin` about `centre` takes its centre: a group member's
/// frame inside its turned group.
fn carried(rect: Rect, spin: Spin, centre: Point) -> Rect {
    let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let (nx, ny) = spin.apply(centre.x, centre.y, cx, cy);
    Rect::new(rect.x + nx - cx, rect.y + ny - cy, rect.w, rect.h)
}

fn group_members(o: &InlineObject, rect: Rect) -> Vec<(Rect, &InlineObject)> {
    // Hidden members aren't placed (nor their text boxes laid out).
    o.group_rects(rect.x, rect.y, rect.w, rect.h)
        .into_iter()
        .filter(|(_, c)| !c.frame().is_some_and(|(_, _, f)| f.hidden))
        .map(|([x, y, w, h], c)| (Rect::new(x, y, w, h), c))
        .collect()
}

/// Position the floating objects anchored in `p` for a paragraph whose text starts at `y0` in
/// the current column, add their wrap areas to the page (replacing any from an earlier attempt,
/// from `excl_mark` on) and lay the paragraph out around them, with its list `label`. `p` is
/// body block `block`. A float the page started with ([`EarlyFloats`]) keeps its place among the
/// page's wrap areas (its area as placed now). Without `place` (a later piece of the paragraph
/// broken again for another column) the floats stay where its first piece placed and drew them:
/// they aren't placed again in the new column, and their record for the page they are drawn on
/// ([`EarlyFloats`]) stands. Floats anchored on a later page ([`late_floats`]), and those in
/// `force`, aren't placed here: the paragraph is laid out without them, and they're returned.
pub(crate) fn anchor_floats(
    ctx: &mut Ctx,
    pb: &mut PageBuilder,
    anchor: FloatAnchor,
    force: &[usize],
) -> (HashMap<usize, Rect>, Arc<ParaLayout>, Vec<para::Exclusion>, Vec<usize>) {
    let FloatAnchor { p, block, y0, place, .. } = anchor;
    ctx.fields.page = pb.number;
    // Each layout of it here counts its automatic equation numbers from the same place.
    let eq0 = ctx.eq_count;
    let (rects, pl, rel) = anchor_floats_but(ctx, pb, anchor, force);
    if !place || pb.web || p.objects.is_empty() {
        return (rects, pl, rel, force.to_vec());
    }
    // Not those left out, nor those the page started with (#2048 settled them here).
    let page = pb.pages.len().saturating_sub(1);
    let seeded: Vec<usize> =
        pb.early.get(&page).map(|v| v.iter().take(pb.seeded).filter(|(b, ..)| *b == block).map(|(_, k, _)| *k).collect()).unwrap_or_default();
    let (bottom, more, col_top) = (pb.bottom, pb.cols.len().saturating_sub(pb.col + 1), pb.top);
    let fit = |pl: &ParaLayout| fitting(pl, 0, y0 + pl.lines.first().map_or(0.0, |l| l.top.max(0.0)), bottom, (more, col_top));
    let left_out = left_out_objects(ctx.doc, p, None, ctx.opts.show_hidden, ctx.hide_deleted);
    let mut late: Vec<usize> =
        late_floats(p, &pl, fit(&pl)).into_iter().chain(force.iter().copied()).filter(|oi| !left_out(*oi) && !seeded.contains(oi)).collect();
    late.sort_unstable();
    late.dedup();
    if late.as_slice() == force {
        return (rects, pl, rel, late);
    }
    ctx.eq_count = eq0;
    let (rects, pl, rel) = anchor_floats_but(ctx, pb, anchor, &late);
    // Without them in the way more of the paragraph may fit, and some of their anchors with it.
    let still = late_floats(p, &pl, fit(&pl));
    let fewer: Vec<usize> = late.iter().copied().filter(|oi| force.binary_search(oi).is_ok() || still.binary_search(oi).is_ok()).collect();
    if fewer.len() == late.len() {
        return (rects, pl, rel, late);
    }
    ctx.eq_count = eq0;
    let (rects, pl, rel) = anchor_floats_but(ctx, pb, anchor, &fewer);
    (rects, pl, rel, fewer)
}

/// The floating objects of `p` (laid out as `pl`) whose anchor line ([`anchor_lines`]) is at or
/// after `fit`, in order: Word draws them on the page their anchor line is on (#4250).
pub(crate) fn late_floats(p: &Paragraph, pl: &ParaLayout, fit: usize) -> Vec<usize> {
    let lines = anchor_lines(p, pl);
    let late =
        |(oi, o): (usize, &InlineObject)| (floating(o).is_some() && lines.get(oi).copied().flatten().is_some_and(|li| li >= fit)).then_some(oi);
    p.objects.iter().enumerate().filter_map(late).collect()
}

/// The end of the lines of `pl` from `l0` that fit on the page: line `l0`'s top at `top` in the
/// column, whose bottom is at `bottom`, then `more` columns from `col_top` (at least one line; a
/// page break ends them, a column break its column).
pub(crate) fn fitting(pl: &ParaLayout, l0: usize, top: f32, bottom: f32, (mut more, col_top): (usize, f32)) -> usize {
    let Some(first) = pl.lines.get(l0) else { return l0 };
    // A line's place in its column: `shift` plus its top.
    let (mut shift, mut start, mut fit) = (top - first.top, l0, l0 + 1);
    for (li, l) in pl.lines.iter().enumerate().skip(l0) {
        if li > start && shift + l.top + l.height > bottom + 0.01 {
            if more == 0 {
                break;
            }
            (more, start, shift) = (more - 1, li, col_top - l.top);
        }
        fit = li + 1;
        match l.end {
            LineEnd::PageBreak => break,
            LineEnd::ColumnBreak if more == 0 => break,
            LineEnd::ColumnBreak => {
                more -= 1;
                start = li + 1;
                shift = pl.lines.get(start).map_or(shift, |n| col_top - n.top);
            }
            _ => {}
        }
    }
    fit
}

/// Place late float `oi` of body paragraph `p` ([`LateFloats`]) in the current column,
/// positioned from `top`, where the paragraph continues; its wrap area joins the page's.
pub(crate) fn place_late(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, oi: usize, top: f32, late: &mut LateFloats) {
    let Some(o) = p.objects.get(oi) else { return };
    let Some((w, h, float)) = floating(o) else { return };
    let frame = PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror };
    let rect = float_rect(Some(frame), (pb.col_x(), pb.col_w()), top, w, h, float);
    let outline = ctx.wrap_outline(o);
    let area = wrap_area(rect, float, outline.as_deref().map(Vec::as_slice)).map(|a| {
        let bounds = a.0;
        pb.excl.push(a);
        bounds
    });
    late.placed.insert(oi, Some(LatePlace { rect, top, spot: (pb.pages.len(), pb.col), drawn: false, area }));
}

/// The line of `pl` each object of `p` goes with (by object index; None if it isn't laid out):
/// the one holding its anchor, or for an anchor right after a page or column break (only other
/// anchors before it on its line), the break's: Word keeps it on the page before (a cover page's
/// title box, #1632).
pub(crate) fn anchor_lines(p: &Paragraph, pl: &ParaLayout) -> Vec<Option<usize>> {
    let mut out = vec![None; p.objects.len()];
    for (li, l) in pl.lines.iter().enumerate() {
        // Still only anchors from the start of a line after a break.
        let mut lead = li.checked_sub(1).and_then(|k| pl.lines.get(k)).is_some_and(|l| matches!(l.end, LineEnd::PageBreak | LineEnd::ColumnBreak));
        for c in pl.clusters.get(l.c0..l.c1).unwrap_or_default() {
            let para::ClKind::Object(oi) = c.kind else {
                lead = false;
                continue;
            };
            if let Some(slot) = out.get_mut(oi).filter(|s| s.is_none()) {
                *slot = Some(if lead { li - 1 } else { li });
            }
        }
    }
    out
}

/// [`anchor_floats`], leaving out the floats in `late` (in order).
fn anchor_floats_but(
    ctx: &mut Ctx,
    pb: &mut PageBuilder,
    anchor: FloatAnchor,
    late: &[usize],
) -> (HashMap<usize, Rect>, Arc<ParaLayout>, Vec<para::Exclusion>) {
    let FloatAnchor { p, block, y0, before, excl_mark, label, place } = anchor;
    let (col_x, width) = (pb.col_x(), pb.col_w());
    let mut rects = HashMap::new();
    if !pb.web && place {
        // (A mark from a page before may be short of the areas this page started with.)
        pb.excl.truncate(excl_mark.max(pb.seeded));
        let page = pb.pages.len().saturating_sub(1);
        let early = pb.early.get(&page);
        // The paragraph's top, and whether text was placed before it on the page.
        let top = y0 - before;
        let text_before = top > pb.top + 0.5 || pb.col > 0;
        let frame = PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror };
        // An object left out of the layout (hidden, or deleted in the final text) is not placed, so
        // it takes no room either.
        let left_out = left_out_objects(ctx.doc, p, None, ctx.opts.show_hidden, ctx.hide_deleted);
        for (oi, o) in p.objects.iter().enumerate() {
            let Some((w, h, float)) = floating(o) else { continue };
            if left_out(oi) {
                continue;
            }
            if late.binary_search(&oi).is_ok() {
                ctx.floats_above.remove(&(block, oi));
                continue;
            }
            // Positions relative to the paragraph start above its space before (Word for Windows).
            let mut r = float_rect(Some(frame), (col_x, width), y0 - before, w, h, float);
            let outline = ctx.wrap_outline(o);
            let outline = outline.as_deref().map(Vec::as_slice);
            let mut area = wrap_area(r, float, outline);
            if pb.settle && text_before {
                let seed = early.and_then(|v| v.iter().find(|(b, k, _)| (*b, *k) == (block, oi))).map(|(_, _, a)| a);
                (r, area) = settle_early(r, area, seed, top, |r| wrap_area(r, float, outline));
            }
            rects.insert(oi, r);
            // Above the paragraph, beside text laid out before it: the page is laid out again
            // with it known from the start (#2048).
            let above = area.as_ref().filter(|a| text_before && a.0.y < top - 0.5).map(|a| (page, a.clone()));
            ctx.floats_above.insert((block, oi), above);
            if let Some(area) = area {
                match early.and_then(|v| v.iter().position(|(b, k, _)| (*b, *k) == (block, oi))).filter(|k| *k < pb.seeded) {
                    Some(k) => {
                        if let Some(slot) = pb.excl.get_mut(k) {
                            *slot = area;
                        }
                    }
                    None => pb.excl.push(area),
                }
            }
        }
    }
    let rel = rel_exclusions(&body_wraps(ctx, pb), col_x, y0);
    let pl = ctx.para_labelled(p, width, None, &rel, label.clone());
    (rects, pl, rel)
}

pub(crate) fn body_wraps(ctx: &mut Ctx, pb: &PageBuilder) -> Vec<WrapArea> {
    let mut areas = footer_table_wraps(ctx, pb);
    areas.extend_from_slice(&pb.excl);
    areas
}

/// Where a paragraph's floating objects are positioned from, when they aren't placed already:
/// the box on its page, the column (left, width) and the paragraph's top. Also where offsets
/// relative to the column and paragraph start, for dragging.
#[derive(Clone, Copy)]
pub(crate) struct ObjFrame<'a> {
    pub(crate) page: Option<PageFrame<'a>>,
    pub(crate) col: (f32, f32),
    pub(crate) para_y: f32,
    /// A table's character formatting under the paragraph's (see `left_out_objects`).
    pub(crate) table_chr: Option<&'a CharProps>,
    /// Floats placed on the page from a header or footer cell (by object index, in page
    /// coordinates): their items go to [`Ctx::pin`].
    pub(crate) pinned: Option<&'a HashMap<usize, Rect>>,
    /// A body paragraph's floats that go on a later page or column than its first piece.
    pub(crate) late: Option<&'a LateFloats>,
}

/// Where a late float ([`LateFloats`]) was placed: its rect, the top it's positioned from, the
/// page and column, `(pages, col)`, whether it's drawn, and the bounds of its wrap area.
#[derive(Clone, Copy)]
pub(crate) struct LatePlace {
    pub(crate) rect: Rect,
    pub(crate) top: f32,
    pub(crate) spot: (usize, usize),
    pub(crate) drawn: bool,
    pub(crate) area: Option<Rect>,
}

/// A body paragraph's floating objects anchored on a later page than its first line
/// ([`late_floats`]): Word draws them there, positioned from where the paragraph continues
/// (#4250). Each is placed when the paragraph gets there, and drawn with that piece.
pub(crate) struct LateFloats {
    /// By object index, where it was placed, once it is.
    pub(crate) placed: HashMap<usize, Option<LatePlace>>,
    /// The page and column of the piece being placed.
    pub(crate) piece: (usize, usize),
    /// [`anchor_lines`], and the paragraph layout they're for.
    pub(crate) lines: Option<(Arc<ParaLayout>, Vec<Option<usize>>)>,
    /// Those a piece let go ([`LateFloats::release`]), and those it keeps all the same
    /// ([`LateFloats::keep_back`]): (piece, object index).
    pub(crate) held: Vec<((usize, usize), usize)>,
    pub(crate) kept: Vec<((usize, usize), usize)>,
}

impl LateFloats {
    pub(crate) fn new(late: &[usize]) -> Self {
        LateFloats { placed: late.iter().map(|oi| (*oi, None)).collect(), piece: (0, 0), lines: None, held: Vec::new(), kept: Vec::new() }
    }

    /// [`anchor_lines`] for `pl`.
    pub(crate) fn refresh(&mut self, p: &Paragraph, pl: &Arc<ParaLayout>) {
        if !self.lines.as_ref().is_some_and(|(of, _)| Arc::ptr_eq(of, pl)) {
            self.lines = Some((pl.clone(), anchor_lines(p, pl)));
        }
    }

    /// The line object `oi` goes with ([`anchor_lines`]).
    pub(crate) fn anchor(&self, oi: usize) -> Option<usize> {
        self.lines.as_ref().and_then(|(_, v)| v.get(oi).copied().flatten())
    }

    /// Those placed with this piece whose anchor line is past `end` after all (notes, widow
    /// control or a row kept together took lines [`fitting`] counted) wait for the next piece,
    /// their wrap areas out of `excl`. Whether there were any.
    pub(crate) fn release(&mut self, p: &Paragraph, pl: &Arc<ParaLayout>, end: usize, excl: &mut Vec<WrapArea>) -> bool {
        self.refresh(p, pl);
        let piece = self.piece;
        let out: Vec<usize> = self
            .placed
            .iter()
            .filter(|(oi, at)| at.is_some_and(|a| a.spot == piece && !a.drawn) && !self.kept.contains(&(piece, **oi)))
            .filter(|(oi, _)| self.anchor(**oi).is_some_and(|li| li >= end))
            .map(|(oi, _)| *oi)
            .collect();
        for oi in &out {
            if let Some(Some(a)) = self.placed.insert(*oi, None)
                && let Some(k) = a.area.and_then(|r| excl.iter().rposition(|w| w.0 == r))
            {
                excl.remove(k);
            }
            self.held.push((piece, *oi));
        }
        !out.is_empty()
    }

    /// Those this piece let go whose anchor line came back into `lines` without them (their own
    /// wrap pushed it on): kept with this piece after all, as before. Whether there were any.
    pub(crate) fn keep_back(&mut self, p: &Paragraph, pl: &Arc<ParaLayout>, lines: std::ops::Range<usize>) -> bool {
        self.refresh(p, pl);
        let piece = self.piece;
        let back: Vec<_> =
            self.held.iter().filter(|(pc, oi)| *pc == piece && self.anchor(*oi).is_some_and(|li| lines.contains(&li))).copied().collect();
        self.held.retain(|h| !back.contains(h));
        self.kept.extend(&back);
        !back.is_empty()
    }

    /// Those to place with this piece: not placed yet (nor let go by it), or placed for a column
    /// the paragraph then didn't go into, with their anchor line in `lines`, in order.
    pub(crate) fn due(&mut self, p: &Paragraph, pl: &Arc<ParaLayout>, lines: std::ops::Range<usize>) -> Vec<usize> {
        self.refresh(p, pl);
        let mut due: Vec<usize> = self
            .placed
            .iter()
            .filter(|(_, at)| at.is_none_or(|a| a.spot != self.piece && !a.drawn))
            .filter(|(oi, _)| !self.held.contains(&(self.piece, **oi)) && self.anchor(**oi).is_some_and(|li| lines.contains(&li)))
            .map(|(oi, _)| *oi)
            .collect();
        due.sort_unstable();
        due
    }

    /// Late float `oi` if it's drawn with this piece.
    pub(crate) fn here(&self, oi: usize) -> Option<LatePlace> {
        self.placed.get(&oi).copied().flatten().filter(|a| a.spot == self.piece && !a.drawn)
    }

    /// The lines holding the anchors of those drawn with this piece (an anchor right after a
    /// break is on the line after its [`anchor_lines`] one).
    pub(crate) fn lines_here(&self) -> Vec<usize> {
        self.placed.keys().filter(|oi| self.here(**oi).is_some()).filter_map(|oi| self.anchor(*oi)).flat_map(|li| [li, li + 1]).collect()
    }

    /// Those drawn with this piece are drawn.
    pub(crate) fn drew(&mut self) {
        let piece = self.piece;
        for a in self.placed.values_mut().flatten().filter(|a| a.spot == piece) {
            a.drawn = true;
        }
    }
}

/// What [`anchor_floats`] anchors: paragraph `p` (body block `block`) starting at `y0` with
/// `before` of space above it, its list `label`, the wrap areas to keep from `excl_mark` on, and
/// whether to `place` its floats.
#[derive(Clone, Copy)]
pub(crate) struct FloatAnchor<'a> {
    pub p: &'a Paragraph,
    pub block: usize,
    pub y0: f32,
    pub before: f32,
    pub excl_mark: usize,
    pub label: &'a Option<(String, Level)>,
    pub place: bool,
}

/// What [`place_objects`] places: the objects of paragraph `p` (layout `pl`, story `story` at
/// `path`) in `lines`, whose lines start at `origin`, in frame `at`; `depth` is the nesting.
#[derive(Clone, Copy)]
pub(crate) struct ObjectPlacement<'a> {
    pub story: StoryRef,
    pub path: &'a [u32],
    pub p: &'a Paragraph,
    pub pl: &'a ParaLayout,
    pub lines: (usize, usize),
    pub origin: (f32, f32),
    pub at: &'a ObjFrame<'a>,
    pub floats: &'a HashMap<usize, Rect>,
    pub depth: usize,
}

/// The pictures, shapes and text boxes in lines `l0..l1` of paragraph `p` (story `story`, at
/// `path`), whose lines start at (x, y): floating ones drawn, every one's area (hit testing,
/// selection) and text boxes' text. Floating rects come from `floats` (by object index) or are
/// worked out from `at`. Returns what goes behind the text, and what goes in front.
pub(crate) fn place_objects(ctx: &mut Ctx, req: &ObjectPlacement) -> (Vec<Placed>, Vec<Placed>) {
    let ObjectPlacement { story, path, p, pl, lines: (l0, l1), origin: (x, y), at, floats, depth } = *req;
    // Each object's items (behind the text, in front), with its stacking order: drawn lowest
    // first once all are placed, as Word stacks floating objects by `relativeHeight`, not by
    // their order in the paragraph. Inline objects and objects without one keep their order.
    let mut stack: Vec<(u32, Vec<Placed>, Vec<Placed>)> = Vec::new();
    // The highest order so far: an object without one (inserted here) stacks above the objects
    // before it, as the writer gives it the next order up on save.
    let mut z_top = 0u32;
    let Some(fl) = pl.lines.get(l0) else { return (Vec::new(), Vec::new()) };
    // Objects left out of the layout (hidden, or deleted in the final text) aren't placed.
    let left_out = left_out_objects(ctx.doc, p, at.table_chr, ctx.opts.show_hidden, ctx.hide_deleted);
    // The paragraph's first piece draws its floating objects, whichever of its lines holds the
    // anchor, except those anchored on a later page (`at.late`): the piece there draws them
    // (#4250). Inline objects go with their own line.
    let first = l0 == 0;
    let late = at.late.filter(|l| !l.placed.is_empty());
    let late_lines = late.map(LateFloats::lines_here).unwrap_or_default();
    for li in 0..pl.lines.len() {
        let in_piece = (l0..l1).contains(&li);
        if !in_piece && !first && !late_lines.contains(&li) {
            continue;
        }
        let Some(line) = pl.lines.get(li) else { continue };
        for k in line.c0..line.c1 {
            let Some(c) = pl.clusters.get(k) else { continue };
            let para::ClKind::Object(oi) = c.kind else { continue };
            if left_out(oi) {
                continue;
            }
            let Some(obj) = p.objects.get(oi) else { continue };
            let Some((w, h, float)) = obj.frame() else { continue };
            let floating = float.wrap != Wrap::Inline;
            let is_late = floating && late.is_some_and(|l| l.placed.contains_key(&oi));
            let late_here = late.and_then(|l| l.here(oi)).filter(|_| floating);
            let skip = if is_late {
                late_here.is_none()
            } else if floating {
                !first
            } else {
                !in_piece
            };
            if skip {
                continue;
            }
            let pinned = at.pinned.and_then(|m| m.get(&oi)).copied().filter(|_| floating);
            let rect = if let Some(r) = pinned {
                r
            } else if let Some(l) = late_here {
                l.rect
            } else if floating {
                floats.get(&oi).copied().unwrap_or_else(|| float_rect(at.page, at.col, at.para_y, w, h, float))
            } else {
                let cx = x + line.cl_left(k).unwrap_or(0.0);
                display::inline_rect(Some(obj), cx, y + (line.baseline - fl.top), c.adv, c.obj_h)
            };
            let (mut behind, mut front) = (Vec::new(), Vec::new());
            let layer = if float.wrap == Wrap::BehindText { &mut behind } else { &mut front };
            // Floating ones are drawn here; inline ones with their line.
            if floating {
                layer.extend(float_items(obj, rect, (Spin::default(), Point::default()), story, path, c.start));
            }
            let text_box = match obj {
                InlineObject::Shape { story: Some(id), .. } if ctx.doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox) => {
                    Some(*id)
                }
                _ => None,
            };
            // A box linked from another shows (and edits) that box's story.
            let shown = text_box.map(|id| textbox::shown_story(ctx, id));
            front.push(Placed::Object {
                rect,
                spin: float.spin(),
                story,
                path: Path(path.to_vec()),
                off: c.start,
                text_box: shown,
                wrap: float.wrap,
                origin: Point::new(at.col.0, late_here.map_or(at.para_y, |l| l.top)),
            });
            // Its text (a group: its text boxes', each where the group's turn carries it; the
            // text itself stays upright), unless the box budget says no (a box inside itself,
            // too deep, too many).
            let centre = Point::new(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            let boxes: Vec<(u32, Rect)> = match obj {
                InlineObject::Group { .. } => group_members(obj, rect)
                    .into_iter()
                    .filter_map(|(r, c)| {
                        c.text_box()
                            .filter(|id| ctx.doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox))
                            .map(|id| (id, carried(r, float.spin(), centre)))
                    })
                    .collect(),
                _ => text_box.map(|id| (id, rect)).into_iter().collect(),
            };
            for (id, rect) in boxes {
                let items = textbox::box_items(ctx, id, rect, depth);
                let layer = if float.wrap == Wrap::BehindText { &mut behind } else { &mut front };
                layer.extend(items);
            }
            // Placed on the page from a header or footer cell: drawn with the header, once.
            if pinned.is_some() {
                if let Some(pin) = ctx.pin.as_mut()
                    && pin.seen.insert((path.to_vec(), c.start))
                {
                    pin.behind.extend(behind);
                    pin.front.extend(front);
                }
                continue;
            }
            let z = if floating && float.z > 0 {
                z_top = z_top.max(float.z);
                float.z
            } else {
                z_top
            };
            stack.push((z, behind, front));
        }
    }
    // Stable: equal orders (none given) keep the paragraph's order.
    stack.sort_by_key(|(z, _, _)| *z);
    let (mut behind, mut front) = (Vec::new(), Vec::new());
    for (_, b, f) in stack {
        behind.extend(b);
        front.extend(f);
    }
    (behind, front)
}

/// The wrap areas for a paragraph laid out against `rel` (relative to its top) whose lines from the
/// one at `t` (relative to its top) on go into the column starting at `x0`, that line at `y` on
/// the page, which has the wrap areas `excl` (#1374). The areas the lines before were broken
/// around end at `t`; the column's start there (one reaching above `t` with an outline keeps its
/// whole outline, so it may change a line before, and the paragraph is then not broken again).
/// Areas well clear of the column (outside `-width..2 * width`, room for negative indents) can't
/// touch its lines and are left out.
pub(crate) fn continued_exclusions(rel: &[para::Exclusion], excl: &[WrapArea], x0: f32, y: f32, t: f32, width: f32) -> Vec<para::Exclusion> {
    let before = rel.iter().filter(|e| e.top < t).map(|e| para::Exclusion { bottom: e.bottom.min(t), ..e.clone() });
    let here = rel_exclusions(excl, x0, y - t)
        .into_iter()
        .filter(|e| e.bottom > t && e.right > -width && e.left < 2.0 * width)
        .map(|e| if e.top < t && e.contour.is_none() { para::Exclusion { top: t, ..e } } else { e });
    before.chain(here).collect()
}

/// Is an object floating (not laid out inline)?
pub fn is_floating(o: &InlineObject) -> bool {
    o.is_floating()
}
