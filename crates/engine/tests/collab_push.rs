//! Protocol 2 end to end through a real plugin process: the push channel and co-editing. The
//! transport is this test binary started again with `--serve-transport` (no libtest harness, so
//! standard output is the protocol's alone). It relays between two rooms, `relay:a` and
//! `relay:b`, as a network would between two people: what the app sends for one arrives at the
//! other, with `from`. Two sessions in this process each join one room.
//!
//! Checked: `initialize` with `push`/`collab` and `host.collab`; a plugin command answering
//! `collab.join`; notifications the transport sends on its own (status, relayed sync and
//! updates) reaching each session's loop, even in the middle of a request; the transport's own
//! requests (`collab/snapshot` with the `.docx`, `host/log`) answered; the joiner getting the
//! document; typing both ways in one paragraph; per-user undo travelling; leaving.
#![allow(clippy::disallowed_methods)] // the clock, off wasm

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};
use wordcraft_engine::Session;
use wordcraft_engine::collab;
use wordcraft_engine::doc::{Document, Path, StoryRef};
use wordcraft_plugin::push::{self, Incoming};

const PLUGIN: &str = "org.wordcraft.test-relay";

fn main() {
    if std::env::args().any(|a| a == "--serve-transport") {
        serve();
        return;
    }
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-relay-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let manifest = json!({
        "id": PLUGIN, "name": "Relay", "run": {"command": exe.display().to_string(), "args": ["--serve-transport"]},
        "commands": [{"id": "share", "label": "Share"}],
        "collab": {"transport": "relay", "label": "Live (test relay)"},
    });
    std::fs::write(dir.join("plugin.json"), manifest.to_string()).unwrap();
    wordcraft_plugin::load(&dir).unwrap();
    two_sessions_edit_through_the_transport();
    println!("test two_sessions_edit_through_the_transport ... ok");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- the transport

fn serve() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let mut send = |m: Json| {
        let _ = writeln!(out, "{m}");
        let _ = out.flush();
    };
    let mut clients: HashMap<String, u64> = HashMap::new();
    let mut asked_snapshot = false;
    let mut init = Json::Null;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Json>(&line) else { continue };
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let p = msg["params"].clone();
        if let Some(id) = msg.get("id").filter(|_| !method.is_empty()) {
            // The app's requests.
            let result = match method.as_str() {
                "initialize" => {
                    init = p["host"].clone();
                    json!({"protocolVersion": 1})
                }
                "commands/run" => {
                    json!({"collab": {"join": {"room": "relay:a", "mode": "share", "user": {"id": "u-a", "name": "Ann Lee", "color": "#3B7DD8"}, "invitation": "craft+relay:a"}}, "result": {"init": init}})
                }
                _ => json!({}),
            };
            // A notification of our own in the middle of a request: it must reach the app's loop.
            if method == "commands/run" {
                send(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "relay:none", "state": "online"}}));
            }
            send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
            continue;
        }
        if method.is_empty() {
            // An answer to one of our requests.
            if msg["id"] == "snap" {
                let r = &msg["result"];
                let ok = r["update"].as_str().is_some_and(|u| !u.is_empty())
                    && r["file"].as_str().is_some_and(|f| f.len() > 100)
                    && r["name"].as_str().is_some_and(|n| n.ends_with(".docx"));
                send(
                    json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "relay:a", "state": "online", "message": format!("snapshot ok: {ok}")}}),
                );
            }
            continue;
        }
        if method == "shutdown" {
            break;
        }
        let room = p["room"].as_str().unwrap_or("").to_string();
        let other = match room.as_str() {
            "relay:a" => "relay:b",
            "relay:b" => "relay:a",
            _ => continue,
        };
        match method.as_str() {
            "collab/joined" => {
                clients.insert(room.clone(), p["clientId"].as_u64().unwrap_or(0));
                send(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": room, "state": "connecting"}}));
                send(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": room, "state": "online", "peers": []}}));
                send(json!({"jsonrpc": "2.0", "id": 7, "method": "host/log", "params": {"text": format!("joined {room}")}}));
            }
            "collab/sync1" | "collab/sync2" | "collab/update" | "collab/awareness" => {
                let mut q = p.clone();
                q["room"] = json!(other);
                q["from"] = json!(clients.get(&room).copied().unwrap_or(0));
                if method == "collab/sync2" {
                    q.as_object_mut().unwrap().remove("to");
                }
                send(json!({"jsonrpc": "2.0", "method": method, "params": q}));
                if method == "collab/update" && room == "relay:a" && !asked_snapshot {
                    asked_snapshot = true;
                    send(json!({"jsonrpc": "2.0", "id": "snap", "method": "collab/snapshot", "params": {"room": "relay:a", "file": true}}));
                }
            }
            "collab/leave" => {
                send(
                    json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": other, "state": "online", "peers": [], "message": "the other left"}}),
                );
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- the test

/// Hands each session the messages for its room.
fn pump(a: &mut Session, b: &mut Session, now: u64) {
    let mut for_a = Vec::new();
    let mut for_b = Vec::new();
    for p in push::take() {
        let room = match &p.incoming {
            Incoming::Message(m) => m["params"]["room"].as_str().unwrap_or("relay:a").to_string(),
            _ => "relay:a".into(),
        };
        if room == "relay:b" { for_b.push(p) } else { for_a.push(p) }
    }
    a.collab_poll_pushed(now, for_a);
    b.collab_poll_pushed(now, for_b);
}

fn wait(a: &mut Session, b: &mut Session, what: &str, mut done: impl FnMut(&Session, &Session) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        let now = start.elapsed().as_millis() as u64 + 1_000;
        pump(a, b, now);
        if done(a, b) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}: A {:?} / B {:?}", text(a, 0), text(b, 0));
}

fn text(s: &Session, i: usize) -> String {
    s.doc.para(StoryRef::Body, &Path::top(i)).map(|p| p.text.clone()).unwrap_or_default()
}

fn caret(s: &mut Session, block: usize, off: usize) {
    s.run("caret.set", &json!({"pos": {"story": "body", "path": [block], "off": off}})).unwrap();
}

fn two_sessions_edit_through_the_transport() {
    let mut a = Session::new(Document::new());
    a.author = "Ann Lee".into();
    a.run("text.insert", &json!({"text": "The fox jumps."})).unwrap();
    // A plugin command answers `collab.join`: returned to the caller, who joins.
    let r = a.run("plugins.run", &json!({"plugin": PLUGIN, "command": "share"})).unwrap();
    let host = &r["result"]["init"];
    assert_eq!(host["features"], json!(["dialogs", "open", "events", "document", "push", "collab", "multiline", "literalValues"]), "{r}");
    assert_eq!(host["collab"], json!({"formats": ["yjs-v1"], "app": "wordcraft", "schema": 1, "encryption": ["aes-256-gcm"]}));
    let mut join = r["collab"]["join"].clone();
    assert_eq!(join["room"], "relay:a", "{r}");
    join["plugin"] = json!(PLUGIN);
    a.run("collab.join", &join).unwrap();
    assert_eq!(a.active_room().and_then(|r| r.invitation.clone()).as_deref(), Some("craft+relay:a"));
    let mut b = Session::new(Document::new());
    b.author = "Bob".into();
    b.run("collab.join", &json!({"plugin": PLUGIN, "room": "relay:b", "mode": "join", "user": {"id": "u-b", "name": "Bob", "color": "#D8513B"}}))
        .unwrap();
    assert_eq!(collab::read_only(&b).as_deref(), Some("Joining…"));
    assert!(b.run("text.insert", &json!({"text": "too early"})).is_err(), "read-only while joining");

    wait(&mut a, &mut b, "the joiner to get the document", |_, b| collab::read_only(b).is_none() && text(b, 0) == "The fox jumps.");

    // Typing both ways, in the same paragraph.
    caret(&mut a, 0, 4);
    a.run("text.insert", &json!({"text": "quick "})).unwrap();
    caret(&mut b, 0, 13);
    b.run("text.insert", &json!({"text": " high"})).unwrap();
    b.run("text.newParagraph", &json!({})).unwrap();
    b.run("text.insert", &json!({"text": "Second"})).unwrap();
    wait(&mut a, &mut b, "the edits to cross", |a, b| {
        text(a, 0) == "The quick fox jumps high" && text(b, 0) == text(a, 0) && text(a, 1) == "Second." && text(b, 1) == "Second."
    });
    // The transport asked for a snapshot with the file; the answer was complete.
    wait(&mut a, &mut b, "the snapshot answer", |a, _| a.active_room().and_then(|r| r.message.clone()).as_deref() == Some("snapshot ok: true"));
    // Awareness: each sees the other, and where their caret is.
    wait(&mut a, &mut b, "awareness", |a, b| {
        a.active_room().is_some_and(|r| r.peers().iter().any(|p| p.user.name == "Bob" && p.focus.is_some()))
            && b.active_room().is_some_and(|r| r.peers().iter().any(|p| p.user.name == "Ann Lee"))
    });

    // Per-user undo: Bob's undo takes back only his last typing.
    b.run("edit.undo", &json!({})).unwrap();
    wait(&mut a, &mut b, "the undo", |a, _| text(a, 1) == "." && text(a, 0) == "The quick fox jumps high");
    assert_eq!(a.collab_fingerprint(), b.collab_fingerprint());

    // Leaving: the document stays, edits stay local.
    a.run("collab.leave", &json!({})).unwrap();
    wait(&mut a, &mut b, "the leave", |_, b| b.active_room().and_then(|r| r.message.clone()).as_deref() == Some("the other left"));
    caret(&mut a, 0, 0);
    a.run("text.insert", &json!({"text": "Alone: "})).unwrap();
    for _ in 0..20 {
        pump(&mut a, &mut b, 99_000);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(text(&a, 0).starts_with("Alone: "));
    assert!(!text(&b, 0).starts_with("Alone"));
}
