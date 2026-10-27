use super::boxes::{BoxContext, BoxLayout, layout_box};
use super::ctx::Ctx;
use super::floats::left_out_objects;
use super::page::{PageBuilder, item_bottom};
use super::placed::Placed;
use super::*;

/// Footnote and endnote numbers (each kind counted separately), in document order, as each
/// section's options say: starting at its "start at", numbered on, or starting again in each
/// section or (footnotes) on each page. `pages` gives the page each footnote was placed on by an
/// earlier pass (empty: numbered on). With `hide_deleted`, notes whose reference mark is a
/// tracked deletion are left out, as in the final text.
pub(crate) fn note_info(doc: &Document, hide_deleted: HideDeleted, pages: &HashMap<u32, usize>) -> HashMap<u32, fields::NoteNum> {
    use wordcraft_doc::section::NoteRestart;
    let sections = doc.sections();
    let opts: Vec<[wordcraft_doc::section::NoteOptions; 2]> =
        sections.iter().map(|(_, s)| [doc.note_options(s, false), doc.note_options(s, true)]).collect();
    let mut m = HashMap::new();
    // Per kind (footnotes, endnotes): the last number, and the section and page of the last note.
    let mut last: [Option<(u32, usize, Option<usize>)>; 2] = [None, None];
    let mut order = [0u32; 2];
    let mut si = 0usize;
    // Reading order, text boxes included where they are. A hidden reference mark still takes
    // its number, as in Word; a deleted one (or one in a deleted or hidden text box) does not,
    // and `para_notes` doesn't place its note either.
    let mut cur: Option<(usize, Vec<bool>)> = None;
    doc.objects_in_reading_order_by_block(&mut |block, p, k| {
        let key = p as *const Paragraph as usize;
        if cur.as_ref().is_none_or(|(at, _)| *at != key) {
            let gone = left_out_objects(doc, p, None, true, hide_deleted);
            cur = Some((key, (0..p.objects.len()).map(gone).collect()));
        }
        if cur.as_ref().is_some_and(|(_, gone)| gone.get(k).copied().unwrap_or(false)) {
            return false;
        }
        if let Some(InlineObject::NoteRef { kind, id, .. }) = p.objects.get(k) {
            while sections.get(si).is_some_and(|(end, _)| block > *end) && si + 1 < sections.len() {
                si += 1;
            }
            let ki = usize::from(*kind == wordcraft_doc::para::NoteKind::Endnote);
            let Some(o) = opts.get(si).and_then(|o| o.get(ki)) else { return true };
            let page = if ki == 0 { pages.get(id).copied() } else { None };
            let Some(slot) = last.get_mut(ki) else { return true };
            let n = match *slot {
                None => o.num_start,
                Some((prev, psect, ppage)) => {
                    let again = match o.num_restart {
                        NoteRestart::Continuous => false,
                        NoteRestart::EachSect => psect != si,
                        NoteRestart::EachPage => page.is_some() && page != ppage,
                    };
                    if again { o.num_start } else { prev.saturating_add(1) }
                }
            };
            *slot = Some((n, si, page));
            let ord = order.get_mut(ki).map(|c| {
                *c = c.saturating_add(1);
                *c
            });
            m.insert(*id, fields::NoteNum { n, fmt: o.num_fmt, section: si, order: ord.unwrap_or(0) });
        }
        true
    });
    m
}

/// Note part ids → numbers, numbered on (see [`note_info`]).
#[cfg(test)]
pub(crate) fn note_numbers(doc: &Document, hide_deleted: HideDeleted) -> HashMap<u32, u32> {
    note_info(doc, hide_deleted, &HashMap::new()).into_iter().map(|(id, n)| (id, n.n)).collect()
}

