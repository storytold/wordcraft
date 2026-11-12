use super::{Hash, Pos2, Rect, Sense, Ui, WordApp, json};

/// One edge of a page's text area on a ruler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MarginEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl MarginEdge {
    pub(super) fn param(self) -> &'static str {
        match self {
            MarginEdge::Left => "left",
            MarginEdge::Right => "right",
            MarginEdge::Top => "top",
            MarginEdge::Bottom => "bottom",
        }
    }
    fn label(self) -> &'static str {
        match self {
            MarginEdge::Left => "Left Margin",
            MarginEdge::Right => "Right Margin",
            MarginEdge::Top => "Top Margin",
            MarginEdge::Bottom => "Bottom Margin",
        }
    }
}

/// The section `i` as the rulers show and drag it: its laid-out page, which is half the sheet
/// under 2 pages per sheet and Book fold (#1836).
pub(super) fn ruler_sect(doc: &wordcraft_doc::Document, i: usize) -> wordcraft_doc::section::SectionProps {
    doc.laid_out_sections().into_iter().nth(i).map(|(_, s)| s).unwrap_or_default()
}

/// The least text width (or height) a margin drag leaves: one inch, in points.
pub(super) const MIN_TEXT: f32 = 72.0;

/// The margin (points) after dragging `edge` of the text area on a ruler to `at` points from the
/// page's left (or top) edge: snapped to 1/16 inch, never negative, and leaving at least an inch
/// of text between it and the opposite margin.
pub(crate) fn dragged_margin(edge: MarginEdge, at: f32, s: &wordcraft_doc::section::SectionProps) -> f32 {
    let (raw, room, cur) = match edge {
        MarginEdge::Left => (at - s.gutter, s.page_w - s.margin_right - s.gutter - MIN_TEXT, s.margin_left),
        MarginEdge::Right => (s.page_w - at, s.page_w - s.margin_left - s.gutter - MIN_TEXT, s.margin_right),
        MarginEdge::Top => (at, s.page_h - s.margin_bottom - MIN_TEXT, s.margin_top),
        MarginEdge::Bottom => (s.page_h - at, s.page_h - s.margin_top - MIN_TEXT, s.margin_bottom),
    };
    if !raw.is_finite() || !room.is_finite() {
        return cur;
    }
    ((raw / 4.5).round() * 4.5).clamp(0.0, room.max(0.0))
}

/// The stored margin a drag of the visible `edge` to `at` points edits, and its new value. On an
/// even page of a mirrored layout (`mirrored`) the text area is the odd page's reflected, so the
/// visible left edge is the stored right margin and the other way round: the pointer is reflected
/// across the page and the edge swapped (#2914). Top and bottom are the same on every page.
pub(crate) fn margin_drag(edge: MarginEdge, at: f32, s: &wordcraft_doc::section::SectionProps, mirrored: bool) -> (MarginEdge, f32) {
    let (edge, at) = match edge {
        MarginEdge::Left if mirrored => (MarginEdge::Right, s.page_w - at),
        MarginEdge::Right if mirrored => (MarginEdge::Left, s.page_w - at),
        e => (e, at),
    };
    (edge, dragged_margin(edge, at, s))
}

/// A draggable margin edge on a ruler: `to_pt` turns the pointer into points from the page's
/// left (or top) edge, and `mirrored` is an even page of a mirrored layout ([`margin_drag`]). A
/// whole drag is one undo step, through `layout.margins`. The view stays put: following the caret
/// mid-drag would move the page under the pointer and the margin with it.
pub(super) fn margin_handle(
    app: &mut WordApp,
    ui: &Ui,
    edge: MarginEdge,
    zone: Rect,
    to_pt: impl Fn(Pos2) -> f32,
    sect: &wordcraft_doc::section::SectionProps,
    mirrored: bool,
) {
    let cursor =
        if matches!(edge, MarginEdge::Left | MarginEdge::Right) { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical };
    let r = ui.interact(zone, ui.scope_id().with(("ruler_margin", edge)), Sense::drag()).on_hover_cursor(cursor);
    let r = if r.dragged() { r } else { r.on_hover_text(tl!(edge.label())) };
    if r.dragged()
        && let Some(pp) = r.interact_pointer_pos()
    {
        let (stored, v) = margin_drag(edge, to_pt(pp), sect, mirrored);
        if !r.drag_started() {
            app.session.join_next_undo();
        }
        let _ = app.run_keep_view("layout.margins", json!({ stored.param(): v }));
    }
}
