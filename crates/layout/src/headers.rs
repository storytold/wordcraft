use super::boxes::{BoxInput, layout_box};
use super::cache::{Totals, hash_of};
use super::ctx::{Ctx, with_pct_base, with_pin};
use super::flow::again;
use super::page::{PageBuilder, PageFrame, page_border_rules};
use super::placed::{HfSlot, Placed};
use super::{
    Arc, Document, HashMap, HashSet, HeaderSet, InlineObject, Page, Path, Rect, SectionProps, SectionStart, StoryRef, ViewMode, fields, styleref,
    work,
};

/// Whether a section that starts on a new page needs a blank page first so it lands on the right
/// side, and the page number it restarts at. `last` is the number of the page before it and
/// `pages` how many pages there are so far.
/// - An odd- or even-page break counts on from the previous section's numbers. A section that
///   restarts its numbering gets no blank page unless the document prints both sides (different
///   odd and even headers, or mirror margins); a restart at the wrong parity takes the next number.
/// - With different odd and even headers, a next-page break that restarts the numbers at an odd
///   (even) number starts on an odd (even) sheet.
///
/// (Word's output for docxide-pdf's fixtures: bush_fires_act_comparison has a blank page 2.)
pub(crate) fn section_side(start: SectionStart, restart: Option<u32>, last: u32, pages: usize, even_odd: bool, mirror: bool) -> (bool, Option<u32>) {
    match start {
        SectionStart::OddPage | SectionStart::EvenPage => {
            let want_odd = start == SectionStart::OddPage;
            let continuing_odd = last.saturating_add(1) % 2 == 1;
            let filler = pages > 0 && continuing_odd != want_odd && (restart.is_none() || even_odd || mirror);
            let restart = restart.map(|n| if (n % 2 == 1) == want_odd { n } else { n.saturating_add(1) });
            (filler, restart)
        }
        SectionStart::NextPage => {
            let filler = even_odd && pages > 0 && restart.is_some_and(|n| (n % 2 == 1) != ((pages + 1) % 2 == 1));
            (filler, restart)
        }
        SectionStart::Continuous | SectionStart::NextColumn => (false, restart),
    }
}

/// The document's sections with their headers and footers (Word's "link to previous"), each
/// section's resolved once for the whole layout in document order rather than by looking back
/// over the sections before it whenever one is needed.
pub(crate) struct HfLinks<'a> {
    pub(crate) sections: Vec<(usize, &'a SectionProps)>,
    pub(crate) heads: Linked,
    pub(crate) feet: Linked,
}

impl<'a> HfLinks<'a> {
    pub(crate) fn new(sections: Vec<(usize, &'a SectionProps)>) -> Self {
        let heads = Linked::new(&sections, |s| s.headers);
        let feet = Linked::new(&sections, |s| s.footers);
        HfLinks { sections, heads, feet }
    }

    /// The text width of the section that defines header (or, `footer`, footer) `id`, when
    /// section `si` inherits it rather than naming it (`None` when it names it). Measured in
    /// Word: an inherited part's percentage-width tables are sized by that section, though it's
    /// laid out on `si`'s pages.
    pub(crate) fn defining_width(&self, footer: bool, si: usize, id: Option<u32>) -> Option<f32> {
        let id = id?;
        work::steps(1);
        let linked = if footer { &self.feet } else { &self.heads };
        let own = self.sections.get(si)?.1;
        if linked.own.get(si).is_some_and(|h| [h.default, h.first, h.even].contains(&Some(id))) {
            return None;
        }
        // The last section before `si` naming it.
        let namers = linked.namers.get(&id)?;
        let k = *namers.get(namers.partition_point(|&k| k < si).checked_sub(1)?)?;
        self.sections.get(k).map(|(_, s)| s.text_width()).filter(|w| (w - own.text_width()).abs() > 0.01)
    }
}

/// The headers (or footers) of every section: those it names, and with those it leaves out
/// inherited from the sections before it.
pub(crate) struct Linked {
    own: Vec<HeaderSet>,
    linked: Vec<HeaderSet>,
    /// The sections naming each part, in order.
    namers: HashMap<u32, Vec<usize>>,
}

impl Linked {
    fn new(sections: &[(usize, &SectionProps)], get: fn(&SectionProps) -> HeaderSet) -> Self {
        let own: Vec<HeaderSet> = sections.iter().map(|(_, s)| get(s)).collect();
        let mut linked = Vec::with_capacity(own.len());
        let mut namers: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut prev = HeaderSet::default();
        for (si, o) in own.iter().enumerate() {
            work::steps(1);
            prev = HeaderSet { default: o.default.or(prev.default), first: o.first.or(prev.first), even: o.even.or(prev.even) };
            linked.push(prev);
            for id in [o.default, o.first, o.even].into_iter().flatten() {
                let v = namers.entry(id).or_default();
                if v.last() != Some(&si) {
                    v.push(si);
                }
            }
        }
        Linked { own, linked, namers }
    }