/// The page each footnote's text starts on.
pub(crate) fn footnote_pages(doc: &Document, pages: &[Page]) -> HashMap<u32, usize> {
    let mut m = HashMap::new();
    for (pi, p) in pages.iter().enumerate() {
        for it in &p.items {
            if let Placed::Lines { story: StoryRef::Part(id), .. } | Placed::Cell { story: StoryRef::Part(id), .. } = it
                && doc.parts.get(id).is_some_and(|part| part.kind == wordcraft_doc::PartKind::Footnote)
            {
                m.entry(*id).or_insert(pi);
            }
        }
    }
    m
}

/// Gap above the footnote separator and its length.
pub(crate) const NOTE_SEP: f32 = 12.0;

/// Length of the default (short) note separator line.
pub(crate) const NOTE_SEP_W: f32 = 144.0;

/// The story of text drawn on a page that belongs to no story of the document (line numbers,
/// note separators and continuation notices): not hit by clicks.
pub const DECOR_STORY: StoryRef = StoryRef::Part(u32::MAX);

/// A note separator or continuation notice laid out from y = 0 (its items and height).
pub(crate) type SepBox = (Vec<Placed>, f32);

/// The document's own footnote or endnote separators and continuation notice, laid out (`None`:
/// the defaults).
#[derive(Clone, Default)]
pub(crate) struct NoteSeps {
    pub(crate) sep: Option<SepBox>,
    pub(crate) cont: Option<SepBox>,
    pub(crate) notice: Option<SepBox>,
}

impl NoteSeps {
    /// Lay out `seps` `width` wide.
    pub(crate) fn new(ctx: &mut Ctx, seps: &wordcraft_doc::NoteSeparators, width: f32) -> NoteSeps {
        let mut lay = |blocks: &Option<Blocks>| blocks.as_ref().map(|b| sep_box(ctx, b, width));
        NoteSeps { sep: lay(&seps.separator), cont: lay(&seps.continuation_separator), notice: lay(&seps.continuation_notice) }
    }
    /// Height of the separator above a page's notes (`cont`: the continuation separator, when the
    /// first is continued from the page before).
    pub(crate) fn sep_h(&self, cont: bool) -> f32 {
        let own = if cont { &self.cont } else { &self.sep };
        own.as_ref().map_or(NOTE_SEP, |s| s.1)
    }
    /// Height of the notice below a note that continues on the next page.
    pub(crate) fn notice_h(&self) -> f32 {
        self.notice.as_ref().map_or(0.0, |s| s.1)
    }
}

/// A note separator or continuation notice story (`blocks`) laid out `width` wide.
fn sep_box(ctx: &mut Ctx, blocks: &Blocks, width: f32) -> SepBox {
    let bl = layout_box(ctx, DECOR_STORY, blocks, &[], width, BoxContext::Other, 0, None);
    let mut items = bl.items;
    items.extend(separator_rules(blocks, &items));
    (items, bl.height)
}

/// The lines drawn by the note separators (see [`InlineObject::note_separator`]) in a separator
/// story's laid-out `items` (`blocks` its paragraphs): across each one's place in its line, half
/// way down the line.
fn separator_rules(blocks: &Blocks, items: &[Placed]) -> Vec<Placed> {
    let mut rules = Vec::new();
    for it in items {
        let Placed::Lines { para, path, l0, l1, x, y, turn: TextDirection::Horizontal, .. } = it else { continue };
        let [i] = path.0.as_slice() else { continue };
        let Some(Block::Para(p)) = blocks.get(*i as usize).map(|b| &**b) else { continue };
        let Some(first) = para.lines.get(*l0) else { continue };
        for line in para.lines.get(*l0..*l1).unwrap_or(&[]) {
            for k in line.c0..line.c1 {
                let Some(para::ClKind::Object(oi)) = para.clusters.get(k).map(|c| c.kind) else { continue };
                if p.objects.get(oi).and_then(InlineObject::as_note_separator).is_none() {
                    continue;
                }
                let (Some(a), Some(b)) = (line.cl_left(k), line.cl_right(k)) else { continue };
                let ry = y + line.top - first.top + line.height / 2.0;
                rules.push(Placed::Rule { x0: x + a.min(b), y0: ry, x1: x + a.max(b), y1: ry, border: Border::single(0.5) });
            }
        }
    }
    rules
}

