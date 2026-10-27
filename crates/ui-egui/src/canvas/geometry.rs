use super::reading::place_read_pages;
use super::*;

/// Page positions in content space (points), for the current zoom.
pub struct Geometry {
    pub rects: Vec<Rect>,
    pub size: egui::Vec2,
    pub scale: f32,
    /// Pages per row.
    pub cols: usize,
    /// Per page, the vertical range (pixels from the page rect's top) that is on screen. The
    /// whole page, except with Hide White Space, which cuts the margins between pages.
    pub bands: Vec<(f32, f32)>,
}

/// Room kept above the first line and below the last line of a page when white space is hidden (points).
pub(crate) const WHITE_SPACE_PAD: f32 = 6.0;

/// The line drawn between two pages when white space is hidden (pixels).
pub(crate) const SEAM: f32 = 1.0;

/// Hide White Space: stack single-column pages (`rects`, full pages) so that only each page's
/// text area (`bodies`: top and bottom of the body in points, plus a little room) shows, with a
/// [`SEAM`] between pages. The first page keeps its top and the last its bottom. Rects keep the
/// full page size, so page coordinates still map as `(p - rect.min) / scale`; neighbouring rects
/// overlap in the hidden margins. Returns the moved rects, the visible band of each, and the
/// content height.
pub(super) fn collapse_white_space(rects: &[Rect], bodies: &[(f32, f32)], scale: f32, avail_h: f32) -> (Vec<Rect>, Vec<(f32, f32)>, f32) {
    let n = rects.len();
    let scale = if scale.is_finite() { scale.clamp(0.01, 10.0) } else { PX_PER_PT };
    let avail_h = if avail_h.is_finite() { avail_h } else { 0.0 };
    let mut out = Vec::with_capacity(n);
    let mut bands = Vec::with_capacity(n);
    let Some(first) = rects.first() else { return (out, bands, avail_h) };
    let mut cursor = first.min.y;
    for (i, r) in rects.iter().enumerate() {
        let h = if r.height().is_finite() { r.height().max(0.0) } else { 0.0 };
        let (bt, bb) = bodies.get(i).copied().unwrap_or((0.0, h));
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let top = if i == 0 { 0.0 } else { (fin(bt, 0.0) * scale - WHITE_SPACE_PAD * scale).clamp(0.0, h) };
        let bot = if i + 1 == n { h } else { (fin(bb, 0.0) * scale + WHITE_SPACE_PAD * scale).clamp(top, h) };
        let y = cursor - top;
        out.push(Rect::from_min_size(pos2(r.min.x, y), r.size()));
        bands.push((top, bot));
        cursor = y + bot + SEAM;
    }
    (out, bands, (cursor - SEAM + GAP).max(avail_h))
}

/// True when any page is cut to a band (white space is hidden).
pub(super) fn white_space_hidden(rects: &[Rect], bands: &[(f32, f32)]) -> bool {
    rects.iter().zip(bands).any(|(r, b)| b.0 > 0.5 || b.1 < r.height() - 0.5)
}

/// True when a double-click at `p` is on the space between two pages (or on the seam when white
/// space is hidden): Word's way to toggle Hide White Space.
pub(super) fn on_page_gap(rects: &[Rect], bands: &[(f32, f32)], p: Pos2) -> bool {
    let hidden = white_space_hidden(rects, bands);
    rects.windows(2).enumerate().any(|(i, w)| {
        let (a, b) = (w[0], w[1]);
        let in_x = p.x >= a.min.x.min(b.min.x) && p.x <= a.max.x.max(b.max.x);
        let (y0, y1) = match bands.get(i) {
            Some(&(_, bot)) if hidden => (a.min.y + bot - 3.0, a.min.y + bot + SEAM + 3.0),
            _ => (a.max.y, b.min.y),
        };
        in_x && p.y >= y0 && p.y <= y1
    })
}

/// True when `p` is on the shown part of a page, not the canvas around or between pages (nor a
/// margin Hide White Space cuts).
pub(super) fn on_visible_page(rects: &[Rect], bands: &[(f32, f32)], p: Pos2) -> bool {
    rects.iter().zip(bands).any(|(r, &(top, bot))| Rect::from_min_max(pos2(r.min.x, r.min.y + top), pos2(r.max.x, r.min.y + bot)).contains(p))
}

/// Width of the card that floats beside a page when comments show only for the selected text (points).
pub(super) const FLOAT_WIDTH: f32 = 216.0;

/// Comments show as a floating card rather than in a margin column (#1055): the option is on,
/// balloons are shown at all, and no other markup needs the column.
pub(super) fn floating_comments(app: &WordApp) -> bool {
    let s = &app.session;
    let m = &s.prefs.markup;
    s.balloons_shown() && m.comments && m.comment_display == CommentDisplay::Selected && !s.doc.comments.is_empty() && markup_width(app) <= 0.0
}

