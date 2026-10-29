//! Headers and footers of the master pages in `styles.xml` (ODF 1.3 §16.10), read with the body
//! reader (#863).
use std::collections::HashSet;

use wordcraft_doc::{Document, PartKind};

use super::{Body, Ev, FBlock, Pages, SectionProps, Styles, events, get, model};

/// Most XML events kept for all headers and footers together; a slot past it is left out.
const MAX_EVENTS: usize = 1 << 20;

/// Where on a master page a header or footer goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Header,
    HeaderLeft,
    HeaderFirst,
    Footer,
    FooterLeft,
    FooterFirst,
}

impl Slot {
    /// The slot of element `q`, if it is a header or footer.
    fn of(q: &str) -> Option<Self> {
        Some(match q {
            "style:header" => Self::Header,
            "style:header-left" => Self::HeaderLeft,
            "style:header-first" | "loext:header-first" => Self::HeaderFirst,
            "style:footer" => Self::Footer,
            "style:footer-left" => Self::FooterLeft,
            "style:footer-first" | "loext:footer-first" => Self::FooterFirst,
            _ => return None,
        })
    }

    /// Whether it is a footer.
    fn footer(self) -> bool {
        matches!(self, Self::Footer | Self::FooterLeft | Self::FooterFirst)
    }

    /// Name part `id` in this slot of `sec` (a first-page one turns on a different first page).
    fn set(self, sec: &mut SectionProps, id: u32) {
        let set = if self.footer() { &mut sec.footers } else { &mut sec.headers };
        match self {
            Self::Header | Self::Footer => set.default = Some(id),
            Self::HeaderLeft | Self::FooterLeft => set.even = Some(id),
            Self::HeaderFirst | Self::FooterFirst => {
                set.first = Some(id);
                sec.title_page = true;
            }
        }
    }
}

/// The headers and footers read, in part-id order from 1, and whether one is for left pages.
#[derive(Default)]
pub(super) struct Parts {
    parts: Vec<(PartKind, Vec<FBlock>)>,
    even_odd: bool,
}

