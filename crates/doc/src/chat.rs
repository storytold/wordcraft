//! Document chat (craft-collab `docs/chat.md`, protocol 2.1): the messages the people editing a
//! document leave each other. The chat belongs to the document: it's saved in the file (the DOCX
//! part `/craft/chat.json`, the native JSON's `chat`), shared live in a room, and stays until
//! someone clears it. It's never part of undo.
//!
//! This module is the model and its JSON form (the file part's content without `version`, which
//! is also the conformance transcripts' form): reading applies the spec's validation and limits,
//! and the plain edits ([`Chat::send`], [`Chat::edit`], [`Chat::delete`], [`Chat::clear`]) are
//! what the `chat.*` commands do outside a live session. In a room the shared Yjs document is
//! the source and this is its mirror (`wordcraft-collab`'s `chat` module).

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map as JsonMap, Value as Json, json};

/// At most this many messages: readers ignore the rest, senders refuse more.
pub const MAX_MESSAGES: usize = 10_000;
/// A message's text: 1 to this many UTF-16 units.
pub const MAX_TEXT: usize = 4_000;
/// What the app says when the chat is full.
pub const FULL: &str = "The chat is full: clear it to go on";
/// The file part's `version`.
pub const PART_VERSION: i64 = 1;
/// The DOCX part, its content type and the package relationship that reaches it.
pub const PART_NAME: &str = "craft/chat.json";
pub const CONTENT_TYPE: &str = "application/vnd.craft.chat+json";
pub const RELATIONSHIP: &str = "urn:craft:relationships:chat";
/// Largest chat part read (10,000 messages of 4,000 characters, with room to spare).
pub const MAX_PART_BYTES: usize = 64 << 20;
/// The latest time a message may carry (2^53 milliseconds).
pub const MAX_MS: i64 = 1 << 53;

/// Who wrote a message: the `user` of the room's join, or the app's own user.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub color: String,
}

impl Author {
    pub fn to_json(&self) -> Json {
        json!({"id": self.id, "name": self.name, "color": self.color})
    }

    /// `{id, name, color}`: `id` and `name` non-empty strings, `color` a string.
    pub fn from_json(j: &Json) -> Option<Author> {
        let s = |k: &str| j.get(k).and_then(Json::as_str).map(str::to_string);
        let (id, name, color) = (s("id")?, s("name")?, s("color")?);
        (!id.is_empty() && !name.is_empty()).then_some(Author { id, name, color })
    }

    /// One or two letters for the author's circle.
    pub fn initials(&self) -> String {
        let mut words = self.name.split_whitespace().filter_map(|w| w.chars().next());
        let first: String = words.next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        let last: String = words.next_back().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        if first.is_empty() { "?".into() } else { first + &last }
    }
}

/// One message.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Message {
    /// 8 characters of `0-9a-z`, unique in the chat.
    pub id: String,
    pub author: Author,
    /// Milliseconds since 1970 UTC, the sender's clock.
    pub at: i64,
    /// `None` once deleted.
    pub text: Option<String>,
    pub reply_to: Option<String>,
    /// When the author last edited it.
    pub edited: Option<i64>,
    pub deleted: bool,
}

impl Message {
    pub fn to_json(&self) -> Json {
        let mut o = JsonMap::new();
        o.insert("id".into(), json!(self.id));
        o.insert("author".into(), self.author.to_json());
        o.insert("at".into(), json!(self.at));
        if let Some(t) = &self.text {
            o.insert("text".into(), json!(t));
        }
        if let Some(r) = &self.reply_to {
            o.insert("replyTo".into(), json!(r));
        }
        if let Some(e) = self.edited {
            o.insert("edited".into(), json!(e));
        }
        if self.deleted {
            o.insert("deleted".into(), json!(true));
        }
        Json::Object(o)
    }

