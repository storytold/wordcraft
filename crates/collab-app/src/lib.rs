//! App-side collaboration session.
//!
//! Wiring layer between the engine's `Session` and the transport: it keeps the
//! session node map, applies authoritative deltas (including command replays),
//! turns a local command into the operations the server expects, publishes
//! presence and exposes participants to the UI.
//!
//! The transport-free half ([`SyncState`], [`Status`], cursors and the
//! participant view) builds for both targets, so the web session in
//! `wordcraft-collab-web` reuses it. Only [`Collab`] — the native socket
//! session — is desktop-only (`docs/collab/host-bridge.md`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use wordcraft_collab::ids::{NodeId, NodeIds, NodeKey};
use wordcraft_collab::room::Role;
#[cfg(not(target_arch = "wasm32"))]
use wordcraft_collab::wire::Participant;
use wordcraft_collab::wire::{Applied, CabinetCommit, NodeRef, OpKind, Reject, RejectCode, Target, Version};
#[cfg(not(target_arch = "wasm32"))]
use wordcraft_collab_client::Event;
#[cfg(not(target_arch = "wasm32"))]
use wordcraft_collab_ws::{Handle, TransportEvent};
use wordcraft_doc::Pos as DocPos;
use wordcraft_doc::{Document, Path, StoryRef};
use wordcraft_engine::Session;

/// An id for one editing session: two apps on the same document must not share
/// an operation id space (`wordcraft_collab_client::session_client_id`).
#[cfg(not(target_arch = "wasm32"))]
pub use wordcraft_collab_client::session_client_id;

/// Where the session is in its handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting,
    InSync,
    Resyncing,
    Offline,
    Failed(String),
}

/// The last commit the room accepted, as the panel shows it. `cabinet` is set
/// when the room posted the file to the cabinet itself (M87): its version and
/// the etag the next commit is checked against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    /// The room's document version the commit was taken at.
    pub version: Version,
    /// SHA-256 of the built `.docx`.
    pub sha256: String,
    /// What the file could not keep (ADR-0011).
    pub warnings: Vec<String>,
    /// The cabinet's copy, when the room published the file.
    pub cabinet: Option<CabinetCommit>,
}

/// What a refusal means for the session. The room refuses before changing the
/// document, so a refused undo (`undo-conflict`) is not a broken session: it is
/// shown as an error and editing continues with its stack intact (ADR-0010). A
/// refusal that asks for a snapshot (`stale`, `gone`, an expired lease or
/// token) resyncs instead.
pub fn refusal_status(reject: &Reject) -> (Status, bool) {
    match reject.code {
        RejectCode::Stale | RejectCode::Lease | RejectCode::TokenExpired | RejectCode::Gone => (Status::Resyncing, true),
        RejectCode::UndoConflict | RejectCode::Unsupported => (Status::InSync, false),
        _ => (Status::Failed(reject.message.clone()), false),
    }
}

/// A participant's cursor: a session node and a byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub story: String,
    pub node_id: u32,
    pub off: usize,
}

/// What the UI shows in the participants list.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticipantView {
    pub client_id: String,
    pub role: String,
    pub color: Option<String>,
    pub cursor: Option<Cursor>,
    pub selection: Option<(Cursor, Cursor)>,
    pub active: bool,
}

/// What applying one authoritative answer requires from the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The replica is in sync.
    Quiet,
    /// The operation allocated ids the replica cannot derive; fetch a snapshot.
    Resync,
}

/// One operation the app should send for a local command.
#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    pub kind: OpKind,
    pub target: Target,
    pub payload: Value,
}

/// The protocol state of one app session, without a socket. The transport
/// owns the socket; this half is testable on its own.
pub struct SyncState {
    pub ids: NodeIds,
    pub client_id: String,
    pub actor: String,
    pub version: Version,
    pub role: Role,
}

impl SyncState {
    pub fn new(client_id: impl Into<String>, actor: impl Into<String>, role: Role) -> SyncState {
        SyncState { ids: NodeIds::new(), client_id: client_id.into(), actor: actor.into(), version: 0, role }
    }

