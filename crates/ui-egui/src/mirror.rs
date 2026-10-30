//! Mirrored interface for right-to-left languages (Hebrew). Instead of every panel, menu and dialog
//! laying itself out twice, the whole window is mirrored as it is painted: an egui plugin flips
//! each shape of the finished frame about the window's centre, and flips the pointer on the way
//! in, so a click lands on what is drawn under it. Text moves as a block and keeps its glyphs, so
//! a label lines up on the right, and the lines of a wrapped label are aligned right. The egui
//! fork lays right-to-left text out itself (UAX #9, paragraph by paragraph), so labels and catalog
//! entries stay in logical order. Icons drawn in code flip, as they do in right-to-left Word;
//! pictures keep their orientation.
//!
//! What has its own direction moves without flipping:
//! - the document: the page canvas with its rulers (a document's paragraphs say which way they
//!   run);
//! - the text field being edited: its text, caret and selection move together, mirrored as one
//!   block inside the field, so the caret stays by the letter it stands at.
//!
//! Popups placed from the page (context menu, mini toolbar, tooltips over the page) are mirrored
//! about the point they open from, so they open leftwards from it, as right-to-left menus do.
//!
//! Each window (egui viewport) is mirrored on its own: one runs its pass inside another's, so the
//! plugin keeps what it learns per viewport.

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::{ClippedShape, Shape};
use egui::layers::ShapeIdx;
use egui::{Context, Event, FullOutput, Id, LayerId, Order, Pos2, RawInput, Rect, Ui, Vec2, ViewportId};

/// How the pointer goes from where it is on screen to where the widgets were laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mapping {
    /// On a popup placed from the page: mirrored about its axis (the sum `x + x'`).
    Mirrored(f32),
    /// On the text field being edited (logical): moved back by this much, and kept inside it.
    Field(f32, Rect),
    /// Anywhere else in the interface: flipped about the window's centre.
    Flipped,
    /// On the page: moved back by this much.
    Moved(f32),
}

/// How a popup placed from the page is mirrored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// Opened rightwards from a point on the page (its left edge): it opens leftwards from it.
    Edge,
    /// Placed over the page (under a field, centred on it): it stays there, mirrored inside.
    InPlace,
}

/// The plugin; [`configure`] sets it up each frame, for the window being drawn.
#[derive(Default)]
pub struct Mirror {
    views: HashMap<ViewportId, View>,
    /// The viewport whose pass is ending: its output comes next.
    ending: Option<ViewportId>,
}

/// What the mirror knows of one window.
#[derive(Default)]
struct View {
    on: bool,
    /// The document canvas (logical coordinates), when it is on screen.
    canvas: Option<Rect>,
    /// Popups placed from document coordinates (their area ids), and how each is mirrored.
    anchored: Vec<(Id, Anchor)>,
    /// Where those popups were this frame (logical), with the axis they were mirrored about (as
    /// the sum `x + x'`), for the pointer.
    anchored_rects: Vec<(Rect, f32)>,
    /// The other menus, popups and windows on screen (logical): they're flipped with the
    /// interface, and the pointer over one goes to it even where it's drawn over the page.
    floating: Vec<Rect>,
    /// Cards painted in a layer over the page that egui doesn't size by them (floating
    /// comments): floating too.
    overlays: Vec<Rect>,
    /// How many of the frame's shapes are those of menus, popups and windows. They come after
    /// the page and the panels (egui paints background layers first), and are never the page's
    /// even where they lie over it: a dialog's text box or check mark there is flipped too.
    floating_shapes: usize,
    /// The text field being edited (logical), and how far its contents moved last frame.
    field: Option<Rect>,
    field_shift: Option<f32>,
    /// The window's horizontal extent, from the last frame.
    span: Option<(f32, f32)>,
    /// The pointer as last seen on screen: wheel scrolling over the page isn't flipped.
    pointer: Option<Pos2>,
    /// The mapping of the button held down, kept until it is released.
    drag: Option<Mapping>,
}

impl egui::Plugin for Mirror {
    fn debug_name(&self) -> &'static str {
        "wordcraft mirror"
    }

    fn input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        self.views.entry(input.viewport_id).or_default().input(input);
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let id = ctx.viewport_id();
        self.ending = Some(id);
        self.views.entry(id).or_default().end_pass(&ctx);
    }

    fn output_hook(&mut self, _ctx: &Context, output: &mut FullOutput) {
        let id = self.ending.take().unwrap_or(ViewportId::ROOT);
        if let Some(view) = self.views.get_mut(&id) {
            view.output(output);
        }
    }
}

