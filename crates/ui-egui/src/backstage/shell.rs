//! Crossfade File and the editor using paint only. The outgoing screen never runs its widgets
//! or commands again, so an old Save, Close or template button cannot handle another click.

use std::sync::Arc;

use egui::epaint::ClippedShape;
use egui::{Painter, Rect, Shape, vec2};

const DURATION: f64 = 0.22;
// Only the visible window is retained. Bound its cached paint even for unusually complex UI.
const MAX_SHAPES: usize = 4096;

#[derive(Clone)]
struct Screen {
    file: bool,
    rect: Rect,
    scale: f32,
    presented: Arc<Vec<ClippedShape>>,
    leaving: Option<Arc<Vec<ClippedShape>>>,
    start: f64,
}

/// Keep each native document window's cached paint independent.
fn id(ctx: &egui::Context) -> egui::Id {
    egui::Id::unique(("backstage_shell_transition", ctx.viewport_id()))
}

/// Mark the first paint slot belonging to the screen, excluding its parent's earlier paint.
pub(crate) fn shape_count(painter: &Painter) -> usize {
    painter.ctx().graphics(|g| g.get(painter.layer_id()).map_or(0, |p| p.next_idx().0))
}

/// Crossfade the previous screen over the live one after layout, without running its controls.
pub(crate) fn paint(painter: &Painter, first: usize, file: bool, animate: bool, rtl: bool) {
    let ctx = painter.ctx();
    let id = id(ctx);
    if !animate {
        ctx.data_mut(|d| d.remove::<Screen>(id));
        return;
    }
    let rect = ctx.content_rect();
    let scale = ctx.pixels_per_point();
    let now = ctx.input(|i| i.time);
    let mut live = Vec::new();
    let mut index = 0;
    painter.for_each_shape(|s| {
        if index >= first && live.len() < MAX_SHAPES {
            live.push(s.clone());
        }
        index += 1;
    });
    let previous = ctx.data(|d| d.get_temp::<Screen>(id));
    // Text meshes depend on pixel density; old geometry also does not fit a resized window.
    let previous = previous.filter(|s| s.rect == rect && s.scale == scale);
    let mut screen = Screen { file, rect, scale, presented: Arc::new(Vec::new()), leaving: None, start: now };
    if let Some(previous) = previous {
        if previous.file != file {
            screen.leaving = Some(previous.presented);
        } else {
            screen.leaving = previous.leaving;
            screen.start = previous.start;
        }
    }
    let t = ((now - screen.start) / DURATION).clamp(0.0, 1.0) as f32;
    if t < 1.0 {
        if let Some(leaving) = &screen.leaving {
            let eased = 1.0 - (1.0 - t).powi(3);
            let distance = if file { 8.0 } else { 24.0 };
            let offset = vec2(if rtl { distance } else { -distance } * eased, 0.0);
            for s in leaving.iter().take(MAX_SHAPES.saturating_sub(live.len())) {
                if !replayable(ctx, &s.shape, 0) {
                    continue;
                }
                let mut faded = s.clone();
                faded.shape.translate(offset);
                faded.clip_rect = faded.clip_rect.translate(offset).intersect(rect);
                egui::epaint::shape_transform::adjust_colors(&mut faded.shape, move |c| {
                    if *c != egui::Color32::PLACEHOLDER {
                        *c = c.gamma_multiply(1.0 - eased);
                    }
                });
                painter.with_clip_rect(faded.clip_rect).add(faded.shape.clone());
                live.push(faded);
            }
            ctx.request_repaint();
        }
    } else {
        screen.leaving = None;
    }
    // Keep the composed frame so reversing the transition starts from what was actually visible.
    screen.presented = Arc::new(live);
    ctx.data_mut(|d| d.insert_temp(id, screen));
}

