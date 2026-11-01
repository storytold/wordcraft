//! Laying a document out again after an edit from the page the edit is on, not from the first
//! page, and only until its pages fall back into step with the layout before.
//!
//! A first pass (no page totals, footnote pages or early floats known yet) notes where its walk
//! is at the top of each page ([`Mark`]) and, after the body, keeps its pages and its walk
//! ([`Resume`]). The next first pass compares its document with that one. When only body blocks
//! changed (the rest of the document, the options and the sections being the same), it starts
//! from the last mark before the change that nothing before it looked ahead into; and when, past
//! the change, a page starts as it did before with the walk as it was, it takes the earlier
//! pages from there. Its pages are those a pass from the first page makes: each mark holds all
//! the state the rest of the pass reads, and what is worked out once per layout (linked text
//! boxes' text split over them, picture transparency) is never under way at a mark. Text boxes on
//! their own are laid out where they are placed, from their own story: a mark only counts them.

use std::collections::VecDeque;
use std::sync::LazyLock;

use wordcraft_doc::Revision;

use super::cache::hash_of;
use super::ctx::Ctx;
use super::fields::NoteNum;
use super::floats::{EarlyFloats, WrapArea};
use super::page::PageBuilder;
use super::placed::Placed;
use super::{
    Arc, Block, Blocks, Counters, Document, FieldCtx, HashMap, HideDeleted, LayoutOptions, Page, Path, SectionProps, StoryRef, TextDirection,
    TypingWord, ViewMode, para,
};

/// What a kept builder borrows until a pass gives it its own section and early floats.
static NO_SECT: LazyLock<SectionProps> = LazyLock::new(SectionProps::default);
static NO_EARLY: LazyLock<EarlyFloats> = LazyLock::new(EarlyFloats::new);

