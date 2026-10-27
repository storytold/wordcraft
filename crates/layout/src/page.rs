use super::boxes::{BoxContext, layout_box};
use super::ctx::{Ctx, with_pin};
use super::floats::{EarlyFloats, WrapArea};
use super::notes::{NOTE_SEP, NOTE_SEP_W, NoteBox, NoteSeps, v_span};
use super::placed::Placed;
use super::*;

#[derive(Clone)]
pub(crate) struct PageBuilder<'a> {
    pub(crate) pages: Vec<Page>,
    pub(crate) sect: &'a SectionProps,
    pub(crate) sect_idx: usize,
    pub(crate) col: usize,
    pub(crate) cols: Vec<(f32, f32)>,
    pub(crate) y: f32,
    pub(crate) top: f32,
    /// The body's top on this page: `top` moves down below the text when a continuous section
    /// starts mid-page.
    pub(crate) page_top: f32,
    pub(crate) bottom: f32,
    pub(crate) number: u32,
    pub(crate) web: bool,
    /// Footnotes waiting for the bottom of the current page (first the rest of notes continued
    /// from the page before).
    pub(crate) notes: Vec<NoteBox>,
    /// The parts of footnotes that didn't fit on their page, continued on the next one.
    pub(crate) carry: Vec<NoteBox>,
    /// Page bottom before footnotes took space.
    pub(crate) orig_bottom: f32,
    /// The paragraph above: its style, contextual spacing, space after, whether that is automatic
    /// and whether it is a list item.
    pub(crate) prev: Option<(String, bool, f32, bool, bool)>,
    /// The previous paragraph ended with a page break (its mark stayed with the break): the next
    /// paragraph, first on the new page, gets no space before (Word).
    pub(crate) after_break_mark: bool,
    /// The next section starts on a new page: this section's closing mark, when empty, needs no
    /// room (Word leaves it on a full page rather than starting a page for it).
    pub(crate) next_starts_page: bool,
    /// The number the next page takes instead of counting on: a section restarting its
    /// numbering, possibly at 0 (a cover page numbered 0).
    pub(crate) restart_at: Option<u32>,
    /// Wrap areas of floating objects on this page (see [`wrap_area`]).
    pub(crate) excl: Vec<WrapArea>,
    /// Wrap areas of the floating tables on this page, which others may not overlap.
    pub(crate) float_tables: Vec<Rect>,
    /// Line numbering counter.
    pub(crate) line_no: u32,
    /// Index of the first body item of the current page (vertical alignment shifts from here).
    pub(crate) page_items_start: usize,
    /// Where each top-level block after the first starts on the current page: its first item's
    /// index counted after the items behind the text (justified vertical alignment spreads the
    /// blocks apart there).
    pub(crate) block_marks: Vec<usize>,
    /// The vertical alignment of the section the current page started in: a page finished when
    /// the next section starts is aligned as its own section asks, not the next one.
    pub(crate) page_valign: wordcraft_doc::section::PageVAlign,
    /// The document's own footnote separators, laid out for the section's text width.
    pub(crate) seps: NoteSeps,
    /// Where the last paragraph's lines ended: page count, column and y.
    pub(crate) para_end: Option<(usize, usize, f32)>,
    /// Per page: how many items were inserted at the front of its list (behind the text), so a
    /// [`Snap`] can take them out again.
    pub(crate) behind: Vec<usize>,
    /// The bottom of the tallest column on the current page before the current one.
    pub(crate) max_y: f32,
    /// The bottom of the lowest line or table row placed on the current page (without the space
    /// after it).
    pub(crate) content_bottom: f32,
    /// Balancing columns: (page index, column height) — that page's columns end `height` below
    /// the top of the section on it.
    pub(crate) balance: Option<(usize, f32)>,
    /// The page with the last explicit column break.
    pub(crate) col_break_page: Option<usize>,
    /// The section's footnotes go right below its text, not at the page bottom ...
    pub(crate) sect_beneath: bool,
    /// ... and so do the current page's (set as the page starts).
    pub(crate) beneath: bool,
    /// Top of the footer of the pages being started: a footer reaching above the bottom margin
    /// pushes the body up (`f32::MAX`: no footer).
    pub(crate) foot_top: f32,
    /// The wrap areas pages start with ([`EarlyFloats`]) ...
    pub(crate) early: &'a EarlyFloats,
    /// ... and how many of the current page's `excl` they are (its first ones).
    pub(crate) seeded: usize,
    /// No layout pass follows for floats found above their anchors: each is kept clear of the
    /// text laid out before it ([`settle_early`]).
    pub(crate) settle: bool,
}

