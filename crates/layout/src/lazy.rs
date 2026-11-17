//! Lazy pages: a long document's layout keeps in full only the pages a reader is near, and
//! the rest thinned ([`ParaLayout::thinned`]), which keeps every page's size, number, items and
//! lines but not the clusters and glyphs drawing reads (three quarters of a layout's memory).
//!
//! Pagination is untouched. The document is laid out in full as before; [`keep_full`] then
//! thins the pages outside a set of ranges, in the layout, in the paragraph cache (which drops
//! them, so they are laid out again if placed again) and in the incremental state the layout
//! left ([`crate::resume`]). Placing never reads a thinned paragraph: the cache serves none, and
//! the incremental state's pages are read only for their lines' extent and paths.
//!
//! [`layout_pages`] lays pages out again in full from the incremental state: it starts at the
//! mark before the block at the top of the first page and places blocks until the state meets
//! the last layout's again, then stops; the paragraphs laid out fill in the thinned ones. A
//! page costs about a page wherever it is. The pages come back as a full layout has them. A layout without an incremental state (one
//! that took more than one pass, columns, text boxes, endnotes, page totals in the body: see
//! [`crate::resume`]) is never thinned.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::cache::Totals;
use super::ctx::Pass;
use super::floats::EarlyFloats;
use super::resume::{self, Run};
use super::{DocLayout, Document, LayoutCache, LayoutOptions, Page, ParaLayout, Placed, StoryRef, pass};

/// What a layout needs to lay its pages out again: the options it was made with and the
/// incremental state it left in the cache.
#[derive(Debug)]
pub struct Lazy {
    pub(crate) opts: LayoutOptions,
    pub(crate) resume: u64,
}

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
static AGAIN: AtomicUsize = AtomicUsize::new(0);
static IN_FULL: AtomicUsize = AtomicUsize::new(0);

/// A number for a new layout ([`DocLayout::epoch`]).
pub(crate) fn next_epoch() -> u64 {
    NEXT_EPOCH.fetch_add(1, Ordering::Relaxed)
}

/// How many times [`layout_pages`] laid pages out again from the incremental state, and how
/// many times it had to lay the whole document out instead (none, if all is well).
pub fn lazy_stats() -> (usize, usize) {
    (AGAIN.load(Ordering::Relaxed), IN_FULL.load(Ordering::Relaxed))
}

/// Whether `page` holds a thinned paragraph: to draw it, read its text or hit-test it, lay it
/// out again with [`layout_pages`].
pub fn page_is_thin(page: &Page) -> bool {
    fn thin(items: &[Placed]) -> bool {
        items.iter().any(|it| match it {
            Placed::Lines { para, .. } => para.thin,
            Placed::Clip { items, .. } => thin(items),
            _ => false,
        })
    }
    thin(&page.items)
}

/// Calls `f` on each body paragraph placed among `items`, replacing it with what `f` returns.
fn map_body(items: &mut [Placed], f: &mut dyn FnMut(&Arc<ParaLayout>) -> Option<Arc<ParaLayout>>) {
    for it in items {
        match it {
            Placed::Lines { story: StoryRef::Body, para, .. } => {
                if let Some(new) = f(para) {
                    *para = new;
                }
            }
            Placed::Clip { items, .. } => map_body(items, f),
            _ => {}
        }
    }
}

/// Calls `f` on each body paragraph placed among `items`.
fn each_body(items: &[Placed], f: &mut dyn FnMut(&Arc<ParaLayout>)) {
    for it in items {
        match it {
            Placed::Lines { story: StoryRef::Body, para, .. } => f(para),
            Placed::Clip { items, .. } => each_body(items, f),
            _ => {}
        }
    }
}

