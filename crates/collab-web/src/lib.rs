//! Web-side collaboration session.
//!
//! The browser build gets its session from the cabinet through the host bridge
//! (`docs/collab/host-bridge.md`): `Message::Session` carries `session_id`,
//! `ws_url`, the JWT and the role. This crate keeps the same transport-free
//! state machine as the desktop (`wordcraft-collab-app::SyncState`), dials the
//! URL with the browser's `WebSocket`, and exposes a `Collab` with the same
//! surface the participants panel and the command gate already use.
//!
//! The token lives in this object and nowhere else: it is never put in the
//! socket URL, a log line, `localStorage` or preferences, and [`HostSession`]'s
//! `Debug` prints it as `<redacted>`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::VecDeque;
use std::fmt;

use serde_json::{Value, json};

use wordcraft_collab::room::Role;
use wordcraft_collab::wire::{ClientMsg, OpKind, ServerMsg, Target, Version};
pub use wordcraft_collab_app::{Commit, CommitCopy, Cursor, ParticipantView, Status};
use wordcraft_collab_app::{CopyBuffer, Outcome, SyncState, apply_presence};
/// An id for one editing session: two apps on the same document must not share
/// an operation id space (`wordcraft_collab_client::session_client_id`).
pub use wordcraft_collab_client::session_client_id;
use wordcraft_collab_client::{Client, Event};
use wordcraft_doc::Pos as DocPos;
use wordcraft_engine::Session;

#[cfg(target_arch = "wasm32")]
mod socket;

/// The session the host bridge opened. A credential-bearing value, so it has a
/// redacted `Debug` and no `Serialize`: it must never reach preferences, a log
/// line or a URL.
#[derive(Clone, PartialEq, Eq)]
pub struct HostSession {
    session_id: String,
    ws_url: String,
    token: String,
    role: String,
}

impl HostSession {
    pub fn new(session_id: impl Into<String>, ws_url: impl Into<String>, token: impl Into<String>, role: impl Into<String>) -> HostSession {
        HostSession { session_id: session_id.into(), ws_url: ws_url.into(), token: token.into(), role: role.into() }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The sync server's WebSocket URL. The token is not part of it.
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Replace the JWT: the cabinet pushes a fresh one before the old expires.
    pub fn set_token(&mut self, token: impl Into<String>) {
        self.token = token.into();
    }
}

impl fmt::Debug for HostSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostSession")
            .field("session_id", &self.session_id)
            .field("ws_url", &self.ws_url)
            .field("token", &"<redacted>")
            .field("role", &self.role)
            .finish()
    }
}

/// What the host bridge delivered to the app.
#[derive(Clone, Debug)]
pub enum HostEvent {
    /// `wordcraft:session` (host → app): open a collaboration session.
    Session(HostSession),
    /// `wordcraft:committed` (host → app): the cabinet stored the commit.
    Committed { version: u64, etag: String },
}

/// What the app must do after one host event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostAction {
    /// Dial the session; the host opened it, so the app owns no network call
    /// before this point.
    Connect(HostSession),
    /// The cabinet stored the commit; show its version and etag.
    Committed { version: u64, etag: String },
}

/// The host-side collaboration state the shell keeps between frames.
///
/// It is deliberately transport-free, so the `wordcraft:session` /
/// `wordcraft:committed` handling is tested headlessly without a browser.
#[derive(Default, Debug)]
pub struct HostState {
    /// The session the host opened, if any. Without it the app stays
    /// single-user and makes no network call.
    pub session: Option<HostSession>,
    /// The last commit the cabinet confirmed.
    pub committed: Option<(u64, String)>,
    /// Whether the socket has joined the room at least once.
    pub connected: bool,
}

impl HostState {
    /// Apply one host message. Anything else leaves the app single-user.
    pub fn on_event(&mut self, event: HostEvent) -> HostAction {
        match event {
            HostEvent::Session(session) => {
                self.connected = false;
                self.session = Some(session.clone());
                HostAction::Connect(session)
            }
            HostEvent::Committed { version, etag } => {
                self.committed = Some((version, etag.clone()));
                HostAction::Committed { version, etag }
            }
        }
    }

