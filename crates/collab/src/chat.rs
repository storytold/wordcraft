//! Document chat in the shared document (craft-collab `docs/chat.md`, protocol 2.1): the root
//! array `chat` of message maps and the root map `chatMeta` (`clearedAt`), the same in every
//! craft app's schema. Reading applies the spec's validation and limits (entries that aren't
//! maps, malformed ids, authors, times or texts, and duplicate ids are ignored; the first 10,000
//! valid messages are kept); writing is what `chat.send`, `chat.edit`, `chat.delete` and
//! `chat.clear` do. Messages are only appended, so concurrent sends land in the same order on
//! every peer. The chat is outside the undo manager's scope: Undo never touches it.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value as Json;
use wordcraft_doc::chat::{Author, Chat, FULL, MAX_MESSAGES, MAX_MS, Message, valid_id, valid_text};
use yrs::any::Number;
use yrs::{Any, Array, Map, MapPrelim, MapRef, Out, ReadTxn, TransactionMut, WriteTxn};

/// The root array of messages.
pub const ROOT: &str = "chat";
/// The root map with `clearedAt`.
pub const META: &str = "chatMeta";

fn any_ms(v: Option<Out>) -> Option<i64> {
    let n = match v? {
        Out::Any(Any::Number(Number::Int(i))) => i,
        Out::Any(Any::Number(Number::Float(f))) if f.fract() == 0.0 && f.is_finite() && f.abs() <= 9.1e15 => f as i64,
        _ => return None,
    };
    (0..=MAX_MS).contains(&n).then_some(n)
}