/// Whether two layouts of a page place the same things in the same places: size, number, first
/// block and each item, with each paragraph's lines, position and stamp.
pub fn same_geometry(a: &Page, b: &Page) -> bool {
    fn items(a: &[Placed], b: &[Placed]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|(x, y)| match (x, y) {
                (
                    Placed::Lines { story: s0, path: p0, para: q0, l0: a0, l1: b0, x: x0, y: y0, .. },
                    Placed::Lines { story: s1, path: p1, para: q1, l0: a1, l1: b1, x: x1, y: y1, .. },
                ) => {
                    s0 == s1
                        && p0 == p1
                        && (a0, b0) == (a1, b1)
                        && x0.to_bits() == x1.to_bits()
                        && y0.to_bits() == y1.to_bits()
                        && q0.stamp == q1.stamp
                        && q0.lines.len() == q1.lines.len()
                        && q0.lines.iter().zip(&q1.lines).all(|(m, n)| {
                            (m.top.to_bits(), m.height.to_bits(), m.c0, m.c1, m.start, m.stop)
                                == (n.top.to_bits(), n.height.to_bits(), n.c0, n.c1, n.start, n.stop)
                        })
                }
                (Placed::Clip { items: i0, .. }, Placed::Clip { items: i1, .. }) => items(i0, i1),
                (Placed::Lines { .. }, _) | (_, Placed::Lines { .. }) => false,
                _ => std::mem::discriminant(x) == std::mem::discriminant(y),
            })
    }
    a.w.to_bits() == b.w.to_bits()
        && a.h.to_bits() == b.h.to_bits()
        && a.number == b.number
        && a.section == b.section
        && a.first_block == b.first_block
        && items(&a.items, &b.items)
        && items(&a.header, &b.header)
        && items(&a.footer, &b.footer)
}

/// `page` with its body paragraphs thinned.
fn thinned_page(page: &Page) -> Page {
    let mut p = page.clone();
    map_body(&mut p.items, &mut |pl| (!pl.thin).then(|| Arc::new(pl.thinned())));
    p
}

/// Whether two layouts' pages are the same, thinned or not: a page thinned in either is
/// compared thinned in both ([`crate::verify_incremental`]).
pub fn same_pages(a: &[Page], b: &[Page]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_page(x, y))
}

/// Whether two layouts of a page are the same, thinned or not, by their Debug: unlike
/// [`same_geometry`] it doesn't compare stamps, so it holds across caches (another session's
/// layout of the same document).
pub fn same_page(x: &Page, y: &Page) -> bool {
    if page_is_thin(x) || page_is_thin(y) {
        format!("{:?}", thinned_page(x)) == format!("{:?}", thinned_page(y))
    } else {
        format!("{x:?}") == format!("{y:?}")
    }
}

/// Pages `range` of `layout` in full, laid out again from the incremental state the layout
/// left in `cache`, exactly as a full layout has them. `layout` stays as it is, and so does
/// the cache, but for the paragraph layouts made, which it keeps only for paragraphs not thinned.
/// `None` if `layout` can't be laid out again: it left no incremental state, or the cache's is
/// another layout's (the document was laid out again since).
pub fn layout_pages(doc: &Document, cache: &mut LayoutCache, layout: &DocLayout, range: Range<usize>) -> Option<Vec<Page>> {
    let lazy = layout.lazy.as_ref()?;
    let n = layout.pages.len();
    let range = range.start.min(n)..range.end.min(n);
    if range.is_empty() {
        return Some(Vec::new());
    }
    if cache.resume.as_ref().map(|r| r.id) != Some(lazy.resume) {
        return None;
    }
    let pages = match again(doc, cache, &lazy.opts, layout, range.clone()) {
        Some(p) => p,
        None => {
            IN_FULL.fetch_add(1, Ordering::Relaxed);
            in_full(doc, &lazy.opts, layout, range)?
        }
    };
    let thinned = std::mem::take(&mut cache.thinned);
    cache.paras.retain(|_, pl| !thinned.contains(&pl.stamp.0));
    cache.thinned = thinned;
    Some(pages)
}

