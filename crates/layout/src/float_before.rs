//! Text before a floating table or object wraps around it too.
//!
//! Word lays out all the text on the page of a floating table (`w:tblpPr`) or a floating picture,
//! shape or text box around it, not only the text after it: a paragraph before one placed over it
//! moves below it (or beside a narrow one), like the paragraphs after it. Measured in Word for Mac
//! with tables positioned on the page, on the margin and (by a negative offset) on the text, and
//! shapes positioned on the page and on their paragraph, on the first page and on later ones, with
//! empty and non-empty paragraphs before them; see `docs/layout-parity.md`.
//!
//! The body is laid out in document order, so the text before a floating table or object is on
//! the page when it gets there. The pagination state before each block on the current page is
//! kept; when a floating table or object lands on lines placed before it, the blocks from the
//! first one it lands on are laid out again with its wrap area (and those of the other floating
//! tables and objects placed among them) on the page from the start, and they all stay where they
//! were placed. When that pushes the text before, or the anchor paragraph with its objects, off
//! the page, the text is laid out as it was, and a paragraph anchoring objects starts the next page
//! with them (as in Word). An anchor paragraph that only runs onto the next page keeps its objects
//! and its first lines on theirs, as in Word (widow control counted).
//!
//! Not laid out again: text in an earlier section on the page (the walk restarts at a section),
//! sections whose columns are balanced before a continuous break, pages aligned other than at the
//! top (leaving a page moves its text, which a second layout can't undo; a page is aligned as the
//! section it starts in says), blocks that started more than a page back, text more than
//! [`MAX_KEPT`] blocks before a float, and floats past the first [`MAX_REWRAPS`] on a page. A
//! floating object whose text before is not laid out again here, or that still has text of an
//! earlier section on the page (or of a block that started more than a page before) under it, is
//! left to the layout pass made again with it known from the page's start (#2048).
use super::*;

/// Most blocks on one page whose state is kept, and most blocks one float lays out again before
/// it (far above a real page): a float that would lay out more leaves the text before it as it
/// is. It bounds the memory a page of tiny paragraphs takes, and with [`MAX_REWRAPS`] the work:
/// each float lays out at most this many blocks before it and itself again, twice when the text
/// then no longer fits on the page.
const MAX_KEPT: usize = 512;
/// Most floats on one page the text before is laid out again for (each may lay the page's text
/// out again, so this bounds the work a page of many floats takes); later ones on the page are
/// placed as before.
const MAX_REWRAPS: usize = 16;

