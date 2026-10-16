//! craft-live mode compiled for the browser (`wasm32-unknown-unknown`), run with
//! wasm-bindgen-test (Node or a browser): the framing, the fragment, and a session that seeds a
//! room from a file, with the engine's co-editing in wasm (random client ids from the browser's
//! `crypto.getRandomValues`). The WebSocket, `fetch` and `postMessage` glue needs a real page.
#![cfg(target_arch = "wasm32")]

use serde_json::json;
use wasm_bindgen_test::wasm_bindgen_test;
use wordcraft_engine::Session;
use wordcraft_engine::doc::Document;
use wordcraft_web::live::{Action, Config, Event, Frame, Live, Phase};

#[wasm_bindgen_test]
fn frames_and_the_fragment() {
    let f = Frame::Control(json!({"type": "hello", "clientId": 7}));
    assert_eq!(Frame::decode(&f.encode()), Some(f));
    assert_eq!(Frame::Sync1(vec![7]).encode(), vec![0, 0, 1, 7]);
    let c = Config::from_fragment("#craft-live=1&ws=wss%3A%2F%2Fh%2Fws%2Fr&room=r&token=t&role=edit&seed=0").unwrap().unwrap();
    assert_eq!(c.socket_url(), "wss://h/ws/r?token=t");
    assert!(!c.view && !c.seed);
}

#[wasm_bindgen_test]
fn a_session_seeds_from_the_file() {
    let bytes = {
        let mut s = Session::new(Document::new());
        s.run("text.insert", &json!({"text": "From Nextcloud"})).unwrap();
        wordcraft_engine::io::save_bytes("A.docx", &s.doc).unwrap()
    };
    let cfg = Config::from_json(&json!({
        "ws": "wss://h/ws/r", "room": "r", "token": "t", "role": "edit", "seed": true,
        "file": "https://h/api/v1/rooms/r/file", "name": "A.docx", "format": "docx", "api": "https://h/api/v1",
        "user": {"id": "ada", "name": "Ada", "color": "#3B7DD8"},
    }))
    .unwrap();
    let mut s = Session::new(Document::new());
    let mut live = Live::new(cfg);
    live.start(&mut s, 1);
    assert_eq!(live.phase, Phase::Fetching);
    assert!(live.take_actions().iter().any(|a| matches!(a, Action::Fetch { .. })));
    live.handle(&mut s, Event::File(Ok(bytes)));
    assert_eq!(live.phase, Phase::Live);
    let actions = live.take_actions();
    assert!(actions.iter().any(|a| matches!(a, Action::Connect(u) if u == "wss://h/ws/r?token=t")));
    assert!(actions.iter().any(|a| matches!(a, Action::Shell(m) if m["type"] == "craft-live:loaded")));
    // The socket opens: hello with the Yjs client id, frames held until `welcome`.
    live.handle(&mut s, Event::Open);
    let hello = live.take_actions();
    let client = s.active_room().map(|r| r.client_id()).unwrap();
    assert!(client > 0);
    assert!(hello.iter().any(|a| matches!(a, Action::Send(b) if Frame::decode(b) == Some(Frame::Control(json!({"type": "hello", "clientId": client, "can": ["upload"], "client": format!("wordcraft-web/{}", env!("CARGO_PKG_VERSION"))}))))));
    let welcome = Frame::Control(json!({"type": "welcome", "seed": true, "seeded": false, "modified": false})).encode();
    live.handle_all(&mut s, vec![Event::Frame(welcome), Event::Frame(Frame::Sync1(vec![0]).encode())], 2);
    // The server's sync step 1 is answered with the whole document.
    let sent: Vec<Frame> = live.take_actions().into_iter().filter_map(|a| if let Action::Send(b) = a { Frame::decode(&b) } else { None }).collect();
    assert!(sent.iter().any(|f| matches!(f, Frame::Sync2(u) if u.len() > 100)), "{sent:?}");
    assert!(s.run("chat.send", &json!({"text": "hi"})).is_ok(), "chat ids need random numbers in wasm too");
}