fn any_string(v: Option<Out>) -> Option<String> {
    match v? {
        Out::Any(Any::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn ms(n: i64) -> Any {
    Any::Number(Number::Int(n))
}

/// One raw entry → a message, `None` when malformed.
fn entry<T: ReadTxn>(txn: &T, m: &MapRef) -> Option<Message> {
    let id = any_string(m.get(txn, "id")).filter(|s| valid_id(s))?;
    let author = any_string(m.get(txn, "author")).and_then(|s| serde_json::from_str::<Json>(&s).ok()).and_then(|j| Author::from_json(&j))?;
    let at = any_ms(m.get(txn, "at"))?;
    let deleted = matches!(m.get(txn, "deleted"), Some(Out::Any(Any::Bool(true))));
    let text = if deleted { None } else { Some(any_string(m.get(txn, "text")).filter(|t| valid_text(t).is_ok())?) };
    let reply_to = any_string(m.get(txn, "replyTo")).filter(|s| valid_id(s));
    let edited = any_ms(m.get(txn, "edited"));
    Some(Message { id, author, at, text, reply_to, edited, deleted })
}

/// The valid messages with their maps (the first of a duplicated id, at most [`MAX_MESSAGES`]).
fn valid<T: ReadTxn>(txn: &T) -> Vec<(MapRef, Message)> {
    let Some(arr) = txn.get_array(ROOT) else { return Vec::new() };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for v in arr.iter(txn) {
        if out.len() >= MAX_MESSAGES {
            break;
        }
        let Out::YMap(m) = v else { continue };
        let Some(msg) = entry(txn, &m) else { continue };
        if seen.insert(msg.id.clone()) {
            out.push((m, msg));
        }
    }
    out
}

/// Reads the chat.
pub fn read<T: ReadTxn>(txn: &T) -> Chat {
    let cleared_at = txn.get_map(META).and_then(|m| any_ms(m.get(txn, "clearedAt")));
    Chat { cleared_at, messages: valid(txn).into_iter().map(|(_, m)| m).collect() }
}

fn fields(m: &Message) -> Vec<(&'static str, Any)> {
    let mut p: Vec<(&'static str, Any)> = vec![
        ("id", Any::String(Arc::from(m.id.as_str()))),
        ("author", Any::String(Arc::from(m.author.to_json().to_string().as_str()))),
        ("at", ms(m.at)),
    ];
    if let Some(t) = &m.text {
        p.push(("text", Any::String(Arc::from(t.as_str()))));
    }
    if let Some(r) = &m.reply_to {
        p.push(("replyTo", Any::String(Arc::from(r.as_str()))));
    }
    if let Some(e) = m.edited {
        p.push(("edited", ms(e)));
    }
    if m.deleted {
        p.push(("deleted", Any::Bool(true)));
    }
    p
}

/// Appends a message as is: a map with its keys set one by one, in a fixed order.
fn push(txn: &mut TransactionMut, m: &Message) {
    let arr = txn.get_or_insert_array(ROOT);
    let map = arr.push_back(txn, MapPrelim::default());
    for (k, v) in fields(m) {
        map.insert(txn, k, v);
    }
}

/// Writes a chat into the document being shared (the file's chat seeds the room's). Writes
/// nothing for an empty chat.
pub fn seed(txn: &mut TransactionMut, chat: &Chat) {
    if let Some(c) = chat.cleared_at {
        txn.get_or_insert_map(META).insert(txn, "clearedAt", ms(c));
    }
    for m in &chat.messages {
        push(txn, m);
    }
}

/// `chat.send`: appends `m` after checking its text, its reply, its id and the limit.
pub fn send(txn: &mut TransactionMut, m: &Message) -> Result<(), String> {
    valid_text(m.text.as_deref().unwrap_or(""))?;
    let all = valid(txn);
    if all.len() >= MAX_MESSAGES {
        return Err(FULL.into());
    }
    if !valid_id(&m.id) || all.iter().any(|(_, x)| x.id == m.id) {
        return Err(format!("There's already a message {}", m.id));
    }
    if let Some(r) = &m.reply_to
        && !all.iter().any(|(_, x)| &x.id == r)
    {
        return Err(format!("No message {r} to reply to"));
    }
    push(txn, m);
    Ok(())
}

/// The message `id` if `author` wrote it and it isn't deleted.
fn own(txn: &TransactionMut, id: &str, author: &str) -> Result<MapRef, String> {
    let (map, m) = valid(txn).into_iter().find(|(_, m)| m.id == id).ok_or_else(|| format!("No message {id}"))?;
    if m.author.id != author {
        return Err("Only the author can change a message".into());
    }
    if m.deleted {
        return Err("The message was deleted".into());
    }
    Ok(map)
}

/// `chat.edit`: the author only.
pub fn edit(txn: &mut TransactionMut, id: &str, author: &str, text: &str, at: i64) -> Result<(), String> {
    valid_text(text)?;
    let map = own(txn, id, author)?;
    map.insert(txn, "text", text);
    map.insert(txn, "edited", ms(at));
    Ok(())
}

/// `chat.delete`: the author only; the entry stays, without its text.
pub fn delete(txn: &mut TransactionMut, id: &str, author: &str) -> Result<(), String> {
    let map = own(txn, id, author)?;
    map.remove(txn, "text");
    map.insert(txn, "deleted", true);
    Ok(())
}

/// `chat.clear`: removes every entry and sets `clearedAt`; the number of messages removed.
pub fn clear(txn: &mut TransactionMut, at: i64) -> usize {
    let removed = valid(txn).len();
    if let Some(arr) = txn.get_array(ROOT) {
        let len = arr.len(txn);
        if len > 0 {
            arr.remove_range(txn, 0, len);
        }
    }
    txn.get_or_insert_map(META).insert(txn, "clearedAt", ms(at));
    removed
}

/// Who's typing: the names of the awareness states with `typing: true`, sorted.
pub fn typing<'a>(states: impl IntoIterator<Item = &'a Json>) -> Vec<String> {
    let mut names: Vec<String> = states
        .into_iter()
        .filter(|s| s.get("typing").and_then(Json::as_bool) == Some(true))
        .filter_map(|s| s.get("user").and_then(|u| u.get("name")).and_then(Json::as_str).map(str::to_string))
        .collect();
    names.sort();
    names
}

/// A chat edit, applied to the shared document by [`crate::Replica::chat`].
#[derive(Clone, Debug)]
pub enum ChatOp {
    Send(Message),
    Edit { id: String, author: String, text: String, at: i64 },
    Delete { id: String, author: String },
    Clear { at: i64 },
}

/// Applies `op` in `txn`: the number of messages removed for a clear, else 0.
pub(crate) fn apply(txn: &mut TransactionMut, op: &ChatOp) -> Result<usize, String> {
    match op {
        ChatOp::Send(m) => send(txn, m).map(|_| 0),
        ChatOp::Edit { id, author, text, at } => edit(txn, id, author, text, *at).map(|_| 0),
        ChatOp::Delete { id, author } => delete(txn, id, author).map(|_| 0),
        ChatOp::Clear { at } => Ok(clear(txn, *at)),
    }
}
