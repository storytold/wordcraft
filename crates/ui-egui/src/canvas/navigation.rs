use craft::behavior::kurbo::Point;
use craft::pages::layout::PagedLayout;
use serde_json::{Value, json};
use wordcraft_doc::StoryRef;
use wordcraft_layout::{DocLayout, ViewMode};

use super::pages::page_screen_scale;
use super::{PX_PER_PT, Pos2, Rect, WordApp, pos2};

/// The page being looked at (#3880): the page of `caret` (page, top, bottom in content
/// coordinates) while any of it is inside the window (`top` .. `top + height`, `top` the layout's y at the window's
/// top edge), else the page at the window's top: the first whose shown band ends below it (with Hide White Space the
/// rects overlap in the hidden margins, and the next page's text already shows there), the last page past the end.
pub(super) fn viewed_page(caret: Option<(usize, f64, f64)>, top: f64, layout: &PagedLayout, height: f64) -> usize {
    match caret {
        Some((page, from, to)) if to >= top && from <= top + height => page,
        _ => layout.rects.iter().zip(&layout.bands).position(|(r, band)| r.y0 + band.1 > top).unwrap_or(layout.rects.len().saturating_sub(1)),
    }
}

/// Convert a page position (points) to screen coordinates (for agents and tests): the page must be on screen.
pub fn page_to_screen(app: &mut WordApp, page: usize, x: f32, y: f32) -> Option<Pos2> {
    let placed = app.canvas.shown.pages.iter().find(|p| p.page == page)?;
    Some(placed.to_screen(Point::new(f64::from(x), f64::from(y))))
}

/// The page whose screen rect is under (or nearest to) a screen point.
fn nearest_page(rects: &[Rect], p: Pos2) -> Option<usize> {
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

/// Page index and document coordinates under a screen point.
fn page_at(rects: &[Rect], layout: &DocLayout, fallback_scale: f32, p: Pos2) -> Option<(usize, f32, f32)> {
    let i = nearest_page(rects, p)?;
    let r = rects.get(i)?;
    let scale = layout.pages.get(i).map_or(fallback_scale, |page| page_screen_scale(*r, page, fallback_scale));
    Some((i, (p.x - r.min.x) / scale, (p.y - r.min.y) / scale))
}

/// How many pages sit side by side in the first row of `rects`.
fn page_cols(rects: &[Rect]) -> usize {
    let Some(first) = rects.first() else { return 1 };
    rects.iter().take_while(|r| (r.min.y - first.min.y).abs() < 1.0).count().max(1)
}

/// Page Up (`caret.pageUp`) and Page Down in Print Layout move one screen, as in Word (#5680):
/// the caret goes to the place a window's height above or below it, and the view scrolls by as
/// much, so the caret keeps its place in the window. One Page shows a page a screen and Multiple
/// Pages a row of pages, so there they go to the next page or row. A caret scrolled out of view
/// moves the same way and the view follows it, as after any caret move. `None` leaves the command
/// to the engine (the same place on the page before or after): another view, a caret outside the
/// body, or no window to measure yet. Scripts, the CLI and agents run the engine's command
/// ([`WordApp::execute`]).
///
/// A recorded macro keeps the relative `caret.pageDown`/`caret.pageUp` step rather than the
/// resolved `caret.set` coordinates.
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
    let extend = params.get("extend").cloned().unwrap_or(Value::Bool(false));
    let set = json!({"page": step.page, "x": step.x, "y": step.y, "extend": extend});
    let recording = app.session.recording.take();
    let r = if !step.shown { app.run("caret.set", set) } else { app.run_keep_view("caret.set", set) };
    app.session.recording = recording;
    if r.is_ok() {
        if let Some((_, steps)) = app.session.recording.as_mut() {
            steps.push((id.to_string(), json!({"extend": extend})));
        }
        if step.shown {
            app.canvas.screen_scroll += f64::from(step.scroll);
        }
    }
    Some(r)
}

