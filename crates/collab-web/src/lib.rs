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
use wordcraft_collab_app::{
    CATCH_UP_REASON, CopyBuffer, Divergence, Outcome, RESYNC_REASON, SnapshotBuffer, SyncState, WelcomeState, apply_presence,
};
pub use wordcraft_collab_app::{Commit, CommitCopy, Cursor, ParticipantView, Restore, Status};
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
    /// The host repeated the message for the connection this shell already
    /// holds (a fresh JWT before the old one expires, ADR WC-0007). Update the
    /// credentials on the existing [`Collab`]; do not build a second one.
    Renew(HostSession),
    /// The cabinet stored the commit; show its version and etag.
    Committed { version: u64, etag: String },
    /// The `session_id` was opened for another document than the one this tab
    /// shows. Joining it would carry the open document's edits into that
    /// document's room (§5.26 finding 9), so the message is not stored and
    /// nothing dials. The shell says so; the host sends a `session` for the
    /// document it opened when this one is to be shared.
    Refused { session_id: String, document: u64 },
}

/// The host-side collaboration state the shell keeps between frames.
///
/// It is deliberately transport-free, so the `wordcraft:session` /
/// `wordcraft:committed` handling is tested headlessly without a browser.
///
/// A connection belongs to one document (§5.26 finding 9): it joined a room
/// with that document as its seed and sends that document's operations. The
/// state therefore remembers which document every `session_id` was opened for,
/// and answers [`HostAction::Refused`] when a message tries to move one to
/// another document, so a socket opened for a window's first document never
/// carries a second one's edits.
#[derive(Default, Debug)]
pub struct HostState {
    /// The session the host opened for the document this tab shows, if any.
    /// Without it the app stays single-user and makes no network call.
    pub session: Option<HostSession>,
    /// The last commit the cabinet confirmed.
    pub committed: Option<(u64, String)>,
    /// Whether the socket has joined the room at least once.
    pub connected: bool,
    /// The document this tab shows, and the one [`Self::session`] answers for.
    document: Option<u64>,
    /// Which document each `session_id` was opened for, oldest first.
    owners: Vec<(String, u64)>,
}

/// Most `session_id → document` bindings one tab remembers. A document switch
/// costs one; a host that sends more sessions than this falls back to refusing
/// the forgotten ones, which is the safe side.
const MAX_SESSION_OWNERS: usize = 64;

impl HostState {
    /// Apply one host message for `document`, the document this tab shows.
    /// Anything else leaves the app single-user.
    ///
    /// A `session` message for the connection this shell already holds — the
    /// same `session_id` at the same `ws_url` for the same document — is a
    /// credential renewal, not a new session: it answers [`HostAction::Renew`]
    /// and leaves `connected` alone, so the shell hands the token to the live
    /// client instead of replacing it and losing its unconfirmed edits (ADR
    /// WC-0007). The token is what rotates, so it is not part of the identity;
    /// a different URL is a different room and a different session. A different
    /// `session_id` for a document the tab has already briefly shown is a new
    /// room too, and opens a connection.
    pub fn on_event(&mut self, event: HostEvent, document: u64) -> HostAction {
        match event {
            HostEvent::Session(session) => {
                // A session id belongs to the document it was opened for. A
                // repeat for another document is not this tab's to join: the
                // room would receive this document's edits under that
                // document's name (§5.26 finding 9).
                if let Some(owner) = self.owner_of(session.session_id())
                    && owner != document
                {
                    return HostAction::Refused { session_id: session.session_id().to_string(), document: owner };
                }
                let same_connection = self.document == Some(document)
                    && self
                        .session
                        .as_ref()
                        .is_some_and(|current| current.session_id() == session.session_id() && current.ws_url() == session.ws_url());
                self.remember(session.session_id(), document);
                self.document = Some(document);
                self.session = Some(session.clone());
                if same_connection {
                    HostAction::Renew(session)
                } else {
                    self.connected = false;
                    HostAction::Connect(session)
                }
            }
            HostEvent::Committed { version, etag } => {
                self.committed = Some((version, etag.clone()));
                HostAction::Committed { version, etag }
            }
        }
    }

    /// The tab now shows `document`. `Some` is the session that must be closed:
    /// it answered for the document that was open before, and a socket that
    /// stayed would carry this document's edits into the room it joined
    /// (§5.26 finding 9). Its binding stays remembered, so a late renewal for
    /// it is refused rather than joined here. `None` means nothing is live.
    ///
    /// The shell opens a connection for the new document when the host sends a
    /// `session` for it — the contract's order is document first, then session
    /// (ADR WC-0004), which a switch repeats.
    pub fn document_changed(&mut self, document: u64) -> Option<HostSession> {
        if self.document == Some(document) {
            return None;
        }
        self.document = Some(document);
        let left = self.session.take();
        if left.is_some() {
            self.connected = false;
        }
        left
    }

    /// The document the live session answers for, once the shell has said
    /// which one it shows.
    pub fn document(&self) -> Option<u64> {
        self.document
    }

    fn owner_of(&self, session_id: &str) -> Option<u64> {
        self.owners.iter().find(|(id, _)| id == session_id).map(|(_, document)| *document)
    }

