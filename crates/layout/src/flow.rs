use super::boxes::{BoxInput, BoxLayout, Edges, joined, layout_box, push_para};
use super::cache::has_page_number_fields;
use super::ctx::Ctx;
use super::floats::{FloatAnchor, ObjFrame, ObjectPlacement, anchor_floats, body_wraps, continued_exclusions, place_objects};
use super::notes::{DECOR_STORY, NoteBox, para_notes};
use super::page::{PageBuilder, PageFrame, item_bottom};
use super::placed::Placed;
use super::{
    Arc, Block, Counters, Document, HashMap, InlineObject, Level, LineEnd, Page, ParaLayout, Paragraph, Path, Rect, StoryRef, TextDirection,
    ViewMode, outline, para,
};
#[cfg(test)]
use crate::WRAP_RELAYS;

/// A closing mark before a continuous section takes no room after content. An otherwise empty
/// section starting a new page retains its paragraph line.
pub(crate) fn place_para(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, body_top: f32) {
    if !pb.web && ctx.quiet_marks.binary_search(&block).is_ok() {
        let follows_section = block.checked_sub(1).and_then(|i| ctx.doc.body.get(i)).and_then(|b| b.as_para()).is_some_and(|p| p.section.is_some());
        let section = pb.sect_idx;
        let own_page = pb.page().is_some_and(|page| page.section == section);
        // Word keeps the line when a new page's section contains only this mark.
        if p.section.is_some()
            && p.objects.is_empty()
            && (ctx.opts.show_hidden || !ctx.doc.styles.resolve_char(p.props.style.as_deref(), &p.mark).hidden)
            && !ctx.has_label(p)
            && (block == 0 || follows_section)
            && pb.at_top()
            && own_page
        {
            place_para_in_flow(ctx, pb, p, block, body_top);
            return;
        }
        // Its (empty) line stays where the text ended, even at the bottom of a full column.
        let (y, prev, bottom, content_bottom, at) = (pb.y, pb.prev.clone(), pb.bottom, pb.content_bottom, (pb.pages.len(), pb.col));
        let drawn = pb.page().map_or(0, |page| page.items.len());
        pb.bottom = f32::MAX / 4.0;
        place_para_in_flow(ctx, pb, p, block, body_top);
        pb.bottom = bottom;
        if (pb.pages.len(), pb.col) == at {
            (pb.y, pb.prev, pb.content_bottom) = (y, prev, content_bottom);
            if let Some(page) = pb.page() {
                strip_mark(page, drawn, true);
            }
        }
        return;
    }
    if !pb.web && ctx.stacked_marks.binary_search(&block).is_ok() {
        // A run of empty continuous-section marks uses each remaining line, but never starts a
        // new page just for a mark; later text starts there instead.
        let (y, prev, bottom, content_bottom, at) = (pb.y, pb.prev.clone(), pb.bottom, pb.content_bottom, (pb.pages.len(), pb.col));
        let drawn = pb.page().map_or(0, |page| page.items.len());
        pb.bottom = f32::MAX / 4.0;
        place_para_in_flow(ctx, pb, p, block, body_top);
        if pb.pages.len() == at.0 {
            pb.bottom = bottom;
        }
        if (pb.pages.len(), pb.col) == at && pb.y > bottom + 0.01 {
            (pb.y, pb.prev, pb.content_bottom) = (bottom.max(y), prev, content_bottom);
            if p.objects.is_empty()
                && let Some(page) = pb.page()
            {
                strip_mark(page, drawn, false);
            }
        }
        return;
    }
    place_para_in_flow(ctx, pb, p, block, body_top);
}

/// Remove the list label from a mark's lines; optionally keep its other drawing items.
fn strip_mark(page: &mut Page, drawn: usize, keep_others: bool) {
    let tail = page.items.split_off(drawn.min(page.items.len()));
    page.items.extend(tail.into_iter().filter_map(|item| match item {
        Placed::Lines { story, path, para, l0, l1, x, y, turn } if para.label.is_some() => {
            let mut bare = (*para).clone();
            bare.label = None;
            Some(Placed::Lines { story, path, para: Arc::new(bare), l0, l1, x, y, turn })
        }
        Placed::Lines { story, .. } if story == DECOR_STORY && !keep_others => None,
        line @ Placed::Lines { .. } => Some(line),
        other => keep_others.then_some(other),
    }));
}

/// How many times one body paragraph is laid out again for the page numbers of the later pages
/// it runs onto (#1352).
const MAX_PAGE_RELAYS: usize = 64;

/// Run `f`, a layout of a paragraph again, with the automatic equation numbers it counted from
/// (`eq_start`) the first time, and leave the count where it was: laying it out again doesn't
/// count its numbered equations twice.
pub(crate) fn again<T>(ctx: &mut Ctx, eq_start: u32, f: impl FnOnce(&mut Ctx) -> T) -> T {
    let now = std::mem::replace(&mut ctx.eq_count, eq_start);
    let out = f(ctx);
    ctx.eq_count = now;
    out
}

/// How many times one body paragraph's lines are broken again for the wrap areas of the later
/// columns and pages it runs onto (#1374).
const MAX_WRAP_RELAYS: usize = 64;

/// Run `f` with `map` as the field context's page map (`FieldCtx::page_from`), then take it back.
fn with_page_map<T>(ctx: &mut Ctx, map: &mut Vec<(usize, u32)>, f: impl FnOnce(&mut Ctx) -> T) -> T {
    ctx.fields.page_from = std::mem::take(map);
    let out = f(ctx);
    *map = std::mem::take(&mut ctx.fields.page_from);
    out
}

/// The room for a paragraph's top and bottom borders; those of a group joined in one box are
/// drawn by its first and last paragraph.
#[derive(Clone, Copy)]
struct Rooms {
    /// Joined to the paragraph above.
    above: bool,
    top: f32,
    bottom: f32,
}

