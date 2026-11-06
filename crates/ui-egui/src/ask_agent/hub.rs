//! The request queue: requests from the window, the agents that listen for them, and who may edit.
//! No egui and no app types, so the control channel threads can share it.

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const NOT_OWNER: &str = "NOT_OWNER";
pub const NO_REQUEST: &str = "NO_REQUEST";
pub const INVALID_PARAMS: &str = "INVALID_PARAMS";
pub const READ_ONLY_REQUEST: &str = "READ_ONLY_REQUEST";
pub const HELD: &str = "HELD";
/// The requested document or revision is no longer the one captured by the request.
pub const STALE_REQUEST: &str = "STALE_REQUEST";
/// The app's selection predicate rejected a proposed edit.
pub const OUTSIDE_SELECTION: &str = "OUTSIDE_SELECTION";
/// A configured resource limit was reached; no partial request or reply was stored.
pub const LIMIT_EXCEEDED: &str = "LIMIT_EXCEEDED";

const MAX_NAME_BYTES: usize = 256;
const MAX_PROPOSAL_BYTES: usize = 200_000;
const MAX_REPLY_CHARS: usize = 4_000;
const MAX_COMMENT_CHARS: usize = 2_000;
const MAX_FORMAT_BYTES: usize = 64;

/// Limits for retained requests, serialized inputs and agent responses.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Includes queued, running and completed requests until the person closes them.
    pub requests: usize,
    /// Serialized bytes of one request, including its selection, document and history.
    pub request_bytes: usize,
    /// Combined bytes of a response's text, error, proposal, format and comment.
    pub reply_bytes: usize,
    /// Total bytes of progress notes retained by one request.
    pub progress_bytes: usize,
    /// Maximum number of progress notes retained by one request.
    pub progress_notes: usize,
    /// Maximum number of registered agent connections.
    pub listeners: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { requests: 128, request_bytes: 1024 * 1024, reply_bytes: 256 * 1024, progress_bytes: 64 * 1024, progress_notes: 128, listeners: 64 }
    }
}

/// An app-owned document identity and monotonically increasing content revision.
///
/// This token comes from the document owner, never from agent-supplied JSON. The app must
/// allocate a new identity when opening/replacing a document, even when its path is reused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Stable for one opened document, independent of the active tab or its filename.
    pub document: String,
    /// Increased after every content edit, including undo and redo.
    pub revision: u64,
}

/// The text of a progress note, a completed answer, or an agent-reported error.
#[derive(Clone, Copy, Debug)]
pub struct Reply<'a> {
    /// Plain text; never markup, commands or a document mutation.
    pub text: &'a str,
    /// False for a progress note; an error always completes the request.
    pub done: bool,
    /// A plain-text failure message, if the operation failed.
    pub error: Option<&'a str>,
    /// An unapplied proposal. It is displayed and saved as text, never executed.
    pub proposal: Option<&'a str>,
    /// Descriptive metadata only; it never selects a renderer or parser.
    pub format: Option<&'a str>,
    /// A note to retain with the response, displayed as literal text.
    pub comment: Option<&'a str>,
}

impl Default for Reply<'_> {
    fn default() -> Self {
        Self { text: "", done: true, error: None, proposal: None, format: None, comment: None }
    }
}

/// Longest `agent.listen` wait.
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

/// Build a refusal without exposing document contents.
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
    /// The document and content revision captured with the selection.
    pub target: Target,
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
    pub proposal: Option<String>,
    pub format: Option<String>,
    pub comment: Option<String>,
}

impl Snapshot {
    /// Plain text for app-native comments. Keeping this note never executes its proposal.
    pub fn review_text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(answer) = &self.answer
            && !answer.is_empty()
        {
            parts.push(answer.clone());
        }
        if let Some(error) = &self.error {
            parts.push(format!("Error: {error}"));
        }
        if let Some(comment) = &self.comment {
            parts.push(comment.clone());
        }
        if let Some(proposal) = &self.proposal {
            parts.push("Proposal (not applied)".into());
            if let Some(format) = &self.format {
                parts.push(format!("Format: {format}"));
            }
            parts.push(proposal.clone());
        } else if let Some(format) = &self.format {
            parts.push(format!("Format: {format}"));
        }
        parts.join("\n\n")
    }
}