/// [`layout_pages`] from the incremental state: one pass that places only from the mark at the
/// top of the first page until the state meets the last layout's again after the last, and
/// stops ([`Run::range`]), so it costs about the pages asked for, wherever they are. The
/// paragraphs it lays out fill in the thinned ones of `layout`'s pages, by stamp, each with the
/// same lines; the rest of each page (where things sit, headers, footers, notes) is `layout`'s
/// own. `None` if any thinned paragraph isn't laid out again the same.
fn again(doc: &Document, cache: &mut LayoutCache, opts: &LayoutOptions, layout: &DocLayout, range: Range<usize>) -> Option<Vec<Page>> {
    let plan = resume::range_plan(doc, cache, opts, range.clone())?;
    AGAIN.fetch_add(1, Ordering::Relaxed);
    let totals = Totals::first_guess(doc.sections().len());
    let pass = pass::layout_pass_with(doc, cache, opts, (&HashMap::new(), &totals, true), (&EarlyFloats::new(), true), Run::range(plan));
    // What this pass would leave to the next layout: the last layout's stays.
    cache.pending = None;
    let Pass::Done(placed) = pass else { return None };
    let mut full: HashMap<u64, Arc<ParaLayout>> = HashMap::new();
    for p in &placed.pages {
        each_body(&p.items, &mut |pl| {
            if pl.stamp.0 != 0 && !pl.thin {
                full.entry(pl.stamp.0).or_insert_with(|| pl.clone());
            }
        });
    }
    let mut out = Vec::with_capacity(range.len());
    for i in range {
        let mut page = layout.pages[i].clone();
        let mut missing = false;
        map_body(&mut page.items, &mut |pl| {
            if !pl.thin {
                return None;
            }
            match full.get(&pl.stamp.0) {
                Some(f) if same_lines(pl, f) => Some(f.clone()),
                _ => {
                    missing = true;
                    None
                }
            }
        });
        if missing {
            return None;
        }
        out.push(page);
    }
    Some(out)
}

/// Whether two layouts of a paragraph break and stack its lines alike.
fn same_lines(a: &ParaLayout, b: &ParaLayout) -> bool {
    a.lines.len() == b.lines.len()
        && a.lines.iter().zip(&b.lines).all(|(m, n)| {
            (m.top.to_bits(), m.height.to_bits(), m.c0, m.c1, m.start, m.stop) == (n.top.to_bits(), n.height.to_bits(), n.c0, n.c1, n.start, n.stop)
        })
}

/// `layout` laid out again in full, from nothing: for a layout whose incremental state is gone
/// (the document was edited and laid out since) when its thinned pages are still wanted. It
/// costs a whole layout. `None` if it doesn't come out with the same pages.
pub fn layout_in_full(doc: &Document, layout: &DocLayout) -> Option<DocLayout> {
    let lazy = layout.lazy.as_ref()?;
    IN_FULL.fetch_add(1, Ordering::Relaxed);
    let l = crate::layout(doc, &mut LayoutCache::new(), &LayoutOptions { incremental: false, ..lazy.opts.clone() });
    (l.pages.len() == layout.pages.len() && l.pages.iter().zip(&layout.pages).all(|(a, b)| same_geometry(a, b))).then_some(l)
}

/// [`layout_pages`] by laying the whole document out again, with a cache of its own: what a
/// pass from the incremental state that doesn't come out the same falls back on.
fn in_full(doc: &Document, opts: &LayoutOptions, layout: &DocLayout, range: Range<usize>) -> Option<Vec<Page>> {
    let mut l = crate::layout(doc, &mut LayoutCache::new(), &LayoutOptions { incremental: false, ..opts.clone() });
    if l.pages.len() != layout.pages.len() || !range.clone().all(|i| same_geometry(&l.pages[i], &layout.pages[i])) {
        return None;
    }
    Some(l.pages.drain(range).collect())
}

