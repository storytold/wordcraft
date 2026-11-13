//! The document chat in the interface: the title bar's chat button appearing once the document
//! has a chat (outside a live session too), the panel opening from it, a message typed in its
//! box and sent with Enter (Shift+Enter keeps writing), and, in a live session, another person's
//! messages raising the unread count while the panel is closed and a mention of you flashing
//! the button. The transport is served in memory by the test; the other person is a second
//! engine session it relays to.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_plugin::push::{Incoming, Pushed};
use wordcraft_ui_egui::{Services, WordApp};

const TRANSPORT: &str = "org.wordcraft.fake-chat-transport";

fn harness(app: WordApp) -> Harness<'static, WordApp> {
    Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    )
}

fn deliver(s: &mut Session, room: &str, msgs: Vec<Value>, now: u64) {
    let pushed = msgs
        .into_iter()
        .filter(|m| !matches!(m["method"].as_str(), Some("collab/joined" | "collab/leave") | None))
        .map(|mut m| {
            m["params"]["room"] = json!(room);
            Pushed { plugin: TRANSPORT.into(), incoming: Incoming::Message(m) }
        })
        .collect();
    s.collab_poll_pushed(now, pushed);
}

fn exchange(h: &mut Harness<'static, WordApp>, b: &mut Session, now: &mut u64) {
    for _ in 0..4 {
        *now += 100;
        let from_a = h.state_mut().session.collab_take_outgoing();
        deliver(b, "fake:b", from_a, *now);
        let from_b = b.collab_take_outgoing();
        deliver(&mut h.state_mut().session, "fake:a", from_b, *now);
        h.run_steps(1);
    }
}

#[test]
fn chat_button_panel_and_unread_count() {
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.session.author = "Ann Lee".into();
    a.session.run("text.insert", &json!({"text": "Shared words"})).unwrap();
    let mut h = harness(a);
    h.run_steps(3);
    assert!(h.query_by_label("Chat").is_none(), "no chat button for a document without a chat, outside a session");

    // A note left in the file: the button appears; it opens the panel.
    h.state_mut().run("chat.send", json!({"text": "A note in the file"})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().collab.chat.unread, 0, "your own messages aren't unread");
    h.get_by_label("Chat").click();
    h.run_steps(3);
    assert!(h.state().session.view.chat_pane, "the panel opened");
    assert!(h.query_by_label("A note in the file").is_some(), "the message is shown");
    assert!(h.query_by_label("Ann Lee").is_some(), "with its author");

    // Typing in the box: Shift+Enter is a new line, Enter sends.
    h.get_by_label("Write a message").click();
    h.run_steps(2);
    h.get_by_label("Write a message").type_text("Second");
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
    h.run_steps(1);
    h.get_by_label("Write a message").type_text("line");
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let chat = h.state().session.doc.chat.clone();
    assert_eq!(chat.messages.len(), 2, "{chat:?}");
    assert_eq!(chat.messages[1].text(), "Second\nline");
    assert!(h.state().collab.chat.draft.is_empty(), "the box is empty again");
    // The menu: Export, Clear chat…, and the note about Office.
    h.get_by_label("Chat options").click();
    h.run_steps(3);
    assert!(h.query_by_label("Export").is_some());
    assert!(h.query_by_label("Kept in the file; Office drops it when it saves").is_some());
    h.get_by_label("Clear chat…").click();
    h.run_steps(3);
    assert!(h.query_by_label("Remove the whole chat from the document for everyone?").is_some(), "it asks first");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().session.doc.chat.messages.len(), 2, "cancelled: nothing removed");

    // Close the panel, start a session, and Bob joins.
    h.state_mut().run("view.chat", json!({"value": false})).unwrap();
    h.state_mut().session.collab_serve_in_memory(TRANSPORT);
    h.state_mut()
        .run(
            "collab.join",
            json!({"plugin": TRANSPORT, "room": "fake:a", "mode": "share", "user": {"id": "u-ann", "name": "Ann Lee", "color": "#3B7DD8"}}),
        )
        .unwrap();
    let mut b = Session::new(wordcraft_doc::Document::new());
    b.collab_serve_in_memory(TRANSPORT);
    b.run("collab.join", &json!({"plugin": TRANSPORT, "room": "fake:b", "mode": "join", "user": {"id": "u-bob", "name": "Bob", "color": "#D8513B"}}))
        .unwrap();
    let mut now = wordcraft_ui_egui::now_ms() as u64;
    exchange(&mut h, &mut b, &mut now);
    assert_eq!(b.doc.chat.messages.len(), 2, "Bob got the document's chat");
    assert_eq!(h.state().collab.chat.unread, 0);

    // Bob writes twice, once mentioning Ann: the count rises, the button flashes.
    b.run("chat.send", &json!({"text": "Hi there"})).unwrap();
    b.run("chat.send", &json!({"text": "@Ann Lee can you check page 2?"})).unwrap();
    exchange(&mut h, &mut b, &mut now);
    h.run_steps(2);
    assert_eq!(h.state().collab.chat.unread, 2);
    assert!(h.state().collab.chat.flash_until > wordcraft_ui_egui::now_ms(), "a mention flashes the button");
    assert!(h.query_by_label("Chat, 2 unread").is_some(), "the button shows the count");
    // Bob is typing: shown in the panel.
    b.run("chat.typing", &json!({"typing": true})).unwrap();
    exchange(&mut h, &mut b, &mut now);
    h.get_by_label("Chat, 2 unread").click();
    h.run_steps(3);
    assert_eq!(h.state().collab.chat.unread, 0, "opening the panel reads them");
    assert!(h.query_by_label("Chat, 2 unread").is_none(), "the count is gone");
    assert!(h.query_by_label("Bob is typing…").is_some(), "who's typing");
    assert!(h.query_by_label("Hi there").is_some());
}