struct Entry {
    request: Request,
    shown: Snapshot,
    progress_bytes: usize,
}

#[derive(Default)]
struct Inner {
    last_id: u64,
    entries: Vec<Entry>,
    /// Connections listening, with the name each gave.
    listeners: HashMap<u64, String>,
}

impl Inner {
    /// Find an existing request; closed requests cannot be resurrected by a late response.
    fn entry(&mut self, id: u64) -> Result<&mut Entry, HubError> {
        self.entries.iter_mut().find(|e| e.request.id == id).ok_or_else(|| err(NO_REQUEST, format!("no Ask Agent request {id}")))
    }

    /// Requeue only work still owned by this disconnected connection.
    fn disconnect(&mut self, conn: u64) {
        self.listeners.remove(&conn);
        for e in self.entries.iter_mut().filter(|e| e.shown.status == Status::Working { conn }) {
            (e.shown.status, e.shown.agent) = (Status::Queued, None);
            (e.shown.proposal, e.shown.format, e.shown.comment) = (None, None, None);
            e.shown.notes.clear();
            e.progress_bytes = 0;
        }
    }
}

/// The queue, shared by the window and the control channel's connection threads.
#[derive(Clone)]
pub struct AgentHub {
    shared: Arc<(Mutex<Inner>, Condvar)>,
    limits: Limits,
}

impl Default for AgentHub {
    fn default() -> Self {
        Self::with_limits(Limits::default())
    }
}

impl AgentHub {
    /// Construct a hub with application-owned bounds. Zero limits reject all relevant input.
    pub fn with_limits(limits: Limits) -> Self {
        Self { shared: Arc::default(), limits }
    }

    /// Recover a poisoned lock without discarding request ownership.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.shared.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue a request from the window; returns its id.
    pub fn ask(&self, mut request: Request) -> Result<u64, HubError> {
        if request.instruction.trim().is_empty() || request.target.document.trim().is_empty() {
            return Err(err(INVALID_PARAMS, "an instruction and a stable document identity are required"));
        }
        check_name(&request.target.document)?;
        if let Some(agent) = &mut request.agent {
            *agent = agent.trim().to_string();
            check_name(agent)?;
        }
        let mut inner = self.lock();
        if inner.entries.len() >= self.limits.requests {
            return Err(err(LIMIT_EXCEEDED, "close an Ask Agent request before opening another"));
        }
        let id = inner.last_id.checked_add(1).ok_or_else(|| err(LIMIT_EXCEEDED, "Ask Agent request ids are exhausted"))?;
        request.id = id;
        check_size(&request, self.limits.request_bytes)?;
        let shown = Snapshot {
            id: request.id,
            status: Status::Queued,
            agent: None,
            notes: vec![],
            answer: None,
            error: None,
            proposal: None,
            format: None,
            comment: None,
        };
        inner.entries.push(Entry { request, shown, progress_bytes: 0 });
        inner.last_id = id;
        self.shared.1.notify_all();
        Ok(id)
    }