#[cfg(test)]
thread_local! {
    /// Passes on this thread started again from an earlier one, and those of them that joined its
    /// pages: tests check the layouts they compare were made that way.
    pub(crate) static STARTS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// The walk loop's own state: the endnotes still to place, the text boxes laid out in balancing
/// attempts thrown away, and the text direction of the section before.
pub(crate) type Loop = (VecDeque<(u32, usize)>, usize, TextDirection);

/// The walk state besides the page builder's.
#[derive(Clone)]
struct Walk {
    counters: Counters,
    fields: FieldCtx,
    eq_count: u32,
    grid: para::Grid,
    hide_deleted: HideDeleted,
    uses_totals: bool,
    track_totals: bool,
    pct_base: Option<f32>,
    floats_above: HashMap<(usize, usize), Option<(usize, WrapArea)>>,
    ahead: usize,
    endnotes: VecDeque<(u32, usize)>,
    discarded: usize,
    turn: TextDirection,
    /// Text boxes laid out so far, those on pages taken from an earlier pass included.
    boxes: usize,
    /// After the body only: the heights of text boxes sized to their text worked out by then, and
    /// whether one could not be measured where it was met (marks are taken only before either).
    fitted: HashMap<(u32, u32), Option<f32>>,
    fitted_unmeasured: bool,
}

impl Walk {
    fn take(ctx: &Ctx, (endnotes, discarded, turn): (&VecDeque<(u32, usize)>, usize, TextDirection)) -> Walk {
        Walk {
            counters: ctx.counters.clone(),
            fields: ctx.fields.clone(),
            eq_count: ctx.eq_count,
            grid: ctx.grid,
            hide_deleted: ctx.hide_deleted,
            uses_totals: ctx.uses_totals,
            track_totals: ctx.track_totals,
            pct_base: ctx.pct_base,
            // Only the floats found above their anchors: a pass reads nothing else of it (one placed
            // again overwrites its entry either way), and every float placed so far has one.
            floats_above: ctx.floats_above.iter().filter(|(_, v)| v.is_some()).map(|(k, v)| (*k, v.clone())).collect(),
            ahead: ctx.ahead.get(),
            endnotes: endnotes.clone(),
            discarded,
            turn,
            boxes: ctx.boxes.total() + ctx.boxes_before,
            fitted: HashMap::new(),
            fitted_unmeasured: false,
        }
    }

    /// Put the walk back in `ctx`, replacing what the start of the pass left there. The pass's
    /// own note numbers stay (they are the same: [`Track::start`]); linked text boxes and picture
    /// transparency start again, as none were laid out or decoded when the walk was kept.
    fn put(self, ctx: &mut Ctx) -> Loop {
        let notes = Arc::clone(&ctx.fields.notes);
        ctx.counters = self.counters;
        ctx.fields = FieldCtx { notes, ..self.fields };
        ctx.eq_count = self.eq_count;
        ctx.grid = self.grid;
        ctx.hide_deleted = self.hide_deleted;
        ctx.uses_totals = self.uses_totals;
        ctx.track_totals = self.track_totals;
        ctx.pct_base = self.pct_base;
        ctx.floats_above = self.floats_above;
        ctx.ahead.set(self.ahead);
        ctx.boxes = Default::default();
        ctx.boxes_before = self.boxes;
        ctx.chains = Default::default();
        ctx.alphas = Default::default();
        // A pass joining an earlier one's pages takes the box heights its body worked out after
        // them (headers and footers read them, and the boxes it laid out count them).
        ctx.fitted.extend(self.fitted);
        ctx.fitted_unmeasured |= self.fitted_unmeasured;
        (self.endnotes, self.discarded, self.turn)
    }

    /// The blocks it names moved by `shift`.
    fn shift(&mut self, shift: Shift) {
        if shift.by != 0 {
            self.ahead = shift.block(self.ahead);
            self.floats_above = self.floats_above.drain().map(|((b, o), v)| ((shift.block(b), o), v)).collect();
        }
    }

    /// The walk written out to be compared, without the note numbers and with the floats in order.
    fn print(&self) -> String {
        let mut floats: Vec<_> = self.floats_above.iter().collect();
        floats.sort_by_key(|(k, _)| **k);
        // Maps in their keys' order: two equal hash maps may list their entries in different orders.
        let mut bookmarks: Vec<_> = self.fields.bookmark_pages.iter().collect();
        bookmarks.sort_by(|a, b| a.0.cmp(b.0));
        let fields = FieldCtx { notes: Default::default(), bookmark_pages: Default::default(), ..self.fields.clone() };
        format!(
            "{:?}",
            (
                &self.counters,
                fields,
                bookmarks,
                self.eq_count,
                self.grid,
                self.hide_deleted,
                (self.uses_totals, self.track_totals),
                self.pct_base,
                floats,
                &self.endnotes,
                (self.discarded, self.turn, self.boxes)
            )
        )
    }
}

/// Where the walk was as body block `block` (of section `si`, whose body starts `body_top` down
/// its pages) began, the first to begin on its page.
#[derive(Clone)]
pub(crate) struct Mark {
    block: usize,
    si: usize,
    body_top: f32,
    /// Pages so far, the last being `page` as far as it had got.
    pages: usize,
    page: Page,
    behind: usize,
    /// The section of the page before (a table's first rows look at it).
    before: Option<usize>,
    /// The builder without its pages.
    pb: PageBuilder<'static>,
    walk: Walk,
}

impl Mark {
    fn take(
        ctx: &Ctx,
        pb: &mut PageBuilder,
        (block, si, body_top): (usize, usize, f32),
        lp: (&VecDeque<(u32, usize)>, usize, TextDirection),
    ) -> Option<Mark> {
        let page = pb.pages.last()?.clone();
        let (pages, behind) = (std::mem::take(&mut pb.pages), std::mem::take(&mut pb.behind));
        let copy = pb.clone();
        (pb.pages, pb.behind) = (pages, behind);
        Some(Mark {
            block,
            si,
            body_top,
            pages: pb.pages.len(),
            page,
            behind: pb.behind.last().copied().unwrap_or(0),
            before: pb.pages.iter().rev().nth(1).map(|p| p.section),
            pb: copy.with_refs(&NO_SECT, &NO_EARLY),
            walk: Walk::take(ctx, lp),
        })
    }

    /// Whether the walk from here certainly goes as it does from the earlier pass's mark `other`,
    /// its blocks moved by `shift` (compared cheaply first).
    fn same(&self, other: &Mark, shift: Shift) -> bool {
        let print = |m: &Mark| {
            // Where the last paragraph ended is read for the next one in the same column of the same
            // page only, and every paragraph placed sets it: one on a page before the mark's is left out.
            let mut pb = m.pb.clone();
            pb.para_end = pb.para_end.filter(|(page, ..)| *page >= m.pages);
            format!("{} {} {:?} {:?} {:?} {:?}", m.si, m.body_top, m.before, m.page, pb, m.behind)
        };
        if self.block != shift.block(other.block)
            || self.pages != other.pages
            || self.pb.y != other.pb.y
            || self.pb.col != other.pb.col
            || self.page.items.len() != other.page.items.len()
        {
            return false;
        }
        let other = other.clone().shifted(shift);
        // The walk first: the page so far is the longer to write out.
        self.walk.print() == other.walk.print() && print(self) == print(&other)
    }

    fn shifted(mut self, shift: Shift) -> Mark {
        self.block = shift.block(self.block);
        shift.page(&mut self.page);
        self.walk.shift(shift);
        self
    }
}

/// Body blocks of the earlier document from `from` on, moved by `by` in this one (the blocks
/// inserted, or removed, before them).
#[derive(Clone, Copy)]
struct Shift {
    from: usize,
    by: i64,
}

impl Shift {
    fn block(self, b: usize) -> usize {
        if b < self.from {
            return b;
        }
        i64::try_from(b).ok().and_then(|b| b.checked_add(self.by)).and_then(|b| usize::try_from(b).ok()).unwrap_or(b)
    }

    fn path(self, story: StoryRef, path: &mut Path) {
        if story == StoryRef::Body
            && let Some(b) = path.0.first_mut()
        {
            *b = u32::try_from(self.block(*b as usize)).unwrap_or(*b);
        }
    }

    fn page(self, page: &mut Page) {
        if self.by == 0 {
            return;
        }
        page.first_block = self.block(page.first_block);
        for t in &mut page.float_tables {
            self.path(StoryRef::Body, t);
        }
        for it in &mut page.items {
            self.item(it);
        }
    }

    fn item(self, it: &mut Placed) {
        match it {
            Placed::Lines { story, path, .. }
            | Placed::Image { story, path, .. }
            | Placed::Graphic { story, path, .. }
            | Placed::Object { story, path, .. }
            | Placed::Cell { story, table: path, .. } => self.path(*story, path),
            Placed::Shape { object, .. } => {
                if let Some(pos) = object {
                    self.path(pos.story, &mut pos.path);
                }
            }
            Placed::Clip { items, .. } => items.iter_mut().for_each(|it| self.item(it)),
            Placed::Fill { .. } | Placed::Rule { .. } => {}
        }
    }
}

/// A first pass kept for the next, in the [`LayoutCache`](crate::LayoutCache).
pub(crate) struct Resume {
    inputs: u64,
    /// What `inputs` names by address, kept so no other blocks or bytes can come to have it.
    _held: Held,
    body: Blocks,
    sections: Vec<(usize, SectionProps)>,
    notes: Arc<HashMap<u32, NoteNum>>,
    typing: Option<TypingWord>,
    revisions: Vec<Revision>,
    /// How many paragraph layouts the cache had in use after the last pass from the first page.
    used: usize,
    marks: Vec<Mark>,
    /// The builder after the body, with its pages, and the walk then.
    end: PageBuilder<'static>,
    end_walk: Walk,
}

type Held = (Vec<Blocks>, Vec<Arc<Vec<u8>>>);

/// The earlier pass being started again.
struct Earlier {
    /// Its marks after the one started from.
    marks: Vec<Mark>,
    /// Its pages from the one started on (`base`), and their items behind the text.
    pages: Vec<Page>,
    behind: Vec<usize>,
    base: usize,
    end: PageBuilder<'static>,
    end_walk: Walk,
    /// The first block after the changed ones in this document, and how its blocks from there
    /// moved from the earlier one's.
    after: usize,
    shift: Shift,
}

/// A pass starting again from a [`Mark`]: the mark and the pages before its page.
pub(crate) struct Start {
    mark: Mark,
    pages: Vec<Page>,
    behind: Vec<usize>,
}

impl Start {
    pub(crate) fn si(&self) -> usize {
        self.mark.si
    }

    /// The walk put back in `ctx`, with the builder (given section `sect` and `early`), the loop's
    /// state, the block to go on from and where the section's body starts.
    pub(crate) fn put<'p>(self, ctx: &mut Ctx, sect: &'p SectionProps, early: &'p EarlyFloats) -> (PageBuilder<'p>, Loop, usize, f32) {
        let Start { mark, mut pages, mut behind } = self;
        let mut pb = mark.pb.with_refs(sect, early);
        pages.push(mark.page);
        behind.push(mark.behind);
        (pb.pages, pb.behind) = (pages, behind);
        (pb, mark.walk.put(ctx), mark.block, mark.body_top)
    }
}

