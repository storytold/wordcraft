//! Ask Agent's queue: requests from the window, the agents that listen for them, and who may edit.
//! Shared as is by WordCraft, GridCraft, DeckCraft and PdfCraft: no egui, no app types.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const NOT_OWNER: &str = "NOT_OWNER";
pub const NO_REQUEST: &str = "NO_REQUEST";
pub const INVALID_PARAMS: &str = "INVALID_PARAMS";
pub const READ_ONLY_REQUEST: &str = "READ_ONLY_REQUEST";
pub const HELD: &str = "HELD";

/// Longest `agent.listen` wait, and the wait when none is given.
pub const MAX_LISTEN: Duration = Duration::from_secs(60);
pub const DEFAULT_LISTEN: Duration = Duration::from_secs(30);

/// A refusal with a stable code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HubError {}

pub fn err(code: &'static str, message: impl Into<String>) -> HubError {
    HubError { code, message: message.into() }
}

/// One earlier exchange in a thread of follow-ups.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub instruction: String,
    pub answer: String,
}

/// What the agent receives: the instruction, the earlier turns, the selection and the document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub instruction: String,
    pub history: Vec<Turn>,
    pub selection: Value,
    pub document: Value,
    /// The agent asked for by name; `None` for any.
    pub agent: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Working { conn: u64 },
    Answered,
    Failed,
}

/// What the window shows for a request.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: u64,
    pub status: Status,
    pub agent: Option<String>,
    pub notes: Vec<String>,
    pub answer: Option<String>,
    pub error: Option<String>,
}

struct Entry {
    request: Request,
    shown: Snapshot,
}

#[derive(Default)]
struct Inner {
    last_id: u64,
    entries: Vec<Entry>,
    /// Connections listening, with the name each gave.
    listeners: HashMap<u64, String>,
}

impl Inner {
    fn entry(&mut self, id: u64) -> Result<&mut Entry, HubError> {
        self.entries.iter_mut().find(|e| e.request.id == id).ok_or_else(|| err(NO_REQUEST, format!("no Ask Agent request {id}")))
    }
}

/// The queue, shared by the window and the control channel's connection threads.
#[derive(Clone, Default)]
pub struct AgentHub(Arc<(Mutex<Inner>, Condvar)>);

impl AgentHub {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue a request from the window; returns its id.
    pub fn ask(&self, mut request: Request) -> u64 {
        let mut inner = self.lock();
        inner.last_id += 1;
        request.id = inner.last_id;
        let shown = Snapshot { id: request.id, status: Status::Queued, agent: None, notes: vec![], answer: None, error: None };
        inner.entries.push(Entry { request, shown });
        self.0.1.notify_all();
        inner.last_id
    }

    /// Wait up to `wait` for a request this agent may take, and take it.
    pub fn listen(&self, conn: u64, name: Option<&str>, wait: Duration, alive: impl Fn() -> bool) -> Option<Request> {
        let name = name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Agent").to_string();
        // Counted down in short steps rather than read from a clock, which wasm builds lack.
        let mut left = wait;
        let mut inner = self.lock();
        inner.listeners.insert(conn, name.clone());
        loop {
            let free = |e: &&mut Entry| e.shown.status == Status::Queued && e.request.agent.as_ref().is_none_or(|a| *a == name);
            if let Some(e) = inner.entries.iter_mut().find(free) {
                e.shown.status = Status::Working { conn };
                e.shown.agent = Some(name);
                return Some(e.request.clone());
            }
            if left.is_zero() || !alive() {
                return None;
            }
            let step = left.min(Duration::from_millis(250));
            inner = self.0.1.wait_timeout(inner, step).unwrap_or_else(PoisonError::into_inner).0;
            left -= step;
        }
    }

    /// A progress note (`done` false), the answer, or a failure, from the agent working on `id`.
    pub fn reply(&self, conn: u64, id: u64, text: &str, done: bool, error: Option<&str>) -> Result<(), HubError> {
        let mut inner = self.lock();
        let shown = &mut inner.entry(id)?.shown;
        if shown.status != (Status::Working { conn }) {
            return Err(err(NOT_OWNER, format!("Ask Agent request {id} is not yours: listen for it first")));
        }
        match (error, done) {
            (Some(e), _) => (shown.status, shown.error) = (Status::Failed, Some(e.into())),
            (None, true) => (shown.status, shown.answer) = (Status::Answered, Some(text.into())),
            (None, false) => shown.notes.push(text.into()),
        }
        Ok(())
    }