impl View {
    /// Map the pointer back from where it is on screen to where the widgets were laid out.
    fn input(&mut self, input: &mut RawInput) {
        if let Some(r) = input.screen_rect {
            self.span = Some((r.min.x, r.max.x));
        }
        // A release that never came (the window lost focus mid-drag, or the mirror was off) must
        // not hold the next press to an old mapping.
        if !self.on || input.events.iter().any(|e| matches!(e, Event::WindowFocused(false))) {
            self.drag = None;
        }
        let Some(span) = self.span.filter(|_| self.on) else { return };
        // Areas kept unflipped, with how far each moves: the field being edited and the page.
        let field = self.field.map(|f| (f, self.field_shift.unwrap_or_else(|| keep_shift(span, f))));
        let canvas = self.canvas.map(|c| (c, keep_shift(span, c)));
        let shown = |(r, d): (Rect, f32), p: Pos2| r.translate(Vec2::new(d, 0.0)).contains(p);
        let anchored = &self.anchored_rects;
        let floating = &self.floating;
        // Over a menu, popup or window other than the one holding `inside`?
        let on_floating = |p: Pos2, inside: Option<Rect>| {
            let q = Pos2::new(flip_x(span, p.x), p.y);
            floating.iter().any(|r| r.contains(q) && !inside.is_some_and(|i| r.contains_rect(i)))
        };
        let mapping_at = |p: Pos2| -> Mapping {
            if let Some((_, sum)) = anchored.iter().find(|(r, sum)| flip_rect((0.0, *sum), *r).contains(p)) {
                return Mapping::Mirrored(*sum);
            }
            // The field being edited (in a dialog too) is drawn moved, not flipped: so is the pointer,
            // with the field's contents. Beside its text it stays in the field, past the text's end.
            if let Some((f, d)) = field.filter(|&(f, _)| shown((f, keep_shift(span, f)), p) && !on_floating(p, Some(f))) {
                return Mapping::Field(d, f);
            }
            if on_floating(p, None) {
                return Mapping::Flipped;
            }
            match canvas.filter(|&k| shown(k, p)) {
                Some((_, d)) => Mapping::Moved(d),
                None => Mapping::Flipped,
            }
        };
        let apply = |m: Mapping, p: Pos2| -> Pos2 {
            match m {
                Mapping::Mirrored(sum) => Pos2::new(sum - p.x, p.y),
                Mapping::Field(d, f) => Pos2::new((p.x - d).clamp(f.min.x + 0.5, (f.max.x - 0.5).max(f.min.x + 0.5)), p.y),
                Mapping::Moved(d) => Pos2::new(p.x - d, p.y),
                Mapping::Flipped => Pos2::new(flip_x(span, p.x), p.y),
            }
        };
        let over_page = |p: Option<Pos2>| p.is_some_and(|p| !on_floating(p, None) && canvas.is_some_and(|k| shown(k, p)));
        let flip_wheel = !over_page(self.pointer);
        for event in &mut input.events {
            match event {
                Event::PointerMoved(p) => {
                    self.pointer = Some(*p);
                    *p = apply(self.drag.unwrap_or_else(|| mapping_at(*p)), *p);
                }
                // A press fixes the mapping until its release, so a drag that leaves what it started
                // on (a dialog dragged quickly over the page) doesn't jump to another mapping.
                Event::PointerButton { pos, pressed, .. } => {
                    let m = self.drag.unwrap_or_else(|| mapping_at(*pos));
                    self.drag = pressed.then_some(m);
                    *pos = apply(m, *pos);
                }
                Event::Touch { pos, .. } => *pos = apply(mapping_at(*pos), *pos),
                Event::PointerGone => self.pointer = None,
                Event::MouseMoved(delta) => delta.x = -delta.x,
                Event::MouseWheel { delta, .. } if flip_wheel => delta.x = -delta.x,
                _ => {}
            }
        }
    }

    /// Note what this frame laid out (the field being edited, the menus and popups) and mirror the
    /// popups placed from the page about the point they open from.
    fn end_pass(&mut self, ctx: &Context) {
        let screen = ctx.content_rect();
        let span = (screen.min.x, screen.max.x);
        self.span = Some(span);
        self.anchored_rects.clear();
        self.floating.clear();
        self.floating_shapes = 0;
        let field = editing_field(ctx).filter(|_| self.on);
        if field != self.field {
            self.field_shift = None;
        }
        self.field = field;
        if !self.on {
            return;
        }
        let above: Vec<LayerId> = ctx.memory(|m| m.layer_ids().filter(|l| l.order != Order::Background).collect());
        self.floating_shapes = ctx.graphics(|g| above.iter().filter_map(|l| g.get(*l)).map(|list| list.next_idx().0).sum());
        // Menus, popups and windows (the page is in the background, in the central panel).
        let anchored = &self.anchored;
        let layers: Vec<LayerId> = ctx.memory(|m| {
            let areas = m.areas();
            m.layer_ids()
                .filter(|l| {
                    matches!(l.order, Order::Middle | Order::Foreground) && areas.is_visible(l) && !anchored.iter().any(|(id, _)| *id == l.id)
                })
                .collect()
        });
        self.floating =
            layers.into_iter().filter_map(|l| egui::containers::AreaState::load(ctx, l.id).filter(|s| s.interactable).map(|s| s.rect())).collect();
        self.floating.extend(self.overlays.iter().copied());
        let Some(canvas) = self.canvas else { return };
        // Tooltips follow the pointer: over the page they belong to the page.
        const ORDERS: [Order; 5] = [Order::Background, Order::Middle, Order::Foreground, Order::Tooltip, Order::Debug];
        let mut layers: Vec<(LayerId, Anchor)> =
            self.anchored.iter().flat_map(|&(id, anchor)| ORDERS.map(|o| (LayerId::new(o, id), anchor))).collect();
        if ctx.pointer_hover_pos().is_some_and(|p| canvas.contains(p)) {
            layers.extend(ctx.memory(|m| m.layer_ids().filter(|l| l.order == Order::Tooltip).map(|l| (l, Anchor::Edge)).collect::<Vec<_>>()));
        }
        // A popup opens rightwards from logical point `a` (its left edge), which is drawn at
        // `a + d` with the page. Moved here and flipped with everything else, it is mirrored about
        // that point: `x' = 2a + d - x`, so it opens leftwards from it. One placed over the page
        // is mirrored about its own centre instead, and stays where it was, moved with the page.
        let d = keep_shift(span, canvas);
        let mut placed = Vec::new();
        for (layer, anchor) in layers {
            let Some(area) = ctx.memory(|m| m.area_rect(layer.id)).filter(|r| r.is_positive()) else { continue };
            // Drawn at `sum - x`: kept on screen, as egui keeps it in its own coordinates.
            let mut sum = match anchor {
                Anchor::Edge => 2.0 * area.min.x,
                Anchor::InPlace => area.min.x + area.max.x,
            } + d;
            if area.width() <= span.1 - span.0 {
                sum = sum.clamp(span.0 + area.max.x, span.1 + area.min.x);
            }
            let shift = Vec2::new(span.0 + span.1 - sum, 0.0);
            let found = ctx.graphics_mut(|g| {
                let Some(list) = g.get_mut(layer) else { return false };
                for i in 0..list.next_idx().0 {
                    list.mutate_shape(ShapeIdx(i), |c| {
                        c.clip_rect = c.clip_rect.translate(shift);
                        c.shape.translate(shift);
                    });
                }
                true
            });
            if found {
                placed.push((area, sum));
            }
        }
        self.anchored_rects = placed;
    }

