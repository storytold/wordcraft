use super::geometry::sane_dim;
use super::*;

/// Full pagination is expensive. During a resize keep the already laid-out pages and reflow
/// once the viewport has been stable this long (seconds).
pub(super) const RESIZE_SETTLE: f64 = 0.16;

/// A layout size that follows the window (Read Mode's columns, Web Layout's width): the last
/// viewport seen and when it last changed, so a window drag reflows once it settles, not per frame.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Settle {
    viewport: Option<egui::Vec2>,
    resized_at: f64,
}

impl Settle {
    /// Whether nothing has been seen since the view was entered (or [`Settle::leave`]).
    pub(super) fn entering(&self) -> bool {
        self.viewport.is_none()
    }

    /// Notes this frame's viewport `size` at time `now`. Returns whether a layout size that
    /// `changed` should be applied now: at once when entering or `forced`, otherwise only after
    /// the viewport has held still for [`RESIZE_SETTLE`].
    pub(super) fn apply(&mut self, size: egui::Vec2, now: f64, changed: bool, forced: bool) -> bool {
        let entering = self.entering();
        if self.viewport.is_none_or(|v| (v - size).length_sq() > 0.5) {
            self.viewport = Some(size);
            self.resized_at = now;
        }
        changed && (entering || forced || now - self.resized_at >= RESIZE_SETTLE)
    }

    pub(super) fn leave(&mut self) {
        self.viewport = None;
    }
}

/// Web Layout's text width in points for a work area `avail_px` wide at `scale`: the whole width,
/// clamped to what the layout accepts so a tiny or huge window never gives zero or a giant page.
pub(super) fn web_layout_width(avail_px: f32, scale: f32) -> f32 {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { PX_PER_PT };
    let w = avail_px / scale;
    if w.is_finite() { w.clamp(144.0, 20_000.0) } else { 800.0 }
}

/// Reading columns use readable text at the user's chosen zoom. The window holds roughly 3–5
/// narrow, full-height columns, and a resize reflows the text, not just the paper scale.
pub(super) fn reading_size(avail: egui::Vec2, scale: f32, requested: Option<f32>) -> (f32, f32) {
    let width = if avail.x.is_finite() { avail.x.max(0.0) } else { 0.0 };
    let height = if avail.y.is_finite() { avail.y.max(0.0) } else { 0.0 };
    let px = if let Some(pt) = requested.filter(|v| v.is_finite()) {
        // Keep a selected full-page column usable on a narrow window, not clipped offscreen.
        (pt.clamp(144.0, 612.0) * scale).min((width - 2.0 * GAP).max(144.0))
    } else {
        let count = (width / 295.0).round().clamp(1.0, 5.0);
        ((width - (count + 1.0) * GAP) / count).max(192.0)
    };
    // The columns end above the page-turn bar, which used to cover their last lines (#1772).
    (px / scale, (height - GAP - READ_NAV_BAR).max(192.0) / scale)
}

/// Height kept free under Read Mode's columns for the page-turn bar at the bottom right.
pub(super) const READ_NAV_BAR: f32 = 44.0;

/// The part of the canvas Read Mode scrolls in: as many whole columns as fit, centred. A chosen
/// column width (or a resize) otherwise left the last column cut off at the right edge (#1772).
/// `scale` is pixels per point: the column width is only re-laid out when it moves by more than
/// half a point, so each column may overrun by that much before one is dropped.
pub(super) fn reading_viewport(full: Rect, column_px: f32, scale: f32) -> Rect {
    if !column_px.is_finite() || column_px <= 0.0 || !full.width().is_finite() {
        return full;
    }
    let step = column_px + GAP;
    let slack = if scale.is_finite() { (0.5 * scale).clamp(0.0, 4.0) } else { 0.0 };
    // A hair of tolerance too: columns sized to fill the canvas exactly come back from points
    // as 2.9999998 of them, and flooring that dropped a whole column.
    let whole = ((full.width() - GAP) / (step - slack) + 1e-3).floor().max(1.0);
    let width = (whole * step + GAP).min(full.width());
    Rect::from_min_size(pos2(full.center().x - width / 2.0, full.min.y), vec2(width, full.height()))
}

