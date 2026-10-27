use super::cache::{LayoutCache, Totals, env_hash, hash_of};
use super::ctx::{Ctx, Pass, with_pct_base};
use super::floats::{EarlyFloats, WrapArea, early_covered};
use super::flow::{OutlinePass, place_outline_para, place_para};
use super::headers::{HfLinks, bookmark_pages, headers_footers, headers_footers_refer_to_pages, marks_key, mirror_even_pages, section_side};
use super::notes::{MAX_NOTE_PAGES, NoteSeps, note_info, place_endnotes};
use super::page::{PageBuilder, body_top_for, foot_top_for, item_bottom};
use super::placed::Placed;
use super::table_flow::place_table;
use super::*;

/// One layout pass, numbering footnotes restarted on each page by `note_pages` ([`note_info`]),
/// with `totals` assumed for the page totals in the body. With `may_repeat`, a pass whose body
/// shows totals that turn out different asks to be done again. Pages start with the wrap areas
/// of the `early` floats; with `may_rewrap`, a pass that finds others asks to be done again.
pub(crate) fn layout_pass(
    doc: &Document,
    cache: &mut LayoutCache,
    opts: &LayoutOptions,
    note_pages: &HashMap<u32, usize>,
    totals: &Totals,
    may_repeat: bool,
    (early, may_rewrap): (&EarlyFloats, bool),
) -> Pass {
    let env = env_hash(doc, opts);
    if env != cache.env {
        cache.paras.clear();
        cache.measures.clear();
        cache.env = env;
    }
    cache.used.clear();
    // Note references are counted as the body shows them.
    let notes = note_info(doc, opts.hides_deleted(StoryRef::Body), note_pages);
    let notes_hash = hash_of(&{
        let mut v: Vec<_> = notes.iter().map(|(a, b)| (*a, b.n, b.fmt)).collect();
        v.sort_by_key(|(a, ..)| *a);
        v
    });
    let mut ctx = Ctx {
        doc,
        cache,
        opts,
        hide_deleted: opts.hides_deleted(StoryRef::Body),
        counters: Counters::default(),
        fields: FieldCtx {
            page: 1,
            pages: totals.pages,
            section_pages: totals.section(0),
            section: 1,
            notes: Arc::new(notes),
            title: doc.core.title.as_str().into(),
            author: doc.core.creator.as_str().into(),
            filename: opts.filename.clone().unwrap_or_else(|| "".into()),
            filepath: opts.filepath.clone().unwrap_or_else(|| "".into()),
            ..Default::default()
        },
        notes_hash,
        numbers: HashMap::new(),
        eq_count: 0,
        boxes: wordcraft_doc::BoxBudget::default(),
        chains: Default::default(),
        grid: para::Grid::default(),
        uses_totals: false,
        track_totals: true,
        pct_base: None,
        quiet_marks: Vec::new(),
        mirror: false,
        table_keys: Vec::new(),
        fitted: HashMap::new(),
        header_tables: HashMap::new(),
        footer_wraps: HashMap::new(),
        pin: None,
        floats_above: HashMap::new(),
        hf: Arc::new(HfLinks::new(doc.sections())),
        alphas: Default::default(),
    };
    let hf = Arc::clone(&ctx.hf);
    let sections = &hf.sections;
    let web = matches!(opts.view, ViewMode::Web | ViewMode::Draft | ViewMode::Outline);
    let read = opts.view == ViewMode::Read;
    let reflow = web || read;
    ctx.mirror = doc.settings.mirror_margins && !reflow;
    ctx.quiet_marks = sections
        .windows(2)
        .filter(|w| w[1].1.start == SectionStart::Continuous)
        .map(|w| w[0].0)
        .filter(|end| doc.body.get(*end).and_then(|b| b.as_para()).is_some_and(|p| p.text.is_empty()))
        .collect();
    if !opts.show_hidden {
        ctx.quiet_marks.extend(
            doc.body
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.as_para().filter(|p| p.mark.hidden == Some(true) && p.plain_text().is_empty()).map(|_| i)),
        );
    }
    ctx.quiet_marks.sort_unstable();
    ctx.quiet_marks.dedup();
    let outline = (opts.view == ViewMode::Outline).then(|| OutlinePass::new(doc, opts.outline.clone().unwrap_or_default()));
    let default_sect = SectionProps::default();
    let mut web_sect;
    let first_sect = match sections.first() {
        Some((_, s)) => *s,
        None => &default_sect,
    };
    let sect_ref = if reflow {
        web_sect = first_sect.clone();
        web_sect.page_w = if opts.web_width.is_finite() { opts.web_width.clamp(144.0, 20_000.0) } else { 300.0 };
        if read {
            web_sect.page_h = if opts.read_height.is_finite() { opts.read_height.clamp(144.0, 20_000.0) } else { 500.0 };
            web_sect.margin_bottom = 18.0;
            web_sect.page_borders = None;
            web_sect.line_numbers = None;
        }
        web_sect.margin_left = 18.0;
        web_sect.margin_right = 18.0;
        web_sect.margin_top = 18.0;
        web_sect.gutter = 0.0;
        web_sect.columns = Default::default();
        web_sect.valign = wordcraft_doc::section::PageVAlign::Top;
        &web_sect
    } else {
        first_sect
    };
    let mut pb = PageBuilder {
        pages: Vec::new(),
        sect: sect_ref,
        sect_idx: 0,
        col: 0,
        cols: Vec::new(),
        y: 0.0,
        top: 0.0,
        bottom: 0.0,
        number: 0,
        web,
        notes: Vec::new(),
        carry: Vec::new(),
        orig_bottom: 0.0,
        prev: None,
        after_break_mark: false,
        next_starts_page: false,
        restart_at: None,
        excl: Vec::new(),
        float_tables: Vec::new(),
        line_no: 0,
        page_items_start: 0,
        block_marks: Vec::new(),
        page_valign: wordcraft_doc::section::PageVAlign::Top,
        seps: NoteSeps::default(),
        para_end: None,
        behind: Vec::new(),
        max_y: 0.0,
        content_bottom: 0.0,
        balance: None,
        col_break_page: None,
        sect_beneath: false,
        beneath: false,
        foot_top: f32::MAX,
        early,
        seeded: 0,
        settle: !may_rewrap && !reflow,
    };
    // Endnotes in document order, with the section of their reference; placed at the end of a
    // section that keeps them there, else at the end of the document.
    let mut endnotes: std::collections::VecDeque<(u32, usize)> = {
        let mut ids: Vec<(u32, u32, usize)> = ctx
            .fields
            .notes
            .iter()
            .filter(|(id, _)| doc.parts.get(id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::Endnote))
            .map(|(id, n)| (n.order, *id, n.section))
            .collect();
        ids.sort();
        ids.into_iter().map(|(_, id, sect)| (id, sect)).collect()
    };
    // Text boxes laid out in balancing attempts that were thrown away.
    let mut boxes_discarded = 0usize;
    let mut block = 0usize;
    // Vertical sections are laid out on their pages turned on their side (see `vertical`).
    let turned: Vec<Option<SectionProps>> = sections
        .iter()
        .map(|(_, s)| {
            (!web && s.text_direction.is_turned()).then(|| {
                let top = body_top_for(&mut ctx, s, s.headers.default);
                vertical::unturned_sect(s, top)
            })
        })
        .collect();
    let mut prev_turn = TextDirection::Horizontal;
    for (si, (end, sect)) in sections.iter().enumerate() {
        let turn = if reflow { TextDirection::Horizontal } else { sect.text_direction };
        let sect: &SectionProps = match turned.get(si) {
            Some(Some(t)) => t,
            _ if reflow => sect_ref,
            _ => sect,
        };
        pb.sect = sect;
        pb.sect_idx = si;
        ctx.grid = para::Grid::default();
        let (foot, end_opts) = (doc.note_options(sect, false), doc.note_options(sect, true));
        pb.sect_beneath = foot.pos == wordcraft_doc::section::NotePos::BeneathText;
        if !web && !doc.footnote_separators.is_empty() {
            pb.seps = NoteSeps::new(&mut ctx, &doc.footnote_separators, sect.text_width());
        }
        ctx.fields.section = u32::try_from(si).unwrap_or(u32::MAX).saturating_add(1);
        ctx.fields.section_pages = totals.section(si);
        // Headers and footers left out are linked to the previous section's, and take its room.
        let (heads, feet) = (hf.heads.set(si), hf.feet.set(si));
        let hb = |i: Option<u32>| hf.defining_width(false, si, i);
        let fb = |i: Option<u32>| hf.defining_width(true, si, i);
        let body_top = if reflow || turn.is_turned() {
            sect.margin_top
        } else {
            with_pct_base(&mut ctx, hb(heads.default), |c| body_top_for(c, sect, heads.default))
        };
        let first_top = if reflow || turn.is_turned() || !sect.title_page {
            body_top
        } else {
            with_pct_base(&mut ctx, hb(heads.first), |c| body_top_for(c, sect, heads.first))
        };
        let foot_top =
            if reflow || turn.is_turned() { f32::MAX } else { with_pct_base(&mut ctx, fb(feet.default), |c| foot_top_for(c, sect, feet.default)) };
        let first_foot = if reflow || turn.is_turned() || !sect.title_page {
            foot_top
        } else {
            with_pct_base(&mut ctx, fb(feet.first), |c| foot_top_for(c, sect, feet.first))
        };
        ctx.grid = para::Grid::of(doc, sect);
        let restart = sect.page_num_start;
        if sect.line_numbers.as_ref().is_some_and(|l| l.restart == wordcraft_doc::section::LineNumberRestart::Section) {
            pb.line_no = 0;
        }
        let start = if si == 0 { SectionStart::NextPage } else { sect.start };
        if web && si > 0 {
            // one long page
        } else if pb.pages.is_empty() || start != SectionStart::Continuous || turn != prev_turn {
            // A page holds text running one way: a section turning the other way starts a page.
            if !web {
                pb.take_back_break_page();
            }
            let s = &doc.settings;
            let (filler, restart) = section_side(start, restart, pb.number, pb.pages.len(), s.even_odd_headers, s.mirror_margins);
            // The blank page that puts the section on the right side ends the previous section.
            if filler
                && !web
                && let Some((_, prev)) = si.checked_sub(1).and_then(|k| sections.get(k))
            {
                pb.sect = prev;
                pb.sect_idx = si - 1;
                let (ph, pf) = (hf.heads.set(si - 1), hf.feet.set(si - 1));
                let top = body_top_for(&mut ctx, prev, ph.default);
                pb.foot_top = foot_top_for(&mut ctx, prev, pf.default);
                pb.new_page(block, top);
                pb.sect = sect;
                pb.sect_idx = si;
            }
            if restart.is_some() {
                pb.restart_at = restart;
            }
            pb.foot_top = first_foot;
            pb.new_page(block, first_top);
            pb.foot_top = foot_top;
        } else {
            // Continuous: new column layout below the current text (its tallest column).
            pb.cols = sect.column_boxes();
            pb.col = 0;
            pb.top = pb.text_bottom();
            pb.y = pb.top;
            pb.max_y = pb.top;
        }
        prev_turn = turn;
        pb.foot_top = foot_top;
        let last = (*end).min(doc.body.len().saturating_sub(1));
        // Newspaper columns are balanced on the section's last page when the next section starts
        // on the same page (a continuous break), as in Word. That page is laid out again from the
        // start of the block that reaches it, with shorter columns (see `balance_columns`).
        let balance = !web
            && pb.cols.len() > 1
            && sect.valign == wordcraft_doc::section::PageVAlign::Top
            && sections.get(si + 1).is_some_and(|(_, s)| s.start == SectionStart::Continuous);
        pb.col_break_page = None;
        pb.next_starts_page = !web && sections.get(si + 1).is_some_and(|(_, s)| s.start != SectionStart::Continuous);
        let mut from = balance.then(|| (block, Snap::take(&mut pb, &ctx)));
        while block <= last {
            let Some(b) = doc.body.get(block) else { break };
            let before = balance.then(|| (pb.pages.len(), Snap::take(&mut pb, &ctx)));
            pb.mark_block();
            match (&**b, &outline) {
                (Block::Para(p), Some(o)) => place_outline_para(&mut ctx, &mut pb, p, block, o),
                (Block::Table(_), Some(o)) if o.hidden.get(block).copied().unwrap_or(false) => {}
                (Block::Para(p), None) => place_para(&mut ctx, &mut pb, p, block, body_top),
                (Block::Table(t), _) => place_table(&mut ctx, &mut pb, t, block, body_top),
            }
            if let Some((pages, snap)) = before
                && pb.pages.len() != pages
            {
                from = Some((block, snap));
            }
            block += 1;
        }
        if let Some((first, snap)) = from {
            boxes_discarded += balance_columns(&mut ctx, &mut pb, (first, block), &snap, body_top);
        }
        // Endnotes at the end of the section: those referenced so far.
        if end_opts.pos == wordcraft_doc::section::NotePos::SectEnd && !pb.pages.is_empty() {
            let mut here = Vec::new();
            while let Some((id, _)) = endnotes.front().filter(|(_, s)| *s <= si).copied() {
                endnotes.pop_front();
                here.push(id);
            }
            place_endnotes(&mut ctx, &mut pb, here, block.saturating_sub(1));
        }
    }
    if pb.pages.is_empty() {
        pb.new_page(0, sect_ref.margin_top);
    }
    // The rest of the endnotes after the last paragraph.
    let rest: Vec<u32> = endnotes.into_iter().map(|(id, _)| id).collect();
    place_endnotes(&mut ctx, &mut pb, rest, block);
    pb.apply_valign();
    pb.flush_notes();
    // Footnotes still continuing after the last page get pages of their own. Each page takes at
    // least a line of the first note, so this ends; the cap is a backstop.
    let last_block = doc.body.len().saturating_sub(1);
    for _ in 0..MAX_NOTE_PAGES {
        if pb.carry.is_empty() {
            break;
        }
        let top = pb.top;
        pb.new_page(last_block, top);
        pb.flush_notes();
    }
    pb.carry.clear();
    let mut pages = pb.pages;
    for p in &mut pages {
        if let Some((_, s)) = sections.get(p.section)
            && !web
        {
            vertical::turn_page(p, s.text_direction, s);
        }
    }
    if web && let Some(p) = pages.first_mut() {
        let bottom = p
            .items
            .iter()
            .filter_map(|it| if let Placed::Lines { y, para, l0, l1, .. } = it { item_bottom(*y, para, *l0, *l1) } else { None })
            .fold(0.0f32, f32::max);
        p.h = bottom + 36.0;
    }
    ctx.grid = para::Grid::default();
    if !reflow && may_rewrap {
        let mut above: Vec<((usize, usize), usize, WrapArea)> =
            ctx.floats_above.drain().filter_map(|(k, v)| v.map(|(page, area)| (k, page, area))).collect();
        above.sort_by_key(|(k, ..)| *k);
        let mut found = EarlyFloats::new();
        for ((block, oi), page, area) in above {
            found.entry(page).or_default().push((block, oi, area));
        }
        if !early_covered(&found, early) {
            return Pass::Rewrap(found);
        }
    }
    if !reflow {
        let found = Totals::of(&pages, sections);
        if ctx.uses_totals && may_repeat && found != *totals {
            return Pass::Again(found);
        }
        ctx.track_totals = false;
        // Page references in headers and footers are filled in after pagination, like their page
        // totals: they don't make the body lay out again.
        if headers_footers_refer_to_pages(doc) {
            let marks = bookmark_pages(doc, &pages, sections);
            ctx.fields.bookmarks_key = marks_key(&marks);
            ctx.fields.bookmark_pages = Arc::new(marks);
        }
        headers_footers(&mut ctx, &mut pages, sections, &found);
        if doc.settings.mirror_margins {
            mirror_even_pages(&mut pages, sections);
        }
    }
    // Evict paragraph layouts that weren't used this pass if the cache grew large.
    if ctx.cache.paras.len() + ctx.cache.measures.len() > 4 * ctx.cache.used.len().max(1024) {
        let used = std::mem::take(&mut ctx.cache.used);
        ctx.cache.paras.retain(|k, _| used.contains(k));
        ctx.cache.measures.retain(|k, _| used.contains(k));
        ctx.cache.used = used;
    }
    let mut index: HashMap<(StoryRef, Path), Vec<(usize, usize)>> = HashMap::new();
    for (pi, p) in pages.iter().enumerate() {
        for (ii, it) in p.items.iter().enumerate() {
            if let Placed::Lines { story, path, .. } = it {
                index.entry((*story, path.clone())).or_default().push((pi, ii));
            }
        }
    }
    let mut lay = DocLayout { pages, index, ms: 0.0, text_boxes: 0 };
    if opts.comment_balloons && !web {
        balloons::add(&mut ctx, &mut lay);
    }
    lay.text_boxes = ctx.boxes.total().saturating_sub(boxes_discarded);
    Pass::Done(lay)
}