/// Page borders, measured from the page edge (Word's default), so they surround the header,
/// footer and line numbers. The first items of every printed page.
pub(crate) fn page_border_rules(s: &SectionProps) -> Vec<Placed> {
    let mut v = Vec::new();
    if let Some(b) = &s.page_borders {
        let sp = |e: &Option<Border>| e.map(|x| x.space).unwrap_or(24.0);
        let (x0, x1, y0, y1) = (sp(&b.left), s.page_w - sp(&b.right), sp(&b.top), s.page_h - sp(&b.bottom));
        for (e, a, bb) in [(b.top, (x0, y0), (x1, y0)), (b.bottom, (x0, y1), (x1, y1)), (b.left, (x0, y0), (x0, y1)), (b.right, (x1, y0), (x1, y1))] {
            if let Some(e) = e.filter(Border::is_visible) {
                v.push(Placed::Rule { x0: a.0, y0: a.1, x1: bb.0, y1: bb.1, border: e });
            }
        }
    }
    v
}

impl PageBuilder<'_> {
    /// Space footnotes take at the bottom of the page.
    pub(crate) fn notes_h(&self) -> f32 {
        self.stack_h(&self.notes)
    }
    /// The height of a page's note area holding `notes`, with its separator.
    fn stack_h(&self, notes: &[NoteBox]) -> f32 {
        match notes.first() {
            None => 0.0,
            Some(first) => self.seps.sep_h(first.cont) + notes.iter().map(|n| n.h).sum::<f32>(),
        }
    }
    /// The lowest bottom of the body text and tables on the current page.
    pub(crate) fn body_floor(&self) -> f32 {
        let Some(pg) = self.pages.last() else { return self.top };
        pg.items
            .iter()
            .skip(self.page_items_start)
            .filter(|it| matches!(it, Placed::Lines { story: StoryRef::Body, .. } | Placed::Cell { story: StoryRef::Body, .. }))
            .map(|it| v_span(it).1)
            .fold(self.top, f32::max)
    }
    /// Place the collected footnotes at the bottom of the current page. When they don't all fit
    /// below the body (`bottom`), the note that doesn't is split at a line and its rest, with
    /// the notes after it, continues on the next page.
    pub(crate) fn flush_notes(&mut self) {
        if self.notes.is_empty() {
            return;
        }
        let total = self.notes_h();
        let x = self.sect.margin_left + self.sect.gutter;
        // The note area starts below the text (a page of continued notes alone: at its top).
        let area = self.bottom.max(self.body_floor());
        let fits = total <= self.orig_bottom - area + 0.01;
        let top = match (fits, self.beneath) {
            (false, _) => area,
            // Below the page's last line of text.
            (true, true) => self.body_floor().min(self.orig_bottom - total),
            (true, false) => self.orig_bottom - total,
        };
        let notes = std::mem::take(&mut self.notes);
        // A page that starts with a continued note gets the continuation separator.
        let cont = notes.first().is_some_and(|n| n.cont);
        let mut y = top + self.seps.sep_h(cont);
        let mut placed = Vec::new();
        // Where the continuation notice goes, below a note that continues on the next page.
        let mut notice_y = None;
        let mut rest = notes.into_iter();
        while let Some(n) = rest.next() {
            let room = self.orig_bottom - y;
            if fits || n.h <= room + 0.01 {
                let ny = y;
                y += n.h;
                placed.push((n, ny));
                continue;
            }
            // Split at the last line that fits with the notice below it. The page's first note
            // always leaves something here (its first line, or all of it if it can't be split),
            // so continuing ends.
            let first = placed.is_empty();
            let cuts = n.cuts();
            let room = room - self.seps.notice_h();
            match cuts.iter().rev().find(|&&c| c <= room + 0.01).or(cuts.first().filter(|_| first)).copied() {
                Some(c) => {
                    let (head, tail) = n.split(c);
                    notice_y = Some(y + head.h);
                    placed.push((head, y));
                    self.carry.push(tail);
                }
                None if first => placed.push((n, y)),
                None => self.carry.push(n),
            }
            self.carry.extend(rest.by_ref());
            break;
        }
        if let Some(pg) = self.pages.last_mut() {
            match if cont { &self.seps.cont } else { &self.seps.sep } {
                Some((items, _)) => pg.items.extend(items.iter().cloned().map(|mut it| {
                    it.translate(x, top);
                    it
                })),
                None => {
                    let len = if cont { self.sect.text_width() } else { NOTE_SEP_W };
                    let sy = top + NOTE_SEP / 2.0;
                    pg.items.push(Placed::Rule { x0: x, y0: sy, x1: x + len, y1: sy, border: Border::single(0.5) });
                }
            }
            for (n, ny) in placed {
                for mut it in n.items {
                    it.translate(x, ny);
                    pg.items.push(it);
                }
            }
            if let (Some(ny), Some((items, _))) = (notice_y, &self.seps.notice) {
                pg.items.extend(items.iter().cloned().map(|mut it| {
                    it.translate(x, ny);
                    it
                }));
            }
        }
    }
    /// A top-level block starts here (see `block_marks`).
    pub(crate) fn mark_block(&mut self) {
        if let Some(pg) = self.pages.last() {
            let behind = self.behind.last().copied().unwrap_or(0);
            self.block_marks.push(pg.items.len().saturating_sub(behind));
        }
    }
    /// Justified vertical alignment: share the room left below the page's text out equally
    /// between the gaps of its blocks, so the last line sits on the bottom margin (above the
    /// footnotes). A page with one block, or several columns, stays at the top.
    fn justify_page(&mut self) {
        if self.web || self.cols.len() > 1 {
            return;
        }
        let behind = self.behind.last().copied().unwrap_or(0);
        let first = self.page_items_start;
        // The room is measured below the last paragraph's space after: Word for Mac 16.113.4 keeps
        // it inside the justified height (72 pt after leaves the last line 72 pt above the margin).
        let space = (self.orig_bottom - self.notes_h() - self.y).max(0.0);
        let Some(pg) = self.pages.last_mut() else { return };
        let front = pg.items.len().saturating_sub(behind);
        // Blocks that moved on to the next page whole leave a mark at the end: not a gap here.
        let mut cuts: Vec<usize> = self.block_marks.iter().copied().filter(|&m| m > first && m < front).collect();
        cuts.sort_unstable();
        cuts.dedup();
        if cuts.is_empty() || space <= 0.0 {
            return;
        }
        let gap = space / cuts.len() as f32;
        for (i, it) in pg.items.iter_mut().enumerate().skip(behind.saturating_add(first)) {
            let k = cuts.partition_point(|&c| c <= i.saturating_sub(behind));
            if k > 0 {
                it.translate(0.0, gap * k as f32);
            }
        }
    }
    /// Section vertical alignment: move the page's body down (centre/bottom), or spread its
    /// blocks out (justified).
    pub(crate) fn apply_valign(&mut self) {
        let k = match self.page_valign {
            wordcraft_doc::section::PageVAlign::Top => return,
            wordcraft_doc::section::PageVAlign::Justify => return self.justify_page(),
            wordcraft_doc::section::PageVAlign::Center => 0.5,
            wordcraft_doc::section::PageVAlign::Bottom => 1.0,
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
    /// Takes back the page a page break has just started while nothing is on it, so a page start
    /// that follows (a next-page section) doesn't leave it blank: Word keeps the break's mark on
    /// the page before, and starts one page.
    pub(crate) fn take_back_break_page(&mut self) -> bool {
        let empty = self.after_break_mark
            && self.at_top()
            && self.notes.is_empty()
            && self.pages.len() > 1
            && self.pages.last().is_some_and(|p| p.items.len() == self.page_items_start);
        if !empty {
            return false;
        }
        self.pages.pop();
        self.behind.pop();
        self.number = self.number.saturating_sub(1);
        self.after_break_mark = false;
        // The page before was aligned when it was left: nothing on it moves again.
        self.page_items_start = self.pages.last().map_or(0, |p| p.items.len());
        true
    }
    pub(crate) fn new_page(&mut self, first_block: usize, body_top: f32) {
        self.apply_valign();
        self.flush_notes();
        self.beneath = self.sect_beneath;
        self.excl.clear();
        // Floats placed above their anchors further down this page are known from its start.
        let early = self.early;
        self.excl.extend(early.get(&self.pages.len()).into_iter().flatten().map(|(_, _, a)| a.clone()));
        self.seeded = self.excl.len();
        self.float_tables.clear();
        if self.sect.line_numbers.as_ref().is_some_and(|l| l.restart == wordcraft_doc::section::LineNumberRestart::Page) {
            self.line_no = 0;
        }
        let s = self.sect;
        self.number = self.restart_at.take().unwrap_or_else(|| self.number.saturating_add(1));
        let (w, h) = if self.web { (s.page_w, f32::MAX / 4.0) } else { (s.page_w, s.page_h) };
        let bottom = self.body_bottom(body_top);
        let body = Rect::new(s.margin_left + s.gutter, body_top, s.text_width(), (bottom - body_top).max(36.0));
        let mut decor = Vec::new();
        if !self.web {
            decor = page_border_rules(s);
            // Lines between columns.
            if s.columns.separator && s.columns.count > 1 {
                let cols = s.column_boxes();
                for w in cols.windows(2) {
                    if let [(ax, aw), (bx, bw)] = w {
                        // Midway across the space between neighbours (leftwards in an RTL section).
                        let mid = if bx >= ax { (ax + aw + bx) / 2.0 } else { (bx + bw + ax) / 2.0 };
                        let x = s.margin_left + s.gutter + mid;
                        decor.push(Placed::Rule { x0: x, y0: body_top, x1: x, y1: bottom, border: Border::single(0.5) });
                    }
                }
            }
        }
        self.page_items_start = decor.len();
        self.block_marks.clear();
        self.page_valign = s.valign;
        self.pages.push(Page { w, h, section: self.sect_idx, number: self.number, body, first_block, items: decor, ..Default::default() });
        self.behind.push(0);
        self.col = 0;
        self.cols = s.column_boxes();
        self.top = body_top;
        self.page_top = body_top;
        self.bottom = if self.web { f32::MAX / 8.0 } else { bottom };
        self.orig_bottom = self.bottom;
        self.y = self.top;
        // Notes continued from the page before come first in this page's note area.
        // (`cap_columns` below keeps the columns above them.)
        if !self.carry.is_empty() && !self.web {
            self.notes = std::mem::take(&mut self.carry);
        }
        self.max_y = self.top;
        self.content_bottom = self.top;
        self.cap_columns();
    }
    /// Where the columns end on this page: the page bottom above its footnotes, or higher up
    /// when the columns are balanced.
    pub(crate) fn cap_columns(&mut self) {
        self.bottom = self.uncapped_bottom();
        if let Some((page, h)) = self.balance
            && page + 1 == self.pages.len()
        {
            self.bottom = self.bottom.min(self.top + h);
        }
    }
    /// The bottom of the body on a page started with its body at `body_top`: a footer taller than
    /// the bottom margin pushes it up (Word: no gap), leaving it at least a little room.
    fn body_bottom(&self, body_top: f32) -> f32 {
        (self.sect.page_h - self.sect.margin_bottom).min(self.foot_top.max(body_top + 36.0))
    }
    /// The room for text in the next column, or on the page [`Self::advance`] starts after the
    /// last: there, below its body top (`body_top`) and above its footer and the notes carried
    /// onto it (#3907 CodeRabbit: a different first page's header or footer doesn't shrink it).
    pub(crate) fn next_room(&self, body_top: f32) -> f32 {
        if self.col + 1 < self.cols.len() {
            self.uncapped_bottom() - self.top
        } else if self.web {
            f32::MAX / 8.0 - body_top
        } else {
            self.body_bottom(body_top) - self.stack_h(&self.carry) - body_top
        }
    }
    /// The bottom of the columns without balancing: above the footnotes.
    pub(crate) fn uncapped_bottom(&self) -> f32 {
        self.orig_bottom - self.notes_h()
    }
    /// Below the text on this page: the tallest column's bottom.
    pub(crate) fn text_bottom(&self) -> f32 {
        self.max_y.max(self.y)
    }
    pub(crate) fn col_x(&self) -> f32 {
        self.sect.margin_left + self.sect.gutter + self.cols.get(self.col).map(|c| c.0).unwrap_or(0.0)
    }
    pub(crate) fn col_w(&self) -> f32 {
        self.cols.get(self.col).map(|c| c.1).unwrap_or_else(|| self.sect.text_width())
    }
    pub(crate) fn at_top(&self) -> bool {
        self.y <= self.top + 0.01
    }
    /// At the top with nothing above on this page, so moving on gains no room. A continuous
    /// section starting mid-page is at its top but not fresh: what doesn't fit there goes to the
    /// next page (#3907).
    pub(crate) fn fresh(&self) -> bool {
        self.at_top() && self.top <= self.page_top + 0.01
    }
    /// Move to the next column or page.
    pub(crate) fn advance(&mut self, block: usize, body_top: f32) {
        if self.col + 1 < self.cols.len() {
            self.max_y = self.max_y.max(self.y);
            self.col += 1;
            self.y = self.top;
        } else {
            self.new_page(block, body_top);
        }
    }
    pub(crate) fn page(&mut self) -> Option<&mut Page> {
        self.pages.last_mut()
    }
}

pub(crate) fn item_bottom(y: f32, para: &ParaLayout, l0: usize, l1: usize) -> Option<f32> {
    let f = para.lines.get(l0)?;
    let l = para.lines.get(l1.checked_sub(1)?)?;
    Some(y + l.top + l.height - f.top)
}

/// Where body text starts: the top margin, or below the header if it's taller.
pub(crate) fn body_top_for(ctx: &mut Ctx, sect: &SectionProps, header: Option<u32>) -> f32 {
    let Some(id) = header else { return sect.margin_top };
    let Some(part) = ctx.doc.parts.get(&id) else { return sect.margin_top };
    let blocks = part.blocks.clone();
    // Placed as `headers_footers` draws it, so page-relative floats wrap the same way.
    let frame = PageFrame { sect, origin: (sect.margin_left + sect.gutter, sect.header), number: 1, mirror: ctx.mirror };
    let track = std::mem::replace(&mut ctx.track_totals, false);
    let (h, ..) = with_pin(ctx, id, sect, 1, |ctx| {
        layout_box(ctx, StoryRef::Part(id), &blocks, &[], sect.text_width(), BoxContext::Other, 0, Some(frame)).height
    });
    ctx.track_totals = track;
    // Word starts the body right below a header that reaches past the top margin, no gap.
    sect.margin_top.max(sect.header + h)
}

/// Where the footer starts (as `headers_footers` places it), `f32::MAX` without one.
pub(crate) fn foot_top_for(ctx: &mut Ctx, sect: &SectionProps, footer: Option<u32>) -> f32 {
    let Some(part) = footer.and_then(|id| ctx.doc.parts.get(&id).map(|p| (id, p))) else { return f32::MAX };
    let (id, blocks) = (part.0, part.1.blocks.clone());
    // A page total in the footer is filled in after pagination, like the header's: it doesn't
    // make the body lay out again.
    let track = std::mem::replace(&mut ctx.track_totals, false);
    let (laid, ..) =
        with_pin(ctx, id, sect, 1, |ctx| layout_box(ctx, StoryRef::Part(id), &blocks, &[], sect.text_width(), BoxContext::Other, 0, None));
    ctx.track_totals = track;
    let top = sect.page_h - sect.footer - laid.height;
    // Flow-table strokes extend above their cell boxes. Floating decorations keep their wrap rules.
    let flow_cells: Vec<_> = laid
        .items
        .iter()
        .filter_map(|item| match item {
            Placed::Cell { rect, table, story: own, .. }
                if *own == StoryRef::Part(id)
                    && table.0.len() == 1
                    && table
                        .0
                        .first()
                        .and_then(|i| blocks.get(*i as usize))
                        .is_some_and(|b| matches!(&**b, Block::Table(t) if t.props.float.is_none())) =>
            {
                Some(rect)
            }
            _ => None,
        })
        .collect();
    laid.items
        .iter()
        .filter_map(|item| match item {
            Placed::Rule { x0, x1, y0, y1, border }
                if border.is_visible()
                    && (y0 - y1).abs() < 0.01
                    && flow_cells.iter().any(|r| (r.y - y0).abs() < 0.01 && *x0 <= r.x + 0.01 && *x1 >= r.right() - 0.01) =>
            {
                Some(top + y0 - border.width / 2.0)
            }
            _ => None,
        })
        .fold(top, f32::min)
}

/// Where a box of laid-out items sits on its page, for floating objects positioned relative to
/// the page or its margins.
#[derive(Clone, Copy)]
pub(crate) struct PageFrame<'a> {
    pub(crate) sect: &'a SectionProps,
    /// Page position of the box's (0, 0); (0, 0) for body text.
    pub(crate) origin: (f32, f32),
    /// The page's number: inside and outside are left and right on odd pages, the other way
    /// round on even ones.
    pub(crate) number: u32,
    /// Whether an even page is mirrored ([`mirror_even_pages`]).
    pub(crate) mirror: bool,
}