    /// Mirror the finished frame, and the rectangles the platform is told about with it.
    fn output(&mut self, output: &mut FullOutput) {
        let Some(span) = self.span.filter(|_| self.on) else { return };
        let first_floating = output.shapes.len().saturating_sub(self.floating_shapes);
        self.field_shift = self.field.map(|f| field_shift(span, f, &output.shapes));
        let field = self.field.zip(self.field_shift);
        for (i, clipped) in output.shapes.iter_mut().enumerate() {
            let canvas = self.canvas.filter(|_| i < first_floating);
            mirror_clipped(span, canvas, field, clipped);
        }
        if let Some(ime) = &mut output.platform_output.ime {
            ime.rect = self.place_ime(span, ime.rect);
            ime.cursor_rect = self.place_ime(span, ime.cursor_rect);
        }
        // Screen readers, magnifiers and their hit testing find each element where it is drawn.
        if let Some(update) = &mut output.platform_output.accesskit_update {
            for (_, node) in &mut update.nodes {
                if let Some(b) = node.bounds() {
                    let r = self.place(span, Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32)));
                    node.set_bounds(egui::accesskit::Rect { x0: r.min.x.into(), x1: r.max.x.into(), ..b });
                }
            }
        }
    }

    /// Where the input method's rectangle (`r`, logical) is drawn. It is the field being edited's,
    /// or else the page caret's, which moves with the page even under a menu or the Focus ribbon
    /// dropped over it, as the caret itself does.
    fn place_ime(&self, span: (f32, f32), r: Rect) -> Rect {
        if self.field.is_none()
            && let Some(c) = self.canvas.filter(|c| c.expand(1.0).contains_rect(r))
        {
            return r.translate(Vec2::new(keep_shift(span, c), 0.0));
        }
        self.place(span, r)
    }

    /// Where an element of the frame (`r`, logical) is drawn, as the pointer finds it: a popup
    /// placed from the page is mirrored about its axis, the field being edited and the page move,
    /// menus, popups and dialogs (over the page too) flip with the rest of the interface.
    fn place(&self, span: (f32, f32), r: Rect) -> Rect {
        if let Some((_, sum)) = self.anchored_rects.iter().find(|(a, _)| a.expand(1.0).contains_rect(r)) {
            return flip_rect((0.0, *sum), r);
        }
        if let Some(f) = self.field.filter(|f| f.expand(1.0).contains_rect(r)) {
            // The field itself (its frame) flips in place; what is inside moves with its text.
            let d = if r.width() >= f.width() - 1.0 { keep_shift(span, f) } else { self.field_shift.unwrap_or_else(|| keep_shift(span, f)) };
            return r.translate(Vec2::new(d, 0.0));
        }
        match self.canvas.filter(|c| c.expand(1.0).contains_rect(r) && !self.floating.iter().any(|f| f.contains(r.center()))) {
            Some(c) => r.translate(Vec2::new(keep_shift(span, c), 0.0)),
            None => flip_rect(span, r),
        }
    }
}

/// Whether this frame's window is mirrored: icon letters (`x²`) are placed so the mirror keeps
/// their order.
pub fn active(ctx: &Context) -> bool {
    ctx.plugin_opt::<Mirror>().is_some_and(|h| h.lock().views.get(&ctx.viewport_id()).is_some_and(|v| v.on))
}

/// Turn the mirror on (a right-to-left interface) or off for the window being drawn. `canvas` is
/// the document canvas with its rulers when it is on screen, `anchored` the area ids of popups
/// placed from it, with how each is mirrored, and `overlays` cards painted over it in a layer
/// egui doesn't size by them (floating comments). Call once per frame, after the interface is
/// laid out.
pub fn configure(ctx: &Context, on: bool, canvas: Option<Rect>, anchored: &[(Id, Anchor)], overlays: &[Rect]) {
    if ctx.plugin_opt::<Mirror>().is_none() {
        ctx.add_plugin(Mirror::default());
    }
    if let Some(handle) = ctx.plugin_opt::<Mirror>() {
        let mut m = handle.lock();
        let view = m.views.entry(ctx.viewport_id()).or_default();
        view.on = on;
        view.canvas = canvas;
        view.anchored = anchored.to_vec();
        view.overlays = overlays.to_vec();
    }
}

/// Where the mirror draws `r`, laid out on the page (or the canvas round it) of the window
/// `viewport`: the control channel's coordinates are the window's as drawn.
pub fn page_to_window(ctx: &Context, viewport: ViewportId, r: Rect) -> Rect {
    let Some(handle) = ctx.plugin_opt::<Mirror>() else { return r };
    let m = handle.lock();
    match m.views.get(&viewport).map(|v| (v.on, v.span, v.canvas)) {
        Some((true, Some(span), Some(canvas))) => r.translate(Vec2::new(keep_shift(span, canvas), 0.0)),
        _ => r,
    }
}

/// Where the mirror draws `r`, laid out in the interface of the window `viewport` (a menu, or a
/// floating comment over the page): flipped with the interface.
pub fn interface_to_window(ctx: &Context, viewport: ViewportId, r: Rect) -> Rect {
    let Some(handle) = ctx.plugin_opt::<Mirror>() else { return r };
    let m = handle.lock();
    match m.views.get(&viewport).map(|v| (v.on, v.span)) {
        Some((true, Some(span))) => flip_rect(span, r),
        _ => r,
    }
}

fn flip_x((min, max): (f32, f32), x: f32) -> f32 {
    min + max - x
}

fn flip_rect(span: (f32, f32), r: Rect) -> Rect {
    if !r.is_finite() {
        return r;
    }
    Rect::from_x_y_ranges(flip_x(span, r.max.x)..=flip_x(span, r.min.x), r.y_range())
}

/// How far an area kept unflipped moves to sit where the mirror puts it.
fn keep_shift(span: (f32, f32), k: Rect) -> f32 {
    flip_x(span, k.max.x) - k.min.x
}

/// The text field being edited, if any: its caret and selection are drawn apart from its text.
fn editing_field(ctx: &Context) -> Option<Rect> {
    let id: Id = ctx.memory(|m| m.focused())?;
    egui::TextEdit::load_state(ctx, id)?;
    ctx.read_response(id).map(|r| r.rect)
}

/// Is `clipped` drawn inside the text field `field`: its text, caret or selection? egui clips
/// those to the field's inside; the field's frame is clipped with the panel around it.
fn in_field(field: Rect, clipped: &ClippedShape) -> bool {
    clipped.clip_rect.is_positive() && field.expand(1.0).contains_rect(clipped.clip_rect)
}

/// How far the contents of the field being edited move: as one block, mirrored about the
/// window's centre like any label, so text that starts at the field's left margin ends at its
/// right one, and the caret and selection stay where they are in the text. Text wider than the
/// field (scrolled) moves with the field instead.
fn field_shift(span: (f32, f32), field: Rect, shapes: &[ClippedShape]) -> f32 {
    let mut block = Rect::NOTHING;
    for clipped in shapes.iter().filter(|c| in_field(field, c)) {
        block = block.union(content_rect(&clipped.shape));
    }
    if !block.is_finite() || block.width() > field.width() {
        return keep_shift(span, field);
    }
    flip_x(span, block.max.x) - block.min.x
}

