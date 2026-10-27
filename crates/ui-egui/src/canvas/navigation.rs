use super::pages::{page_screen_scale, to_screen};
use super::*;

/// Page index and document coordinates under a screen point.
pub fn page_at(rects: &[Rect], layout: &DocLayout, scale: f32, p: Pos2) -> Option<(usize, f32, f32)> {
    let i = nearest_page(rects, p)?;
    let r = rects.get(i)?;
    let scale = layout.pages.get(i).map_or(scale, |page| page_screen_scale(*r, page, scale));
    Some((i, (p.x - r.min.x) / scale, (p.y - r.min.y) / scale))
}

/// The page whose screen rect is under (or nearest to) a screen point.
pub fn nearest_page(rects: &[Rect], p: Pos2) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, r) in rects.iter().enumerate() {
        let d = if r.contains(p) {
            // Pages overlap in the margins Hide White Space cuts: the one whose middle is nearer wins.
            1e-3 * (p.y - r.center().y).abs() / r.height().max(1.0)
        } else {
            let dx = (r.min.x - p.x).max(p.x - r.max.x).max(0.0);
            let dy = (r.min.y - p.y).max(p.y - r.max.y).max(0.0);
            dx + dy * 2.0
        };
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// While the vertical scrollbar of the canvas scroll area `scroll` is dragged, a tip beside the
/// pointer says which page is at the top of the window, as the status bar says it ("Page 15 of
/// 42"): dragging through a long document, you see where you'll land (#2551). `rects` are the
/// pages in content coordinates, `bands` the part of each on screen ([`Geometry::bands`]).
pub(super) fn scrollbar_page_tip(app: &mut WordApp, ui: &Ui, scroll: egui::Id, rects: &[Rect], bands: &[(f32, f32)], pages: usize) {
    // egui's scroll bars sense under the scroll area's id with the axis (1: vertical).
    let dragged = ui.ctx().is_being_dragged(scroll.with(1usize));
    app.canvas.scrollbar_page =
        (dragged && !rects.is_empty() && pages > 0).then(|| top_page(rects, bands, app.canvas.scroll_offset.y).min(pages - 1));
    let (Some(page), Some(at)) = (app.canvas.scrollbar_page, ui.ctx().pointer_latest_pos()) else { return };
    let text = crate::i18n::fmt(tl!("Page {page} of {pages}"), &[("page", &(page + 1).to_string()), ("pages", &pages.to_string())]);
    egui::Area::new(egui::Id::unique("scrollbar_page_tip"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .pivot(egui::Align2::RIGHT_CENTER)
        .fixed_pos(at - vec2(16.0, 0.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                // Unwrapped: the area keeps last frame's width, too narrow once the number grows (#3341).
                ui.add(egui::Label::new(text).extend());
            });
        });
}

/// The page shown at height `y` (content coordinates), or the first one below it: the page at
/// the top of a window scrolled to `y`. The last page past the end. A page ends where its band
/// (`bands`, the part on screen) does: with Hide White Space the rects overlap in the hidden
/// margins, and the next page's text already shows there.
pub(super) fn top_page(rects: &[Rect], bands: &[(f32, f32)], y: f32) -> usize {
    let bottom = |i: usize, r: &Rect| r.min.y + bands.get(i).map_or(r.height(), |b| b.1);
    rects.iter().enumerate().position(|(i, r)| bottom(i, r) > y).unwrap_or(rects.len().saturating_sub(1))
}

/// The page being looked at (#3880): the page of `caret` (page, top, bottom in content
/// coordinates) while any of it is inside the window (`top` .. `top + height`), else the page at
/// the window's top ([`top_page`]).
pub(super) fn viewed_page(caret: Option<(usize, f32, f32)>, rects: &[Rect], bands: &[(f32, f32)], top: f32, height: f32) -> usize {
    match caret {
        Some((page, from, to)) if to >= top && from <= top + height => page,
        _ => top_page(rects, bands, top),
    }
}

/// Convert a page position (points) to screen coordinates (for agents and tests).
pub fn page_to_screen(app: &mut WordApp, page: usize, x: f32, y: f32) -> Option<Pos2> {
    let r = app.canvas.page_rects.get(page)?;
    let layout = app.session.layout();
    let scale = layout.pages.get(page).map_or(app.canvas.scale, |page_data| page_screen_scale(*r, page_data, app.canvas.scale));
    Some(to_screen(pos2(0.0, 0.0), *r, scale, x, y))
}
