//! Incremental pagination (#554): typing in a long document re-paginates from the page the edit
//! is on, not from the first page, and stops as soon as the pages after the edit come out as they
//! did before.
//!
//! Each pass records the paging state at the first block boundary of every page (a
//! [`Checkpoint`]) and the state after the last block, with the body blocks it laid out (shared
//! `Arc`s, so an edited block is a different pointer: holding them keeps the document from
//! changing a block in place). The next pass, if nothing outside the body changed, resumes from
//! the last checkpoint before the first changed block, with the pages before it as they were.
//! Past the edit, at each page's first block boundary, it compares its state and the page so far
//! with the previous pass's at the same block: when they match, the rest of the pages are the
//! previous pass's, and the walk stops.

use std::sync::Arc;

use wordcraft_doc::numbering::Counters;
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::section::{PageVAlign, SectionProps};
use wordcraft_doc::{Block, Document};
use wordcraft_geom::Rect;

use crate::cache::hash_of;
use crate::ctx::Ctx;
use crate::notes::NoteBox;
use crate::page::PageBuilder;
use crate::{LayoutOptions, Page, Placed, ViewMode};

/// The paging state between two body blocks: everything placing the following blocks reads
/// besides the document and the caches.
#[derive(Clone)]
pub(crate) struct PageState {
    /// The next block to place, and its section's index in `Document::sections()`.
    pub block: usize,
    pub si: usize,
    col: usize,
    cols: Vec<(f32, f32)>,
    y: f32,
    top: f32,
    bottom: f32,
    number: u32,
    notes: Vec<NoteBox>,
    carry: Vec<NoteBox>,
    orig_bottom: f32,
    prev: Option<(String, bool, f32, bool, bool)>,
    after_break_mark: bool,
    excl: Vec<crate::floats::WrapArea>,
    float_tables: Vec<Rect>,
    line_no: u32,
    page_items_start: usize,
    counters: Counters,
    eq_count: u32,
    field_page: u32,
    page_format: NumFormat,
    boxes: usize,
    max_y: f32,
    content_bottom: f32,
    beneath: bool,
    col_break_page: Option<usize>,
    restart_at: Option<u32>,
    next_starts_page: bool,
    /// The current page's block marks, items behind its text and vertical alignment.
    block_marks: Vec<usize>,
    behind: usize,
    page_valign: PageVAlign,
    /// Where the paragraph before ended (page, column, y): a bordered group draws up to it.
    para_end: Option<(usize, usize, f32)>,
}

fn note_sig(n: &[NoteBox]) -> Vec<(u32, u32, bool, usize)> {
    n.iter().map(|n| (n.id, n.h.to_bits(), n.cont, n.items.len())).collect()
}

impl PageState {
    pub fn take(pb: &PageBuilder, ctx: &Ctx, block: usize, si: usize) -> PageState {
        PageState {
            block,
            si,
            col: pb.col,
            cols: pb.cols.clone(),
            y: pb.y,
            top: pb.top,
            bottom: pb.bottom,
            number: pb.number,
            notes: pb.notes.clone(),
            carry: pb.carry.clone(),
            orig_bottom: pb.orig_bottom,
            prev: pb.prev.clone(),
            after_break_mark: pb.after_break_mark,
            excl: pb.excl.clone(),
            float_tables: pb.float_tables.clone(),
            line_no: pb.line_no,
            page_items_start: pb.page_items_start,
            counters: ctx.counters.clone(),
            eq_count: ctx.eq_count,
            field_page: ctx.fields.page,
            page_format: ctx.fields.page_format,
            boxes: ctx.boxes.total(),
            max_y: pb.max_y,
            content_bottom: pb.content_bottom,
            beneath: pb.beneath,
            col_break_page: pb.col_break_page,
            restart_at: pb.restart_at,
            next_starts_page: pb.next_starts_page,
            block_marks: pb.block_marks.clone(),
            behind: pb.behind.last().copied().unwrap_or(0),
            page_valign: pb.page_valign,
            para_end: pb.para_end,
        }
    }