/// What a shape inside a text field takes up: a text's laid-out box (its glyphs may overhang),
/// the caret's line itself (not its stroke), anything else's bounds.
fn content_rect(shape: &Shape) -> Rect {
    match shape {
        Shape::Vec(shapes) => shapes.iter().fold(Rect::NOTHING, |r, s| r.union(content_rect(s))),
        Shape::Text(t) => t.galley.rect.translate(t.pos.to_vec2()),
        Shape::LineSegment { points, .. } => Rect::from_two_pos(points[0], points[1]),
        s => s.visual_bounding_rect(),
    }
}

/// Mirror one shape of the finished frame: shapes clipped to the canvas move without flipping,
/// and so do those inside the field being edited, by the field's own shift (`field`).
fn mirror_clipped(span: (f32, f32), canvas: Option<Rect>, field: Option<(Rect, f32)>, clipped: &mut ClippedShape) {
    let d = if let Some(k) = canvas.filter(|c| c.expand(1.0).contains_rect(clipped.clip_rect)) {
        keep_shift(span, k)
    } else if let Some((_, d)) = field.filter(|(f, _)| in_field(*f, clipped)) {
        d
    } else {
        clipped.clip_rect = flip_rect(span, clipped.clip_rect);
        mirror_shape(span, &mut clipped.shape);
        return;
    };
    let d = Vec2::new(d, 0.0);
    clipped.clip_rect = clipped.clip_rect.translate(d);
    clipped.shape.translate(d);
}

fn mirror_shape(span: (f32, f32), shape: &mut Shape) {
    let fx = |p: &mut Pos2| p.x = flip_x(span, p.x);
    match shape {
        Shape::Noop => {}
        Shape::Vec(shapes) => shapes.iter_mut().for_each(|s| mirror_shape(span, s)),
        Shape::Circle(c) => fx(&mut c.center),
        Shape::Ellipse(e) => fx(&mut e.center),
        Shape::LineSegment { points, .. } => points.iter_mut().for_each(fx),
        // A check box's tick keeps its direction, as in right-to-left Windows: it only moves.
        Shape::Path(p) if !p.closed && is_check_mark(&p.points) => {
            let r = Rect::from_points(&p.points);
            let d = flip_x(span, r.max.x) - r.min.x;
            p.points.iter_mut().for_each(|q| q.x += d);
        }
        Shape::Path(p) => p.points.iter_mut().for_each(fx),
        Shape::QuadraticBezier(b) => b.points.iter_mut().for_each(fx),
        Shape::CubicBezier(b) => b.points.iter_mut().for_each(fx),
        Shape::Rect(r) => {
            r.rect = flip_rect(span, r.rect);
            let c = &mut r.corner_radius;
            std::mem::swap(&mut c.nw, &mut c.ne);
            std::mem::swap(&mut c.sw, &mut c.se);
        }
        // Text moves as a block and keeps its glyphs (laid out in visual order by the fork);
        // wrapped lines align right.
        Shape::Text(t) => {
            let r = t.galley.rect.translate(t.pos.to_vec2());
            t.pos.x += flip_x(span, r.max.x) - r.min.x;
            if t.galley.rows.len() > 1 {
                let galley = Arc::make_mut(&mut t.galley);
                let right = galley.rect.max.x;
                for row in &mut galley.rows {
                    row.pos.x += right - row.rect().max.x;
                }
            }
        }
        Shape::Mesh(mesh) => {
            let m = Arc::make_mut(mesh);
            if m.texture_id == egui::TextureId::default() {
                // Drawn in code (icons, arrows): flip.
                m.vertices.iter_mut().for_each(|v| fx(&mut v.pos));
            } else {
                // A picture: move it, keep it the right way round.
                let r = m.calc_bounds();
                m.translate(Vec2::new(flip_x(span, r.max.x) - r.min.x, 0.0));
            }
        }
        Shape::Callback(c) => c.rect = flip_rect(span, c.rect),
        // A band (a chart's filled range) is drawn along its own axis: it moves, unflipped.
        Shape::Band(b) => {
            let r = b.visual_bounding_rect();
            let mut band = Shape::Band(b.clone());
            band.translate(Vec2::new(flip_x(span, r.max.x) - r.min.x, 0.0));
            *shape = band;
        }
    }
}

