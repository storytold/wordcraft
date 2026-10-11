//! A text box's text (Shape Format › Text): which way it runs (Text Direction), where it sits
//! between the box's edges (Align Text), and linked boxes, whose text continues from one box in
//! the next (Create Link).
//!
//! The text is laid out in the box's own frame (lines `len` long, stacking `across` deep), cut to
//! what fits, aligned, then turned onto the page as a table cell's turned text is. A chain of
//! linked boxes lays out the first box's story and gives each box the next stretch of it: the
//! sizes come from the boxes' shapes, so a box shows the same text wherever it is laid out.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use wordcraft_doc::props::{TextDirection, TextVert, VAlign};
use wordcraft_doc::{Block, Blocks, Path, StoryRef, TextBody};
use wordcraft_geom::Rect;

use crate::{BOX_INSET_X, BOX_INSET_Y, Ctx, Placed, fit_box, layout_box};

/// Linked text boxes for one layout, worked out when a linked box is first laid out.
#[derive(Default)]
pub(crate) struct Chains {
    built: bool,
    /// Box → (chain, place in it).
    member: HashMap<u32, (usize, usize)>,
    chains: Vec<Vec<u32>>,
    /// Each chain's text split over its boxes: per box, items in its unturned frame.
    flows: HashMap<usize, Vec<Vec<Placed>>>,
}

impl Chains {
    fn build(&mut self, doc: &wordcraft_doc::Document) {
        if self.built {
            return;
        }
        self.built = true;
        if !doc.parts.values().any(|p| p.body.next.is_some()) {
            return;
        }
        self.chains = doc.text_box_chains();
        for (ci, c) in self.chains.iter().enumerate() {
            for (k, id) in c.iter().enumerate() {
                self.member.insert(*id, (ci, k));
            }
        }
    }
}

/// The story text box `id` shows: the first box's, for a box linked from another.
pub(crate) fn shown_story(ctx: &mut Ctx, id: u32) -> u32 {
    ctx.chains.build(ctx.doc);
    match ctx.chains.member.get(&id) {
        Some((ci, _)) => ctx.chains.chains.get(*ci).and_then(|c| c.first()).copied().unwrap_or(id),
        None => id,
    }
}

/// Line length and the depth lines stack in, for a `w` × `h` box whose text runs `vert`.
fn frame(vert: TextVert, w: f32, h: f32) -> (f32, f32) {
    let (w, h) = (wordcraft_geom::finite(w), wordcraft_geom::finite(h));
    let (along, across) = match vert.turn() {
        TextDirection::Horizontal => (w - 2.0 * BOX_INSET_X, h - 2.0 * BOX_INSET_Y),
        _ => (h - 2.0 * BOX_INSET_Y, w - 2.0 * BOX_INSET_X),
    };
    (along.max(12.0), across)
}

/// The width stacked text is laid out in (a paragraph's narrowest).
const STACK_WIDTH: f32 = 12.0;

/// Lay `blocks` of story `id` out in lines `len` long; stacked text one letter to a line.
fn lay(ctx: &mut Ctx, id: u32, blocks: &Blocks, len: f32, vert: TextVert, depth: usize) -> Vec<Placed> {
    let story = StoryRef::Part(id);
    if vert == TextVert::Stacked {
        // One letter to a line: words may break anywhere, and each letter takes the narrowest
        // line a paragraph gets (the spacing after it doesn't show).
        let stacked: Blocks = blocks
            .iter()
            .map(|b| match &**b {
                Block::Para(p) => {
                    let mut p = p.clone();
                    p.props.word_wrap = Some(false);
                    for r in &mut p.runs {
                        r.props.spacing = Some(STACK_WIDTH);
                    }
                    p.touch();
                    Arc::new(Block::Para(p))
                }
                _ => b.clone(),
            })
            .collect();
        return layout_box(ctx, story, &stacked, &[], STACK_WIDTH, None, depth + 1, None).items;
    }
    layout_box(ctx, story, blocks, &[], len, None, depth + 1, None).items
}