    /// The one edit rule: while a request is open, only the connection working on it edits, for it.
    pub fn may_edit(&self, conn: u64, request: Option<u64>) -> Result<(), HubError> {
        let mut inner = self.lock();
        let open = inner.entries.iter().any(|e| matches!(e.shown.status, Status::Queued | Status::Working { .. }));
        match request {
            None if open => Err(err(HELD, "the document is held for an Ask Agent request: pass its `request` id")),
            None => Ok(()),
            Some(id) => match inner.entry(id)?.shown.status {
                Status::Working { conn: c } if c == conn => Ok(()),
                Status::Answered | Status::Failed => Err(err(READ_ONLY_REQUEST, format!("Ask Agent request {id} is finished"))),
                _ => Err(err(NOT_OWNER, format!("Ask Agent request {id} is not yours: listen for it first"))),
            },
        }
    }

    /// The selection of each request being worked on, with its agent's name.
    pub fn held(&self) -> Vec<(Value, String)> {
        let inner = self.lock();
        let working = inner.entries.iter().filter(|e| matches!(e.shown.status, Status::Working { .. }));
        working.map(|e| (e.request.selection.clone(), e.shown.agent.clone().unwrap_or_default())).collect()
    }

    /// The connection is gone: its unanswered request goes back in the queue.
    pub fn disconnect(&self, conn: u64) {
        let mut inner = self.lock();
        inner.listeners.remove(&conn);
        for e in inner.entries.iter_mut().filter(|e| e.shown.status == Status::Working { conn }) {
            (e.shown.status, e.shown.agent) = (Status::Queued, None);
        }
        self.0.1.notify_all();
    }

    /// Names of the agents listening, sorted, for the picker.
    pub fn agents(&self) -> Vec<String> {
        let mut names: Vec<String> = self.lock().listeners.values().cloned().collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn snapshot(&self, id: u64) -> Option<Snapshot> {
        self.lock().entries.iter().find(|e| e.request.id == id).map(|e| e.shown.clone())
    }

    /// The person is done with the request: it leaves the queue.
    pub fn close(&self, id: u64) {
        self.lock().entries.retain(|e| e.request.id != id);
    }

    /// Answer `agent.listen` and `agent.reply` for connection `conn`; `None` for any other method.
    pub fn handle(&self, conn: u64, method: &str, p: &Value, alive: impl Fn() -> bool) -> Option<Value> {
        let reply = match method {
            "agent.listen" => self.handle_listen(conn, p, alive),
            "agent.reply" => self.handle_reply(conn, p),
            _ => return None,
        };
        Some(match reply {
            Ok(result) => json!({"ok": true, "result": result}),
            Err(e) => json!({"ok": false, "error": e.message, "errorCode": e.code}),
        })
    }

    fn handle_listen(&self, conn: u64, p: &Value, alive: impl Fn() -> bool) -> Result<Value, HubError> {
        let wait = match p.get("timeoutMs") {
            None | Some(Value::Null) => DEFAULT_LISTEN,
            Some(v) => v.as_u64().map(Duration::from_millis).ok_or_else(|| err(INVALID_PARAMS, "`timeoutMs` is a number of milliseconds"))?,
        };
        let request = self.listen(conn, str_param(p, "agent")?, wait.min(MAX_LISTEN), alive);
        Ok(request.map_or_else(|| json!({"none": true}), |r| json!(r)))
    }

    fn handle_reply(&self, conn: u64, p: &Value) -> Result<Value, HubError> {
        let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| err(INVALID_PARAMS, "missing `id`: the request's id"))?;
        let done = match p.get("done") {
            None | Some(Value::Null) => true,
            Some(v) => v.as_bool().ok_or_else(|| err(INVALID_PARAMS, "`done` is true or false"))?,
        };
        let (text, error) = (str_param(p, "text")?.unwrap_or(""), str_param(p, "error")?);
        if text.trim().is_empty() && error.is_none() {
            return Err(err(INVALID_PARAMS, "missing `text`: what to show in the window"));
        }
        self.reply(conn, id, text, done, error)?;
        Ok(json!({"id": id, "done": done}))
    }
}

/// An optional string parameter.
pub fn str_param<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>, HubError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(err(INVALID_PARAMS, format!("`{key}` is a string"))),
    }
}