    fn remember(&mut self, session_id: &str, document: u64) {
        if let Some(entry) = self.owners.iter_mut().find(|(id, _)| id == session_id) {
            entry.1 = document;
            return;
        }
        if self.owners.len() >= MAX_SESSION_OWNERS {
            self.owners.remove(0);
        }
        self.owners.push((session_id.to_string(), document));
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
        // A diverged session asked for a snapshot and is on its way back; the
        // panel and `last_error` carry the reason, and the cabinet keeps its
        // spinner instead of freezing on an error (finding 8).
        Status::Connecting | Status::Resyncing | Status::Diverged => "pending",
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
    /// The document this connection answers for (§5.26 finding 9): the tab's
    /// document at the moment the host's `session` opened it. Its seed, its
    /// presence and its operations are that document's, so the shell closes it
    /// rather than let another document's edits into the room.
    document_id: u64,
    server: String,
    token: String,
    status: Status,
    /// An edit the room never received: tracked apart from `status`, which a
    /// closed socket or a failed resync frame moves to `Offline`.
    divergence: Divergence,
    participants: Vec<ParticipantView>,
    commit: Option<Commit>,
    /// The cabinet refused the last commit (`409`); the panel offers the room's
    /// file as a copy (TASK.md §6).
    conflict: Option<String>,
    copy: Option<CommitCopy>,
    copy_buffer: CopyBuffer,
    snapshots: SnapshotBuffer,
    /// The document this tab has open, serialized the way the server snapshots
    /// a room. Offered while nothing is accepted yet, so a room the server has
    /// never snapshotted starts from it instead of blank (ADR WC-0004).
    ///
    /// Only the browser dials, so only the wasm build reads it: a native build
    /// has no socket (`send` refuses).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    seed: Vec<u8>,
    last_presence_ms: f64,
    last_error: Option<String>,
    /// The queue a restored room took from this tab (ADR WC-0008). The snapshot
    /// that follows clears the status and not this: the author keeps seeing what
    /// the restore cost until the session ends.
    restore: Option<Restore>,
    /// Operations planned before `welcome`; replayed through [`Client::op`] so
    /// their ids stay ordered. They wait for the document to reach the room's
    /// head: a welcome that announces a `tail` is not that moment.
    waiting: VecDeque<(OpKind, Target, Value)>,
    /// Whether this connection has already retried its unconfirmed operations.
    /// One retry per connection: their ids are unchanged, so the room answers a
    /// duplicate rather than applying it twice (ADR-0003).
    resent: bool,
    #[cfg(target_arch = "wasm32")]
    transport: socket::Transport,
}

impl Collab {
    /// Dial the session the host sent. The token goes in the first frame, never
    /// in the URL. `document_id` is the document the seed came from: the
    /// connection answers for it and for no other (§5.26 finding 9).
    #[cfg(target_arch = "wasm32")]
    pub fn connect(
        session: &HostSession,
        actor: &str,
        client_id: &str,
        document: &wordcraft_doc::Document,
        document_id: u64,
    ) -> Result<Collab, String> {
        let role = Role::parse(session.role()).ok_or_else(|| format!("unknown collaboration role {:?}", session.role()))?;
        let seed = serde_json::to_vec(document).map_err(|err| format!("cannot encode the open document: {err}"))?;
        let transport = socket::Transport::open(session.ws_url())?;
        Ok(Collab {
            client: Client::new(client_id),
            state: SyncState::new(client_id, actor, role),
            session_id: session.session_id().to_string(),
            document_id,
            server: session.ws_url().to_string(),
            token: session.token().to_string(),
            status: Status::Connecting,
            divergence: Divergence::default(),
            participants: Vec::new(),
            commit: None,
            conflict: None,
            copy: None,
            copy_buffer: CopyBuffer::default(),
            snapshots: SnapshotBuffer::default(),
            seed,
            last_presence_ms: 0.0,
            last_error: None,
            restore: None,
            waiting: VecDeque::new(),
            resent: false,
            transport,
        })
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The document this connection answers for.
    pub fn document_id(&self) -> u64 {
        self.document_id
    }

    /// Whether this connection belongs to `document` (§5.26 finding 9). The
    /// shell asks before it polls or sends: a connection that answers for a
    /// document no longer open must be closed, not reused, or that document's
    /// room would receive the newly open one's edits.
    pub fn carries(&self, document: u64) -> bool {
        self.document_id == document
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

    /// The queue a restored room took from this tab (ADR WC-0008), for the
    /// panel's line about the edits that are no longer in the document.
    pub fn restore(&self) -> Option<&Restore> {
        self.restore.as_ref()
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
                    self.resent = false;
                    for frame in self.join_frames() {
                        let _ = self.send(&frame);
                    }
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
        let resync = matches!(&event, Event::Resync { .. } | Event::Conflict { .. });
        self.on_event(event, session);
        if resync {
            self.request_snapshot();
        }
        self.flush_queue();
        true
    }

    /// Hand the queue and the unconfirmed retries to the room once this
    /// document is at the room's head. A welcome that announces a `tail` is not
    /// that moment: an operation planned then would name a base this tab never
    /// received (CodeRabbit on the `welcome` arm). The retry runs once per
    /// connection; queued operations leave in order, so their ids stay ordered.
    fn flush_queue(&mut self) {
        if self.client.state() != wordcraft_collab_client::State::Joined || self.status != Status::InSync {
            return;
        }
        if !self.resent {
            self.resent = true;
            for message in self.client.unconfirmed() {
                let _ = self.send(&message);
            }
        }
        while let Some((kind, target, payload)) = self.waiting.pop_front() {
            let op = self.client.op(kind, target, payload);
            let _ = self.send(&ClientMsg::Op { op });
        }
    }

    fn on_event(&mut self, event: Event, session: &mut Session) {
        match event {
            Event::Welcome { version, resync, role, participants, dropped, catching_up, .. } => {
                // The room's head is this base once the document has it (ADR
                // WC-0008): a restored room answers below it, and keeping the
                // higher number would plan the next operation on a document the
                // room no longer has. While the missed frames are on their way
                // the document is still at the version it applied.
                if !catching_up {
                    self.state.version = version;
                }
                self.state.role = Role::parse(&role).unwrap_or(self.state.role);
                self.set_participants(&participants);
                // A handshake ends any half-received snapshot: a retry at the
                // same version must not append to its bytes.
                self.snapshots.clear();
                // A handshake is not a snapshot: `Offline`, a renewal or a
                // closed socket never cleared the divergence, so this reads the
                // flag, not the status (review 5853542478).
                let welcome = WelcomeState::after_welcome(self.divergence, version, dropped, resync, catching_up);
                self.status = welcome.status;
                if self.status == Status::Resyncing {
                    // The window between the welcome and the frames that follow
                    // is named: an edit here is refused, and the panel says why
                    // (ADR-0025).
                    let reason = if catching_up && !resync { CATCH_UP_REASON } else { RESYNC_REASON };
                    self.last_error = Some(reason.to_string());
                }
                if let Some(restore) = welcome.restore {
                    // The panel draws the count and the ids; the session reads
                    // `Resyncing` until the snapshot lands (ADR WC-0008).
                    self.restore = Some(restore);
                }
                if self.divergence.is_paused() {
                    // Reconnecting does not replace this document: the edit the
                    // room never received is still in it. Stay paused and ask
                    // for the snapshot that repairs it (finding 8).
                    self.request_snapshot();
                }
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
                // A refusal that reads `InSync` leaves a paused session paused:
                // only a snapshot clears the divergence (review 5855379673).
                self.status = self.divergence.status_after_refusal(status);
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
                // A document over the frame-sized chunk arrives in several
                // frames; only the one marked `last` completes the snapshot, and
                // the earlier chunks are the same document, not stale answers.
                if let Some(document) = self.snapshots.push(version, &bytes, last) {
                    match self.apply_snapshot(version, &document, session) {
                        Ok(version) => {
                            self.state.version = version;
                            // The document is the room's now: the local edit
                            // that never reached it is gone with the document it
                            // was in.
                            self.divergence.snapshot();
                            self.status = Status::InSync;
                            self.last_error = None;
                        }
                        // A snapshot that cannot be opened leaves this tab
                        // without the room's state; `InSync` would enable
                        // editing a document the room does not have.
                        Err(message) => {
                            self.last_error = Some(message.clone());
                            self.status = Status::Failed(message);
                        }
                    }
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
            Event::Tail { entries, version } => {
                // The reconnect was answered with the operations this tab
                // missed, not with a snapshot: each carries an authoritative
                // delta and has to be applied like a live answer.
                self.state.version = version;
                let mut resync = false;
                for applied in entries {
                    match self.state.apply(&applied, session) {
                        Ok(Outcome::Quiet) => {}
                        Ok(Outcome::Resync) => {
                            self.status = Status::Resyncing;
                            self.request_snapshot();
                            resync = true;
                        }
                        Err(message) => {
                            self.status = Status::Failed(message);
                            resync = true;
                        }
                    }
                }
                // The catch-up landed: the document is at the room's head, and a
                // paused session stays paused until a snapshot repairs it.
                if !resync {
                    self.status = self.divergence.status_after_welcome();
                    if self.status == Status::InSync {
                        self.last_error = None;
                    }
                }
            }
            Event::Resync { reason } => {
                self.status = Status::Resyncing;
                self.last_error = Some(reason);
            }
            Event::None => {}
        }
    }

    /// Replace the document with the room's, from a snapshot the buffer has
    /// completed.
    fn apply_snapshot(&mut self, version: Version, document: &[u8], session: &mut Session) -> Result<Version, String> {
        let doc: wordcraft_doc::Document = serde_json::from_slice(document).map_err(|err| format!("bad snapshot: {err}"))?;
        session.set_document(doc);
        Ok(version)
    }

    fn presence_of(&mut self, client_id: &str, role: &str, state: &Value) {
        apply_presence(&mut self.participants, client_id, role, state);
    }

    /// Apply a local command: the caller hands the document from before the
    /// command; the session sends the primitives or the command itself.
    ///
    /// A command that cannot be planned or sent has already changed this
    /// document and never reached the room: the session leaves `InSync` so no
    /// commit can build the room's version without it (finding 8).
    pub fn after_command(&mut self, before: &wordcraft_doc::Document, session: &Session, command: &str, params: &Value) -> Result<(), String> {
        if !self.can_write() {
            return Ok(());
        }
        let plans = match wordcraft_collab_app::plan_command(&self.state, before, session, command, params) {
            Ok(plans) => plans,
            Err(pause) => return Err(self.pause(pause)),
        };
        let result = wordcraft_collab_app::submit_plans(plans, |plan| self.deliver(plan));
        match result {
            Ok(()) => Ok(()),
            Err(pause) => Err(self.pause(pause)),
        }
    }

    /// Hand one planned operation to the socket, or queue it until `welcome`.
    fn deliver(&mut self, plan: wordcraft_collab_app::Outgoing) -> Result<(), String> {
        if self.client.state() == wordcraft_collab_client::State::Joined {
            let op = self.client.op(plan.kind, plan.target, plan.payload);
            self.send(&ClientMsg::Op { op })
        } else {
            self.waiting.push_back((plan.kind, plan.target, plan.payload));
            Ok(())
        }
    }

    /// Show the pause and ask for the room's state. The snapshot that follows
    /// replaces this document, so an edit the room never received stops being
    /// shown as synchronized (finding 8). A snapshot that cannot be asked for
    /// means there is no open socket; the transport reconnects and the status
    /// follows it rather than pretending the edit was sent.
    fn pause(&mut self, pause: wordcraft_collab_app::Pause) -> String {
        self.last_error = Some(pause.message.clone());
        self.divergence.pause();
        self.status = pause.status;
        if self.send(&ClientMsg::Resync { last_version: self.client.version() }).is_err() {
            // The socket is gone, not the divergence: `Offline` describes the
            // connection and must not clear what the document still holds
            // (review 5853542478).
            self.status = Status::Offline;
        }
        pause.message
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

    /// Take a repeated `session` message for this connection (ADR WC-0007): the
    /// cabinet pushes a fresh JWT before the old one expires, with the same
    /// `session_id`. Only the credentials change — the client keeps its
    /// unconfirmed operations, its version and the participant list, so the
    /// writer's edit and presence survive the rotation. An unchanged token is
    /// not a reason to drop the socket, so that repeat costs nothing.
    ///
    /// `false` means the message is not for this connection (another
    /// `session_id`); the caller installs a new [`Collab`] instead.
    pub fn renew(&mut self, session: &HostSession) -> bool {
        if self.session_id != session.session_id() {
            return false;
        }
        self.server = session.ws_url().to_string();
        if self.token != session.token() {
            self.renew_token(session.token());
        }
        true
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
    /// The frames that open a connection: the auth, followed by the document
    /// this tab has open while nothing is accepted yet (ADR WC-0004). A room
    /// that already holds state ignores the seed and answers with its snapshot.
    fn join_frames(&mut self) -> Vec<ClientMsg> {
        let seeding = self.client.version() == 0;
        let last = if seeding { None } else { Some(self.client.version()) };
        let mut frames = vec![self.client.auth(&self.token, last, seeding)];
        if seeding {
            frames.extend(wordcraft_collab_client::seed_frames(&self.seed));
        }
        frames
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
    use wordcraft_collab::wire::OpId;

    /// The document these tests have open; the session is scoped to it.
    const DOC: u64 = 7;

    #[test]
    fn a_session_message_starts_a_connection_and_keeps_the_token_out_of_debug() {
        let mut state = HostState::default();
        assert!(!state.has_session());
        let session = HostSession::new("s1", "wss://collab.example/ws", "jwt-secret", "editor");
        assert_eq!(state.on_event(HostEvent::Session(session.clone()), DOC), HostAction::Connect(session));
        assert!(state.has_session());
        let debug = format!("{:?}", state.session.as_ref().expect("the session is stored"));
        assert!(debug.contains("s1"), "{debug}");
        assert!(!debug.contains("jwt-secret"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }

    /// A web session as the shell holds it, built without a socket: the native
    /// build has no transport, and these tests drive the state machine.
    #[cfg(not(target_arch = "wasm32"))]
    fn test_collab(session: &HostSession, client_id: &str) -> Collab {
        test_collab_for(session, client_id, DOC)
    }

    /// The same, for a connection opened with `document` as its seed.
    #[cfg(not(target_arch = "wasm32"))]
    fn test_collab_for(session: &HostSession, client_id: &str, document: u64) -> Collab {
        Collab {
            client: Client::new(client_id),
            state: SyncState::new(client_id, "writer", Role::Editor),
            session_id: session.session_id().to_string(),
            document_id: document,
            server: session.ws_url().to_string(),
            token: session.token().to_string(),
            status: Status::InSync,
            divergence: Divergence::default(),
            participants: Vec::new(),
            commit: None,
            conflict: None,
            copy: None,
            copy_buffer: CopyBuffer::default(),
            snapshots: SnapshotBuffer::default(),
            seed: Vec::new(),
            last_presence_ms: 0.0,
            last_error: None,
            restore: None,
            waiting: VecDeque::new(),
            resent: false,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_repeated_session_renews_the_same_connection_and_keeps_an_unconfirmed_edit() {
        const URL: &str = "wss://collab.example/ws";
        let mut host = HostState::default();
        let opened = HostSession::new("s1", URL, "jwt-1", "editor");
        assert_eq!(host.on_event(HostEvent::Session(opened.clone()), DOC), HostAction::Connect(opened.clone()));
        let mut live = Some(test_collab(&opened, "c1"));
        if let Some(collab) = live.as_mut() {
            // The writer's edit, planned and sent, not answered yet.
            collab.client.op(OpKind::Primitive, Target::new("body", 7, 0), json!({"replaceRange": {"text": "x"}}));
            collab.participants.push(ParticipantView {
                client_id: "c2".into(),
                role: "viewer".into(),
                color: None,
                cursor: None,
                selection: None,
                active: true,
            });
            assert_eq!(collab.client.unconfirmed().len(), 1);
        }
        // The cabinet pushes a fresh JWT before the old one expires.
        let renewed = HostSession::new("s1", URL, "jwt-2", "editor");
        let action = host.on_event(HostEvent::Session(renewed.clone()), DOC);
        assert_eq!(action, HostAction::Renew(renewed.clone()));
        let mut dials = 0;
        if let HostAction::Renew(session) = action {
            let in_place = live.as_mut().is_some_and(|collab| collab.renew(&session));
            if !in_place {
                live = Some(test_collab(&session, "c1"));
                dials += 1;
            }
        }
        assert_eq!(dials, 0, "a renewal for the live session must not dial a second connection");
        assert!(live.is_some());
        if let Some(collab) = live.as_ref() {
            assert_eq!(collab.session_id(), "s1");
            assert_eq!(collab.token, "jwt-2", "the new credentials are in the live client");
            assert_eq!(collab.status, Status::Connecting, "the transport re-dials with them");
            assert_eq!(collab.client_id(), "c1", "the room still knows this tab by the same id");
            assert_eq!(collab.client.unconfirmed().len(), 1, "the edit the room has not confirmed is still queued");
            assert_eq!(collab.participants.len(), 1, "presence survives the rotation");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_repeated_session_with_the_same_token_keeps_the_socket() {
        const URL: &str = "wss://collab.example/ws";
        let mut host = HostState::default();
        let opened = HostSession::new("s1", URL, "jwt-1", "editor");
        let _ = host.on_event(HostEvent::Session(opened.clone()), DOC);
        let mut collab = test_collab(&opened, "c1");
        // The same message again, unchanged: nothing to renew, nothing to drop.
        let repeat = HostSession::new("s1", URL, "jwt-1", "editor");
        let action = host.on_event(HostEvent::Session(repeat.clone()), DOC);
        assert_eq!(action, HostAction::Renew(repeat.clone()));
        if let HostAction::Renew(session) = action {
            assert!(collab.renew(&session));
        }
        assert_eq!(collab.status, Status::InSync, "an unchanged token is no reason to reconnect");
        assert_eq!(collab.token, "jwt-1");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_session_for_another_id_still_replaces_the_connection() {
        const URL: &str = "wss://collab.example/ws";
        let mut host = HostState::default();
        let opened = HostSession::new("s1", URL, "jwt-1", "editor");
        let _ = host.on_event(HostEvent::Session(opened.clone()), DOC);
        host.mark_connected();
        let other = HostSession::new("s2", URL, "jwt-2", "editor");
        assert_eq!(host.on_event(HostEvent::Session(other.clone()), DOC), HostAction::Connect(other.clone()));
        assert!(!host.connected, "a new session has not joined yet");
        // And the live client does not take another session's message in place.
        let mut collab = test_collab(&opened, "c1");
        assert!(!collab.renew(&other));
        assert_eq!(collab.token, "jwt-1");
        assert_eq!(collab.session_id(), "s1");
    }

    #[test]
    fn a_session_for_another_url_is_a_new_connection() {
        let mut host = HostState::default();
        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "editor");
        let _ = host.on_event(HostEvent::Session(opened.clone()), DOC);
        assert!(!host.connected);
        host.mark_connected();
        let moved = HostSession::new("s1", "wss://collab.example/other", "jwt-2", "editor");
        assert_eq!(host.on_event(HostEvent::Session(moved.clone()), DOC), HostAction::Connect(moved.clone()));
        assert!(!host.connected, "another room is another connection");
    }

    /// Two documents in one tab do not share a connection (§5.26 finding 9).
    /// Document 7's session is closed when document 8 becomes active, its
    /// unconfirmed edit stays in its own queue and never reaches document 8's
    /// connection, and a late renewal for it is refused instead of joining
    /// document 8 into document 7's room.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn two_documents_in_one_tab_neither_carry_each_others_edits_nor_share_the_queue() {
        const URL: &str = "wss://collab.example/ws";
        let mut host = HostState::default();
        let first = HostSession::new("s-7", URL, "jwt-7", "editor");
        assert_eq!(host.on_event(HostEvent::Session(first.clone()), 7), HostAction::Connect(first.clone()));
        // Document 7's connection plans an edit the room has not answered.
        let mut live = Some(test_collab_for(&first, "tab-7", 7));
        if let Some(collab) = live.as_mut() {
            collab.client.op(OpKind::Primitive, Target::new("body", 3, 0), json!({"replaceRange": {"text": "seven"}}));
            assert_eq!(collab.client.pending().len(), 1);
            assert!(collab.carries(7), "the connection answers for the document it joined with");
        }
        // The tab opens document 8: the shell closes document 7's connection.
        let left = host.document_changed(8).expect("the connection that answered for document 7 must be closed");
        assert_eq!(left.session_id(), "s-7");
        assert!(!host.has_session(), "no session answers for document 8");
        live = None;
        assert!(live.is_none(), "the socket and its queue go with the closed connection");
        // A late renewal for document 7's session, while document 8 is open, is
        // refused: joining it here would send document 8's edits into room s-7.
        let late = HostSession::new("s-7", URL, "jwt-7b", "editor");
        assert_eq!(host.on_event(HostEvent::Session(late), 8), HostAction::Refused { session_id: "s-7".into(), document: 7 });
        assert!(!host.has_session(), "a refused session is not stored");
        // Document 8's own session connects with an empty queue, and its
        // connection does not answer for document 7 either.
        let second = HostSession::new("s-8", URL, "jwt-8", "editor");
        assert_eq!(host.on_event(HostEvent::Session(second.clone()), 8), HostAction::Connect(second.clone()));
        let fresh = test_collab_for(&second, "tab-8", 8);
        assert!(fresh.client.pending().is_empty(), "document 8 starts with no edit of document 7's");
        assert!(fresh.carries(8));
        assert!(!fresh.carries(7));
        // Switching back to document 7 waits for the host to say so again: its
        // own session is still refused while another document is shown.
        assert_eq!(host.on_event(HostEvent::Session(first.clone()), 8), HostAction::Refused { session_id: "s-7".into(), document: 7 });
        assert_eq!(host.on_event(HostEvent::Session(first.clone()), 7), HostAction::Connect(first), "document 7's session is its own again");
    }

    #[test]
    fn a_commit_message_is_recorded_for_the_ui() {
        let mut state = HostState::default();
        let action = state.on_event(HostEvent::Committed { version: 7, etag: "\"e7\"".into() }, DOC);
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

    /// A pause that could not even send its resync is still a pause: the status
    /// falls to `Offline` with the socket, and the divergence must survive it
    /// until a snapshot replaces the document (CodeRabbit 4533747527).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_pause_survives_a_closed_socket_until_the_snapshot_lands() {
        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "editor");
        let mut collab = test_collab(&opened, "c1");
        let mut session = Session::new(wordcraft_doc::Document::new());

        // The native build has no socket, so the resync frame cannot be sent.
        collab.pause(wordcraft_collab_app::Pause::after_failed_command("the socket closed"));
        assert_eq!(*collab.status(), Status::Offline, "the socket status follows the socket");

        // The handshake after a reconnect arrives before the snapshot does.
        let welcome =
            ServerMsg::Welcome { session_id: "s1".into(), role: "editor".into(), version: 3, epoch: 0, participants: Vec::new(), resync: true };
        let text = serde_json::to_string(&welcome).expect("json");
        collab.on_text(&text, &mut session);
        assert_eq!(*collab.status(), Status::Diverged, "a welcome is not the snapshot that repairs the edit");

        // The snapshot replaces the document, and with it the divergence.
        let bytes = serde_json::to_vec(&wordcraft_doc::Document::new()).expect("model");
        let snapshot = ServerMsg::Snapshot { version: 3, bytes, last: true };
        let text = serde_json::to_string(&snapshot).expect("json");
        collab.on_text(&text, &mut session);
        assert_eq!(*collab.status(), Status::InSync, "the room's document is this document again");
    }

    /// A room restored from an older snapshot answers below this tab's base and
    /// takes its unconfirmed queue with it (ADR WC-0008). The panel needs the
    /// head and the ids, and the session must not read `InSync` until the
    /// snapshot replaces the document the edits were in.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_restored_room_is_named_with_its_count_and_ids_and_leaves_in_sync() {
        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "editor");
        let mut collab = test_collab(&opened, "c1");
        let mut session = Session::new(wordcraft_doc::Document::new());
        let welcome = |version: u64, resync: bool| {
            serde_json::to_string(&ServerMsg::Welcome {
                session_id: "s1".into(),
                role: "editor".into(),
                version,
                epoch: 0,
                participants: Vec::new(),
                resync,
            })
            .expect("json")
        };
        // The tab joins at 3: the welcome announces the room's state and the
        // snapshot that follows is the document, so the base reaches 3 there.
        collab.on_text(&welcome(3, true), &mut session);
        let joined = serde_json::to_vec(&wordcraft_doc::Document::new()).expect("model");
        let joined = serde_json::to_string(&ServerMsg::Snapshot { version: 3, bytes: joined, last: true }).expect("json");
        collab.on_text(&joined, &mut session);
        assert_eq!(collab.version(), 3, "the snapshot's version is the base editing starts from");

        // Two edits the room has not answered yet.
        collab.client.op(OpKind::Primitive, Target::new("body", 7, 0), json!({"replaceRange": {"text": "a"}}));
        collab.client.op(OpKind::Primitive, Target::new("body", 7, 0), json!({"replaceRange": {"text": "b"}}));
        assert_eq!(collab.client.pending().len(), 2);

        // The room comes back from an older snapshot: head 1, below the base.
        collab.on_text(&welcome(1, true), &mut session);
        assert_eq!(*collab.status(), Status::Resyncing, "the snapshot that replaces the document has not landed");
        assert!(!collab.can_write(), "an edit would be planned on a document the room no longer has");
        assert_eq!(collab.version(), 3, "the announcement does not move the base: this document is still the one at 3");
        let restore = collab.restore().cloned().expect("the queue was taken and is reported");
        assert_eq!(restore.head(), 1);
        assert_eq!(restore.count(), 2);
        assert_eq!(restore.dropped(), [OpId::new("c1", 1), OpId::new("c1", 2)], "the ids the panel draws");

        // The snapshot lands: the session is usable again, the notice stays.
        let bytes = serde_json::to_vec(&wordcraft_doc::Document::new()).expect("model");
        let snapshot = serde_json::to_string(&ServerMsg::Snapshot { version: 1, bytes, last: true }).expect("json");
        collab.on_text(&snapshot, &mut session);
        assert_eq!(*collab.status(), Status::InSync);
        assert!(collab.can_write());
        assert_eq!(collab.restore().map(Restore::count), Some(2), "the cost does not disappear with the status");
    }

    /// An ordinary handshake takes nothing: no notice, no paused session.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_welcome_without_dropped_operations_reports_nothing() {
        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "editor");
        let mut collab = test_collab(&opened, "c1");
        let mut session = Session::new(wordcraft_doc::Document::new());
        let welcome = serde_json::to_string(&ServerMsg::Welcome {
            session_id: "s1".into(),
            role: "editor".into(),
            version: 0,
            epoch: 0,
            participants: Vec::new(),
            resync: false,
        })
        .expect("json");
        collab.on_text(&welcome, &mut session);
        assert!(collab.restore().is_none(), "nothing was taken");
        assert_eq!(*collab.status(), Status::InSync);
        assert_eq!(collab.last_error(), None, "an ordinary handshake is not an error");
    }

    /// ADR-0025: a welcome with `resync: true` is the room's word that its
    /// document is coming and will replace this one — whether or not a restore
    /// emptied the queue. The tab reads `Resyncing` from that moment, so the host
    /// gate refuses the edit before the engine runs, the document stays
    /// byte-for-byte the author's, and a commit is refused too. The last chunk of
    /// the snapshot is what reopens both gates.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_welcome_with_resync_holds_the_document_until_the_snapshot() {
        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "owner");
        let mut collab = test_collab(&opened, "c1");
        collab.state.role = Role::Owner;
        let mut session = Session::new(wordcraft_doc::Document::new());
        session.run("text.insert", &json!({"text": "Kept"})).expect("type");
        let before = serde_json::to_vec(&session.doc).expect("model");

        // The room answers a join at 4 and will send its document; nothing was
        // taken off the queue.
        let welcome =
            ServerMsg::Welcome { session_id: "s1".into(), role: "owner".into(), version: 4, epoch: 0, participants: Vec::new(), resync: true };
        collab.on_text(&serde_json::to_string(&welcome).expect("json"), &mut session);
        assert_eq!(*collab.status(), Status::Resyncing, "the document is on its way and this one will be replaced");
        assert!(collab.restore().is_none(), "no queue was taken: the state is the snapshot, not a restore");
        assert_eq!(collab.last_error(), Some(RESYNC_REASON), "the panel names the reason");

        // Both gates read the status: the host refuses the edit before the engine
        // runs, and the room would refuse a commit of the document the snapshot
        // is replacing. The document is untouched and nothing was queued.
        assert!(!collab.can_write(), "an edit planned now would be dropped by the snapshot");
        assert!(!collab.can_commit(), "a commit would build the document the snapshot replaces");
        assert_eq!(serde_json::to_vec(&session.doc).expect("model"), before, "the document is byte-for-byte the author's");
        assert!(collab.client.pending().is_empty(), "nothing was queued for a base the room is replacing");

        // The snapshot lands: one base, and both gates are open again on it.
        let bytes = serde_json::to_vec(&wordcraft_doc::Document::new()).expect("model");
        let snapshot = serde_json::to_string(&ServerMsg::Snapshot { version: 4, bytes, last: true }).expect("json");
        collab.on_text(&snapshot, &mut session);
        assert_eq!(*collab.status(), Status::InSync);
        assert!(collab.can_write() && collab.can_commit(), "the snapshot is what reopens the session");
        assert_eq!(collab.last_error(), None, "the reason goes with the window it described");
        assert_eq!(collab.version(), 4, "the base is the snapshot's version, the one editing continues at");
    }

    /// A head below the base with no snapshot promised cannot be joined: the
    /// client asks for the document and the host shows its reason instead of
    /// pretending the session is synchronized (ADR WC-0008, defensive branch).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_welcome_below_the_base_without_a_snapshot_shows_the_reason() {
        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "editor");
        let mut collab = test_collab(&opened, "c1");
        let mut session = Session::new(wordcraft_doc::Document::new());
        let frame = |version: u64, resync: bool| {
            serde_json::to_string(&ServerMsg::Welcome {
                session_id: "s1".into(),
                role: "editor".into(),
                version,
                epoch: 0,
                participants: Vec::new(),
                resync,
            })
            .expect("json")
        };
        collab.on_text(&frame(5, false), &mut session);
        collab.client.op(OpKind::Primitive, Target::new("body", 7, 0), json!({"replaceRange": {"text": "a"}}));
        collab.on_text(&frame(2, false), &mut session);
        assert_eq!(*collab.status(), Status::Resyncing);
        let reason = collab.last_error().unwrap_or_default();
        assert!(reason.contains("below this document's base"), "{reason}");
        assert!(collab.restore().is_none(), "the client answered with `Resync`, not a welcome");
    }

    /// The outstanding undo of a paused session is refused `undo-conflict` when
    /// its target moved. That refusal reads `InSync`, and writing it over the
    /// status would reopen both gates while the document still holds the edit the
    /// room never received (review 5855379673).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_refused_undo_does_not_reopen_a_paused_session() {
        use wordcraft_collab::wire::{OpId, Reject, RejectCode};

        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "owner");
        let mut collab = test_collab(&opened, "c1");
        collab.state.role = Role::Owner;
        let mut session = Session::new(wordcraft_doc::Document::new());
        assert!(collab.can_write() && collab.can_commit(), "an owner in sync writes and commits");

        collab.pause(wordcraft_collab_app::Pause::after_failed_command("the socket closed"));
        // What the handshake after a reconnect leaves: still paused.
        let welcome =
            ServerMsg::Welcome { session_id: "s1".into(), role: "owner".into(), version: 3, epoch: 0, participants: Vec::new(), resync: true };
        let text = serde_json::to_string(&welcome).expect("json");
        collab.on_text(&text, &mut session);
        assert_eq!(*collab.status(), Status::Diverged);

        let refusal = ServerMsg::Rejected { reject: Reject::new(OpId::new("c1", 3), RejectCode::UndoConflict, "the step's target changed") };
        let text = serde_json::to_string(&refusal).expect("json");
        collab.on_text(&text, &mut session);
        assert_eq!(*collab.status(), Status::Diverged, "the refusal is shown without clearing the pause");
        assert!(!collab.can_write(), "the editor stays paused");
        assert!(!collab.can_commit(), "and an owner commit would omit the lost edit");

        let bytes = serde_json::to_vec(&wordcraft_doc::Document::new()).expect("model");
        let snapshot = ServerMsg::Snapshot { version: 3, bytes, last: true };
        let text = serde_json::to_string(&snapshot).expect("json");
        collab.on_text(&text, &mut session);
        assert_eq!(*collab.status(), Status::InSync);
        assert!(collab.can_write() && collab.can_commit(), "the snapshot is what reopens the session");
    }
    /// A document over the frame-sized chunk arrives in several `snapshot`
    /// frames. Only the chunk marked `last` was parsed, so every larger document
    /// failed to open and the tab waited in `Resyncing` forever; the chunks
    /// belong to one snapshot and are joined in order.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn snapshot_frames_join_until_last_and_replace_the_document() {
        use wordcraft_doc::Document;
        use wordcraft_engine::Session;

        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "editor");
        let mut collab = test_collab(&opened, "c1");
        let mut session = Session::new(Document::new());
        session.run("text.insert", &json!({"text": "chunked"})).expect("the room's document has text");
        let model = serde_json::to_vec(&session.doc).expect("model");
        let (first, second) = model.split_at(model.len() / 2);
        assert!(!first.is_empty() && !second.is_empty(), "the test needs two frames");