    /// Replace the map with the server's session ids. Ids that disappeared are
    /// left alone: the delta path tombstones them when the removal is applied.
    pub fn set_nodes(&mut self, nodes: &[NodeRef]) {
        // The map may reorder ids (a move, a sort): replace all paths in one
        // pass so one id's new key cannot erase another's old one.
        self.ids.rebase_many(nodes.iter().map(|node| (NodeId(node.id), NodeKey::new(node.story.clone(), node.path.clone()))));
    }

    /// Apply an authoritative answer. Our own operations are already in the
    /// document (the UI applied them optimistically); their delta is skipped so
    /// the edit does not land twice. An undo/redo answer is the exception: its
    /// inverse is never applied optimistically (the caller sends the operation
    /// and the engine does not run), so this delta is the only copy and is
    /// applied exactly once (ADR-0010). Everything else is replayed.
    pub fn apply(&mut self, applied: &Applied, session: &mut Session) -> Result<Outcome, String> {
        self.version = self.version.max(applied.version);
        let command = applied.delta.as_ref().and_then(|delta| delta.get("command")).and_then(Value::as_str);
        // A command that creates parts/objects allocates ids on the server that
        // this replica cannot derive; only the authoritative snapshot has them.
        if command.is_some_and(allocates_ids) {
            return Ok(Outcome::Resync);
        }
        if applied.op_id.client == self.client_id && applied.history.is_none() {
            return Ok(Outcome::Quiet);
        }
        if let Some(delta) = &applied.delta {
            wordcraft_collab_engine::apply_delta(session, &mut self.ids, delta)?;
        }
        Ok(Outcome::Quiet)
    }

