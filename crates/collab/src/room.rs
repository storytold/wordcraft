//! One document room: journal, ids, roles and undo over an application model.
//!
//! The server binary owns a `Room` per open document. The model itself (a
//! `wordcraft_engine::Session` in the app adapter) is hidden behind [`Model`],
//! so the protocol can be tested with a fake model and without a network.

use serde_json::Value;

use crate::ids::NodeIds;
use crate::journal::Journal;
use crate::undo::{UndoStacks, UndoStep};
use crate::wire::{Applied, Op, Reject, RejectCode, Version};

/// What an accepted operation changed, and how to undo it (ADR-0010).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effect {
    /// Authoritative delta the clients apply; `None` for operations whose
    /// result they can only learn from the next snapshot.
    pub delta: Option<Value>,
    /// Operations that undo this one; empty when the change is not undoable.
    pub inverse: Vec<Op>,
    /// Original operations for redo; empty when redo is not available.
    pub forward: Vec<Op>,
    /// Label for the undo menu.
    pub label: String,
}

/// The application half of a room: applies an operation to the document.
pub trait Model {
    /// Apply an operation to the authoritative state. `Err` refuses the
    /// operation and leaves the state untouched (ADR-0004).
    fn apply(&mut self, op: &Op) -> Result<Effect, Reject>;

    /// Native snapshot of the state, plus the source file bytes when the
    /// format keeps unmodelled parts (ADR-0005).
    fn snapshot(&self) -> Vec<u8>;
}

/// `owner | editor | commenter | viewer` (ADR-0008).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Editor,
    Commenter,
    Viewer,
}

impl Role {
    pub fn parse(text: &str) -> Option<Role> {
        match text {
            "owner" => Some(Role::Owner),
            "editor" => Some(Role::Editor),
            "commenter" => Some(Role::Commenter),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Editor => "editor",
            Role::Commenter => "commenter",
            Role::Viewer => "viewer",
        }
    }

    /// Whether the role may send any operation.
    pub fn can_write(self) -> bool {
        !matches!(self, Role::Viewer)
    }

    /// Whether the role may commit the file (ADR-0008, ADR-0009).
    pub fn can_commit(self) -> bool {
        matches!(self, Role::Owner)
    }
}

/// The answer to a join: what to send before the client is in sync.
#[derive(Clone, Debug, PartialEq)]
pub struct JoinPlan {
    pub version: Version,
    pub epoch: u64,
    /// Journal tail after the client's `lastVersion`; `None` means a snapshot
    /// is required (ADR-0003, ADR-0005).
    pub replay: Option<Vec<Applied>>,
    pub snapshot: Option<Vec<u8>>,
}

/// A document room.
pub struct Room<M: Model> {
    model: M,
    journal: Journal,
    ids: NodeIds,
    undo: UndoStacks,
}

impl<M: Model> Room<M> {
    pub fn new(model: M) -> Room<M> {
        Room { model, journal: Journal::new(10_000, 10_000), ids: NodeIds::new(), undo: UndoStacks::new(200) }
    }

    pub fn version(&self) -> Version {
        self.journal.head()
    }

    pub fn epoch(&self) -> u64 {
        self.journal.epoch()
    }

    pub fn ids(&self) -> &NodeIds {
        &self.ids
    }

    pub fn ids_mut(&mut self) -> &mut NodeIds {
        &mut self.ids
    }

    pub fn model(&self) -> &M {
        &self.model
    }

    /// Plan the catch-up for a client that saw `last_version`.
    pub fn join(&self, last_version: Option<Version>, snapshot_wanted: bool) -> JoinPlan {
        let version = self.journal.head();
        match last_version {
            Some(v) if !snapshot_wanted => match self.journal.since(v) {
                Some(tail) => JoinPlan { version, epoch: self.journal.epoch(), replay: Some(tail), snapshot: None },
                None => JoinPlan { version, epoch: self.journal.epoch(), replay: None, snapshot: Some(self.model.snapshot()) },
            },
            _ => JoinPlan { version, epoch: self.journal.epoch(), replay: None, snapshot: Some(self.model.snapshot()) },
        }
    }

    /// Submit an operation for `actor` (the JWT `sub`).
    pub fn submit(&mut self, actor: &str, role: Role, op: Op) -> Result<Applied, Reject> {
        if !role.can_write() {
            return Err(Reject::new(op.op_id.clone(), RejectCode::Forbidden, "the role may not edit this document"));
        }
        if let Some(cached) = self.journal.cached(&op.op_id) {
            return Ok(cached.clone());
        }
        self.journal.check(&op)?;
        let effect = self.model.apply(&op)?;
        let applied = self.journal.accept(&op, actor, effect.delta, effect.inverse.clone());
        if !effect.inverse.is_empty() || !effect.forward.is_empty() {
            let step = UndoStep::new(effect.label, effect.inverse, effect.forward);
            self.undo.record(actor, step);
        }
        Ok(applied)
    }

    /// The newest undo step of `actor`, to be sent as a new operation.
    pub fn undo(&mut self, actor: &str) -> Option<UndoStep> {
        self.undo.undo(actor)
    }

    /// The newest redo step of `actor`.
    pub fn redo(&mut self, actor: &str) -> Option<UndoStep> {
        self.undo.redo(actor)
    }

    pub fn undo_depth(&self, actor: &str) -> (usize, usize) {
        self.undo.depth(actor)
    }

    /// Compact after a snapshot was written (ADR-0005).
    pub fn compact(&mut self, upto: Version) {
        self.journal.compact(upto);
        self.ids.compact(upto);
    }
}
