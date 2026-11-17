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
    /// The session node ids this operation issued, moved or retired (ADR-0002).
    /// A command the document adapter cannot name as primitives replays on the
    /// replica, but the ids it allocates (a comment, a text box, a footnote, a
    /// table) are the server's: this delta lets the replica keep addressing the
    /// new nodes without asking for a snapshot. It is attached to the live
    /// answer only; a reconnecting replica still receives the whole map with
    /// `ServerMsg::Nodes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<NodeDelta>,
    /// `true` when `base_version` was behind `head` and the operation landed on
    /// a newer state (ADR-0004).
    #[serde(default)]
    pub conflict: bool,
    /// Set when this answer is the result of an undo/redo (ADR-0010). Unlike an
    /// ordinary edit, which the caller already applied optimistically, the
    /// inverse exists only here: the author's replica must apply this delta,
    /// while the echo of its own edit is still skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<HistoryAction>,
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

/// The cabinet's copy of a committed file: the version it stored and the tag
/// the next commit must send as `expectedVersion` (INTEGRATION.md §5.2, M87).
/// The room publishes the file itself when the server is configured with the
/// cabinet callback; without one the commit only builds the file and the host
/// owns the storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CabinetCommit {
    pub version: u64,
    pub etag: String,
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
        /// `seed` frames carrying the document this client has open follow
        /// (ADR WC-0004). The server uses them only for a document it has
        /// never snapshotted: without one it refuses the join instead of
        /// starting a blank room. An older client sends no field and cannot
        /// open a room this server has no state for.
        #[serde(default)]
        seed: bool,
    },
    /// One chunk of the document a joining client has open, sent right after
    /// `Auth { seed: true }`; the last chunk sets `last` (ADR WC-0004).
    ///
    /// The room is built from these bytes when the server has never
    /// snapshotted the document, so its first operation lands on the document
    /// the client is looking at, not on an empty page whose snapshot would
    /// replace that document.
    Seed {
        bytes: Vec<u8>,
        last: bool,
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
    /// Ask for the file the room built for a commit the cabinet refused with
    /// `409`. The owner saves a copy instead of overwriting the cabinet
    /// (TASK.md §6). The answer is chunked [`ServerMsg::SaveCopy`].
    SaveCopy {
        op_id: OpId,
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

/// The node-map change one accepted operation made (ADR-0002).
///
/// It carries only what moved, so a command that allocates ids stays cheap on
/// the wire; a full map is still sent on join and after a structural primitive.
/// An operation whose change set passes [`MAX_NODE_DELTA`] reports
/// `overflow: true` instead, and the replica takes a snapshot: that is the
/// fallback when a delta would be larger than sending the map it replaces.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDelta {
    /// Ids the operation issued or moved, in id order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<NodeRef>,
    /// Ids the operation retired; a late operation on one is answered `gone`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<u32>,
    /// The change set passed [`MAX_NODE_DELTA`]; the replica must take a
    /// snapshot instead of applying a partial map.
    #[serde(default)]
    pub overflow: bool,
}

/// The most node changes one delta may carry. A command that inserts a large
/// table can allocate thousands of ids; past this bound the replica snapshots
/// rather than trust a delta that was cut short.
pub const MAX_NODE_DELTA: usize = 4096;

/// Whether a command can allocate session ids (a new part, a table's rows and
/// cells, a comment, a shape). Such a command always travels with a
/// [`NodeDelta`] so replicas need no snapshot; a command outside this list
/// either changes no node map or is a primitive the replica derives itself.
pub fn may_allocate_ids(command: &str) -> bool {
    ["review.", "insert.", "table.", "shape.", "chart.", "equation.", "references.", "mailings."].iter().any(|prefix| command.starts_with(prefix))
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
        /// The sender's role, taken by the server from the JWT claim (ADR-0008)
        /// and never from the presence payload. It lets a peer that joined
        /// before the sender show the right role on the sender's first frame.
        /// Defaulted so a peer on the older wire still parses.
        #[serde(default)]
        role: String,
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
        /// The cabinet's version and etag when the room posted the file to the
        /// cabinet itself (`COLLAB_CABINET_URL`). Absent when the server only
        /// built the file (the host owns the commit) and on an older server.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cabinet: Option<CabinetCommit>,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
    /// One chunk of the `.docx` the room built for a commit the cabinet
    /// refused with `409`; the owner can save it as a copy without writing the
    /// cabinet (TASK.md §6). `version`, `sha256` and `warnings` describe the
    /// whole file and repeat on every chunk; `last` ends the copy. Sent only to
    /// the peer that asked, never broadcast.
    SaveCopy {
        version: Version,
        sha256: String,
        warnings: Vec<String>,
        bytes: Vec<u8>,
        last: bool,
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
    fn an_applied_carries_a_node_delta_and_an_old_answer_still_parses() {
        let applied = Applied {
            op_id: OpId::new("c1", 9),
            version: 12,
            epoch: 1,
            delta: Some(serde_json::json!({"op": "command", "command": "review.newComment"})),
            nodes: Some(NodeDelta { changed: vec![NodeRef { id: 7, story: "part:5".into(), path: vec![0] }], removed: vec![3], overflow: false }),
            conflict: false,
            history: None,
        };
        let text = serde_json::to_string(&applied).expect("serialize");
        assert!(text.contains("part:5"), "{text}");
        let back: Applied = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(applied, back);

        // A server built before the node delta sent no `nodes`: the field defaults.
        let old = r#"{"opId":{"client":"c1","seq":1},"version":1,"epoch":0,"conflict":false}"#;
        let back: Applied = serde_json::from_str(old).expect("an older answer parses");
        assert!(back.nodes.is_none());
    }

    #[test]
    fn an_older_presence_without_a_role_still_parses() {
        // A server built before the role field sent exactly this frame: the
        // control-message fields are snake_case, and there is no `role`.
        let text = r#"{"type":"presence","client_id":"c1","state":{"active":true}}"#;
        let msg: ServerMsg = serde_json::from_str(text).expect("an older peer's frame parses");
        assert!(matches!(msg, ServerMsg::Presence { role, .. } if role.is_empty()));

        // The join announcement is the same frame with an empty state: a peer
        // built before the role field parses it and keeps what it already knew.
        let join = r#"{"type":"presence","client_id":"c2","state":{}}"#;
        let msg: ServerMsg = serde_json::from_str(join).expect("a role-less join frame parses");
        assert!(matches!(msg, ServerMsg::Presence { client_id, role, .. } if client_id == "c2" && role.is_empty()));
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
        let msg = ClientMsg::Auth { token: "t".into(), client_id: "c1".into(), last_version: Some(4), seed: true };
        let text = serde_json::to_string(&msg).expect("serialize");
        assert!(text.contains("\"type\":\"auth\""), "{text}");
        assert!(text.contains("\"seed\":true"), "{text}");
        let back: ClientMsg = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(msg, back);
    }

    #[test]
    fn an_older_auth_without_a_seed_field_parses() {
        // A client built before the seed frame sent exactly this first frame:
        // it offers no document, and the server refuses a room it has no state
        // for instead of the client promising a stream that never comes.
        let text = r#"{"type":"auth","token":"t","client_id":"c1","last_version":null}"#;
        let msg: ClientMsg = serde_json::from_str(text).expect("an older client's auth parses");
        assert!(matches!(msg, ClientMsg::Auth { seed, .. } if !seed));
    }

    #[test]
    fn a_seed_chunk_round_trips_through_json() {
        let msg = ClientMsg::Seed { bytes: b"{\"body\":[]}".to_vec(), last: true };
        let text = serde_json::to_string(&msg).expect("serialize");
        assert!(text.contains("\"type\":\"seed\""), "{text}");
        let back: ClientMsg = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(msg, back);
    }
}