/// Read each visible header and footer of the master pages in `pages`, and name it on its master
/// page's section as part id 1, 2, … in order, so the sections the body starts carry them.
/// [`add`] puts those parts into the document.
pub(super) fn read(xml: &[u8], styles: &Styles, pages: &mut Pages, fetch: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Parts {
    let mut found: Vec<(String, Slot, Vec<Ev>)> = Vec::new();
    let mut master: Option<String> = None;
    // The slot being read, its nesting depth and whether it is kept.
    let mut slot: Option<(usize, bool)> = None;
    let mut seen = HashSet::new();
    let mut room = MAX_EVENTS;
    events(xml, |ev| {
        if let Some((depth, keep)) = &mut slot {
            match &ev {
                Ev::End(_) if *depth == 0 => {
                    slot = None;
                    return;
                }
                Ev::Start(..) => *depth += 1,
                Ev::End(_) => *depth -= 1,
                Ev::Text(_) => {}
            }
            if *keep && room == 0 {
                *keep = false;
                found.pop();
            }
            if *keep && let Some((.., evs)) = found.last_mut() {
                room -= 1;
                evs.push(ev);
            }
            return;
        }
        match ev {
            Ev::Start(q, a) if q == "style:master-page" => {
                master = get(&a, "style:name").filter(|n| pages.masters.contains_key(*n)).map(str::to_string);
            }
            Ev::End(q) if q == "style:master-page" => master = None,
            Ev::Start(q, a) => {
                if let (Some(m), Some(kind)) = (&master, Slot::of(&q)) {
                    let keep = get(&a, "style:display") != Some("false") && seen.insert((m.clone(), q.clone()));
                    if keep {
                        found.push((m.clone(), kind, Vec::new()));
                    }
                    slot = Some((0, keep));
                }
            }
            _ => {}
        }
    });
    let none = Pages::default();
    let mut out = Parts::default();
    let mut areas = HashSet::new();
    for (name, kind, evs) in found {
        areas.insert((name.clone(), kind.footer()));
        let mut reader = Body::new(styles, &none, fetch);
        reader.in_text = true;
        for ev in evs {
            reader.feed(ev);
        }
        out.parts.push((if kind.footer() { PartKind::Footer } else { PartKind::Header }, reader.take_blocks()));
        if let Some(sec) = pages.masters.get_mut(&name) {
            kind.set(sec, out.parts.len() as u32);
        }
        out.even_odd |= matches!(kind, Slot::HeaderLeft | Slot::FooterLeft);
    }
    for (name, footer) in areas {
        let height = pages.hf.get(&name).and_then(|h| h.get(usize::from(footer)).copied().flatten());
        if let (Some(sec), Some(h)) = (pages.masters.get_mut(&name), height) {
            make_room(sec, footer, h);
        }
    }
    // Each master page stands alone, but a section without its own header or footer continues
    // the previous one's: give those an empty one, and left pages the right pages' one.
    for footer in [false, true] {
        let pick = |s: &SectionProps| if footer { s.footers } else { s.headers };
        if !pages.masters.values().any(|s| pick(s).default.is_some() || pick(s).even.is_some()) {
            continue;
        }
        let mut empty = None;
        for sec in pages.masters.values_mut() {
            let set = if footer { &mut sec.footers } else { &mut sec.headers };
            if set.default.is_none() {
                set.default = Some(*empty.get_or_insert_with(|| {
                    out.parts.push((if footer { PartKind::Footer } else { PartKind::Header }, Vec::new()));
                    out.parts.len() as u32
                }));
            }
            if out.even_odd && set.even.is_none() {
                set.even = set.default;
            }
        }
    }
    if let Some(first) = pages.first_name.as_ref().and_then(|n| pages.masters.get(n)) {
        pages.first = Some(first.clone());
    }
    out
}

/// Put the parts [`read`] found into `doc`, and point its sections at them: [`read`] numbered
/// them 1, 2, … in order, but `doc` may already have parts of its own (the body's text boxes).
pub(super) fn add(doc: &mut Document, parts: Parts) {
    let mut ids = Vec::with_capacity(parts.parts.len());
    for (kind, blocks) in parts.parts {
        // The blocks first, so the text boxes in a header get ids of their own.
        let blocks = model::part_blocks(doc, &blocks);
        ids.push(doc.add_part(kind, blocks));
    }
    let real = |n: Option<u32>| n.and_then(|n| n.checked_sub(1)).and_then(|i| ids.get(i as usize).copied());
    let fix = |s: &mut SectionProps| {
        for set in [&mut s.headers, &mut s.footers] {
            set.default = real(set.default);
            set.first = real(set.first);
            set.even = real(set.even);
        }
    };
    for b in &mut doc.body {
        if let wordcraft_doc::Block::Para(p) = &**b
            && p.section.is_some()
            && let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b)
            && let Some(s) = p.section.as_deref_mut()
        {
            fix(s);
        }
    }
    fix(&mut doc.last_section);
    doc.settings.even_odd_headers |= parts.even_odd;
}

/// ODF measures the page margin to the header (footer) and puts the body text below the header
/// area, `height` points tall; Word measures the margin to the body text and the header distance
/// to the header. Set `sec`'s header (footer) distance and margin to match, leaving the body at
/// least half an inch.
fn make_room(sec: &mut SectionProps, footer: bool, height: f32) {
    let (edge, other) = if footer { (sec.margin_bottom, sec.margin_top) } else { (sec.margin_top, sec.margin_bottom) };
    let most = (sec.page_h - other - 36.0).max(edge);
    let margin = (edge + height).min(most);
    if footer {
        sec.footer = edge;
        sec.margin_bottom = margin;
    } else {
        sec.header = edge;
        sec.margin_top = margin;
    }
}