/// Marks taken during a pass, and the earlier pass it started again from.
#[derive(Default)]
pub(crate) struct Track {
    on: bool,
    inputs: u64,
    held: Held,
    sections: Vec<(usize, SectionProps)>,
    typing: Option<TypingWord>,
    revisions: Vec<Revision>,
    /// What the cache had in use after the last pass from the first page (`Resume::used`).
    used: Option<usize>,
    /// Pages when the last mark was taken.
    last: usize,
    marks: Vec<Mark>,
    earlier: Option<Earlier>,
    /// The earlier mark the pages fell back into step at, and how far blocks were looked ahead then.
    joined: Option<(usize, usize)>,
}

/// The inputs of a layout besides the body, as one hash: the cache environment, the options but
/// the word being typed, the rest of the document but its tracked changes, and the address of each other story's blocks
/// and each picture's bytes (kept alive by the returned [`Held`]). `None` when they can't be read.
fn inputs(doc: &Document, opts: &LayoutOptions, env: u64) -> Option<(u64, Held)> {
    let rest = serde_json::to_string(&(
        &doc.last_section,
        &doc.comments,
        &doc.settings,
        &doc.core,
        &doc.sources,
        &doc.custom_props,
        &doc.footnote_separators,
        &doc.endnote_separators,
        &doc.glossary,
    ))
    .ok()?;
    let parts: Vec<_> =
        doc.parts.iter().map(|(id, p)| (*id, p.kind, p.body, p.blocks.iter().map(|b| Arc::as_ptr(b) as usize).collect::<Vec<_>>())).collect();
    let media: Vec<_> = doc.media.iter().chain(&doc.passthrough).map(|(k, v)| (k.as_str(), Arc::as_ptr(v) as usize, v.len())).collect();
    let opts = format!("{:?}", LayoutOptions { typing_word: None, ..opts.clone() });
    let held = (doc.parts.values().map(|p| p.blocks.clone()).collect(), doc.media.values().chain(doc.passthrough.values()).cloned().collect());
    Some((hash_of(&(env, rest, format!("{parts:?}"), media, opts)), held))
}