    /// Put this state back into `pb` and `ctx` (the pages are the caller's).
    pub fn restore(&self, pb: &mut PageBuilder, ctx: &mut Ctx) {
        pb.sect_idx = self.si;
        pb.col = self.col;
        pb.cols = self.cols.clone();
        pb.y = self.y;
        pb.top = self.top;
        pb.bottom = self.bottom;
        pb.number = self.number;
        pb.notes = self.notes.clone();
        pb.carry = self.carry.clone();
        pb.orig_bottom = self.orig_bottom;
        pb.prev = self.prev.clone();
        pb.after_break_mark = self.after_break_mark;
        pb.excl = self.excl.clone();
        pb.float_tables = self.float_tables.clone();
        pb.line_no = self.line_no;
        pb.page_items_start = self.page_items_start;
        ctx.counters = self.counters.clone();
        ctx.eq_count = self.eq_count;
        ctx.fields.page = self.field_page;
        ctx.fields.page_format = self.page_format;
        ctx.boxes = wordcraft_doc::BoxBudget::counted(self.boxes);
        pb.max_y = self.max_y;
        pb.content_bottom = self.content_bottom;
        pb.beneath = self.beneath;
        pb.col_break_page = self.col_break_page;
        pb.restart_at = self.restart_at;
        pb.next_starts_page = self.next_starts_page;
        pb.block_marks.clone_from(&self.block_marks);
        if let Some(b) = pb.behind.last_mut() {
            *b = self.behind;
        }
        pb.page_valign = self.page_valign;
        pb.para_end = self.para_end;
    }

    /// Whether placing the same blocks from here gives the same pages.
    fn same(&self, o: &PageState) -> bool {
        let f = |a: f32, b: f32| a.to_bits() == b.to_bits();
        self.block == o.block
            && self.si == o.si
            && self.col == o.col
            && self.cols.len() == o.cols.len()
            && self.cols.iter().zip(&o.cols).all(|(a, b)| f(a.0, b.0) && f(a.1, b.1))
            && f(self.y, o.y)
            && f(self.top, o.top)
            && f(self.bottom, o.bottom)
            && self.number == o.number
            && note_sig(&self.notes) == note_sig(&o.notes)
            && note_sig(&self.carry) == note_sig(&o.carry)
            && f(self.orig_bottom, o.orig_bottom)
            && self.prev.as_ref().map(|(s, c, a, b0, b1)| (s, c, a.to_bits(), b0, b1))
                == o.prev.as_ref().map(|(s, c, a, b0, b1)| (s, c, a.to_bits(), b0, b1))
            && self.after_break_mark == o.after_break_mark
            && self.excl == o.excl
            && self.float_tables == o.float_tables
            && self.line_no == o.line_no
            && self.page_items_start == o.page_items_start
            && self.eq_count == o.eq_count
            && self.field_page == o.field_page
            && self.page_format == o.page_format
            && self.boxes == o.boxes
            && f(self.max_y, o.max_y)
            && f(self.content_bottom, o.content_bottom)
            && self.beneath == o.beneath
            && self.col_break_page == o.col_break_page
            && self.restart_at == o.restart_at
            && self.next_starts_page == o.next_starts_page
            && self.block_marks == o.block_marks
            && self.behind == o.behind
            && self.page_valign == o.page_valign
            && self.para_end.map(|(p, c, y)| (p, c, y.to_bits())) == o.para_end.map(|(p, c, y)| (p, c, y.to_bits()))
            && format!("{:?}", self.counters) == format!("{:?}", o.counters)
    }
}

/// Two pages so far hold the same items. Lines compare by their paragraph layout's identity
/// (unchanged paragraphs come from the cache, so the same `Arc`), everything else by value.
fn same_page(a: &Page, b: &Page) -> bool {
    a.section == b.section
        && a.number == b.number
        && a.first_block == b.first_block
        && a.items.len() == b.items.len()
        && a.items.iter().zip(&b.items).all(|(x, y)| match (x, y) {
            (
                Placed::Lines { story, path, para, l0, l1, x, y, turn },
                Placed::Lines { story: s2, path: p2, para: q2, l0: a2, l1: b2, x: x2, y: y2, turn: t2 },
            ) => {
                story == s2
                    && path == p2
                    && Arc::ptr_eq(para, q2)
                    && (l0, l1) == (a2, b2)
                    && x.to_bits() == x2.to_bits()
                    && y.to_bits() == y2.to_bits()
                    && turn == t2
            }
            (Placed::Lines { .. }, _) | (_, Placed::Lines { .. }) => false,
            _ => format!("{x:?}") == format!("{y:?}"),
        })
}

