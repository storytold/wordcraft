//! Document chat (craft-collab `docs/chat.md`): the `chat.*` commands outside and inside a live
//! session. Outside one the chat is notes left in the document; in a room it's shared: sessions
//! chatting concurrently (2 and 3 people, with edits, replies, deletes and text edits mixed in,
//! updates delivered in random order and sometimes twice) end with the same chat, which a
//! newcomer gets too; sharing seeds the room from the file's chat; clearing reaches everyone;
//! an encrypted room carries the chat encrypted; chat changes never enter undo but mark the
//! document modified; awareness carries who's typing.

use std::sync::Arc;

use serde_json::{Value as Json, json};
use wordcraft_engine::Session;
use wordcraft_engine::collab::IN_MEMORY;
use wordcraft_engine::doc::chat::{Author, Chat, FULL, MAX_MESSAGES, MAX_TEXT, Message};
use wordcraft_engine::doc::{Document, Path, StoryRef};
use wordcraft_plugin::push::{Incoming, Pushed};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

struct Peer {
    s: Session,
    now: u64,
}

impl Peer {
    fn new(name: &str) -> Peer {
        let mut s = Session::new(Document::new());
        s.author = name.into();
        Peer { s, now: 1_000 }
    }
    fn deliver(&mut self, msg: Json) {
        self.now += 7;
        self.s.collab_poll_pushed(self.now, vec![Pushed { plugin: IN_MEMORY.into(), incoming: Incoming::Message(msg) }]);
    }
}

/// Every message a peer sent, for every other peer (and sometimes twice).
fn route(peers: &mut [Peer], from: usize, queue: &mut Vec<(usize, Json)>, rng: &mut Rng, sent: &mut Vec<Json>) {
    for msg in peers[from].s.collab_take_outgoing() {
        let method = msg["method"].as_str().unwrap_or("");
        if method == "collab/joined" || method == "collab/leave" || method.is_empty() {
            continue;
        }
        sent.push(msg.clone());
        for to in 0..peers.len() {
            if to != from {
                queue.push((to, msg.clone()));
                if rng.chance(8) {
                    queue.push((to, msg.clone()));
                }
            }
        }
    }
}

fn settle(peers: &mut [Peer], queue: &mut Vec<(usize, Json)>, rng: &mut Rng, sent: &mut Vec<Json>) {
    for _ in 0..50_000 {
        if queue.is_empty() {
            break;
        }
        let k = rng.below(queue.len());
        let (to, msg) = queue.remove(k);
        peers[to].deliver(msg);
        route(peers, to, queue, rng, sent);
    }
}

/// A room of `n` people: the first shares `doc` (with `key`, when given), the others join.
fn room(n: usize, doc: Document, key: Option<&str>, rng: &mut Rng, sent: &mut Vec<Json>) -> Vec<Peer> {
    let mut peers: Vec<Peer> = (0..n).map(|i| Peer::new(&format!("Person {i}"))).collect();
    peers[0].s.set_document(doc);
    let user = |i: usize| json!({"id": format!("u-{i}"), "name": format!("Person {i}"), "color": "#3B7DD8"});
    let mut p = json!({"room": "chat", "mode": "share", "user": user(0)});
    if let Some(k) = key {
        p["key"] = json!(k);
    }
    peers[0].s.collab_join_in_memory(p).unwrap();
    for (i, peer) in peers.iter_mut().enumerate().skip(1) {
        let mut p = json!({"room": "chat", "mode": "join", "user": user(i)});
        if let Some(k) = key {
            p["key"] = json!(k);
        }
        peer.s.collab_join_in_memory(p).unwrap();
    }
    let mut queue = Vec::new();
    for i in 0..n {
        route(&mut peers, i, &mut queue, rng, sent);
    }
    settle(&mut peers, &mut queue, rng, sent);
    for (i, p) in peers.iter().enumerate() {
        assert!(wordcraft_engine::collab::read_only(&p.s).is_none(), "peer {i} is still joining");
    }
    peers
}

