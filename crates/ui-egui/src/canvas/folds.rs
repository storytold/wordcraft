use super::pages::to_screen;
use super::{Color32, DocLayout, Rect, StoryRef, Ui, WordApp, page_at, page_screen_scale, pos2, vec2};

/// Whether the view folds collapsed headings (#1745): every view but Outline.
pub(super) fn folds_headings(app: &WordApp) -> bool {
    app.session.view.mode != wordcraft_layout::ViewMode::Outline || app.session.view.read_mode
}

/// The fold arrows beside headings (#1745), in the left margin: always for a collapsed heading
/// (pointing along the line), and for an expanded one while the pointer is over its line or the
/// arrow (pointing down). Drawn on screen only, never printed.
pub(super) fn paint_fold_arrows(app: &WordApp, ui: &Ui, layout: &DocLayout, rects: &[Rect], scale: f32, view: Rect) {
    if !folds_headings(app) || app.session.sel.focus.story != StoryRef::Body {
        return;
    }
    let hover = ui.input(|i| i.pointer.hover_pos()).and_then(|p| page_at(rects, layout, scale, p));
    // Clipped to the canvas viewport: not over the scroll bar, nor a neighbouring column's margin.
    let painter = &ui.painter_at(view);
    for (i, r) in rects.iter().enumerate() {
        if !view.intersects(*r) {
            continue;
        }
        let Some(page) = layout.pages.get(i) else { continue };
        let k = page_screen_scale(*r, page, scale);
        for a in layout.fold_arrows(i) {
            // Over the heading's first line or the arrow (not the same height in another column).
            let (x0, x1) = (a.line.0.min(a.rect.x), a.line.1.max(a.rect.x + a.rect.w));
            let over = hover.is_some_and(|(hp, x, y)| hp == i && y >= a.rect.y && y <= a.rect.y + a.rect.h && x >= x0 && x <= x1);
            if !a.collapsed && !over {
                continue;
            }
            let fold = match (a.collapsed, a.rtl) {
                (false, _) => crate::icons::Fold::Open,
                (true, false) => crate::icons::Fold::ClosedLtr,
                (true, true) => crate::icons::Fold::ClosedRtl,
            };
            // Centred on the first line, at least 10 screen points across.
            let centre = to_screen(pos2(0.0, 0.0), *r, k, a.rect.x + a.rect.w / 2.0, a.rect.y + a.rect.h / 2.0);
            let side = (a.rect.w * k).max(FOLD_ARROW_MIN_PX);
            // The page is paper (white in both themes): greys that read on it.
            let c = if over { Color32::from_gray(0x50) } else { Color32::from_gray(0x8C) };
            crate::icons::fold_arrow(painter, Rect::from_center_size(centre, vec2(side, side)), fold, c, 1.6);
        }
    }
}

/// The smallest fold arrow drawn on screen (points); [`paint_fold_arrows`] and its hit test share it.
pub(super) const FOLD_ARROW_MIN_PX: f32 = 10.0;

/// [`FOLD_ARROW_MIN_PX`] in page points on `page`, for [`DocLayout::fold_arrow_at`].
pub(super) fn fold_arrow_min_side(rects: &[Rect], layout: &DocLayout, page: usize, scale: f32) -> f32 {
    let k = match (rects.get(page), layout.pages.get(page)) {
        (Some(r), Some(p)) => page_screen_scale(*r, p, scale),
        _ => scale,
    };
    if k > 0.0 { FOLD_ARROW_MIN_PX / k } else { 0.0 }
}
