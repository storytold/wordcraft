//! The chat pane (right dock, Review › Chat): Start and Stop chat, invite and remove agents, the
//! conversation and the owner's input. Pane text goes through `tl!`; the chat itself (system
//! lines, what agents read) stays English.

use egui::{RichText, Ui};
use serde_json::{Value, json};
use wordcraft_chat::Role;

use crate::WordApp;
use crate::theme::{Tokens, semibold};

const TITLE: &str = "Chat";
const IDLE: &str = "Talk with AI agents about this document. Start Chat opens a port on this computer for them.";
const START: &str = "Start Chat";
const STOP: &str = "Stop Chat";
const RUNNING: &str = "Agents join at {address}";
const USER_NAME: &str = "User name:";
const NAME_NO_AT: &str = "The user name cannot start with @.";
const INVITE: &str = "Invite Agent";
const NEED_NAME: &str = "Type an agent name";
const INVITE_HINT: &str = "Give this line to the agent. It works once, for 10 minutes.";
const COPY: &str = "Copy";
const MEMBERS: &str = "Members";
const REMOVE: &str = "Remove";
const NO_MEMBERS: &str = "No agents in the chat yet.";
const INPUT_HINT: &str = "Message (@name …), Enter to send";
const LOG_NOT_SAVED: &str = "Chat log not saved: {error}";
const IN_MEMORY: &str = "The chat is kept in memory until you save the document.";
const BRAKE: &str = "Agents wait for you (8 messages in a row).";
const NOT_SENT: &str = "Not sent: {error}";
const FAILED: &str = "Failed: {error}";
const NO_CHAT: &str = "No chat in this window.";

/// Every string the pane shows (each catalog must have them all).
pub const PANE_STRINGS: [&str; 21] = [
    TITLE,
    IDLE,
    START,
    STOP,
    RUNNING,
    USER_NAME,
    NAME_NO_AT,
    INVITE,
    INVITE_HINT,
    NEED_NAME,
    COPY,
    MEMBERS,
    REMOVE,
    NO_MEMBERS,
    INPUT_HINT,
    LOG_NOT_SAVED,
    IN_MEMORY,
    BRAKE,
    NOT_SENT,
    FAILED,
    NO_CHAT,
];

/// The owner's name must not start with `@` (agents' names always do).
pub fn valid_owner_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.starts_with('@')
}

/// Enter (without Shift) in the input: the text to send (trimmed, without a trailing newline;
/// `None` when there is nothing to send) and the box content after a successful send.
pub fn on_enter(text: &str) -> (Option<String>, String) {
    let t = text.strip_suffix('\n').unwrap_or(text).trim();
    if t.is_empty() { (None, String::new()) } else { (Some(t.to_string()), String::new()) }
}

/// What the name field shows: the session's user name, or the owner's own typing while it is
/// not applied (a name that is refused, or the user name with spaces still around it).
fn owner_field(kept: Option<String>, author: &str) -> String {
    match kept {
        Some(k) if !valid_owner_name(&k) || k.trim() == author => k,
        _ => author.to_string(),
    }
}

/// The message box.
fn input_id() -> egui::Id {
    egui::Id::new("chat_input_box")
}

/// The name the invite field starts with.
const DEFAULT_HANDLE: &str = "@agent";

/// The field's text now: what the user left there, else the default (real text, not a hint).
fn invite_field(ui: &Ui) -> String {
    ui.data(|d| d.get_temp::<String>(egui::Id::new("chat_invite_handle"))).unwrap_or_else(|| DEFAULT_HANDLE.to_string())
}

/// An invite needs a name that is not empty or only spaces.
fn can_invite(handle: &str) -> bool {
    !handle.trim().is_empty()
}

/// An invite waiting for its agent (what `chat.invite` returned).
#[derive(Clone, Debug, PartialEq)]
pub struct PendingInvite {
    pub handle: String,
    pub code: String,
    pub line: String,
}

