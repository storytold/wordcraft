//! Document chat (craft-collab `docs/chat.md`): `chat.list`, `chat.send`, `chat.edit`,
//! `chat.delete`, `chat.clear`, `chat.export` and `view.chat`.
//!
//! The chat belongs to the document ([`wordcraft_doc::chat`]): it's saved in the file and, in a
//! live session, shared through the room (the room's Yjs document is the source; see
//! `crate::collab`). Outside a session the messages are notes left in the file, written by the
//! app's own user. Chat changes are never part of undo (the commands are "pure" for the undo
//! history) but they do mark the document modified: it has to be saved for the chat to be kept.
//! Programmatic calls never ask; `chat.clear`'s confirmation is the interface's.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use wordcraft_collab::chat::ChatOp;
use wordcraft_doc::chat::{Author, Chat, Message};

use crate::{CmdError, CmdResult, CommandSpec, Session, p};

const LOC: &str = "Title bar › Chat";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("chat.list", "Chat Messages", LOC, list)
            .params("{since?: message id | time (ms), limit?} → {messages: [{id, author: {id, name, color}, at, text?, replyTo?, edited?, deleted?}], clearedAt?, count}: the messages after `since` (newest last), the newest `limit` of them")
            .pure(),
        CommandSpec::new("chat.send", "Send Chat Message", LOC, send)
            .params("{text: 1–4,000 characters, replyTo?: message id} → {id}. In a live session it goes to everyone; outside one it's a note left in the file")
            .pure(),
        CommandSpec::new("chat.edit", "Edit Chat Message", LOC, edit).params("{id, text} → {id}. Only the author, not a deleted message").pure(),
        CommandSpec::new("chat.delete", "Delete Chat Message", LOC, delete)
            .params("{id} → {id}. Only the author; a \"Message deleted\" entry stays so replies still point somewhere")
            .pure(),
        CommandSpec::new("chat.clear", "Clear Chat", LOC, clear).params("{} → {removed}. Removes the whole chat from the document, for everyone in a live session").pure(),
        CommandSpec::new("chat.export", "Export Chat", LOC, export).params("{format?: \"text\" | \"json\"} → {format, text} (json also gives `chat`)").pure(),
        CommandSpec::new("chat.typing", "Typing in Chat", LOC, typing)
            .params("{typing: bool} → {typing}. Tells the others in a live session that you're typing in the chat (awareness `typing`)")
            .pure(),
        CommandSpec::new("view.chat", "Chat", "View › Show", toggle).params(r#"{"value"?: bool} → {value}. Shows or hides the chat panel"#).pure(),
    ]
}

fn bad(m: impl Into<String>) -> CmdError {
    CmdError::Params(m.into())
}

fn list(s: &mut Session, prm: &Value) -> CmdResult {
    Ok(s.doc.chat.list(prm))
}

fn typing(s: &mut Session, prm: &Value) -> CmdResult {
    let on = p::bool(prm, "typing").or_else(|| p::bool(prm, "value")).unwrap_or(false);
    s.chat_typing(on);
    Ok(json!({"typing": on}))
}

fn toggle(s: &mut Session, prm: &Value) -> CmdResult {
    s.view.chat_pane = p::bool(prm, "value").unwrap_or(!s.view.chat_pane);
    Ok(json!({"value": s.view.chat_pane}))
}

/// A new message id: 8 random base-36 characters, not in the chat yet.
fn new_id(chat: &Chat) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
    for _ in 0..32 {
        let mut b = [0u8; 8];
        let v = if wordcraft_collab::random_bytes(&mut b).is_ok() {
            u64::from_le_bytes(b)
        } else {
            // No random source (the browser build): a well-mixed clock and counter.
            let mut z = (crate::cmd::now_ms() as u64) ^ COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let id = wordcraft_doc::chat::id_from_bits(v);
        if chat.get(&id).is_none() {
            return id;
        }
    }
    wordcraft_doc::chat::id_from_bits(COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Applies a chat edit: through the room in a live session, to the document's chat otherwise.
/// Marks the document modified (not an undo step). The messages a clear removed.
fn apply(s: &mut Session, op: ChatOp) -> Result<usize, CmdError> {
    let removed = match crate::collab::chat_op(s, &op) {
        Some(r) => {
            let (removed, chat) = r?;
            s.doc.chat = Arc::new(chat);
            removed
        }
        None => {
            let mut chat = (*s.doc.chat).clone();
            let r = match &op {
                ChatOp::Send(m) => chat.send(m.clone()).map(|_| 0),
                ChatOp::Edit { id, author, text, at } => chat.edit(id, author, text, *at).map(|_| 0),
                ChatOp::Delete { id, author } => chat.delete(id, author).map(|_| 0),
                ChatOp::Clear { at } => Ok(chat.clear(*at)),
            };
            let removed = r.map_err(CmdError::Params)?;
            s.doc.chat = Arc::new(chat);
            removed
        }
    };
    s.touch();
    Ok(removed)
}

fn author(s: &mut Session) -> Author {
    let u = crate::collab::chat_user(s);
    Author { id: u.id, name: u.name, color: u.color }
}

fn send(s: &mut Session, prm: &Value) -> CmdResult {
    let text = p::req_str(prm, "text")?.to_string();
    wordcraft_doc::chat::valid_text(&text).map_err(bad)?;
    let reply_to = match prm.get("replyTo") {
        None | Some(Value::Null) => None,
        Some(Value::String(r)) => Some(r.clone()),
        Some(_) => return Err(bad("`replyTo` is a message id")),
    };
    let id = new_id(&s.doc.chat);
    let m = Message { id: id.clone(), author: author(s), at: crate::cmd::now_ms().max(0), text: Some(text), reply_to, edited: None, deleted: false };
    apply(s, ChatOp::Send(m))?;
    // Sending ends "typing…".
    s.chat_typing(false);
    s.status = "Message sent".into();
    Ok(json!({"id": id}))
}

fn edit(s: &mut Session, prm: &Value) -> CmdResult {
    let id = p::req_str(prm, "id")?.to_string();
    let text = p::req_str(prm, "text")?.to_string();
    let author = author(s).id;
    apply(s, ChatOp::Edit { id: id.clone(), author, text, at: crate::cmd::now_ms().max(0) })?;
    Ok(json!({"id": id}))
}

fn delete(s: &mut Session, prm: &Value) -> CmdResult {
    let id = p::req_str(prm, "id")?.to_string();
    let author = author(s).id;
    apply(s, ChatOp::Delete { id: id.clone(), author })?;
    Ok(json!({"id": id}))
}

fn clear(s: &mut Session, _prm: &Value) -> CmdResult {
    let removed = apply(s, ChatOp::Clear { at: crate::cmd::now_ms().max(0) })?;
    s.status = "Chat cleared".into();
    Ok(json!({"removed": removed}))
}

fn export(s: &mut Session, prm: &Value) -> CmdResult {
    match p::str(prm, "format").unwrap_or("text") {
        "text" => Ok(json!({"format": "text", "text": s.doc.chat.to_text()})),
        "json" => {
            let j = s.doc.chat.to_json();
            Ok(json!({"format": "json", "text": serde_json::to_string_pretty(&j).unwrap_or_default(), "chat": j}))
        }
        other => Err(bad(format!("`format` is \"text\" or \"json\", not {other:?}"))),
    }
}
