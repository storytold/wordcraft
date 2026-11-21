//! Compact ink footprints painted by the framework's active page tool.

use craft::behavior::{Cursor, CursorIcon, IconId, PaintedCursor};
use craft::pages::ui::{PageBody, PlacedPage};
use craft::ui::canvas::ToolContext;
use craft::ui::primitives::{DataWidth, Draw, IconSpec, PaintShape};
use craft::ui::style::DataColor;
use egui::{Color32, Rect, vec2};
use wordcraft_doc::freeform::InkTool;
use wordcraft_engine::cmd::draw::DrawMode;

use crate::WordApp;

// A drag may continue beyond the paper. Its ink still follows the tool, but its
// footprint belongs only to visible paper, never chrome, review controls or a modal.
fn target<'a>(app: &WordApp, body: &'a PageBody<'_>, cx: &ToolContext<'_>) -> Option<&'a PlacedPage> {
    if app.modal_open() || app.canvas.table_tool.is_some() {
        return None;
    }
    let at = cx.pointer?.pos;
    if !at.is_finite() || !cx.rect.contains(at) || app.canvas.balloon_rects.iter().any(|(_, rect)| rect.contains(at)) {
        return None;
    }
    body.pages.iter().find(|page| page.rect.contains(at))
}

pub(super) fn cursor(app: &WordApp, body: &PageBody<'_>, cx: &ToolContext<'_>) -> Cursor {
    if matches!(app.session.view.draw.mode, DrawMode::Pen(_) | DrawMode::Eraser) && target(app, body, cx).is_some() {
        // ToolHost owns priority and cancellation; the footprint is painted on its
        // canvas layer below overlays. Lasso and momentary pan retain their cursors.
        Cursor::Painted(PaintedCursor::Hidden)
    } else {
        Cursor::System(CursorIcon::Crosshair)
    }
}

pub(super) fn paint(app: &WordApp, body: &PageBody<'_>, draw: &Draw<'_>, cx: &ToolContext<'_>) {
    let Some(page) = target(app, body, cx) else { return };
    let Some(pointer) = cx.pointer else { return };
    let at = pointer.pos;
    match app.session.view.draw.mode {
        DrawMode::Pen(tool) => {
            let set = app.session.view.draw.settings(tool);
            let radius = footprint_radius(set.width, page.scale as f32, tool);
            let color = super::screen_ink(crate::canvas::dark_page(app), set.color, (tool.alpha() * 255.0).round() as u8);
            let outline = if tool == InkTool::Highlighter {
                PaintShape::Rect(Rect::from_center_size(at, vec2(radius + 1.0, radius + 1.0) * 2.0), 0.0.into())
            } else {
                PaintShape::Circle(at, radius + 1.0)
            };
            for (width, color) in [(2.0, Color32::WHITE), (0.75, Color32::BLACK)] {
                draw.stroke(&outline, DataWidth::points(width), DataColor::new(color));
            }
            draw.data_fill(&PaintShape::Circle(at, radius), DataColor::new(color));
        }
        DrawMode::Eraser => {
            // This deletes whole strokes, so a disk would suggest a false erase radius.
            draw.data_fill(&PaintShape::Circle(at, 2.0), DataColor::new(Color32::WHITE));
            draw.data_fill(&PaintShape::Circle(at, 1.0), DataColor::new(Color32::BLACK));
            let icon = Rect::from_min_size(at + vec2(2.0, -18.0), vec2(20.0, 20.0));
            let ink = super::screen_ink(crate::canvas::dark_page(app), wordcraft_doc::Rgb(0, 0, 0), 255);
            draw.icon(icon, &IconSpec::svg(IconId::new("eraser")).data_color(DataColor::new(ink)));
        }
        _ => {}
    }
}

fn footprint_radius(width: f32, scale: f32, tool: InkTool) -> f32 {
    let width = if width.is_finite() { width.clamp(0.25, 72.0) } else { tool.default_width() };
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    // Imported view geometry must not create an unbounded or nonfinite cursor mesh.
    (width * scale / 2.0).clamp(0.5, 256.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_settings_and_extreme_geometry_keep_the_cursor_bounded() {
        for width in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX] {
            for scale in [f32::NAN, f32::INFINITY, -1.0, 0.0, f32::MAX] {
                let radius = footprint_radius(width, scale, InkTool::Pen);
                assert!(radius.is_finite() && (0.5..=256.0).contains(&radius));
            }
        }
    }
}