    /// Section `si`'s headers (or footers), those it leaves out inherited.
    pub(crate) fn set(&self, si: usize) -> HeaderSet {
        self.linked.get(si).copied().unwrap_or_default()
    }
}

/// Header rows are re-placed on every page a table spills onto, so a table is O(header items ×
/// pages) placed items — and an exact-height header row keeps all its overflowing text without
/// growing, so a few KB of document could otherwise fill memory. Once this many items (a row
/// counts at least one) have been repeated across a table, later pages carry no header rows;
/// pagination still finishes. Far above any real table (three ordinary header rows over some
/// 5 000 pages).
pub(crate) const MAX_HEADER_ITEMS: usize = 500_000;

/// Header tables can displace a non-overlapping body table. Page-positioned drawings keep
/// their own wrapping rules, so only table areas are retained from the header layout.
pub(crate) fn header_table_areas(ctx: &mut Ctx, pb: &PageBuilder) -> Vec<Rect> {
    if pb.web || ctx.opts.view == ViewMode::Read {
        return Vec::new();
    }
    let Some(page) = pb.pages.last() else { return Vec::new() };
    let first = pb.pages.iter().rev().nth(1).is_none_or(|prev| prev.section != page.section);
    let key = (page.section, page.number, first);
    if let Some(areas) = ctx.header_tables.get(&key) {
        return areas.clone();
    }
    let hf = Arc::clone(&ctx.hf);
    let Some((_, sect)) = hf.sections.get(page.section) else { return Vec::new() };
    if sect.text_direction.is_turned() {
        return Vec::new();
    }
    let set = hf.heads.set(page.section);
    let id = if sect.title_page && first {
        set.first
    } else if ctx.doc.settings.even_odd_headers && page.number.is_multiple_of(2) {
        set.even.or(set.default)
    } else {
        set.default
    };
    let Some(id) = id else { return Vec::new() };
    let Some(part) = ctx.doc.parts.get(&id) else { return Vec::new() };
    let blocks = part.blocks.clone();
    let x = sect.margin_left + sect.gutter;
    let frame = PageFrame { sect, origin: (x, sect.header), number: page.number, mirror: ctx.mirror };
    let base = hf.defining_width(false, page.section, Some(id));
    let counters = std::mem::take(&mut ctx.counters);
    let boxes = std::mem::take(&mut ctx.boxes);
    let chains = std::mem::take(&mut ctx.chains);
    let fields = ctx.fields.clone();
    let grid = std::mem::take(&mut ctx.grid);
    let track = std::mem::replace(&mut ctx.track_totals, false);
    ctx.fields.page = page.number;
    let mut areas = again(ctx, 0, |ctx| {
        with_pct_base(ctx, base, |ctx| layout_box(ctx, BoxInput::other(StoryRef::Part(id), &blocks, sect.text_width(), 0, Some(frame)))).table_areas
    });
    ctx.counters = counters;
    ctx.boxes = boxes;
    ctx.chains = chains;
    ctx.fields = fields;
    ctx.grid = grid;
    ctx.track_totals = track;
    for r in &mut areas {
        r.x += x;
        r.y += sect.header;
    }
    ctx.header_tables.insert(key, areas.clone());
    areas
}

/// Whether a header, a footer or a text box shows a `PAGEREF` field, which they resolve from the
/// paginated body (see [`bookmark_pages`]).
pub(crate) fn headers_footers_refer_to_pages(doc: &Document) -> bool {
    use wordcraft_doc::PartKind;
    doc.parts.iter().filter(|(_, part)| matches!(part.kind, PartKind::Header | PartKind::Footer | PartKind::TextBox)).any(|(id, _)| {
        let story = StoryRef::Part(*id);
        doc.para_paths(story)
            .iter()
            .filter_map(|path| doc.para(story, path))
            .any(|p| p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if fields::field_name(instr) == "PAGEREF")))
    })
}