fn ids(c: &Chat) -> Vec<String> {
    c.messages.iter().map(|m| m.id.clone()).collect()
}

/// Random chatting (and typing in the document) by `n` people at once.
fn chatting(seed: u64, n: usize, steps: usize, key: Option<&str>) {
    let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) ^ 0xC4A7);
    let mut sent = Vec::new();
    let mut peers = room(n, Document::from_text("Agenda"), key, &mut rng, &mut sent);
    let mut queue = Vec::new();
    let mut ok_sends = 0;
    for step in 0..steps {
        let i = rng.below(n);
        if rng.chance(55) || queue.is_empty() {
            let chat = peers[i].s.doc.chat.clone();
            let mine: Vec<&Message> = chat.messages.iter().filter(|m| m.author.id == format!("u-{i}") && !m.deleted).collect();
            let r = match rng.below(10) {
                0..=4 => peers[i].s.run("chat.send", &json!({"text": format!("p{i} s{step} 😀 @Person {}", (i + 1) % n)})),
                5 if !chat.messages.is_empty() => {
                    let to = &chat.messages[rng.below(chat.messages.len())].id;
                    peers[i].s.run("chat.send", &json!({"text": format!("re {step}"), "replyTo": to}))
                }
                6 if !mine.is_empty() => {
                    peers[i].s.run("chat.edit", &json!({"id": mine[rng.below(mine.len())].id, "text": format!("edited {step}")}))
                }
                7 if !mine.is_empty() => peers[i].s.run("chat.delete", &json!({"id": mine[rng.below(mine.len())].id})),
                _ => {
                    peers[i].s.run("caret.set", &json!({"pos": {"story": "body", "path": [0], "off": 0}})).unwrap();
                    peers[i].s.run("text.insert", &json!({"text": format!("{i}")}))
                }
            };
            if r.is_ok() && step % 10 < 5 {
                ok_sends += 1;
            }
            route(&mut peers, i, &mut queue, &mut rng, &mut sent);
        } else {
            let k = rng.below(queue.len());
            let (to, msg) = queue.remove(k);
            peers[to].deliver(msg);
            route(&mut peers, to, &mut queue, &mut rng, &mut sent);
        }
    }
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    for i in 0..n {
        peers[i].deliver(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "chat", "state": "offline"}}));
        peers[i].deliver(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "chat", "state": "online"}}));
        route(&mut peers, i, &mut queue, &mut rng, &mut sent);
    }
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    assert!(ok_sends > 0);
    let first = peers[0].s.doc.chat.clone();
    assert!(!first.messages.is_empty(), "seed {seed}: nobody chatted");
    let mut unique = ids(&first);
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), first.messages.len(), "every message once");
    for (i, p) in peers.iter().enumerate().skip(1) {
        assert_eq!(*p.s.doc.chat, *first, "seed {seed}: peer {i}'s chat differs from peer 0's");
        assert_eq!(p.s.collab_fingerprint(), peers[0].s.collab_fingerprint(), "seed {seed}: the documents converge too");
    }
    // Replies point at messages in the chat.
    for m in &first.messages {
        if let Some(r) = &m.reply_to {
            assert!(first.get(r).is_some(), "a reply to a message that's there");
        }
    }
    // A newcomer gets the same chat.
    let mut late = Peer::new("Late");
    let mut p = json!({"room": "chat", "mode": "join", "user": {"id": "u-late", "name": "Late", "color": "#123456"}});
    if let Some(k) = key {
        p["key"] = json!(k);
    }
    late.s.collab_join_in_memory(p).unwrap();
    peers.push(late);
    let last = peers.len() - 1;
    route(&mut peers, last, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    assert_eq!(*peers[last].s.doc.chat, *first, "seed {seed}: the newcomer's chat");
    if key.is_some() {
        // Nothing readable went over the wire.
        for m in &sent {
            let text = m.to_string();
            assert!(!text.contains("p0 s") && !text.contains("edited"), "plaintext in {text}");
        }
    }
}

