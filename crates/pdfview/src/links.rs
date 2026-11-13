//! PDF link annotations and destinations.
//!
//! `hayro-syntax` deliberately does not expose annotations (see its "Limitations" section), but
//! it does expose every page's raw [`Dict`], the [`XRef`] and the object model. That is enough to
//! read `/Annots` ourselves, without another dependency.
//!
//! Three shapes of "go somewhere" are handled:
//! * `/Dest [pageRef|pageNumber /XYZ left top zoom]` — the common explicit destination;
//! * a *named* destination (`/Dest /name`, or `/A << /S /GoTo /D /name >>`) looked up in the
//!   catalog's old-style `/Dests` dictionary or the `/Names /Dests` name tree;
//! * `/A << /S /URI /URI (...) >>` — an external link, handed to the UI's `open_url` queue.

use std::collections::HashMap;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::dict::keys::{A, ANNOTS, D, DEST, DESTS, KIDS, NAMES, RECT, S, SUBTYPE, URI};
use hayro::hayro_syntax::object::{Array, Dict, MaybeRef, Name, ObjRef, Object, ObjectIdentifier, Rect as PdfBox};
use hayro::hayro_syntax::xref::XRef;

use crate::{PageSize, PdfRect, page_transform};

/// Cap on annotation dictionaries per page. A hostile file can claim millions.
const MAX_ANNOTS_PER_PAGE: usize = 4096;
/// Cap on named destinations collected from the catalog.
const MAX_NAMED_DESTS: usize = 65_536;
/// Longest name or URI we keep.
const MAX_NAME_LEN: usize = 4096;
/// Depth limit for recursive name trees.
const MAX_TREE_DEPTH: usize = 32;

/// Where a link leads.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    /// A page of this document. `top` is a distance from the page's top edge, in points, when
    /// the destination specifies one (e.g. `/XYZ`).
    Page { page: usize, top: Option<f32> },
    /// An external URL.
    Uri(String),
}

/// A clickable region of a page, in page points with a top-left origin (the same coordinates as
/// the page bitmap and the text layer).
#[derive(Clone, Debug, PartialEq)]
pub struct PdfLink {
    pub rect: PdfRect,
    pub target: LinkTarget,
}

/// A destination before its optional y coordinate has been flipped into page coordinates.
#[derive(Clone, Copy, Debug)]
struct Dest {
    page: usize,
    y_up: Option<f64>,
}

fn name_is(d: &Dict<'_>, key: &[u8], expected: &[u8]) -> bool {
    d.get::<Name<'_>>(key).is_some_and(|n| n.as_ref() == expected)
}

fn text_of(o: &Object<'_>) -> Option<String> {
    match o {
        Object::Name(n) => Some(n.as_str().to_owned()),
        Object::String(s) => Some(String::from_utf8_lossy(s.as_bytes()).into_owned()),
        _ => None,
    }
}

fn number_of(o: &Object<'_>) -> Option<f64> {
    match o {
        Object::Number(n) => Some(n.as_f64()),
        _ => None,
    }
}

/// The raw value of a dictionary key, preserving indirect references.
fn raw_value<'a>(d: &'a Dict<'a>, key: &[u8]) -> Option<MaybeRef<Object<'a>>> {
    d.entries().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v)
}

/// First object of a destination array: either an indirect page reference or a 0-based page
/// number (PDF 32000-1 §12.3.2.2 allows both).
fn dest_page(item: &MaybeRef<Object<'_>>, page_ids: &HashMap<(i32, i32), usize>, pages: usize) -> Option<usize> {
    let idx = match item {
        MaybeRef::Ref(r) => *page_ids.get(&(r.obj_number, r.gen_number))?,
        MaybeRef::NotRef(o) => {
            let n = number_of(o)?;
            if !n.is_finite() || n < 0.0 {
                return None;
            }
            usize::try_from(n.floor() as i64).ok()?
        }
    };
    (idx < pages).then_some(idx)
}