    pub fn mark_connected(&mut self) {
        self.connected = true;
    }

    pub fn has_session(&self) -> bool {
        self.session.is_some()
    }
}

/// The cabinet's sync-status word (`session-state.status`): the contract names
/// `synced`, `pending`, `offline` and `error`, and the cabinet shows it as is.
pub fn host_status(status: &Status) -> &'static str {
    match status {
        Status::InSync => "synced",
        Status::Connecting | Status::Resyncing => "pending",
        Status::Offline => "offline",
        Status::Failed(_) => "error",
    }
}

/// One participant in the shape the host bridge sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostParticipant {
    pub client_id: String,
    pub role: String,
    pub color: Option<String>,
}

pub fn host_participants(participants: &[ParticipantView]) -> Vec<HostParticipant> {
    participants
        .iter()
        .map(|participant| HostParticipant {
            client_id: participant.client_id.clone(),
            role: participant.role.clone(),
            color: participant.color.clone(),
        })
        .collect()
}

/// A live web collaboration session.
pub struct Collab {
    client: Client,
    state: SyncState,
    session_id: String,
    server: String,
    token: String,
    status: Status,
    participants: Vec<ParticipantView>,
    commit: Option<Commit>,
    /// The cabinet refused the last commit (`409`); the panel offers the room's
    /// file as a copy (TASK.md §6).
    conflict: Option<String>,
    copy: Option<CommitCopy>,
    copy_buffer: CopyBuffer,
    snapshot: Vec<u8>,
    snapshot_version: Version,
    last_presence_ms: f64,
    last_error: Option<String>,
    /// Operations planned before `welcome`; replayed through [`Client::op`] so
    /// their ids stay ordered.
    waiting: VecDeque<(OpKind, Target, Value)>,
    #[cfg(target_arch = "wasm32")]
    transport: socket::Transport,
}