/// `layout` with the pages in `keep` in full (laid out again where they were thinned) and the
/// body paragraphs of every other page thinned, unless they are also on a kept page. The cache
/// is thinned to match: it drops the paragraphs thinned, and its incremental state holds the
/// same thinned paragraphs and the kept pages' full ones, so an edit near the kept pages lays
/// them out in full again. `None` when nothing changes, or `layout` can't be thinned (as for
/// [`layout_pages`]).
pub fn keep_full(doc: &Document, cache: &mut LayoutCache, layout: &DocLayout, keep: &[Range<usize>]) -> Option<DocLayout> {
    let lazy = layout.lazy.as_ref()?;
    if cache.resume.as_ref().map(|r| r.id) != Some(lazy.resume) {
        return None;
    }
    let n = layout.pages.len();
    let kept = |i: usize| keep.iter().any(|r| r.contains(&i));
    // Most calls (after each command) change nothing: no kept page is thin, and every paragraph
    // elsewhere is thinned already or on a kept page too. Found without copying a page.
    if !(0..n).any(|i| kept(i) && page_is_thin(&layout.pages[i])) {
        let mut on_kept: HashSet<u64> = HashSet::new();
        for (_, p) in layout.pages.iter().enumerate().filter(|(i, _)| kept(*i)) {
            each_body(&p.items, &mut |pl| {
                if pl.stamp.0 != 0 {
                    on_kept.insert(pl.stamp.0);
                }
            });
        }
        let mut more = false;
        for (_, p) in layout.pages.iter().enumerate().filter(|(i, _)| !kept(*i)) {
            each_body(&p.items, &mut |pl| more |= pl.stamp.0 != 0 && !pl.thin && !on_kept.contains(&pl.stamp.0));
            if more {
                break;
            }
        }
        if !more {
            return None;
        }
    }
    // The kept pages that are thin, in runs, laid out again.
    let mut full: Vec<(usize, Page)> = Vec::new();
    let mut i = 0;
    while i < n {
        if !(kept(i) && page_is_thin(&layout.pages[i])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && kept(i) && page_is_thin(&layout.pages[i]) {
            i += 1;
        }
        let pages = layout_pages(doc, cache, layout, start..i)?;
        full.extend((start..i).zip(pages));
    }
    let mut pages = layout.pages.clone();
    let mut changed = !full.is_empty();
    for (i, p) in full {
        pages[i] = p;
    }
    // What the kept pages hold, by stamp.
    let mut on_kept: HashMap<u64, Arc<ParaLayout>> = HashMap::new();
    let mut kept_thin: HashSet<u64> = HashSet::new();
    for page in pages.iter().enumerate().filter(|(i, _)| kept(*i)).map(|(_, p)| p) {
        each_body(&page.items, &mut |pl| match (pl.stamp.0, pl.thin) {
            (0, _) => {}
            (s, true) => {
                kept_thin.insert(s);
            }
            (s, false) => {
                on_kept.entry(s).or_insert_with(|| pl.clone());
            }
        });
    }
    // Thin the rest, one copy per paragraph.
    let mut thin: HashMap<u64, Arc<ParaLayout>> = HashMap::new();
    let mut thinned: HashSet<u64> = HashSet::new();
    for (_, page) in pages.iter_mut().enumerate().filter(|(i, _)| !kept(*i)) {
        map_body(&mut page.items, &mut |pl| {
            if pl.stamp.0 == 0 || on_kept.contains_key(&pl.stamp.0) {
                return None;
            }
            thinned.insert(pl.stamp.0);
            if pl.thin {
                return None;
            }
            changed = true;
            Some(thin.entry(pl.stamp.0).or_insert_with(|| Arc::new(pl.thinned())).clone())
        });
    }
    // A kept page still thin (it couldn't be laid out again) keeps its paragraphs out of the cache.
    thinned.extend(kept_thin.into_iter().filter(|s| !on_kept.contains_key(s)));
    if !changed {
        cache.thinned = thinned;
        return None;
    }
    cache.paras.retain(|_, pl| !thinned.contains(&pl.stamp.0));
    if let Some(r) = cache.resume.as_mut() {
        for p in Arc::make_mut(r).pages_mut() {
            map_body(&mut p.items, &mut |pl| {
                if let Some(f) = on_kept.get(&pl.stamp.0).filter(|_| pl.thin) {
                    Some(f.clone())
                } else if !pl.thin && thinned.contains(&pl.stamp.0) {
                    Some(thin.entry(pl.stamp.0).or_insert_with(|| Arc::new(pl.thinned())).clone())
                } else {
                    None
                }
            });
        }
    }
    cache.thinned = thinned;
    let thinned = pages.iter().any(page_is_thin);
    Some(DocLayout {
        pages,
        index: layout.index.clone(),
        ms: layout.ms,
        text_boxes: layout.text_boxes,
        epoch: layout.epoch,
        lazy: layout.lazy.clone(),
        thinned,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wordcraft_doc::{Block, CharProps, Document, Paragraph};

    use super::{keep_full, layout_pages, page_is_thin, same_geometry, same_pages};
    use crate::{DocLayout, LayoutCache, LayoutOptions, layout};

    const WORDS: [&str; 10] = ["the", "court", "held", "that", "appeal", "was", "allowed", "with", "costs", "notwithstanding"];

    /// `n` paragraphs of one to two hundred words, some kept with the next, some starting a page.
    fn para(i: usize, extra: usize) -> Paragraph {
        let words = 1 + (i * 37 + extra) % 200;
        let text = (0..words).map(|w| WORDS[(i + w * 7) % WORDS.len()]).collect::<Vec<_>>().join(" ");
        let mut p = Paragraph::with_text(&text, CharProps::default());
        match i % 11 {
            0 => p.props.keep_next = Some(true),
            5 => p.props.page_break_before = Some(true),
            7 => p.props.keep_lines = Some(true),
            _ => {}
        }
        p
    }

    fn doc(n: usize) -> Document {
        let mut d = Document::default();
        d.body = (0..n).map(|i| Arc::new(Block::Para(para(i, 0)))).collect();
        d
    }

    fn opts() -> LayoutOptions {
        LayoutOptions { incremental: true, ..Default::default() }
    }

    fn full(d: &Document) -> DocLayout {
        layout(d, &mut LayoutCache::new(), &LayoutOptions { incremental: false, ..opts() })
    }

    /// Pages `range` of `l`, laid out again, are the full layout's, Debug for Debug.
    fn again_matches(d: &Document, cache: &mut LayoutCache, l: &DocLayout, full: &DocLayout, range: std::ops::Range<usize>) {
        let got = layout_pages(d, cache, l, range.clone()).expect("laid out again");
        assert_eq!(got.len(), range.len());
        for (i, p) in range.zip(&got) {
            assert!(!page_is_thin(p), "page {i} still thin");
            assert_eq!(format!("{p:?}"), format!("{:?}", full.pages[i]), "page {i}");
        }
    }

    #[test]
    fn thinned_pages_lay_out_again_as_they_were() {
        let d = doc(400);
        let whole = full(&d);
        let mut cache = LayoutCache::new();
        let l = layout(&d, &mut cache, &opts());
        let n = l.pages.len();
        assert!(n > 40, "{n} pages");
        assert!(l.lazy.is_some() && !l.thinned);
        let t = keep_full(&d, &mut cache, &l, &[0..3]).expect("thinned");
        assert!(t.thinned && !page_is_thin(&t.pages[1]) && page_is_thin(&t.pages[n / 2]));
        assert_eq!(t.epoch, l.epoch);
        assert!(t.pages.iter().zip(&whole.pages).all(|(a, b)| same_geometry(a, b)));
        assert!(same_pages(&t.pages, &whole.pages));
        for start in (0..n).step_by(7) {
            again_matches(&d, &mut cache, &t, &whole, start..(start + 7).min(n));
        }
        // The window moves: its pages come back in full, the old one is thinned.
        let mid = n / 2;
        let w = keep_full(&d, &mut cache, &t, &[mid..mid + 3]).expect("moved");
        for i in mid..mid + 3 {
            assert_eq!(format!("{:?}", w.pages[i]), format!("{:?}", whole.pages[i]), "page {i}");
        }
        assert!(page_is_thin(&w.pages[1]));
        assert!(keep_full(&d, &mut cache, &w, &[mid..mid + 3]).is_none(), "nothing more to change");
    }

    #[test]
    fn edits_after_thinning_match_a_full_layout() {
        let mut d = doc(400);
        let mut cache = LayoutCache::new();
        let l = layout(&d, &mut cache, &opts());
        let _thinned = keep_full(&d, &mut cache, &l, &[0..3]).expect("thinned");
        for (step, k) in [5usize, 200, 391, 120].into_iter().enumerate() {
            d.body[k] = Arc::new(Block::Para(para(k, 13 + step)));
            let l = layout(&d, &mut cache, &opts());
            let whole = full(&d);
            assert!(same_pages(&l.pages, &whole.pages), "edit {step}: pages differ from a full layout");
            let page = l.index.get(&(wordcraft_doc::StoryRef::Body, wordcraft_doc::Path::top(k))).and_then(|v| v.first()).map_or(0, |p| p.0);
            let near = page.saturating_sub(1)..(page + 2).min(l.pages.len());
            let cur = keep_full(&d, &mut cache, &l, &[near.clone()]).unwrap_or(l);
            for i in near {
                assert_eq!(format!("{:?}", cur.pages[i]), format!("{:?}", whole.pages[i]), "edit {step}, page {i}");
            }
            if cur.lazy.is_some() {
                let n = cur.pages.len();
                again_matches(&d, &mut cache, &cur, &whole, n - 4..n);
            }
        }
    }
}
