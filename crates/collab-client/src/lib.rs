//! Client half of the collaboration protocol: operation ids, the unconfirmed
//! queue and the reconnect state machine.
//!
//! Transport-free on purpose: the app wires [`Client`] to a WebSocket (desktop)
//! or a host bridge (web), and tests drive it against a real
//! `wordcraft_collab::Room` without a socket. The rules it implements are
//! ADR-0003 (`opId` dedup, `version`), ADR-0004 (reject and resync) and
//! ADR-0010 (undo is a new operation, so the client never rewinds).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;

use serde_json::Value;

use wordcraft_collab::wire::{
    Applied, CabinetCommit, ClientMsg, HistoryAction, Op, OpId, OpKind, Participant, Reject, RejectCode, ServerMsg, Target, Version,
};

/// An id for one editing session, unique among the windows, tabs and machines
/// working on the same document.
///
/// A document id is unique only inside one process (it is a counter that starts
/// at 1), so two apps editing `Document1` would both call themselves `ui-1` or
/// `web-1`: the server files their `(clientId, seq)` operations as duplicates of
/// each other, one joins as the other, and the participant list shows a single
/// peer. The suffix is what each target offers cheaply: the process id, the wall
/// clock and a counter natively; a random number in the browser. It is made once
/// per session, so a reconnect keeps the id and its retries stay deduplicated.
pub fn session_client_id(prefix: &str, document: u64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let random = (js_sys::Math::random() * f64::from(u32::MAX)) as u32;
        format!("{prefix}-{document}-{random:x}")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos() as u64).unwrap_or(0);
        let count = NEXT.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{document}-{:x}-{nanos:x}-{count}", std::process::id())
    }
}

/// Where the client is in the session handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Sent `auth`, waiting for `welcome`.
    AwaitingWelcome,
    /// In sync; operations may be sent.
    Joined,
    /// The server asked for a snapshot (stale base or compaction).
    Resyncing,
    /// The session ended.
    Closed,
}

/// What the app must react to after a server message.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Welcome {
        version: Version,
        epoch: u64,
        resync: bool,
        role: String,
        participants: Vec<Participant>,
    },
    Applied {
        applied: Applied,
    },
    Rejected {
        reject: Reject,
    },
    /// The client must fetch a snapshot and then resend confirmed-only state.
    Resync {
        reason: String,
    },
    /// Session node ids for addressing paragraphs (ADR-0002).
    Nodes {
        nodes: Vec<wordcraft_collab::wire::NodeRef>,
    },
    Presence {
        client_id: String,
        /// The sender's role from the server (its JWT claim), empty on an older
        /// server that does not send one.
        role: String,
        state: Value,
    },
    Left {
        client_id: String,
    },
    Snapshot {
        version: Version,
        bytes: Vec<u8>,
        last: bool,
    },
    Committed {
        version: Version,
        sha256: String,
        warnings: Vec<String>,
        /// The cabinet's copy, when the room posted the file itself.
        cabinet: Option<CabinetCommit>,
    },
    /// The room built the file but the cabinet refused it (`409`: the file
    /// changed outside the session). The replica resyncs; the owner may ask for
    /// the refused bytes with [`Client::save_copy`] and save them as a copy
    /// (TASK.md §6).
    Conflict {
        reason: String,
    },
    /// One chunk of the refused commit's `.docx`; `last` ends the copy.
    SaveCopy {
        version: Version,
        sha256: String,
        warnings: Vec<String>,
        bytes: Vec<u8>,
        last: bool,
    },
    /// Nothing for the app to do.
    None,
}

/// One operation the client sent but the server has not answered yet.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub op: Op,
    /// Version the client had when it sent the operation.
    pub base: Version,
}

/// The client state machine.
pub struct Client {
    client_id: String,
    seq: u64,
    state: State,
    version: Version,
    epoch: u64,
    pending: Vec<Pending>,
    /// Operations whose answer arrived, for duplicate detection on reconnect.
    confirmed: HashMap<OpId, Version>,
    /// This client's undoable steps, oldest first. A step is named by the
    /// operation that created it.
    steps: Vec<(OpId, OpKind, Target)>,
    /// Steps moved to the redo stack by a confirmed undo.
    undone: Vec<(OpId, OpKind, Target)>,
    /// Undo/redo operations in flight, so a confirmed answer moves the right
    /// stack and a rejection leaves it alone.
    history_pending: HashMap<OpId, HistoryAction>,
}