/// Skip backend callbacks and expired textures; bound traversal of nested paint groups.
fn replayable(ctx: &egui::Context, shape: &Shape, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    match shape {
        Shape::Callback(_) => false,
        Shape::Vec(shapes) => shapes.iter().all(|s| replayable(ctx, s, depth + 1)),
        _ => {
            let texture = shape.texture_id();
            // A different document or a refreshed preview may already have released its texture.
            texture == egui::TextureId::default() || ctx.tex_manager().read().meta(texture).is_some()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Pos2, RawInput};

    const EDITOR: Color32 = Color32::from_rgb(200, 40, 40);
    const FILE: Color32 = Color32::from_rgb(40, 40, 200);

    /// Advance a synthetic root window with one unmistakable colour per screen.
    fn frame(ctx: &egui::Context, file: bool, now: f64, animate: bool, width: f32) -> Vec<ClippedShape> {
        frame_in_viewport(ctx, file, now, animate, width, egui::ViewportId::ROOT)
    }

    /// Draw the same fixture in one of the context's native viewports.
    fn frame_in_viewport(ctx: &egui::Context, file: bool, now: f64, animate: bool, width: f32, viewport_id: egui::ViewportId) -> Vec<ClippedShape> {
        let mut input =
            RawInput { viewport_id, time: Some(now), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0))), ..Default::default() };
        input.viewports.insert(viewport_id, egui::ViewportInfo::default());
        let mut out = ctx.run_ui(input, |ui| {
            let first = shape_count(ui.painter());
            ui.painter().rect_filled(ui.max_rect(), 0.0, if file { FILE } else { EDITOR });
            paint(ui.painter(), first, file, animate, false);
        });
        out.textures_delta.clear();
        out.shapes
    }

    /// Read the last, overlaid rectangle's position and opacity.
    fn outgoing(shapes: &[ClippedShape]) -> (Rect, u8) {
        let Shape::Rect(r) = &shapes.last().unwrap().shape else { panic!("outgoing screen") };
        (r.rect, r.fill.a())
    }

    /// Both screen boundaries draw intermediate frames and release the outgoing paint at rest.
    #[test]
    fn entering_and_leaving_file_fade_the_previous_screen_and_settle() {
        let ctx = egui::Context::default();
        frame(&ctx, false, 0.0, true, 800.0);
        for (file, start) in [(true, 1.0), (false, 2.0)] {
            let begin = frame(&ctx, file, start, true, 800.0);
            let middle = frame(&ctx, file, start + 0.11, true, 800.0);
            let end = frame(&ctx, file, start + 0.3, true, 800.0);
            assert_eq!(begin.len(), 2, "both screens are drawn at the boundary");
            assert_eq!(outgoing(&begin).1, 255);
            assert!(outgoing(&middle).1 > 0 && outgoing(&middle).1 < 255);
            assert!(outgoing(&middle).0.min.x < outgoing(&begin).0.min.x, "the old screen slides toward the navigation edge");
            assert_eq!(end.len(), 1, "the old screen is released when the transition ends");
        }
    }

    /// Reduced motion and a new window size must never replay stale paint.
    #[test]
    fn reduced_motion_and_resizing_discard_the_old_screen() {
        let ctx = egui::Context::default();
        frame(&ctx, false, 0.0, true, 800.0);
        frame(&ctx, true, 1.0, true, 800.0);
        assert_eq!(frame(&ctx, true, 1.05, false, 800.0).len(), 1, "turning effects off finishes immediately");
        assert_eq!(frame(&ctx, true, 1.06, true, 800.0).len(), 1, "turning effects back on does not replay it");
        frame(&ctx, false, 2.0, true, 800.0);
        assert_eq!(frame(&ctx, false, 2.05, true, 900.0).len(), 1, "old geometry is not stretched into the new window");
    }

    /// A rapid reverse begins with the last visible composition, without jumping to a raw screen.
    #[test]
    fn reversing_mid_transition_keeps_the_composed_screen() {
        let ctx = egui::Context::default();
        frame(&ctx, false, 0.0, true, 800.0);
        frame(&ctx, true, 1.0, true, 800.0);
        let middle = frame(&ctx, true, 1.1, true, 800.0);
        let reverse = frame(&ctx, false, 1.1, true, 800.0);
        assert_eq!(&reverse[1..], &middle, "the visible composition is the starting point of the reverse");
        assert_eq!(frame(&ctx, false, 1.4, true, 800.0).len(), 1);
    }

    /// Changing File in one native window leaves the other window's paint alone.
    #[test]
    fn native_windows_keep_independent_shell_transitions() {
        let ctx = egui::Context::default();
        let other = egui::ViewportId::from_hash_of("other document");
        frame(&ctx, false, 0.0, true, 800.0);
        frame_in_viewport(&ctx, true, 0.0, true, 800.0, other);
        assert_eq!(frame(&ctx, true, 1.0, true, 800.0).len(), 2);
        assert_eq!(frame_in_viewport(&ctx, true, 1.1, true, 800.0, other).len(), 1);
        assert_eq!(frame(&ctx, true, 1.3, true, 800.0).len(), 1);
    }

    /// Expired preview images cannot be handed back to the GPU during a transition.
    #[test]
    fn a_preview_whose_texture_was_freed_is_not_replayed() {
        let ctx = egui::Context::default();
        let texture = ctx.load_texture("outgoing preview", egui::ColorImage::filled([2, 2], Color32::WHITE), Default::default());
        let mesh = egui::epaint::Mesh::with_texture(texture.id());
        let shape = Shape::mesh(mesh);
        assert!(replayable(&ctx, &shape, 0));
        drop(texture);
        assert!(!replayable(&ctx, &shape, 0));
    }
}