impl Rooms {
    fn of(ctx: &Ctx, block: usize, pl: &ParaLayout) -> Self {
        let above = joined(ctx.doc, &ctx.doc.body, block, block.checked_sub(1), None);
        let below = joined(ctx.doc, &ctx.doc.body, block, Some(block + 1), None);
        let own = Edges::own(pl, 0, pl.lines.len());
        Rooms { above, top: if above { 0.0 } else { own.top }, bottom: if below { 0.0 } else { own.bottom } }
    }
}

/// The lines of a paragraph that go on one page, and what goes with them.
struct Fit {
    /// The end of the lines (exclusive).
    l1: usize,
    /// How the last line ends, if it forces a break.
    forced: Option<LineEnd>,
    /// A page break followed only by the paragraph mark: the mark's (empty) line stays on this
    /// page with the break, so nothing of the paragraph goes to the next page (Word).
    mark_with_break: bool,
    /// The footnotes referenced on those lines.
    notes: Vec<NoteBox>,
}

/// A body paragraph being placed: its layout and what it was laid out for, which change as it is
/// laid out again for the columns and pages it runs onto.
struct Flow<'a> {
    p: &'a Paragraph,
    block: usize,
    body_top: f32,
    width: f32,
    label: Option<(String, Level)>,
    /// Where it was expected to start; floats anchored here are placed again if it starts elsewhere.
    guess: f32,
    excl_mark: usize,
    /// Where its automatic equation numbers start: each layout of it again counts from here.
    eq_start: u32,
    float_rects: HashMap<usize, Rect>,
    pl: Arc<ParaLayout>,
    rel: Vec<para::Exclusion>,
    /// The page its fields were laid out for, and the width.
    laid_page: u32,
    laid_width: f32,
    /// The anchor paragraph's top (above its space before) the floats were placed from.
    anchor_y: f32,
    /// The pages this paragraph's fields are on after its first (`FieldCtx::page_from`), kept here
    /// and lent to the field context only while this paragraph itself is laid out, so footnotes,
    /// text boxes and headers laid out in between never see it (#1352).
    page_map: Vec<(usize, u32)>,
    /// How many times it was laid out again for a later page (bounded: a paragraph over hundreds
    /// of pages keeps the last number it got), and for the wrap areas of a later column (#1374).
    page_relays: usize,
    wrap_relays: usize,
    /// The first line not placed yet.
    l0: usize,
}

impl<'a> Flow<'a> {
    fn new(ctx: &mut Ctx, pb: &mut PageBuilder, p: &'a Paragraph, (block, body_top): (usize, f32)) -> Self {
        let width = pb.col_w();
        // Floating objects anchored here join the page's wrap areas before the text is laid out; if
        // the paragraph then starts somewhere else (another column or page, or contextual spacing
        // drops its space before), they are placed again there.
        let space_before = ctx.doc.styles.resolve_para(&p.props).space_before;
        let excl_mark = pb.excl.len();
        let guess = pb.y + space_before;
        let label = ctx.next_label(p);
        let eq_start = ctx.eq_count;
        let (float_rects, pl, rel) =
            anchor_floats(ctx, pb, FloatAnchor { p, block, y0: guess, before: space_before, excl_mark, label: &label, place: true });
        Flow {
            p,
            block,
            body_top,
            width,
            label,
            guess,
            excl_mark,
            eq_start,
            float_rects,
            pl,
            rel,
            laid_page: pb.number,
            laid_width: pb.col_w(),
            anchor_y: pb.y,
            page_map: Vec::new(),
            page_relays: 0,
            wrap_relays: 0,
            l0: 0,
        }
    }

    fn n(&self) -> usize {
        self.pl.lines.len()
    }

    /// Place the floats and lay the paragraph out again from `y0`, `before` the space before it.
    fn reanchor(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder, (y0, before): (f32, f32)) {
        let (rects, pl, rel) = again(ctx, self.eq_start, |ctx| {
            anchor_floats(
                ctx,
                pb,
                FloatAnchor { p: self.p, block: self.block, y0, before, excl_mark: self.excl_mark, label: &self.label, place: true },
            )
        });
        (self.float_rects, self.pl, self.rel) = (rects, pl, rel);
        (self.laid_page, self.laid_width) = (pb.number, pb.col_w());
    }

    /// Lay the paragraph out again around the wrap areas `rel`, with its fields on their pages.
    fn relay(&mut self, ctx: &mut Ctx, rel: &[para::Exclusion]) -> Arc<ParaLayout> {
        let (p, width, eq_start, label) = (self.p, self.laid_width, self.eq_start, &self.label);
        with_page_map(ctx, &mut self.page_map, |ctx| again(ctx, eq_start, |ctx| ctx.para_labelled(p, width, None, rel, label.clone())))
    }

    /// The space before the paragraph, after the one between it and the previous paragraph is
    /// settled: same-style contextual spacing removes it, or subtracts the previous paragraph's
    /// after-space from the merged gap (ISO/IEC 29500-1 §17.3.1.9).
    fn gap_before(&self, ctx: &Ctx, pb: &mut PageBuilder, broke_before: bool) -> f32 {
        let mut before = self.pl.rp.space_before;
        let prev_para = pb.prev.clone();
        // At the top of a page, measured in Word for Windows: no space before after a paragraph that
        // ended with a page break, nor (from Word 2013's layout, compatibility mode 15) for a page
        // break before; a section break or, in older modes, a page break before keeps it.
        let after_break_mark = std::mem::take(&mut pb.after_break_mark);
        if pb.at_top() && (after_break_mark || (broke_before && ctx.doc.settings.compat_mode >= 15)) {
            before = 0.0;
        } else if pb.at_top()
            && !ctx.doc.settings.sum_paragraph_spacing
            && let Some((_, _, after, _, _)) = &prev_para
        {
            // After a page break before or a section break the space between the paragraphs is
            // still counted as one: the previous paragraph's space after (left on its page) comes
            // off the space before here (Word: 24 pt before after 10 pt after leaves 14 pt).
            before = (before - after.max(0.0)).max(0.0);
        }
        if let Some(prev) = pb.prev.take()
            && !pb.at_top()
        {
            before = self.merge_gap(ctx, pb, before, prev);
        }
        // Automatic spacing: none at the top of the document.
        if self.pl.rp.before_auto && self.block == 0 {
            before = 0.0;
        }
        before
    }