    /// Wait up to `wait` for a request this agent may take, and take it.
    /// Connection ids must be unique for each connection lifetime; never reuse a disconnected id.
    /// An omitted or blank name keeps this connection's registered name, or starts as "Agent".
    pub fn listen(&self, conn: u64, name: Option<&str>, wait: Duration, alive: impl Fn() -> bool) -> Result<Option<Request>, HubError> {
        let supplied = name.map(str::trim).filter(|name| !name.is_empty());
        if let Some(name) = supplied {
            check_name(name)?;
        }
        // Single-threaded browser builds cannot block on a condition variable.
        // Other targets count down without reading a clock, which wasm builds lack.
        let mut left = if cfg!(all(target_arch = "wasm32", target_os = "unknown", not(target_feature = "atomics"))) {
            Duration::ZERO
        } else {
            wait.min(MAX_LISTEN)
        };
        let mut inner = self.lock();
        if !alive() {
            inner.disconnect(conn);
            self.shared.1.notify_all();
            return Ok(None);
        }
        if !inner.listeners.contains_key(&conn) && inner.listeners.len() >= self.limits.listeners {
            return Err(err(LIMIT_EXCEEDED, "too many Ask Agent connections"));
        }
        let name = supplied.map(str::to_owned).or_else(|| inner.listeners.get(&conn).cloned()).unwrap_or_else(|| "Agent".into());
        inner.listeners.insert(conn, name.clone());
        loop {
            // Never assign queued work to a connection that died while we waited.
            if !alive() || !inner.listeners.contains_key(&conn) {
                inner.disconnect(conn);
                self.shared.1.notify_all();
                return Ok(None);
            }
            let free = |e: &&mut Entry| e.shown.status == Status::Queued && e.request.agent.as_ref().is_none_or(|a| *a == name);
            if let Some(e) = inner.entries.iter_mut().find(free) {
                e.shown.status = Status::Working { conn };
                e.shown.agent = Some(name);
                return Ok(Some(e.request.clone()));
            }
            if left.is_zero() {
                return Ok(None);
            }
            let step = left.min(Duration::from_millis(250));
            inner = self.shared.1.wait_timeout(inner, step).unwrap_or_else(PoisonError::into_inner).0;
            left -= step;
        }
    }

    /// A progress note (`done` false), the answer, or a failure, from the agent working on `id`.
    /// The app resolves `current` from its own document state and accepts the reply in the same
    /// serialized document operation. Socket threads must dispatch this to the document owner.
    /// A missing/changed document fails the request and drops the response, including progress.
    pub fn reply(&self, conn: u64, id: u64, reply: Reply<'_>, current: Option<&Target>) -> Result<(), HubError> {
        let mut inner = self.lock();
        let entry = inner.entry(id)?;
        check_owner(entry, conn)?;
        check_target(entry, current)?;
        check_reply(reply, self.limits.reply_bytes)?;
        let Reply { text, done, error, proposal, format, comment } = reply;
        if error.is_none() && !done {
            if entry.shown.notes.len() >= self.limits.progress_notes || text.len() > self.limits.progress_bytes.saturating_sub(entry.progress_bytes) {
                return Err(err(LIMIT_EXCEEDED, "too many Ask Agent progress notes"));
            }
            entry.progress_bytes += text.len();
        }
        let shown = &mut entry.shown;
        if let Some(proposal) = proposal {
            shown.proposal = Some(proposal.into());
        }
        if let Some(format) = format {
            shown.format = Some(format.into());
        }
        if let Some(comment) = comment {
            shown.comment = Some(comment.into());
        }
        match (error, done) {
            (Some(e), _) => (shown.status, shown.error) = (Status::Failed, Some(e.into())),
            (None, true) => (shown.status, shown.answer) = (Status::Answered, Some(text.into())),
            (None, false) if !text.is_empty() => shown.notes.push(text.into()),
            (None, false) => {}
        }
        Ok(())
    }

    /// Authorize an edit against app-owned current document state and selection geometry.
    ///
    /// `contains` must inspect the actual proposed command against the captured selection;
    /// it must not trust a selection supplied by the agent. The app serializes this check,
    /// the command and [`Self::advance`] on its document thread. No document mutation is
    /// performed here, and no permission or prompt can replace the app's command checks.
    pub fn may_edit(&self, conn: u64, request: Option<u64>, current: &Target, contains: impl FnOnce(&Value) -> bool) -> Result<(), HubError> {
        let mut inner = self.lock();
        let open = inner
            .entries
            .iter()
            .any(|e| e.request.target.document == current.document && matches!(e.shown.status, Status::Queued | Status::Working { .. }));
        match request {
            None if open => Err(err(HELD, "the document is held for an Ask Agent request: pass its `request` id")),
            None => Ok(()),
            Some(id) => {
                let entry = inner.entry(id)?;
                check_owner(entry, conn)?;
                check_target(entry, Some(current))?;
                if !contains(&entry.request.selection) {
                    return Err(err(OUTSIDE_SELECTION, "the edit is outside the request's selection"));
                }
                Ok(())
            }
        }
    }

