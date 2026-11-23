use egui::{Response, Sense, Ui, vec2};

fn hint_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
    ctx
}

fn hint_showing(ctx: &egui::Context, t0: f64, frames: u32, events: Vec<egui::Event>, build: &mut dyn FnMut(&mut Ui)) -> bool {
    let mut shown = false;
    for k in 0..frames {
        let input = egui::RawInput {
            time: Some(t0 + f64::from(k) * 0.1),
            predicted_dt: 0.1,
            events: if k == 0 { events.clone() } else { vec![] },
            ..Default::default()
        };
        ctx.run_ui(input, |ui| build(ui)).drop_without_applying_deltas();
        shown = ctx.memory(|m| m.areas().visible_layer_ids().iter().any(|l| l.order == egui::Order::Tooltip));
    }
    shown
}

fn id_since_focus(resp: &Response) -> Option<f32> {
    let key = resp.id.with("hint_focus_since");
    let now = resp.ctx.input(|i| i.time);
    if !resp.has_focus() {
        resp.ctx.data_mut(|d| d.remove::<f64>(key));
        return None;
    }
    let since = resp.ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
    Some((now - since) as f32)
}

fn show_hint(resp: &Response, title: &str, notes: &[String]) {
    let add = |ui: &mut Ui| {
        ui.set_max_width(260.0);
        ui.label(egui::RichText::new(title));
        for n in notes {
            ui.label(egui::RichText::new(n).small().weak());
        }
    };
    let ctx = &resp.ctx;
    let since_key = id_since_focus(resp);
    let hover_key = egui::Id::unique("hint_pointer_over_a_control");
    let pass = ctx.cumulative_pass_nr();
    let over = resp.hovered() || (!resp.enabled() && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| resp.rect.contains(p)));
    if over {
        ctx.data_mut(|d| d.insert_temp(hover_key, pass));
    }
    if resp.hovered() || !resp.has_focus() {
        if resp.enabled() {
            egui::Tooltip::for_enabled(resp).show(add);
        } else {
            egui::Tooltip::for_disabled(resp).show(add);
        }
    }
    if let Some(waited) = since_key {
        let delay = ctx.global_style().interaction.tooltip_delay;
        let pointer_idle = !ctx.input(|i| i.pointer.has_pointer()) || ctx.input(|i| i.pointer.time_since_last_movement()) > 2.0;
        let pointer_over_other = ctx.data(|d| d.get_temp::<u64>(hover_key)).is_some_and(|p| p + 1 >= pass);
        if waited < delay {
            ctx.request_repaint_after_secs(delay - waited);
        } else if !resp.hovered() && pointer_idle && !pointer_over_other && !egui::Popup::is_any_open(ctx) {
            egui::Tooltip::for_widget(resp).show(add);
        }
    }
}

#[test]
fn keyboard_focus_shows_the_hint_after_a_delay_but_not_over_a_menu() {
    let ctx = hint_ctx();
    let btn_id = std::cell::Cell::new(None);
    let menu = std::cell::Cell::new(false);
    let mut build = |ui: &mut Ui| {
        let (_, resp) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
        btn_id.set(Some(resp.id));
        if menu.get() {
            egui::Popup::menu(&resp).open_memory(egui::SetOpenCommand::Bool(true)).show(|ui| ui.label("menu"));
        }
        show_hint(&resp, "Bold", &[]);
    };
    hint_showing(&ctx, 1.0, 2, vec![], &mut build);
    let Some(id) = btn_id.get() else { return };
    // Tab moves the keyboard focus onto the button, as it does for a user.
    let tab = egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    // Just focused: no hint yet; after the tooltip delay: shown.
    assert!(!hint_showing(&ctx, 2.0, 2, vec![tab], &mut build));
    assert!(ctx.memory(|m| m.has_focus(id)), "Tab did not focus the button");
    assert!(hint_showing(&ctx, 2.2, 10, vec![], &mut build), "focused control shows no hint");
    // A menu open anywhere hides it.
    menu.set(true);
    assert!(!hint_showing(&ctx, 4.0, 3, vec![], &mut build), "hint drawn over an open menu");
}