    /// `before`, the space before the paragraph, with the previous paragraph (`prev`) taken into account.
    fn merge_gap(&self, ctx: &Ctx, pb: &mut PageBuilder, mut before: f32, prev: (String, bool, f32, bool, bool)) -> f32 {
        let (style, ctxl, mut after, after_auto, listed) = prev;
        let rp = &self.pl.rp;
        // Contextual spacing suppresses each paragraph's own side of the gap.
        if style == rp.style {
            if ctxl {
                pb.y -= after;
                before = (before - after).max(0.0);
                after = 0.0;
            }
            if rp.contextual_spacing {
                before = 0.0;
            }
        }
        // Automatic spacing: none between two list items (as in an HTML list; measured in Word:
        // any two items, of one list or two, at any level).
        if listed && self.label.is_some() {
            if after_auto {
                pb.y -= after;
                after = 0.0;
            }
            if rp.before_auto {
                before = 0.0;
            }
        }
        // The space between two paragraphs is the larger of the first's space after and the
        // second's space before, not their sum.
        if before > 0.0 && after > 0.0 && !ctx.doc.settings.sum_paragraph_spacing {
            before = (before - after).max(0.0);
        }
        before
    }

    /// Keep lines together: if it doesn't fit but would on an empty column, move it. Keep with
    /// next alone moves a paragraph only when it would fit here by itself (Word for Windows): one
    /// that doesn't breaks as usual, its last lines going over with the next paragraph. Returns
    /// the space before applied (`before`, or what is left of it at the top of the next column).
    fn keep_together(&self, ctx: &mut Ctx, pb: &mut PageBuilder, rooms: Rooms, before: f32) -> f32 {
        let rp = &self.pl.rp;
        let height = self.pl.height + rooms.top + rooms.bottom;
        if !(rp.keep_lines || (rp.keep_next && pb.y + height <= pb.bottom + 0.01)) {
            return before;
        }
        let need = height + if rp.keep_next { next_first_line(ctx, self.block, self.width, 0, &mut ctx.counters.clone()) } else { 0.0 };
        // (Against the whole column: balanced columns grow to hold it.)
        // With the 0.01 pt of slack the other fit checks take.
        if pb.y + need > pb.bottom + 0.01 && !pb.fresh() && need <= pb.next_room(self.body_top) {
            pb.advance(self.block, self.body_top);
            pb.y += rp.space_before.min(0.0);
            return rp.space_before.min(0.0);
        }
        before
    }

    /// The lines going into a new column (or page) were broken around the wrap areas where the
    /// paragraph started: break them again around this column's (the lines before stay as they
    /// were, and so do the paragraph's own floating objects), when they differ (#1374).
    fn relay_wrap(&mut self, ctx: &mut Ctx, pb: &PageBuilder) {
        if self.l0 == 0 || pb.web || self.wrap_relays >= MAX_WRAP_RELAYS {
            return;
        }
        let Some((resume, t)) = self.pl.lines.get(self.l0).map(|l| (l.start, l.top)) else { return };
        let here = continued_exclusions(&self.rel, &body_wraps(ctx, pb), pb.col_x(), pb.y, t, self.laid_width);
        if here == self.rel {
            return;
        }
        self.wrap_relays += 1;
        ctx.fields.page = self.laid_page;
        #[cfg(test)]
        WRAP_RELAYS.with(|c| c.set(c.get() + 1));
        let relaid = self.relay(ctx, &here);
        // Only if the rest still starts at the same line, at the same height (an outline
        // reaching above it could move the lines before down, and the column's areas with them).
        if let Some(k) = relaid.lines.iter().position(|l| l.start == resume)
            && relaid.lines.get(k).is_some_and(|l| (l.top - t).abs() < 0.01)
        {
            (self.pl, self.rel, self.l0) = (relaid, here, k);
        }
    }

    /// Page fields in the lines going onto a new page show that page's number: lay the
    /// paragraph out again with the fields from there on on this page (the lines before stay as
    /// they were, so the line the rest starts with is still there), breaking the rest again for
    /// the new results' widths (#1352). Only a paragraph with page fields pays for this.
    fn relay_pages(&mut self, ctx: &mut Ctx, pb: &PageBuilder) {
        if self.l0 == 0 || !self.pl.has_page_fields || pb.web || self.page_relays >= MAX_PAGE_RELAYS {
            return;
        }
        let Some(resume) = self.pl.lines.get(self.l0).map(|l| l.start) else { return };
        if self.page_map.last().map_or(self.laid_page, |e| e.1) == pb.number || !has_page_number_fields(self.p) {
            return;
        }
        self.page_relays += 1;
        self.page_map.push((resume, pb.number));
        ctx.fields.page = self.laid_page;
        let rel = self.rel.clone();
        let relaid = self.relay(ctx, &rel);
        match relaid.lines.iter().position(|l| l.start == resume) {
            Some(k) => (self.pl, self.l0) = (relaid, k),
            None => {
                self.page_map.pop();
            }
        }
    }

