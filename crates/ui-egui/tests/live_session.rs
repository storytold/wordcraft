//! A live session in the interface: the title bar's Share button (explaining what's needed when no
//! transport is loaded, opening the Share dialog when one is), its state once in a session, and
//! the people's circles appearing as others join. The transport is a fake one served in memory
//! by the test (`collab_serve_in_memory`); the other person is a second engine session it
//! relays to. Its own test binary: add-ins are process-wide.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_engine::collab::RoomState;
use wordcraft_plugin::push::{Incoming, Pushed};
use wordcraft_ui_egui::{Services, WordApp};

const TRANSPORT: &str = "org.wordcraft.fake-transport";

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

#[test]
fn share_button_session_state_and_people() {
    // Without a transport, Share exports a copy (File › Export).
    let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    a.session.author = "Ann Lee".into();
    a.session.run("text.insert", &json!({"text": "Shared words"})).unwrap();
    let mut h = harness(a);
    h.run_steps(3);
    h.get_by_label("Share").click();
    h.run_steps(3);
    assert!(h.state().ui.backstage && h.state().ui.backstage_page == "export", "Share without a transport: Export");
    h.state_mut().ui.backstage = false;
    h.run_steps(2);

    // A transport add-in (served in memory here): Share opens the Share dialog.
    let manifest = json!({
        "id": TRANSPORT, "name": "Fake transport", "run": {"command": "./not-started"},
        "commands": [{"id": "share", "label": "Share"}, {"id": "join", "label": "Join"}],
        "collab": {"transport": "fake", "label": "Live (fake)"},
    });
    wordcraft_plugin::add(wordcraft_plugin::Manifest::parse(&manifest.to_string()).unwrap(), std::env::temp_dir());
    h.state_mut().session.collab_serve_in_memory(TRANSPORT);
    h.get_by_label("Share").click();
    h.run_steps(3);
    assert!(matches!(h.state().dialog, Some(wordcraft_ui_egui::dialogs::Dialog::Share(_))), "the Share dialog opened");
    assert!(h.query_by_label("Start a live session").is_some());
    h.state_mut().dialog = None;
    h.run_steps(2);

    // Start a session (as the transport's `share` answer would), and a second person joins.
    h.state_mut()
        .run(
            "collab.join",
            json!({"plugin": TRANSPORT, "room": "fake:a", "mode": "share", "user": {"id": "u-ann", "name": "Ann Lee", "color": "#3B7DD8"}, "invitation": "craft+fake:a"}),
        )
        .unwrap();
    let mut b = Session::new(wordcraft_doc::Document::new());
    b.collab_serve_in_memory(TRANSPORT);
    b.run("collab.join", &json!({"plugin": TRANSPORT, "room": "fake:b", "mode": "join", "user": {"id": "u-bob", "name": "Bob", "color": "#D8513B"}}))
        .unwrap();
    // The interface's frames poll with the wall clock: the test's messages use it too.
    let mut now = wordcraft_ui_egui::now_ms() as u64;
    for _ in 0..6 {
        now += 100;
        let from_a = h.state_mut().session.collab_take_outgoing();
        deliver(&mut b, "fake:b", from_a, now);
        let from_b = b.collab_take_outgoing();
        deliver(&mut h.state_mut().session, "fake:a", from_b, now);
        h.run_steps(1);
    }
    let status = json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"state": "online", "peers": [{"user": {"id": "u-bob", "name": "Bob", "color": "#D8513B"}}]}});
    deliver(&mut h.state_mut().session, "fake:a", vec![status], now + 10);
    // Bob puts his caret somewhere: his awareness reaches Ann.
    b.run("caret.set", &json!({"pos": {"story": "body", "path": [0], "off": 6}})).unwrap();
    b.collab_poll_pushed(now + 100, vec![]);
    let from_b = b.collab_take_outgoing();
    deliver(&mut h.state_mut().session, "fake:a", from_b, now + 120);
    h.run_steps(3);

    let room = h.state().session.active_room().expect("in a room");
    assert_eq!(room.state, RoomState::Online, "the button shows the session online");
    assert_eq!(b.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).map(|p| p.text.clone()).as_deref(), Some("Shared words"));
    let peers = room.peers();
    assert!(peers.iter().any(|p| p.user.name == "Bob" && p.focus.is_some()), "Bob and his caret: {peers:?}");
    // The circles: Bob's and Ann's own.
    assert!(h.query_by_label("Bob").is_some(), "Bob's circle is drawn");
    assert!(h.query_by_label("Ann Lee").is_some(), "Ann's own circle is drawn");
    assert!(h.query_by_label("Share").is_some(), "the Share button stays");
    // The session popover: Copy Invitation and Leave.
    h.get_by_label("Share").click();
    h.run_steps(3);
    assert!(h.query_by_label("Copy Invitation").is_some());
    h.get_by_label("Leave Live Session").click();
    h.run_steps(3);
    assert!(h.state().session.active_room().is_none(), "left the session");
    assert!(h.query_by_label("Bob").is_none(), "the circles go with it");
}
