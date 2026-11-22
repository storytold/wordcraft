//! The Read Aloud player: a small floating window with previous / play-pause / next / stop,
//! a speed slider and "Skip citations & bibliography". Every control runs a `readAloud.*`
//! command; the player itself is [`wordcraft_engine::speech::ReadAloud`].
//!
//! While reading, the caret follows the sentence being spoken (so stopping leaves it where
//! reading stopped) and the sentence is highlighted on the page.

use craft::behavior::IconId;
use craft::theme::TypeRole;
use craft::theme::variant::Tone;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::Dialog;
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::notices::Notice;
use craft::ui::value_drag::Slider;
use egui::{Align2, Ui, vec2};
use serde_json::json;
use wordcraft_engine::speech::{MAX_RATE, MIN_RATE, State};

use crate::WordApp;

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
    if let Some(e) = error_notice(&mut app.read_aloud_error, app.session.read_aloud.error()) {
        app.status(e);
    }
    // The caret follows the reading only in the document being read.
    let current = if reads_shown_document(app) { app.session.read_aloud.current().map(|u| u.start) } else { None };
    if current != app.read_aloud_at {
        if let Some(start) = &current {
            app.session.set_selection(wordcraft_engine::Selection::caret(app.session.doc.clamp(start)));
            app.session.goal_x = None;
            app.canvas.scroll_to_caret = true;
        }
        app.read_aloud_at = current;
    }
}

/// The notice for the player's error `now`, given the one last `shown`: each message is shown
/// once, and when the error clears (Play after a halt) the last one is forgotten, so the same
/// message coming back is shown again.
fn error_notice(shown: &mut Option<String>, now: Option<String>) -> Option<String> {
    match now {
        None => {
            *shown = None;
            None
        }
        Some(e) if shown.as_deref() == Some(e.as_str()) => None,
        Some(e) => {
            *shown = Some(e.clone());
            Some(e)
        }
    }
}

/// The page rectangles (in page units) of the sentence being read, to highlight.
pub fn highlight(app: &mut WordApp) -> Option<(wordcraft_engine::doc::Pos, wordcraft_engine::doc::Pos)> {
    if !app.session.read_aloud.open || !reads_shown_document(app) {
        return None;
    }
    let u = app.session.read_aloud.current()?;
    Some((app.session.doc.clamp(&u.start), app.session.doc.clamp(&u.end)))
}

/// Whether the reading is of the document the window shows: its sentence positions are in it.
fn reads_shown_document(app: &WordApp) -> bool {
    app.session.read_aloud.document == Some(app.session.document_id())
}

/// The player: a modeless dialog shown each pass from the body of the window it belongs to, while
/// the reading is open. It borrows the app, so the dialog host cannot own it.
// PLAN: assumed caller (app-hub, `app/frame.rs:142`) passes the window body's Ui.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui) {
    if !app.session.read_aloud.open {
        return;
    }
    // With a window per document, the player is in the window of the document being read.
    if app.windows.native()
        && let Some(d) = app.session.read_aloud.document
        && d != app.session.document_id()
        && app.session.is_open(d)
    {
        return;
    }
    let state = app.session.read_aloud.state();
    let (index, count) = app.session.read_aloud.progress();
    let mut run: Option<(&str, serde_json::Value)> = None;
    // PLAN: assumed a public `Ui` -> panel padding accessor (`space.pad.regular`); until it exists the theme's window margin stands in.
    let pad = f32::from(ui.style().spacing.window_margin.left);
    let at = app.canvas.canvas_rect().map_or(ui.ctx().content_rect().right_top() + vec2(-pad, pad), |r| r.right_top() + vec2(-pad, pad));
    let mut open = true;
    let dialog = Dialog::new(tl!("Read Aloud")).modeless().first_place(at, Align2::RIGHT_TOP).opened(Bind::bound(&mut open));
    dialog.show(ui, |ui, _| {
        Bar::new(tl!("Read Aloud"), BarKind::Options).tab_stops().show(ui, |ui, _| {
            let previous = IconButton::new(IconId::new("step_back"))
                .label(tl!("Previous sentence"))
                .tooltip(tl!("Previous sentence"))
                .enabled(count > 0 && index > 0)
                .show(ui);
            if previous.clicked() {
                run = Some(("readAloud.previous", json!({})));
            }
            let (icon, tip) = if state == State::Playing { ("pause", tl!("Pause")) } else { ("play", tl!("Play")) };
            if IconButton::new(crate::icons::id(icon)).label(tip).tooltip(tip).show(ui).clicked() {
                run = Some(("readAloud.playPause", json!({})));
            }
            let next = IconButton::new(IconId::new("step_forward"))
                .label(tl!("Next sentence"))
                .tooltip(tl!("Next sentence"))
                .enabled(index + 1 < count)
                .show(ui);
            if next.clicked() {
                run = Some(("readAloud.next", json!({})));
            }
            if IconButton::new(IconId::new("square")).label(tl!("Stop")).tooltip(tl!("Stop")).show(ui).clicked() {
                run = Some(("readAloud.stop", json!({})));
            }
            let what = match state {
                State::Playing => format!("{} {} / {}", tl!("Sentence"), (index + 1).min(count), count),
                State::Paused => tl!("Paused").to_string(),
                State::Stopped if count > 0 && index >= count => tl!("Finished").to_string(),
                State::Stopped => tl!("Stopped").to_string(),
            };
            Text::new(what).shade(Shade::Muted).role(TypeRole::Caption).show(ui);
        });
        // Speed: sent when the drag is committed, so dragging doesn't restart the sentence.
        let mut rate = f64::from(app.session.read_aloud.rate());
        Row::new().show(ui, |ui, _| {
            let value_text = format!("{rate:.2}×");
            let slider = Slider::new(Bind::bound(&mut rate), f64::from(MIN_RATE)..=f64::from(MAX_RATE))
                .label(tl!("Speed"))
                .step(0.05)
                .value_text(value_text)
                .show(ui);
            if let Some(value) = slider.events.committed() {
                run = Some(("readAloud.speed", json!({"value": *value})));
            }
            if Button::new(tl!("1×")).tooltip(tl!("Normal speed")).show(ui).clicked() {
                run = Some(("readAloud.speed", json!({"value": 1.0})));
            }
        });
        let skip = app.session.read_aloud.skip_citations;
        let checkbox = Checkbox::new(Bind::given(skip))
            .label(tl!("Skip citations & bibliography"))
            .tooltip(tl!("Don't read Zotero, Mendeley or EndNote citations and bibliographies"))
            .show(ui);
        if let Some(on) = checkbox.value().copied() {
            run = Some(("readAloud.skipCitations", json!({"value": on})));
        }
        // The error `poll` saw this frame, not the worker's latest: the player thread can set
        // one between the two, and the window would show a message the status bar hasn't.
        if let Some(e) = app.read_aloud_error.as_ref().filter(|e| app.session.read_aloud.error().as_ref() == Some(*e)) {
            Notice::new(Tone::Bad, e.clone()).show(ui);
        }
    });
    // Escape and the close box both write the bound open state.
    if !open {
        run = Some(("readAloud.stop", json!({})));
    }
    if let Some((id, params)) = run {
        let _ = app.run(id, params);
        app.ui.read_aloud_rate = app.session.read_aloud.rate();
        app.ui.read_aloud_skip_citations = app.session.read_aloud.skip_citations;
        ui.ctx().request_repaint();
    }
}
