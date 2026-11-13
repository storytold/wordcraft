//! The Paste Options button (#1747): after a paste that could have gone another way, a small
//! button under the end of the pasted text — a clipboard icon and "(Ctrl)" — opens a menu to
//! redo it as Keep Source Formatting, Merge Formatting, a Picture or Keep Text Only, in place.
//! Tapping Ctrl opens the same menu. The offer lives in the engine
//! ([`wordcraft_engine::cmd::paste::paste_options`]): it ends when anything is typed or the caret
//! moves away, and Esc hides it.

use egui::{Context, Order, Sense, pos2, vec2};
use serde_json::json;

use crate::{WordApp, theme::Tokens};

const BUTTON_W: f32 = 58.0;
const BUTTON_H: f32 = 20.0;

/// What the UI remembers about the button.
#[derive(Default)]
pub struct PasteOptionsUi {
    /// The document revision whose offer Esc hid.
    hidden_rev: Option<u64>,
    /// The menu is open.
    open: bool,
    /// Ctrl is down.
    ctrl_down: bool,
    /// Ctrl went down and nothing else happened since: letting go is a tap.
    ctrl_tap: bool,
}

impl PasteOptionsUi {
    /// Follow this frame's events in order; true when Ctrl was tapped: pressed and released with
    /// no key, text or click in between, so Ctrl+V and the other shortcuts never count. Ctrl
    /// arrives as its own key event and as a modifier change, whichever the platform sends.
    fn ctrl_tapped(&mut self, events: &[egui::Event]) -> bool {
        use egui::{Event, Key};
        let mut tapped = false;
        for e in events {
            match e {
                Event::Key { key: Key::ControlLeft | Key::ControlRight, pressed, repeat, .. } => {
                    if *repeat {
                        continue;
                    }
                    if *pressed {
                        self.ctrl_down = true;
                        self.ctrl_tap = true;
                    } else {
                        tapped |= self.ctrl_down && self.ctrl_tap;
                        self.ctrl_down = false;
                        self.ctrl_tap = false;
                    }
                }
                Event::ModifiersChanged(m) => {
                    if m.ctrl && !self.ctrl_down {
                        self.ctrl_down = true;
                        self.ctrl_tap = true;
                    } else if !m.ctrl && self.ctrl_down {
                        tapped |= self.ctrl_tap;
                        self.ctrl_down = false;
                        self.ctrl_tap = false;
                    }
                }
                Event::Key { pressed: true, .. }
                | Event::Text(_)
                | Event::Paste(_)
                | Event::Copy
                | Event::Cut
                | Event::PointerButton { pressed: true, .. } => self.ctrl_tap = false,
                _ => {}
            }
        }
        tapped
    }
}

/// Where the button sits (below the caret) while the engine offers paste options and the caret is
/// in view; `None` when there is no offer, Esc hid it, or the caret is scrolled out of the page
/// area (under the ribbon or the status bar), where the button must not float over other panes.
fn button_pos(app: &mut WordApp, ctx: &Context) -> Option<egui::Pos2> {
    if wordcraft_engine::cmd::paste::paste_options(&app.session).is_empty() || app.paste_options.hidden_rev == Some(app.session.rev()) {
        return None;
    }
    let (caret, height) = crate::canvas::caret_screen(app)?;
    let at = pos2(caret.x, caret.y + height + 4.0);
    let view = app.canvas.canvas_rect.unwrap_or_else(|| ctx.content_rect());
    view.contains_rect(egui::Rect::from_min_size(at, vec2(BUTTON_W + 6.0, BUTTON_H + 6.0))).then_some(at)
}

/// Whether the Paste Options button is showing, so that Escape closes it first and does nothing
/// else (it must not also leave full screen).
pub fn wants_escape(app: &mut WordApp, ctx: &Context) -> bool {
    button_pos(app, ctx).is_some()
}