    /// Advance a request after its authorized edit, rebasing the app-owned held selection.
    /// Other requests for that document are invalidated against the new revision.
    pub fn advance(&self, conn: u64, id: u64, previous: &Target, next: Target, selection: Value) -> Result<(), HubError> {
        let mut inner = self.lock();
        let entry = inner.entry(id)?;
        check_owner(entry, conn)?;
        check_target(entry, Some(previous))?;
        if next.document != previous.document || next.revision <= previous.revision {
            return Err(err(INVALID_PARAMS, "an edit must advance the same document's revision"));
        }
        let mut updated = entry.request.clone();
        updated.target = next.clone();
        updated.selection = selection;
        check_size(&updated, self.limits.request_bytes)?;
        entry.request = updated;
        invalidate_entries(&mut inner, &next);
        self.shared.1.notify_all();
        Ok(())
    }

    /// Notify the hub after any other document edit or replacement. Stale work cannot resume.
    pub fn invalidate(&self, current: &Target) {
        invalidate_entries(&mut self.lock(), current);
        self.shared.1.notify_all();
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
        inner.disconnect(conn);
        self.shared.1.notify_all();
    }

    /// Names of the agents listening, sorted, for the picker.
    pub fn agents(&self) -> Vec<String> {
        let mut names: Vec<String> = self.lock().listeners.values().cloned().collect();
        names.sort();
        names.dedup();
        names
    }

    /// Return bounded display state without exposing the request's document payload.
    pub fn snapshot(&self, id: u64) -> Option<Snapshot> {
        self.lock().entries.iter().find(|e| e.request.id == id).map(|e| e.shown.clone())
    }

    /// Resolve a request's document for routing, including its latest authorized revision.
    /// This is not current app state or authorization; edits still require [`Self::may_edit`].
    pub fn request_target(&self, id: u64) -> Option<Target> {
        self.lock().entries.iter().find(|e| e.request.id == id).map(|e| e.request.target.clone())
    }

    /// The person is done with the request: it leaves the queue.
    pub fn close(&self, id: u64) {
        self.lock().entries.retain(|e| e.request.id != id);
        self.shared.1.notify_all();
    }

    /// Answer `agent.listen` and `agent.reply` for connection `conn`; `None` for any other method.
    /// `current` resolves the captured document from trusted app state, not the active tab.
    pub fn handle(
        &self,
        conn: u64,
        method: &str,
        p: &Value,
        alive: impl Fn() -> bool,
        current: impl FnOnce(&Target) -> Option<Target>,
    ) -> Option<Value> {
        let reply = match method {
            "agent.listen" => self.handle_listen(conn, p, alive),
            "agent.reply" => self.handle_reply(conn, p, current),
            _ => return None,
        };
        Some(match reply {
            Ok(result) => json!({"ok": true, "result": result}),
            Err(e) => json!({"ok": false, "error": e.message, "errorCode": e.code}),
        })
    }

    /// Validate listen parameters before registering or assigning a connection.
    fn handle_listen(&self, conn: u64, p: &Value, alive: impl Fn() -> bool) -> Result<Value, HubError> {
        check_object(p)?;
        let wait = match p.get("timeoutMs") {
            None | Some(Value::Null) => DEFAULT_LISTEN,
            Some(v) => v.as_u64().map(Duration::from_millis).ok_or_else(|| err(INVALID_PARAMS, "`timeoutMs` is a number of milliseconds"))?,
        };
        let request = self.listen(conn, str_param(p, "agent")?, wait, alive)?;
        Ok(request.map_or_else(|| json!({"none": true}), |r| json!(r)))
    }

    /// Resolve the expected document afresh for every progress note and final response.
    fn handle_reply(&self, conn: u64, p: &Value, current: impl FnOnce(&Target) -> Option<Target>) -> Result<Value, HubError> {
        check_object(p)?;
        let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| err(INVALID_PARAMS, "missing `id`: the request's id"))?;
        let done = match p.get("done") {
            None | Some(Value::Null) => true,
            Some(v) => v.as_bool().ok_or_else(|| err(INVALID_PARAMS, "`done` is true or false"))?,
        };
        let (text, error) = (str_param(p, "text")?.unwrap_or(""), str_param(p, "error")?);
        let (proposal, format, comment) = (str_param(p, "proposal")?, str_param(p, "format")?, str_param(p, "comment")?);
        let target = self.lock().entry(id)?.request.target.clone();
        let current = current(&target);
        self.reply(conn, id, Reply { text, done, error, proposal, format, comment }, current.as_ref())?;
        Ok(json!({"id": id, "done": done || error.is_some()}))
    }
}