/// Pagination state saved before a block, to lay the blocks from there out again (balancing
/// columns): the builder without its pages, how far its last page had got, and the walk state.
struct Snap<'a> {
    pb: PageBuilder<'a>,
    pages: usize,
    items: usize,
    behind: usize,
    counters: Counters,
    eq_count: u32,
    page_field: u32,
    boxes: usize,
}

impl<'a> Snap<'a> {
    fn take(pb: &mut PageBuilder<'a>, ctx: &Ctx) -> Self {
        // The pages stay out of the copy: it would cost the whole document per block.
        let (pages, behind) = (std::mem::take(&mut pb.pages), std::mem::take(&mut pb.behind));
        let copy = pb.clone();
        (pb.pages, pb.behind) = (pages, behind);
        Snap {
            pb: copy,
            pages: pb.pages.len(),
            items: pb.pages.last().map_or(0, |p| p.items.len()),
            behind: pb.behind.last().copied().unwrap_or(0),
            counters: ctx.counters.clone(),
            eq_count: ctx.eq_count,
            page_field: ctx.fields.page,
            boxes: ctx.boxes.total(),
        }
    }

    /// Go back to this state: drop the pages added since and what was added to the last one
    /// (appended, or inserted at the front behind the text).
    fn restore(&self, pb: &mut PageBuilder<'a>, ctx: &mut Ctx) {
        let (mut pages, mut behind) = (std::mem::take(&mut pb.pages), std::mem::take(&mut pb.behind));
        pages.truncate(self.pages);
        behind.truncate(self.pages);
        if let Some(pg) = pages.last_mut() {
            let k = behind.last().copied().unwrap_or(0).saturating_sub(self.behind).min(pg.items.len());
            pg.items.drain(..k);
            pg.items.truncate(self.items);
        }
        if let Some(b) = behind.last_mut() {
            *b = self.behind;
        }
        *pb = self.pb.clone();
        (pb.pages, pb.behind) = (pages, behind);
        ctx.counters = self.counters.clone();
        ctx.eq_count = self.eq_count;
        ctx.fields.page = self.page_field;
    }
}

