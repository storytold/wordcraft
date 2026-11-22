//! Modal dialog chrome: modal nudge/shake animations and decorations (#3859, #4076, #7321).

use craft::ui::runtime::env::Env;
use craft::ui::style::rgba_color;
use egui::emath::TSTransform;
use egui::{Context, CornerRadius, Id, LayerId, Order, Stroke, StrokeKind, Ui, vec2};

use crate::WordApp;

/// How long a modal window shakes or flashes after a press beside it or close attempt, in seconds.
pub(crate) const NUDGE_SECS: f64 = 0.4;

/// The `ctx.data` key for when the window `id` was last nudged in `viewport`.
pub(crate) fn nudge_id_in(id: Id, viewport: egui::ViewportId) -> Id {
    id.with(("nudge", viewport))
}

/// The `ctx.data` key for when the window `id` was last nudged, in this document window: the data
/// is shared by all of them.
pub(crate) fn nudge_id(ctx: &Context, id: Id) -> Id {
    nudge_id_in(id, ctx.viewport_id())
}

/// Forgets a nudge of the window `id` in `viewport` as it closes, so it doesn't go on when the window reopens.
pub(crate) fn forget_nudge_in(ctx: &Context, id: Id, viewport: egui::ViewportId) {
    let key = nudge_id_in(id, viewport);
    let cur_key = nudge_id(ctx, id);
    ctx.data_mut(|d| {
        d.remove::<f64>(key);
        d.remove::<f64>(cur_key);
    });
}

/// The shake's sideways offset at `t` (0 to 1) through it: three swings either way of up to 6
/// points, dying away.
pub(crate) fn shake_offset(t: f32) -> f32 {
    6.0 * (t * 3.0 * std::f32::consts::TAU).sin() * (1.0 - t)
}

/// Triggers a shake/nudge animation on dialog `id` if not already running.
pub(crate) fn trigger_prompt_nudge(ctx: &Context, id: Id) {
    let key = nudge_id(ctx, id);
    let now = ctx.input(|i| i.time);
    let running = ctx.data(|d| d.get_temp::<f64>(key)).is_some_and(|s| (0.0..NUDGE_SECS).contains(&(now - s)));
    if !running {
        ctx.data_mut(|d| d.insert_temp(key, now));
    }
    ctx.request_repaint();
}

/// Applies modal shake / nudge animation (or accent border flashes under reduced motion) to the
/// active modal dialog. Also detects presses beside the dialog (on the backdrop or outside) to
/// trigger the nudge.
pub(crate) fn apply_modal_nudge(app: &mut WordApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if craft::ui::runtime::measure::active(&ctx) {
        return;
    }
    let window_vp = app.ctx.as_ref().and_then(|c| app.windows.viewport(c, app.windows.shown())).unwrap_or(egui::ViewportId::ROOT);
    if ctx.viewport_id() != window_vp {
        return;
    }

    let prompt_id = app.closing.as_ref().and_then(|c| c.prompt).filter(|&id| app.dialog_host.get_dyn(id).is_some());
    let target_id =
        prompt_id.or_else(|| app.dialog_host.shows_modal().then(|| app.dialog_host.open_dialogs().into_iter().next_back().map(|d| d.id)).flatten());
    let Some(id) = target_id else {
        return;
    };

    let hosted = app.dialog_host.get_dyn(id).is_some();
    if !hosted {
        return;
    }

    if let Some(closing) = app.closing.as_mut()
        && closing.pending_nudge
        && closing.prompt == Some(id)
    {
        closing.pending_nudge = false;
        closing.viewport = Some(ctx.viewport_id());
        trigger_prompt_nudge(&ctx, id);
    }

    let top_id = craft::ui::runtime::overlay::top(&ctx);
    let top_entry = top_id.and_then(|tid| craft::ui::runtime::overlay::entry(&ctx, tid));
    let modal_layer_id = ctx.memory(|m| m.top_modal_layer());

    let (dialog_rect, area_layer_id) = if let Some(entry) = top_entry {
        (Some(entry.rect), Some(LayerId::new(Order::Foreground, craft::ui::runtime::memory::sub(entry.id, "area"))))
    } else if let Some(layer) = modal_layer_id {
        (ctx.memory(|m| m.area_rect(layer.id)), Some(layer))
    } else {
        (None, None)
    };

    let Some(rect) = dialog_rect else {
        return;
    };

    let beside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| !rect.contains(p)));
    if beside {
        if let Some(closing) = app.closing.as_mut()
            && closing.prompt == Some(id)
        {
            closing.viewport = Some(ctx.viewport_id());
        }
        trigger_prompt_nudge(&ctx, id);
    }

    let key = nudge_id(&ctx, id);
    let now = ctx.input(|i| i.time);
    let start = match ctx.data(|d| d.get_temp::<f64>(key)).filter(|s| (0.0..NUDGE_SECS).contains(&(now - s))) {
        Some(s) => s,
        None => return,
    };

    ctx.request_repaint();
    let t = ((now - start) / NUDGE_SECS).clamp(0.0, 1.0) as f32;
    let animate = app.animations_on() && app.system_animations();

    if animate {
        let offset = shake_offset(t);
        let transform = TSTransform::from_translation(vec2(offset, 0.0));
        if let Some(area_layer) = area_layer_id {
            ctx.transform_layer_shapes(area_layer, transform);
        }
        if let Some(modal_layer) = modal_layer_id
            && Some(modal_layer) != area_layer_id
        {
            ctx.transform_layer_shapes(modal_layer, transform);
        }
    } else if ((t * 4.0) as u32).is_multiple_of(2) {
        let env = Env::of(ui);
        let radius = CornerRadius::from(env.tokens.shape.radius.window);
        let stroke = Stroke::new(2.0, rgba_color(env.tokens.color.accent.solid));
        if let Some(area_layer) = area_layer_id {
            ctx.layer_painter(area_layer).rect_stroke(rect, radius, stroke, StrokeKind::Inside);
        }
        if let Some(modal_layer) = modal_layer_id
            && Some(modal_layer) != area_layer_id
        {
            ctx.layer_painter(modal_layer).rect_stroke(rect, radius, stroke, StrokeKind::Inside);
        }
    }
}