/// The comment shown when only the selected one is: the clicked balloon, else the comment whose
/// range holds the caret.
pub(super) fn shown_comment(app: &WordApp) -> Option<u32> {
    app.canvas.balloon.or_else(|| wordcraft_engine::cmd::review::comment_range_at_caret(&app.session))
}

/// Width of the markup area beside each page for comment and revision balloons (points), or 0.
pub fn markup_width(app: &WordApp) -> f32 {
    // Word shows comments either in balloons (contextual) or in the Comments pane (list); Track
    // Changes Options can put every revision inline (`Session::balloons_shown`).
    let s = &app.session;
    if !s.balloons_shown() {
        return 0.0;
    }
    let m = &s.prefs.markup;
    let comments = m.comments && !s.doc.comments.is_empty() && m.comment_display == CommentDisplay::Margin;
    let [has_formats, has_moves, has_deletions] = match app.canvas.markup_facts.get() {
        Some((rev, facts)) if rev == (s.document_id(), s.rev()) => facts,
        _ => {
            use wordcraft_engine::cmd::{moves, review};
            let facts = [review::has_format_changes(&s.doc), moves::has_moves(&s.doc), review::has_deletions(&s.doc)];
            app.canvas.markup_facts.set(Some(((s.document_id(), s.rev()), facts)));
            facts
        }
    };
    let formats = m.formatting && has_formats;
    let moves = m.insertions_deletions && has_moves;
    let deletions = s.revisions_in_balloons() && has_deletions;
    if s.balloons_shown() && (comments || formats || moves || deletions) { 216.0 } else { 0.0 }
}

/// A page dimension (points) safe to lay out: finite, at least 1 pt, at most `cap`.
pub(super) fn sane_dim(v: f32, cap: f32) -> f32 {
    if v.is_finite() { v.clamp(1.0, cap) } else { 72.0 }
}

/// How many pages fit side by side: as many slots of `slot_px` (a page plus its markup area, on
/// screen) as the width holds with a gap around each, like Word's Print Layout when zoomed out.
/// At least `min_cols`, never more than `pages` (or 1 when `auto` is off and `min_cols` is 1).
pub(super) fn columns_for(avail_w: f32, slot_px: f32, pages: usize, min_cols: usize, auto: bool) -> usize {
    let fit = if auto && avail_w.is_finite() && slot_px.is_finite() && slot_px > 0.0 {
        // Saturating float → int cast; bounded by the page count below.
        ((avail_w - GAP) / (slot_px + GAP)).floor().max(1.0) as usize
    } else {
        1
    };
    fit.max(min_cols).min(pages.max(1)).max(1)
}

/// Place pages (sizes in points) in rows of `cols`, left to right then top to bottom, each in a slot
/// as wide as the widest page plus `markup` and centred in `avail_w`. Returns the screen-space rects
/// (relative to the content's top-left) and the content size.
pub(super) fn place_pages(sizes: &[(f32, f32)], markup: f32, scale: f32, cols: usize, avail: egui::Vec2) -> (Vec<Rect>, egui::Vec2) {
    let cols = cols.max(1);
    let scale = if scale.is_finite() { scale.clamp(0.01, 10.0) } else { PX_PER_PT };
    let markup = if markup.is_finite() { markup.clamp(0.0, 10_000.0) } else { 0.0 };
    let avail = vec2(if avail.x.is_finite() { avail.x.max(0.0) } else { 0.0 }, if avail.y.is_finite() { avail.y.max(0.0) } else { 0.0 });
    let maxw = sizes.iter().map(|&(w, _)| sane_dim(w, 1e5)).fold(0.0f32, f32::max).max(72.0) + markup;
    let slot = maxw * scale;
    let row_w = cols as f32 * slot + (cols as f32 - 1.0) * GAP;
    let content_w = (row_w + 2.0 * GAP).max(avail.x);
    let mut rects = Vec::with_capacity(sizes.len());
    let mut y = GAP;
    for row in sizes.chunks(cols) {
        let mut x = (content_w - row_w) / 2.0;
        let mut h = 0.0f32;
        for &(w, ph) in row {
            let (w, ph) = (sane_dim(w, 1e5), sane_dim(ph, 1e6));
            // Pages narrower than the widest keep their markup area beside them.
            let px = x + (maxw - markup - w).max(0.0) * scale / 2.0;
            rects.push(Rect::from_min_size(pos2(px, y), vec2(w * scale, ph * scale)));
            h = h.max(ph * scale);
            x += slot + GAP;
        }
        y += h + GAP;
    }
    (rects, vec2(content_w, y.max(avail.y)))
}

