//! The control channel's `doc`: methods addressed to an open document that isn't the active one.

use serde_json::{Value, json};
use wordcraft_engine::Session;

use super::{ControlRequest, Outcome, handle};
use crate::{Services, WordApp};

fn app() -> WordApp {
    WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
}

/// Send one request the way the control channel does and return the reply.
fn send(a: &mut WordApp, method: &str, params: Value) -> Value {
    let ctx = egui::Context::default();
    let (req, _reply) = ControlRequest::new(method, params);
    match handle(a, &ctx, &req) {
        Outcome::Done(v) => v,
        Outcome::Screenshot { .. } => json!({"ok": false, "error": "not a reply"}),
    }
}

fn exec(a: &mut WordApp, command: &str, params: Value) -> Value {
    send(a, "engine.execute", json!({"command": command, "params": params}))
}

fn body(a: &WordApp) -> String {
    a.session.doc.plain_text(wordcraft_doc::StoryRef::Body)
}

/// Two documents, the first typed in and parked behind the second: (first, second).
fn two_documents(a: &mut WordApp) -> (u64, u64) {
    let first = a.session.document_id();
    assert_eq!(exec(a, "text.insert", json!({"text": "First"}))["ok"], true);
    assert_eq!(exec(a, "file.new", json!({}))["ok"], true);
    let second = a.session.document_id();
    assert_ne!(first, second);
    assert_eq!(exec(a, "text.insert", json!({"text": "Second"}))["ok"], true);
    (first, second)
}

/// W1: `doc` sends a command, an inspection or a render to a document in the background; the
/// window's document stays active and unchanged, and an id that names no open document is an
/// error.
#[test]
fn methods_address_an_open_document_by_id() {
    let mut a = app();
    let (first, second) = two_documents(&mut a);

    let r = send(&mut a, "engine.execute", json!({"command": "text.insert", "params": {"text": " more"}, "doc": first}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(a.session.document_id(), second);
    assert_eq!(body(&a).trim(), "Second");
    let r = send(&mut a, "document.inspect", json!({"doc": first}));
    assert_eq!(r["result"]["blocks"][0]["text"], "First more", "{r}");
    let r = send(&mut a, "ui.render", json!({"doc": first, "scale": 0.2}));
    assert!(r["result"]["png"].as_str().is_some_and(|p| !p.is_empty()), "{r}");
    // The active document's own id works like no `doc` at all.
    let r = send(&mut a, "engine.execute", json!({"command": "text.insert", "params": {"text": "!"}, "doc": second}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(body(&a).trim(), "Second!");

    let r = send(&mut a, "ui.inspect", json!({}));
    assert_eq!(r["result"]["document"], second, "{r}");
    let ids: Vec<u64> = r["result"]["documents"].as_array().into_iter().flatten().filter_map(|d| d["id"].as_u64()).collect();
    assert_eq!(ids, vec![first, second]);

    // Window commands need the document on screen.
    let r = send(&mut a, "engine.execute", json!({"command": "ui.tab", "params": {"value": "insert"}, "doc": first}));
    assert_eq!(r["ok"], false, "{r}");

    for bad in [json!(u64::MAX), json!(-1), json!("1"), json!(first.to_string()), json!(1.5), json!(1e300), json!({}), json!([first])] {
        for (method, params) in [
            ("engine.execute", json!({"command": "text.insert", "params": {"text": "x"}, "doc": bad})),
            ("document.inspect", json!({"doc": bad})),
            ("ui.render", json!({"doc": bad})),
        ] {
            let r = send(&mut a, method, params);
            assert_eq!(r["ok"], false, "{method} with doc {bad}: {r}");
        }
    }
    assert_eq!(a.session.document_id(), second);
    assert_eq!(body(&a).trim(), "Second!");
    let r = send(&mut a, "document.inspect", json!({"doc": first}));
    assert_eq!(r["result"]["blocks"][0]["text"], "First more", "{r}");
}

/// W1: `file.close` addressed to a document in the background refuses its unsaved changes
/// unless forced (only a boolean true), as it does for the window's document (#1268), and closes
/// only that document.
#[test]
fn closing_an_addressed_document_keeps_unsaved_work_unless_forced() {
    let mut a = app();
    let (first, second) = two_documents(&mut a);

    for params in [json!({}), json!({"force": "true"}), json!({"force": 1})] {
        let r = send(&mut a, "engine.execute", json!({"command": "file.close", "params": params, "doc": first}));
        assert_eq!(r["ok"], false, "{r}");
        let e = r["error"].as_str().unwrap_or("");
        assert!(e.contains("unsaved") && e.contains("force"), "{e}");
        assert!(a.session.is_open(first));
    }

    let r = send(&mut a, "engine.execute", json!({"command": "file.close", "params": {"force": true}, "doc": first}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(!a.session.is_open(first));
    assert_eq!(a.session.document_id(), second);
    assert!(a.has_document() && !a.quit_requested);
    assert_eq!(body(&a).trim(), "Second");
    let r = send(&mut a, "engine.execute", json!({"command": "text.insert", "params": {"text": "x"}, "doc": first}));
    assert_eq!(r["ok"], false, "a closed document can't be addressed: {r}");

    // A document without unsaved changes closes at once.
    assert_eq!(exec(&mut a, "file.new", json!({}))["ok"], true);
    let clean = a.session.document_id();
    assert_eq!(exec(&mut a, "file.new", json!({}))["ok"], true);
    let active = a.session.document_id();
    let r = send(&mut a, "engine.execute", json!({"command": "file.close", "doc": clean}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(!a.session.is_open(clean) && a.session.is_open(second));
    assert_eq!(a.session.document_id(), active);
}
