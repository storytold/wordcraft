//! Incremental layout ([`LayoutOptions::incremental`]): a layout records where each page began
//! (the pagination state before the block placed first there), and the next layout of the same
//! document with a few blocks edited lays out again only from the page before the first edit.
//! Once it reaches a block after the edits in the same state as the last layout did, the rest is
//! taken from the last layout as it was.
//!
//! Anything that could change pages before the edit, or after the point where the states meet,
//! makes the layout a full one: other parts of the document, the options, notes, bookmarks,
//! floating objects, section breaks or text boxes in the edited blocks, and documents with
//! columns, endnotes, page totals in the body or anything else that needs a second pass.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use wordcraft_doc::{Block, Document, InlineObject, Paragraph};

use super::ctx::Ctx;
use super::fields::FieldCtx;
use super::notes::NoteBox;
use super::page::PageBuilder;
use super::pass::Snap;
use super::placed::Placed;
use super::{LayoutCache, LayoutOptions, Page, Path, StoryRef, TypingWord};

/// Blocks between marks when no page begins (a reflowed document is one page).
const EVERY: usize = 24;
/// Incremental layouts in a row before a full one (which also clears out the paragraph cache).
const MAX_IN_A_ROW: u32 = 64;
/// How far back keep-with-next can reach from an edited paragraph ([`crate::flow`] looks 8 ahead).
const KEEP_DEPTH: usize = 8;

/// The pagination state before a block (in section `si`). Kept apart from the block's number,
/// which moves when blocks before it are added or removed.
pub(crate) struct Mark {
    pub(crate) si: usize,
    pub(crate) snap: Snap,
    pub(crate) fields: FieldCtx,
}

/// What a layout leaves to the next one.
#[derive(Clone)]
pub(crate) struct Resume {
    /// Which one this is, for the layout that left it ([`crate::lazy::Lazy`]).
    pub(crate) id: u64,
    /// The document laid out (blocks are shared: an edited one is another pointer).
    doc: Document,
    key: u64,
    typing: Option<TypingWord>,
    /// By block, ascending.
    marks: Vec<(usize, Arc<Mark>)>,
    /// The pages as placed, before they were finished.
    pages: Vec<Page>,
    behind: Vec<usize>,
    /// The state after the last block.
    end: Arc<Mark>,
    in_a_row: u32,
}

impl Resume {
    /// Whether the layout that left this resumed the one before.
    pub(crate) fn resumed(&self) -> bool {
        self.in_a_row > 0
    }

    /// The pages as placed, for [`crate::lazy`] to thin or fill in: placing from them again
    /// reads only their lines' extent and paths, which a thinned paragraph keeps.
    pub(crate) fn pages_mut(&mut self) -> &mut [Page] {
        &mut self.pages
    }
}

/// Numbers each [`Resume`].
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A plan that lays out pages `pages` of the last layout again, as they were, for
/// [`crate::lazy`]: no block was edited, so it starts at the mark at or before the block at
/// the top of the first page and meets the last layout again after the block at the top of the
/// page after the last. `None` unless the document and options are the ones the last layout had.
pub(crate) fn range_plan(doc: &Document, cache: &LayoutCache, opts: &LayoutOptions, pages: std::ops::Range<usize>) -> Option<Plan> {
    let old = cache.resume.clone()?;
    if old.key != options_key(opts) || old.typing != opts.typing_word || !rest_same(&old.doc, doc) {
        return None;
    }
    let (was, now) = (&old.doc.body, &doc.body);
    if was.len() != now.len() || !(0..was.len()).all(|i| Arc::ptr_eq(&was[i], &now[i])) {
        return None;
    }
    let first = old.pages.get(pages.start)?.first_block;
    // Placing stops being new once a mark after this block is reached in the same state.
    let edited_end = old.pages.get(pages.end).map_or(usize::MAX, |p| p.first_block);
    let start = old.marks.partition_point(|(b, _)| *b <= first).checked_sub(1)?;
    Some(Plan { old, start, edited_end, delta: 0, started: false })
}