        let mut mine = Session::new(Document::new());
        for (bytes, last) in [(first, false), (second, true)] {
            let frame = ServerMsg::Snapshot { version: 3, bytes: bytes.to_vec(), last };
            let text = serde_json::to_string(&frame).expect("json");
            collab.on_text(&text, &mut mine);
        }
        let text = mine.doc.body.first().and_then(|block| block.as_para()).map(|para| para.text.clone()).unwrap_or_default();
        assert_eq!(text, "chunked", "the whole snapshot opened");
        assert_eq!(*collab.status(), Status::InSync);
    }

    /// A reconnect inside the journal window is answered with the operations
    /// this tab missed, not with a snapshot. Confirming them without applying
    /// them left the document behind the version it reported, and no later
    /// answer exposed the gap.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_journal_tail_is_applied_like_a_live_answer() {
        use wordcraft_collab::wire::{Applied, NodeDelta, NodeRef};
        use wordcraft_doc::Document;
        use wordcraft_engine::Session;

        let opened = HostSession::new("s1", "wss://collab.example/ws", "jwt-1", "editor");
        let mut collab = test_collab(&opened, "c1");
        let mut mine = Session::new(Document::new());

        let welcome =
            ServerMsg::Welcome { session_id: "s1".into(), role: "editor".into(), version: 1, epoch: 0, participants: Vec::new(), resync: false };
        let text = serde_json::to_string(&welcome).expect("json");
        collab.on_text(&text, &mut mine);

        let nodes = ServerMsg::Nodes { nodes: vec![NodeRef { id: 7, story: "body".into(), path: vec![0] }] };
        let text = serde_json::to_string(&nodes).expect("json");
        collab.on_text(&text, &mut mine);

        let tail = ServerMsg::Tail {
            entries: vec![Applied {
                op_id: OpId::new("bob", 1),
                version: 2,
                epoch: 0,
                delta: Some(json!({"op": "insert_text", "nodeId": 7, "off": 0, "text": "from bob"})),
                nodes: Some(NodeDelta::default()),
                conflict: false,
                history: None,
            }],
        };
        let text = serde_json::to_string(&tail).expect("json");
        collab.on_text(&text, &mut mine);

        let text = mine.doc.body.first().and_then(|block| block.as_para()).map(|para| para.text.clone()).unwrap_or_default();
        assert_eq!(text, "from bob", "the missed edit is in the document");
        assert_eq!(*collab.status(), Status::InSync);
    }
}