/// The state at the first block boundary of a page.
#[derive(Clone)]
pub(crate) struct Checkpoint {
    pub state: PageState,
    /// Pages so far, this one included.
    pub pages: usize,
    /// This page as it was then.
    pub page: Page,
}

/// What a pass leaves for the next one.
pub(crate) struct Pass {
    key: PassKey,
    body: Vec<Arc<Block>>,
    sections: Vec<(usize, SectionProps)>,
    pub checkpoints: Vec<Checkpoint>,
    /// The state and pages after the last block (before endnotes, headers and footers).
    pub end: PageState,
    pub end_pages: Vec<Page>,
    /// Per page of `end_pages`: how many of its items are behind the text.
    pub end_behind: Vec<usize>,
    /// Whether its body shows page totals (`NUMPAGES`, `SECTIONPAGES`): a later pass can't resume.
    pub uses_totals: bool,
}

/// Everything outside the body blocks a pass depends on.
pub(crate) struct PassKey {
    view: ViewMode,
    opts: u64,
    /// The word being typed: not part of `opts` (it changes with every key); [`Pass::plan`]
    /// lays its paragraph out again.
    typing: Option<crate::TypingWord>,
    env: u64,
    notes: u64,
    context: u64,
    /// The blocks of every header, footer, note and text box, held so that none is freed (or
    /// edited in place) while the pass is kept: an address seen again is the same block.
    parts: Vec<(u32, Vec<Arc<Block>>)>,
    /// The blocks of the document's footnote and endnote separators and continuation notices
    /// (`None`: the default), held the same way.
    separators: Vec<Option<Vec<Arc<Block>>>>,
}

impl PartialEq for PassKey {
    fn eq(&self, o: &PassKey) -> bool {
        self.view == o.view
            && self.opts == o.opts
            && self.env == o.env
            && self.notes == o.notes
            && self.context == o.context
            && self.parts.len() == o.parts.len()
            && self.parts.iter().zip(&o.parts).all(|((a, x), (b, y))| a == b && x.len() == y.len() && x.iter().zip(y).all(|(p, q)| Arc::ptr_eq(p, q)))
            && self.separators.len() == o.separators.len()
            && self.separators.iter().zip(&o.separators).all(|(x, y)| match (x, y) {
                (Some(x), Some(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| Arc::ptr_eq(p, q)),
                (None, None) => true,
                _ => false,
            })
    }
}

impl PassKey {
    pub fn new(doc: &Document, opts: &LayoutOptions, env: u64, notes: u64) -> PassKey {
        let context = serde_json::to_string(&(
            &doc.settings,
            &doc.core.title,
            &doc.core.creator,
            &doc.revisions,
            &doc.sources,
            doc.comments.keys().collect::<Vec<_>>(),
            doc.media.keys().collect::<Vec<_>>(),
        ))
        .map(|s| hash_of(&s))
        .unwrap_or(0);
        // Part blocks are compared by identity, like the body's (the key keeps them alive).
        let parts = doc.parts.iter().map(|(id, p)| (*id, p.blocks.clone())).collect();
        // So are the note separators'.
        let separators = [&doc.footnote_separators, &doc.endnote_separators]
            .into_iter()
            .flat_map(|s| [&s.separator, &s.continuation_separator, &s.continuation_notice])
            .cloned()
            .collect();
        PassKey {
            view: opts.view,
            opts: hash_of(&format!("{:?}", LayoutOptions { typing_word: None, ..opts.clone() })),
            typing: opts.typing_word,
            env,
            notes,
            context,
            parts,
            separators,
        }
    }
}

/// How the next pass may reuse the previous one.
pub(crate) struct Plan {
    /// Resume from this checkpoint of the previous pass.
    pub from: Option<Checkpoint>,
    /// Pages before the checkpoint's page.
    pub pages: Vec<Page>,
    /// The first block of the unchanged tail; from the first page boundary in it whose state matches,
    /// the rest is the previous pass's (`None`: the block count changed, or the sections did).
    pub converge_after: Option<usize>,
}

impl Pass {
    pub fn new(key: PassKey, doc: &Document, sections: &[(usize, &SectionProps)]) -> Pass {
        Pass {
            key,
            body: doc.body.to_vec(),
            sections: sections.iter().map(|(e, s)| (*e, (*s).clone())).collect(),
            checkpoints: Vec::new(),
            end: PageState::default_for_end(),
            end_pages: Vec::new(),
            end_behind: Vec::new(),
            uses_totals: false,
        }
    }