/// Where a layout picks up from the last one.
pub(crate) struct Plan {
    old: Arc<Resume>,
    /// The mark to start from.
    start: usize,
    /// The first block after the edited ones, and how many blocks were added (or removed).
    edited_end: usize,
    delta: isize,
    started: bool,
}

/// The incremental state of one layout pass.
pub(crate) struct Run {
    pub(crate) plan: Option<Plan>,
    marks: Vec<(usize, Arc<Mark>)>,
    last_pages: usize,
    last_block: usize,
    /// The states met: the rest of the pages are the last layout's.
    pub(crate) met: bool,
    pub(crate) off: bool,
    /// Lays out only the pages a [`range_plan`] asks for ([`Run::range`]).
    pub(crate) only: bool,
}

impl Run {
    pub(crate) fn new(plan: Option<Plan>) -> Self {
        let marks = plan.as_ref().map_or_else(Vec::new, |p| p.old.marks[..p.start].to_vec());
        Run { plan, marks, last_pages: usize::MAX, last_block: 0, met: false, off: false, only: false }
    }

    /// A run that places only from a [`range_plan`]'s mark until the state meets the last
    /// layout's again, and stops there: no marks, no rest of the pages taken, nothing left for
    /// the next layout. The pages before the mark are placeholders (size, section and number,
    /// no items) but for the last few, which placing may reach back into. For [`crate::lazy`],
    /// which takes only the paragraphs laid out: a page costs about a page.
    pub(crate) fn range(plan: Plan) -> Self {
        Run { plan: Some(plan), marks: Vec::new(), last_pages: usize::MAX, last_block: 0, met: false, off: false, only: true }
    }

    /// Stop: this layout can't be resumed from, nor can it resume.
    pub(crate) fn turn_off(&mut self) {
        self.off = true;
        self.plan = None;
    }

    /// Whether blocks of section `si` are skipped because the plan starts after them.
    pub(crate) fn skips(&self, si: usize) -> bool {
        !self.off && self.plan.as_ref().is_some_and(|p| !p.started && p.old.marks[p.start].1.si > si)
    }

    /// At the start of section `si`'s blocks (from `block`): the block to place first, after
    /// restoring the last layout's state if the plan starts in this section.
    pub(crate) fn enter(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder, si: usize, block: usize) -> usize {
        if self.off {
            return block;
        }
        let Some(p) = self.plan.as_mut().filter(|p| !p.started) else { return block };
        let (at, m) = &p.old.marks[p.start];
        if m.si != si {
            return block;
        }
        p.started = true;
        let n = m.snap.pages;
        let mut pages: Vec<Page> = if self.only {
            let full = n.saturating_sub(KEEP_DEPTH);
            p.old.pages[..full].iter().map(Page::placeholder).chain(p.old.pages[full..n].iter().cloned()).collect()
        } else {
            p.old.pages[..n].to_vec()
        };
        let mut behind = p.old.behind[..n].to_vec();
        if let (Some(pg), Some(b)) = (pages.last_mut(), behind.last_mut()) {
            let k = b.saturating_sub(m.snap.behind).min(pg.items.len());
            pg.items.drain(..k);
            pg.items.truncate(m.snap.items);
            *b = m.snap.behind;
        }
        restore(ctx, pb, m, pages, behind);
        *at
    }

    /// Before placing `block`: record a mark when one is due, or, after the edits, take the rest
    /// of the last layout if the state is the same as it was there. True when it was taken.
    pub(crate) fn at(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder, block: usize, si: usize) -> bool {
        if self.off {
            return false;
        }
        if let Some(p) = &self.plan
            && p.started
            && block > p.edited_end
            && let Some(was) = block.checked_add_signed(-p.delta)
            && let Ok(k) = p.old.marks.binary_search_by_key(&was, |(b, _)| *b)
            && same(&p.old.marks[k].1, pb, ctx)
        {
            if self.only {
                self.met = true;
                return true;
            }
            let old = Arc::clone(&p.old);
            let delta = p.delta;
            self.take_rest(ctx, pb, &old, (k, delta));
            return true;
        }
        if self.only {
            return false;
        }
        let pages = pb.pages.len();
        if pages != self.last_pages || block >= self.last_block + EVERY {
            (self.last_pages, self.last_block) = (pages, block);
            self.marks.push((block, Arc::new(Mark { si, snap: Snap::take(pb, ctx), fields: ctx.fields.clone() })));
        }
        false
    }

