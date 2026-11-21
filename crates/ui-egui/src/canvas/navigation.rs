use craft::behavior::kurbo::Point;
use craft::pages::layout::PagedLayout;

use super::{Pos2, WordApp};

/// The page being looked at (#3880): the page of `caret` (page, top, bottom in content
/// coordinates) while any of it is inside the window (`top` .. `top + height`, `top` the layout's y at the window's
/// top edge), else the page at the window's top: the first whose shown band ends below it (with Hide White Space the
/// rects overlap in the hidden margins, and the next page's text already shows there), the last page past the end.
/// A page whose shown part at the top is no more than the gap to the page below it (the next row down) does not count: Go To leaves the
/// pages' margin above a page, which is more than that gap, so a sliver of the page above still shows (#7149).
pub(super) fn viewed_page(caret: Option<(usize, f64, f64)>, top: f64, layout: &PagedLayout, height: f64) -> usize {
    match caret {
        Some((page, from, to)) if to >= top && from <= top + height => page,
        _ => (0..layout.rects.len())
            .position(|i| {
                let end = layout.rects[i].y0 + layout.bands.get(i).map_or(0.0, |band| band.1);
                // The gap to the first later page whose shown part starts at or below this one's end: the page below,
                // whatever the column (an incomplete last row, a Two Up cover) and with Hide White Space (a seam).
                let below = |j: &usize| layout.rects.get(*j).is_some_and(|r| r.y0 + layout.bands.get(*j).map_or(0.0, |b| b.0) >= end);
                let gap = ((i + 1)..layout.rects.len()).find(below).and_then(|j| layout.rects.get(j)).map_or(0.0, |next| (next.y0 - end).max(0.0));
                end > top + gap
            })
            .unwrap_or(layout.rects.len().saturating_sub(1)),
    }
}

/// Convert a page position (points) to screen coordinates (for agents and tests): the page must be on screen.
pub fn page_to_screen(app: &mut WordApp, page: usize, x: f32, y: f32) -> Option<Pos2> {
    let placed = app.canvas.shown.pages.iter().find(|p| p.page == page)?;
    Some(placed.to_screen(Point::new(f64::from(x), f64::from(y))))
}