/// A single horizontal row of reflowed reading pages; no wrapping at the viewport edge.
pub(super) fn place_read_pages(sizes: &[(f32, f32)], scale: f32, avail: egui::Vec2) -> (Vec<Rect>, egui::Vec2) {
    let mut x = GAP;
    let mut rects = Vec::with_capacity(sizes.len());
    for &(w, h) in sizes {
        let w = sane_dim(w, 20_000.0) * scale;
        let h = sane_dim(h, 20_000.0) * scale;
        rects.push(Rect::from_min_size(pos2(x, GAP), vec2(w, h)));
        x += w + GAP;
    }
    // Extra trailing canvas allows even the last column to sit directly beside the nav pane.
    let end = rects.last().map_or(avail.x, |p| p.left() + avail.x - GAP);
    (rects, vec2(x.max(end).max(avail.x), avail.y.max(0.0)))
}

/// Text at the start of a reading page. Unlike the editing caret, this records what the reader
/// can currently see, even when the caret is elsewhere in the document.
pub(super) fn reading_page_start(layout: &DocLayout, page: usize) -> Option<Pos> {
    let p = layout.pages.get(page)?;
    p.items
        .iter()
        .find_map(|item| match item {
            Placed::Lines { story: StoryRef::Body, path, para, l0, .. } => Some(Pos::new(StoryRef::Body, path.clone(), para.lines.get(*l0)?.start)),
            _ => None,
        })
        .or_else(|| Some(Pos::body(p.first_block, 0)))
}

/// Resolve a reading anchor after page breaks change. Preserves the first visible text instead
/// of jumping to the editing caret (which may be thousands of columns away).
pub(super) fn reading_anchor_page(layout: &DocLayout, anchor: &Pos) -> Option<usize> {
    layout.caret_on(anchor, 0).map(|c| c.page)
}

/// Turn one reading column left/right. Multiple ticks during an animation queue additional
/// columns, rather than rounding the partially animated position and losing a tick.
pub(crate) fn turn_read_page(app: &mut WordApp, direction: f32) {
    let step = app.session.view.read_width * app.canvas.scale + GAP;
    if step.is_finite() && step > 0.0 {
        let current = app.canvas.read_target_page.unwrap_or_else(|| (app.canvas.scroll_offset.x / step).round().max(0.0) as usize);
        let next = if direction < 0.0 { current.saturating_sub(1) } else { current.saturating_add(1) };
        app.canvas.read_target_page = Some(next.min(app.canvas.cols.saturating_sub(1)));
    }
}

/// Ease toward an exact column boundary; a late frame never overshoots. The final offset
/// is snapped exactly, so stopping leaves no partially visible column.
pub(super) fn reading_animation_step(current: f32, target: f32, dt: f32) -> f32 {
    if !current.is_finite() || !target.is_finite() {
        return target.max(0.0);
    }
    let t = 1.0 - (-18.0 * dt.clamp(0.0, 0.1)).exp();
    let next = current + (target - current) * t;
    if (target - next).abs() <= 0.5 { target } else { next }
}

/// One mechanical wheel notch turns one column; precise touchpads accumulate to a turn.
/// Do not feed reading input into the editor's animated pixel scroller.
pub(super) fn read_wheel_turns(events: &[egui::Event], hovered: bool, accum: &mut f32) -> i32 {
    if !hovered {
        return 0;
    }
    let mut turns = 0i32;
    for event in events {
        if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event {
            if modifiers.command || modifiers.ctrl || !delta.is_finite() {
                continue;
            }
            let d = delta.x + delta.y;
            match unit {
                egui::MouseWheelUnit::Line => {
                    // Browsers typically emit three lines for one physical notch.
                    let n = if cfg!(target_arch = "wasm32") { (d / 3.0).round() } else { d.round() };
                    turns = turns.saturating_sub(n.clamp(-5.0, 5.0) as i32);
                    *accum = 0.0;
                }
                egui::MouseWheelUnit::Page => {
                    turns -= d.signum() as i32;
                    *accum = 0.0;
                }
                egui::MouseWheelUnit::Point => {
                    *accum = (*accum + d).clamp(-300.0, 300.0);
                    if accum.abs() >= 55.0 {
                        turns -= accum.signum() as i32;
                        // Discard excess pixels: a large wheel tick must not turn twice on
                        // the next event, while small touchpad deltas still add up to a turn.
                        *accum = 0.0;
                    }
                }
            }
        }
    }
    turns.clamp(-5, 5)
}
