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

use wordcraft_collab::wire::{Applied, ClientMsg, HistoryAction, Op, OpId, OpKind, Reject, RejectCode, ServerMsg, Target, Version};

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
            ServerMsg::Welcome { version, epoch, resync, .. } => {
                self.version = self.version.max(version);
                self.epoch = epoch;
                self.state = if resync { State::Resyncing } else { State::Joined };
                Event::Welcome { version, epoch, resync }
            }
            ServerMsg::Tail { entries } => {
                for applied in entries {
                    self.confirm(&applied);
                }
                self.state = State::Joined;
                Event::None
            }
            ServerMsg::Applied { applied } => {
                self.confirm(&applied);
                Event::Applied { applied }
            }
            ServerMsg::Rejected { reject } => {
                // A refused undo/redo leaves its stack where it was.
                self.history_pending.remove(&reject.op_id);
                if reject.retryable || reject.code == RejectCode::Gone {
                    self.state = State::Resyncing;
                    return Event::Resync { reason: reject.message.clone() };
                }
                Event::Rejected { reject }
            }
            ServerMsg::Nodes { nodes } => Event::Nodes { nodes },
            ServerMsg::Presence { client_id, state } => Event::Presence { client_id, state },
            ServerMsg::Left { client_id } => Event::Left { client_id },
            ServerMsg::Snapshot { version, bytes, last } => {
                if last {
                    self.version = self.version.max(version);
                    self.state = State::Joined;
                }
                Event::Snapshot { version, bytes, last }
            }
            ServerMsg::Committed { version, sha256, warnings } => Event::Committed { version, sha256, warnings },
            ServerMsg::Error { code, message, retryable } => {
                if retryable {
                    self.state = State::Resyncing;
                }
                Event::Resync { reason: format!("{code}: {message}") }
            }
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
