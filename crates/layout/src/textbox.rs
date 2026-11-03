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

use crate::{BoxInput, Ctx, Placed, fit_box, layout_box};

/// Linked text boxes for one layout, worked out when a linked box is first laid out.
#[derive(Default)]
pub(crate) struct Chains {
    built: bool,
    /// Box → (chain, place in it).
    member: HashMap<u32, (usize, usize)>,
    chains: Vec<Vec<u32>>,
    /// Each chain's text split over its boxes: per box, items in its unturned frame.
    flows: HashMap<usize, Vec<Vec<Placed>>>,
    /// New entries can be discarded after a sizing probe without cloning placed items.
    flow_order: Vec<usize>,
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

/// Line length and the depth lines stack in, for a `w` × `h` box whose text runs as `body`
/// says, inside its internal margins.
fn frame(body: &TextBody, w: f32, h: f32) -> (f32, f32) {
    let (w, h) = (wordcraft_geom::finite(w), wordcraft_geom::finite(h));
    let [l, t, r, b] = body.insets_pt();
    let (along, across) = match body.vert.turn() {
        TextDirection::Horizontal => (w - l - r, h - t - b),
        _ => (h - t - b, w - l - r),
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
        return layout_box(ctx, BoxInput::other(story, &stacked, STACK_WIDTH, depth + 1, None)).items;
    }
    layout_box(ctx, BoxInput::other(story, blocks, len, depth + 1, None)).items
}

// Allow repeated probe/placement visits, but cap shared graphs separately from the
// 65 actual expansions. Exhaustion keeps the saved frame; this is not a parity bound.
const MAX_MEASURE_BOXES: usize = (wordcraft_doc::BoxBudget::MAX_NESTED + 1) * (1 << wordcraft_doc::MAX_TEXT_BOX_DEPTH);

#[cfg(test)]
thread_local! {
    static MEASURE_COUNTS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
    static COPIED_BLOCKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn reset_measure(ctx: &mut Ctx) {
    ctx.measure_left = MAX_MEASURE_BOXES;
    ctx.measure_exhausted = false;
    #[cfg(test)]
    MEASURE_COUNTS.with(|counts| counts.borrow_mut().push(0));
}

/// Charge before preparation or reuse. This allowance is never part of a semantic snapshot.
fn take_measure(ctx: &mut Ctx) -> bool {
    if ctx.measure_depth == 0 {
        return true;
    }
    if ctx.measure_left == 0 {
        ctx.measure_exhausted = true;
        return false;
    }
    ctx.measure_left -= 1;
    #[cfg(test)]
    MEASURE_COUNTS.with(|counts| {
        if let Some(last) = counts.borrow_mut().last_mut() {
            *last = last.saturating_add(1);
        }
    });
    true
}

fn enter_box(ctx: &mut Ctx, id: u32) -> bool {
    if ctx.measure_depth == 0 && !ctx.boxes.inside() {
        reset_measure(ctx);
    }
    take_measure(ctx) && ctx.boxes.enter(id)
}

/// Called only after admission, so rejected probes never copy a long story's block vector.
fn box_blocks(ctx: &Ctx, id: u32) -> Blocks {
    let Some(part) = ctx.doc.parts.get(&id) else { return Vec::new() };
    #[cfg(test)]
    if ctx.measure_depth > 0 {
        COPIED_BLOCKS.with(|count| count.set(count.get().saturating_add(part.blocks.len())));
    }
    part.blocks.clone()
}

/// Speculative layout keeps the active cycle/depth guards, but consumes no expansion or numbering.
/// Only cache entries created by this probe are removed; existing linked flows remain counted.
fn measure(ctx: &mut Ctx, id: u32, len: f32, vert: TextVert, depth: usize) -> Option<Vec<Placed>> {
    if ctx.measure_depth == 0 && !ctx.boxes.inside() {
        reset_measure(ctx);
    }
    let budget = ctx.boxes.clone();
    ctx.measure_depth += 1;
    if !enter_box(ctx, id) {
        ctx.measure_depth -= 1;
        return None;
    }
    let snap = (ctx.counters.clone(), ctx.eq_count);
    let flows = ctx.chains.flow_order.len();
    let fitted = ctx.fitted_order.len();
    let blocks = box_blocks(ctx, id);
    let items = lay(ctx, id, &blocks, len, vert, depth);
    (ctx.counters, ctx.eq_count, ctx.boxes) = (snap.0, snap.1, budget);
    while ctx.chains.flow_order.len() > flows {
        if let Some(key) = ctx.chains.flow_order.pop() {
            ctx.chains.flows.remove(&key);
        }
    }
    // A nested inline probe may have exhausted a temporary budget. Never retain that height.
    while ctx.fitted_order.len() > fitted {
        if let Some(key) = ctx.fitted_order.pop() {
            ctx.fitted.remove(&key);
        }
    }
    ctx.measure_depth -= 1;
    (!ctx.measure_exhausted).then_some(items)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FrameKey {
    id: u32,
    len: u32,
    direct: bool,
    depth: usize,
    grid: (u32, u32, u32, bool),
    fitted: usize,
}

/// Only outer measurements of source-stable graphs can share an extent.
#[derive(Default)]
pub(super) struct FrameCache {
    eligible: Option<std::collections::HashSet<u32>>,
    extents: HashMap<FrameKey, Option<f32>>,
}

impl FrameCache {
    fn supports(&mut self, doc: &wordcraft_doc::Document, id: u32) -> bool {
        if self.eligible.is_none() {
            let mut bad = std::collections::HashSet::new();
            let mut parents: HashMap<u32, Vec<u32>> = HashMap::new();
            for (&id, part) in &doc.parts {
                if part.kind != wordcraft_doc::PartKind::TextBox {
                    bad.insert(id);
                    continue;
                }
                if let Some(next) = part.body.next {
                    // Header/footer probes replace flow maps, so linked graphs keep fresh measurements.
                    bad.insert(id);
                    parents.entry(next).or_default().push(id);
                    parents.entry(id).or_default().push(next);
                }
                for block in &part.blocks {
                    let Block::Para(p) = block.as_ref() else {
                        bad.insert(id);
                        continue;
                    };
                    if p.props.numbering.or_else(|| doc.styles.resolve_para(&p.props).numbering).is_some_and(|n| n.num != 0) {
                        bad.insert(id);
                    }
                    for obj in &p.objects {
                        match obj {
                            wordcraft_doc::para::InlineObject::Shape { story: Some(child), .. } => parents.entry(*child).or_default().push(id),
                            wordcraft_doc::para::InlineObject::Shape { story: None, .. }
                            | wordcraft_doc::para::InlineObject::Image { .. }
                            | wordcraft_doc::para::InlineObject::BookmarkStart { .. }
                            | wordcraft_doc::para::InlineObject::BookmarkEnd { .. } => {}
                            // Fields, counters and unclassified compound content keep their current context.
                            _ => {
                                bad.insert(id);
                            }
                        }
                    }
                }
            }
            // Reverse reachability handles shared graphs and cycles once, without expanding stories.
            let mut pending: Vec<_> = bad.iter().copied().collect();
            while let Some(child) = pending.pop() {
                for &parent in parents.get(&child).into_iter().flatten() {
                    if bad.insert(parent) {
                        pending.push(parent);
                    }
                }
            }
            self.eligible = Some(doc.parts.keys().copied().filter(|id| !bad.contains(id)).collect());
        }
        self.eligible.as_ref().is_some_and(|eligible| eligible.contains(&id))
    }
}

/// Measure an unlinked horizontal auto-fit shape before positioning and wrapping it.
pub(super) fn fit_size(ctx: &mut Ctx, obj: &wordcraft_doc::para::InlineObject, w: f32, h: f32, depth: usize) -> (f32, f32) {
    let Some(id) = obj.text_box() else { return (w, h) };
    let Some(part) = ctx.doc.parts.get(&id) else { return (w, h) };
    let body = part.body;
    if !body.auto_fit || body.vert != TextVert::Horz {
        return (w, h);
    }
    ctx.chains.build(ctx.doc);
    if ctx.chains.member.contains_key(&id) {
        return (w, h);
    }
    let (len, _) = frame(&body, w, h);
    let direct = direct_parent(ctx, obj, id);
    let key = FrameKey { id, len: len.to_bits(), direct, depth, grid: ctx.grid.key(), fitted: ctx.fitted_order.len() };
    // Nested calls retain their active cycle/depth/work guards and cannot create cache entries.
    let reusable = ctx.measure_depth == 0 && !ctx.boxes.inside() && ctx.frames.supports(ctx.doc, id);
    let used = match reusable.then(|| ctx.frames.extents.get(&key).copied()).flatten() {
        Some(used) => used,
        None => {
            let used = measure(ctx, id, len, body.vert, depth).map(|mut items| {
                if direct {
                    let owners = suppressed_rects(ctx, id, &items);
                    items.retain(|item| !item_owner(item).is_some_and(|owner| owners.contains(&owner)));
                }
                extent(&items)
            });
            if reusable {
                ctx.frames.extents.insert(key, used);
            }
            used
        }
    };
    let Some(used) = used else { return (w, h) };
    let [_, top, _, bottom] = body.insets_pt();
    // Word fits the frame to the text and both margins, including a visible outline's width.
    let outline = match obj {
        wordcraft_doc::para::InlineObject::Shape { stroke: Some(_), stroke_width, .. } => wordcraft_geom::finite(*stroke_width).max(0.0),
        _ => 0.0,
    };
    (w, used + top + bottom + outline)
}

/// The items showing text box `id` in its frame `rect` (page coordinates): its text, or its
/// share of a linked chain's, cut to what fits, aligned and turned as the box says.
pub(crate) fn box_items(ctx: &mut Ctx, id: u32, rect: Rect, depth: usize, parent: Option<&wordcraft_doc::para::InlineObject>) -> Vec<Placed> {
    let body = ctx.doc.parts.get(&id).map(|p| p.body).unwrap_or_default();
    let (len, across) = frame(&body, rect.w, rect.h);
    let mut items = match chain_share(ctx, id, depth) {
        Some(items) => items,
        None => {
            if !enter_box(ctx, id) {
                return Vec::new();
            }
            let blocks = box_blocks(ctx, id);
            let inner = lay(ctx, id, &blocks, len, body.vert, depth);
            ctx.boxes.leave();
            // Text that doesn't fit inside the margins is hidden, as in Word; a box that resizes
            // to fit its text shows it all (#743).
            if body.auto_fit { inner } else { fit_box(inner, across) }
        }
    };
    if parent.is_some_and(|obj| direct_parent(ctx, obj, id)) {
        let owners = suppressed_rects(ctx, id, &items);
        // Preserve the source and hit metadata; these direct rectangles do not paint in Word.
        items.retain(|item| !matches!(item, Placed::Shape { object: Some(owner), .. } if owners.contains(owner)));
    }
    place(items, &body, rect, across)
}

/// The native controls cover direct floating, unlinked boxes with horizontal, top-aligned text.
fn direct_parent(ctx: &mut Ctx, obj: &wordcraft_doc::para::InlineObject, id: u32) -> bool {
    use wordcraft_doc::para::{InlineObject, Wrap};

    let InlineObject::Shape { story: Some(owner), float, .. } = obj else { return false };
    if *owner != id || float.wrap == Wrap::Inline || float.spin() != wordcraft_geom::Spin::default() {
        return false;
    }
    let Some(part) = ctx.doc.parts.get(&id) else { return false };
    if part.kind != wordcraft_doc::PartKind::TextBox || !part.body.auto_fit || part.body.vert != TextVert::Horz || part.body.anchor != VAlign::Top {
        return false;
    }
    ctx.chains.build(ctx.doc);
    !ctx.chains.member.contains_key(&id)
}

fn item_owner(item: &Placed) -> Option<wordcraft_doc::Pos> {
    match item {
        Placed::Shape { object, .. } => object.clone(),
        Placed::Object { story, path, off, .. } => Some(wordcraft_doc::Pos::new(*story, path.clone(), *off)),
        _ => None,
    }
}

/// Match source markers once per direct paragraph, then require both paint and hit ownership.
fn suppressed_rects(ctx: &Ctx, id: u32, items: &[Placed]) -> BTreeSet<wordcraft_doc::Pos> {
    use wordcraft_doc::para::{Anchor, InlineObject, ShapeKind, Wrap};

    let mut paths = BTreeSet::new();
    for item in items {
        if let Some(owner) = item_owner(item)
            && owner.story == StoryRef::Part(id)
            && owner.path.0.len() == 1
        {
            paths.insert(owner.path);
        }
    }
    let mut eligible = BTreeSet::new();
    for path in paths {
        let Some(p) = ctx.doc.para(StoryRef::Part(id), &path) else { continue };
        if p.text.chars().filter(|c| *c == wordcraft_doc::para::OBJ).count() != p.objects.len() {
            continue;
        }
        for ((off, _), object) in p.text.char_indices().filter(|(_, c)| *c == wordcraft_doc::para::OBJ).zip(&p.objects) {
            let InlineObject::Shape { kind: ShapeKind::Rectangle, story: None, freeform: None, effects, stroke: None, float, .. } = object else {
                continue;
            };
            if *effects != Default::default()
                || float.spin() != wordcraft_geom::Spin::default()
                || !matches!(float.wrap, Wrap::InFrontOfText | Wrap::BehindText | Wrap::Square)
                || float.h_rel != Anchor::Column
                || float.v_rel != Anchor::Paragraph
                || float.h_align.is_some()
                || float.v_align.is_some()
            {
                continue;
            }
            eligible.insert(wordcraft_doc::Pos::new(StoryRef::Part(id), path.clone(), off));
        }
    }
    let mut painted = BTreeSet::new();
    let mut hits = BTreeSet::new();
    for item in items {
        let Some(owner) = item_owner(item).filter(|owner| eligible.contains(owner)) else { continue };
        match item {
            Placed::Shape { .. } => {
                painted.insert(owner);
            }
            Placed::Object { .. } => {
                hits.insert(owner);
            }
            _ => {}
        }
    }
    painted.retain(|owner| hits.contains(owner));
    painted
}

/// The height of a `w` wide text box `id` that resizes to fit its text (`a:spAutoFit`): its text's
/// depth plus its top and bottom margins, as Word fits it when laying the document out. `None`
/// for a box that keeps its own height: one that doesn't resize, with turned text, or linked.
pub(crate) fn fitted_height(ctx: &mut Ctx, id: u32, w: f32) -> Option<f32> {
    let body = ctx.doc.parts.get(&id).map(|p| p.body)?;
    if !body.auto_fit || body.vert.turn() != TextDirection::Horizontal || body.next.is_some() {
        return None;
    }
    ctx.chains.build(ctx.doc);
    if ctx.chains.member.contains_key(&id) {
        return None;
    }
    let (len, _) = frame(&body, w, 0.0);
    let items = measure(ctx, id, len, body.vert, 0)?;
    let [_, t, _, b] = body.insets_pt();
    let h = extent(&items) + t + b;
    h.is_finite().then_some(h)
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
    let [l, t, r, b] = body.insets_pt();
    let (ox, oy) = match turn {
        TextDirection::Horizontal => (rect.x + l, rect.y + t + off),
        // Top-to-bottom lines stack from the right edge, bottom-to-top ones from the left.
        TextDirection::Down => (rect.right() - r - off, rect.y + t),
        TextDirection::Up => (rect.x + l + off, rect.bottom() - b),
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
            | Placed::Clip { rect, .. }
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
    if !take_measure(ctx) {
        return Some(Vec::new());
    }
    if !ctx.chains.flows.contains_key(&ci) {
        let chain = ctx.chains.chains.get(ci).cloned().unwrap_or_default();
        let flow = flow(ctx, &chain, depth);
        if ctx.measure_depth > 0 && ctx.measure_exhausted {
            return Some(Vec::new());
        }
        if ctx.chains.flows.insert(ci, flow).is_none() {
            ctx.chains.flow_order.push(ci);
        }
    }
    Some(ctx.chains.flows.get(&ci).and_then(|f| f.get(k)).cloned().unwrap_or_default())
}

/// Where a box's text starts: the line holding byte `byte` of paragraph `path`.
struct Cut {
    path: Path,
    byte: usize,
}

/// Most line lengths (and text directions) a chain's story is laid out at. Each lays the whole
/// story out again and is kept while the chain is split, so a chain of boxes that each differ in
/// width would multiply its story's layout work and memory. Past 16, a box whose length isn't
/// among them takes the nearest narrower one laid out in its text direction, so its lines still
/// fit across it (they stop short of its far edge), else the narrowest, whose lines are longer
/// than the box. Far above a real chain's few box sizes.
pub(crate) const MAX_FLOW_WIDTHS: usize = 16;

/// Line lengths are told apart to a twentieth of a point (a twip): resized boxes a hair apart
/// share one layout. Stacked text is laid out at [`STACK_WIDTH`] whatever the box's size.
fn flow_key(len: f32, vert: TextVert) -> (i32, TextVert) {
    let len = if vert == TextVert::Stacked { STACK_WIDTH } else { len };
    ((len * 20.0).round() as i32, vert)
}

/// Split the text of `chain`'s first box over its boxes.
fn flow(ctx: &mut Ctx, chain: &[u32], depth: usize) -> Vec<Vec<Placed>> {
    let Some(&head) = chain.first() else { return Vec::new() };
    if !enter_box(ctx, head) {
        return vec![Vec::new(); chain.len()];
    }
    let ids: BTreeSet<u32> = chain.iter().copied().collect();
    let sizes = ctx.doc.text_box_sizes(&ids);
    let blocks = box_blocks(ctx, head);
    // Each box lays the whole story out at its own line length (the same list numbers every
    // time), sharing a layout with the boxes before it of the same length.
    let snap = (ctx.counters.clone(), ctx.eq_count);
    let mut after = None;
    let mut layouts: Vec<((i32, TextVert), Vec<Placed>)> = Vec::new();
    let mut out = Vec::with_capacity(chain.len());
    // Where the next box's text starts: `Some(None)` at the beginning, `None` when all is shown.
    let mut start: Option<Option<Cut>> = Some(None);
    for id in chain {
        // A chain can reflow a long story at many widths, even without entering another box.
        if !take_measure(ctx) {
            ctx.boxes.leave();
            return Vec::new();
        }
        let Some(cut) = start.take() else {
            out.push(Vec::new());
            continue;
        };
        let body = ctx.doc.parts.get(id).map(|p| p.body).unwrap_or_default();
        let (w, h) = sizes.get(id).copied().unwrap_or((0.0, 0.0));
        let (len, across) = frame(&body, w, h);
        let key = flow_key(len, body.vert);
        // Past the cap the box takes the nearest layout in its direction: the widest line length
        // that fits it, else the narrowest. The first box in a direction is laid out (one more
        // layout for each of the few directions at most).
        let near = || layouts.iter().filter(|((_, v), _)| *v == key.1).min_by_key(|((k, _), _)| (*k > key.0, k.abs_diff(key.0)));
        let kept = layouts.iter().find(|(k, _)| *k == key).or_else(|| if layouts.len() >= MAX_FLOW_WIDTHS { near() } else { None });
        let items = match kept {
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
            | Placed::Clip { rect, .. }
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

#[cfg(test)]
#[path = "tests/textbox_probe.rs"]
pub(super) mod probe_work_tests;

#[cfg(test)]
#[path = "tests/textbox_policy.rs"]
mod fitting_policy_tests;