#[test]
fn two_people_chatting_converge() {
    for seed in 0..4 {
        chatting(seed, 2, 80, None);
    }
}

#[test]
fn three_people_chatting_converge() {
    for seed in 0..4 {
        chatting(seed + 100, 3, 90, None);
    }
}

#[test]
fn an_encrypted_room_carries_the_chat_encrypted() {
    chatting(7, 2, 60, Some("BRAbJjE8R1JdaHN+iZSfqrXAy9bh7PcCDRgjLjlET1o="));
}

fn note(id: &str, text: &str) -> Message {
    Message {
        id: id.into(),
        author: Author { id: "u-file".into(), name: "Writer".into(), color: "#2E9E5B".into() },
        at: 1_760_000_000_000,
        text: Some(text.into()),
        ..Message::default()
    }
}

#[test]
fn sharing_seeds_the_room_and_joining_brings_the_chat_in() {
    let mut d = Document::from_text("Report");
    d.chat = Arc::new(Chat { cleared_at: Some(5), messages: vec![note("aaaa0001", "Left in the file"), note("aaaa0002", "Second note")] });
    let mut rng = Rng(3);
    let mut sent = Vec::new();
    let mut peers = room(2, d, None, &mut rng, &mut sent);
    assert_eq!(peers[1].s.doc.chat.messages.len(), 2, "the joiner got the file's chat");
    assert_eq!(peers[1].s.doc.chat.cleared_at, Some(5));
    assert_eq!(peers[1].s.doc.chat, peers[0].s.doc.chat);
    assert!(!peers[1].s.dirty, "joining isn't a change of the joiner's");
    // A message from the joiner lands after the notes, for both.
    let id = peers[1].s.run("chat.send", &json!({"text": "Hello", "replyTo": "aaaa0001"})).unwrap()["id"].as_str().unwrap().to_string();
    let mut queue = Vec::new();
    route(&mut peers, 1, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    assert_eq!(ids(&peers[0].s.doc.chat), ["aaaa0001", "aaaa0002", id.as_str()]);
    assert!(peers[0].s.dirty, "others' messages mark the document modified");
    // Only the author edits or deletes.
    assert!(peers[0].s.run("chat.edit", &json!({"id": id, "text": "mine now"})).is_err());
    assert!(peers[0].s.run("chat.delete", &json!({"id": id})).is_err());
    peers[1].s.run("chat.delete", &json!({"id": id})).unwrap();
    route(&mut peers, 1, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    let m = peers[0].s.doc.chat.get(&id).cloned().unwrap();
    assert!(m.deleted && m.text.is_none(), "the entry stays as \"Message deleted\"");
    // Leaving keeps the chat in the document.
    peers[0].s.run("collab.leave", &json!({})).unwrap();
    assert_eq!(peers[0].s.doc.chat.messages.len(), 3);
}

#[test]
fn clearing_reaches_everyone() {
    let mut rng = Rng(11);
    let mut sent = Vec::new();
    let mut peers = room(3, Document::from_text("x"), None, &mut rng, &mut sent);
    let mut queue = Vec::new();
    for i in 0..3 {
        peers[i].s.run("chat.send", &json!({"text": format!("hi from {i}")})).unwrap();
        route(&mut peers, i, &mut queue, &mut rng, &mut sent);
    }
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    assert_eq!(peers[2].s.doc.chat.messages.len(), 3);
    let r = peers[1].s.run("chat.clear", &json!({})).unwrap();
    assert_eq!(r["removed"], 3);
    route(&mut peers, 1, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    for p in &peers {
        assert!(p.s.doc.chat.messages.is_empty());
        assert!(p.s.doc.chat.cleared_at.is_some());
    }
    peers[2].s.run("chat.send", &json!({"text": "fresh start"})).unwrap();
    route(&mut peers, 2, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    assert_eq!(peers[0].s.doc.chat.messages.len(), 1);
}

#[test]
fn chat_is_outside_undo_in_a_room_and_typing_is_shown() {
    let mut rng = Rng(5);
    let mut sent = Vec::new();
    let mut peers = room(2, Document::from_text("one"), None, &mut rng, &mut sent);
    let mut queue = Vec::new();
    peers[0].s.run("caret.set", &json!({"pos": {"story": "body", "path": [0], "off": 3}})).unwrap();
    peers[0].s.run("text.insert", &json!({"text": " two"})).unwrap();
    peers[0].s.run("chat.send", &json!({"text": "I added two"})).unwrap();
    route(&mut peers, 0, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    peers[0].s.run("edit.undo", &json!({})).unwrap();
    route(&mut peers, 0, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    let text = |s: &Session| s.doc.para(StoryRef::Body, &Path::top(0)).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(text(&peers[1].s), "one", "undo took the typing back");
    for p in &peers {
        assert_eq!(p.s.doc.chat.messages.len(), 1, "but not the message");
    }
    // Typing in the chat shows on the other side, and stops with the send.
    peers[0].s.run("chat.typing", &json!({"typing": true})).unwrap();
    route(&mut peers, 0, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    assert_eq!(peers[1].s.chat_typing_names(), ["Person 0"]);
    peers[0].s.run("chat.send", &json!({"text": "done"})).unwrap();
    route(&mut peers, 0, &mut queue, &mut rng, &mut sent);
    settle(&mut peers, &mut queue, &mut rng, &mut sent);
    assert!(peers[1].s.chat_typing_names().is_empty());
}

#[test]
fn commands_outside_a_session() {
    let mut s = Session::new(Document::from_text("Notes"));
    s.author = "Ann Lee".into();
    s.dirty = false;
    let a = s.run("chat.send", &json!({"text": "First note\nwith a second line"})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(s.dirty, "a message marks the document modified");
    assert!(!s.can_undo(), "and isn't an undo step");
    let b = s.run("chat.send", &json!({"text": "A reply", "replyTo": a})).unwrap()["id"].as_str().unwrap().to_string();
    let l = s.run("chat.list", &json!({})).unwrap();
    assert_eq!(l["count"], 2);
    assert_eq!(l["messages"][0]["author"]["name"], "Ann Lee");
    assert_eq!(l["messages"][1]["replyTo"], a.as_str());
    assert_eq!(s.run("chat.list", &json!({"since": a})).unwrap()["messages"].as_array().unwrap().len(), 1);
    assert_eq!(s.run("chat.list", &json!({"limit": 1})).unwrap()["messages"][0]["id"], b.as_str());
    // Refusals: nothing changes.
    let before = s.doc.chat.clone();
    for (cmd, p) in [
        ("chat.send", json!({"text": ""})),
        ("chat.send", json!({"text": "x".repeat(MAX_TEXT + 1)})),
        ("chat.send", json!({"text": "to nobody", "replyTo": "zzzzzzzz"})),
        ("chat.send", json!({"text": 5})),
        ("chat.edit", json!({"id": "zzzzzzzz", "text": "x"})),
        ("chat.edit", json!({"id": a, "text": ""})),
        ("chat.delete", json!({"id": "nope"})),
        ("chat.export", json!({"format": "pdf"})),
    ] {
        assert!(s.run(cmd, &p).is_err(), "{cmd} {p} should be refused");
    }
    assert_eq!(s.doc.chat, before);
    s.run("chat.send", &json!({"text": "é".repeat(MAX_TEXT)})).unwrap();
    // Someone else's message (left in the file by another person).
    Arc::make_mut(&mut s.doc.chat).messages.push(note("other001", "theirs"));
    assert!(s.run("chat.edit", &json!({"id": "other001", "text": "mine"})).is_err());
    assert!(s.run("chat.delete", &json!({"id": "other001"})).is_err());
    s.run("chat.edit", &json!({"id": a, "text": "First note, edited"})).unwrap();
    s.run("chat.delete", &json!({"id": b})).unwrap();
    assert!(s.run("chat.edit", &json!({"id": b, "text": "back"})).is_err(), "a deleted message can't be edited");
    let msgs = &s.doc.chat.messages;
    assert!(msgs[0].edited.is_some() && msgs[1].deleted);
    let t = s.run("chat.export", &json!({})).unwrap();
    assert!(t["text"].as_str().unwrap().contains("Ann Lee: First note, edited (edited)"), "{t}");
    let j = s.run("chat.export", &json!({"format": "json"})).unwrap();
    assert_eq!(j["chat"]["messages"].as_array().unwrap().len(), 4);
    // Undo doesn't touch the chat; typing and undoing it leaves the messages.
    s.run("text.insert", &json!({"text": "typed "})).unwrap();
    s.run("chat.send", &json!({"text": "after typing"})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.chat.messages.len(), 5, "undo never removes a message");
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc.chat.messages.len(), 5);
    // Clear.
    let r = s.run("chat.clear", &json!({})).unwrap();
    assert_eq!(r["removed"], 5);
    assert!(s.doc.chat.messages.is_empty() && s.doc.chat.cleared_at.is_some());
    // view.chat shows and hides the panel.
    assert_eq!(s.run("view.chat", &json!({})).unwrap()["value"], true);
    assert_eq!(s.run("view.chat", &json!({"value": false})).unwrap()["value"], false);
}

#[test]
fn the_limit() {
    let mut s = Session::new(Document::new());
    let id = s.run("chat.send", &json!({"text": "mine"})).unwrap()["id"].as_str().unwrap().to_string();
    {
        let c = Arc::make_mut(&mut s.doc.chat);
        for i in 1..MAX_MESSAGES {
            c.messages.push(note(&format!("{i:08}"), "m"));
        }
    }
    assert_eq!(s.doc.chat.messages.len(), MAX_MESSAGES);
    let e = s.run("chat.send", &json!({"text": "one more"})).unwrap_err().to_string();
    assert!(e.contains(FULL), "{e}");
    // Deleting doesn't free a place; clearing does.
    s.run("chat.delete", &json!({"id": id})).unwrap();
    assert!(s.run("chat.send", &json!({"text": "one more"})).is_err());
    assert_eq!(s.run("chat.clear", &json!({})).unwrap()["removed"], MAX_MESSAGES);
    s.run("chat.send", &json!({"text": "one more"})).unwrap();
    // And in a room: the shared chat refuses the next one too.
    let mut rng = Rng(9);
    let mut sent = Vec::new();
    let mut d = Document::new();
    d.chat = Arc::new(Chat { cleared_at: None, messages: (0..MAX_MESSAGES).map(|i| note(&format!("{i:08}"), "m")).collect() });
    let mut peers = room(1, d, None, &mut rng, &mut sent);
    let e = peers[0].s.run("chat.send", &json!({"text": "over"})).unwrap_err().to_string();
    assert!(e.contains(FULL), "{e}");
    assert!(peers[0].s.collab_take_outgoing().iter().all(|m| m["method"] != "collab/update"), "nothing sent");
}

#[test]
fn the_chat_is_saved_with_the_document() {
    let mut s = Session::new(Document::from_text("Saved"));
    s.run("chat.send", &json!({"text": "Kept in the file"})).unwrap();
    let dir = std::env::temp_dir().join(format!("wordcraft-chat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for ext in ["docx", "json"] {
        let path = dir.join(format!("chat.{ext}"));
        s.run("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
        let mut t = Session::new(Document::new());
        t.run("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(t.doc.chat, s.doc.chat, "{ext}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