impl PageFrame<'_> {
    pub(crate) fn even(&self) -> bool {
        self.number.is_multiple_of(2)
    }

    /// The widths of the page's left and right margins (the gutter is on the inside), and how
    /// far [`mirror_even_pages`] moves what is laid out on it.
    pub(crate) fn sides(&self) -> (f32, f32, f32) {
        let (inside, outside) = (self.sect.margin_left + self.sect.gutter, self.sect.margin_right);
        let shift = outside - inside;
        if self.mirror && self.even() && shift.is_finite() { (outside, inside, shift) } else { (inside, outside, 0.0) }
    }

    /// Alignment `a` across the page: inside and outside become right and left on an even page.
    pub(crate) fn h_align(frame: Option<&Self>, a: Option<FloatAlign>) -> Option<FloatAlign> {
        match a {
            Some(FloatAlign::Inside) if frame.is_some_and(Self::even) => Some(FloatAlign::End),
            Some(FloatAlign::Outside) if frame.is_some_and(Self::even) => Some(FloatAlign::Start),
            a => a,
        }
    }
}

/// The page `pb` is filling, for placing floating objects on it (`mirror`: [`Ctx::mirror`]).
pub(crate) fn page_frame<'a>(pb: &PageBuilder<'a>, mirror: bool) -> PageFrame<'a> {
    PageFrame { sect: pb.sect, origin: (0.0, 0.0), number: pb.number, mirror }
}