fn parse_dest_array(arr: &Array<'_>, page_ids: &HashMap<(i32, i32), usize>, pages: usize) -> Option<Dest> {
    let mut it = arr.raw_iter();
    let page = dest_page(&it.next()?, page_ids, pages)?;
    // /XYZ left top zoom: `top` is the third item. Other fit types have no usable y.
    let mut items = [None, None, None];
    for (i, slot) in items.iter_mut().enumerate() {
        let Some(v) = it.next() else { break };
        *slot = Some(v);
        if i >= 2 {
            break;
        }
    }
    let is_xyz = items.first().and_then(|x| x.as_ref()).is_some_and(|v| match v {
        MaybeRef::NotRef(Object::Name(n)) => n.as_ref() == b"XYZ",
        _ => false,
    });
    let y_up = if is_xyz {
        items.get(2).and_then(|x| x.as_ref()).and_then(|v| match v {
            MaybeRef::NotRef(o) => number_of(o),
            _ => None,
        })
    } else {
        None
    };
    Some(Dest { page, y_up })
}

/// A destination value can be a reference to an array, a direct array, or a dictionary with
/// `/D` (PDF 1.2+ named destinations).
fn dest_from_value(v: &MaybeRef<Object<'_>>, xref: &XRef, page_ids: &HashMap<(i32, i32), usize>, pages: usize) -> Option<Dest> {
    let resolve = |r: ObjRef| {
        let id = ObjectIdentifier::new(r.obj_number, r.gen_number);
        xref.get::<Array<'_>>(id)
            .and_then(|a| parse_dest_array(&a, page_ids, pages))
            .or_else(|| xref.get::<Dict<'_>>(id).and_then(|d| d.get::<Array<'_>>(D)).and_then(|a| parse_dest_array(&a, page_ids, pages)))
    };
    match v {
        MaybeRef::Ref(r) => resolve(*r),
        MaybeRef::NotRef(Object::Array(a)) => parse_dest_array(a, page_ids, pages),
        MaybeRef::NotRef(Object::Dict(d)) => d.get::<Array<'_>>(D).and_then(|a| parse_dest_array(&a, page_ids, pages)),
        _ => None,
    }
}

fn collect_old_dests(out: &mut HashMap<String, Dest>, catalog: &Dict<'_>, xref: &XRef, page_ids: &HashMap<(i32, i32), usize>, pages: usize) {
    let Some(dests) = catalog.get::<Dict<'_>>(DESTS) else { return };
    for (key, value) in dests.entries() {
        if out.len() >= MAX_NAMED_DESTS {
            break;
        }
        let k = key.as_str();
        if k.len() > MAX_NAME_LEN {
            continue;
        }
        if let Some(d) = dest_from_value(&value, xref, page_ids, pages) {
            out.insert(k.to_owned(), d);
        }
    }
}

/// Walk a `/Names` name tree: `/Kids` recursion, `/Names` alternating (string, destination).
fn collect_name_tree(
    out: &mut HashMap<String, Dest>,
    node: &Dict<'_>,
    xref: &XRef,
    page_ids: &HashMap<(i32, i32), usize>,
    pages: usize,
    depth: usize,
    budget: &mut usize,
) {
    if depth > MAX_TREE_DEPTH || out.len() >= MAX_NAMED_DESTS || *budget == 0 {
        return;
    }
    *budget -= 1;
    if let Some(names) = node.get::<Array<'_>>(NAMES) {
        let mut pending: Option<String> = None;
        for item in names.raw_iter() {
            if let Some(name) = pending.take() {
                if let Some(d) = dest_from_value(&item, xref, page_ids, pages) {
                    out.insert(name, d);
                }
            } else if let MaybeRef::NotRef(o) = &item {
                pending = text_of(o).filter(|s| s.len() <= MAX_NAME_LEN);
            }
            if out.len() >= MAX_NAMED_DESTS {
                break;
            }
        }
    }
    if let Some(kids) = node.get::<Array<'_>>(KIDS) {
        for kid in kids.iter::<Dict<'_>>() {
            collect_name_tree(out, &kid, xref, page_ids, pages, depth + 1, budget);
        }
    }
}

fn named_dests(pdf: &Pdf, page_ids: &HashMap<(i32, i32), usize>, pages: usize) -> HashMap<String, Dest> {
    let mut out = HashMap::new();
    let xref = pdf.xref();
    let Some(catalog) = xref.get::<Dict<'_>>(xref.root_id()) else { return out };
    collect_old_dests(&mut out, &catalog, xref, page_ids, pages);
    if let Some(names) = catalog.get::<Dict<'_>>(NAMES)
        && let Some(root) = names.get::<Dict<'_>>(DESTS)
    {
        let mut budget = 4096usize;
        collect_name_tree(&mut out, &root, xref, page_ids, pages, 0, &mut budget);
    }
    out
}