impl Track {
    /// Marks for a pass with a first pass's inputs (`first`) in Print Layout; none otherwise.
    pub(crate) fn new(doc: &Document, opts: &LayoutOptions, env: u64, first: bool) -> Track {
        let Some((inputs, held)) = (first && opts.view == ViewMode::Print).then(|| inputs(doc, opts, env)).flatten() else {
            return Track::default();
        };
        let sections = doc.sections().into_iter().map(|(end, s)| (end, s.clone())).collect();
        Track { on: true, inputs, held, sections, typing: opts.typing_word, revisions: doc.revisions.clone(), ..Default::default() }
    }

    pub(crate) fn on(&self) -> bool {
        self.on
    }

    /// Where this pass over `doc` (whose notes are numbered `notes`, the layout cache having noted
    /// `used` paragraph layouts in use) can start from the `earlier` one; `None`: from the first page.
    pub(crate) fn start(&mut self, earlier: Option<Box<Resume>>, doc: &Document, (notes, used): (&HashMap<u32, NoteNum>, usize)) -> Option<Start> {
        // Passes started again keep what the pages before them used in the cache, and so the
        // paragraph layouts of every edit since: a pass from the first page now and then lets
        // the cache forget those.
        // Tracked changes made since are added after those before, which keep their authors (and
        // colours) and moves: only the changed blocks can show the new ones. (A block naming one
        // that wasn't there yet takes it as a plain change either way, unless it is a move.)
        let earlier = earlier.filter(|e| {
            self.on
                && e.inputs == self.inputs
                && self.revisions.starts_with(&e.revisions)
                && self.revisions.get(e.revisions.len()..).is_some_and(|new| new.iter().all(|r| r.move_name.is_none()))
                && *e.notes == *notes
                && used <= e.used.saturating_mul(2).saturating_add(1024)
        })?;
        let (a, b) = (&earlier.body, &doc.body);
        let first = a.iter().zip(b).take_while(|(x, y)| Arc::ptr_eq(x, y)).count();
        let room = a.len().min(b.len()).saturating_sub(first);
        let last = a.iter().rev().zip(b.iter().rev()).take(room).take_while(|(x, y)| Arc::ptr_eq(x, y)).count();
        // The changed blocks of `doc`, and the paragraphs whose word being typed changed.
        let (mut from, mut to) = (first, b.len().saturating_sub(last));
        if earlier.typing != self.typing {
            for t in [earlier.typing, self.typing].into_iter().flatten() {
                let is = |blk: &Arc<Block>| matches!(&**blk, Block::Para(p) if p.rev == t.rev);
                // A revision shared by two paragraphs (one copied without a new one): both take the word.
                if b.iter().filter(|blk| is(blk)).count() > 1 {
                    return None;
                }
                let at = |body: &Blocks| body.iter().position(is);
                match (at(b), at(a)) {
                    (Some(i), _) => (from, to) = (from.min(i), to.max(i + 1)),
                    // Edited since: its block is one of the changed ones.
                    (None, Some(_)) => {}
                    // In a table or another story.
                    (None, None) => return None,
                }
            }
        }
        // The same sections, those after the change moved with it.
        let moved = |old: usize, new: usize| if old < from { new == old } else { new + a.len() == old + b.len() };
        let sections = doc.sections();
        if sections.len() != earlier.sections.len()
            || !sections.iter().zip(&earlier.sections).all(|((new, s), (old, t))| moved(*old, *new) && *s == t)
        {
            return None;
        }
        let shift = Shift { from: (to + a.len()).checked_sub(b.len())?, by: i64::try_from(b.len()).ok()? - i64::try_from(a.len()).ok()? };
        // The last mark before the change that nothing before looked ahead into; none (an edit on
        // the first page's first blocks): the pass starts from the first page, and may still join.
        let i = earlier.marks.iter().rposition(|m| m.block <= from && m.walk.ahead < from);
        self.used = Some(earlier.used);
        let Resume { mut marks, mut end, end_walk, .. } = *earlier;
        let base = match i {
            Some(i) => marks.get(i)?.pages.checked_sub(1)?,
            None => 0,
        };
        if base > end.pages.len() || base > end.behind.len() {
            return None;
        }
        let rest = marks.split_off(i.unwrap_or(0));
        let mut rest = rest.into_iter();
        let mark = if i.is_some() { Some(rest.next()?) } else { None };
        let (mut pages, mut behind) = (std::mem::take(&mut end.pages), std::mem::take(&mut end.behind));
        let (tail, tail_behind) = (pages.split_off(base), behind.split_off(base));
        self.marks = marks;
        self.earlier = Some(Earlier { marks: rest.collect(), pages: tail, behind: tail_behind, base, end, end_walk, after: to, shift });
        #[cfg(test)]
        STARTS.with(|s| s.set((s.get().0 + 1, s.get().1)));
        Some(Start { mark: mark?, pages, behind })
    }

