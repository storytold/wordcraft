//! Compact drawing-tool pointers. Colour and width update on every platform, including
//! the web. The footprint is centred on the actual drawing hotspot, without a tool badge.

use egui::{Color32, CursorIcon, Id, LayerId, Order, Painter, Pos2, Rect, Response, Stroke, StrokeKind, Ui, vec2};
use wordcraft_doc::freeform::InkTool;
use wordcraft_engine::cmd::draw::DrawMode;
use wordcraft_layout::DocLayout;

use crate::WordApp;

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
    match mode {
        DrawMode::Pen(tool) => {
            let set = app.session.view.draw.settings(tool);
            let radius = footprint_radius(set.width, scale, tool);
            let color = super::screen_ink(crate::canvas::dark_page(app), set.color, (tool.alpha() * 255.0).round() as u8);
            // A thin light/dark rim keeps black, white and translucent ink visible without
            // covering nearby text. The square bounds distinguish the marker's nib, but its
            // coloured centre stays round to preview the existing round-ended ink stroke.
            for (width, color) in [(2.0, Color32::WHITE), (0.75, Color32::BLACK)] {
                let stroke = Stroke::new(width, color);
                if tool == InkTool::Highlighter {
                    painter.rect_stroke(Rect::from_center_size(at, vec2(radius + 1.0, radius + 1.0) * 2.0), 0.0, stroke, StrokeKind::Middle);
                } else {
                    painter.circle_stroke(at, radius + 1.0, stroke);
                }
            }
            painter.circle_filled(at, radius, color);
        }
        DrawMode::Eraser => {
            // This eraser deletes whole strokes, not a configurable disk of pixels.
            // Mark its exact hit point without suggesting a nonexistent erase radius.
            hotspot(&painter, at);
            let icon = Rect::from_min_size(at + vec2(2.0, -18.0), vec2(22.0, 22.0));
            // Paper colour, rather than interface theme, determines the outline contrast.
            let outline = super::screen_ink(crate::canvas::dark_page(app), wordcraft_doc::Rgb(0, 0, 0), 255);
            crate::icons::paint(&painter, icon, "eraser", outline, outline);
        }
        _ => return,
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
    painter.circle_filled(at, 2.0, Color32::WHITE);
    painter.circle_filled(at, 1.0, Color32::BLACK);
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