    /// Turn a local command into the operations the server expects: primitives
    /// derived from the document difference, or the command itself when the
    /// difference is not expressible as primitives (formatting, tables,
    /// several moves). `command` is only sent with an explicit selection, so
    /// the server never reads a caret from its own window.
    pub fn plan(&self, before: &Document, session: &Session, command: &str, params: &Value) -> Result<Vec<Outgoing>, String> {
        let as_command = || -> Result<Vec<Outgoing>, String> {
            let anchor = self.address(&session.sel.anchor).ok_or_else(|| format!("{command} needs a session id for its selection"))?;
            let focus = self.address(&session.sel.focus).unwrap_or_else(|| anchor.clone());
            let pending = serde_json::to_value(session.pending.clone()).unwrap_or(Value::Null);
            Ok(vec![Outgoing {
                kind: OpKind::Command,
                target: Target::new(anchor.0.clone(), anchor.1, anchor.2),
                payload: json!({
                    "command": command,
                    "params": params,
                    "pending": pending,
                    "selection": {
                        "anchor": {"nodeId": anchor.1, "off": anchor.2},
                        "focus": {"nodeId": focus.1, "off": focus.2},
                    },
                }),
            }])
        };
        // Track changes lives in the session, not in the text: a primitive would
        // lose the revision and its author, so every edit goes through the engine
        // command on the server.
        if session.doc.settings.track_changes {
            return as_command();
        }
        match wordcraft_collab_doc::diff(before, &session.doc, &self.ids) {
            Ok(changes) => {
                Ok(changes.into_iter().map(|change| Outgoing { kind: change.kind, target: change.target, payload: change.payload }).collect())
            }
            Err(wordcraft_collab_doc::DiffError::Unsupported(_)) => as_command(),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Resolve a document position to `(story, nodeId, off)`.
    pub fn address(&self, pos: &DocPos) -> Option<(String, u32, usize)> {
        let story = story_name(pos.story);
        let key = NodeKey::new(story.clone(), pos.path.0.clone());
        self.ids.get(&key).map(|id| (story, id.0, pos.off))
    }

    /// The document position of a participant's cursor, if the id is still live.
    pub fn position(&self, cursor: &Cursor) -> Option<DocPos> {
        let key = self.ids.resolve(NodeId(cursor.node_id)).ok()?;
        let story = parse_story(&key.story)?;
        Some(DocPos { story, path: Path(key.path.clone()), off: cursor.off })
    }
}

/// A live collaboration session: transport plus the state above.
#[cfg(not(target_arch = "wasm32"))]
pub struct Collab {
    _runtime: tokio::runtime::Runtime,
    handle: Handle,
    state: SyncState,
    server: String,
    status: Status,
    participants: Vec<ParticipantView>,
    commit: Option<Commit>,
    snapshot: Vec<u8>,
    snapshot_version: Version,
    last_presence: Option<Instant>,
    last_error: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Collab {
    /// Connect to `server` with a cabinet JWT. The token is kept by the
    /// transport and never logged, stored in preferences or put in the URL.
    pub fn connect(server: &str, token: &str, actor: &str, client_id: &str) -> Result<Collab, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|err| format!("cannot start the collaboration runtime: {err}"))?;
        let handle = {
            let _entered = runtime.enter();
            wordcraft_collab_ws::spawn(server.to_string(), token.to_string(), client_id.to_string())
        };
        Ok(Collab {
            _runtime: runtime,
            handle,
            state: SyncState::new(client_id, actor, Role::Editor),
            server: server.to_string(),
            status: Status::Connecting,
            participants: Vec::new(),
            commit: None,
            snapshot: Vec::new(),
            snapshot_version: 0,
            last_presence: None,
            last_error: None,
        })
    }

    pub fn status(&self) -> &Status {
        &self.status
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

    /// Resolve a remote cursor for drawing.
    pub fn cursor_position(&self, cursor: &Cursor) -> Option<DocPos> {
        self.state.position(cursor)
    }

    /// Drain transport events and apply authoritative results to `session`.
    /// Returns whether the UI-visible state changed.
    pub fn poll(&mut self, session: &mut Session) -> bool {
        let mut changed = false;
        loop {
            match self.handle.events.try_recv() {
                Ok(TransportEvent::Event(event)) => {
                    changed = true;
                    self.on_event(event, session);
                }
                Ok(TransportEvent::Disconnected { reason }) => {
                    self.status = Status::Offline;
                    self.last_error = Some(reason);
                    changed = true;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.status = Status::Failed("the collaboration task ended".into());
                    break;
                }
            }
        }
        changed
    }

    fn on_event(&mut self, event: Event, session: &mut Session) {
        match event {
            Event::Welcome { version, role, participants, .. } => {
                self.state.version = self.state.version.max(version);
                self.state.role = Role::parse(&role).unwrap_or(Role::Editor);
                self.set_participants(&participants);
                self.status = Status::InSync;
            }
            Event::Nodes { nodes } => self.state.set_nodes(&nodes),
            Event::Applied { applied } => match self.state.apply(&applied, session) {
                Ok(Outcome::Quiet) => {}
                Ok(Outcome::Resync) => {
                    self.status = Status::Resyncing;
                    let _ = self.handle.resync();
                }
                Err(message) => self.status = Status::Failed(message),
            },
            Event::Rejected { reject } => {
                let (status, resync) = refusal_status(&reject);
                self.status = status;
                // The panel shows the reason; a refused undo must be visible at
                // its author, not silent.
                self.last_error = Some(reject.message.clone());
                if resync {
                    let _ = self.handle.resync();
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
            }
            Event::Resync { reason } => {
                self.status = Status::Resyncing;
                self.last_error = Some(reason);
                let _ = self.handle.resync();
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
        let doc: Document = serde_json::from_slice(&self.snapshot).map_err(|err| format!("bad snapshot: {err}"))?;
        session.set_document(doc);
        self.snapshot.clear();
        Ok(version)
    }

    fn presence_of(&mut self, client_id: &str, role: &str, state: &Value) {
        apply_presence(&mut self.participants, client_id, role, state);
    }

    /// Apply a local command: the caller hands the document from before the
    /// command; the session sends the primitives or the command itself.
    pub fn after_command(&mut self, before: &Document, session: &Session, command: &str, params: &Value) -> Result<(), String> {
        if !self.can_write() {
            return Ok(());
        }
        let plans = self.state.plan(before, session, command, params)?;
        for plan in plans {
            self.handle.submit(plan.kind, plan.target, plan.payload)?;
        }
        Ok(())
    }

    /// Publish the caret and selection at most every 100 ms.
    pub fn publish_presence(&mut self, session: &Session, now: Instant) {
        if self.status != Status::InSync {
            return;
        }
        if self.last_presence.is_some_and(|at| now.duration_since(at) < Duration::from_millis(100)) {
            return;
        }
        let Some(cursor) = self.state.address(&session.sel.focus) else { return };
        let anchor = self.state.address(&session.sel.anchor).unwrap_or_else(|| cursor.clone());
        self.last_presence = Some(now);
        let state = json!({
            "cursor": {"story": cursor.0, "nodeId": cursor.1, "off": cursor.2},
            "selection": {
                "anchor": {"story": anchor.0, "nodeId": anchor.1, "off": anchor.2},
                "focus": {"story": cursor.0, "nodeId": cursor.1, "off": cursor.2},
            },
            "active": true,
        });
        let _ = self.handle.presence(state);
    }

    pub fn undo(&self) {
        let _ = self.handle.undo();
    }

    pub fn redo(&self) {
        let _ = self.handle.redo();
    }

    pub fn commit(&self) {
        let _ = self.handle.commit();
    }

    /// Replace the JWT (the cabinet pushes a fresh one before `exp`) and
    /// reconnect. Queued operations survive and are resent with their ids.
    pub fn renew_token(&mut self, token: &str) {
        if self.handle.renew_token(token.to_string()).is_ok() {
            self.status = Status::Connecting;
        }
    }

    /// The client id this session sends to the server and receives its own
    /// answers under.
    pub fn client_id(&self) -> &str {
        &self.state.client_id
    }

    /// Add the server's own view of the participants after `welcome`.
    pub fn set_participants(&mut self, participants: &[Participant]) {
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
}

/// Fold one presence frame into the participant list.
///
/// The role comes from the server, which takes it from the JWT claim
/// (ADR-0008) — never from the presence payload — so a peer that joined the
/// room earlier learns a newcomer's role with the newcomer's first frame
/// instead of showing a placeholder. An empty role means an older server: an
/// existing participant keeps the role it already had, a new one stays
/// `participant`.
pub fn apply_presence(participants: &mut Vec<ParticipantView>, client_id: &str, role: &str, state: &Value) {
    let cursor = state.get("cursor").and_then(parse_cursor);
    let selection = state.get("selection").and_then(parse_selection);
    if let Some(participant) = participants.iter_mut().find(|participant| participant.client_id == client_id) {
        participant.cursor = cursor.or(participant.cursor.take());
        participant.selection = selection.or(participant.selection.take());
        participant.active = state.get("active").and_then(Value::as_bool).unwrap_or(true);
        if !role.is_empty() {
            participant.role = role.to_string();
        }
        return;
    }
    participants.push(ParticipantView {
        client_id: client_id.to_string(),
        role: if role.is_empty() { "participant".into() } else { role.to_string() },
        color: None,
        cursor,
        selection,
        active: true,
    });
}

/// Commands whose result includes ids the protocol does not carry (comment
/// parts, footnotes, shapes, tables). A replica takes a snapshot after them.
fn allocates_ids(command: &str) -> bool {
    ["review.", "insert.", "table.", "shape.", "chart.", "equation.", "references.", "mailings."].iter().any(|prefix| command.starts_with(prefix))
}

fn parse_cursor(value: &Value) -> Option<Cursor> {
    Some(Cursor {
        story: value.get("story")?.as_str()?.to_string(),
        node_id: u32::try_from(value.get("nodeId")?.as_u64()?).ok()?,
        off: usize::try_from(value.get("off")?.as_u64()?).ok()?,
    })
}

fn parse_selection(value: &Value) -> Option<(Cursor, Cursor)> {
    Some((parse_cursor(value.get("anchor")?)?, parse_cursor(value.get("focus")?)?))
}

fn story_name(story: StoryRef) -> String {
    match story {
        StoryRef::Body => "body".to_string(),
        StoryRef::Part(id) => format!("part:{id}"),
    }
}

fn parse_story(name: &str) -> Option<StoryRef> {
    match name {
        "body" => Some(StoryRef::Body),
        other => other.strip_prefix("part:").and_then(|id| id.parse().ok()).map(StoryRef::Part),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