/// Where a Page Up or Page Down by one screen takes the caret ([`page_by_screen`]).
struct ScreenStep {
    page: usize,
    x: f32,
    y: f32,
    /// Vertical scroll to apply (screen points): the row-to-row pitch when whole pages fit in the
    /// window so repeated presses do not drift past page tops, else one viewport height.
    scroll: f32,
    /// Whether the caret was in the window, across and down.
    shown: bool,
}

/// How many complete rows of laid-out pages fit inside `window_h` (at least 1).
fn rows_per_screen(laid: &[Rect], from_page: usize, cols: usize, window_h: f32, dir: f32) -> usize {
    let row_start = (from_page / cols) * cols;
    let mut rows = 1usize;
    if dir > 0.0
        && let Some(first) = laid.get(row_start)
    {
        while let Some(next) = laid.get(row_start + rows * cols) {
            if next.max.y - first.min.y <= window_h {
                rows += 1;
            } else {
                break;
            }
        }
        return rows;
    }
    if dir < 0.0
        && let Some(last) = laid.get(row_start)
    {
        while let Some(prev_idx) = row_start.checked_sub(rows * cols)
            && let Some(prev) = laid.get(prev_idx)
        {
            if last.max.y - prev.min.y <= window_h {
                rows += 1;
            } else {
                break;
            }
        }
    }
    rows
}

fn screen_step(app: &mut WordApp, dir: f32) -> Option<ScreenStep> {
    let window = app.canvas.view_rect().filter(|r| r.is_finite() && r.height() > 0.0)?;
    let layout = app.session.layout();
    let fallback_scale = app.canvas.zoom() as f32 * PX_PER_PT;
    let c = layout.caret_on(&app.session.sel.focus, app.session.page_hint)?;
    let laid = &app.canvas.shown.laid;
    let from = laid.get(c.page).copied()?;
    let from_page = layout.pages.get(c.page)?;
    let scale = page_screen_scale(from, from_page, fallback_scale);
    let at = pos2(from.min.x + c.x * scale, from.min.y + (c.top + c.height / 2.0) * scale);
    if !at.is_finite() {
        return None;
    }
    let pending_y = app.canvas.screen_scroll as f32;
    // A caret beside the window (scrolled sideways past it) is out of view too.
    let shown = window.contains(pos2(at.x, at.y - pending_y));
    let cols = page_cols(laid);

    // When a whole page fits in the window (One Page, Full Screen fit, Multiple Pages), advance
    // by whole page rows and scroll by the exact row-to-row pitch so repeated presses do not
    // accumulate (window.height() - pitch) drift past page tops.
    if from.height() <= window.height() {
        let step_pages = rows_per_screen(laid, c.page, cols, window.height(), dir).saturating_mul(cols);
        let to = if dir > 0.0 { c.page.checked_add(step_pages) } else { c.page.checked_sub(step_pages) }?;
        let target_rect = laid.get(to)?;
        let y = c.top + c.height / 2.0;
        let (page, x, y) = past_the_text(&layout, to, c.x, y, dir, cols);
        if !x.is_finite() || !y.is_finite() || layout.hit(page, x, y, StoryRef::Body).is_none() {
            return None;
        }
        let dest_rect = laid.get(page).unwrap_or(target_rect);
        let scroll = dest_rect.min.y - from.min.y;
        return Some(ScreenStep { page, x, y, scroll, shown });
    }

    // `at` is already in `laid` coordinates (not shifted by `pending_y`), so advance directly by
    // one viewport height; `pending_y` applies only to the visibility check above.
    let target = pos2(at.x, at.y + dir * window.height());
    let (page, x, y) = page_at(laid, &layout, fallback_scale, target)?;
    let (page, x, y) = past_the_text(&layout, page, x, y, dir, cols);
    if !x.is_finite() || !y.is_finite() || layout.hit(page, x, y, StoryRef::Body).is_none() {
        return None;
    }
    Some(ScreenStep { page, x, y, scroll: dir * window.height(), shown })
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