    /// How to lay out `doc` reusing this pass, or `None` for a full layout.
    pub fn plan(&self, key: &PassKey, doc: &Document, sections: &[(usize, &SectionProps)]) -> Option<Plan> {
        if *key != self.key || key.view != ViewMode::Print {
            return None;
        }
        let same = |a: &Arc<Block>, b: &Arc<Block>| Arc::ptr_eq(a, b);
        let mut first_diff = self.body.iter().zip(doc.body.iter()).position(|(a, b)| !same(a, b)).unwrap_or(self.body.len().min(doc.body.len()));
        let mut dirty_to = 0;
        if key.typing != self.key.typing {
            // The paragraphs with the word being typed, before and after, are laid out again
            // (their squiggles differ). One that isn't a top-level paragraph: a full layout.
            for (typing, body) in [(self.key.typing, &self.body), (key.typing, &doc.body)] {
                let Some(t) = typing else { continue };
                let at = body.iter().position(|b| matches!(&**b, Block::Para(p) if p.rev == t.rev))?;
                first_diff = first_diff.min(at);
                dirty_to = dirty_to.max(at + 1);
            }
        }
        // The last checkpoint strictly before the first changed block (the block before it may
        // keep with it, so it is placed again), whose section still has the same settings.
        let sect_same = |i: usize, si: usize| match (sections.get(i), self.sections.get(i)) {
            (Some((e, s)), Some((oe, os))) => (i == si || e == oe) && *s == os,
            _ => false,
        };
        let cp = self.checkpoints.iter().rev().find(|c| c.state.block < first_diff).filter(|c| (0..=c.state.si).all(|i| sect_same(i, c.state.si)));
        let pages = match cp {
            Some(cp) => self.end_pages.get(..cp.pages.saturating_sub(1))?.to_vec(),
            None => Vec::new(),
        };
        let all_sections = sections.len() == self.sections.len()
            && (0..sections.len()).all(|i| match (sections.get(i), self.sections.get(i)) {
                (Some((e, s)), Some((oe, os))) => e == oe && *s == os,
                _ => false,
            });
        let converge_after = (doc.body.len() == self.body.len() && all_sections).then(|| {
            // The first block of the unchanged tail.
            let tail = self.body.iter().rev().zip(doc.body.iter().rev()).take_while(|(a, b)| same(a, b)).count();
            (doc.body.len() - tail).max(dirty_to)
        });
        Some(Plan { from: cp.cloned(), pages, converge_after })
    }

    /// Whether the previous pass had this state, these pages and this much of the current page
    /// at `st.block`: placing the same blocks from there gives the same pages.
    pub fn converges(&self, st: &PageState, pages: usize, page: &Page) -> bool {
        let Ok(i) = self.checkpoints.binary_search_by(|c| c.state.block.cmp(&st.block)) else { return false };
        self.checkpoints.get(i).is_some_and(|c| c.pages == pages && c.state.same(st) && same_page(&c.page, page))
    }
}

impl PageState {
    fn default_for_end() -> PageState {
        PageState {
            block: 0,
            si: 0,
            col: 0,
            cols: Vec::new(),
            y: 0.0,
            top: 0.0,
            bottom: 0.0,
            number: 0,
            notes: Vec::new(),
            carry: Vec::new(),
            orig_bottom: 0.0,
            prev: None,
            after_break_mark: false,
            excl: Vec::new(),
            float_tables: Vec::new(),
            line_no: 0,
            page_items_start: 0,
            counters: Counters::default(),
            eq_count: 0,
            field_page: 0,
            page_format: NumFormat::default(),
            boxes: 0,
            max_y: 0.0,
            content_bottom: 0.0,
            beneath: false,
            col_break_page: None,
            restart_at: None,
            next_starts_page: false,
            block_marks: Vec::new(),
            behind: 0,
            page_valign: PageVAlign::default(),
            para_end: None,
        }
    }
}