    /// The lines from `l0` that go on this page (`lead`: the room above the first line, from a
    /// float pushing it down or its border), with the footnotes on them.
    fn fit_lines(&self, ctx: &mut Ctx, pb: &PageBuilder, notes: &[(usize, u32)], lead: f32) -> Fit {
        let (pl, n, l0) = (&self.pl, self.n(), self.l0);
        let first_top = pl.lines.get(l0).map_or(0.0, |f| f.top);
        let mut fit = Fit { l1: l0, forced: None, mark_with_break: false, notes: Vec::new() };
        // Worked out only for a last line on a page with notes.
        let mut after_on_page: Option<f32> = None;
        while let Some(l) = pl.lines.get(fit.l1) {
            let line_notes = notes_on_line(ctx, pb, l, notes, &fit.notes);
            let limit = notes_limit(pb, &fit.notes, &line_notes);
            let free = pb.notes.is_empty() && fit.notes.is_empty() && line_notes.is_empty();
            let bottom = pb.y + lead + (l.top + fit_height(pb, l, &pl.rp, free) - first_top);
            // On a page with footnotes the paragraph's last line keeps its space after clear of
            // the separator (Word): if the space doesn't fit, the line goes to the next page (and,
            // with widow control, one more with it). Without notes the space after may run past
            // the bottom margin as usual.
            let tail = if !free && fit.l1 + 1 == n {
                *after_on_page.get_or_insert_with(|| space_after_kept(ctx, self.block, &pl.rp, self.label.is_some()))
            } else {
                0.0
            };
            let over = bottom + tail > limit + 0.01;
            let split = bottom > limit + 0.01 && notes_split(pb, (&fit.notes, &line_notes), bottom);
            if over && fit.l1 > l0 && !split {
                break;
            }
            if over && fit.l1 == l0 && !pb.fresh() && !split && !self.stays_at_bottom(pb, l) {
                break;
            }
            fit.notes.extend(line_notes);
            fit.l1 += 1;
            if self.ends_page(pb, l, &mut fit) || split {
                break;
            }
        }
        fit
    }

    /// Whether the line may overflow the page: one holding nothing but a page or column break stays
    /// at the bottom of the page even when it doesn't fit (it only ends the page, and moving it
    /// would leave a blank page behind it, as in Word), and so does an empty paragraph closing a
    /// section before a new page (measured in Word: it stays on a full page instead of starting
    /// one of its own).
    fn stays_at_bottom(&self, pb: &PageBuilder, l: &para::Line) -> bool {
        let p = self.p;
        let only_break = matches!(l.end, LineEnd::PageBreak | LineEnd::ColumnBreak)
            && p.text
                .get(l.start..l.stop)
                .is_some_and(|t| t.chars().all(|c| matches!(c, wordcraft_doc::para::PAGE_BREAK | wordcraft_doc::para::COLUMN_BREAK)));
        only_break || (pb.next_starts_page && p.section.is_some() && p.text.is_empty())
    }

    /// Whether the line (just taken, `fit.l1` is after it) forces a break, noted in `fit`.
    fn ends_page(&self, pb: &PageBuilder, l: &para::Line, fit: &mut Fit) -> bool {
        if !matches!(l.end, LineEnd::PageBreak | LineEnd::ColumnBreak) || pb.web {
            return false;
        }
        fit.forced = Some(l.end);
        let mark_follows = self.pl.lines.get(fit.l1).is_some_and(|m| only_markers(self.p, m.start..m.stop));
        if l.end == LineEnd::PageBreak && fit.l1 + 1 == self.n() && mark_follows {
            fit.l1 += 1;
            fit.mark_with_break = true;
        }
        true
    }

    /// The end of the lines on this page after widow/orphan control, and keeping the rows beside
    /// floating objects together.
    fn settled_end(&self, pb: &PageBuilder, fit: &Fit) -> usize {
        let (n, l0, mut l1) = (self.n(), self.l0, fit.l1);
        // Widow/orphan control (2-line minimum at either end).
        if fit.forced.is_none() && l1 < n && self.pl.rp.widow_control && n >= 2 {
            if l1 - l0 == 1 && l0 == 0 && !pb.fresh() {
                l1 = l0; // orphan: move the first line to the next page
            } else if n - l1 == 1 && (l1 - l0 >= 3 || (l0 > 0 && l1 - l0 >= 2)) {
                l1 -= 1; // widow: take one more line along
            } else if n - l1 == 1 && l0 == 0 && l1 - l0 == 2 && !pb.fresh() {
                // Neither split of a three-line paragraph is allowed (two and a widow, or an
                // orphan and two): it moves whole, as in Word.
                l1 = l0;
            }
        }
        // A row (text both sides of a floating object) stays together.
        while l1 > l0 && l1 < n && self.pl.lines.get(l1).is_some_and(|l| l.beside) {
            l1 -= 1;
        }
        l1
    }

    /// Nothing fits on this page: move to the next column or page, and place the floats again
    /// there if this is the paragraph's start.
    fn move_on(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder) {
        pb.advance(self.block, self.body_top);
        if self.l0 == 0 {
            let y0 = pb.y;
            self.reanchor(ctx, pb, (y0, 0.0));
            self.anchor_y = y0;
        }
    }