/// Where each bookmark in the body starts: the number and number format of the page it is placed
/// on (with two of the same name, the first in the document).
pub(crate) fn bookmark_pages(doc: &Document, pages: &[Page], sections: &[(usize, &SectionProps)]) -> fields::BookmarkPages {
    // Each paragraph's bookmarks: name, offset, and its line in the paragraph layout last looked
    // up (the pieces of a paragraph usually share one).
    let mut wanted: HashMap<Path, Vec<(String, usize, Option<(usize, usize)>)>> = HashMap::new();
    let mut seen = HashSet::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        if !p.objects.iter().any(|o| matches!(o, InlineObject::BookmarkStart { .. })) {
            continue;
        }
        for (off, o) in p.object_offsets().into_iter().zip(&p.objects) {
            if let InlineObject::BookmarkStart { name } = o
                && seen.insert(name.as_str())
            {
                wanted.entry(path.clone()).or_default().push((name.clone(), off, None));
            }
        }
    }
    let mut out = fields::BookmarkPages::new();
    for page in pages {
        if wanted.is_empty() {
            break;
        }
        let format = sections.get(page.section).map(|(_, s)| s.page_num_format).unwrap_or_default();
        for it in &page.items {
            let Placed::Lines { story: StoryRef::Body, path, para, l0, l1, .. } = it else { continue };
            let Some(marks) = wanted.get_mut(path) else { continue };
            let layout = Arc::as_ptr(para) as usize;
            marks.retain_mut(|(name, off, line)| {
                let li = match *line {
                    Some((k, li)) if k == layout => li,
                    _ => {
                        let li = para.line_of(*off);
                        *line = Some((layout, li));
                        li
                    }
                };
                if !(*l0..*l1).contains(&li) {
                    return true;
                }
                out.insert(std::mem::take(name), (page.number, format));
                false
            });
            if marks.is_empty() {
                wanted.remove(path);
            }
        }
    }
    out
}

/// A key for caches standing for `marks` (0 for none).
pub(crate) fn marks_key(marks: &fields::BookmarkPages) -> u64 {
    if marks.is_empty() {
        return 0;
    }
    let mut v: Vec<_> = marks.iter().collect();
    v.sort_by(|a, b| a.0.cmp(b.0));
    hash_of(&v)
}