/// The most pages added after the body for footnotes still continuing.
pub(crate) const MAX_NOTE_PAGES: usize = 10_000;

/// A footnote waiting for the bottom of a page: its items laid out from y = 0, and its height.
#[derive(Clone)]
pub(crate) struct NoteBox {
    pub(crate) id: u32,
    pub(crate) items: Vec<Placed>,
    pub(crate) h: f32,
    /// The rest of a note that didn't fit on an earlier page (drawn below the continuation
    /// separator).
    pub(crate) cont: bool,
}

/// The vertical extent of an item laid out in a horizontal frame.
pub(crate) fn v_span(it: &Placed) -> (f32, f32) {
    if let Some(r) = it.turned_bounds() {
        return (r.y, r.y + r.h);
    }
    match it {
        Placed::Lines { para, l0, l1, y, .. } => (*y, item_bottom(*y, para, *l0, *l1).unwrap_or(*y)),
        Placed::Fill { rect, .. }
        | Placed::Image { rect, .. }
        | Placed::Shape { rect, .. }
        | Placed::Graphic { rect, .. }
        | Placed::Clip { rect, .. }
        | Placed::Cell { rect, .. }
        | Placed::Object { rect, .. } => (rect.y, rect.y + rect.h),
        Placed::Rule { y0, y1, .. } => (y0.min(*y1), y0.max(*y1)),
    }
}

impl NoteBox {
    /// Lines of the note's own paragraphs, which may be split between pages (not those of its
    /// tables or text boxes).
    fn splittable(&self, it: &Placed) -> bool {
        matches!(it, Placed::Lines { story: StoryRef::Part(s), path, turn: TextDirection::Horizontal, .. } if *s == self.id && path.0.len() == 1)
    }

