//! The Read Aloud player: a small floating window with previous / play-pause / next / stop,
//! a speed slider and "Skip citations & bibliography". Every control runs a `readAloud.*`
//! command; the player itself is [`wordcraft_engine::speech::ReadAloud`].
//!
//! While reading, the caret follows the sentence being spoken (so stopping leaves it where
//! reading stopped) and the sentence is highlighted on the page.

use egui::{Color32, Rect, Sense, Ui, vec2};
use serde_json::json;
use wordcraft_engine::speech::{MAX_RATE, MIN_RATE, State};

use crate::WordApp;
use crate::theme::Tokens;

/// Per-frame bookkeeping: follow the spoken sentence with the caret, report errors, and keep
/// frames coming while reading.
pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    let ra = &app.session.read_aloud;
    if !ra.open {
        app.read_aloud_at = None;
        return;
    }
    if ra.state() == State::Playing {
        ctx.request_repaint_after(std::time::Duration::from_millis(120));
    }
    if let Some(e) = app.session.read_aloud.error()
        && app.read_aloud_error.as_deref() != Some(e.as_str())
    {
        app.read_aloud_error = Some(e.clone());
        app.status(e);
    }
    let current = app.session.read_aloud.current().map(|u| u.start);
    if current != app.read_aloud_at {
        if let Some(start) = &current {
            app.session.sel = wordcraft_engine::Selection::caret(app.session.doc.clamp(start));
            app.session.goal_x = None;
            app.canvas.scroll_to_caret = true;
        }
        app.read_aloud_at = current;
    }
}

/// The page rectangles (in page units) of the sentence being read, to highlight.
pub fn highlight(app: &mut WordApp) -> Option<(wordcraft_engine::doc::Pos, wordcraft_engine::doc::Pos)> {
    if !app.session.read_aloud.open {
        return None;
    }
    let u = app.session.read_aloud.current()?;
    Some((app.session.doc.clamp(&u.start), app.session.doc.clamp(&u.end)))
}

/// The player window.
pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    if !app.session.read_aloud.open {
        return;
    }
    let t = Tokens::get(ctx);
    let state = app.session.read_aloud.state();
    let (index, count) = app.session.read_aloud.progress();
    let mut open = true;
    let mut run: Option<(&str, serde_json::Value)> = None;
    let right_top = app.canvas.canvas_rect.map_or(ctx.content_rect().right_top() + vec2(-24.0, 160.0), |r| r.right_top() + vec2(-24.0, 16.0));
    egui::Window::new(tl!("Read Aloud"))
        .id(egui::Id::new("read-aloud-player"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::RIGHT_TOP)
        .default_pos(right_top)
        .show(ctx, |ui| {
            ui.set_width(236.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if button(ui, &t, "mediaPrev", tl!("Previous sentence"), count > 0 && index > 0) {
                    run = Some(("readAloud.previous", json!({})));
                }
                let (icon, tip) = if state == State::Playing { ("mediaPause", tl!("Pause")) } else { ("mediaPlay", tl!("Play")) };
                if button(ui, &t, icon, tip, true) {
                    run = Some(("readAloud.playPause", json!({})));
                }
                if button(ui, &t, "mediaNext", tl!("Next sentence"), index + 1 < count) {
                    run = Some(("readAloud.next", json!({})));
                }
                if button(ui, &t, "mediaStop", tl!("Stop"), true) {
                    run = Some(("readAloud.stop", json!({})));
                }
                ui.add_space(4.0);
                let what = match state {
                    State::Playing => format!("{} {} / {}", tl!("Sentence"), (index + 1).min(count), count),
                    State::Paused => tl!("Paused").to_string(),
                    State::Stopped if count > 0 && index >= count => tl!("Finished").to_string(),
                    State::Stopped => tl!("Stopped").to_string(),
                };
                ui.label(egui::RichText::new(what).color(t.text_dim).small());
            });
            ui.add_space(6.0);
            // Speed: applied when the slider is let go, so dragging doesn't restart the sentence
            // on every frame.
            let id = egui::Id::new("read-aloud-rate");
            let mut rate = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or_else(|| app.session.read_aloud.rate());
            ui.horizontal(|ui| {
                ui.label(tl!("Speed"));
                ui.spacing_mut().slider_width = 110.0;
                let r = ui.add(
                    egui::Slider::new(&mut rate, MIN_RATE..=MAX_RATE)
                        .step_by(0.05)
                        .trailing_fill(true)
                        .custom_formatter(|v, _| format!("{v:.2}×"))
                        .custom_parser(|s| s.trim().trim_end_matches(['×', 'x']).trim().parse().ok()),
                );
                if r.dragged() {
                    ui.data_mut(|d| d.insert_temp(id, rate));
                } else {
                    ui.data_mut(|d| d.remove::<f32>(id));
                    if r.changed() || r.drag_stopped() {
                        run = Some(("readAloud.speed", json!({"value": rate})));
                    }
                }
                if ui.small_button("1×").on_hover_text(tl!("Normal speed")).clicked() {
                    run = Some(("readAloud.speed", json!({"value": 1.0})));
                }
            });
            let mut skip = app.session.read_aloud.skip_citations;
            if ui
                .checkbox(&mut skip, tl!("Skip citations & bibliography"))
                .on_hover_text(tl!("Don't read Zotero, Mendeley or EndNote citations and bibliographies"))
                .changed()
            {
                run = Some(("readAloud.skipCitations", json!({"value": skip})));
            }
            if let Some(e) = app.session.read_aloud.error() {
                ui.label(egui::RichText::new(e).color(t.red).small());
            }
        });
    if !open {
        run = Some(("readAloud.stop", json!({})));
    }
    if let Some((id, params)) = run {
        let _ = app.run(id, params);
        app.ui.read_aloud_rate = app.session.read_aloud.rate();
        app.ui.read_aloud_skip_citations = app.session.read_aloud.skip_citations;
        ctx.request_repaint();
    }
}

/// An icon button; returns whether it was clicked.
fn button(ui: &mut Ui, t: &Tokens, icon: &str, tip: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    let resp = resp.on_hover_text(tip);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, if resp.is_pointer_button_down_on() { t.pressed } else { t.hover });
    }
    let c = if enabled { t.icon } else { t.text_disabled };
    let r = Rect::from_center_size(rect.center(), vec2(20.0, 20.0));
    crate::icons::paint(ui.painter(), r, icon, c, if enabled { t.accent } else { Color32::TRANSPARENT });
    enabled && resp.clicked()
}