/// The invite while it still waits; `None` once its agent joined or it expired.
pub fn still_open(hub: &wordcraft_chat::Hub, inv: Option<PendingInvite>) -> Option<PendingInvite> {
    inv.filter(|i| hub.invite_open(&i.code))
}

pub fn show(app: &mut WordApp, ui: &mut Ui) {
    if !app.ui.chat_pane {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::right("chat_pane")
        .default_size(320.0)
        .resizable(true)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(10).stroke(egui::Stroke::new(1.0, t.border)))
        .show(ui, |ui| pane(app, ui));
}

fn red(ui: &mut Ui, text: String) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(text).small().color(t.red));
}

fn failed(e: &str) -> String {
    crate::i18n::fmt(tl!(FAILED), &[("error", e)])
}

fn pane(app: &mut WordApp, ui: &mut Ui) {
    if crate::panes::header(ui, TITLE) {
        let _ = app.run("chat.open", json!({"value": false}));
        app.ui.chat_pane = false;
        return;
    }
    let Some(chat) = app.session.chat.clone() else {
        ui.label(tl!(NO_CHAT));
        return;
    };
    let t = Tokens::get(ui.ctx());
    let err_id = egui::Id::new("chat_error");
    if !chat.running() {
        ui.label(tl!(IDLE));
        if ui.button(tl!(START)).clicked() {
            match app.run("chat.start", json!({})) {
                Ok(_) => ui.data_mut(|d| d.remove::<String>(err_id)),
                Err(e) => ui.data_mut(|d| {
                    d.insert_temp(err_id, failed(&e));
                }),
            }
        }
        if let Some(e) = ui.data(|d| d.get_temp::<String>(err_id)) {
            red(ui, e);
        }
        return;
    }
    let hub = chat.hub().clone();
    ui.horizontal(|ui| {
        let address = chat.address().unwrap_or_default();
        ui.label(RichText::new(crate::i18n::fmt(tl!(RUNNING), &[("address", &address)])).small().color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button(tl!(STOP)).clicked() {
                let _ = app.run("chat.stop", json!({}));
                ui.data_mut(|d| d.remove::<String>(err_id));
            }
        });
    });
    // The owner's name: the author of the owner's own edits.
    let name_id = egui::Id::new("chat_owner_name");
    let mut name = owner_field(ui.data(|d| d.get_temp::<String>(name_id)), &app.session.author);
    ui.horizontal(|ui| {
        ui.label(tl!(USER_NAME));
        if ui.add(egui::TextEdit::singleline(&mut name).desired_width(160.0)).changed() && valid_owner_name(&name) {
            app.session.author = name.trim().to_string();
        }
    });
    if name.trim().starts_with('@') {
        red(ui, tl!(NAME_NO_AT).to_string());
    }
    ui.data_mut(|d| d.insert_temp(name_id, name));
    if let Some(e) = hub.log_error() {
        red(ui, crate::i18n::fmt(tl!(LOG_NOT_SAVED), &[("error", &e)]));
    }
    if app.session.path.is_none() {
        ui.label(RichText::new(tl!(IN_MEMORY)).small().color(t.text_dim));
    }
    // Members.
    ui.label(RichText::new(tl!(MEMBERS)).font(semibold(12.0)));
    let members = hub.members();
    if members.is_empty() {
        ui.label(RichText::new(tl!(NO_MEMBERS)).small().color(t.text_dim));
    }
    for m in members {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&m.handle).font(semibold(12.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(tl!(REMOVE)).clicked() {
                    let _ = app.run("chat.remove", json!({"name": &m.handle}));
                }
            });
        });
    }
    // Invite: the line shows the name bound to the code (never the field's live text) and goes
    // away once the agent joined or the code expired.
    let (hid, lid) = (egui::Id::new("chat_invite_handle"), egui::Id::new("chat_invite_pending"));
    let mut handle = invite_field(ui);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut handle).desired_width(140.0));
        let ready = can_invite(&handle);
        if ui.add_enabled(ready, egui::Button::new(tl!(INVITE))).on_disabled_hover_text(tl!(NEED_NAME)).clicked() {
            match app.run("chat.invite", json!({"name": handle})) {
                Ok(v) => {
                    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                    let inv = PendingInvite { handle: s("handle"), code: s("code"), line: s("line") };
                    ui.data_mut(|d| {
                        d.insert_temp(lid, inv);
                        d.remove::<String>(err_id);
                    });
                }
                Err(e) => ui.data_mut(|d| {
                    d.insert_temp(err_id, failed(&e));
                }),
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(hid, handle));
    if let Some(e) = ui.data(|d| d.get_temp::<String>(err_id)) {
        red(ui, e);
    }
    match still_open(&hub, ui.data(|d| d.get_temp::<PendingInvite>(lid))) {
        Some(inv) => {
            ui.label(RichText::new(tl!(INVITE_HINT)).small().color(t.text_dim));
            egui::Frame::NONE.fill(t.input).corner_radius(6).inner_margin(6).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&inv.line).monospace().small()).wrap());
                if ui.small_button(tl!(COPY)).clicked() {
                    ui.ctx().copy_text(inv.line.clone());
                }
            });
        }
        None => ui.data_mut(|d| d.remove::<PendingInvite>(lid)),
    }
    ui.separator();
    // Messages.
    let owner = app.session.author.clone();
    let input_h = 56.0;
    egui::ScrollArea::vertical().stick_to_bottom(true).max_height((ui.available_height() - input_h).max(80.0)).show(ui, |ui| {
        for m in &hub.recent(500) {
            let (who, color) = match m.role {
                Role::Owner => (owner.clone(), t.accent),
                Role::Agent => (m.from.clone(), t.text),
                Role::System => (String::new(), t.text_dim),
            };
            ui.horizontal_wrapped(|ui| {
                if !who.is_empty() {
                    ui.label(RichText::new(format!("{who}:")).font(semibold(12.0)).color(color));
                }
                let text = RichText::new(&m.text);
                ui.label(if m.role == Role::System { text.italics().color(t.text_dim) } else { text });
            });
        }
        if hub.brake_on() {
            ui.label(RichText::new(tl!(BRAKE)).italics().color(t.text_dim));
        }
    });
    // Input: the only place in the window that makes owner messages. Shift+Enter is the box's
    // new line, so Enter sends wherever the text cursor is.
    let (iid, eid) = (egui::Id::new("chat_input"), egui::Id::new("chat_input_err"));
    let mut text = ui.data(|d| d.get_temp::<String>(iid)).unwrap_or_default();
    let r = ui.add(
        egui::TextEdit::multiline(&mut text)
            .id(input_id())
            .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .hint_text(tl!(INPUT_HINT)),
    );
    let enter = |e: &egui::Event| matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift);
    if r.has_focus() && ui.input(|i| i.events.iter().any(enter)) {
        match on_enter(&text) {
            (None, after) => text = after,
            (Some(msg), after) => match app.run("chat.post", json!({"text": msg})) {
                Ok(_) => {
                    text = after;
                    ui.data_mut(|d| d.remove::<String>(eid));
                }
                Err(e) => {
                    text = msg;
                    ui.data_mut(|d| d.insert_temp(eid, crate::i18n::fmt(tl!(NOT_SENT), &[("error", &e)])));
                }
            },
        }
    }
    if let Some(e) = ui.data(|d| d.get_temp::<String>(eid)) {
        red(ui, e);
    }
    ui.data_mut(|d| d.insert_temp(iid, text));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use wordcraft_chat::testing::{FakePort, TestEnv};
    use wordcraft_chat::{Chat, Hub, Role};
    use wordcraft_engine::Session;

    use super::*;

    fn app_with_chat() -> WordApp {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        a.session.chat = Some(Arc::new(Chat::new(Hub::new(TestEnv::at(1_000)), FakePort::closed())));
        a
    }

    /// A context with the app's fonts (the pane uses the `semibold` family).
    fn ctx_with_fonts() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx
    }

    #[test]
    fn pane_strings_are_translated() {
        assert_eq!(PANE_STRINGS.len(), 21);
        for l in crate::i18n::Lang::all().filter(|l| *l != crate::i18n::Lang::EN) {
            for s in PANE_STRINGS {
                assert!(crate::i18n::has(l, s), "{}: {s:?}", l.code());
            }
        }
    }

    #[test]
    fn the_review_tab_button_opens_and_closes_the_pane() {
        let mut a = app_with_chat();
        a.run("chat.open", json!({"value": true})).unwrap();
        assert!(a.ui.chat_pane);
        a.run("chat.open", json!({"value": false})).unwrap();
        assert!(!a.ui.chat_pane);
        let legacy: crate::UiState = serde_json::from_str(r#"{"tab":"Home"}"#).unwrap();
        assert!(!legacy.chat_pane, "closed by default");
    }

    #[test]
    fn the_pane_draws_every_state() {
        let ctx = ctx_with_fonts();
        let frame = |a: &mut WordApp| {
            for _ in 0..2 {
                ctx.run_ui(egui::RawInput::default(), |ui| show(a, ui)).drop_without_applying_deltas();
            }
        };
        let mut none = WordApp::new(Session::new(wordcraft_doc::Document::new()), Default::default());
        none.ui.chat_pane = true;
        frame(&mut none);
        let mut a = app_with_chat();
        a.ui.chat_pane = true;
        frame(&mut a);
        a.run("chat.start", json!({})).unwrap();
        let inv = a.run("chat.invite", json!({"name": "@claude"})).unwrap();
        let hub = a.session.chat.as_ref().unwrap().hub().clone();
        hub.join(inv["code"].as_str().unwrap()).unwrap();
        for i in 0..600 {
            let _ = hub.post_agent("@claude", &format!("line {i} \u{202e}rtl ✅ {}", "x".repeat(i)));
            let _ = hub.post_owner("go on");
        }
        frame(&mut a);
        a.run("chat.stop", json!({})).unwrap();
        frame(&mut a);
    }

    fn run_frames(ctx: &egui::Context, a: &mut WordApp) {
        for _ in 0..2 {
            ctx.run_ui(egui::RawInput::default(), |ui| show(a, ui)).drop_without_applying_deltas();
        }
    }

    #[test]
    fn a_fresh_invite_field_holds_the_default_name() {
        let ctx = ctx_with_fonts();
        let mut a = app_with_chat();
        a.ui.chat_pane = true;
        a.run("chat.start", json!({})).unwrap();
        run_frames(&ctx, &mut a);
        let got = ctx.data(|d| d.get_temp::<String>(egui::Id::new("chat_invite_handle")));
        assert_eq!(got.as_deref(), Some("@claude"));
    }

    #[test]
    fn an_empty_invite_field_cannot_invite() {
        assert!(can_invite("@claude"));
        assert!(!can_invite(""));
        assert!(!can_invite("   "));
        let ctx = ctx_with_fonts();
        let mut a = app_with_chat();
        a.ui.chat_pane = true;
        a.run("chat.start", json!({})).unwrap();
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("chat_invite_handle"), String::new()));
        run_frames(&ctx, &mut a);
        let hub = a.session.chat.as_ref().unwrap().hub().clone();
        assert!(ctx.data(|d| d.get_temp::<PendingInvite>(egui::Id::new("chat_invite_pending"))).is_none());
        assert!(hub.invite("  ").is_err(), "the hub itself still refuses a blank name");
        assert_eq!(ctx.data(|d| d.get_temp::<String>(egui::Id::new("chat_invite_handle"))).as_deref(), Some(""), "stays empty, not refilled");
    }

    #[test]
    fn invite_box_shows_the_bound_handle_until_used_or_expired() {
        let env = TestEnv::at(1_000);
        let hub = Hub::new(env.clone());
        hub.set_open(true);
        let (h, code) = hub.invite("@test").unwrap();
        let line = wordcraft_chat::invite_line(wordcraft_chat::DEFAULT_CLIENT_COMMAND, "127.0.0.1:7981", &code, "@test");
        let inv = PendingInvite { handle: h, code: code.clone(), line };
        assert!(still_open(&hub, Some(inv.clone())).is_some());
        env.advance(wordcraft_chat::hub::INVITE_TTL_MS + 1);
        assert!(still_open(&hub, Some(inv.clone())).is_none(), "expired");
        let (_, code2) = hub.invite("@test2").unwrap();
        hub.join(&code2).unwrap();
        assert!(still_open(&hub, Some(PendingInvite { code: code2, ..inv })).is_none(), "used");
    }

    #[test]
    fn enter_handling() {
        assert_eq!(on_enter(""), (None, String::new()));
        assert_eq!(on_enter("\n"), (None, String::new()));
        assert_eq!(on_enter("hello\n"), (Some("hello".into()), String::new()));
        assert_eq!(on_enter("a\nb\n"), (Some("a\nb".into()), String::new()));
    }

    #[test]
    fn owner_name_never_starts_with_at() {
        assert!(valid_owner_name("Owner") && valid_owner_name("  Owner "));
        assert!(!valid_owner_name("@claude") && !valid_owner_name("  @claude") && !valid_owner_name("") && !valid_owner_name("   "));
    }

    /// The name field shows the session's user name, except while the owner is typing one.
    #[test]
    fn owner_name_field_follows_the_user_name() {
        assert_eq!(owner_field(None, "Owner"), "Owner");
        assert_eq!(owner_field(Some("Ada ".into()), "Ada"), "Ada ", "typing goes on (a space before the last name)");
        assert_eq!(owner_field(Some("@x".into()), "Owner"), "@x", "a refused name stays in the field to be fixed");
        assert_eq!(owner_field(Some(String::new()), "Owner"), "", "an emptied field stays empty while typing");
        assert_eq!(owner_field(Some("Ada".into()), "Grace"), "Grace", "a name set in File › Options shows here too");
    }

    /// Enter sends wherever the text cursor is; Shift+Enter makes a new line.
    #[test]
    fn enter_sends_and_shift_enter_makes_a_new_line() {
        let ctx = ctx_with_fonts();
        let mut a = app_with_chat();
        a.ui.chat_pane = true;
        a.run("chat.start", json!({})).unwrap();
        let key = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let enter = |modifiers| key(egui::Key::Enter, modifiers);
        let frame = |a: &mut WordApp, events: Vec<egui::Event>| {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(a, ui)).drop_without_applying_deltas();
        };
        frame(&mut a, Vec::new());
        ctx.memory_mut(|m| m.request_focus(input_id()));
        frame(&mut a, vec![egui::Event::Text("hello".into())]);
        frame(&mut a, vec![enter(egui::Modifiers::SHIFT)]);
        frame(&mut a, vec![egui::Event::Text("world".into())]);
        let hub = a.session.chat.as_ref().unwrap().hub().clone();
        let owner = || hub.recent(10).into_iter().filter(|m| m.role == Role::Owner).map(|m| m.text).collect::<Vec<_>>();
        assert!(owner().is_empty(), "Shift+Enter does not send");
        frame(&mut a, vec![key(egui::Key::ArrowLeft, egui::Modifiers::NONE), enter(egui::Modifiers::NONE)]);
        assert_eq!(owner(), vec!["hello\nworld"], "Enter in the middle of the text sends it as typed");
        frame(&mut a, vec![enter(egui::Modifiers::NONE)]);
        assert_eq!(owner().len(), 1, "the box is empty after a send");
    }

    /// Chat commands leave the owner's view of the document where it is.
    #[test]
    fn chat_commands_do_not_scroll_the_document() {
        let mut a = app_with_chat();
        a.canvas.scroll_to_caret = false;
        a.run("chat.open", json!({"value": true})).unwrap();
        a.run("chat.start", json!({})).unwrap();
        a.run("chat.post", json!({"text": "hello"})).unwrap();
        assert!(!a.canvas.scroll_to_caret);
    }
}