    /// Where the note may be split, ascending: the bottoms of its lines that have more lines
    /// below them and don't cut through a picture, table or other unsplittable item.
    pub(crate) fn cuts(&self) -> Vec<f32> {
        let blocked: Vec<(f32, f32)> =
            self.items.iter().filter(|it| !self.splittable(it) && !matches!(it, Placed::Fill { .. } | Placed::Rule { .. })).map(v_span).collect();
        let mut last_top = f32::MIN;
        let mut cuts = Vec::new();
        for it in &self.items {
            let Placed::Lines { para, l0, l1, y, .. } = it else { continue };
            if !self.splittable(it) {
                continue;
            }
            let Some(first) = para.lines.get(*l0) else { continue };
            for l in para.lines.get(*l0..*l1).unwrap_or(&[]) {
                last_top = last_top.max(y + l.top - first.top);
                cuts.push(y + l.top - first.top + l.height);
            }
        }
        cuts.retain(|&b| b > 0.01 && b <= last_top + 0.01 && !blocked.iter().any(|&(t, e)| t < b - 0.01 && e > b + 0.01));
        cuts.sort_by(f32::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        cuts
    }

    /// The least of the note that must stay with its reference: its first line (all of it if it
    /// can't be split).
    pub(crate) fn first_piece(&self) -> f32 {
        self.cuts().first().copied().unwrap_or(self.h).min(self.h)
    }

    /// Split the note at line bottom `cut`: the part above (`cut` tall) and the rest, moved up to
    /// start at 0 and marked as a continuation.
    pub(crate) fn split(self, cut: f32) -> (NoteBox, NoteBox) {
        let (mut head, mut tail) = (Vec::new(), Vec::new());
        for it in self.items {
            let (top, bottom) = v_span(&it);
            if bottom <= cut + 0.01 {
                head.push(it);
                continue;
            }
            if top >= cut - 0.01 {
                tail.push(it);
                continue;
            }
            match it {
                Placed::Lines { story, path, para, l0, l1, x, y, turn } => {
                    let Some(first) = para.lines.get(l0) else { continue };
                    let ft = first.top;
                    // The first line that ends below the cut starts the tail.
                    let k = (l0..l1).find(|&k| para.lines.get(k).is_none_or(|l| y + l.top - ft + l.height > cut + 0.01)).unwrap_or(l1);
                    let ky = para.lines.get(k).map_or(y, |l| y + l.top - ft);
                    if k > l0 {
                        head.push(Placed::Lines { story, path: path.clone(), para: para.clone(), l0, l1: k, x, y, turn });
                    }
                    if k < l1 {
                        tail.push(Placed::Lines { story, path, para, l0: k, l1, x, y: ky, turn });
                    }
                }
                Placed::Fill { rect, color } => {
                    head.push(Placed::Fill { rect: Rect::new(rect.x, rect.y, rect.w, cut - rect.y), color });
                    tail.push(Placed::Fill { rect: Rect::new(rect.x, cut, rect.w, rect.y + rect.h - cut), color });
                }
                Placed::Rule { x0, y0, x1, y1, border } => {
                    head.push(Placed::Rule { x0, y0: y0.min(cut), x1, y1: y1.min(cut), border });
                    tail.push(Placed::Rule { x0, y0: y0.max(cut), x1, y1: y1.max(cut), border });
                }
                // Not split (cuts avoid them): stays above.
                other => head.push(other),
            }
        }
        // The rest starts at its first line (the space between paragraphs goes with the break).
        let start = tail.iter().filter(|it| matches!(it, Placed::Lines { .. })).map(|it| v_span(it).0).fold(f32::MAX, f32::min);
        let start = if start == f32::MAX { cut } else { start.max(cut) };
        for it in &mut tail {
            it.translate(0.0, -start);
            // Shading cut at the break starts at the rest's top.
            if let Placed::Fill { rect, .. } = it
                && rect.y < 0.0
            {
                rect.h = (rect.h + rect.y).max(0.0);
                rect.y = 0.0;
            }
        }
        let rest = (self.h - start).max(0.0);
        (NoteBox { id: self.id, items: head, h: cut, cont: self.cont }, NoteBox { id: self.id, items: tail, h: rest, cont: true })
    }
}

/// Passes the layout makes at most for footnotes numbered again on each page: their numbers
/// depend on the pages they land on, which (barely) depend on the numbers' widths.
pub(crate) const MAX_NOTE_PASSES: usize = 3;

/// Place endnotes `ids` below the text so far: a separator (the document's own, else a short
/// line), then each note, moving to the next column or page when one doesn't fit.
pub(crate) fn place_endnotes(ctx: &mut Ctx, pb: &mut PageBuilder, ids: Vec<u32>, block: usize) {
    if ids.is_empty() || pb.web {
        return;
    }
    // The document's own endnote separator, else a short line.
    let width = pb.col_w();
    if let Some((items, h)) = ctx.doc.endnote_separators.separator.as_ref().map(|b| sep_box(ctx, b, width)) {
        let (x, y) = (pb.col_x(), pb.y);
        if let Some(pg) = pb.page() {
            pg.items.extend(items.into_iter().map(|mut it| {
                it.translate(x, y);
                it
            }));
        }
        pb.y += h;
    } else {
        pb.y += 12.0;
        let (x, ry) = (pb.col_x(), pb.y);
        if let Some(pg) = pb.page() {
            pg.items.push(Placed::Rule { x0: x, y0: ry, x1: x + NOTE_SEP_W, y1: ry, border: Border::single(0.5) });
        }
        pb.y += 8.0;
    }
    let mut added = 0usize;
    for id in ids {
        let Some(part) = ctx.doc.parts.get(&id) else { continue };
        let blocks = part.blocks.clone();
        let BoxLayout { items, height: h, .. } = layout_box(ctx, StoryRef::Part(id), &blocks, &[], pb.col_w(), BoxContext::Other, 0, None);
        // A note longer than the room left continues at a line break in the next column or on
        // a new page (Word). Each page takes at least a line, so this ends; the page cap is a
        // backstop (the rest then runs off the last page).
        let mut note = NoteBox { id, items, h, cont: false };
        loop {
            let room = pb.bottom - pb.y;
            let cut = if note.h <= room + 0.01 || added >= MAX_NOTE_PAGES {
                None
            } else {
                let cuts = note.cuts();
                match cuts.iter().rev().find(|&&c| c <= room + 0.01).or(cuts.first().filter(|_| pb.at_top())) {
                    Some(&c) => Some(c),
                    None if pb.at_top() => None,
                    None => {
                        pb.advance(block, pb.top);
                        added += 1;
                        continue;
                    }
                }
            };
            let (head, rest) = match cut {
                Some(c) => {
                    let (head, rest) = note.split(c);
                    (head, Some(rest))
                }
                None => (note, None),
            };
            let (x, y) = (pb.col_x(), pb.y);
            if let Some(pg) = pb.page() {
                for mut it in head.items {
                    it.translate(x, y);
                    pg.items.push(it);
                }
            }
            pb.y += head.h;
            let Some(rest) = rest else { break };
            note = rest;
            pb.advance(block, pb.top);
            added += 1;
        }
    }
}

/// Footnotes referenced in a paragraph laid out as `pl`: in its text, and inside its text boxes
/// (on the box's cluster), as (cluster, note id).
pub(crate) fn para_notes(doc: &Document, p: &Paragraph, pl: &ParaLayout) -> Vec<(usize, u32)> {
    let mut notes = pl.notes.clone();
    if p.objects.iter().any(|o| !o.text_boxes().is_empty()) {
        for (ci, c) in pl.clusters.iter().enumerate() {
            if let para::ClKind::Object(oi) = c.kind
                && let Some(o) = p.objects.get(oi).filter(|o| !o.frame().is_some_and(|(_, _, f)| f.hidden))
            {
                for id in o.text_boxes() {
                    notes.extend(doc.notes_in_text_box(id).into_iter().map(|n| (ci, n)));
                }
            }
        }
    }
    notes
}

/// Footnotes referenced in the cell lines of a placed table row (or part of one) go to the
/// bottom of the page, like those of body paragraphs; notes that don't fit there continue on
/// the next page (see [`PageBuilder::flush_notes`]).
pub(crate) fn table_notes(ctx: &mut Ctx, pb: &mut PageBuilder, items: &[Placed]) {
    if pb.web {
        return;
    }
    let mut ids: Vec<u32> = Vec::new();
    for it in items {
        let Placed::Lines { story: StoryRef::Body, para, l0, l1, .. } = it else { continue };
        let (Some(a), Some(b)) = (para.lines.get(*l0), para.lines.get(l1.saturating_sub(1))) else { continue };
        for (ci, id) in &para.notes {
            if *ci >= a.c0 && *ci < b.c1 && !ids.contains(id) && !pb.notes.iter().any(|n| n.id == *id) {
                ids.push(*id);
            }
        }
    }
    let before = pb.notes_h();
    for id in ids {
        let Some(part) = ctx.doc.parts.get(&id).filter(|p| p.kind == wordcraft_doc::PartKind::Footnote) else { continue };
        let blocks = part.blocks.clone();
        let BoxLayout { items, height: h, .. } = layout_box(ctx, StoryRef::Part(id), &blocks, &[], pb.sect.text_width(), BoxContext::Other, 0, None);
        pb.notes.push(NoteBox { id, items, h, cont: false });
    }
    pb.bottom -= pb.notes_h() - before;
}