    /// Put the lines `l0..fit.l1` on the page, with their notes, objects and line numbers.
    fn emit(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder, rooms: Rooms, (lead, fit, notes): (f32, Fit, &[(usize, u32)])) {
        let (l0, l1, n) = (self.l0, fit.l1, self.n());
        add_notes(pb, fit.notes, notes, self.pl.lines.get(l1.saturating_sub(1)).map_or(0, |l| l.c1));
        // Mirror indents: on an even (left-hand) page the inside indent is on the right, so the
        // paragraph's lines (broken for the same width) move over by the indents' difference.
        let x = pb.col_x() + mirror_shift(&self.pl.rp, pb.number, pb.web);
        let y = pb.y + lead;
        // Notes that no longer fit below the text (a long one, or the rest of one continued from
        // the page before) get what's left under it and continue on the next page.
        if !pb.notes.is_empty()
            && !pb.web
            && let Some(placed_bottom) = item_bottom(y, &self.pl, l0, l1)
            && placed_bottom > pb.bottom + 0.01
        {
            pb.bottom = pb.body_floor().max(placed_bottom).min(pb.orig_bottom);
        }
        let mut items = Vec::new();
        let mut edges = Edges { top: if l0 == 0 { rooms.top } else { 0.0 }, bottom: if l1 >= n { rooms.bottom } else { 0.0 }, up: 0.0 };
        if l0 == 0
            && rooms.above
            && let Some((page, col, end)) = pb.para_end
            && page == pb.pages.len()
            && col == pb.col
        {
            edges.up = (y - end).max(0.0);
        }
        let end = push_para(&mut items, (StoryRef::Body, &[self.block as u32]), &self.pl, (l0, l1), (x, y), self.width, edges);
        pb.para_end = Some((pb.pages.len(), pb.col, end));
        // Pictures, shapes and text boxes in these lines (web view: floats placed by this piece).
        let at = ObjFrame {
            page: Some(PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror }),
            col: (x, self.width),
            para_y: if pb.web { y } else { self.anchor_y },
            table_chr: None,
            pinned: None,
        };
        let (behind, front) = place_objects(
            ctx,
            &ObjectPlacement {
                story: StoryRef::Body,
                path: &[self.block as u32],
                p: self.p,
                pl: &self.pl,
                lines: (l0, l1),
                origin: (x, y),
                at: &at,
                floats: &self.float_rects,
                depth: 0,
            },
        );
        items.extend(front);
        items.extend(line_numbers(ctx, pb, &self.pl, (l0, l1), y));
        add_items(pb, behind, items);
        if let (Some(f), Some(l)) = (self.pl.lines.get(l0), self.pl.lines.get(l1 - 1)) {
            pb.y = y + (l.top + l.height - f.top);
            pb.content_bottom = pb.content_bottom.max(pb.y);
        }
        self.l0 = l1;
    }

    /// Start the next page or column if the lines just placed end one, or are not the last.
    fn after_lines(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder, forced: Option<LineEnd>, mark_with_break: bool) {
        let (block, body_top) = (self.block, self.body_top);
        // A paragraph that starts with a page break starts on the new page, its space before
        // there (Word).
        let starts_with_break = self.l0 == 1
            && self.pl.lines.first().is_some_and(|l| {
                self.p.text.get(l.start..l.stop).is_some_and(|t| !t.is_empty() && t.chars().all(|c| c == wordcraft_doc::para::PAGE_BREAK))
            });
        match forced {
            Some(LineEnd::PageBreak) if mark_with_break => {
                pb.new_page(block, body_top);
                pb.after_break_mark = true;
            }
            Some(LineEnd::PageBreak) if starts_with_break && !pb.web => {
                pb.new_page(block, body_top);
                pb.y += self.pl.rp.space_before.max(0.0);
            }
            Some(LineEnd::PageBreak) => pb.new_page(block, body_top),
            Some(LineEnd::ColumnBreak) => {
                pb.col_break_page = Some(pb.pages.len().saturating_sub(1));
                pb.advance(block, body_top);
                self.rewidth_after_column_break(ctx, pb);
            }
            _ if self.l0 < self.n() => pb.advance(block, body_top),
            _ => {}
        }
    }

    /// Unequal columns: the text after a column break is broken for its own column's width (Word),
    /// not the width of the column it started in.
    fn rewidth_after_column_break(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder) {
        if (pb.col_w() - self.width).abs() <= 0.5 {
            return;
        }
        let Some(resume) = self.pl.lines.get(self.l0).map(|l| l.start) else { return };
        let y0 = pb.y;
        // The floats were placed and drawn with the first piece: they stay there.
        let (p, block, excl_mark, eq_start, label) = (self.p, self.block, self.excl_mark, self.eq_start, &self.label);
        let (rects, relaid, relaid_rel) = with_page_map(ctx, &mut self.page_map, |ctx| {
            again(ctx, eq_start, |ctx| anchor_floats(ctx, pb, FloatAnchor { p, block, y0, before: 0.0, excl_mark, label, place: false }))
        });
        if let Some(k) = relaid.lines.iter().position(|l| l.start == resume) {
            (self.float_rects, self.pl, self.rel) = (rects, relaid, relaid_rel);
            (self.laid_page, self.laid_width) = (pb.number, pb.col_w());
            self.anchor_y = y0;
            self.l0 = k;
        }
    }

    /// Leave the space after the paragraph, for the next one to merge with.
    fn finish(&self, pb: &mut PageBuilder, rooms: Rooms) {
        // A mark that stayed with its page break keeps its space after on that page too, and its
        // bottom border, drawn there: the next page starts with the next paragraph (Word,
        // compatibility modes 12 to 15).
        let rp = &self.pl.rp;
        let (bottom_room, space_after) = if pb.after_break_mark { (0.0, 0.0) } else { (rooms.bottom, rp.space_after) };
        pb.y += bottom_room + space_after;
        pb.prev = Some((rp.style.clone(), rp.contextual_spacing, space_after, rp.after_auto, self.label.is_some()));
    }
}

/// The footnotes referenced on line `l` that aren't on the page yet, laid out.
fn notes_on_line(ctx: &mut Ctx, pb: &PageBuilder, l: &para::Line, notes: &[(usize, u32)], new_notes: &[NoteBox]) -> Vec<NoteBox> {
    let mut out = Vec::new();
    if pb.web {
        return out;
    }
    for (ci, id) in notes {
        if *ci >= l.c0
            && *ci < l.c1
            && !pb.notes.iter().chain(new_notes).any(|x| x.id == *id)
            && let Some(part) = ctx.doc.parts.get(id).filter(|p| p.kind == wordcraft_doc::PartKind::Footnote)
        {
            let blocks = part.blocks.clone();
            let BoxLayout { items, height: h, .. } = layout_box(ctx, BoxInput::other(StoryRef::Part(*id), &blocks, pb.sect.text_width(), 0, None));
            out.push(NoteBox { id: *id, items, h, cont: false });
        }
    }
    out
}

