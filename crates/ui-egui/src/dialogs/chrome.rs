//! Modal dialog chrome: modal nudge/shake animations and decorations (#3859, #4076, #7321).

use craft::ui::runtime::env::Env;
use craft::ui::style::rgba_color;
use egui::emath::TSTransform;
use egui::{Context, CornerRadius, Id, LayerId, Order, Stroke, StrokeKind, Ui, vec2};

use crate::WordApp;

/// How long a modal window shakes or flashes after a press beside it, in seconds.
pub(crate) const NUDGE_SECS: f64 = 0.4;

/// The `ctx.data` key for when the window `id` was last nudged, in this document window: the data
/// is shared by all of them.
pub(crate) fn nudge_id(ctx: &Context, id: Id) -> Id {
    id.with(("nudge", ctx.viewport_id()))
}

/// Forgets a nudge of the window `id` as it closes, so it doesn't go on when the window reopens.
pub(crate) fn forget_nudge(ctx: &Context, id: Id) {
    let key = nudge_id(ctx, id);
    ctx.data_mut(|d| {
        d.remove::<f64>(key);
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

    // Forget nudge on every dialog close, so reopening within NUDGE_SECS opens still.
    // Tracking the host-provided open_serial distinguishes successive opens sharing the same ID.
    let prev_open_key = Id::NULL.with("modal_dialogs_open_last_pass").with(ctx.viewport_id());
    let prev_open: Vec<(Id, u64)> = ctx.data(|d| d.get_temp(prev_open_key)).unwrap_or_default();
    let current_open: Vec<(Id, u64)> =
        app.dialog_host.open_dialogs().into_iter().filter_map(|d| app.dialog_host.open_serial(d.id).map(|s| (d.id, s))).collect();
    for (closed_id, prev_serial) in &prev_open {
        if !current_open.iter().any(|(id, serial)| id == closed_id && serial == prev_serial) {
            forget_nudge(&ctx, *closed_id);
        }
    }
    ctx.data_mut(|d| d.insert_temp(prev_open_key, current_open));

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

    // When popups/menus are open inside a modal dialog, overlay::top points to the popup.
    // Walk up .parent to locate the target modal dialog's own overlay entry so clicking
    // inside the dialog (outside the popup) does not falsely trigger a shake.
    let top_id = craft::ui::runtime::overlay::top(&ctx);
    let mut modal_entry = top_id.and_then(|tid| craft::ui::runtime::overlay::entry(&ctx, tid));
    while let Some(entry) = modal_entry.as_ref() {
        if entry.rank.is_modal() {
            break;
        }
        modal_entry = entry.parent.and_then(|pid| craft::ui::runtime::overlay::entry(&ctx, pid));
    }
    let modal_entry = modal_entry.filter(|e| e.rank.is_modal());
    let modal_layer_id = ctx.memory(|m| m.top_modal_layer());

    let (dialog_rect, area_layer_id) = if let Some(ref entry) = modal_entry {
        (Some(entry.rect), Some(LayerId::new(Order::Foreground, craft::ui::runtime::memory::sub(entry.id, "area"))))
    } else if let Some(layer) = modal_layer_id {
        (ctx.memory(|m| m.area_rect(layer.id)), Some(layer))
    } else {
        (None, None)
    };

    let Some(rect) = dialog_rect else {
        return;
    };

    // Include the modal dialog's rectangle and all open overlays in its family
    // (such as dropdown popups or submenus) so presses on popup extensions are inside.
    let mut inside_rects = vec![rect];
    if let Some(ref entry) = modal_entry {
        for family_id in craft::ui::runtime::overlay::family(&ctx, entry.id) {
            if let Some(e) = craft::ui::runtime::overlay::entry(&ctx, family_id) {
                inside_rects.push(e.rect);
                if let Some(anchor) = e.anchor {
                    inside_rects.push(anchor);
                }
            }
        }
    }
    let is_inside = |pos: egui::Pos2| inside_rects.iter().any(|r| r.contains(pos));

    let beside = ctx.input(|i| {
        let event_pressed = i.events.iter().any(|e| match e {
            egui::Event::PointerButton { pos, pressed: true, button: egui::PointerButton::Primary, .. } => !is_inside(*pos),
            _ => false,
        });
        let held_pressed = i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| !is_inside(p));
        event_pressed || held_pressed
    });
    if beside {
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