    /// A message in the JSON form (`author` an object); `None` when malformed. A malformed
    /// optional key (`replyTo`, `edited`) is dropped, not the message.
    pub fn from_json(j: &Json) -> Option<Message> {
        let id = j.get("id").and_then(Json::as_str).filter(|s| valid_id(s))?.to_string();
        let author = Author::from_json(j.get("author")?)?;
        let at = j.get("at").and_then(json_ms)?;
        let deleted = j.get("deleted").and_then(Json::as_bool) == Some(true);
        let text = if deleted { None } else { Some(j.get("text").and_then(Json::as_str).filter(|t| valid_text(t).is_ok())?.to_string()) };
        let reply_to = j.get("replyTo").and_then(Json::as_str).filter(|s| valid_id(s)).map(str::to_string);
        let edited = j.get("edited").and_then(json_ms);
        Some(Message { id, author, at, text, reply_to, edited, deleted })
    }

    /// The text, or "" once deleted.
    pub fn text(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }
}

/// The chat: valid messages in order (oldest first), at most [`MAX_MESSAGES`], and when it was
/// last cleared.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Chat {
    pub cleared_at: Option<i64>,
    pub messages: Vec<Message>,
}

impl Serialize for Chat {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Chat {
    /// Lenient, like the file part: a chat that isn't one reads as empty, and malformed
    /// messages are skipped, so a damaged chat never stops a document from opening.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Chat, D::Error> {
        let j = Json::deserialize(d)?;
        Ok(Chat::from_json(&j).unwrap_or_default())
    }
}