/// The items showing text box `id` in its frame `rect` (page coordinates): its text, or its
/// share of a linked chain's, cut to what fits, aligned and turned as the box says.
pub(crate) fn box_items(ctx: &mut Ctx, id: u32, rect: Rect, depth: usize) -> Vec<Placed> {
    let body = ctx.doc.parts.get(&id).map(|p| p.body).unwrap_or_default();
    let (len, across) = frame(body.vert, rect.w, rect.h);
    let items = match chain_share(ctx, id, depth) {
        Some(items) => items,
        None => {
            if !ctx.boxes.enter(id) {
                return Vec::new();
            }
            let blocks = ctx.doc.parts.get(&id).map(|p| p.blocks.clone()).unwrap_or_default();
            let inner = lay(ctx, id, &blocks, len, body.vert, depth);
            ctx.boxes.leave();
            // Text that doesn't fit inside the margins is hidden, as in Word.
            fit_box(inner, across)
        }
    };
    place(items, &body, rect, across)
}

/// Move items laid out in a box's frame onto the page: aligned in the `across` room (top,
/// middle, bottom; for turned text, from the side its lines start on) and turned.
fn place(mut items: Vec<Placed>, body: &TextBody, rect: Rect, across: f32) -> Vec<Placed> {
    let used = extent(&items);
    let off = match body.anchor {
        VAlign::Top => 0.0,
        VAlign::Center => ((across - used) / 2.0).max(0.0),
        VAlign::Bottom => (across - used).max(0.0),
    };
    let turn = body.vert.turn();
    let (ox, oy) = match turn {
        TextDirection::Horizontal => (rect.x + BOX_INSET_X, rect.y + BOX_INSET_Y + off),
        // Top-to-bottom lines stack from the right edge, bottom-to-top ones from the left.
        TextDirection::Down => (rect.right() - BOX_INSET_X - off, rect.y + BOX_INSET_Y),
        TextDirection::Up => (rect.x + BOX_INSET_X + off, rect.bottom() - BOX_INSET_Y),
    };
    for it in &mut items {
        it.turn(turn, ox, oy);
    }
    items
}

/// How deep laid-out items reach below their frame's top.
fn extent(items: &[Placed]) -> f32 {
    let mut bottom = 0.0f32;
    for it in items {
        let b = match it {
            _ if it.turned_bounds().is_some() => it.turned_bounds().map_or(0.0, |r| r.bottom()),
            Placed::Lines { para, l0, l1, y, .. } => {
                let (Some(first), Some(last)) = (para.lines.get(*l0), l1.checked_sub(1).and_then(|k| para.lines.get(k))) else { continue };
                y + last.top + last.height - first.top
            }
            Placed::Rule { y0, y1, .. } => y0.max(*y1),
            Placed::Fill { rect, .. }
            | Placed::Image { rect, .. }
            | Placed::Shape { rect, .. }
            | Placed::Graphic { rect, .. }
            | Placed::Cell { rect, .. }
            | Placed::Object { rect, .. } => rect.bottom(),
        };
        if b.is_finite() {
            bottom = bottom.max(b);
        }
    }
    bottom
}

/// Box `id`'s share of its chain's text (items in its unturned frame), if it is linked.
fn chain_share(ctx: &mut Ctx, id: u32, depth: usize) -> Option<Vec<Placed>> {
    ctx.chains.build(ctx.doc);
    let (ci, k) = *ctx.chains.member.get(&id)?;
    if !ctx.chains.flows.contains_key(&ci) {
        let chain = ctx.chains.chains.get(ci).cloned().unwrap_or_default();
        let flow = flow(ctx, &chain, depth);
        ctx.chains.flows.insert(ci, flow);
    }
    Some(ctx.chains.flows.get(&ci).and_then(|f| f.get(k)).cloned().unwrap_or_default())
}

/// Where a box's text starts: the line holding byte `byte` of paragraph `path`.
struct Cut {
    path: Path,
    byte: usize,
}

