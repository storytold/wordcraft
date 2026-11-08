//! Wire types of the collaboration protocol.
//!
//! The envelope is ADR-0001: `{opId, docId, actorId, baseVersion, epoch, kind,
//! target, payload}`. `docId` and `actorId` are connection state (from `auth`),
//! not repeated in every operation; the server supplies the actor from the JWT
//! `sub` (ADR-0008), so a client cannot name another author.

use serde::{Deserialize, Serialize};

/// Monotonic document version (ADR-0003).
pub type Version = u64;

/// `(clientId, seq)`, unique per client and never reused (ADR-0003).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpId {
    pub client: String,
    pub seq: u64,
}

impl OpId {
    pub fn new(client: impl Into<String>, seq: u64) -> OpId {
        OpId { client: client.into(), seq }
    }
}

impl std::fmt::Display for OpId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.client, self.seq)
    }
}

/// How the server must treat an operation (ADR-0001, ADR-0004).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpKind {
    /// Positional edit of the model (WordCraft: `insert_text`, `delete_range`, …).
    Primitive,
    /// Normalized engine command with explicit address and context.
    Command,
    /// Exclusive operation whose result depends on other objects: tables, parts,
    /// styles, paste. Requires `base_version == head` (ADR-0003, ADR-0004).
    Structural,
    /// Independent property/annotation edit that may apply on any base.
    Merge,
}

impl OpKind {
    /// Structural operations serialize and need the head version.
    pub fn is_structural(self) -> bool {
        matches!(self, OpKind::Structural)
    }
}

/// Where an operation lands: a story, a session node and a byte offset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    /// `body`, `header`, `footer`, `footnote`, `comment`, `textbox`.
    pub story: String,
    /// Session node id (ADR-0002); `0` for stories without a node.
    #[serde(default)]
    pub node_id: u32,
    /// Byte offset inside the node (paragraph text offsets are UTF-8 bytes).
    #[serde(default)]
    pub off: usize,
}

impl Target {
    pub fn new(story: impl Into<String>, node_id: u32, off: usize) -> Target {
        Target { story: story.into(), node_id, off }
    }
}

/// One client operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Op {
    pub op_id: OpId,
    /// Version the client had seen; context, not a lock (ADR-0003).
    pub base_version: Version,
    /// Merge/structural epoch the client last saw (ADR-0002, ADR-0004).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    pub kind: OpKind,
    pub target: Target,
    #[serde(default)]
    pub payload: serde_json::Value,
    /// Set when the operation moves the sender's history instead of making a
    /// new step (ADR-0010). The server checks the step still holds, applies its
    /// inverse (undo) or original operations (redo), and journals the result as
    /// one new operation with its own version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<HistoryAction>,
}

/// Which way one actor's history moves. Undo is a new operation in every
/// case: it is sent with `base_version = head`, checked like any other, and
/// gets its own version. `op_id` names the step being taken back (the client's
/// id for the operation that created it), so a stale menu cannot undo a
/// different step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HistoryAction {
    Undo { op_id: OpId },
    Redo { op_id: OpId },
}

/// Server answer to an accepted operation (ADR-0001).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub op_id: OpId,
    pub version: Version,
    pub epoch: u64,
    /// Authoritative delta of the touched state; the client never derives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<serde_json::Value>,
    /// `true` when `base_version` was behind `head` and the operation landed on
    /// a newer state (ADR-0004).
    #[serde(default)]
    pub conflict: bool,
}

/// Why an operation was refused (ADR-0004).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    /// The base version is no longer in the journal: a snapshot is needed.
    Stale,
    /// The operation cannot land on the current state.
    Conflict,
    /// The role may not perform it (ADR-0008).
    Forbidden,
    /// The addressed node was deleted; tombstones stop resurrection.
    Gone,
    /// Another session holds the file lease.
    Lease,
    /// The JWT expired; reconnect with a fresh one.
    TokenExpired,
    /// Frame or document limit exceeded.
    TooLarge,
    /// Malformed operation.
    Invalid,
    /// Undo or redo would overwrite a change made after the step (ADR-0010).
    UndoConflict,
    /// The adapter does not implement the operation in this version.
    Unsupported,
}