/// One balancing pass: blocks `first..end` laid out again from `snap` with the columns on `page`
/// ending `h` below `top`; true if the page's text fits them.
fn balance_pass<'a>(
    ctx: &mut Ctx,
    pb: &mut PageBuilder<'a>,
    snap: &Snap<'a>,
    (first, end): (usize, usize),
    (page, top, h): (usize, f32, f32),
    body_top: f32,
) -> bool {
    snap.restore(pb, ctx);
    pb.balance = Some((page, h));
    pb.cap_columns();
    for block in first..end {
        match ctx.doc.body.get(block).map(|b| &**b) {
            Some(Block::Para(p)) => place_para(ctx, pb, p, block, body_top),
            Some(Block::Table(t)) => place_table(ctx, pb, t, block, body_top),
            None => break,
        }
        // Spilled onto another page: too short (and no need to lay out the rest).
        if pb.pages.len() > page + 1 {
            break;
        }
    }
    pb.balance = None;
    pb.cap_columns();
    pb.pages.len() == page + 1 && pb.content_bottom <= top + h + 0.5
}

/// Balancing lays out at most this many pages again per pass (a block reaching the last page
/// from further back is left unbalanced), so its cost stays a small multiple of one page.
const BALANCE_MAX_PAGES: usize = 16;