/// The bottom of the room for text on a page with the notes of the lines before (`new_notes`)
/// and those of this line (`line_notes`) at its foot.
fn notes_limit(pb: &PageBuilder, new_notes: &[NoteBox], line_notes: &[NoteBox]) -> f32 {
    let height = |notes: &[NoteBox]| notes.iter().map(|n| n.h).sum::<f32>();
    let extra = height(line_notes) + if pb.notes.is_empty() && new_notes.is_empty() && !line_notes.is_empty() { pb.seps.sep_h(false) } else { 0.0 };
    pb.bottom - height(new_notes) - if pb.notes.is_empty() && !new_notes.is_empty() { pb.seps.sep_h(false) } else { 0.0 } - extra
}

/// How much of the line has to fit. Multiple spacing above single puts its extra leading below
/// the text (see `baseline_offset`), and Word lets that leading of a page's last line run into the
/// bottom margin: only the text has to fit (observed: a double-spaced 12pt line stays on the page
/// while its text ends above the margin, about half a line lower than when the whole line must
/// fit). Only the bottom margin takes that leading: a footer taller than the margin, or
/// footnotes (`free` is false), keep the whole line clear of them.
fn fit_height(pb: &PageBuilder, l: &para::Line, rp: &wordcraft_doc::resolve::ResolvedPara, free: bool) -> f32 {
    let at_margin = pb.bottom >= pb.sect.page_h - pb.sect.margin_bottom - 0.01;
    match rp.line_spacing {
        // `baseline` is from the paragraph's top; the text ends a descent below it.
        wordcraft_doc::props::LineSpacing::Multiple(m) if m > 1.0 && at_margin && free => (l.baseline - l.top + l.text_desc).clamp(0.0, l.height),
        _ => l.height,
    }
}

/// Notes too long for the rest of the page continue on the next one: the line still fits if the
/// notes before its last one, and that one's first line, fit below it. The notes then take the
/// rest of the page.
fn notes_split(pb: &PageBuilder, (new_notes, line_notes): (&[NoteBox], &[NoteBox]), bottom: f32) -> bool {
    line_notes.split_last().is_some_and(|(last, others)| {
        let before: f32 = pb.notes.iter().chain(new_notes).chain(others).map(|n| n.h).sum();
        let sep = pb.seps.sep_h(pb.notes.first().is_some_and(|n| n.cont));
        pb.body_floor().max(bottom) + sep + before + last.first_piece() + pb.seps.notice_h() <= pb.orig_bottom + 0.01
    })
}

/// Keep the notes of the lines that are placed (those referenced before `placed_end`) on the page.
fn add_notes(pb: &mut PageBuilder, mut new_notes: Vec<NoteBox>, notes: &[(usize, u32)], placed_end: usize) {
    new_notes.retain(|n| notes.iter().any(|(ci, nid)| *nid == n.id && *ci < placed_end));
    let before = pb.notes_h();
    pb.notes.extend(new_notes);
    if pb.balance.is_some() {
        // Balanced columns end where the balance puts them unless the notes reach higher.
        pb.cap_columns();
    } else {
        // Relative, so a bottom moved down for notes continuing on the next page stays put.
        pb.bottom -= pb.notes_h() - before;
    }
}

/// Line numbers in the left margin for lines `l0..l1` of `pl` laid out from `y`.
fn line_numbers(ctx: &mut Ctx, pb: &mut PageBuilder, pl: &ParaLayout, (l0, l1): (usize, usize), y: f32) -> Vec<Placed> {
    let mut items = Vec::new();
    if let Some(ln) = pb.sect.line_numbers.clone().filter(|_| !pb.web && pl.rp.style != "Header" && !pl.rp.suppress_line_numbers)
        && let Some(first) = pl.lines.get(l0)
    {
        for li in l0..l1 {
            let Some(line) = pl.lines.get(li).filter(|l| !l.beside) else { continue };
            pb.line_no = pb.line_no.saturating_add(1);
            let n = pb.line_no.saturating_add(ln.start.saturating_sub(1));
            if ln.count_by > 1 && !n.is_multiple_of(ln.count_by) {
                continue;
            }
            let num = ctx.number_para(n);
            let w = num.lines.first().and_then(|l| l.xs.last().copied()).unwrap_or(10.0);
            let dist = if ln.distance > 0.0 { ln.distance } else { 18.0 };
            let ly = y + (line.top - first.top) + (line.baseline - line.top) - num.lines.first().map(|l| l.baseline).unwrap_or(10.0);
            items.push(Placed::Lines {
                story: DECOR_STORY,
                path: Path(vec![]),
                para: num.clone(),
                l0: 0,
                l1: 1,
                x: pb.sect.margin_left + pb.sect.gutter - dist - w,
                y: ly,
                turn: TextDirection::Horizontal,
            });
        }
    }
    items
}

/// Add a piece's items to the last page: those `behind` the text at the front, the rest at the end.
fn add_items(pb: &mut PageBuilder, behind: Vec<Placed>, items: Vec<Placed>) {
    let n_behind = behind.len();
    if let Some(pg) = pb.pages.last_mut() {
        for (i, it) in behind.into_iter().enumerate() {
            pg.items.insert(i.min(pg.items.len()), it);
        }
        pg.items.extend(items);
    }
    if let Some(b) = pb.behind.last_mut() {
        *b += n_behind;
    }
}