impl RejectCode {
    /// Whether resending the same operation may succeed (ADR-0004).
    pub fn retryable(self) -> bool {
        matches!(self, RejectCode::Stale | RejectCode::Lease | RejectCode::TokenExpired)
    }
}

/// A refusal with a machine-readable code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reject {
    pub op_id: OpId,
    pub code: RejectCode,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl Reject {
    pub fn new(op_id: OpId, code: RejectCode, message: impl Into<String>) -> Reject {
        Reject { op_id, code, message: message.into(), retryable: code.retryable(), details: None }
    }
}

/// A participant as the UI shows it (ADR-0006).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub client_id: String,
    /// `owner | editor | commenter | viewer` (ADR-0008).
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub presence: Option<serde_json::Value>,
}

/// Messages the client sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMsg {
    /// First frame on the socket; the token is never in the URL (ADR-0008).
    Auth {
        token: String,
        client_id: String,
        #[serde(default)]
        last_version: Option<Version>,
    },
    Op {
        op: Op,
    },
    /// Ephemeral, never journaled (ADR-0006).
    Presence {
        state: serde_json::Value,
    },
    /// Ask for a fresh snapshot even when the journal tail is available (a
    /// command allocated ids the replica cannot derive locally).
    Resync {
        #[serde(default)]
        last_version: Version,
    },
    Commit {
        op_id: OpId,
        expected_version: Version,
        #[serde(default)]
        etag: Option<String>,
    },
    Ping {
        ts: u64,
    },
}

/// A session node the client can address (ADR-0002).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeRef {
    pub id: u32,
    pub story: String,
    pub path: Vec<u32>,
}

/// Messages the server sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMsg {
    Welcome {
        session_id: String,
        role: String,
        version: Version,
        epoch: u64,
        participants: Vec<Participant>,
        /// `true` when the journal tail is too old and a snapshot follows.
        resync: bool,
    },
    Applied {
        applied: Applied,
    },
    Rejected {
        reject: Reject,
    },
    Presence {
        client_id: String,
        state: serde_json::Value,
    },
    Left {
        client_id: String,
    },
    /// Journal tail the client missed, oldest first (ADR-0003, ADR-0004).
    Tail {
        entries: Vec<Applied>,
    },
    /// Session node ids, sent on join and after a structural edit moved paths
    /// (ADR-0002).
    Nodes {
        nodes: Vec<NodeRef>,
    },
    /// Snapshot chunk: the client replaces its state on `last`.
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
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
    Pong {
        ts: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_round_trips_through_json() {
        let op = Op {
            op_id: OpId::new("c1", 7),
            base_version: 41,
            epoch: Some(3),
            kind: OpKind::Primitive,
            target: Target::new("body", 18, 42),
            payload: serde_json::json!({"text": "Привет"}),
            history: None,
        };
        let text = serde_json::to_string(&op).expect("serialize");
        let back: Op = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(op, back);
    }

    #[test]
    fn op_id_display_is_client_colon_seq() {
        assert_eq!(OpId::new("c7", 12).to_string(), "c7:12");
    }

    #[test]
    fn only_transient_codes_are_retryable() {
        assert!(RejectCode::Stale.retryable());
        assert!(RejectCode::Lease.retryable());
        assert!(RejectCode::TokenExpired.retryable());
        assert!(!RejectCode::Gone.retryable());
        assert!(!RejectCode::Forbidden.retryable());
    }

    #[test]
    fn client_messages_are_tagged() {
        let msg = ClientMsg::Auth { token: "t".into(), client_id: "c1".into(), last_version: Some(4) };
        let text = serde_json::to_string(&msg).expect("serialize");
        assert!(text.contains("\"type\":\"auth\""), "{text}");
        let back: ClientMsg = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(msg, back);
    }
}
