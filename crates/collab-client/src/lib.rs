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

use wordcraft_collab::wire::{Applied, ClientMsg, Op, OpId, OpKind, Reject, RejectCode, ServerMsg, Target, Version};

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
        self.seq = self.seq.saturating_add(1);
        let op = Op {
            op_id: OpId::new(self.client_id.clone(), self.seq),
            base_version: self.version,
            epoch: if kind.is_structural() { Some(self.epoch) } else { None },
            kind,
            target,
            payload,
        };
        self.pending.push(Pending { op: op.clone(), base: self.version });
        op
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
                if reject.retryable || reject.code == RejectCode::Gone {
                    self.state = State::Resyncing;
                    return Event::Resync { reason: reject.message.clone() };
                }
                Event::Rejected { reject }
            }
            ServerMsg::Nodes { .. } => Event::None,
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
        self.pending.retain(|pending| pending.op.op_id != applied.op_id);
        self.confirmed.insert(applied.op_id.clone(), applied.version);
        self.version = self.version.max(applied.version);
        self.epoch = self.epoch.max(applied.epoch);
    }

    /// Whether an operation id was already confirmed (duplicate after a
    /// reconnect).
    pub fn was_confirmed(&self, op_id: &OpId) -> bool {
        self.confirmed.contains_key(op_id)
    }
}

#[cfg(test)]
mod tests;
