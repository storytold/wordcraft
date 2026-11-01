//! Print Markup: the comments in balloons in a markup area on the right of each page, joined to
//! the text they comment on, the way Word prints and exports a document "showing markup".
//!
//! [`crate::layout`] calls [`add`] when [`crate::LayoutOptions::comment_balloons`] is on: each
//! page grows by [`MARKUP_AREA`] ([`crate::Page::markup`]) and gets, for each comment anchored on
//! it, a card (author colour stripe and border) holding the author's name and the comment's own
//! laid-out text, with its replies under it, and a leader from the anchor to the card. The cards
//! are ordinary placed items, so the PDF writer, the renderer and print draw them unchanged
//! (the PDF writer scales each page down to its paper size). The editor draws its own interactive
//! balloons instead.
//!
//! Hostile documents: comment, reply and parent-chain counts are capped; cards that don't fit
//! are clipped to the page, and when a page has more comments than its markup area can hold the
//! ones that find no room are left out.

use std::collections::{BTreeMap, HashSet};

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Border, BorderStyle, CharProps, Rgb, TextColor};
use wordcraft_doc::{Document, Paragraph, Pos, StoryRef};
use wordcraft_geom::Rect;

use crate::display::{author_order, revision_color};
use crate::{BoxInput, BoxLayout, Ctx, DocLayout, Edges, Placed, fit_box, layout_box, push_para};

/// Width of the markup area beside each page (points): the editor's.
pub const MARKUP_AREA: f32 = 216.0;
/// Space between the page and the cards, and on their right.
const SIDE: f32 = 9.0;
/// Space between cards.
const GAP: f32 = 6.0;
/// Padding inside a card.
const PAD: f32 = 5.0;
/// The author-colour stripe on a card's left.
const STRIPE: f32 = 3.0;
/// Space kept above the first and below the last card on a page.
const EDGE: f32 = 12.0;
/// Smallest card height when cards have to be clipped to fit a page.
const MIN_CARD: f32 = 18.0;
const MAX_BALLOONS: usize = 5_000;
const MAX_REPLIES: usize = 200;
const MAX_PARENT_CHAIN: usize = 64;

/// Path given to the synthetic author-name paragraphs: no paragraph of the comment's story has it.
const HEADER_PATH: u32 = u32::MAX;

/// One comment's balloon before it is positioned: items relative to the card's top-left.
struct Card {
    page: usize,
    /// The anchor: the comment's start, at the foot of its line.
    ax: f32,
    ay: f32,
    items: Vec<Placed>,
    h: f32,
    color: Rgb,
}

/// The thread a comment belongs to: the comment at the top of its parent chain.
fn root(doc: &Document, id: u32) -> u32 {
    let mut cur = id;
    for _ in 0..MAX_PARENT_CHAIN {
        match doc.comments.get(&cur).and_then(|c| c.parent) {
            Some(p) if p != cur && doc.comments.contains_key(&p) => cur = p,
            _ => break,
        }
    }
    cur
}

/// Each top-level comment anchored in the body, with where it starts, in document order. Replies
/// have no balloon of their own: they go under the comment they answer.
pub fn anchors(doc: &Document) -> Vec<(u32, Pos)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for off in p.object_offsets() {
            let Some(InlineObject::CommentStart { id }) = p.object_at(off) else { continue };
            if !doc.comments.contains_key(id) || root(doc, *id) != *id || !seen.insert(*id) {
                continue;
            }
            out.push((*id, Pos { story: StoryRef::Body, path: path.clone(), off }));
            if out.len() >= MAX_BALLOONS {
                return out;
            }
        }
    }
    out
}

/// Replies by the top-level comment they belong to, oldest (lowest id) first.
pub fn replies(doc: &Document) -> BTreeMap<u32, Vec<u32>> {
    let mut m: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for id in doc.comments.keys() {
        let r = root(doc, *id);
        if r != *id {
            let v = m.entry(r).or_default();
            if v.len() < MAX_REPLIES {
                v.push(*id);
            }
        }
    }
    m
}

fn author_name(a: &str) -> &str {
    if a.trim().is_empty() { "Author" } else { a }
}