#[cfg(test)]
thread_local! {
    /// The most floats one page laid text out again for, and the most blocks one float laid out
    /// again before it, in the layouts on this thread: tests check the bounds hold.
    pub(crate) static MOST: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// A floating table, or a paragraph's floating objects that text wraps around, placed on a page.
#[derive(Clone, Debug)]
pub(crate) struct PlacedFloat {
    /// The body block: the table or the paragraph.
    pub block: usize,
    /// The page's index.
    pub page: usize,
    /// Where it stands: the table (index 0), or each of the paragraph's floating objects by index.
    pub rects: Vec<(usize, Rect)>,
    /// The areas the text keeps clear of around it.
    pub areas: Vec<WrapArea>,
    /// A table placed from the text (any vertical anchor but the page or margin, or no vertical
    /// offset, as `place_floating_table` places it): a line before it whose bottom touches its top
    /// stays where it is (Word), as it doesn't for a table placed on the page or margin
    /// ([`reaches`]).
    pub from_text: bool,
}

impl PlacedFloat {
    /// The floating objects of paragraph `p` (block `block`) placed at `rects` on page `page`, when
    /// any of them has text wrap around it.
    pub(crate) fn of_para(ctx: &mut Ctx, p: &Paragraph, block: usize, page: usize, rects: &HashMap<usize, Rect>) -> Option<PlacedFloat> {
        let mut rects: Vec<(usize, Rect)> = rects.iter().map(|(oi, r)| (*oi, *r)).collect();
        rects.sort_by_key(|(oi, _)| *oi);
        let mut areas = Vec::new();
        for (oi, r) in &rects {
            let Some(o) = p.objects.get(*oi) else { continue };
            let Some((_, _, float)) = floating(o) else { continue };
            let outline = ctx.wrap_outline(o);
            areas.extend(wrap_area(*r, float, outline.as_deref().map(Vec::as_slice)));
        }
        (!areas.is_empty()).then_some(PlacedFloat { block, page, rects, areas, from_text: false })
    }
}

/// Whether the body holds a floating table or a floating object text wraps around: only then is
/// the state before each block kept.
pub(crate) fn has_floats(doc: &Document) -> bool {
    doc.body.iter().any(|b| match &**b {
        Block::Table(t) => t.props.float.is_some(),
        Block::Para(p) => p.objects.iter().any(|o| floating(o).is_some_and(|(_, _, f)| !matches!(f.wrap, Wrap::BehindText | Wrap::InFrontOfText))),
    })
}

/// The blocks placed on the current page, each with the pagination state before it (also the
/// block that ran onto the page from the one before, or moved to it), and the floating tables and
/// objects placed on it.
#[derive(Default)]
pub(crate) struct PageBlocks<'a> {
    /// `pages.len()` when the blocks were kept.
    pages: usize,
    kept: Vec<(usize, Snap<'a>)>,
    floats: Vec<PlacedFloat>,
}

/// How many times text on a physical page was laid out again, across the sections on it (the
/// blocks are kept per section, the bound is the page's).
#[derive(Default)]
pub(crate) struct Rewraps {
    /// The page the count is for: the latest one a float was placed on.
    page: Option<usize>,
    done: usize,
}

impl Rewraps {
    /// Whether text before a float on `page` may be laid out again (then it counts).
    fn take(&mut self, page: usize) -> bool {
        if self.page.is_none_or(|p| page > p) {
            self.page = Some(page);
            self.done = 0;
        }
        if self.done >= MAX_REWRAPS {
            return false;
        }
        self.done += 1;
        #[cfg(test)]
        MOST.with(|c| c.set((c.get().0.max(self.done), c.get().1)));
        true
    }
}

impl<'a> PageBlocks<'a> {
    /// Keep the state before `block`. After a page break only the block it happened in stays, when
    /// it started on the page before (one running over more pages is not laid out again).
    pub(crate) fn before(&mut self, pb: &mut PageBuilder<'a>, ctx: &mut Ctx, block: usize) {
        if pb.pages.len() != self.pages {
            self.pages = pb.pages.len();
            let last = self.kept.pop().filter(|(b, s)| b.saturating_add(1) == block && s.pages.saturating_add(1) == pb.pages.len());
            self.kept.clear();
            self.kept.extend(last);
            let page = pb.pages.len().saturating_sub(1);
            self.floats.retain(|f| f.page == page);
        }
        self.keep(pb, ctx, block);
    }

    fn keep(&mut self, pb: &mut PageBuilder<'a>, ctx: &mut Ctx, block: usize) {
        if self.kept.len() < MAX_KEPT {
            self.kept.push((block, Snap::take(pb, ctx)));
        }
    }
}

fn is_table(doc: &Document, block: usize) -> bool {
    matches!(doc.body.get(block).map(|b| &**b), Some(Block::Table(_)))
}

/// Whether a line at `r` runs into wrap area `area`: overlapping it, or, with `contact`, touching
/// the top of a floating table's with its bottom (#3977; measured in Word for tables placed on the
/// page, before or after the line in the document, not for tables placed from the text).
fn reaches(r: &Rect, area: &WrapArea, contact: bool) -> bool {
    let (a, table) = (area.0, area.4 && contact);
    let top = if table { a.y <= r.bottom() } else { a.y < r.bottom() };
    r.x < a.right() && a.x < r.right() && r.y < a.bottom() && top
}

/// The body blocks before `before` with lines on `page` that one of `areas` lands on ([`reaches`],
/// with `contact`), a paragraph's last line with its space after.
fn covered<'p>(ctx: &'p Ctx, page: &'p Page, (areas, contact): (&'p [WrapArea], bool), before: usize) -> impl Iterator<Item = usize> + 'p {
    page.items.iter().filter_map(move |it| {
        let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, x, y, .. } = it else { return None };
        let block = usize::try_from(*path.0.first()?).ok()?;
        if block >= before {
            return None;
        }
        let lines = para.lines.get(*l0..*l1)?;
        // `y` is where the first of them starts.
        let first = lines.first()?.top;
        // Before Word 2013 layout a paragraph's last line keeps its space after clear of
        // floating objects too.
        let after =
            if path.0.len() == 1 && *l1 == para.lines.len() { space_after_clear_of_floats(ctx, block, &para.rp, para.label.is_some()) } else { 0.0 };
        let last = lines.len().saturating_sub(1);
        lines
            .iter()
            .enumerate()
            .any(|(k, l)| {
                let h = l.height + if k == last { after } else { 0.0 };
                let r = Rect::new(x + l.left, y + l.top - first, (l.right - l.left).max(1.0), h);
                areas.iter().any(|area| reaches(&r, area, contact))
            })
            .then_some(block)
    })
}