/// Passes of the binary search on the column height (enough for well under a point).
const BALANCE_PASSES: usize = 12;

/// Balance the columns on the last page of a section followed by a continuous break, as Word
/// does: lay blocks `first..end` out again from `snap` (taken before `first`) with the shortest
/// columns that still hold that page's text — a binary search on the column height, where a
/// height fits when nothing moves to another page and nothing (an unsplittable row, a line
/// taller than the column) runs past it. Keep-together, keep-with-next and widow/orphan control
/// apply as usual, so the columns may come out a little uneven. Not balanced: a page with an
/// explicit column break, which says where the columns end. Returns the text boxes laid out in the
/// passes thrown away.
fn balance_columns<'a>(ctx: &mut Ctx, pb: &mut PageBuilder<'a>, blocks: (usize, usize), snap: &Snap<'a>, body_top: f32) -> usize {
    let page = pb.pages.len().saturating_sub(1);
    let ncols = pb.cols.len().max(1) as f32;
    let top = pb.top;
    let full = pb.uncapped_bottom() - top;
    let used = pb.content_bottom.min(pb.uncapped_bottom()) - top;
    if pb.col_break_page == Some(page)
        || snap.pages == 0
        || (page + 1).saturating_sub(snap.pages) > BALANCE_MAX_PAGES
        || used <= 1.0
        || full <= 1.0
        || pb.cols.len() < 2
    {
        return 0;
    }
    let pass = |ctx: &mut Ctx, pb: &mut PageBuilder<'a>, h: f32| balance_pass(ctx, pb, snap, blocks, (page, top, h), body_top);
    // The text on the page fills `ncols` columns at least `used / ncols` high; the original layout
    // (`full`) fits, and so, nearly always, do columns as high as its tallest one.
    let mut lo = (used / ncols - 1.0).max(0.0);
    let mut hi = full;
    let mut last = None;
    let discard = |ctx: &Ctx| ctx.boxes.total().saturating_sub(snap.boxes);
    let mut thrown = discard(ctx);
    if used < full - 0.5 {
        let ok = pass(ctx, pb, used);
        if ok {
            hi = used;
        }
        last = Some((used, ok));
    }
    for _ in 0..BALANCE_PASSES {
        if hi - lo <= 1.0 {
            break;
        }
        let mid = (lo + hi) / 2.0;
        thrown = discard(ctx);
        let ok = pass(ctx, pb, mid);
        if ok {
            hi = mid;
        } else {
            lo = mid;
        }
        last = Some((mid, ok));
    }
    // End on the best height (the full column, as laid out first, when none was shorter).
    if last != Some((hi, true)) {
        thrown = discard(ctx);
        if !pass(ctx, pb, hi) && hi < full {
            thrown = discard(ctx);
            pass(ctx, pb, full);
        }
    }
    thrown
}

/// Milliseconds since an arbitrary epoch (monotonic on native; the browser's clock on the web).
pub fn now_ms() -> f64 {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }
    // `Instant::now()` panics on wasm32-unknown-unknown, so ask the browser's clock.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let ms = js_sys::Date::now();
        if ms.is_finite() { ms } else { 0.0 }
    }
}