impl Collab {
    /// Dial the session the host sent. The token goes in the first frame, never
    /// in the URL.
    #[cfg(target_arch = "wasm32")]
    pub fn connect(session: &HostSession, actor: &str, client_id: &str) -> Result<Collab, String> {
        let role = Role::parse(session.role()).ok_or_else(|| format!("unknown collaboration role {:?}", session.role()))?;
        let transport = socket::Transport::open(session.ws_url())?;
        Ok(Collab {
            client: Client::new(client_id),
            state: SyncState::new(client_id, actor, role),
            session_id: session.session_id().to_string(),
            server: session.ws_url().to_string(),
            token: session.token().to_string(),
            status: Status::Connecting,
            participants: Vec::new(),
            commit: None,
            conflict: None,
            copy: None,
            copy_buffer: CopyBuffer::default(),
            snapshot: Vec::new(),
            snapshot_version: 0,
            last_presence_ms: 0.0,
            last_error: None,
            waiting: VecDeque::new(),
            transport,
        })
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn role(&self) -> Role {
        self.state.role
    }

    pub fn can_write(&self) -> bool {
        self.state.role.can_write() && self.status == Status::InSync
    }

    pub fn can_commit(&self) -> bool {
        self.state.role.can_commit() && self.status == Status::InSync
    }

    pub fn version(&self) -> Version {
        self.state.version
    }

    pub fn server(&self) -> &str {
        &self.server
    }

    pub fn participants(&self) -> &[ParticipantView] {
        &self.participants
    }

    pub fn last_commit(&self) -> Option<&Commit> {
        self.commit.as_ref()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Why the cabinet refused the last commit (`409`), while the panel may
    /// still offer its file as a copy (TASK.md §6).
    pub fn conflict(&self) -> Option<&str> {
        self.conflict.as_deref()
    }

    /// The refused file, once it has fully arrived; taken so the UI saves it
    /// exactly once.
    pub fn take_copy(&mut self) -> Option<CommitCopy> {
        self.copy.take()
    }

    /// Resolve a remote cursor for drawing.
    pub fn cursor_position(&self, cursor: &wordcraft_collab_app::Cursor) -> Option<DocPos> {
        self.state.position(cursor)
    }

    pub fn client_id(&self) -> &str {
        &self.state.client_id
    }

    /// Drain the socket and apply authoritative results to `session`.
    /// Returns whether the UI-visible state changed.
    #[cfg(target_arch = "wasm32")]
    pub fn poll(&mut self, session: &mut Session) -> bool {
        let mut changed = false;
        for incoming in self.transport.poll() {
            changed = true;
            match incoming {
                socket::Incoming::Opened => {
                    let auth = self.client.auth(&self.token, if self.client.version() > 0 { Some(self.client.version()) } else { None });
                    let _ = self.send(&auth);
                }
                socket::Incoming::Text(text) => {
                    changed |= self.on_text(&text, session);
                }
                socket::Incoming::Closed => {
                    self.status = Status::Offline;
                    self.last_error = Some("the socket closed; reconnecting".into());
                }
            }
        }
        changed
    }

    /// Handle one server frame. Public so tests can drive the state machine
    /// without a browser.
    pub fn on_text(&mut self, text: &str, session: &mut Session) -> bool {
        let Ok(message) = serde_json::from_str::<ServerMsg>(text) else { return false };
        let event = self.client.on(message);
        let welcome = matches!(&event, Event::Welcome { .. });
        let resync = matches!(&event, Event::Resync { .. } | Event::Conflict { .. });
        self.on_event(event, session);
        if welcome {
            for message in self.client.unconfirmed() {
                let _ = self.send(&message);
            }
            while let Some((kind, target, payload)) = self.waiting.pop_front() {
                let op = self.client.op(kind, target, payload);
                let _ = self.send(&ClientMsg::Op { op });
            }
        }
        if resync {
            self.request_snapshot();
        }
        true
    }

    fn on_event(&mut self, event: Event, session: &mut Session) {
        match event {
            Event::Welcome { version, role, participants, .. } => {
                self.state.version = self.state.version.max(version);
                self.state.role = Role::parse(&role).unwrap_or(self.state.role);
                self.set_participants(&participants);
                self.status = Status::InSync;
            }
            Event::Nodes { nodes } => self.state.set_nodes(&nodes),
            Event::Applied { applied } => match self.state.apply(&applied, session) {
                Ok(Outcome::Quiet) => {}
                Ok(Outcome::Resync) => {
                    self.status = Status::Resyncing;
                    self.request_snapshot();
                }
                Err(message) => self.status = Status::Failed(message),
            },
            Event::Rejected { reject } => {
                let (status, resync) = wordcraft_collab_app::refusal_status(&reject);
                self.status = status;
                // A refused undo is visible at its author; the document stays as
                // it was because the room refused before changing it.
                self.last_error = Some(reject.message.clone());
                if resync {
                    self.request_snapshot();
                }
            }
            Event::Presence { client_id, role, state } => self.presence_of(&client_id, &role, &state),
            Event::Left { client_id } => self.participants.retain(|participant| participant.client_id != client_id),
            Event::Snapshot { version, bytes, last } => {
                if last && let Ok(version) = self.take_snapshot(version, &bytes, session) {
                    self.state.version = version;
                    self.status = Status::InSync;
                    self.last_error = None;
                }
            }
            Event::Committed { version, sha256, warnings, cabinet } => {
                self.state.version = self.state.version.max(version);
                self.commit = Some(Commit { version, sha256, warnings, cabinet });
                self.conflict = None;
            }
            Event::Conflict { reason } => {
                // The cabinet refused the commit (409): the snapshot follows
                // (`on_text` asked for it), and the panel keeps the room's file
                // available as a copy (TASK.md §6).
                self.status = Status::Resyncing;
                self.last_error = Some(reason.clone());
                self.conflict = Some(reason);
            }
            Event::SaveCopy { version, sha256, warnings, bytes, last } => {
                if let Some(copy) = self.copy_buffer.push(version, sha256, warnings, &bytes, last) {
                    self.copy = Some(copy);
                }
            }
            Event::Resync { reason } => {
                self.status = Status::Resyncing;
                self.last_error = Some(reason);
            }
            Event::None => {}
        }
    }

    /// Accumulate snapshot chunks; the last one replaces the document.
    fn take_snapshot(&mut self, version: Version, bytes: &[u8], session: &mut Session) -> Result<Version, String> {
        if self.snapshot_version != version {
            self.snapshot_version = version;
            self.snapshot.clear();
        }
        self.snapshot.extend_from_slice(bytes);
        let doc: wordcraft_doc::Document = serde_json::from_slice(&self.snapshot).map_err(|err| format!("bad snapshot: {err}"))?;
        session.set_document(doc);
        self.snapshot.clear();
        Ok(version)
    }

    fn presence_of(&mut self, client_id: &str, role: &str, state: &Value) {
        apply_presence(&mut self.participants, client_id, role, state);
    }

    /// Apply a local command: the caller hands the document from before the
    /// command; the session sends the primitives or the command itself.
    pub fn after_command(&mut self, before: &wordcraft_doc::Document, session: &Session, command: &str, params: &Value) -> Result<(), String> {
        if !self.can_write() {
            return Ok(());
        }
        let plans = self.state.plan(before, session, command, params)?;
        for plan in plans {
            if self.client.state() == wordcraft_collab_client::State::Joined {
                let op = self.client.op(plan.kind, plan.target, plan.payload);
                self.send(&ClientMsg::Op { op })?;
            } else {
                self.waiting.push_back((plan.kind, plan.target, plan.payload));
            }
        }
        Ok(())
    }

    /// Publish the caret and selection at most every 100 ms (`now_ms` is the
    /// browser clock or a test's clock).
    pub fn publish_presence_at(&mut self, session: &Session, now_ms: f64) {
        if self.status != Status::InSync {
            return;
        }
        if now_ms - self.last_presence_ms < 100.0 {
            return;
        }
        let Some(cursor) = self.state.address(&session.sel.focus) else { return };
        let anchor = self.state.address(&session.sel.anchor).unwrap_or_else(|| cursor.clone());
        self.last_presence_ms = now_ms;
        let state = json!({
            "cursor": {"story": cursor.0, "nodeId": cursor.1, "off": cursor.2},
            "selection": {
                "anchor": {"story": anchor.0, "nodeId": anchor.1, "off": anchor.2},
                "focus": {"story": cursor.0, "nodeId": cursor.1, "off": cursor.2},
            },
            "active": true,
        });
        if self.client.state() == wordcraft_collab_client::State::Joined {
            let _ = self.send(&ClientMsg::Presence { state });
        }
    }

    /// The browser clock: `std::time::Instant::now()` panics on wasm32.
    #[cfg(target_arch = "wasm32")]
    pub fn publish_presence_now(&mut self, session: &Session) {
        self.publish_presence_at(session, js_sys::Date::now());
    }

    pub fn undo(&mut self) {
        if let Some(op) = self.client.undo() {
            let _ = self.send(&ClientMsg::Op { op });
        }
    }

    pub fn redo(&mut self) {
        if let Some(op) = self.client.redo() {
            let _ = self.send(&ClientMsg::Op { op });
        }
    }

    /// Ask the sync server to build the file. The host-owned commit path in
    /// `apps/wordcraft-web` posts `wordcraft:commit` to the cabinet instead.
    pub fn commit(&mut self) {
        if self.client.state() != wordcraft_collab_client::State::Joined {
            return;
        }
        let op_id = self.client.next_op_id();
        let message = ClientMsg::Commit { op_id, expected_version: self.client.version(), etag: None };
        let _ = self.send(&message);
    }

    /// Ask the room for the file of a commit the cabinet refused (`409`), to
    /// save it as a copy (TASK.md §6).
    pub fn request_copy(&mut self) {
        if self.client.state() != wordcraft_collab_client::State::Joined {
            return;
        }
        let message = self.client.save_copy();
        let _ = self.send(&message);
    }

    /// Replace the JWT and reconnect. Queued operations survive and are resent
    /// with their ids.
    pub fn renew_token(&mut self, token: &str) {
        self.token = token.to_string();
        self.status = Status::Connecting;
        #[cfg(target_arch = "wasm32")]
        self.transport.reconnect();
    }

    /// Add the server's own view of the participants after `welcome`.
    pub fn set_participants(&mut self, participants: &[wordcraft_collab::wire::Participant]) {
        for participant in participants {
            let view = ParticipantView {
                client_id: participant.client_id.clone(),
                role: participant.role.clone(),
                color: participant.color.clone(),
                cursor: participant.presence.as_ref().and_then(|state| state.get("cursor")).and_then(parse_cursor),
                selection: participant.presence.as_ref().and_then(|state| state.get("selection")).and_then(parse_selection),
                active: true,
            };
            match self.participants.iter_mut().find(|existing| existing.client_id == view.client_id) {
                Some(existing) => *existing = view,
                None => self.participants.push(view),
            }
        }
    }

    fn request_snapshot(&mut self) {
        let _ = self.send(&ClientMsg::Resync { last_version: self.client.version() });
    }

    #[cfg(target_arch = "wasm32")]
    fn send(&self, message: &ClientMsg) -> Result<(), String> {
        let text = serde_json::to_string(message).map_err(|err| format!("cannot encode a collaboration message: {err}"))?;
        self.transport.send(&text)
    }

    /// Native builds have no socket; the tests drive [`Collab::on_text`].
    #[cfg(not(target_arch = "wasm32"))]
    fn send(&self, _message: &ClientMsg) -> Result<(), String> {
        Err("the web transport is not available on this target".into())
    }
}

fn parse_cursor(value: &Value) -> Option<wordcraft_collab_app::Cursor> {
    Some(wordcraft_collab_app::Cursor {
        story: value.get("story")?.as_str()?.to_string(),
        node_id: u32::try_from(value.get("nodeId")?.as_u64()?).ok()?,
        off: usize::try_from(value.get("off")?.as_u64()?).ok()?,
    })
}

fn parse_selection(value: &Value) -> Option<(wordcraft_collab_app::Cursor, wordcraft_collab_app::Cursor)> {
    Some((parse_cursor(value.get("anchor")?)?, parse_cursor(value.get("focus")?)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_message_starts_a_connection_and_keeps_the_token_out_of_debug() {
        let mut state = HostState::default();
        assert!(!state.has_session());
        let session = HostSession::new("s1", "wss://collab.example/ws", "jwt-secret", "editor");
        assert_eq!(state.on_event(HostEvent::Session(session.clone())), HostAction::Connect(session));
        assert!(state.has_session());
        let debug = format!("{:?}", state.session.as_ref().expect("the session is stored"));
        assert!(debug.contains("s1"), "{debug}");
        assert!(!debug.contains("jwt-secret"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }

    #[test]
    fn a_commit_message_is_recorded_for_the_ui() {
        let mut state = HostState::default();
        let action = state.on_event(HostEvent::Committed { version: 7, etag: "\"e7\"".into() });
        assert_eq!(action, HostAction::Committed { version: 7, etag: "\"e7\"".into() });
        assert_eq!(state.committed, Some((7, "\"e7\"".to_string())));
    }

    #[test]
    fn without_a_session_the_app_stays_single_user() {
        let state = HostState::default();
        assert!(!state.has_session());
        assert!(state.committed.is_none());
    }

    #[test]
    fn status_words_follow_the_host_contract() {
        assert_eq!(host_status(&Status::InSync), "synced");
        assert_eq!(host_status(&Status::Connecting), "pending");
        assert_eq!(host_status(&Status::Resyncing), "pending");
        assert_eq!(host_status(&Status::Offline), "offline");
        assert_eq!(host_status(&Status::Failed("boom".into())), "error");
    }

    #[test]
    fn participants_keep_the_client_id_role_and_colour() {
        let people = [
            ParticipantView {
                client_id: "c1".into(),
                role: "owner".into(),
                color: Some("#112233".into()),
                cursor: None,
                selection: None,
                active: true,
            },
            ParticipantView { client_id: "c2".into(), role: "viewer".into(), color: None, cursor: None, selection: None, active: true },
        ];
        let host = host_participants(&people);
        assert_eq!(host[0], HostParticipant { client_id: "c1".into(), role: "owner".into(), color: Some("#112233".into()) });
        assert_eq!(host[1], HostParticipant { client_id: "c2".into(), role: "viewer".into(), color: None });
    }
}