impl Client {
    pub fn new(client_id: impl Into<String>) -> Client {
        Client {
            client_id: client_id.into(),
            seq: 0,
            state: State::AwaitingWelcome,
            version: 0,
            epoch: 0,
            pending: Vec::new(),
            confirmed: HashMap::new(),
            steps: Vec::new(),
            undone: Vec::new(),
            history_pending: HashMap::new(),
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The auth message that opens the session (ADR-0008: token first, never in
    /// the URL).
    pub fn auth(&self, token: &str, last_version: Option<Version>) -> ClientMsg {
        ClientMsg::Auth { token: token.to_string(), client_id: self.client_id.clone(), last_version }
    }

    /// Build the next operation, assign it a `(clientId, seq)` id and queue it
    /// until the server answers (ADR-0003).
    pub fn op(&mut self, kind: OpKind, target: Target, payload: Value) -> Op {
        self.next_op(kind, target, payload, None)
    }

    fn next_op(&mut self, kind: OpKind, target: Target, payload: Value, history: Option<HistoryAction>) -> Op {
        self.seq = self.seq.saturating_add(1);
        let op = Op {
            op_id: OpId::new(self.client_id.clone(), self.seq),
            base_version: self.version,
            epoch: if kind.is_structural() { Some(self.epoch) } else { None },
            kind,
            target,
            payload,
            history,
        };
        self.pending.push(Pending { op: op.clone(), base: self.version });
        op
    }

    /// The newest step this client can undo, in the shape of an operation
    /// (ADR-0010). `None` when there is nothing on the stack or the session is
    /// not joined. The step's kind and address travel with the request so a
    /// structural undo is checked as structural; the server takes the step from
    /// its own stack.
    pub fn undo(&mut self) -> Option<Op> {
        if self.state != State::Joined {
            return None;
        }
        let (step, kind, target) = self.steps.last().cloned()?;
        let action = HistoryAction::Undo { op_id: step.clone() };
        let op = self.next_op(kind, target, Value::Null, Some(action.clone()));
        self.history_pending.insert(op.op_id.clone(), action);
        Some(op)
    }

    /// The newest undone step, to replay it (ADR-0010).
    pub fn redo(&mut self) -> Option<Op> {
        if self.state != State::Joined {
            return None;
        }
        let (step, kind, target) = self.undone.last().cloned()?;
        let action = HistoryAction::Redo { op_id: step.clone() };
        let op = self.next_op(kind, target, Value::Null, Some(action.clone()));
        self.history_pending.insert(op.op_id.clone(), action);
        Some(op)
    }

    /// `(undoable, redoable)` steps, for the menu.
    pub fn history_depth(&self) -> (usize, usize) {
        (self.steps.len(), self.undone.len())
    }

    /// A fresh operation id for a message that is not an operation (a commit).
    pub fn next_op_id(&mut self) -> OpId {
        self.seq = self.seq.saturating_add(1);
        OpId::new(self.client_id.clone(), self.seq)
    }

    /// Ask the room for the file it built for the commit the cabinet refused
    /// (`409`), so the owner can save a copy (TASK.md §6).
    pub fn save_copy(&mut self) -> ClientMsg {
        ClientMsg::SaveCopy { op_id: self.next_op_id() }
    }

    /// Operations the server has not confirmed yet, oldest first.
    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    /// The operations to resend after a reconnect. Their ids are unchanged, so
    /// the server deduplicates any it already applied.
    pub fn unconfirmed(&self) -> Vec<ClientMsg> {
        self.pending.iter().map(|pending| ClientMsg::Op { op: pending.op.clone() }).collect()
    }

    /// Handle one server message.
    pub fn on(&mut self, msg: ServerMsg) -> Event {
        match msg {
            ServerMsg::Welcome { version, epoch, resync, role, participants, .. } => {
                self.version = self.version.max(version);
                self.epoch = epoch;
                self.state = if resync { State::Resyncing } else { State::Joined };
                Event::Welcome { version, epoch, resync, role, participants }
            }
            ServerMsg::Tail { entries } => {
                for applied in entries {
                    self.confirm(&applied);
                }
                self.state = State::Joined;
                Event::None
            }
            ServerMsg::Applied { applied } => {
                // A retry of an operation the room already applied comes back
                // as the cached answer, broadcast to every peer. A replica that
                // applied it the first time must not apply its delta again.
                if self.was_confirmed(&applied.op_id) {
                    return Event::None;
                }
                if applied.version > self.version + 1 {
                    // A message went missing (a dropped queue on the server, a
                    // half-closed socket): a snapshot is cheaper than guessing.
                    self.state = State::Resyncing;
                    return Event::Resync { reason: format!("version gap: {} after {}", applied.version, self.version) };
                }
                self.confirm(&applied);
                Event::Applied { applied }
            }
            ServerMsg::Rejected { reject } => {
                // A refused undo/redo leaves its stack where it was.
                self.history_pending.remove(&reject.op_id);
                // An operation refused `stale` or `gone` is not resent: the
                // offsets or the node it names are exactly what the refusal is
                // about, so replaying it after the resync would recreate the
                // divergence (ADR WC-0001). A `lease` or `tokenExpired` refusal
                // was never looked at and stays queued for the reconnect.
                if matches!(reject.code, RejectCode::Stale | RejectCode::Gone) {
                    self.pending.retain(|pending| pending.op.op_id != reject.op_id);
                }
                if reject.retryable || reject.code == RejectCode::Gone {
                    self.state = State::Resyncing;
                    return Event::Resync { reason: reject.message.clone() };
                }
                Event::Rejected { reject }
            }
            ServerMsg::Nodes { nodes } => Event::Nodes { nodes },
            ServerMsg::Presence { client_id, role, state } => Event::Presence { client_id, role, state },
            ServerMsg::Left { client_id } => Event::Left { client_id },
            ServerMsg::Snapshot { version, bytes, last } => {
                if last {
                    self.version = self.version.max(version);
                    self.state = State::Joined;
                }
                Event::Snapshot { version, bytes, last }
            }
            ServerMsg::Committed { version, sha256, warnings, cabinet } => Event::Committed { version, sha256, warnings, cabinet },
            ServerMsg::Error { code, message, retryable } => {
                if retryable {
                    self.state = State::Resyncing;
                }
                if code == "cabinetConflict" {
                    // The cabinet refused the commit (409): the copy is still
                    // saveable, so the panel keeps the conflict apart from a
                    // plain resync (TASK.md §6).
                    Event::Conflict { reason: message }
                } else {
                    Event::Resync { reason: format!("{code}: {message}") }
                }
            }
            ServerMsg::SaveCopy { version, sha256, warnings, bytes, last } => Event::SaveCopy { version, sha256, warnings, bytes, last },
            ServerMsg::Pong { .. } => Event::None,
        }
    }

    /// Mark the session closed; pending operations stay for a reconnect.
    pub fn close(&mut self) {
        self.state = State::Closed;
    }

    fn confirm(&mut self, applied: &Applied) {
        let was_pending = self.pending.iter().find(|pending| pending.op.op_id == applied.op_id).cloned();
        self.pending.retain(|pending| pending.op.op_id != applied.op_id);
        self.confirmed.insert(applied.op_id.clone(), applied.version);
        self.version = self.version.max(applied.version);
        self.epoch = self.epoch.max(applied.epoch);

        if let Some(action) = self.history_pending.remove(&applied.op_id) {
            match action {
                HistoryAction::Undo { op_id } => {
                    if let Some(at) = self.steps.iter().rposition(|(step, _, _)| *step == op_id) {
                        let step = self.steps.remove(at);
                        self.undone.push(step);
                    }
                }
                HistoryAction::Redo { op_id } => {
                    if let Some(at) = self.undone.iter().rposition(|(step, _, _)| *step == op_id) {
                        let step = self.undone.remove(at);
                        self.steps.push(step);
                    }
                }
            }
        } else if let Some(pending) = was_pending
            && pending.op.history.is_none()
            && pending.op.op_id.client == self.client_id
        {
            // A new local edit is its own step and drops the redo branch.
            self.steps.push((pending.op.op_id, pending.op.kind, pending.op.target));
            self.undone.clear();
        }
    }

    /// Whether an operation id was already confirmed (duplicate after a
    /// reconnect).
    pub fn was_confirmed(&self, op_id: &OpId) -> bool {
        self.confirmed.contains_key(op_id)
    }
}

#[cfg(test)]
mod tests;