pub(crate) fn headers_footers(ctx: &mut Ctx, pages: &mut [Page], sections: &[(usize, &SectionProps)], totals: &Totals) {
    let total = pages.len() as u32;
    let hf = Arc::clone(&ctx.hf);
    let even_odd = ctx.doc.settings.even_odd_headers;
    let styles = styleref::headers_use_styleref(ctx.doc).then(|| ctx.cache.style_index(ctx.doc));
    let mut first_of_section: HashSet<usize> = HashSet::new();
    let mut seen = HashSet::new();
    for (i, p) in pages.iter().enumerate() {
        if seen.insert(p.section) {
            first_of_section.insert(i);
        }
    }
    for (i, page) in pages.iter_mut().enumerate() {
        let Some((_, sect)) = sections.get(page.section) else { continue };
        // Inherit header/footer references from earlier sections (Word's "link to previous").
        let pick = |linked: &Linked| -> Option<u32> {
            let set = linked.set(page.section);
            if sect.title_page && first_of_section.contains(&i) {
                return set.first;
            }
            if even_odd && page.number % 2 == 0 {
                return set.even.or(set.default);
            }
            set.default
        };
        let hid = pick(&hf.heads);
        let fid = pick(&hf.feet);
        page.hf_slot = if sect.title_page && first_of_section.contains(&i) {
            HfSlot::First
        } else if even_odd && page.number % 2 == 0 {
            HfSlot::Even
        } else {
            HfSlot::Default
        };
        ctx.fields.page = page.number;
        ctx.fields.page_format = sect.page_num_format;
        ctx.fields.pages = total;
        ctx.fields.section_pages = totals.section(page.section);
        ctx.fields.section = page.section as u32 + 1;
        // STYLEREF: the body paragraphs on this page.
        ctx.fields.styleref = styles.as_ref().map(|ix| {
            let paths = page.items.iter().filter_map(|it| match it {
                Placed::Lines { story: StoryRef::Body, path, .. } => Some(path),
                _ => None,
            });
            (ix.clone(), ix.page_at(paths, page.first_block))
        });
        let w = sect.text_width();
        let x = sect.margin_left + sect.gutter;
        if let Some(id) = hid
            && let Some(part) = ctx.doc.parts.get(&id)
        {
            let blocks = part.blocks.clone();
            let frame = PageFrame { sect, origin: (x, sect.header), number: page.number, mirror: ctx.mirror };
            let base = hf.defining_width(false, page.section, hid);
            let (mut items, behind, front) = with_pin(ctx, id, sect, page.number, |ctx| {
                with_pct_base(ctx, base, |c| layout_box(c, BoxInput::other(StoryRef::Part(id), &blocks, w, 0, Some(frame))).items)
            });
            for it in &mut items {
                it.translate(x, sect.header);
            }
            page.header = behind.into_iter().chain(items).chain(front).collect();
            page.header_story = Some(id);
        }
        if let Some(id) = fid
            && let Some(part) = ctx.doc.parts.get(&id)
        {
            let blocks = part.blocks.clone();
            let base = hf.defining_width(true, page.section, fid);
            // The footer's height fixes where it sits, which page-relative floats need.
            let (h, ..) = with_pin(ctx, id, sect, page.number, |ctx| {
                with_pct_base(ctx, base, |c| layout_box(c, BoxInput::other(StoryRef::Part(id), &blocks, w, 0, None)).height)
            });
            let frame = PageFrame { sect, origin: (x, sect.page_h - sect.footer - h), number: page.number, mirror: ctx.mirror };
            let (mut items, behind, front) = with_pin(ctx, id, sect, page.number, |ctx| {
                with_pct_base(ctx, base, |c| layout_box(c, BoxInput::other(StoryRef::Part(id), &blocks, w, 0, Some(frame))).items)
            });
            for it in &mut items {
                it.translate(x, sect.page_h - sect.footer - h);
            }
            page.footer = behind.into_iter().chain(items).chain(front).collect();
            page.footer_story = Some(id);
        }
    }
    ctx.fields.styleref = None;
}

/// Mirrored margins: even pages have the inside margin (and gutter) on the right, so their
/// header, text, notes and footer move to start at the outside margin. Page borders, measured
/// from the page edge, stay.
pub(crate) fn mirror_even_pages(pages: &mut [Page], sections: &[(usize, &SectionProps)]) {
    for page in pages.iter_mut().filter(|p| p.number % 2 == 0) {
        let Some((_, s)) = sections.get(page.section) else { continue };
        let dx = s.margin_right - s.margin_left - s.gutter;
        if dx.abs() < 0.001 || !dx.is_finite() {
            continue;
        }
        let borders = page_border_rules(s).len();
        for it in page.items.iter_mut().skip(borders).chain(page.header.iter_mut()).chain(page.footer.iter_mut()) {
            it.translate(dx, 0.0);
        }
        page.body.x += dx;
    }
}