/// A destination value may itself be a name to look up in the named-destination table.
fn resolve_dest(
    v: &MaybeRef<Object<'_>>,
    xref: &XRef,
    page_ids: &HashMap<(i32, i32), usize>,
    named: &HashMap<String, Dest>,
    pages: usize,
) -> Option<Dest> {
    if let Some(d) = dest_from_value(v, xref, page_ids, pages) {
        return Some(d);
    }
    match v {
        MaybeRef::NotRef(o) => {
            let name = text_of(o)?;
            if name.len() > MAX_NAME_LEN {
                return None;
            }
            named.get(&name).copied()
        }
        MaybeRef::Ref(_) => None,
    }
}

fn link_dest(annot: &Dict<'_>, pdf: &Pdf, page_ids: &HashMap<(i32, i32), usize>, named: &HashMap<String, Dest>, pages: usize) -> Option<Dest> {
    let xref = pdf.xref();
    if let Some(v) = raw_value(annot, DEST)
        && let Some(d) = resolve_dest(&v, xref, page_ids, named, pages)
    {
        return Some(d);
    }
    let a = annot.get::<Dict<'_>>(A)?;
    if !name_is(&a, S, b"GoTo") {
        return None;
    }
    let v = raw_value(&a, D)?;
    resolve_dest(&v, xref, page_ids, named, pages)
}

fn external_uri(annot: &Dict<'_>) -> Option<String> {
    let a = annot.get::<Dict<'_>>(A)?;
    if !name_is(&a, S, b"URI") {
        return None;
    }
    let uri = a.get::<Object<'_>>(URI).and_then(|o| text_of(&o))?;
    (uri.len() <= MAX_NAME_LEN).then_some(uri)
}

/// All link annotations of every page, in page order. Called once when a PDF is opened.
pub fn parse_links(pdf: &Pdf, sizes: &[PageSize]) -> Vec<Vec<PdfLink>> {
    let pages = sizes.len();
    let mut page_ids = HashMap::with_capacity(pages);
    let mut transforms = Vec::with_capacity(pages);
    for (i, page) in pdf.pages().iter().enumerate() {
        if let Some(id) = page.raw().obj_id() {
            page_ids.insert((id.obj_number, id.gen_number), i);
        }
        transforms.push(page_transform(page));
    }
    let named = named_dests(pdf, &page_ids, pages);

    let mut all = Vec::with_capacity(pages);
    for (i, page) in pdf.pages().iter().enumerate() {
        let mut links = Vec::new();
        let Some(size) = sizes.get(i) else {
            all.push(links);
            continue;
        };
        let Some(annots) = page.raw().get::<Array<'_>>(ANNOTS) else {
            all.push(links);
            continue;
        };
        let t = transforms.get(i).copied().unwrap_or_default();
        for annot in annots.iter::<Dict<'_>>().take(MAX_ANNOTS_PER_PAGE) {
            if !name_is(&annot, SUBTYPE, b"Link") {
                continue;
            }
            let Some(box4) = annot.get::<PdfBox>(RECT) else { continue };
            let rect = PdfRect::from_pdf(box4, t, *size);
            if rect.is_empty() {
                continue;
            }
            let target = if let Some(uri) = external_uri(&annot) {
                LinkTarget::Uri(uri)
            } else if let Some(d) = link_dest(&annot, pdf, &page_ids, &named, pages) {
                // The destination's y belongs to the *target* page, which may be a different
                // size from the page carrying the link.
                let target_h = sizes.get(d.page).map_or(size.h, |s| s.h);
                let top = d.y_up.map(|y| (f64::from(target_h) - y).clamp(0.0, f64::from(target_h)) as f32);
                LinkTarget::Page { page: d.page, top }
            } else {
                continue;
            };
            links.push(PdfLink { rect, target });
        }
        all.push(links);
    }
    all
}

/// Hit-test helper shared with the UI: the link under a page point, if any. Smallest matching
/// rectangle wins, so a link inside a larger one stays reachable.
pub fn link_at(links: &[PdfLink], x: f32, y: f32) -> Option<&LinkTarget> {
    let mut best: Option<(&LinkTarget, f32)> = None;
    for l in links {
        if !l.rect.contains(x, y) {
            continue;
        }
        let area = l.rect.area();
        if best.is_none_or(|(_, a)| area < a) {
            best = Some((&l.target, area));
        }
    }
    best.map(|(t, _)| t)
}
