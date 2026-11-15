//! Drawing-tool pointers, painted from our own icons so colour and width update on every
//! platform, including the web. The footprint is centred on the actual drawing hotspot.

use egui::{Color32, CursorIcon, Id, LayerId, Order, Painter, Pos2, Rect, Response, Stroke, StrokeKind, Ui, vec2};
use wordcraft_doc::freeform::InkTool;
use wordcraft_engine::cmd::draw::DrawMode;
use wordcraft_layout::DocLayout;

use crate::{WordApp, theme::Tokens};

/// Replace only the ink tool's winning canvas pointer. Chrome, overlays, scroll gestures,
/// table tools and the lasso keep their own pointers; no stale pointer survives PointerGone.
pub fn paint_cursor(app: &WordApp, ui: &Ui, response: &Response, layout: &DocLayout, rects: &[Rect], scale: f32) {
    if app.modal_open()
        || app.canvas.table_tool.is_some()
        || app.canvas.auto_scroll.blocks_input()
        || !(response.hovered() || response.dragged())
        || ui.ctx().output(|o| o.cursor_icon) != CursorIcon::Crosshair
    {
        return;
    }
    let mode = app.session.view.draw.mode;
    if !matches!(mode, DrawMode::Pen(_) | DrawMode::Eraser) {
        return;
    }
    let clip = response.interact_rect.intersect(ui.clip_rect());
    let Some(at) = ui.input(|i| i.pointer.hover_pos()).filter(|p| p.is_finite() && clip.contains(*p)) else { return };
    if app.canvas.balloon_rects.iter().any(|(_, r)| r.contains(at)) {
        return;
    }
    // page_at intentionally returns the nearest page even outside it for drags. The custom
    // pointer belongs only to visible paper, never the grey canvas or a clipped page.
    let Some((page, _, _)) = crate::canvas::page_at(rects, layout, scale, at) else { return };
    let Some(rect) = rects.get(page).filter(|r| r.contains(at)) else { return };
    let scale = layout.pages.get(page).map_or(scale, |p| crate::canvas::page_screen_scale(*rect, p, scale));
    let painter = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, Id::unique("wordcraft_ink_cursor"))).with_clip_rect(clip);
    let t = Tokens::get(ui.ctx());
    let (radius, icon, accent) = match mode {
        DrawMode::Pen(tool) => {
            let set = app.session.view.draw.settings(tool);
            let radius = footprint_radius(set.width, scale, tool);
            let color = super::screen_ink(crate::canvas::dark_page(app), set.color, (tool.alpha() * 255.0).round() as u8);
            // A light and dark rim keeps even black, white and translucent ink visible.
            painter.circle_stroke(at, radius + 1.5, Stroke::new(3.0, Color32::WHITE));
            painter.circle_stroke(at, radius + 1.5, Stroke::new(1.0, Color32::BLACK));
            painter.circle_filled(at, radius, color);
            let icon = match tool {
                InkTool::Pen => "pen",
                InkTool::Pencil => "pencil",
                InkTool::Highlighter => "highlightNoColor",
            };
            (radius, icon, color)
        }
        DrawMode::Eraser => {
            // This eraser deletes whole strokes, not a configurable disk of pixels.
            // Mark its exact hit point without suggesting a nonexistent erase radius.
            hotspot(&painter, at);
            (0.0, "eraser", t.accent)
        }
        _ => return,
    };
    let offset = radius.min(24.0) + 4.0;
    let badge = Rect::from_min_size(at + vec2(offset, -offset - 22.0), vec2(22.0, 22.0));
    painter.rect_filled(badge, 3.0, t.panel);
    painter.rect_stroke(badge, 3.0, Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
    crate::icons::paint(&painter, badge.shrink(1.0), icon, t.icon, accent);
    if matches!(mode, DrawMode::Pen(_)) {
        painter.line_segment([badge.left_bottom() + vec2(3.0, -2.0), badge.right_bottom() + vec2(-3.0, -2.0)], Stroke::new(2.0, accent));
    }
    ui.ctx().set_cursor_icon(CursorIcon::None);
}

fn footprint_radius(width: f32, scale: f32, tool: InkTool) -> f32 {
    let width = if width.is_finite() { width.clamp(0.25, 72.0) } else { tool.default_width() };
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    // Imported view geometry must not create an unbounded or nonfinite cursor mesh.
    (width * scale / 2.0).clamp(0.5, 256.0)
}

fn hotspot(painter: &Painter, at: Pos2) {
    for (width, color) in [(3.0, Color32::WHITE), (1.0, Color32::BLACK)] {
        let stroke = Stroke::new(width, color);
        painter.line_segment([at - vec2(3.0, 0.0), at + vec2(3.0, 0.0)], stroke);
        painter.line_segment([at - vec2(0.0, 3.0), at + vec2(0.0, 3.0)], stroke);
    }
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