    /// Whether this pass may join an earlier one's pages ([`Track::begin`]): it keeps what the
    /// earlier one used in the cache, as it lays out only part of the document.
    pub(crate) fn joining(&self) -> bool {
        self.earlier.is_some()
    }

    /// A block (`at`: its index, its section's and where the section's body starts) begins: the
    /// first on its page is marked. True when the walk is back in step with the earlier pass's
    /// there, so the pages after are that pass's.
    pub(crate) fn begin(
        &mut self,
        ctx: &Ctx,
        pb: &mut PageBuilder,
        at: (usize, usize, f32),
        lp: (&VecDeque<(u32, usize)>, usize, TextDirection),
    ) -> bool {
        if !self.on || pb.pages.len() == self.last || ctx.chains.any_laid_out() || ctx.boxes.inside() || ctx.alphas.decoded > 0 || ctx.pin.is_some() {
            return false;
        }
        // What headers and footers give the body (their tables' areas, text boxes sized to their
        // text) is worked out once per page, sometimes while a paragraph running on to the page sets
        // where its fields are: a pass started again might work it out otherwise. None: no marks. (Boxes
        // that keep their own size are noted in `fitted` without anything laid out.) Nor once a box
        // sized to its text could not be measured where it was met: a pass started again meets it
        // first elsewhere.
        if ctx.header_tables.values().any(|a| !a.is_empty())
            || ctx.footer_wraps.values().any(|a| !a.is_empty())
            || ctx.fitted.values().any(Option::is_some)
            || ctx.fitted_unmeasured
        {
            return false;
        }
        self.last = pb.pages.len();
        let Some(mark) = Mark::take(ctx, pb, at, lp) else { return false };
        if let Some(e) = &self.earlier
            && mark.block > e.after
            && let Some(k) = e.marks.iter().position(|m| m.block >= e.shift.from && e.shift.block(m.block) == mark.block)
            && e.marks.get(k).is_some_and(|m| mark.same(m, e.shift))
            && mark.pages.checked_sub(1 + e.base).is_some_and(|page| page < e.pages.len() && page < e.behind.len())
        {
            self.joined = Some((k, ctx.ahead.get()));
            return true;
        }
        self.marks.push(mark);
        false
    }