fn place_para_in_flow(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, body_top: f32) {
    let mut f = Flow::new(ctx, pb, p, (block, body_top));
    let spot = |pb: &PageBuilder| (pb.pages.len(), pb.col);
    let placed_at = spot(pb);
    let broke_before = f.pl.rp.page_break_before && !pb.at_top() && !pb.web;
    if broke_before {
        pb.new_page(block, body_top);
    }
    let before = f.gap_before(ctx, pb, broke_before);
    pb.y += before;
    let rooms = Rooms::of(ctx, block, &f.pl);
    let applied_before = f.keep_together(ctx, pb, rooms, before);
    // A different top only matters when there is something to wrap around.
    if spot(pb) != placed_at || ((pb.y - f.guess).abs() > 0.01 && !pb.excl.is_empty()) {
        let y0 = pb.y;
        f.reanchor(ctx, pb, (y0, applied_before));
        f.anchor_y = y0 - applied_before;
    }
    while f.l0 < f.n() {
        f.relay_wrap(ctx, pb);
        f.relay_pages(ctx, pb);
        // What is laid out for these lines from here (their footnotes) is on this page.
        ctx.fields.page = pb.number;
        // A paragraph's first line may start below its top (pushed down by a floating object).
        let Some(first) = f.pl.lines.get(f.l0) else { break };
        let lead = if f.l0 == 0 { first.top.max(0.0) + rooms.top } else { 0.0 };
        let notes = para_notes(ctx.doc, p, &f.pl);
        let mut fit = f.fit_lines(ctx, pb, &notes, lead);
        fit.l1 = f.settled_end(pb, &fit);
        if fit.l1 == f.l0 {
            if !pb.fresh() {
                f.move_on(ctx, pb);
                continue;
            }
            fit.l1 = f.l0 + 1; // can't fit even one line on an empty page: overflow
        }
        let (forced, mark_with_break) = (fit.forced, fit.mark_with_break);
        f.emit(ctx, pb, rooms, (lead, fit, &notes));
        f.after_lines(ctx, pb, forced, mark_with_break);
    }
    f.finish(pb, rooms);
}

/// Whether `range` of `p`'s text holds nothing but zero-width markers (bookmarks, field and
/// control boundaries…), or nothing at all: a line of them takes no room after a page break.
fn only_markers(p: &Paragraph, range: std::ops::Range<usize>) -> bool {
    let Some(text) = p.text.get(range.clone()) else { return false };
    text.char_indices().all(|(i, c)| c == wordcraft_doc::para::OBJ && p.object_at(range.start + i).is_some_and(InlineObject::is_marker))
}

/// The outline of the body for an Outline view pass.
pub(crate) struct OutlinePass {
    view: outline::OutlineView,
    levels: Vec<u8>,
    pub(crate) hidden: Vec<bool>,
}

impl OutlinePass {
    pub(crate) fn new(doc: &Document, view: outline::OutlineView) -> Self {
        let levels = outline::levels(doc);
        let hidden = outline::hidden(&levels, &view);
        OutlinePass { view, levels, hidden }
    }
}

/// A body paragraph in Outline view (one long sheet: no page or column breaks, no floats
/// wrapping): indented by its level behind its symbol; hidden when collapsed or deeper than Show
/// Level; body text cut to its first line with Show First Line Only.
pub(crate) fn place_outline_para(ctx: &mut Ctx, pb: &mut PageBuilder, p: &Paragraph, block: usize, o: &OutlinePass) {
    // List numbers count the hidden paragraphs too.
    let label = ctx.next_label(p);
    if o.hidden.get(block).copied().unwrap_or(false) {
        return;
    }
    let level = o.levels.get(block).copied().unwrap_or(outline::BODY);
    let indent = outline::indent(&o.levels, block);
    let x = pb.col_x() + indent + outline::SYMBOL_W;
    let width = (pb.col_w() - indent - outline::SYMBOL_W).max(72.0);
    let q = outline::outline_para(ctx.doc, p, o.view.plain);
    ctx.fields.page = pb.number;
    let pl = ctx.para_labelled(&q, width, None, &[], label);
    pb.y += pl.rp.space_before.max(0.0);
    let n = pl.lines.len();
    let l1 = if o.view.first_line_only && level >= outline::BODY { n.min(1) } else { n };
    // Its borders take their room above and below its lines.
    let edges = Edges::own(&pl, 0, l1);
    let y = pb.y + pl.lines.first().map(|f| f.top.max(0.0)).unwrap_or(0.0) + edges.top;
    let bottom_room = edges.bottom;
    let mut items = Vec::new();
    push_para(&mut items, (StoryRef::Body, &[block as u32]), &pl, (0, l1), (x, y), width, edges);
    let at = ObjFrame {
        page: Some(PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror: ctx.mirror }),
        col: (x, width),
        para_y: y,
        table_chr: None,
        pinned: None,
    };
    let (behind, front) = place_objects(
        ctx,
        &ObjectPlacement {
            story: StoryRef::Body,
            path: &[block as u32],
            p: &q,
            pl: &pl,
            lines: (0, l1),
            origin: (x, y),
            at: &at,
            floats: &HashMap::new(),
            depth: 0,
        },
    );
    items.extend(front);
    let children = level < outline::BODY && outline::has_children(&o.levels, block);
    // Collapsed, or with something under it hidden by Show Level: the wavy line says so.
    let collapsed = children && o.hidden.get(block + 1).copied().unwrap_or(false);
    items.extend(outline::symbol_items(&pl, x, y, level, children, collapsed));
    if let Some(pg) = pb.page() {
        for (i, it) in behind.into_iter().enumerate() {
            pg.items.insert(i.min(pg.items.len()), it);
        }
        pg.items.extend(items);
    }
    if let (Some(f), Some(l)) = (pl.lines.first(), l1.checked_sub(1).and_then(|k| pl.lines.get(k))) {
        pb.y = y + (l.top + l.height - f.top);
    }
    pb.y += bottom_room + pl.rp.space_after.max(0.0);
}