    fn take_rest(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder, old: &Resume, (k, delta): (usize, isize)) {
        let m = &old.marks[k].1;
        let n = m.snap.pages;
        let mut pages = std::mem::take(&mut pb.pages);
        let mut behind = std::mem::take(&mut pb.behind);
        if n > 0 {
            // The page the states met on: what was there then, then what the last layout added
            // (behind the text at the front, the rest after).
            let (was, was_behind) = (&old.pages[n - 1], old.behind[n - 1]);
            let front = was_behind.saturating_sub(m.snap.behind);
            let now = std::mem::take(&mut pages[n - 1]);
            let mut items = was.items[..front].to_vec();
            items.extend(now.items);
            items.extend_from_slice(&was.items[(front + m.snap.items).min(was.items.len())..]);
            if delta != 0 {
                let after = (front + m.snap.items).min(items.len());
                shift_items(&mut items[..front], delta);
                shift_items(&mut items[after..], delta);
            }
            pages[n - 1] = Page { items, first_block: now.first_block, ..was.clone_without_items() };
            behind[n - 1] = was_behind;
        }
        let rest = pages.len();
        pages.extend_from_slice(&old.pages[n..]);
        if delta != 0 {
            for p in &mut pages[rest..] {
                p.first_block = p.first_block.saturating_add_signed(delta);
                shift_items(&mut p.items, delta);
            }
        }
        behind.truncate(n);
        behind.extend_from_slice(&old.behind[n..]);
        restore(ctx, pb, &old.end, pages, behind);
        self.marks.extend(old.marks[k..].iter().map(|(b, m)| (b.saturating_add_signed(delta), Arc::clone(m))));
        self.met = true;
    }

    /// After the last block: what the next layout resumes from. `None` if this one can't be.
    pub(crate) fn finish(self, doc: &Document, opts: &LayoutOptions, pb: &mut PageBuilder, ctx: &Ctx, si: usize) -> Option<Resume> {
        if self.off || self.only || ctx.uses_totals || ctx.boxes.total() > 0 || pb.pages.iter().any(|p| !p.float_tables.is_empty()) {
            return None;
        }
        // What resuming and taking the rest index by.
        let fits = |m: &Mark| m.snap.pages <= pb.pages.len();
        if pb.behind.len() != pb.pages.len()
            || pb.behind.iter().zip(&pb.pages).any(|(b, p)| *b > p.items.len())
            || !self.marks.iter().all(|(_, m)| fits(m))
        {
            return None;
        }
        let in_a_row = self.plan.as_ref().map_or(0, |p| p.old.in_a_row + 1);
        let end = match (&self.plan, self.met) {
            (Some(p), true) => Arc::clone(&p.old.end),
            _ => Arc::new(Mark { si, snap: Snap::take(pb, ctx), fields: ctx.fields.clone() }),
        };
        Some(Resume {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            doc: doc.clone(),
            key: options_key(opts),
            typing: opts.typing_word,
            marks: self.marks,
            pages: pb.pages.clone(),
            behind: pb.behind.clone(),
            end,
            in_a_row,
        })
    }

    /// Whether this pass resumed the last layout (so it placed only some of the blocks).
    pub(crate) fn resumed(&self) -> bool {
        self.plan.as_ref().is_some_and(|p| p.started)
    }
}