impl Chat {
    /// Nothing to save: no messages and never cleared.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.cleared_at.is_none()
    }

    /// `{clearedAt?, messages}`.
    pub fn to_json(&self) -> Json {
        let mut o = JsonMap::new();
        if let Some(c) = self.cleared_at {
            o.insert("clearedAt".into(), json!(c));
        }
        o.insert("messages".into(), Json::Array(self.messages.iter().map(Message::to_json).collect()));
        Json::Object(o)
    }

    /// The file part's content: `{version: 1, clearedAt?, messages}`.
    pub fn to_part(&self) -> Json {
        let mut j = self.to_json();
        if let Some(o) = j.as_object_mut() {
            o.insert("version".into(), json!(PART_VERSION));
        }
        j
    }

    /// The file part's bytes.
    pub fn to_part_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.to_part()).unwrap_or_default()
    }

    /// The JSON form (with or without `version` 1). Malformed messages are skipped, duplicates
    /// after the first and messages beyond [`MAX_MESSAGES`] too; `Err` when the whole thing isn't
    /// a chat (the file opens without it, with a warning).
    pub fn from_json(j: &Json) -> Result<Chat, String> {
        let o = j.as_object().ok_or("the chat isn't a JSON object")?;
        if let Some(v) = o.get("version")
            && v.as_i64() != Some(PART_VERSION)
        {
            return Err(format!("unknown chat version {v}"));
        }
        let list = o.get("messages").and_then(Json::as_array).ok_or("the chat has no `messages` list")?;
        let mut seen = HashSet::new();
        let messages = list.iter().filter_map(Message::from_json).filter(|m| seen.insert(m.id.clone())).take(MAX_MESSAGES).collect();
        Ok(Chat { cleared_at: o.get("clearedAt").and_then(json_ms), messages })
    }

    /// The file part's bytes → the chat.
    pub fn from_part_bytes(bytes: &[u8]) -> Result<Chat, String> {
        if bytes.len() > MAX_PART_BYTES {
            return Err(format!("the chat part is larger than {} MiB", MAX_PART_BYTES >> 20));
        }
        let j: Json = serde_json::from_slice(bytes).map_err(|e| format!("the chat part isn't JSON: {e}"))?;
        Chat::from_json(&j)
    }

    pub fn get(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// Checks a new message: its text, its reply, its id and the limit.
    pub fn check_send(&self, m: &Message) -> Result<(), String> {
        valid_text(m.text.as_deref().unwrap_or(""))?;
        if self.messages.len() >= MAX_MESSAGES {
            return Err(FULL.into());
        }
        if !valid_id(&m.id) || self.get(&m.id).is_some() {
            return Err(format!("There's already a message {}", m.id));
        }
        if let Some(r) = &m.reply_to
            && self.get(r).is_none()
        {
            return Err(format!("No message {r} to reply to"));
        }
        Ok(())
    }

    /// The message `id` if `author` wrote it and it isn't deleted.
    pub fn check_own(&self, id: &str, author: &str) -> Result<usize, String> {
        let i = self.messages.iter().position(|m| m.id == id).ok_or_else(|| format!("No message {id}"))?;
        let m = self.messages.get(i).ok_or_else(|| format!("No message {id}"))?;
        if m.author.id != author {
            return Err("Only the author can change a message".into());
        }
        if m.deleted {
            return Err("The message was deleted".into());
        }
        Ok(i)
    }

    /// `chat.send`: appends `m` after [`Chat::check_send`].
    pub fn send(&mut self, m: Message) -> Result<(), String> {
        self.check_send(&m)?;
        self.messages.push(m);
        Ok(())
    }

    /// `chat.edit`: the author only, not a deleted message.
    pub fn edit(&mut self, id: &str, author: &str, text: &str, at: i64) -> Result<(), String> {
        valid_text(text)?;
        let i = self.check_own(id, author)?;
        if let Some(m) = self.messages.get_mut(i) {
            m.text = Some(text.to_string());
            m.edited = Some(at);
        }
        Ok(())
    }

    /// `chat.delete`: the author only; the entry stays without its text.
    pub fn delete(&mut self, id: &str, author: &str) -> Result<(), String> {
        let i = self.check_own(id, author)?;
        if let Some(m) = self.messages.get_mut(i) {
            m.text = None;
            m.deleted = true;
        }
        Ok(())
    }

    /// `chat.clear`: removes every message and notes when; the number removed.
    pub fn clear(&mut self, at: i64) -> usize {
        let n = self.messages.len();
        self.messages.clear();
        self.cleared_at = Some(at);
        n
    }

    /// `chat.list {since?, limit?}`: the messages after the one with id `since` (all of them
    /// without it, or when it's gone), or, when `since` is a number (milliseconds), the ones
    /// sent after that time; the newest `limit` of those. `{messages, clearedAt?, count}` with
    /// `count` every message in the chat.
    pub fn list(&self, params: &Json) -> Json {
        let after: Vec<&Message> = match params.get("since") {
            Some(Json::String(id)) => {
                let start = self.messages.iter().position(|m| &m.id == id).map(|i| i.saturating_add(1)).unwrap_or(0);
                self.messages.get(start..).unwrap_or_default().iter().collect()
            }
            Some(n @ Json::Number(_)) => {
                let t = n.as_f64().unwrap_or(0.0);
                self.messages.iter().filter(|m| m.at as f64 > t).collect()
            }
            _ => self.messages.iter().collect(),
        };
        let limit = params.get("limit").and_then(Json::as_u64).map(|l| usize::try_from(l).unwrap_or(usize::MAX)).unwrap_or(usize::MAX);
        let from = after.len().saturating_sub(limit);
        let mut j = json!({
            "messages": after.get(from..).unwrap_or_default().iter().map(|m| m.to_json()).collect::<Vec<_>>(),
            "count": self.messages.len(),
        });
        if let (Some(c), Some(o)) = (self.cleared_at, j.as_object_mut()) {
            o.insert("clearedAt".into(), json!(c));
        }
        j
    }

    /// The chat as plain text, one message per paragraph: `[time] Name: text`, replies and
    /// edits noted (`chat.export`).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for m in &self.messages {
            out.push_str(&format!("[{}] {}", iso_ms(m.at), m.author.name));
            if let Some(r) = m.reply_to.as_deref().and_then(|r| self.get(r)) {
                out.push_str(&format!(" (reply to {})", r.author.name));
            }
            out.push_str(": ");
            if m.deleted {
                out.push_str("(message deleted)");
            } else {
                out.push_str(m.text());
                if m.edited.is_some() {
                    out.push_str(" (edited)");
                }
            }
            out.push('\n');
        }
        out
    }
}