/// Draw the button below the caret while the engine offers paste options.
pub fn show(app: &mut WordApp, ctx: &Context) {
    let tapped = ctx.input(|i| app.paste_options.ctrl_tapped(&i.events));

    let offered = wordcraft_engine::cmd::paste::paste_options(&app.session);
    let rev = app.session.rev();
    let Some(at) = button_pos(app, ctx) else {
        app.paste_options.open = false;
        return;
    };
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.paste_options.hidden_rev = Some(rev);
        app.paste_options.open = false;
        return;
    }
    if tapped {
        app.paste_options.open = !app.paste_options.open;
    }
    let t = Tokens::get(ctx);
    let current = wordcraft_engine::cmd::paste::paste_mode(&app.session);
    let mut chosen = None;
    let mut button_rect = egui::Rect::NOTHING;
    egui::Area::new(egui::Id::unique("wc_paste_options")).order(Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.menu).inner_margin(2.0).show(ui, |ui| {
            let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON_W, BUTTON_H), Sense::click());
            // Screen readers (and tests) find the button by its name; its icon and "(Ctrl)" are drawn.
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Paste Options")));
            button_rect = rect;
            if resp.hovered() || app.paste_options.open {
                ui.painter().rect_filled(rect, 3.0, t.hover);
            }
            crate::icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(2.0, 1.0), vec2(18.0, 18.0)), "paste", t.icon, t.accent);
            ui.painter().text(
                pos2(rect.min.x + 22.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                tl!("(Ctrl)"),
                egui::FontId::proportional(12.0),
                t.text,
            );
            if resp.clicked() {
                app.paste_options.open = !app.paste_options.open;
            }
        });
    });
    if app.paste_options.open {
        // Open downwards, or upwards when the room below the button is short.
        let menu_h = offered.len() as f32 * 26.0 + 16.0;
        let below = ctx.content_rect().max.y - (button_rect.max.y + 6.0) >= menu_h;
        let area = egui::Area::new(egui::Id::unique("wc_paste_options_menu")).order(Order::Foreground);
        let area = if below {
            area.fixed_pos(pos2(at.x, button_rect.max.y + 6.0))
        } else {
            area.pivot(egui::Align2::LEFT_BOTTOM).fixed_pos(pos2(at.x, button_rect.min.y - 6.0))
        };
        let shown = area.show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(t.menu).show(ui, |ui| {
                ui.set_min_width(190.0);
                for (id, label) in wordcraft_engine::cmd::paste::PASTE_OPTIONS {
                    // The option the paste is in now is highlighted.
                    if offered.contains(&id) && ui.selectable_label(current == Some(id), tl!(label)).clicked() {
                        chosen = Some(id);
                    }
                }
            });
        });
        let outside = ctx.input(|i| i.pointer.any_pressed())
            && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| !shown.response.rect.contains(p) && !button_rect.contains(p));
        if outside {
            app.paste_options.open = false;
        }
    }
    if let Some(mode) = chosen {
        app.paste_options.open = false;
        let _ = app.run("edit.pasteChange", json!({"mode": mode}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers};
    use egui_kittest::kittest::Queryable;

    fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
    }

    #[test]
    fn a_bare_ctrl_tap_opens_but_ctrl_shortcuts_do_not() {
        let mut ui = PasteOptionsUi::default();
        // Ctrl as key events and modifier changes, as egui-winit sends it.
        assert!(!ui.ctrl_tapped(&[key(Key::ControlLeft, true, Modifiers::CTRL), Event::ModifiersChanged(Modifiers::CTRL)]));
        assert!(ui.ctrl_tapped(&[key(Key::ControlLeft, false, Modifiers::NONE), Event::ModifiersChanged(Modifiers::NONE)]));
        // Only modifier changes (no key events for Ctrl).
        assert!(!ui.ctrl_tapped(&[Event::ModifiersChanged(Modifiers::CTRL)]));
        assert!(ui.ctrl_tapped(&[Event::ModifiersChanged(Modifiers::NONE)]));
        // Ctrl+V is not a tap.
        assert!(!ui.ctrl_tapped(&[key(Key::ControlLeft, true, Modifiers::CTRL), Event::ModifiersChanged(Modifiers::CTRL)]));
        assert!(!ui.ctrl_tapped(&[key(Key::V, true, Modifiers::CTRL), key(Key::V, false, Modifiers::CTRL)]));
        assert!(!ui.ctrl_tapped(&[key(Key::ControlLeft, false, Modifiers::NONE), Event::ModifiersChanged(Modifiers::NONE)]));
        // A click while Ctrl is held is not a tap either.
        assert!(!ui.ctrl_tapped(&[Event::ModifiersChanged(Modifiers::CTRL)]));
        let click =
            Event::PointerButton { pos: egui::pos2(1.0, 1.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::CTRL };
        assert!(!ui.ctrl_tapped(&[click]));
        assert!(!ui.ctrl_tapped(&[Event::ModifiersChanged(Modifiers::NONE)]));
    }

    fn harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..6 {
            h.step();
        }
        h
    }

    /// A blank document with a formatted paste in it: a blue bold heading and a paragraph.
    fn pasted() -> WordApp {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("view.commentsPane", serde_json::json!({"value": false})).unwrap();
        let html = "<h2 style=\"color:#1f4e79;font-weight:bold\">Shopping day</h2><p>Apples</p>";
        a.run("edit.paste", serde_json::json!({"text": "Shopping day\nApples", "html": html})).unwrap();
        a
    }

    fn press(h: &mut egui_kittest::Harness<'static, WordApp>, k: Key) {
        h.event(Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.event(Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
        for _ in 0..3 {
            h.step();
        }
    }

    #[test]
    fn the_button_opens_a_menu_and_keep_text_only_redoes_the_paste() {
        let mut h = harness(pasted());
        assert!(wordcraft_engine::cmd::paste::paste_options(&h.state().session).len() == 4);
        assert!(
            h.state()
                .session
                .doc
                .para_at(&h.state().session.doc.start_of(wordcraft_doc::StoryRef::Body))
                .is_some_and(|p| p.props.style.is_some() || p.runs.iter().any(|r| r.props.color.is_some()))
        );
        h.get_by_label("Paste Options").click();
        for _ in 0..3 {
            h.step();
        }
        h.get_by_label("Keep Text Only").click();
        for _ in 0..3 {
            h.step();
        }
        let s = &h.state().session;
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body), "Shopping day\nApples");
        assert_eq!(wordcraft_engine::cmd::paste::paste_mode(s), Some("text"));
        let first = s.doc.para_at(&s.doc.start_of(wordcraft_doc::StoryRef::Body)).unwrap();
        assert!(first.props.style.is_none() && first.runs.iter().all(|r| r.props.color.is_none()), "plain text in the destination's formatting");
    }

    /// Choosing Picture for content too long to draw tells the person why on the status line, and
    /// leaves the paste and its button as they were.
    #[test]
    fn a_refused_picture_says_why() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
        a.run("view.commentsPane", serde_json::json!({"value": false})).unwrap();
        let html: String = (0..200).map(|i| format!("<p><b>line {i}</b></p>")).collect();
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        a.run("edit.paste", serde_json::json!({"text": text, "html": html})).unwrap();
        let before = a.session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        let mut h = harness(a);
        h.get_by_label("Paste Options").click();
        for _ in 0..3 {
            h.step();
        }
        h.get_by_label("Picture").click();
        for _ in 0..3 {
            h.step();
        }
        let msg = h.state().status_msg.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        assert!(msg.contains("too long") && msg.contains("choose another option"), "the status line says why and what to do: {msg:?}");
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), before, "the paste is unchanged");
        assert!(h.query_by_label("Paste Options").is_some(), "the button stays");
    }

    #[test]
    fn escape_and_typing_hide_the_button() {
        let mut h = harness(pasted());
        assert!(h.query_by_label("Paste Options").is_some());
        press(&mut h, Key::Escape);
        assert!(h.query_by_label("Paste Options").is_none(), "Escape hides the offer");
        let mut h = harness(pasted());
        h.event(Event::Text("x".into()));
        for _ in 0..3 {
            h.step();
        }
        assert!(h.query_by_label("Paste Options").is_none(), "typing ends the offer");
    }

    #[test]
    fn escape_closes_the_offer_without_leaving_full_screen() {
        let mut a = pasted();
        a.run("view.fullScreen", serde_json::json!({"value": true})).unwrap();
        let mut h = harness(a);
        assert!(h.query_by_label("Paste Options").is_some());
        press(&mut h, Key::Escape);
        assert!(h.query_by_label("Paste Options").is_none());
        assert!(h.state().session.view.full_screen, "the first Escape only closed the offer");
        press(&mut h, Key::Escape);
        assert!(!h.state().session.view.full_screen, "the next one leaves full screen");
    }
}
