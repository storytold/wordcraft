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

use crate::{BoxContext, Ctx, Placed, fit_box, layout_box};

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
        return layout_box(ctx, story, &stacked, &[], STACK_WIDTH, BoxContext::Other, depth + 1, None).items;
    }
    layout_box(ctx, story, blocks, &[], len, BoxContext::Other, depth + 1, None).items
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
    let mut layouts: Vec<((u32, TextVert), Vec<Placed>)> = Vec::new();
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
mod probe_work_tests {
    use super::*;
    use wordcraft_doc::para::{Float, InlineObject, ShapeKind, Wrap};
    use wordcraft_doc::{Document, Paragraph, PartKind, Pos, para_block};

    fn shape(id: u32, w: f32) -> InlineObject {
        InlineObject::Shape {
            kind: ShapeKind::TextBox,
            w,
            h: 2000.0,
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            float: Float { wrap: Wrap::InFrontOfText, ..Default::default() },
            story: Some(id),
            effects: Default::default(),
            freeform: None,
            extra: Default::default(),
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Graph {
        Shared,
        Varied,
        Linked,
        Long,
    }

    #[derive(Clone, Copy)]
    enum Fit {
        Fixed,
        Auto,
    }

    fn shared_graph(graph: Graph, fit: Fit) -> (Document, u32) {
        let mut d = Document::from_text("");
        d.styles.default_chr.font = Some("Source Sans 3".into());
        let ids: Vec<_> =
            (0..4).map(|k| d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text(&format!("Level{k}"), Default::default()))])).collect();
        for id in &ids {
            d.parts.get_mut(id).unwrap().body = TextBody { auto_fit: matches!(fit, Fit::Auto), inset: Some([0; 4]), ..Default::default() };
        }
        if matches!(graph, Graph::Long) {
            d.parts.get_mut(&ids[3]).unwrap().blocks = Document::from_text(&"Long story line.\n".repeat(63)).body;
        }
        if matches!(graph, Graph::Linked) {
            let tail = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("", Default::default()))]);
            d.parts.get_mut(&ids[3]).unwrap().body.next = Some(tail);
            d.insert_object(&Pos::new(StoryRef::Part(ids[2]), Path::top(0), 0), shape(tail, 180.0), &Default::default()).unwrap();
        }
        for pair in ids.windows(2) {
            for k in 0..wordcraft_doc::BoxBudget::MAX_NESTED {
                let w = if matches!(graph, Graph::Shared) { 180.0 } else { 120.0 + (k % 3) as f32 * 8.0 };
                d.insert_object(&Pos::new(StoryRef::Part(pair[0]), Path::top(0), 0), shape(pair[1], w), &Default::default()).unwrap();
            }
        }
        d.insert_object(&Pos::body(0, 0), shape(ids[0], 240.0), &Default::default()).unwrap();
        (d, ids[0])
    }

    fn layout(d: &Document) -> crate::DocLayout {
        crate::layout(d, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default())
    }

    fn visible(l: &crate::DocLayout) -> HashMap<StoryRef, usize> {
        let mut out = HashMap::new();
        for item in l.pages.iter().flat_map(|p| &p.items) {
            if let Placed::Lines { story, l0, l1, .. } = item {
                *out.entry(*story).or_default() += l1 - l0;
            }
        }
        out
    }

    #[test]
    fn shared_probe_work_is_capped() {
        for graph in [Graph::Shared, Graph::Varied, Graph::Linked, Graph::Long] {
            let (control, _) = shared_graph(graph, Fit::Fixed);
            let expected = layout(&control);
            let (d, root) = shared_graph(graph, Fit::Auto);
            MEASURE_COUNTS.with(|counts| counts.borrow_mut().clear());
            COPIED_BLOCKS.with(|count| count.set(0));
            let l = layout(&d);
            let counts = MEASURE_COUNTS.with(|counts| std::mem::take(&mut *counts.borrow_mut()));
            assert!(counts.contains(&MAX_MEASURE_BOXES), "the hostile graph exhausts measurement work: {counts:?}");
            assert!(counts.len() <= 2 && counts.iter().all(|n| *n <= MAX_MEASURE_BOXES), "{counts:?}");
            assert!(counts.iter().sum::<usize>() <= 2 * MAX_MEASURE_BOXES);
            let copied = COPIED_BLOCKS.with(std::cell::Cell::get);
            let max_blocks = d.parts.values().map(|p| p.blocks.len()).max().unwrap();
            assert!(copied <= counts.iter().sum::<usize>() * max_blocks, "preparation only follows charged work: {copied}");
            assert_eq!(visible(&l), visible(&expected), "probe exhaustion never spends actual text-placement allowance");
            assert_eq!(l.text_boxes, expected.text_boxes);
            assert!(l.text_boxes <= 1 + wordcraft_doc::BoxBudget::MAX_NESTED);
            let frame = l
                .pages
                .iter()
                .flat_map(|p| &p.items)
                .find_map(|it| match it {
                    Placed::Object { rect, text_box: Some(id), .. } if *id == root => Some(*rect),
                    _ => None,
                })
                .unwrap();
            assert_eq!((frame.w, frame.h), (240.0, 2000.0), "an incomplete probe keeps its saved frame");
            eprintln!("PROBE_WORK graph={graph:?} phases={counts:?} copied_blocks={copied} actual_boxes={}", l.text_boxes);
        }
    }

    #[test]
    fn rejected_probe_copies_no_story() {
        let (d, _) = shared_graph(Graph::Linked, Fit::Auto);
        let mut cache = crate::LayoutCache::new();
        let opts = crate::LayoutOptions::default();
        let prior_wraps = crate::PriorBodyWraps::new();
        let mut back_wraps = crate::BackBodyWraps::new();
        let mut ctx = Ctx {
            doc: &d,
            sections: d.sections(),
            floats_above: HashMap::new(),
            cache: &mut cache,
            opts: &opts,
            counters: Default::default(),
            fields: Default::default(),
            notes_hash: 0,
            numbers: HashMap::new(),
            eq_count: 0,
            boxes: Default::default(),
            chains: Default::default(),
            grid: Default::default(),
            hide_deleted: opts.hides_deleted(StoryRef::Body),
            uses_totals: false,
            track_totals: true,
            pct_base: None,
            quiet_marks: Vec::new(),
            mirror: false,
            table_keys: Vec::new(),
            prior_wraps: &prior_wraps,
            back_wraps: &mut back_wraps,
            pin: None,
            footer_wraps: HashMap::new(),
            header_tables: HashMap::new(),
            frames: Default::default(),
            fitted: HashMap::new(),
            fitted_order: Vec::new(),
            measure_left: 3,
            measure_depth: 1,
            measure_exhausted: false,
        };
        ctx.chains.build(ctx.doc);
        let head = *ctx.chains.chains[0].first().unwrap();
        MEASURE_COUNTS.with(|counts| *counts.borrow_mut() = vec![0]);
        COPIED_BLOCKS.with(|count| count.set(0));
        // Chain copy, head entry and first width consume three units. The next width is rejected.
        assert!(chain_share(&mut ctx, head, 0).unwrap().is_empty());
        assert!(ctx.measure_exhausted);
        assert_eq!(ctx.measure_left, 0);
        assert!(ctx.chains.flows.is_empty(), "partial linked flow must not be cached");
        assert!(!ctx.boxes.inside(), "an interrupted flow leaves its head");
        let copied = COPIED_BLOCKS.with(std::cell::Cell::get);
        assert_eq!(copied, 1);
        let obj = shape(1, 240.0);
        for _ in 0..2000 {
            assert!(chain_share(&mut ctx, head, 0).unwrap().is_empty());
            assert_eq!(fit_size(&mut ctx, &obj, 240.0, 2000.0, 0), (240.0, 2000.0));
        }
        assert_eq!(COPIED_BLOCKS.with(std::cell::Cell::get), copied, "rejected probes never clone Blocks");
        assert_eq!(MEASURE_COUNTS.with(|counts| counts.borrow()[0]), 3);
    }

    fn with_probe_ctx(d: &Document, f: impl FnOnce(&mut Ctx)) {
        let mut cache = crate::LayoutCache::new();
        let opts = crate::LayoutOptions::default();
        let prior_wraps = crate::PriorBodyWraps::new();
        let mut back_wraps = crate::BackBodyWraps::new();
        let mut ctx = Ctx {
            doc: d,
            sections: d.sections(),
            floats_above: HashMap::new(),
            cache: &mut cache,
            opts: &opts,
            counters: Default::default(),
            fields: Default::default(),
            notes_hash: 0,
            numbers: HashMap::new(),
            eq_count: 0,
            boxes: Default::default(),
            chains: Default::default(),
            grid: Default::default(),
            hide_deleted: opts.hides_deleted(StoryRef::Body),
            uses_totals: false,
            track_totals: true,
            pct_base: None,
            quiet_marks: Vec::new(),
            mirror: false,
            table_keys: Vec::new(),
            prior_wraps: &prior_wraps,
            back_wraps: &mut back_wraps,
            pin: None,
            footer_wraps: HashMap::new(),
            header_tables: HashMap::new(),
            frames: Default::default(),
            fitted: HashMap::new(),
            fitted_order: Vec::new(),
            measure_left: 0,
            measure_depth: 0,
            measure_exhausted: false,
        };
        f(&mut ctx);
    }

    fn inline_shape(id: u32, w: f32) -> InlineObject {
        let mut obj = shape(id, w);
        if let InlineObject::Shape { float, .. } = &mut obj {
            float.wrap = Wrap::Inline;
        }
        obj
    }

    #[test]
    fn root_none_is_reused() {
        let (mut d, root) = shared_graph(Graph::Shared, Fit::Auto);
        let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        p.objects[0] = inline_shape(root, 240.0);
        p.touch();
        let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
        with_probe_ctx(&d, |ctx| {
            MEASURE_COUNTS.with(|counts| counts.borrow_mut().clear());
            COPIED_BLOCKS.with(|count| count.set(0));
            let first = ctx.para_labelled(p, 500.0, None, &[], None);
            let before = MEASURE_COUNTS.with(|counts| counts.borrow().clone());
            assert_eq!(before, [MAX_MEASURE_BOXES]);
            assert!(ctx.measure_exhausted);
            let copied = COPIED_BLOCKS.with(std::cell::Cell::get);
            for _ in 0..4 {
                let repeated = ctx.para_labelled(p, 500.0, None, &[], None);
                assert!(Arc::ptr_eq(&first, &repeated));
                let measured = ctx.para_measured(p, 500.0, None, None, |pl| (pl.height, pl.height));
                assert_eq!(measured, (first.height, first.height));
            }
            let after = MEASURE_COUNTS.with(|counts| counts.borrow().clone());
            eprintln!(
                "ROOT_PROBE_WORK before={before:?} after={after:?} copied_before={copied} copied_after={}",
                COPIED_BLOCKS.with(std::cell::Cell::get)
            );
            assert_eq!(after, before, "repeated layout and table measurement reuse the exhausted root result");
            assert_eq!(COPIED_BLOCKS.with(std::cell::Cell::get), copied);
            let key = (root, 240.0f32.to_bits());
            assert_eq!(ctx.fitted.get(&key), Some(&None));
            assert_eq!(ctx.fitted_order, [key]);
            assert_eq!(ctx.measure_depth, 0);
            assert!(!ctx.boxes.inside());
            assert_eq!(ctx.boxes.total(), 0);
            assert!(ctx.chains.flows.is_empty());
        });
        let mut fixed = d.clone();
        for part in fixed.parts.values_mut() {
            part.body.auto_fit = false;
        }
        let expected = layout(&fixed);
        let actual = layout(&d);
        assert_eq!(visible(&actual), visible(&expected));
        assert_eq!(actual.text_boxes, expected.text_boxes);
        assert!(actual.text_boxes <= 1 + wordcraft_doc::BoxBudget::MAX_NESTED);
    }

    #[derive(Clone, Copy, Debug)]
    enum Nest {
        ActualBox,
        Probe,
    }

    #[test]
    fn nested_none_is_not_cached() {
        let mut d = Document::from_text("");
        d.styles.default_chr.font = Some("Source Sans 3".into());
        let list = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        let parent = d.add_part(PartKind::TextBox, vec![]);
        let leaf = d.add_part(PartKind::TextBox, Document::from_text("Visible child").body);
        let root = d.add_part(PartKind::TextBox, Document::from_text("Numbered root").body);
        for id in [root, leaf] {
            d.parts.get_mut(&id).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
        }
        d.para_mut(StoryRef::Part(root), &Path::top(0)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
        d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), inline_shape(leaf, 180.0), &Default::default()).unwrap();
        d.insert_object(&Pos::body(0, 0), inline_shape(root, 240.0), &Default::default()).unwrap();
        let p = d.para(StoryRef::Body, &Path::top(0)).unwrap();
        for nest in [Nest::ActualBox, Nest::Probe] {
            with_probe_ctx(&d, |ctx| {
                match nest {
                    Nest::ActualBox => assert!(ctx.boxes.enter(parent)),
                    Nest::Probe => ctx.measure_depth = 1,
                }
                ctx.eq_count = 7;
                let before = (ctx.boxes.total(), ctx.measure_depth);
                ctx.measure_left = 1;
                MEASURE_COUNTS.with(|counts| *counts.borrow_mut() = vec![0]);
                assert!(ctx.fitted_boxes(p).is_empty());
                assert!(ctx.measure_exhausted);
                assert_eq!(ctx.measure_left, 0);
                assert_eq!((ctx.boxes.total(), ctx.measure_depth), before);
                assert_eq!(ctx.boxes.inside(), matches!(nest, Nest::ActualBox));
                assert_eq!(ctx.eq_count, 7);
                assert!(ctx.fitted.is_empty(), "exhausted {nest:?} must not poison the fitted cache");
                assert!(ctx.fitted_order.is_empty());
                assert!(ctx.chains.flows.is_empty());
                let mut counters = ctx.counters.clone();
                assert_eq!(counters.next_label(&d.numbering, list, 0).unwrap().0, "1.");
                match nest {
                    Nest::ActualBox => ctx.boxes.leave(),
                    Nest::Probe => ctx.measure_depth = 0,
                }
                let measured = ctx.fitted_boxes(p);
                assert_eq!(measured.len(), 1, "a fresh root must recover from exhausted {nest:?}");
                assert!(measured[0].1 > 0.0);
                assert!(!ctx.measure_exhausted);
                let key = (root, 240.0f32.to_bits());
                assert_eq!(ctx.fitted.get(&key), Some(&Some(measured[0].1)));
                assert_eq!(ctx.fitted_order, [key]);
                assert_eq!(ctx.eq_count, 7);
                let mut counters = ctx.counters.clone();
                assert_eq!(counters.next_label(&d.numbering, list, 0).unwrap().0, "1.");
                eprintln!(
                    "NESTED_PROBE_CACHE context={nest:?} exhausted_uncached=true fresh_height={} phases={:?}",
                    measured[0].1,
                    MEASURE_COUNTS.with(|counts| counts.borrow().clone())
                );
            });
        }
    }
    #[test]
    fn suppressed_owner_identity() {
        let mut d = Document::from_text("");
        let root = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Owner", Default::default()))]);
        let mut object = shape(root, 40.0);
        if let InlineObject::Shape { kind, story, h, float, fill, .. } = &mut object {
            *kind = ShapeKind::Rectangle;
            *story = None;
            *h = 10.0;
            *fill = Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(19, 47, 83)));
            float.h_rel = wordcraft_doc::para::Anchor::Column;
            float.v_rel = wordcraft_doc::para::Anchor::Paragraph;
        }
        let owner = Pos::new(StoryRef::Part(root), Path::top(0), 0);
        d.insert_object(&owner, object, &Default::default()).unwrap();
        with_probe_ctx(&d, |ctx| {
            let blocks = &d.parts.get(&root).unwrap().blocks;
            assert!(ctx.boxes.enter(root));
            let items = lay(ctx, root, blocks, 180.0, TextVert::Horz, 0);
            ctx.boxes.leave();
            assert_eq!(suppressed_rects(ctx, root, &items), BTreeSet::from([owner.clone()]));
            let no_hit: Vec<_> = items.iter().filter(|item| !matches!(item, Placed::Object { .. })).cloned().collect();
            assert!(suppressed_rects(ctx, root, &no_hit).is_empty());
            let no_paint: Vec<_> = items.iter().filter(|item| !matches!(item, Placed::Shape { .. })).cloned().collect();
            assert!(suppressed_rects(ctx, root, &no_paint).is_empty());
            for foreign in [Pos::body(0, 0), Pos::new(owner.story, Path::top(1), 0), Pos::new(owner.story, owner.path.clone(), 1)] {
                let mut changed = items.clone();
                for item in &mut changed {
                    match item {
                        Placed::Shape { object: Some(pos), .. } => *pos = foreign.clone(),
                        Placed::Object { story, path, off, .. } => {
                            *story = foreign.story;
                            *path = foreign.path.clone();
                            *off = foreign.off;
                        }
                        _ => {}
                    }
                }
                assert!(suppressed_rects(ctx, root, &changed).is_empty(), "{foreign:?}");
            }
        });
        d.para_mut(owner.story, &owner.path).unwrap().objects.push(InlineObject::BookmarkEnd { name: "unmatched".into() });
        with_probe_ctx(&d, |ctx| {
            let blocks = &d.parts.get(&root).unwrap().blocks;
            let items = lay(ctx, root, blocks, 180.0, TextVert::Horz, 0);
            assert!(suppressed_rects(ctx, root, &items).is_empty(), "malformed marker ownership is not classified");
        });
    }

    fn floating_doc(text: &str) -> (Document, u32, InlineObject) {
        let mut d = Document::from_text("");
        d.styles.default_chr.font = Some("Source Sans 3".into());
        let id = d.add_part(PartKind::TextBox, Document::from_text(text).body);
        d.parts.get_mut(&id).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
        let obj = shape(id, 240.0);
        d.insert_object(&Pos::body(0, 0), obj.clone(), &Default::default()).unwrap();
        (d, id, obj)
    }

    #[test]
    fn floating_none_is_reused() {
        for graph in [Graph::Shared, Graph::Varied, Graph::Long] {
            let (d, root) = shared_graph(graph, Fit::Auto);
            let obj = shape(root, 240.0);
            with_probe_ctx(&d, |ctx| {
                MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
                COPIED_BLOCKS.with(|v| v.set(0));
                assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), (240.0, 2000.0));
                let before = MEASURE_COUNTS.with(|v| v.borrow().clone());
                let copied = COPIED_BLOCKS.with(std::cell::Cell::get);
                assert_eq!(before, [MAX_MEASURE_BOXES]);
                for _ in 0..=wordcraft_doc::BoxBudget::MAX_NESTED {
                    assert!(ctx.boxes.enter(root));
                    ctx.boxes.leave();
                }
                let actual_total = ctx.boxes.total();
                assert!(actual_total > wordcraft_doc::BoxBudget::MAX_NESTED);
                for saved in [2000.0, 2100.0, 80.0] {
                    assert_eq!(fit_size(ctx, &obj, 240.0, saved, 0), (240.0, saved));
                }
                assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), before, "one exhausted outer measurement per context");
                assert_eq!(COPIED_BLOCKS.with(std::cell::Cell::get), copied);
                assert_eq!(ctx.boxes.total(), actual_total);
                assert!(!ctx.boxes.inside());
                assert_eq!(ctx.measure_depth, 0);
            });
        }
    }

    #[test]
    fn floating_success_is_reused() {
        let (mut d, _, obj) = floating_doc("Original floating content");
        let list = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
        let unused = d.add_part(PartKind::TextBox, Document::from_text("Unrelated numbered part").body);
        d.para_mut(StoryRef::Part(unused), &Path::top(0)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: list, level: 0 });
        with_probe_ctx(&d, |ctx| {
            MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
            let first = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            let before = MEASURE_COUNTS.with(|v| v.borrow().clone());
            assert!(first.1 < 2000.0);
            assert_eq!(before, [1]);
            for _ in 0..=wordcraft_doc::BoxBudget::MAX_NESTED {
                assert!(ctx.boxes.enter(obj.text_box().unwrap()));
                ctx.boxes.leave();
            }
            let actual_total = ctx.boxes.total();
            assert!(actual_total > wordcraft_doc::BoxBudget::MAX_NESTED);
            for _ in 0..4 {
                assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), first);
            }
            assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), before, "unrelated numbering does not disable static fitting reuse");
            assert_eq!(ctx.boxes.total(), actual_total);
        });
    }

    #[test]
    fn floating_active_keeps_guards() {
        let (d, root, obj) = floating_doc("Original cycle guard");
        with_probe_ctx(&d, |ctx| {
            MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
            let expected = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            assert!(expected.1 < 2000.0);
            assert!(ctx.boxes.enter(root));
            let total = ctx.boxes.total();
            ctx.eq_count = 7;
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), (240.0, 2000.0), "an active owner must still reject its cycle");
            assert_eq!(ctx.boxes.total(), total);
            assert_eq!(ctx.eq_count, 7);
            assert_eq!(ctx.measure_depth, 0);
            assert!(ctx.boxes.inside());
            ctx.boxes.leave();
            let before = MEASURE_COUNTS.with(|v| v.borrow().clone());
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), expected);
            assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), before, "nested rejection must not overwrite the outer cache");
        });
    }

    #[test]
    fn floating_page_stays_fresh() {
        let (mut d, root, mut obj) = floating_doc("");
        d.insert_object(
            &Pos::new(StoryRef::Part(root), Path::top(0), 0),
            InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() },
            &Default::default(),
        )
        .unwrap();
        if let InlineObject::Shape { w, .. } = &mut obj {
            *w = 12.0;
        }
        with_probe_ctx(&d, |ctx| {
            ctx.fields.page = 1;
            let first = fit_size(ctx, &obj, 12.0, 2000.0, 0);
            ctx.fields.page = 1234567890;
            let later = fit_size(ctx, &obj, 12.0, 2000.0, 0);
            let cold = with_probe_ctx_result(&d, |fresh| {
                fresh.fields.page = 1234567890;
                fit_size(fresh, &obj, 12.0, 2000.0, 0)
            });
            let shown: Vec<_> = ctx.cache.paras.values().flat_map(|p| p.shown.iter().map(|v| v.1.as_str())).collect();
            assert!(shown.contains(&"1") && shown.contains(&"1234567890"), "both page contexts were measured: {shown:?}");
            assert!(first.1 > 0.0);
            assert_eq!(later, cold, "the static scalar key must not reuse a different page's extent");
        });
    }

    fn with_probe_ctx_result<T>(d: &Document, f: impl FnOnce(&mut Ctx) -> T) -> T {
        let mut result = None;
        with_probe_ctx(d, |ctx| result = Some(f(ctx)));
        result.unwrap()
    }

    #[test]
    fn floating_outline_is_caller() {
        use wordcraft_doc::props::Rgb;
        let (d, _, obj) = floating_doc("Original outline");
        with_probe_ctx(&d, |ctx| {
            let mut narrow = obj.clone();
            let mut wide = obj.clone();
            for (o, width) in [(&mut narrow, 1.0), (&mut wide, 7.0)] {
                if let InlineObject::Shape { stroke, stroke_width, .. } = o {
                    *stroke = Some(Rgb(20, 60, 100));
                    *stroke_width = width;
                }
            }
            let a = fit_size(ctx, &narrow, 240.0, 2000.0, 0);
            let b = fit_size(ctx, &wide, 240.0, 2000.0, 0);
            assert!((b.1 - a.1 - 6.0).abs() < 0.001, "cache content extent, not caller outline");
            assert_eq!(fit_size(ctx, &narrow, 240.0, 2000.0, 0), a);
        });
    }

    #[test]
    fn floating_direct_is_separate() {
        use wordcraft_doc::para::Anchor;
        use wordcraft_doc::props::Rgb;
        let (mut d, root, direct) = floating_doc("Original ownership");
        let child = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 20.0,
            h: 20.0,
            fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(25, 73, 137))),
            stroke: None,
            stroke_width: 0.0,
            float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, y: 500.0, ..Default::default() },
            story: None,
            effects: Default::default(),
            freeform: None,
            extra: Default::default(),
        };
        d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), child, &Default::default()).unwrap();
        let mut turned = direct.clone();
        turned.float_mut().unwrap().rot = 90.0;
        let a = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &direct, 240.0, 2000.0, 0));
        let b = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &turned, 240.0, 2000.0, 0));
        assert!(a.1 < 500.0 && b.1 >= 520.0);
        for order in [[(&direct, a), (&turned, b)], [(&turned, b), (&direct, a)]] {
            with_probe_ctx(&d, |ctx| {
                for (obj, expected) in order.into_iter().cycle().take(4) {
                    assert_eq!(fit_size(ctx, obj, 240.0, 2000.0, 0), expected);
                }
            });
        }
    }

    #[test]
    fn floating_widths_stay_distinct() {
        let text = "Original width-sensitive content ".repeat(8);
        let (d, _, obj) = floating_doc(&text);
        let a = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &obj, 24.0, 2000.0, 0));
        let b = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &obj, 240.0, 2000.0, 0));
        assert!(a.1 > b.1);
        with_probe_ctx(&d, |ctx| {
            for (width, expected) in [(24.0, a), (240.0, b), (24.0, a), (240.0, b)] {
                assert_eq!(fit_size(ctx, &obj, width, 2000.0, 0), expected);
            }
        });
    }

    #[test]
    fn floating_grid_keeps_context() {
        let (d, _, obj) = floating_doc("Original grid context");
        with_probe_ctx(&d, |ctx| {
            MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
            let plain = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), plain);
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 1), plain);
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 1), plain);
            ctx.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
            let snapped = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            assert!(snapped.1 > plain.1);
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), snapped);
            ctx.grid = Default::default();
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), plain);
            assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), [1, 1, 1], "separate depth/grid contexts each measure once");
        });
    }

    #[test]
    fn floating_nested_keeps_budget() {
        let (mut d, root, obj) = floating_doc("Original outer");
        let leaf = d.add_part(PartKind::TextBox, Document::from_text("Original inline leaf").body);
        d.parts.get_mut(&leaf).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
        d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), inline_shape(leaf, 180.0), &Default::default()).unwrap();
        let parent = d.add_part(PartKind::TextBox, vec![]);
        let expected = with_probe_ctx_result(&d, |ctx| fit_size(ctx, &obj, 240.0, 2000.0, 0));
        for nest in [Nest::ActualBox, Nest::Probe] {
            with_probe_ctx(&d, |ctx| {
                match nest {
                    Nest::ActualBox => assert!(ctx.boxes.enter(parent)),
                    Nest::Probe => ctx.measure_depth = 1,
                }
                let before = (ctx.boxes.total(), ctx.measure_depth);
                ctx.eq_count = 7;
                ctx.measure_left = 1;
                MEASURE_COUNTS.with(|v| *v.borrow_mut() = vec![0]);
                assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), (240.0, 2000.0));
                assert!(ctx.measure_exhausted);
                assert_eq!((ctx.boxes.total(), ctx.measure_depth), before);
                assert_eq!(ctx.eq_count, 7);
                assert!(ctx.fitted.is_empty() && ctx.fitted_order.is_empty());
                match nest {
                    Nest::ActualBox => ctx.boxes.leave(),
                    Nest::Probe => ctx.measure_depth = 0,
                }
                assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), expected, "exhausted nested work must not poison a fresh outer result");
                let counts = MEASURE_COUNTS.with(|v| v.borrow().clone());
                assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), expected);
                assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), counts);
            });
        }
    }

    #[test]
    fn floating_nested_is_not_memoed() {
        let (mut d, _, obj) = floating_doc("Original nested context");
        let parent = d.add_part(PartKind::TextBox, vec![]);
        with_probe_ctx(&d, |ctx| {
            MEASURE_COUNTS.with(|v| *v.borrow_mut() = vec![0]);
            assert!(ctx.boxes.enter(parent));
            ctx.measure_left = MAX_MEASURE_BOXES;
            ctx.eq_count = 7;
            let nested = fit_size(ctx, &obj, 240.0, 2000.0, 0);
            assert!(!ctx.measure_exhausted);
            assert_eq!(ctx.boxes.total(), 1);
            assert_eq!(ctx.eq_count, 7);
            ctx.boxes.leave();
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), nested);
            let before = MEASURE_COUNTS.with(|v| v.borrow().clone());
            assert_eq!(before, [1, 1], "a successful nested result must not seed the outer cache");
            assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), nested);
            assert_eq!(MEASURE_COUNTS.with(|v| v.borrow().clone()), before);
        });
    }

    #[test]
    fn floating_inline_state_changes() {
        let (mut d, root, obj) = floating_doc("");
        let leaf = d.add_part(PartKind::TextBox, Document::from_text("Original inline state").body);
        d.parts.get_mut(&leaf).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
        for owner in [Pos::new(StoryRef::Part(root), Path::top(0), 0), Pos::body(0, 0)] {
            d.insert_object(&owner, inline_shape(leaf, 180.0), &Default::default()).unwrap();
        }
        let carrier = d.para(StoryRef::Body, &Path::top(0)).unwrap();
        for seed_first in [false, true] {
            with_probe_ctx(&d, |ctx| {
                let first = fit_size(ctx, &obj, 240.0, 2000.0, 0);
                let before = ctx.fitted_order.clone();
                ctx.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
                assert_eq!(ctx.fitted_boxes(carrier).len(), 1);
                assert!(ctx.fitted_order.starts_with(&before));
                let seeded = ctx.fitted_order.clone();
                let wide = if seed_first { Some(fit_size(ctx, &obj, 240.0, 2000.0, 0)) } else { None };
                ctx.grid = Default::default();
                let later = fit_size(ctx, &obj, 240.0, 2000.0, 0);
                let cold = with_probe_ctx_result(&d, |fresh| {
                    fresh.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
                    assert_eq!(fresh.fitted_boxes(carrier).len(), 1);
                    fresh.grid = Default::default();
                    fit_size(fresh, &obj, 240.0, 2000.0, 0)
                });
                assert!(cold.1 > first.1, "the inline cache is a measurement input: {first:?} {cold:?}");
                assert_eq!(later, cold, "outer memoization must retain existing inline-cache context");
                ctx.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
                let after = fit_size(ctx, &obj, 240.0, 2000.0, 0);
                if let Some(wide) = wide {
                    assert_eq!(after, wide);
                }
                ctx.grid = Default::default();
                assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), cold);
                assert_eq!(ctx.fitted_order, seeded, "probes retain the persistent prefix");
            });
        }
    }
    #[test]
    fn floating_linked_keeps_context() {
        let (mut d, root, obj) = floating_doc("");
        let head = d.add_part(PartKind::TextBox, Document::from_text("Original linked context").body);
        let tail = d.add_part(PartKind::TextBox, Document::from_text("").body);
        d.parts.get_mut(&head).unwrap().body.next = Some(tail);
        d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), inline_shape(head, 180.0), &Default::default()).unwrap();
        d.insert_object(&Pos::body(0, 0), shape(tail, 180.0), &Default::default()).unwrap();
        for seed_first in [false, true] {
            with_probe_ctx(&d, |ctx| {
                if seed_first {
                    assert!(chain_share(ctx, head, 0).is_some());
                }
                let first = fit_size(ctx, &obj, 240.0, 2000.0, 0);
                let saved = std::mem::take(&mut ctx.chains);
                ctx.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
                assert!(chain_share(ctx, head, 0).is_some());
                ctx.grid = Default::default();
                MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
                let temporary = fit_size(ctx, &obj, 240.0, 2000.0, 0);
                let cold = with_probe_ctx_result(&d, |fresh| {
                    fresh.grid = crate::para::Grid { line: Some(40.0), ..Default::default() };
                    assert!(chain_share(fresh, head, 0).is_some());
                    fresh.grid = Default::default();
                    fit_size(fresh, &obj, 240.0, 2000.0, 0)
                });
                assert_eq!(temporary, cold);
                ctx.chains = saved;
                MEASURE_COUNTS.with(|v| v.borrow_mut().clear());
                assert_eq!(fit_size(ctx, &obj, 240.0, 2000.0, 0), first);
                assert!(!MEASURE_COUNTS.with(|v| v.borrow().is_empty()), "replaced flow maps cannot reuse a floating extent");
            });
        }
    }
}