/// 8 characters of `0-9a-z`.
pub fn valid_id(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
}

/// 1 to [`MAX_TEXT`] UTF-16 units.
pub fn valid_text(text: &str) -> Result<(), String> {
    match text.encode_utf16().count() {
        0 => Err("A message can't be empty".into()),
        n if n > MAX_TEXT => Err(format!("A message holds at most {MAX_TEXT} characters")),
        _ => Ok(()),
    }
}

/// A message id from 64 random bits: 8 base-36 characters.
pub fn id_from_bits(mut v: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut s = String::with_capacity(8);
    for _ in 0..8 {
        let d = (v % 36) as usize;
        s.push(char::from(DIGITS.get(d).copied().unwrap_or(b'0')));
        v /= 36;
    }
    s
}

/// A time in milliseconds: a non-negative integer, at most 2^53.
pub fn json_ms(v: &Json) -> Option<i64> {
    let n = v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && f.is_finite() && f.abs() <= 9.1e15).map(|f| f as i64))?;
    (0..=MAX_MS).contains(&n).then_some(n)
}

/// `YYYY-MM-DD HH:MM` (UTC) of a time in milliseconds.
pub fn iso_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, rem % 3600 / 60)
}

/// Days since 1970-01-01 → (year, month, day).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z.saturating_add(719_468);
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `Document::chat` in the native JSON: left out when empty.
pub(crate) fn arc_is_empty(c: &std::sync::Arc<Chat>) -> bool {
    c.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn author(id: &str) -> Author {
        Author { id: id.into(), name: id.to_uppercase(), color: "#112233".into() }
    }

    fn msg(id: &str, who: &str, text: &str) -> Message {
        Message { id: id.into(), author: author(who), at: 1_760_000_000_000, text: Some(text.into()), ..Message::default() }
    }

    #[test]
    fn send_edit_delete_clear() {
        let mut c = Chat::default();
        c.send(msg("aaaaaaaa", "a", "hi")).unwrap();
        assert!(c.send(msg("aaaaaaaa", "a", "again")).is_err());
        assert!(c.send(msg("bbbbbbbb", "a", "")).is_err());
        assert!(c.send(msg("bbbbbbbb", "a", &"x".repeat(MAX_TEXT + 1))).is_err());
        assert!(c.send(Message { reply_to: Some("zzzzzzzz".into()), ..msg("bbbbbbbb", "a", "re") }).is_err());
        c.send(msg("bbbbbbbb", "b", &"é".repeat(MAX_TEXT))).unwrap();
        assert!(c.edit("aaaaaaaa", "b", "no", 1).is_err());
        c.edit("aaaaaaaa", "a", "hello", 7).unwrap();
        assert!(c.delete("bbbbbbbb", "a").is_err());
        c.delete("bbbbbbbb", "b").unwrap();
        assert!(c.edit("bbbbbbbb", "b", "back", 8).is_err());
        assert_eq!(c.messages[0].edited, Some(7));
        assert!(c.messages[1].deleted && c.messages[1].text.is_none());
        assert_eq!(Chat::from_json(&c.to_part()).unwrap(), c);
        assert_eq!(c.clear(9), 2);
        assert_eq!(c, Chat { cleared_at: Some(9), messages: vec![] });
        assert!(!c.is_empty());
    }

    #[test]
    fn the_limit() {
        let mut c = Chat::default();
        for i in 0..MAX_MESSAGES {
            c.messages.push(msg(&format!("{i:08}"), "a", "m"));
        }
        assert_eq!(c.send(msg("yyyyyyyy", "a", "one more")), Err(FULL.to_string()));
        c.delete("00000000", "a").unwrap();
        assert_eq!(c.send(msg("yyyyyyyy", "a", "one more")), Err(FULL.to_string()));
        assert_eq!(c.clear(1), MAX_MESSAGES);
        c.send(msg("yyyyyyyy", "a", "one more")).unwrap();
        let many: Vec<Json> = (0..MAX_MESSAGES + 5).map(|i| msg(&format!("{i:08}"), "a", "m").to_json()).collect();
        assert_eq!(Chat::from_json(&json!({"version": 1, "messages": many})).unwrap().messages.len(), MAX_MESSAGES);
    }

    #[test]
    fn malformed_parts() {
        assert!(Chat::from_json(&json!([])).is_err());
        assert!(Chat::from_json(&json!({"version": 2, "messages": []})).is_err());
        assert!(Chat::from_json(&json!({"version": 1})).is_err());
        assert!(Chat::from_part_bytes(b"{not json").is_err());
        let c = Chat::from_json(&json!({"version": 1, "clearedAt": "x", "messages": [
            {"id": "aaaaaaaa", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "text": "ok"},
            {"id": "aaaaaaaa", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "text": "duplicate"},
            {"id": "AAAAAAAA", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "text": "bad id"},
            {"id": "bbbbbbbb", "author": "A", "at": 1, "text": "bad author"},
            {"id": "cccccccc", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": -1, "text": "bad at"},
            {"id": "dddddddd", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1},
            {"id": "eeeeeeee", "author": {"id": "a", "name": "A", "color": "#000000"}, "at": 1, "deleted": true, "replyTo": 5},
            3
        ]}))
        .unwrap();
        assert_eq!(c.cleared_at, None);
        assert_eq!(c.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["aaaaaaaa", "eeeeeeee"]);
        assert!(c.messages[1].reply_to.is_none());
        // The native JSON reads a damaged chat as empty rather than failing.
        let d: Chat = serde_json::from_value(json!({"messages": 5})).unwrap();
        assert!(d.is_empty());
    }

    #[test]
    fn list_since_and_limit() {
        let c = Chat { cleared_at: Some(3), messages: ["aaaaaaaa", "bbbbbbbb", "cccccccc"].iter().map(|id| msg(id, "a", "x")).collect() };
        let l = c.list(&json!({"since": "aaaaaaaa", "limit": 1}));
        assert_eq!(l["count"], 3);
        assert_eq!(l["clearedAt"], 3);
        assert_eq!(l["messages"].as_array().unwrap().len(), 1);
        assert_eq!(l["messages"][0]["id"], "cccccccc");
        assert_eq!(c.list(&json!({}))["messages"].as_array().unwrap().len(), 3);
        assert_eq!(c.list(&json!({"since": "gone0000"}))["messages"].as_array().unwrap().len(), 3);
        let mut d = c.clone();
        d.messages[2].at += 10;
        let by_time = d.list(&json!({"since": 1_760_000_000_000_i64}));
        assert_eq!(by_time["messages"].as_array().unwrap().len(), 1);
        assert_eq!(by_time["messages"][0]["id"], "cccccccc");
    }

    #[test]
    fn ids_and_text() {
        let id = id_from_bits(u64::MAX);
        assert!(valid_id(&id), "{id}");
        assert!(valid_id(&id_from_bits(0)));
        assert_eq!(iso_ms(1_760_605_200_000), "2025-10-16 09:00");
        let c = Chat {
            cleared_at: None,
            messages: vec![msg("aaaaaaaa", "ann", "hi"), Message { reply_to: Some("aaaaaaaa".into()), ..msg("bbbbbbbb", "bo", "yes") }],
        };
        let t = c.to_text();
        assert!(t.contains("ANN: hi") && t.contains("BO (reply to ANN): yes"), "{t}");
    }
}