/// The first block on `page` before `placed` whose lines it lands on, a paragraph's last line
/// with its space after, from `kept_from` on (the first block whose state is kept; the lines of
/// the floating tables placed there don't count).
fn first_covered(ctx: &Ctx, page: &Page, placed: &PlacedFloat, (floats, kept_from): (&[PlacedFloat], usize)) -> Option<usize> {
    covered(ctx, page, (&placed.areas, !placed.from_text), placed.block)
        .filter(|block| *block >= kept_from && !(is_table(ctx.doc, *block) && floats.iter().any(|f| f.block == *block)))
        .min()
}

/// Whether text on `page` before body block `before` runs under `area`, a wrap area of a floating
/// object kept where it was: `before` is the first block this walk lays out again, so that text is
/// an earlier section's on the page (the walk restarts at a section) or a block's that started more
/// than a page back. The lines of floating tables don't count.
pub(crate) fn text_under(ctx: &Ctx, page: &Page, before: usize, area: &WrapArea) -> bool {
    covered(ctx, page, (std::slice::from_ref(area), true), before)
        .any(|b| !matches!(ctx.doc.body.get(b).map(|b| &**b), Some(Block::Table(t)) if t.props.float.is_some()))
}

/// Place body block `block` (a floating table at the place `floats` gives it, a paragraph's
/// floating objects where `pb.fixed_objects` says).
fn place_block(ctx: &mut Ctx, pb: &mut PageBuilder, block: usize, floats: &[PlacedFloat], body_top: f32) -> Option<PlacedFloat> {
    let doc = ctx.doc;
    pb.mark_block();
    match doc.body.get(block).map(|b| &**b) {
        Some(Block::Para(p)) => place_para(ctx, pb, p, block, body_top),
        Some(Block::Table(t)) => {
            let at = floats.iter().find(|f| f.block == block).and_then(|f| f.rects.first()).map(|(_, r)| *r);
            place_table(ctx, pb, t, block, body_top, at)
        }
        None => None,
    }
}

/// Lay out blocks `from..to` again from `snap` (taken before `from`), keeping the state before
/// each, with the wrap areas of `floats` (on page `page`) on that page from the start and those
/// floating tables and objects where they were.
fn replay<'a>(
    ctx: &mut Ctx,
    pb: &mut PageBuilder<'a>,
    blocks: &mut PageBlocks<'a>,
    (snap, from, to): (&Snap<'a>, usize, usize),
    (page, floats): (usize, &[PlacedFloat]),
    body_top: f32,
) {
    snap.restore(pb, ctx);
    pb.walk_from = blocks.kept.first().map_or(from, |(b, _)| *b);
    #[cfg(test)]
    MOST.with(|c| c.set((c.get().0, c.get().1.max(to.saturating_sub(from)))));
    let doc = ctx.doc;
    for f in floats {
        pb.pending_floats.extend(f.areas.iter().map(|a| (page, a.clone())));
        if !is_table(doc, f.block) {
            pb.fixed_objects.extend(f.rects.iter().map(|(oi, r)| (f.block, *oi, *r)));
        }
    }
    pb.arm_pending_floats(pb.pages.len().saturating_sub(1));
    for block in from..to {
        blocks.keep(pb, ctx, block);
        place_block(ctx, pb, block, floats, body_top);
    }
}