/// Widen every page by the markup area and place the comment balloons in it.
pub(crate) fn add(ctx: &mut Ctx, lay: &mut DocLayout) {
    let doc = ctx.doc;
    if doc.comments.is_empty() || lay.pages.is_empty() {
        return;
    }
    let anchors = anchors(doc);
    if anchors.is_empty() {
        return;
    }
    let threads = replies(doc);
    let card_w = MARKUP_AREA - 2.0 * SIDE;
    let inner = card_w - STRIPE - 2.0 * PAD;
    let mut cards: Vec<Card> = Vec::new();
    // The comment authors in order of first appearance (for their colours), once for every card.
    let authors = author_order(doc.comments.values().map(|c| c.author.as_str()));
    for (id, pos) in anchors {
        let Some(caret) = lay.caret_on(&pos, 0) else { continue };
        let Some(c) = doc.comments.get(&id) else { continue };
        let color = revision_color(authors.get(c.author.as_str()).copied().unwrap_or(0));
        let mut thread = vec![id];
        thread.extend(threads.get(&id).into_iter().flatten().copied());
        let x = STRIPE + PAD;
        let mut items = Vec::new();
        let mut y = PAD;
        for (k, cid) in thread.iter().enumerate() {
            let Some(cm) = doc.comments.get(cid) else { continue };
            let story = StoryRef::Part(cm.part);
            if k > 0 {
                // A reply starts: a hairline between it and what it answers.
                y += 3.0;
                let border = Border { style: BorderStyle::Single, width: 0.25, color: Some(Rgb(0xC8, 0xC8, 0xC8)), space: 0.0 };
                items.push(Placed::Rule { x0: x, y0: y, x1: x + inner, y1: y, border });
                y += 3.0;
            }
            // Who wrote it, in their colour.
            let head = Paragraph::with_text(
                author_name(&cm.author),
                CharProps { bold: Some(true), size: Some(9.0), color: Some(TextColor::Rgb(color)), ..Default::default() },
            );
            let pl = ctx.para_labelled(&head, inner, None, &[], None);
            let lead = pl.lines.first().map_or(0.0, |l| l.top.max(0.0));
            push_para(&mut items, (story, &[HEADER_PATH]), &pl, (0, pl.lines.len()), (x, y + lead), inner, Edges::own(&pl, 0, pl.lines.len()));
            y += pl.height + 2.0;
            // The comment itself, as written (its own formatting).
            if let Some(part) = doc.parts.get(&cm.part) {
                let BoxLayout { items: body, height, .. } = layout_box(ctx, BoxInput::other(story, &part.blocks, inner, 1, None));
                for mut it in body {
                    it.translate(x, y);
                    items.push(it);
                }
                y += height;
            }
        }
        cards.push(Card { page: caret.page, ax: caret.x, ay: caret.top + caret.height, items, h: y + PAD, color });
    }
    // Every page gets the markup area, so the pages keep one size.
    let widths: Vec<f32> = lay.pages.iter().map(|p| p.w).collect();
    for p in &mut lay.pages {
        p.w += MARKUP_AREA;
        p.markup = MARKUP_AREA;
    }
    let mut by_page: BTreeMap<usize, Vec<Card>> = BTreeMap::new();
    for c in cards {
        by_page.entry(c.page).or_default().push(c);
    }
    for (pi, mut list) in by_page {
        let (Some(page), Some(&pw)) = (lay.pages.get_mut(pi), widths.get(pi)) else { continue };
        list.sort_by(|a, b| a.ay.total_cmp(&b.ay).then(a.ax.total_cmp(&b.ax)));
        let (top_min, bottom) = (EDGE, (page.h - EDGE).max(EDGE + MIN_CARD));
        // Each card as near its anchor as the ones above allow; then pushed up from the page
        // foot; then down again below the top, clipping what still doesn't fit.
        let mut tops: Vec<f32> = Vec::with_capacity(list.len());
        let mut next = top_min;
        for c in &list {
            let t = (c.ay - 10.0).max(next);
            tops.push(t);
            next = t + c.h + GAP;
        }
        let mut limit = bottom;
        for (t, c) in tops.iter_mut().zip(&list).rev() {
            if *t + c.h > limit {
                *t = limit - c.h;
            }
            limit = *t - GAP;
        }
        let mut next = top_min;
        let x0 = pw + SIDE;
        for (c, t) in list.into_iter().zip(tops) {
            let top = t.max(next);
            // No room left beside this page (dozens of comments on it): the rest are left out
            // rather than drawn off the paper.
            if top + MIN_CARD > bottom + 0.5 {
                break;
            }
            let avail = bottom - top;
            let (items, h) = if c.h > avail { (fit_box(c.items, avail - PAD), avail) } else { (c.items, c.h) };
            next = top + h + GAP;
            let line = |style, width: f32| Border { style, width, color: Some(c.color), space: 0.0 };
            // Leader: along the text from the anchor to the page edge, then to the card.
            page.items.push(Placed::Rule { x0: c.ax, y0: c.ay, x1: pw, y1: c.ay, border: line(BorderStyle::Dashed, 0.5) });
            page.items.push(Placed::Rule { x0: pw, y0: c.ay, x1: x0, y1: top + 8.0, border: line(BorderStyle::Single, 0.5) });
            // The card: white, outlined and striped in the author's colour.
            let card = Rect::new(x0, top, card_w, h);
            page.items.push(Placed::Fill { rect: card, color: Rgb(0xFF, 0xFF, 0xFF) });
            page.items.push(Placed::Fill { rect: Rect::new(x0, top, STRIPE, h), color: c.color });
            let (l, r, b) = (card.x, card.right(), card.bottom());
            for (ax, ay, bx, by) in [(l, top, r, top), (r, top, r, b), (l, b, r, b), (l, top, l, b)] {
                page.items.push(Placed::Rule { x0: ax, y0: ay, x1: bx, y1: by, border: line(BorderStyle::Single, 0.5) });
            }
            for mut it in items {
                it.translate(x0, top);
                page.items.push(it);
            }
        }
    }
}