/// A stable colour for an agent's name, from a palette readable on light and dark themes.
pub fn colour(name: &str) -> [u8; 3] {
    const PALETTE: [[u8; 3]; 8] =
        [[37, 99, 235], [5, 150, 105], [217, 119, 6], [219, 39, 119], [124, 58, 237], [8, 145, 178], [220, 38, 38], [101, 163, 13]];
    let hash = name.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    PALETTE[hash as usize % PALETTE.len()]
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const NOW: Duration = Duration::ZERO;

    fn req(text: &str, agent: Option<&str>) -> Request {
        Request {
            id: 0,
            instruction: text.into(),
            history: vec![],
            selection: json!({"text": text}),
            document: json!({}),
            agent: agent.map(Into::into),
        }
    }

    #[test]
    fn a_request_goes_to_one_agent_who_answers_it() {
        let hub = AgentHub::default();
        let id = hub.ask(req("Tighten", None));
        let taken = hub.listen(1, Some("Claude"), NOW, || true).unwrap();
        assert_eq!((taken.id, hub.snapshot(id).unwrap().status), (id, Status::Working { conn: 1 }));
        assert!(hub.listen(2, Some("Codex"), NOW, || true).is_none(), "taken once");
        hub.reply(1, id, "Halfway", false, None).unwrap();
        hub.reply(1, id, "Done", true, None).unwrap();
        let s = hub.snapshot(id).unwrap();
        assert_eq!(
            (s.status, s.notes, s.answer.as_deref(), s.agent.as_deref()),
            (Status::Answered, vec!["Halfway".into()], Some("Done"), Some("Claude"))
        );
        assert_eq!(hub.reply(2, id, "x", true, None).unwrap_err().code, NOT_OWNER);
    }

    #[test]
    fn only_the_working_connection_edits_while_a_request_is_open() {
        let hub = AgentHub::default();
        assert_eq!(hub.may_edit(9, None), Ok(()), "nothing open: anyone edits");
        let id = hub.ask(req("Fix", None));
        hub.listen(1, None, NOW, || true).unwrap();
        for (conn, request, code) in [(1, Some(id), None), (2, Some(id), Some(NOT_OWNER)), (1, None, Some(HELD)), (1, Some(99), Some(NO_REQUEST))] {
            assert_eq!(hub.may_edit(conn, request).err().map(|e| e.code), code, "{conn} {request:?}");
        }
        assert_eq!(hub.held(), vec![(json!({"text": "Fix"}), "Agent".to_string())]);
        hub.reply(1, id, "Done", true, None).unwrap();
        assert_eq!(hub.may_edit(1, Some(id)).unwrap_err().code, READ_ONLY_REQUEST);
        assert!(hub.held().is_empty());
    }

    #[test]
    fn a_dropped_agent_hands_its_request_back_and_routing_follows_names() {
        let hub = AgentHub::default();
        let id = hub.ask(req("Review", Some("Codex")));
        assert!(hub.listen(1, Some("Claude"), NOW, || true).is_none(), "asked for Codex");
        hub.listen(2, Some("Codex"), NOW, || true).unwrap();
        assert_eq!(hub.agents(), vec!["Claude".to_string(), "Codex".to_string()]);
        hub.disconnect(2);
        assert_eq!((hub.snapshot(id).unwrap().status, hub.agents()), (Status::Queued, vec!["Claude".to_string()]));
        assert_eq!(hub.listen(3, Some("Codex"), NOW, || true).unwrap().id, id);
    }

    #[test]
    fn the_wire_methods_validate_their_parameters() {
        let hub = AgentHub::default();
        assert_eq!(hub.handle(1, "agent.listen", &json!({"timeoutMs": 0}), || true).unwrap()["result"], json!({"none": true}));
        let id = hub.ask(req("Go", None));
        assert_eq!(hub.handle(1, "agent.listen", &json!({"agent": "Claude", "timeoutMs": 0}), || true).unwrap()["result"]["id"], id);
        for bad in [json!({"text": "x"}), json!({"id": id}), json!({"id": id, "text": 3}), json!({"id": id, "text": "x", "done": "yes"})] {
            assert_eq!(hub.handle(1, "agent.reply", &bad, || true).unwrap()["errorCode"], INVALID_PARAMS, "{bad}");
        }
        assert_eq!(hub.handle(1, "agent.reply", &json!({"id": id, "text": "Done"}), || true).unwrap()["ok"], true);
        assert!(hub.handle(1, "app.save", &json!({}), || true).is_none());
    }

    #[test]
    fn colours_are_stable_and_spread() {
        assert_eq!(colour("Claude"), colour("Claude"));
        assert_ne!(colour("Claude"), colour("Codex"));
    }
}