pub fn geometry(app: &WordApp, l: &DocLayout, avail: egui::Vec2) -> Geometry {
    let v = &app.session.view;
    let markup = markup_width(app);
    let maxw = l.pages.iter().map(|p| sane_dim(p.w, 1e5)).fold(0.0f32, f32::max).max(72.0) + markup;
    let maxh = l.pages.iter().map(|p| sane_dim(p.h, 20_000.0)).fold(0.0f32, f32::max).max(72.0);
    let web = v.mode != wordcraft_layout::ViewMode::Print;
    let mut scale = v.zoom.clamp(0.1, 5.0) * PX_PER_PT;
    if v.read_mode {
        scale = v.read_zoom.clamp(0.75, 1.5) * PX_PER_PT;
        let sizes: Vec<(f32, f32)> = l.pages.iter().map(|p| (p.w, p.h)).collect();
        let (rects, size) = place_read_pages(&sizes, scale, avail);
        // Reading columns show whole pages (Hide White Space doesn't apply).
        let bands = rects.iter().map(|r| (0.0, r.height())).collect();
        return Geometry { rects, size, scale, cols: sizes.len().max(1), bands };
    }
    match v.fit.as_str() {
        "pageWidth" => scale = ((avail.x - 2.0 * GAP - 20.0) / maxw).clamp(0.1, 6.0),
        "onePage" => scale = ((avail.y - 2.0 * GAP) / maxh).min((avail.x - 2.0 * GAP) / maxw).clamp(0.05, 6.0),
        "multiplePages" => scale = ((avail.y - 2.0 * GAP) / maxh).min((avail.x - 3.0 * GAP) / (2.0 * maxw)).clamp(0.05, 6.0),
        _ => {}
    }
    if web {
        scale = PX_PER_PT * v.zoom.clamp(0.1, 5.0);
    }
    if !scale.is_finite() {
        scale = PX_PER_PT;
    }
    // Pages flow side by side in Print Layout whenever more than one fits across; One Page and
    // Page Width show one per row (and so does a zoom after One Page, #768), Web Layout has no pages. Reading Mode is handled above.
    let (min_cols, auto) = if web || v.fit == "onePage" || v.fit == "pageWidth" || v.single_column {
        (1, false)
    } else if v.multi_page {
        (2, true)
    } else {
        (1, true)
    };
    let cols = columns_for(avail.x, maxw * scale, l.pages.len(), min_cols, auto);
    let sizes: Vec<(f32, f32)> = l.pages.iter().map(|p| (p.w, p.h)).collect();
    if v.mode == wordcraft_layout::ViewMode::Web {
        // The sheet runs from side to side of the work area, no centred page with grey margins.
        let rects: Vec<Rect> = sizes
            .iter()
            .scan(0.0f32, |y, &(w, h)| {
                let r = Rect::from_min_size(pos2(0.0, *y), vec2(sane_dim(w, 1e5) * scale, sane_dim(h, 1e6) * scale));
                *y = r.bottom();
                Some(r)
            })
            .collect();
        let size = vec2(rects.iter().map(|r| r.right()).fold(avail.x.max(0.0), f32::max), rects.last().map_or(0.0, |r| r.bottom()).max(avail.y));
        let bands = rects.iter().map(|r| (0.0, r.height())).collect();
        return Geometry { rects, size, scale, cols: 1, bands };
    }
    let (mut rects, mut size) = place_pages(&sizes, markup, scale, cols, avail);
    let mut bands: Vec<(f32, f32)> = rects.iter().map(|r| (0.0, r.height())).collect();
    let editing_part = matches!(app.session.sel.focus.story, StoryRef::Part(_));
    if v.hide_white_space && v.mode == wordcraft_layout::ViewMode::Print && !v.read_mode && cols == 1 && !editing_part && rects.len() > 1 {
        let bodies: Vec<(f32, f32)> = l.pages.iter().map(|p| (p.body.y, p.body.bottom())).collect();
        let (r, b, h) = collapse_white_space(&rects, &bodies, scale, avail.y);
        (rects, bands) = (r, b);
        size.y = h;
    }
    Geometry { rects, size, scale, cols, bands }
}

/// While a fit mode (Page Width, One Page, Multiple Pages) sizes the page, keep the session's zoom
/// equal to what is shown, as Word does. Zoom In/Out, the Zoom dialog and `view.state` then start
/// from the visible zoom; before, they stepped from the stale manual zoom, so Zoom In from a 163%
/// Page Width jumped to 110% (issue #67).
pub fn sync_fit_zoom(app: &mut WordApp, scale: f32) {
    let v = &mut app.session.view;
    if v.fit.is_empty() || v.read_mode || v.mode != wordcraft_layout::ViewMode::Print || !scale.is_finite() {
        return;
    }
    v.zoom = (scale / PX_PER_PT).clamp(0.1, 5.0);
}