/// Body paths moved `delta` blocks on.
fn shift_items(items: &mut [Placed], delta: isize) {
    let shift = |story: &StoryRef, path: &mut Path| {
        if *story == StoryRef::Body
            && let Some(b) = path.0.first_mut()
        {
            *b = b.saturating_add_signed(delta as i32);
        }
    };
    for it in items {
        match it {
            Placed::Lines { story, path, .. }
            | Placed::Image { story, path, .. }
            | Placed::Graphic { story, path, .. }
            | Placed::Object { story, path, .. }
            | Placed::Cell { story, table: path, .. } => shift(story, path),
            Placed::Shape { object: Some(pos), .. } => shift(&pos.story, &mut pos.path),
            Placed::Clip { items, .. } => shift_items(items, delta),
            _ => {}
        }
    }
}

fn restore(ctx: &mut Ctx, pb: &mut PageBuilder, m: &Mark, pages: Vec<Page>, behind: Vec<usize>) {
    *pb = m.snap.pb.clone();
    (pb.pages, pb.behind) = (pages, behind);
    ctx.counters = m.snap.counters.clone();
    ctx.eq_count = m.snap.eq_count;
    ctx.header_repeats = m.snap.header_repeats;
    let (f, g) = (&mut ctx.fields, &m.fields);
    f.page = g.page;
    f.page_format = g.page_format;
    f.section_pages = g.section_pages;
    f.section = g.section;
    f.styleref = g.styleref.clone();
    f.page_from = g.page_from.clone();
}

/// Whether placing from here goes as it went from `m`.
fn same(m: &Mark, pb: &PageBuilder, ctx: &Ctx) -> bool {
    let s = &m.snap;
    let a = &s.pb;
    s.pages == pb.pages.len()
        && s.items == pb.pages.last().map_or(0, |p| p.items.len())
        && s.behind == pb.behind.last().copied().unwrap_or(0)
        && pb.pages.last().is_none_or(|p| p.float_tables.is_empty())
        && s.counters == ctx.counters
        && s.eq_count == ctx.eq_count
        && s.header_repeats == ctx.header_repeats
        && s.boxes == ctx.boxes.total()
        && fields_same(&m.fields, &ctx.fields)
        && a.sect_idx == pb.sect_idx
        && (Arc::ptr_eq(&a.sect, &pb.sect) || a.sect == pb.sect)
        && a.page_format == pb.page_format
        && a.col == pb.col
        && a.cols == pb.cols
        && a.y == pb.y
        && a.top == pb.top
        && a.page_top == pb.page_top
        && a.bottom == pb.bottom
        && a.number == pb.number
        && notes_same(&a.notes, &pb.notes)
        && notes_same(&a.carry, &pb.carry)
        && a.orig_bottom == pb.orig_bottom
        && a.prev == pb.prev
        && a.after_break_mark == pb.after_break_mark
        && a.next_starts_page == pb.next_starts_page
        && a.restart_at == pb.restart_at
        && a.excl.is_empty()
        && pb.excl.is_empty()
        && a.float_tables.is_empty()
        && pb.float_tables.is_empty()
        && a.line_no == pb.line_no
        && a.page_items_start == pb.page_items_start
        && a.block_marks == pb.block_marks
        && a.page_valign == pb.page_valign
        && a.para_end == pb.para_end
        && a.max_y == pb.max_y
        && a.content_bottom == pb.content_bottom
        && a.balance.is_none()
        && pb.balance.is_none()
        && a.col_break_page == pb.col_break_page
        && a.sect_beneath == pb.sect_beneath
        && a.beneath == pb.beneath
        && a.foot_top == pb.foot_top
        && a.seeded == pb.seeded
        && a.settle == pb.settle
        && a.column_lines.is_none()
        && pb.column_lines.is_none()
}

/// The same notes waiting for the foot of the page.
fn notes_same(a: &[NoteBox], b: &[NoteBox]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.id == y.id && x.h == y.h && x.cont == y.cont && x.items.len() == y.items.len())
}