/// egui's check box tick: a line from the middle of a square's left side to the middle of its
/// bottom, then up to its top right corner.
fn is_check_mark(p: &[Pos2]) -> bool {
    let [a, b, c] = p else { return false };
    let close = |x: f32, y: f32| (x - y).abs() < 0.01;
    a.x < b.x && b.x < c.x && close(b.x - a.x, c.x - b.x) && c.y < a.y && a.y < b.y && close(a.y, (b.y + c.y) / 2.0) && close(c.x - a.x, b.y - c.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Stroke, pos2};

    const SPAN: (f32, f32) = (0.0, 100.0);

    fn view(canvas: Option<Rect>) -> View {
        View { on: true, canvas, ..Default::default() }
    }

    #[test]
    fn shapes_flip_about_the_window_centre() {
        let rect = Rect::from_min_max(pos2(10.0, 0.0), pos2(30.0, 5.0));
        let mut filled = ClippedShape::new(Rect::EVERYTHING, Shape::rect_filled(rect, 0.0, Color32::RED));
        mirror_clipped(SPAN, None, None, &mut filled);
        assert_eq!(filled.shape.visual_bounding_rect(), Rect::from_min_max(pos2(70.0, 0.0), pos2(90.0, 5.0)));
        let mut line = Shape::line_segment([pos2(0.0, 1.0), pos2(20.0, 1.0)], Stroke::new(1.0, Color32::RED));
        mirror_shape(SPAN, &mut line);
        assert!(matches!(line, Shape::LineSegment { points, .. } if points == [pos2(100.0, 1.0), pos2(80.0, 1.0)]));
    }

    #[test]
    fn the_page_and_the_field_being_edited_move_without_flipping() {
        // The canvas spans 0–60 of a 100-wide window: mirrored it spans 40–100, its contents in
        // the same order.
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0));
        let caret = Rect::from_min_max(pos2(5.0, 0.0), pos2(6.0, 9.0));
        let mut on_page = ClippedShape::new(canvas, Shape::rect_filled(caret, 0.0, Color32::BLACK));
        mirror_clipped(SPAN, Some(canvas), None, &mut on_page);
        assert_eq!(on_page.shape.visual_bounding_rect().min.x, 45.0);
        assert_eq!(on_page.clip_rect, Rect::from_min_max(pos2(40.0, 0.0), pos2(100.0, 50.0)));
        // A field at 70–90 mirrors to 10–30. Its text at 72–80 and its caret at 80 (clipped to
        // the field's inside) move as one block, to 20–28 and 20: the caret stays after the text.
        let field = Rect::from_min_max(pos2(70.0, 60.0), pos2(90.0, 70.0));
        let inside = field.shrink(1.0);
        let text = Rect::from_min_max(pos2(72.0, 61.0), pos2(80.0, 69.0));
        let mut shapes = vec![
            ClippedShape::new(inside, Shape::rect_filled(text, 0.0, Color32::BLACK)),
            ClippedShape::new(inside, Shape::line_segment([pos2(80.0, 61.0), pos2(80.0, 69.0)], Stroke::new(1.0, Color32::BLACK))),
            // The frame, clipped with the panel round it, flips in place.
            ClippedShape::new(Rect::EVERYTHING, Shape::rect_filled(field, 0.0, Color32::GRAY)),
        ];
        let d = field_shift(SPAN, field, &shapes);
        assert_eq!(d, -52.0);
        for s in &mut shapes {
            mirror_clipped(SPAN, Some(canvas), Some((field, d)), s);
        }
        let x = |i: usize| (shapes[i].shape.visual_bounding_rect().min.x, shapes[i].shape.visual_bounding_rect().max.x);
        assert_eq!(x(0), (20.0, 28.0));
        assert!(matches!(shapes[1].shape, Shape::LineSegment { points, .. } if points[0].x == 28.0), "the caret after the text");
        assert_eq!(x(2), (10.0, 30.0));
        // Text wider than the field (scrolled) moves with the field.
        let wide = [ClippedShape::new(inside, Shape::rect_filled(Rect::from_min_max(pos2(40.0, 61.0), pos2(89.0, 69.0)), 0.0, Color32::BLACK))];
        assert_eq!(field_shift(SPAN, field, &wide), keep_shift(SPAN, field));
    }

    #[test]
    fn the_pointer_is_flipped_back_to_where_widgets_are_laid_out() {
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0));
        let mut m = view(Some(canvas));
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 80.0))),
            events: vec![
                Event::PointerMoved(pos2(90.0, 70.0)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(3.0, 1.0),
                    modifiers: Default::default(),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                },
                Event::PointerMoved(pos2(45.0, 10.0)),
            ],
            ..Default::default()
        };
        m.input(&mut input);
        assert!(matches!(input.events[0], Event::PointerMoved(p) if p == pos2(10.0, 70.0)), "outside the page: flipped");
        assert!(matches!(input.events[1], Event::MouseWheel { delta, .. } if delta == egui::vec2(-3.0, 1.0)), "sideways scrolling flips");
        assert!(matches!(input.events[2], Event::PointerMoved(p) if p == pos2(5.0, 10.0)), "on the mirrored page: shifted");
        // A context menu laid out at 10–30 and mirrored about 130 (`2a + d`) is drawn at 100–120.
        m.anchored_rects = vec![(Rect::from_min_max(pos2(10.0, 0.0), pos2(30.0, 40.0)), 130.0)];
        let mut on_menu = RawInput { events: vec![Event::PointerMoved(pos2(112.0, 5.0))], ..Default::default() };
        m.input(&mut on_menu);
        assert!(matches!(on_menu.events[0], Event::PointerMoved(p) if p == pos2(18.0, 5.0)), "on a popup from the page: its own axis");
        // The Font list, laid out at 50–75 and flipped to 25–50, hangs over the page (drawn at
        // 40–100): the pointer there goes to the list, beside it to the page.
        m.floating = vec![Rect::from_min_max(pos2(50.0, 0.0), pos2(75.0, 40.0))];
        let mut list = RawInput { events: vec![Event::PointerMoved(pos2(45.0, 5.0)), Event::PointerMoved(pos2(55.0, 5.0))], ..Default::default() };
        m.input(&mut list);
        assert!(matches!(list.events[0], Event::PointerMoved(p) if p == pos2(55.0, 5.0)), "on a menu over the page: flipped");
        assert!(matches!(list.events[1], Event::PointerMoved(p) if p == pos2(15.0, 5.0)), "beside it, on the page: shifted");
        m.on = false;
        let mut plain = RawInput { events: vec![Event::PointerMoved(pos2(90.0, 70.0))], ..Default::default() };
        m.input(&mut plain);
        assert!(matches!(plain.events[0], Event::PointerMoved(p) if p == pos2(90.0, 70.0)), "off: untouched");
    }

    /// A dialog over the page is flipped like the rest of the interface: its shapes (the last of
    /// the frame) aren't taken for the page's, even where they're clipped inside it.
    #[test]
    fn dialogs_over_the_page_are_flipped() {
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0));
        let mut m = View { span: Some(SPAN), floating_shapes: 1, ..view(Some(canvas)) };
        let r = Rect::from_min_max(pos2(10.0, 10.0), pos2(20.0, 20.0));
        let mut out = egui::FullOutput::default();
        for _ in 0..2 {
            out.shapes.push(ClippedShape::new(r, Shape::rect_filled(r, 0.0, Color32::BLACK)));
        }
        m.output(&mut out);
        assert_eq!(out.shapes[0].shape.visual_bounding_rect().min.x, 50.0, "the page's: moved with the page");
        assert_eq!(out.shapes[1].shape.visual_bounding_rect().min.x, 80.0, "the dialog's: flipped");
        // Its text box being edited (laid out at 30–50, drawn at 50–70) holds text at 30–40,
        // drawn mirrored at 60–70: the pointer on a letter goes to that letter.
        m.field = Some(Rect::from_min_max(pos2(30.0, 0.0), pos2(50.0, 10.0)));
        m.field_shift = Some(30.0);
        m.floating = vec![Rect::from_min_max(pos2(25.0, 0.0), pos2(55.0, 40.0))];
        let mut input = RawInput {
            events: vec![Event::PointerMoved(pos2(62.0, 5.0)), Event::PointerMoved(pos2(52.0, 5.0)), Event::PointerMoved(pos2(52.0, 20.0))],
            ..Default::default()
        };
        m.input(&mut input);
        assert!(matches!(input.events[0], Event::PointerMoved(p) if p == pos2(32.0, 5.0)), "on its text: moved with the text");
        assert!(matches!(input.events[1], Event::PointerMoved(p) if p == pos2(30.5, 5.0)), "in the field, left of its text: at the field's edge");
        assert!(matches!(input.events[2], Event::PointerMoved(p) if p == pos2(48.0, 20.0)), "elsewhere in the dialog: flipped");
    }

    /// A dialog dragged quickly: the pointer leaves the dialog for the page between two frames, but
    /// keeps the mapping it was pressed with until the button is released, so the dialog doesn't
    /// jump to its mirrored place.
    #[test]
    fn a_drag_keeps_the_mapping_it_started_with() {
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0));
        // A dialog laid out at 50–75 is drawn at 25–50, over the page (drawn at 40–100).
        let mut m = View { floating: vec![Rect::from_min_max(pos2(50.0, 0.0), pos2(75.0, 40.0))], ..view(Some(canvas)) };
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut input = RawInput {
            screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 80.0))),
            events: vec![
                button(pos2(30.0, 5.0), true),
                Event::PointerMoved(pos2(70.0, 5.0)),
                button(pos2(70.0, 5.0), false),
                Event::PointerMoved(pos2(80.0, 6.0)),
            ],
            ..Default::default()
        };
        m.input(&mut input);
        assert!(matches!(input.events[0], Event::PointerButton { pos, .. } if pos == pos2(70.0, 5.0)), "pressed on the dialog: flipped");
        assert!(matches!(input.events[1], Event::PointerMoved(p) if p == pos2(30.0, 5.0)), "dragged over the page: still flipped");
        assert!(matches!(input.events[2], Event::PointerButton { pos, .. } if pos == pos2(30.0, 5.0)), "released: flipped");
        assert!(
            matches!(input.events[3], Event::PointerMoved(p) if p == pos2(40.0, 6.0)),
            "after the release, the page's mapping (moved, not flipped to 20)"
        );
    }

    /// A drag whose release never comes (the window lost focus, or the mirror was turned off)
    /// doesn't hold the next press to its mapping.
    #[test]
    fn a_drag_cut_short_lets_go_of_its_mapping() {
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0));
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let frame = |m: &mut View, events| {
            let mut input = RawInput { screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 80.0))), events, ..Default::default() };
            m.input(&mut input);
            input.events
        };
        // Pressed on a dialog (flipped), then focus lost before the release.
        for cut in [vec![Event::WindowFocused(false)], vec![]] {
            let mut m = View { floating: vec![Rect::from_min_max(pos2(50.0, 0.0), pos2(75.0, 40.0))], ..view(Some(canvas)) };
            frame(&mut m, vec![button(pos2(30.0, 5.0), true)]);
            assert!(m.drag.is_some());
            if cut.is_empty() {
                m.on = false;
                frame(&mut m, vec![]);
                m.on = true;
            } else {
                frame(&mut m, cut);
            }
            let events = frame(&mut m, vec![button(pos2(90.0, 5.0), true)]);
            assert!(matches!(events[0], Event::PointerButton { pos, .. } if pos == pos2(50.0, 5.0)), "a press on the page: moved, not flipped to 10");
        }
    }

    /// A red popup 150 wide placed from the page at `x`, in a mirrored 400-wide window whose page
    /// is laid out at 0–300 and drawn at 100–400: where it is drawn, and the axis clicks map through.
    fn popup_from_the_page(anchor: Anchor, x: f32) -> (Rect, Option<f32>) {
        let ctx = Context::default();
        let popup = egui::Id::unique("popup from the page");
        configure(&ctx, true, Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 300.0))), &[(popup, anchor)], &[]);
        let screen = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))), ..Default::default() };
        let mut drawn = None;
        for _ in 0..3 {
            let mut out = ctx.run_ui(screen.clone(), |ui| {
                egui::Area::new(popup).fixed_pos(pos2(x, 20.0)).show(ui.ctx(), |ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(150.0, 40.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 0.0, Color32::RED);
                });
            });
            out.textures_delta.clear();
            drawn = out.shapes.iter().find_map(|c| match &c.shape {
                // Red, faded in or not.
                Shape::Rect(r) if r.fill.r() > 0 && r.fill.g() == 0 && r.fill.b() == 0 => Some(r.rect),
                _ => None,
            });
        }
        let handle = ctx.plugin_opt::<Mirror>().expect("the mirror is on");
        let axis = handle.lock().views[&ViewportId::ROOT].anchored_rects.first().map(|a| a.1);
        (drawn.expect("the popup is painted"), axis)
    }

    /// A floating comment is painted in a layer over the page that egui doesn't size by its cards
    /// (they're painted, not allocated). Told of the card, the mirror flips the pointer over it
    /// with the interface, as the card is drawn, so a click on the card as drawn reaches it.
    #[test]
    fn a_click_on_a_floating_comment_reaches_it() {
        let ctx = Context::default();
        // The page is laid out at 0–300 and drawn at 100–400; the card at 200–250 is drawn
        // flipped, at 150–200, where it's clicked.
        let card = Rect::from_min_max(pos2(200.0, 50.0), pos2(250.0, 80.0));
        let at = pos2(175.0, 65.0);
        let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut clicked = false;
        for events in [vec![], vec![], vec![Event::PointerMoved(at), press(true)], vec![press(false)]] {
            configure(&ctx, true, Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 300.0))), &[], &[card]);
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                egui::Area::new(egui::Id::unique("floating comment"))
                    .order(Order::Foreground)
                    .fixed_pos(Pos2::ZERO)
                    .sense(egui::Sense::hover())
                    .show(ui.ctx(), |ui| {
                        ui.painter().rect_filled(card, 0.0, Color32::RED);
                        clicked |= ui.interact(card, egui::Id::unique("card"), egui::Sense::click()).clicked();
                    });
            });
            out.textures_delta.clear();
        }
        assert!(clicked, "the click on the card as drawn reaches it");
    }

    /// Run `ui` over `events` in a mirrored 400 × 300 window with no page, one frame per entry.
    fn mirrored_frames(ctx: &Context, frames: Vec<Vec<Event>>, ui: impl Fn(&mut Ui)) {
        for events in frames {
            configure(ctx, true, None, &[], &[]);
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))), events, ..Default::default() };
            // A frame a sixtieth of a second after the last, as a window draws them.
            let input = RawInput { time: Some(ctx.input(|i| i.time) + 1.0 / 60.0), ..input };
            let mut out = ctx.run_ui(input, &ui);
            out.textures_delta.clear();
        }
    }

    /// A drag from `at` to `to`, one event a frame, after two frames that lay the widgets out: the
    /// pointer comes over `at`, presses, moves halfway and the rest, and lets go.
    fn drag_frames(at: Pos2, to: Pos2) -> Vec<Vec<Event>> {
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mid = at.lerp(to, 0.5);
        vec![
            vec![],
            vec![],
            vec![Event::PointerMoved(at)],
            vec![button(at, true)],
            vec![Event::PointerMoved(mid)],
            vec![Event::PointerMoved(to)],
            vec![button(to, false)],
        ]
    }

    /// The interface's scroll bars flip with it, to the left of what they scroll, and a drag on
    /// one where it is drawn scrolls: the pointer flips back onto the bar.
    #[test]
    fn a_scroll_bar_drawn_at_the_left_scrolls_when_dragged() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let offset = std::cell::Cell::new(0.0);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(400.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui(|ui| {
            configure(ui.ctx(), true, None, &[], &[]);
            let out = egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(ui, |ui| {
                ui.allocate_exact_size(egui::vec2(300.0, 3000.0), egui::Sense::hover());
            });
            offset.set(out.state.clamped_offset().y);
        });
        h.ctx.all_styles_mut(|s| s.spacing.scroll = egui::style::ScrollStyle::solid());
        for _ in 0..4 {
            h.step();
        }
        // Where it is drawn: the mirror reports the accessibility bounds as drawn.
        let bar = h.query_all_by_role(Role::ScrollBar).map(|n| n.rect()).find(|r| r.height() > r.width()).expect("the vertical scroll bar");
        // The 300-point content is drawn against the window's right edge, at 100–400.
        assert!(bar.max.x <= 100.0, "the bar is drawn left of the content: {bar:?}");
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let from = pos2(bar.center().x, bar.top() + 8.0);
        h.event(Event::PointerMoved(from));
        h.step();
        h.event(button(from, true));
        h.step();
        for k in 1..=6 {
            h.event(Event::PointerMoved(from + egui::vec2(0.0, 100.0) * (k as f32 / 6.0)));
            h.step();
        }
        h.event(button(from + egui::vec2(0.0, 100.0), false));
        h.step();
        drop(h);
        assert!(offset.get() > 100.0, "dragging the bar down scrolls down: {}", offset.get());
    }

    /// A number dragged sideways grows in the reading direction: leftwards in a mirrored
    /// interface, so dragging it right lowers it.
    #[test]
    fn a_number_dragged_right_in_the_mirrored_interface_goes_down() {
        let ctx = Context::default();
        let (value, field) = (std::cell::Cell::new(50.0_f64), std::cell::Cell::new(Rect::NOTHING));
        let show = |ui: &mut Ui| {
            let mut v = value.get();
            field.set(ui.add(egui::DragValue::new(&mut v).speed(1.0)).rect);
            value.set(v);
        };
        mirrored_frames(&ctx, vec![vec![]], show);
        let at = pos2(400.0 - field.get().center().x, field.get().center().y);
        mirrored_frames(&ctx, drag_frames(at, at + egui::vec2(30.0, 0.0)), show);
        assert!(value.get() < 50.0, "dragged right, the number goes down: {}", value.get());
    }

    /// A popup opened from the page near the window's left edge opens leftwards in a mirrored
    /// interface: it is kept on screen, as egui keeps it in its own coordinates.
    #[test]
    fn popups_from_the_page_stay_on_screen() {
        let (r, axis) = popup_from_the_page(Anchor::Edge, 10.0);
        assert!((r.min.x - 0.0).abs() < 0.5 && (r.max.x - 150.0).abs() < 0.5, "kept at the left edge, not at -40: {r:?}");
        assert_eq!(axis, Some(160.0), "clicks map through the same axis");
    }

    /// A popup placed over the page (a content control's list, under its field) moves with the
    /// page and stays over the same spot, instead of opening leftwards from its left edge.
    #[test]
    fn popups_placed_over_the_page_move_with_it() {
        let (r, axis) = popup_from_the_page(Anchor::InPlace, 120.0);
        assert!((r.min.x - 220.0).abs() < 0.5 && (r.max.x - 370.0).abs() < 0.5, "at 120–270 moved by 100, not 70–220: {r:?}");
        assert_eq!(axis, Some(490.0), "clicks map through its centre");
    }

    /// Hebrew typed into a field, laid out right to left by egui: in a mirrored interface the
    /// text sits against the field's right edge and the caret after the last letter typed, at the
    /// text's left end, with the letters in reading order (the first at the right).
    #[test]
    fn a_field_being_edited_lines_up_right_with_the_caret_after_the_last_letter() {
        let ctx = Context::default();
        let Some(face) = crate::theme::interface_fallback('ק') else {
            eprintln!("no installed font has Hebrew letters: skipped");
            return;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert("hebrew".into(), Arc::new(egui::FontData::from_owned(face.data().to_vec())));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().push("hebrew".into());
        ctx.set_fonts(fonts);
        configure(&ctx, true, None, &[], &[]);
        let id = egui::Id::unique("hebrew field");
        let mut text = String::from("אני רק בודק");
        let screen = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 300.0))), ..Default::default() };
        let mut placed = None;
        let mut frame = |ctx: &Context, text: &mut String| {
            let mut out = ctx.run_ui(screen.clone(), |ui| {
                configure(ui.ctx(), true, None, &[], &[]);
                let out = egui::TextEdit::singleline(text).id(id).desired_width(300.0).show(ui);
                out.response.request_focus();
                placed = Some((out.response.rect, out.text_clip_rect));
            });
            out.textures_delta.clear();
            out
        };
        frame(&ctx, &mut text);
        let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap_or_default();
        let end = egui::text::CCursor::new(text.chars().count());
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
        state.store(&ctx, id);
        frame(&ctx, &mut text);
        let out = frame(&ctx, &mut text);
        let (field, inside) = placed.expect("the field was shown");
        // The text's left margin in the field, mirrored: its right margin.
        let margin = inside.min.x - field.min.x;
        let mirrored_field = flip_rect((0.0, 600.0), field);
        let mut letters = (f32::MAX, f32::MIN);
        let mut first_letter = None;
        let mut caret = None;
        let mut walk: Vec<Shape> = out.shapes.iter().map(|c| c.shape.clone()).collect();
        while let Some(shape) = walk.pop() {
            match shape {
                Shape::Vec(v) => walk.extend(v),
                Shape::Text(t) if t.galley.job.text.contains('ק') => {
                    for placed in &t.galley.rows {
                        for g in &placed.row.glyphs {
                            let x = t.pos.x + placed.pos.x + g.pos.x;
                            letters = (letters.0.min(x), letters.1.max(x + g.advance_width));
                            if g.chr == 'א' {
                                first_letter = Some(x);
                            }
                        }
                    }
                }
                Shape::LineSegment { points, .. } if (points[0].x - points[1].x).abs() < 0.01 => caret = Some(points[0].x),
                _ => {}
            }
        }
        let caret = caret.expect("the caret is painted");
        assert!((letters.1 - (mirrored_field.max.x - margin)).abs() < 2.0, "lined up on the right: {letters:?}, field {mirrored_field:?}");
        assert!((caret - letters.0).abs() < 2.0, "caret {caret} after the last letter, at the left end {}", letters.0);
        assert!(first_letter.is_some_and(|x| x > (letters.0 + letters.1) / 2.0), "the first letter at the right");
    }

    /// Screen readers and the control channel find things where they are drawn: on the page they
    /// move with it, a dialog over the page flips with the interface, a popup placed from the page
    /// is mirrored about its axis, and the rest of the interface flips.
    #[test]
    fn accessibility_bounds_and_control_coordinates_follow_the_mirror() {
        let canvas = Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0));
        let m = View {
            span: Some(SPAN),
            floating: vec![Rect::from_min_max(pos2(50.0, 0.0), pos2(75.0, 40.0))],
            anchored_rects: vec![(Rect::from_min_max(pos2(10.0, 30.0), pos2(20.0, 45.0)), 70.0)],
            ..view(Some(canvas))
        };
        let r = |x0: f32, x1: f32, y: f32| Rect::from_min_max(pos2(x0, y), pos2(x1, y + 4.0));
        assert_eq!(m.place(SPAN, r(5.0, 9.0, 5.0)), r(45.0, 49.0, 5.0), "on the page (drawn at 40–100): moved");
        assert_eq!(m.place(SPAN, r(52.0, 56.0, 5.0)), r(44.0, 48.0, 5.0), "a dialog over the page: flipped");
        assert_eq!(m.place(SPAN, r(12.0, 14.0, 32.0)), r(56.0, 58.0, 32.0), "a popup from the page: about its axis");
        assert_eq!(m.place(SPAN, r(80.0, 90.0, 5.0)), r(10.0, 20.0, 5.0), "the interface: flipped");
        // The page's caret under a menu over the page (the Focus ribbon) moves with the page.
        assert_eq!(m.place_ime(SPAN, r(52.0, 53.0, 5.0)), r(92.0, 93.0, 5.0), "the input method at the page's caret");
        let editing = View { span: Some(SPAN), field: Some(r(50.0, 70.0, 4.0).expand(2.0)), field_shift: Some(-30.0), ..view(Some(canvas)) };
        assert_eq!(editing.place_ime(SPAN, r(52.0, 53.0, 5.0)), r(22.0, 23.0, 5.0), "a field being edited: with its text");
        let ctx = Context::default();
        let second = ViewportId::from_hash_of("second window");
        ctx.add_plugin(Mirror { views: HashMap::from([(second, m)]), ending: None });
        assert_eq!(page_to_window(&ctx, second, r(5.0, 9.0, 5.0)), r(45.0, 49.0, 5.0), "ui.inspect's page rectangles, as drawn");
        assert_eq!(page_to_window(&ctx, ViewportId::ROOT, r(5.0, 9.0, 5.0)), r(5.0, 9.0, 5.0), "another window isn't mirrored");
        assert_eq!(interface_to_window(&ctx, second, r(52.0, 56.0, 5.0)), r(44.0, 48.0, 5.0), "a floating comment: flipped");
        assert_eq!(interface_to_window(&ctx, ViewportId::ROOT, r(52.0, 56.0, 5.0)), r(52.0, 56.0, 5.0), "another window isn't mirrored");
    }

    /// Each window is mirrored on its own: what one window's pass learns (its size, its page)
    /// doesn't move another's pointer, even when one window's pass runs inside another's.
    #[test]
    fn each_window_is_mirrored_on_its_own() {
        let mut m = Mirror::default();
        let second = ViewportId::from_hash_of("second window");
        m.views.insert(ViewportId::ROOT, view(None));
        m.views.insert(second, View { canvas: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(60.0, 50.0))), ..view(None) });
        let ctx = Context::default();
        let moved = |viewport_id, width: f32| RawInput {
            viewport_id,
            screen_rect: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(width, 80.0))),
            events: vec![Event::PointerMoved(pos2(45.0, 10.0))],
            ..Default::default()
        };
        let mut root = moved(ViewportId::ROOT, 200.0);
        egui::Plugin::input_hook(&mut m, &ctx, &mut root);
        let mut inner = moved(second, 100.0);
        egui::Plugin::input_hook(&mut m, &ctx, &mut inner);
        assert!(matches!(root.events[0], Event::PointerMoved(p) if p == pos2(155.0, 10.0)), "the root window flips about its own centre");
        assert!(matches!(inner.events[0], Event::PointerMoved(p) if p == pos2(5.0, 10.0)), "the other window, on its page: shifted");
        assert_eq!(m.views[&ViewportId::ROOT].span, Some((0.0, 200.0)));
    }

    /// A check box's tick only moves: flipped, it would point the wrong way.
    #[test]
    fn check_marks_keep_their_direction() {
        let tick = vec![pos2(10.0, 15.0), pos2(15.0, 20.0), pos2(20.0, 10.0)];
        let mut s = Shape::line(tick, Stroke::new(1.0, Color32::BLACK));
        mirror_shape(SPAN, &mut s);
        assert!(matches!(&s, Shape::Path(p) if p.points == vec![pos2(80.0, 15.0), pos2(85.0, 20.0), pos2(90.0, 10.0)]));
        let mut chevron = Shape::line(vec![pos2(10.0, 10.0), pos2(15.0, 15.0), pos2(10.0, 20.0)], Stroke::new(1.0, Color32::BLACK));
        mirror_shape(SPAN, &mut chevron);
        assert!(matches!(&chevron, Shape::Path(p) if p.points[1] == pos2(85.0, 15.0) && p.points[0] == pos2(90.0, 10.0)), "other lines flip");
    }
}