    /// A page was taken back (pages now `pages`): its marks go.
    pub(crate) fn taken_back(&mut self, pages: usize) {
        self.marks.retain(|m| m.pages <= pages);
        self.last = 0;
    }

    /// When the walk is back in step ([`Track::begin`]): the earlier pass's builder after the body,
    /// with the pages of `pb` before the page it joined on and the earlier ones from there; its
    /// walk then is put in `ctx`.
    pub(crate) fn join(&mut self, ctx: &mut Ctx, pb: &mut PageBuilder) -> Option<(PageBuilder<'static>, Loop)> {
        let (k, ahead) = self.joined?;
        let e = self.earlier.take()?;
        let page = e.marks.get(k)?.pages.checked_sub(1)?.checked_sub(e.base)?;
        let (mut pages, mut behind) = (std::mem::take(&mut pb.pages), std::mem::take(&mut pb.behind));
        let shift = e.shift;
        pages.truncate(e.base + page);
        behind.truncate(e.base + page);
        pages.extend(e.pages.into_iter().skip(page).map(|mut p| {
            shift.page(&mut p);
            p
        }));
        behind.extend(e.behind.into_iter().skip(page));
        let mut end = e.end;
        (end.pages, end.behind) = (pages, behind);
        self.marks.extend(e.marks.into_iter().skip(k).map(|m| {
            let mut m = m.shifted(shift);
            m.walk.ahead = m.walk.ahead.max(ahead);
            m
        }));
        let mut walk = e.end_walk;
        walk.shift(shift);
        #[cfg(test)]
        STARTS.with(|s| s.set((s.get().0, s.get().1 + 1)));
        Some((end, walk.put(ctx)))
    }

    /// Keep this pass for the next (`pb` after the body, `lp` the loop's state), when it marked.
    pub(crate) fn keep(self, ctx: &mut Ctx, pb: &PageBuilder, lp: (&VecDeque<(u32, usize)>, usize, TextDirection)) {
        if !self.on {
            return;
        }
        ctx.cache.resume = None;
        // Linked boxes laid out after the last mark are no hindrance: no pass starts or joins past them.
        if ctx.boxes.inside() || ctx.alphas.decoded > 0 {
            return;
        }
        let walk = Walk { fitted: ctx.fitted.clone(), fitted_unmeasured: ctx.fitted_unmeasured, ..Walk::take(ctx, lp) };
        ctx.cache.resume = Some(Box::new(Resume {
            inputs: self.inputs,
            _held: self.held,
            body: ctx.doc.body.clone(),
            sections: self.sections,
            notes: Arc::clone(&ctx.fields.notes),
            typing: self.typing,
            revisions: self.revisions,
            used: self.used.unwrap_or(ctx.cache.used.len()),
            marks: self.marks,
            end: pb.clone().with_refs(&NO_SECT, &NO_EARLY),
            end_walk: walk,
        }));
    }
}