/// Floating table or paragraph `placed.block` was just placed, with floating tables or objects
/// (`placed`). If they stand on lines placed before it on their page, lay the blocks from the
/// first of them out again around them (and around the floating tables and objects placed among
/// them) and place it again with them where they were. When that text, or the block itself, no
/// longer ends on their page, the text is laid out as it was instead, and a paragraph starts the
/// next page with its objects. Returns the text boxes laid out in the layouts thrown away.
pub(crate) fn wrap_text_before<'a>(
    ctx: &mut Ctx,
    pb: &mut PageBuilder<'a>,
    (blocks, rewraps): (&mut PageBlocks<'a>, &mut Rewraps),
    placed: PlacedFloat,
    body_top: f32,
) -> usize {
    let (block, page) = (placed.block, placed.page);
    blocks.floats.push(placed);
    let Some(placed) = blocks.floats.last() else { return 0 };
    let Some(from) =
        pb.pages.get(page).and_then(|pg| first_covered(ctx, pg, placed, (&blocks.floats, blocks.kept.first().map_or(usize::MAX, |(b, _)| *b))))
    else {
        return 0;
    };
    let Some(i) = blocks.kept.iter().position(|(b, _)| *b == from) else { return 0 };
    if block.saturating_sub(from) > MAX_KEPT || !rewraps.take(page) {
        return 0;
    }
    let mut tail = blocks.kept.split_off(i);
    let Some((_, snap)) = (!tail.is_empty()).then(|| tail.swap_remove(0)) else { return 0 };
    drop(tail);
    let floats: Vec<PlacedFloat> = blocks.floats.iter().filter(|f| f.page == page && f.block >= from).cloned().collect();
    let thrown = ctx.boxes.total().saturating_sub(snap.boxes);
    replay(ctx, pb, blocks, (&snap, from, block), (page, &floats), body_top);
    // The wrap areas went onto the page, and the text before the block still ends there ...
    let armed = !pb.pending_floats.iter().any(|(p, _)| *p == page);
    if armed && pb.pages.len() == page.saturating_add(1) {
        pb.pending_floats.retain(|(p, _)| *p != page);
        blocks.keep(pb, ctx, block);
        let again = place_block(ctx, pb, block, &floats, body_top);
        pb.fixed_objects.clear();
        // ... and so does the block itself, with its floating objects.
        if again.as_ref().is_some_and(|f| f.page == page) {
            blocks.floats.retain(|f| f.block != block);
            blocks.floats.extend(again);
            return thrown;
        }
    }
    // Word: when the text before, or the paragraph itself, would no longer fit on the page, the
    // text before stays as it was and a paragraph starts the next page, taking its floating
    // objects along (a table stays where it was placed).
    let thrown = ctx.boxes.total().saturating_sub(snap.boxes);
    blocks.kept.truncate(i);
    replay(ctx, pb, blocks, (&snap, from, block), (page, &[]), body_top);
    pb.fixed_objects.clear();
    pb.pending_floats.retain(|(p, _)| *p != page);
    if !is_table(ctx.doc, block) && !pb.at_top() {
        pb.advance(block, body_top);
    }
    blocks.keep(pb, ctx, block);
    let again = place_block(ctx, pb, block, &[], body_top);
    blocks.floats.retain(|f| f.block != block);
    blocks.floats.extend(again);
    thrown
}
