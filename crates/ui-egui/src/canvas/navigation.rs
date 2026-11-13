use serde_json::{Value, json};
use wordcraft_layout::ViewMode;

use super::pages::{page_screen_scale, to_screen};
use super::{DocLayout, Pos2, Rect, StoryRef, Ui, WordApp, pos2, vec2};

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

/// Page Up (`caret.pageUp`) and Page Down in Print Layout move one screen, as in Word (#5680):
/// the caret goes to the place a window's height above or below it, and the view scrolls by as
/// much, so the caret keeps its place in the window. One Page shows a page a screen and Multiple
/// Pages a row of pages, so there they go to the next page or row. A caret scrolled out of view
/// moves the same way and the view follows it, as after any caret move. `None` leaves the command
/// to the engine (the same place on the page before or after): another view, a caret outside the
/// body, or no window to measure yet. Scripts, the CLI and agents run the engine's command
/// ([`WordApp::execute`]).
pub(crate) fn page_by_screen(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let dir = match id {
        "caret.pageDown" => 1.0,
        "caret.pageUp" => -1.0,
        _ => return None,
    };
    let view = &app.session.view;
    if view.read_mode || view.mode != ViewMode::Print || app.session.sel.focus.story != StoryRef::Body {
        return None;
    }
    let step = screen_step(app, dir)?;
    let set = json!({"page": step.page, "x": step.x, "y": step.y, "extend": params.get("extend").cloned().unwrap_or(Value::Bool(false))});
    if !step.shown {
        return Some(app.run("caret.set", set));
    }
    let r = app.run_keep_view("caret.set", set);
    if r.is_ok() {
        let to = (step.from.y + dir * step.window.height()).min(app.canvas.scroll_max.y).max(0.0);
        app.canvas.scroll_request = Some(vec2(step.from.x, to));
        // Past the end of the document the view stops short, and the caret's new place may be
        // beside the window: still show the caret.
        let shift = vec2(step.from.x, to) - app.canvas.scroll_offset;
        app.canvas.scroll_to_caret = caret_middle(app).is_none_or(|p| !step.window.contains(p - shift));
    }
    Some(r)
}

/// Where a Page Up or Page Down by one screen takes the caret ([`page_by_screen`]).
struct ScreenStep {
    page: usize,
    x: f32,
    y: f32,
    /// The canvas's visible rect last frame; its height is the step.
    window: Rect,
    /// The scroll offset the step starts from.
    from: egui::Vec2,
    /// Whether the caret was in the window, across and down.
    shown: bool,
}

fn screen_step(app: &mut WordApp, dir: f32) -> Option<ScreenStep> {
    // One screen, as a click in the scroll bar's track pages (#5853).
    let window = app.canvas.viewport.filter(|r| r.is_finite() && r.height() > 0.0)?;
    let at = caret_middle(app)?;
    // The pages are where last frame drew them; a scroll asked for since (another press in the
    // same frame) moves them by the difference.
    let now = app.canvas.scroll_offset;
    let from = app.canvas.scroll_request.unwrap_or(now);
    if !from.is_finite() || !now.is_finite() {
        return None;
    }
    let layout = app.session.layout();
    let target = pos2(at.x, at.y + dir * window.height());
    let (page, x, y) = page_at(&app.canvas.page_rects, &layout, app.canvas.scale, target)?;
    let (page, x, y) = past_the_text(&layout, page, x, y, dir, app.canvas.cols);
    if !x.is_finite() || !y.is_finite() || layout.hit(page, x, y, StoryRef::Body).is_none() {
        return None;
    }
    // A caret beside the window (scrolled sideways past it) is out of view too.
    let shown = window.contains(at - (from - now));
    Some(ScreenStep { page, x, y, window, from, shown })
}

/// The middle of the caret's line on screen, where last frame drew the pages.
fn caret_middle(app: &mut WordApp) -> Option<Pos2> {
    let layout = app.session.layout();
    let c = layout.caret_on(&app.session.sel.focus, app.session.page_hint)?;
    page_to_screen(app, c.page, c.x, c.top + c.height / 2.0).filter(|p| p.is_finite())
}

/// A screen step that lands past the last line of a page's text (Page Down) or before its first
/// (Page Up), in a margin, the gap between pages or below a short page's text, goes on to the
/// first line of the page below (the last line of the one above) rather than back to a line the
/// caret may already be on. With `cols` pages to a row (Multiple Pages) that page is `cols` on, so
/// the caret keeps its column, as in Word. Otherwise (`page`, `x`, `y`) as it is.
fn past_the_text(layout: &DocLayout, page: usize, x: f32, y: f32, dir: f32, cols: usize) -> (usize, f32, f32) {
    let line = |p: usize, y: f32| layout.hit(p, x, y, StoryRef::Body).and_then(|at| layout.caret_on(&at, p));
    let height = |p: usize| layout.pages.get(p).map(|p| p.h);
    let cols = cols.max(1);
    let (beyond, edge) = if dir > 0.0 {
        let past = height(page).and_then(|h| line(page, h)).is_some_and(|last| y > last.top + last.height);
        (past.then_some(page.saturating_add(cols)), Some(0.0))
    } else {
        let before = line(page, 0.0).is_some_and(|first| y < first.top);
        let prev = if before { page.checked_sub(cols) } else { None };
        (prev, prev.and_then(height))
    };
    match (beyond, edge) {
        (Some(to), Some(edge)) if line(to, edge).is_some() => (to, x, edge),
        _ => (page, x, y),
    }
}
