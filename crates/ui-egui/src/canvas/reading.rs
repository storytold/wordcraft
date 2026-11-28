use super::{DocLayout, PX_PER_PT, Placed, Pos, StoryRef};

/// How long a window size must hold still before a layout size follows it (seconds).
pub(super) const RESIZE_SETTLE: f64 = 0.16;

/// A layout size that follows the window: the last viewport seen and when it last changed, so a window drag reflows
/// once it settles, not per frame.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Settle {
    viewport: Option<egui::Vec2>,
    resized_at: f64,
}

impl Settle {
    /// Notes this frame's viewport `size` at time `now`. Returns whether a layout size that `changed` should be
    /// applied now: at once when entering or `forced`, otherwise only after the viewport has held still.
    pub(super) fn apply(&mut self, size: egui::Vec2, now: f64, changed: bool, forced: bool) -> bool {
        let entering = self.viewport.is_none();
        if self.viewport.is_none_or(|v| (v - size).length_sq() > 0.5) {
            self.viewport = Some(size);
            self.resized_at = now;
        }
        changed && (entering || forced || now - self.resized_at >= RESIZE_SETTLE)
    }

    /// The view was left: the next visit applies at once.
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