/// How far a paragraph with mirrored indents (`w:mirrorIndents`) moves right on page `number`:
/// its left and right indents are the inside and outside ones, which swap sides on even pages.
fn mirror_shift(rp: &wordcraft_doc::resolve::ResolvedPara, number: u32, web: bool) -> f32 {
    if !rp.mirror_indents || web || !number.is_multiple_of(2) {
        return 0.0;
    }
    let (left, right) = rp.visual_indents();
    right - left
}

/// The space after body paragraph `block` (resolved as `rp`) that is really left below it: none
/// when contextual spacing or automatic spacing between list items drops it before the next
/// paragraph. Same tests as the spacing between paragraphs in `place_para_in_flow`: contextual
/// spacing removes this paragraph's space after only when this one asks for it, and the next
/// counts as a list item when it gets a label.
fn space_after_kept(ctx: &Ctx, block: usize, rp: &wordcraft_doc::resolve::ResolvedPara, listed: bool) -> f32 {
    let after = rp.space_after.max(0.0);
    if after <= 0.0 {
        return 0.0;
    }
    if let Some(Block::Para(next)) = ctx.doc.body.get(block + 1).map(|b| &**b) {
        let nrp = ctx.doc.styles.resolve_para(&next.props);
        if nrp.style == rp.style && rp.contextual_spacing {
            return 0.0;
        }
        if listed && rp.after_auto && ctx.has_label(next) {
            return 0.0;
        }
    }
    after
}

/// The room the paragraphs after `block` need on the page for keep with next: enough of the
/// next one to start there — its first line, two of its lines with widow/orphan control (it
/// doesn't leave a single line at the bottom of a page; it moves over whole instead), or all
/// of it under keep lines together — or all of it and what it keeps with in turn when it is
/// kept with the next too (a chain moves together in Word). Word carries a kept heading over
/// when the paragraph after it cannot start on the page (#966).
fn next_first_line(ctx: &mut Ctx, block: usize, width: f32, depth: usize, counters: &mut Counters) -> f32 {
    match ctx.doc.body.get(block + 1).map(|b| &**b) {
        Some(Block::Para(p)) => {
            // Keep list indents and label metrics without consuming the document counters
            // (the caller passes a copy). Memoised like paragraph layouts: headings keep with
            // next, so every pass asks for these. The key mustn't count the paragraph's
            // equations: it is laid out again for real.
            let rp = ctx.doc.styles.resolve_para(&p.props);
            let label =
                p.props.numbering.or(rp.numbering).filter(|n| n.num != 0).and_then(|n| counters.next_label(&ctx.doc.numbering, n.num, n.level));
            let kept = (ctx.eq_count, ctx.uses_totals);
            let (key, _) = ctx.para_key(p, width, None, &[], label.as_ref(), &[]);
            (ctx.eq_count, ctx.uses_totals) = kept;
            // Cached per paragraph: (the room it needs to start on the page, whole height with
            // spacing, keep with next, border room above and below). A chain is summed from
            // these, so an edit to a later paragraph can't leave a stale total behind. Whether
            // the borders count depends on the neighbours, so it is decided below.
            let cached = ctx.cache.lookahead.get(&key).copied();
            let (start, whole, keep, own_top, own_bottom) = if let Some(v) = cached {
                v
            } else {
                // Through the cache when the paragraph is laid out later with these same inputs.
                let pl = match ctx.para_ahead(p, width, label.clone()) {
                    Some(pl) => pl,
                    None => {
                        let env = para::ParaEnv {
                            doc: ctx.doc,
                            width,
                            label,
                            fields: &ctx.fields,
                            show_hidden: ctx.opts.show_hidden,
                            field_codes: ctx.opts.field_codes,
                            hide_deleted: ctx.opts.hides_deleted(StoryRef::Body),
                            table: None,
                            proofing: None,
                            typing_word: None,
                            numeral: ctx.opts.numeral,
                            exclusions: &[],
                            eq_number: ctx.eq_count,
                            read_line_spacing: if ctx.opts.view == ViewMode::Read { ctx.opts.read_line_spacing } else { None },
                            grid: ctx.grid,
                            fit_objects: false,
                            fitted: &[],
                        };
                        Arc::new(para::layout_para(p, &env))
                    }
                };
                let own = Edges::own(&pl, 0, pl.lines.len());
                // The room this one needs to start on the page with the kept paragraph before
                // it (#966).
                let to = |k: usize| match (pl.lines.first(), pl.lines.get(k)) {
                    (Some(first), Some(last)) => last.top + last.height - first.top,
                    _ => 0.0,
                };
                let start = pl.rp.space_before
                    + if pl.rp.keep_lines {
                        pl.height
                    } else if pl.rp.widow_control && pl.lines.len() >= 2 {
                        to(1)
                    } else {
                        to(0)
                    };
                let v = (start, pl.rp.space_before + pl.height + pl.rp.space_after, pl.rp.keep_next, own.top, own.bottom);
                if ctx.cache.lookahead.len() >= MAX_LOOKAHEAD {
                    ctx.cache.lookahead.clear();
                }
                ctx.cache.lookahead.insert(key, v);
                v
            };
            // Its border room, unless it joins the paragraph before (or after) in one box.
            let top = if joined(ctx.doc, &ctx.doc.body, block + 1, Some(block), None) { 0.0 } else { own_top };
            if keep && depth < 8 {
                let bottom = if joined(ctx.doc, &ctx.doc.body, block + 1, Some(block + 2), None) { 0.0 } else { own_bottom };
                whole + top + bottom + next_first_line(ctx, block + 1, width, depth + 1, counters)
            } else {
                start + top
            }
        }
        Some(Block::Table(_)) => 24.0,
        None => 0.0,
    }
}
/// Bound on [`LayoutCache::lookahead`] (entries are small; it starts over past this).
const MAX_LOOKAHEAD: usize = 1 << 16;