/// Split the text of `chain`'s first box over its boxes.
fn flow(ctx: &mut Ctx, chain: &[u32], depth: usize) -> Vec<Vec<Placed>> {
    let Some(&head) = chain.first() else { return Vec::new() };
    if !ctx.boxes.enter(head) {
        return vec![Vec::new(); chain.len()];
    }
    let ids: BTreeSet<u32> = chain.iter().copied().collect();
    let sizes = ctx.doc.text_box_sizes(&ids);
    let blocks = ctx.doc.parts.get(&head).map(|p| p.blocks.clone()).unwrap_or_default();
    // Each box lays the whole story out at its own line length (the same list numbers every
    // time), sharing a layout with the boxes before it of the same length.
    let snap = (ctx.counters.clone(), ctx.eq_count);
    let mut after = None;
    let mut layouts: Vec<((u32, TextVert), Vec<Placed>)> = Vec::new();
    let mut out = Vec::with_capacity(chain.len());
    // Where the next box's text starts: `Some(None)` at the beginning, `None` when all is shown.
    let mut start: Option<Option<Cut>> = Some(None);
    for id in chain {
        let Some(cut) = start.take() else {
            out.push(Vec::new());
            continue;
        };
        let body = ctx.doc.parts.get(id).map(|p| p.body).unwrap_or_default();
        let (w, h) = sizes.get(id).copied().unwrap_or((0.0, 0.0));
        let (len, across) = frame(body.vert, w, h);
        let key = (len.to_bits(), body.vert);
        let items = match layouts.iter().find(|(k, _)| *k == key) {
            Some((_, items)) => items.clone(),
            None => {
                (ctx.counters, ctx.eq_count) = (snap.0.clone(), snap.1);
                let items = lay(ctx, head, &blocks, len, body.vert, depth);
                after.get_or_insert_with(|| (ctx.counters.clone(), ctx.eq_count));
                layouts.push((key, items.clone()));
                items
            }
        };
        let items = match &cut {
            None => items,
            Some(cut) => match cut_y(&items, cut) {
                Some(top) => from_y(items, top),
                None => {
                    out.push(Vec::new());
                    continue;
                }
            },
        };
        let (kept, next) = fit_flow(items, across);
        start = next.map(Some);
        out.push(kept);
    }
    if let Some(a) = after {
        (ctx.counters, ctx.eq_count) = a;
    }
    ctx.boxes.leave();
    out
}

/// Where `cut` is in a layout: the top of the line holding its byte.
fn cut_y(items: &[Placed], cut: &Cut) -> Option<f32> {
    items.iter().find_map(|it| match it {
        Placed::Lines { path, para, l0, l1, y, .. } if *path == cut.path => {
            let first = para.lines.get(*l0)?;
            let k = (*l0..*l1).find(|k| para.lines.get(*k).is_some_and(|l| l.stop > cut.byte)).unwrap_or(l1.saturating_sub(1).max(*l0));
            para.lines.get(k).map(|l| y + l.top - first.top)
        }
        _ => None,
    })
}

/// The items from `top` down, moved up to start at 0.
fn from_y(items: Vec<Placed>, top: f32) -> Vec<Placed> {
    let lim = top - 0.5;
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        if let Some(r) = it.turned_bounds() {
            if r.y >= lim {
                out.push(it);
            }
            continue;
        }
        match it {
            Placed::Lines { story, path, para, l0, l1, x, y, turn } => {
                let Some(first) = para.lines.get(l0) else { continue };
                let Some(k) = (l0..l1).find(|k| para.lines.get(*k).is_some_and(|l| y + l.top - first.top >= lim)) else { continue };
                let ky = para.lines.get(k).map_or(y, |l| y + l.top - first.top);
                out.push(Placed::Lines { story, path, para, l0: k, l1, x, y: ky, turn });
            }
            Placed::Fill { rect, color } if rect.bottom() > lim => {
                let y0 = rect.y.max(top);
                out.push(Placed::Fill { rect: Rect::new(rect.x, y0, rect.w, rect.bottom() - y0), color });
            }
            Placed::Rule { y0, y1, .. } if y0.max(y1) >= lim => out.push(it),
            Placed::Image { rect, .. }
            | Placed::Shape { rect, .. }
            | Placed::Graphic { rect, .. }
            | Placed::Cell { rect, .. }
            | Placed::Object { rect, .. }
                if rect.y >= lim =>
            {
                out.push(it)
            }
            _ => {}
        }
    }
    for it in &mut out {
        it.translate(0.0, -top);
    }
    out
}

/// What fits in `across` (see [`fit_box`]), and where the text left over starts.
fn fit_flow(items: Vec<Placed>, across: f32) -> (Vec<Placed>, Option<Cut>) {
    let limit = across + 0.5;
    let mut kept_line = false;
    let mut cut: Option<(f32, Cut)> = None;
    for it in &items {
        if it.turned_bounds().is_some() {
            continue;
        }
        let Placed::Lines { path, para, l0, l1, y, .. } = it else { continue };
        let Some(first) = para.lines.get(*l0) else { continue };
        for k in *l0..*l1 {
            let Some(l) = para.lines.get(k) else { break };
            let ly = y + l.top - first.top;
            if ly + l.height > limit && kept_line {
                if cut.as_ref().is_none_or(|(cy, _)| ly < *cy) {
                    cut = Some((ly, Cut { path: path.clone(), byte: l.start }));
                }
                break;
            }
            kept_line = true;
        }
    }
    (fit_box(items, across), cut.map(|(_, c)| c))
}