fn fields_same(a: &FieldCtx, b: &FieldCtx) -> bool {
    let styleref = match (&a.styleref, &b.styleref) {
        (None, None) => true,
        (Some((x, p)), Some((y, q))) => Arc::ptr_eq(x, y) && p == q,
        _ => false,
    };
    a.page == b.page
        && a.page_format == b.page_format
        && a.pages == b.pages
        && a.section_pages == b.section_pages
        && a.section == b.section
        && a.page_from == b.page_from
        && styleref
}

/// The options as they bear on every page (the word being typed is compared apart: it bears
/// only on its paragraph).
fn options_key(opts: &LayoutOptions) -> u64 {
    let o = LayoutOptions { typing_word: None, incremental: false, ..opts.clone() };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{o:?}").hash(&mut h);
    h.finish()
}

/// Where to resume the last layout for `doc`, if it can be.
pub(crate) fn plan(doc: &Document, cache: &LayoutCache, opts: &LayoutOptions) -> Option<Plan> {
    let old = cache.resume.clone()?;
    if old.in_a_row >= MAX_IN_A_ROW || old.key != options_key(opts) || !rest_same(&old.doc, doc) {
        return None;
    }
    let (was, now) = (&old.doc.body, &doc.body);
    let (n0, n1) = (was.len(), now.len());
    let mut p = 0;
    while p < n0.min(n1) && Arc::ptr_eq(&was[p], &now[p]) {
        p += 1;
    }
    let mut s = 0;
    while p + s < n0.min(n1) && Arc::ptr_eq(&was[n0 - 1 - s], &now[n1 - 1 - s]) {
        s += 1;
    }
    // Notes are numbered through the document: the edited blocks keep theirs, in order.
    fn refs(blocks: &[Arc<Block>]) -> Vec<(u32, &str)> {
        let mut v = Vec::new();
        for b in blocks {
            note_refs(b, &mut v);
        }
        v
    }
    if !was[p..n0 - s].iter().chain(&now[p..n1 - s]).all(|b| plain_block(b)) || refs(&was[p..n0 - s]) != refs(&now[p..n1 - s]) {
        return None;
    }
    let delta = n1 as isize - n0 as isize;
    // In the new numbering: the edited blocks, then those whose word being typed changed.
    let mut edited = (p < n1 - s || p < n0 - s).then_some((p, (n1 - s).max(p + 1)));
    if old.typing != opts.typing_word {
        for (t, body, new) in [(old.typing, was, false), (opts.typing_word, now, true)] {
            let Some(t) = t else { continue };
            let mut i = body.iter().position(|b| matches!(&**b, Block::Para(p) if p.rev == t.rev))?;
            if !new && i >= n0 - s {
                i = i.checked_add_signed(delta)?;
            }
            edited = Some(edited.map_or((i, i + 1), |(a, b)| (a.min(i), b.max(i + 1))));
        }
    }
    let (first, edited_end) = edited?;
    // The paragraphs kept with the next (looking ahead to the edited one), and the one before.
    let mut from = first.saturating_sub(1);
    let keeps = |b: &Block| matches!(b, Block::Para(p) if doc.styles.resolve_para(&p.props).keep_next);
    while from > 0 && first - from < KEEP_DEPTH && keeps(&now[from - 1]) {
        from -= 1;
    }
    let start = old.marks.partition_point(|(b, _)| *b <= from).checked_sub(1)?;
    Some(Plan { old, start, edited_end, delta, started: false })
}

fn note_refs<'b>(b: &'b Block, v: &mut Vec<(u32, &'b str)>) {
    match b {
        Block::Para(p) => v.extend(p.objects.iter().filter_map(|o| match o {
            InlineObject::NoteRef { id, custom, .. } => Some((*id, custom.as_str())),
            _ => None,
        })),
        Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).flat_map(|c| &c.blocks).for_each(|b| note_refs(b, v)),
    }
}