#[cfg(test)]
mod fitting_policy_tests {
    use super::*;
    use wordcraft_doc::para::{Anchor, Float, InlineObject, ShapeKind, Wrap};
    use wordcraft_doc::props::{LineSpacing, Rgb};
    use wordcraft_doc::{Document, Paragraph, PartKind, Pos, para_block};

    const FRAME_WIDTH: f32 = 180.0;
    const SAVED_HEIGHT: f32 = 80.0;
    const LINE_HEIGHT: f32 = 16.0;
    const CHILD_COLOR: Rgb = Rgb(25, 73, 137);

    fn paragraph() -> Paragraph {
        let mut p = Paragraph::with_text("Parent", Default::default());
        p.props.line_spacing = Some(LineSpacing::Exactly(LINE_HEIGHT));
        p.props.space_before = Some(0.0);
        p.props.space_after = Some(0.0);
        p
    }

    fn rectangle(wrap: Wrap, y: f32) -> InlineObject {
        InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 40.0,
            h: 10.0,
            fill: Some(wordcraft_doc::fill::Fill::solid(CHILD_COLOR)),
            stroke: None,
            stroke_width: 0.0,
            float: Float { wrap, h_rel: Anchor::Column, v_rel: Anchor::Paragraph, y, ..Default::default() },
            story: None,
            effects: Default::default(),
            freeform: None,
            extra: Default::default(),
        }
    }

    fn policy_doc(child: Option<InlineObject>) -> (Document, u32) {
        let mut d = Document::from_text("");
        d.styles.default_chr.font = Some("Arial".into());
        d.styles.default_chr.size = Some(10.0);
        let root = d.add_part(PartKind::TextBox, vec![para_block(paragraph())]);
        d.parts.get_mut(&root).unwrap().body = TextBody { auto_fit: true, inset: Some([0; 4]), ..Default::default() };
        if let Some(child) = child {
            d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), 0), child, &Default::default()).unwrap();
        }
        let parent = InlineObject::Shape {
            kind: ShapeKind::TextBox,
            w: FRAME_WIDTH,
            h: SAVED_HEIGHT,
            fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(240, 248, 240))),
            stroke: Some(Rgb(192, 0, 0)),
            stroke_width: 1.0,
            float: Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x: 72.0, y: 100.0, ..Default::default() },
            story: Some(root),
            effects: Default::default(),
            freeform: None,
            extra: Default::default(),
        };
        d.insert_object(&Pos::body(0, 0), parent, &Default::default()).unwrap();
        (d, root)
    }

    fn lay(d: &Document) -> crate::DocLayout {
        crate::layout(d, &mut crate::LayoutCache::new(), &crate::LayoutOptions::default())
    }

    fn frame(l: &crate::DocLayout, root: u32) -> Rect {
        l.pages
            .iter()
            .flat_map(|p| &p.items)
            .find_map(|it| match it {
                Placed::Object { rect, text_box: Some(id), .. } if *id == root => Some(*rect),
                _ => None,
            })
            .unwrap()
    }

    fn paint(l: &crate::DocLayout, owner: &Pos) -> usize {
        l.pages.iter().flat_map(|p| &p.items).filter(|item| matches!(item, Placed::Shape { object: Some(pos), .. } if pos == owner)).count()
    }

    fn child_draws(draws: &[crate::display::Draw]) -> usize {
        use crate::display::Draw;
        draws
            .iter()
            .map(|draw| match draw {
                Draw::Shape { fill: Some(crate::display::Paint::Solid(color, _)), .. } if *color == CHILD_COLOR => 1,
                Draw::Rotated { items, .. } | Draw::Turned { items, .. } | Draw::Clip { items, .. } => child_draws(items),
                _ => 0,
            })
            .sum()
    }

    fn draws(d: &Document, l: &crate::DocLayout) -> usize {
        l.pages.iter().map(|p| child_draws(&crate::display::page_display(d, p, &Default::default()))).sum()
    }

    #[test]
    fn floating_rect_height() {
        let mut observed = Vec::new();
        for wrap in [Wrap::InFrontOfText, Wrap::BehindText] {
            for y in [0.0, 12.0, 500.0] {
                let (d, root) = policy_doc(Some(rectangle(wrap, y)));
                let l = lay(&d);
                observed.push((wrap, y, frame(&l, root).h));
            }
        }
        eprintln!("FLOAT_PARENT_HEIGHT {observed:?}");
        assert!(observed.iter().all(|(_, _, h)| (*h - 17.0).abs() < 0.01), "{observed:?}");
    }

    #[test]
    fn floating_rect_paint() {
        let mut observed = Vec::new();
        for wrap in [Wrap::InFrontOfText, Wrap::BehindText] {
            for y in [0.0, 12.0, 500.0] {
                let (d, root) = policy_doc(Some(rectangle(wrap, y)));
                let l = lay(&d);
                let owner = Pos::new(StoryRef::Part(root), Path::top(0), 0);
                let hits = l
                    .pages
                    .iter()
                    .flat_map(|p| &p.items)
                    .filter(|item| {
                        matches!(item,
                    Placed::Object { story, path, off, .. } if *story == owner.story && *path == owner.path && *off == owner.off)
                    })
                    .count();
                assert_eq!(hits, 1, "source object remains selectable");
                assert_eq!(d.para(owner.story, &owner.path).unwrap().objects.len(), 1);
                observed.push((wrap, y, paint(&l, &owner), draws(&d, &l)));
            }
        }
        eprintln!("FLOAT_PARENT_PAINT {observed:?}");
        assert!(observed.iter().all(|(_, _, placed, draw)| *placed == 0 && *draw == 0), "{observed:?}");
    }

    #[test]
    fn square_keeps_flow() {
        let mut child = rectangle(Wrap::Square, 0.0);
        if let InlineObject::Shape { w, h, .. } = &mut child {
            *w = 170.0;
            *h = 40.0;
        }
        let (d, root) = policy_doc(Some(child));
        let l = lay(&d);
        let r = frame(&l, root);
        let top = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .find_map(|it| match it {
                Placed::Lines { story: StoryRef::Part(id), y, .. } if *id == root => Some(*y),
                _ => None,
            })
            .unwrap();
        eprintln!("SQUARE_FRAME {r:?} text_top={top} paint={}", draws(&d, &l));
        assert!((r.h - 57.0).abs() < 0.01, "{r:?}");
        assert!((top - r.y - 40.0).abs() < 0.01, "wrapping displacement survives");
        assert_eq!(draws(&d, &l), 0);
    }

    #[test]
    fn plain_and_inline_keep_flow() {
        let (d, root) = policy_doc(None);
        assert!((frame(&lay(&d), root).h - 17.0).abs() < 0.01);
        let image = InlineObject::Image {
            media: "original-policy-control".into(),
            w: 40.0,
            h: 100.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        };
        let (mut d, root) = policy_doc(Some(image));
        d.para_mut(StoryRef::Part(root), &Path::top(0)).unwrap().props.line_spacing = Some(LineSpacing::AtLeast(LINE_HEIGHT));
        let l = lay(&d);
        let h = frame(&l, root).h;
        let owner = Pos::new(StoryRef::Part(root), Path::top(0), 0);
        let objects: Vec<_> = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Object { story, path, off, rect, .. } if *story == owner.story && *path == owner.path && *off == 0 => Some(*rect),
                _ => None,
            })
            .collect();
        eprintln!("MODEL_TALL_INLINE height={h} hits={objects:?}");
        assert!((101.0..110.0).contains(&h), "{h}");
        assert_eq!(objects.len(), 1);
        assert_eq!((objects[0].w, objects[0].h), (40.0, 100.0));
        let root_lines: Vec<_> = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Lines { story: StoryRef::Part(id), para, l0, l1, .. } if *id == root => Some((para, l0, l1)),
                _ => None,
            })
            .collect();
        assert_eq!(root_lines.len(), 1);
        assert!(root_lines[0].0.clusters.iter().any(|c| matches!(c.kind, crate::para::ClKind::Object(0))));
    }

    #[derive(Clone, Copy, Debug)]
    enum OtherChild {
        Ellipse,
        Outline,
        Effect,
        Turn,
        Flip,
        TopBottom,
        Tight,
        PageAnchor,
        TextBox,
    }

    #[test]
    fn other_children_keep_paint() {
        for case in [
            OtherChild::Ellipse,
            OtherChild::Outline,
            OtherChild::Effect,
            OtherChild::Turn,
            OtherChild::Flip,
            OtherChild::TopBottom,
            OtherChild::Tight,
            OtherChild::PageAnchor,
            OtherChild::TextBox,
        ] {
            let (mut d, root) = policy_doc(Some(rectangle(Wrap::InFrontOfText, 500.0)));
            let nested = d.add_part(PartKind::TextBox, vec![para_block(paragraph())]);
            let child = d.para_mut(StoryRef::Part(root), &Path::top(0)).unwrap().object_at_mut(0).unwrap();
            if let InlineObject::Shape { kind, stroke, effects, float, story, .. } = child {
                match case {
                    OtherChild::Ellipse => *kind = ShapeKind::Ellipse,
                    OtherChild::Outline => *stroke = Some(CHILD_COLOR),
                    OtherChild::Effect => effects.soft_edge = Some(1.0),
                    OtherChild::Turn => float.rot = 15.0,
                    OtherChild::Flip => float.flip_h = true,
                    OtherChild::TopBottom => float.wrap = Wrap::TopAndBottom,
                    OtherChild::Tight => float.wrap = Wrap::Tight,
                    OtherChild::PageAnchor => float.v_rel = Anchor::Page,
                    OtherChild::TextBox => {
                        *kind = ShapeKind::TextBox;
                        *story = Some(nested);
                    }
                }
            }
            let l = lay(&d);
            assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), 0)), 1, "{case:?}");
            assert!(draws(&d, &l) > 0, "{case:?}");
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum OtherParent {
        Fixed,
        Inline,
        Turned,
        Center,
        Bottom,
        Linked,
    }

    #[test]
    fn other_parents_keep_paint() {
        for case in [OtherParent::Fixed, OtherParent::Inline, OtherParent::Turned, OtherParent::Center, OtherParent::Bottom, OtherParent::Linked] {
            // The child is inside the saved frame, so fixed/linked clipping is not the policy under test.
            let (mut d, root) = policy_doc(Some(rectangle(Wrap::InFrontOfText, 0.0)));
            let tail = d.add_part(PartKind::TextBox, vec![para_block(paragraph())]);
            match case {
                OtherParent::Fixed => d.parts.get_mut(&root).unwrap().body.auto_fit = false,
                OtherParent::Inline => {
                    if let InlineObject::Shape { float, .. } = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().object_at_mut(0).unwrap() {
                        float.wrap = Wrap::Inline;
                    }
                }
                OtherParent::Turned => d.parts.get_mut(&root).unwrap().body.vert = TextVert::Vert,
                OtherParent::Center => d.parts.get_mut(&root).unwrap().body.anchor = VAlign::Center,
                OtherParent::Bottom => d.parts.get_mut(&root).unwrap().body.anchor = VAlign::Bottom,
                OtherParent::Linked => d.parts.get_mut(&root).unwrap().body.next = Some(tail),
            }
            let l = lay(&d);
            assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), 0)), 1, "{case:?}");
            assert!(draws(&d, &l) > 0, "{case:?}");
        }
    }

    #[test]
    fn same_rect_distinct_owners() {
        let (mut d, root) = policy_doc(Some(rectangle(Wrap::InFrontOfText, 500.0)));
        let mut other = rectangle(Wrap::InFrontOfText, 500.0);
        if let InlineObject::Shape { kind, .. } = &mut other {
            *kind = ShapeKind::Ellipse;
        }
        let off = d.para(StoryRef::Part(root), &Path::top(0)).unwrap().len();
        d.insert_object(&Pos::new(StoryRef::Part(root), Path::top(0), off), other, &Default::default()).unwrap();
        let l = lay(&d);
        assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), 0)), 0);
        assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), off)), 1);
        assert_eq!(draws(&d, &l), 1);
        assert_eq!(d.para(StoryRef::Part(root), &Path::top(0)).unwrap().objects.len(), 2);
    }

    #[test]
    fn table_child_keeps_paint() {
        let (mut d, root) = policy_doc(None);
        let mut p = paragraph();
        p.text.insert(0, wordcraft_doc::para::OBJ);
        p.runs[0].len += wordcraft_doc::para::OBJ.len_utf8();
        p.objects.push(rectangle(Wrap::InFrontOfText, 0.0));
        let mut t = wordcraft_doc::Table::new(1, 1, FRAME_WIDTH);
        t.rows[0].cells[0].blocks = vec![para_block(p)];
        d.parts.get_mut(&root).unwrap().blocks.push(Arc::new(Block::Table(t)));
        let l = lay(&d);
        let owners: Vec<_> = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|it| match it {
                Placed::Shape { object: Some(owner), .. } if owner.story == StoryRef::Part(root) => Some(owner),
                _ => None,
            })
            .collect();
        assert_eq!(owners.len(), 1);
        assert!(owners[0].path.0.len() > 1);
        assert_eq!(draws(&d, &l), 1);
    }

    #[test]
    fn inset_and_color_are_not_keys() {
        let (mut d, root) = policy_doc(Some(rectangle(Wrap::BehindText, 123.0)));
        d.parts.get_mut(&root).unwrap().body.inset = Some([12700; 4]);
        let child = d.para_mut(StoryRef::Part(root), &Path::top(0)).unwrap().object_at_mut(0).unwrap();
        if let InlineObject::Shape { fill, w, h, .. } = child {
            *fill = Some(wordcraft_doc::fill::Fill::solid(Rgb(107, 31, 48)));
            *w = 53.0;
            *h = 19.0;
        }
        let parent = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().object_at_mut(0).unwrap();
        if let InlineObject::Shape { w, h, .. } = parent {
            *w = 230.0;
            *h = 120.0;
        }
        let l = lay(&d);
        assert!((frame(&l, root).h - 19.0).abs() < 0.01);
        assert_eq!(paint(&l, &Pos::new(StoryRef::Part(root), Path::top(0), 0)), 0);
    }
}