/// Refuse non-object wire parameters before looking up optional fields.
fn check_object(p: &Value) -> Result<(), HubError> {
    if !p.is_object() {
        return Err(err(INVALID_PARAMS, "parameters must be an object"));
    }
    Ok(())
}

/// Names and document identities are short, nonempty application identifiers.
fn check_name(name: &str) -> Result<(), HubError> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err(err(INVALID_PARAMS, "identities must contain 1 to 256 bytes"));
    }
    Ok(())
}

/// Check identity before disclosing target-specific state to a connection.
fn check_owner(entry: &Entry, conn: u64) -> Result<(), HubError> {
    match entry.shown.status {
        Status::Working { conn: owner } if owner == conn => Ok(()),
        Status::Answered | Status::Failed => Err(err(READ_ONLY_REQUEST, "the Ask Agent request is finished")),
        _ => Err(err(NOT_OWNER, "listen for this Ask Agent request first")),
    }
}

/// Stale work is terminal: neither disconnect nor a later reply can revive it.
fn check_target(entry: &mut Entry, current: Option<&Target>) -> Result<(), HubError> {
    if current != Some(&entry.request.target) {
        fail_stale(entry);
        return Err(err(STALE_REQUEST, "the request's document changed or was closed"));
    }
    Ok(())
}

/// Invalidate the content revision without retaining any untrusted late response.
fn fail_stale(entry: &mut Entry) {
    entry.shown.status = Status::Failed;
    entry.shown.answer = None;
    (entry.shown.proposal, entry.shown.format, entry.shown.comment) = (None, None, None);
    entry.shown.error = Some("The document changed. Start a new Ask Agent request.".into());
}

/// Keep completed requests visible while invalidating queued and working stale requests.
fn invalidate_entries(inner: &mut Inner, current: &Target) {
    for entry in &mut inner.entries {
        if entry.request.target.document == current.document
            && entry.request.target.revision != current.revision
            && matches!(entry.shown.status, Status::Queued | Status::Working { .. })
        {
            fail_stale(entry);
        }
    }
}

/// Validate the complete answer before changing status or retaining any text.
fn check_reply(reply: Reply<'_>, limit: usize) -> Result<(), HubError> {
    let Reply { text, error, proposal, format, comment, .. } = reply;
    if (text.trim().is_empty() && error.is_none() && proposal.is_none_or(|proposal| proposal.trim().is_empty()))
        || error.is_some_and(|error| error.trim().is_empty())
    {
        return Err(err(INVALID_PARAMS, "a reply needs text, an error, or a nonempty proposal"));
    }
    if text.chars().count() > MAX_REPLY_CHARS
        || error.is_some_and(|text| text.chars().count() > MAX_REPLY_CHARS)
        || proposal.is_some_and(|text| text.len() > MAX_PROPOSAL_BYTES)
        || comment.is_some_and(|text| text.chars().count() > MAX_COMMENT_CHARS)
        || format.is_some_and(|text| text.len() > MAX_FORMAT_BYTES)
    {
        return Err(err(LIMIT_EXCEEDED, "an Ask Agent response field is too large"));
    }
    let bytes = [Some(text), error, proposal, format, comment].iter().flatten().try_fold(0_usize, |total, text| total.checked_add(text.len()));
    if bytes.is_none_or(|bytes| bytes > limit) {
        return Err(err(LIMIT_EXCEEDED, "the Ask Agent response is too large"));
    }
    Ok(())
}

/// Count serialized bytes without allocating another copy of the document.
struct BoundedWriter(usize);

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| io::Error::other("request size limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serialize only into a counter, stopping as soon as the configured size is exceeded.
fn check_size(request: &Request, limit: usize) -> Result<(), HubError> {
    serde_json::to_writer(BoundedWriter(limit), request).map_err(|_| err(LIMIT_EXCEEDED, "the Ask Agent request is too large"))
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
#[path = "tests/hub.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/regression.rs"]
mod regression_tests;