/// A block whose edits change nothing outside it (beyond its notes, compared apart): no
/// section break or floating objects.
fn plain_block(b: &Block) -> bool {
    match b {
        Block::Para(p) => plain_para(p),
        Block::Table(t) => t.rows.iter().all(|r| r.cells.iter().all(|c| c.blocks.iter().all(|b| plain_block(b)))),
    }
}

fn plain_para(p: &Paragraph) -> bool {
    p.section.is_none()
        && p.objects.iter().all(|o| {
            matches!(
                o,
                InlineObject::NoteRef { .. }
                    | InlineObject::BookmarkStart { .. }
                    | InlineObject::BookmarkEnd { .. }
                    | InlineObject::Field { .. }
                    | InlineObject::FieldStart { .. }
                    | InlineObject::FieldEnd
                    | InlineObject::ControlStart { .. }
                    | InlineObject::ControlEnd
                    | InlineObject::CommentStart { .. }
                    | InlineObject::CommentEnd { .. }
                    | InlineObject::PermStart { .. }
                    | InlineObject::PermEnd { .. }
                    | InlineObject::Equation { .. }
                    | InlineObject::Ruby { .. }
                    | InlineObject::PositionalTab { .. }
            )
        })
}

/// Everything but the body is as it was.
fn rest_same(a: &Document, b: &Document) -> bool {
    let parts = a.parts.len() == b.parts.len()
        && a.parts.iter().zip(b.parts.iter()).all(|((i, p), (j, q))| {
            i == j
                && p.kind == q.kind
                && p.body == q.body
                && p.blocks.len() == q.blocks.len()
                && p.blocks.iter().zip(q.blocks.iter()).all(|(x, y)| Arc::ptr_eq(x, y))
        });
    let media = a.media.len() == b.media.len() && a.media.iter().zip(b.media.iter()).all(|((i, x), (j, y))| i == j && Arc::ptr_eq(x, y));
    parts
        && media
        && (Arc::ptr_eq(&a.styles, &b.styles) || a.styles == b.styles)
        && (Arc::ptr_eq(&a.numbering, &b.numbering) || a.numbering == b.numbering)
        && a.last_section == b.last_section
        && a.settings == b.settings
        && a.core == b.core
        && a.custom_props == b.custom_props
        && a.sources == b.sources
        && a.comments == b.comments
        && a.revisions == b.revisions
        && a.footnote_separators == b.footnote_separators
        && a.endnote_separators == b.endnote_separators
        && a.glossary == b.glossary
        && a.missing_pictures == b.missing_pictures
        && a.fit_state == b.fit_state
}

impl Page {
    /// The page's size, section, number and first block, with no items, header or footer: what
    /// a [`Run::range`] keeps of a page long before the one it places from.
    fn placeholder(&self) -> Page {
        Page {
            w: self.w,
            h: self.h,
            section: self.section,
            number: self.number,
            items: Vec::new(),
            header: Vec::new(),
            footer: Vec::new(),
            body: self.body,
            header_story: self.header_story,
            footer_story: self.footer_story,
            hf_slot: self.hf_slot,
            first_block: self.first_block,
            markup: self.markup,
            float_tables: Vec::new(),
        }
    }

    fn clone_without_items(&self) -> Page {
        Page {
            w: self.w,
            h: self.h,
            section: self.section,
            number: self.number,
            items: Vec::new(),
            header: self.header.clone(),
            footer: self.footer.clone(),
            body: self.body,
            header_story: self.header_story,
            footer_story: self.footer_story,
            hf_slot: self.hf_slot,
            first_block: self.first_block,
            markup: self.markup,
            float_tables: self.float_tables.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wordcraft_doc::{Block, CharProps, Document, Paragraph};

    use crate::{LayoutCache, LayoutOptions, ViewMode, layout};

    /// A small deterministic generator, so a failure repeats.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    const WORDS: [&str; 12] = ["the", "court", "held", "that", "appeal", "was", "allowed", "with", "costs", "notwithstanding", "a", "submission"];

    fn text(r: &mut Rng, words: usize) -> String {
        (0..words).map(|_| WORDS[r.next(WORDS.len())]).collect::<Vec<_>>().join(" ")
    }

    fn para(r: &mut Rng) -> Paragraph {
        let words = 3 + r.next(120);
        let mut p = Paragraph::with_text(&text(r, words), CharProps::default());
        match r.next(10) {
            0 => {
                p.props.keep_next = Some(true);
                p.props.space_before = Some(18.0);
            }
            1 => p.props.page_break_before = Some(true),
            2 => p.props.keep_lines = Some(true),
            _ => {}
        }
        p
    }

    fn page_debug(doc: &Document, opts: &LayoutOptions) -> String {
        let full = LayoutOptions { incremental: false, ..opts.clone() };
        format!("{:?}", layout(doc, &mut LayoutCache::new(), &full).pages)
    }

    /// Edits paragraphs in place (typing, deleting, growing past a page, shrinking) and checks
    /// each incremental layout page for page against a full one.
    fn edits_match(view: ViewMode, seed: u64) {
        let mut r = Rng(seed);
        let mut doc = Document::default();
        doc.body = (0..400).map(|_| Arc::new(Block::Para(para(&mut r)))).collect();
        let opts = LayoutOptions { view, web_width: 500.0, incremental: true, ..Default::default() };
        let mut cache = LayoutCache::new();
        layout(&doc, &mut cache, &opts);
        let mut resumed = 0;
        for step in 0..60 {
            let i = r.next(doc.body.len());
            match r.next(6) {
                // Return: a paragraph split in two (the first half edited, a new one after).
                0 => {
                    doc.body.insert(i + 1, Arc::new(Block::Para(para(&mut r))));
                }
                // Deleting a paragraph mark: two paragraphs become one.
                1 if doc.body.len() > 2 => {
                    doc.body.remove(i);
                }
                _ => {}
            }
            let i = i.min(doc.body.len() - 1);
            let Block::Para(old) = &*doc.body[i] else { continue };
            let now: String = match r.next(4) {
                0 => format!("{} {}", old.text, text(&mut r, 1)),
                1 => old.text.chars().take(old.text.len().saturating_sub(1 + r.next(40))).collect(),
                2 => format!("{} {}", old.text, text(&mut r, 150)),
                _ => {
                    let n = 1 + r.next(5);
                    text(&mut r, n)
                }
            };
            let mut p = Paragraph::with_text(&now, CharProps::default());
            p.props = old.props.clone();
            doc.body[i] = Arc::new(Block::Para(p));
            let l = layout(&doc, &mut cache, &opts);
            resumed += usize::from(cache.resume.as_ref().is_some_and(|r| r.in_a_row > 0));
            assert_eq!(format!("{:?}", l.pages), page_debug(&doc, &opts), "{view:?} seed {seed}, step {step}: block {i}");
        }
        assert!(resumed > 40, "only {resumed} of 60 layouts were incremental");
    }

    #[test]
    fn incremental_print_layout_matches_a_full_one() {
        for seed in [7, 1913, 88_123] {
            edits_match(ViewMode::Print, seed);
        }
    }

    #[test]
    fn incremental_reflow_matches_a_full_one() {
        edits_match(ViewMode::Web, 42);
    }

    /// A paragraph added before the rest moves the rest's block numbers on.
    #[test]
    fn blocks_added_move_the_rest_on() {
        let mut r = Rng(5);
        let mut doc = Document::default();
        doc.body = (0..50).map(|_| Arc::new(Block::Para(para(&mut r)))).collect();
        let opts = LayoutOptions { incremental: true, ..Default::default() };
        let mut cache = LayoutCache::new();
        layout(&doc, &mut cache, &opts);
        doc.body.insert(10, Arc::new(Block::Para(para(&mut r))));
        let l = layout(&doc, &mut cache, &opts);
        assert_eq!(cache.resume.as_ref().map(|r| r.in_a_row), Some(1));
        assert_eq!(format!("{:?}", l.pages), page_debug(&doc, &opts));
    }
}
