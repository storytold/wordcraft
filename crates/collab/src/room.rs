//! One document room: journal, ids, roles and undo over an application model.
//!
//! The server binary owns a `Room` per open document. The model itself (a
//! `wordcraft_engine::Session` in the app adapter) is hidden behind [`Model`],
//! so the protocol can be tested with a fake model and without a network.

use std::collections::HashMap;

use serde_json::Value;

use crate::ids::NodeIds;
use crate::journal::Journal;
use crate::undo::{UndoStacks, UndoStep};
use crate::wire::{Applied, HistoryAction, NodeRef, Op, OpId, Reject, RejectCode, Version};

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
    /// Whether the operation moved block paths or added a block, so the
    /// server must send a fresh node map (ADR-0002). Text edits leave it false.
    pub nodes_changed: bool,
}

/// The application half of a room: applies an operation to the document.
pub trait Model {
    /// Apply an operation to the authoritative state as `actor` (the JWT
    /// `sub`). `Err` refuses the operation and leaves the state untouched
    /// (ADR-0004). The actor is what the model puts in tracked changes and
    /// comment authors; it never comes from the operation itself (ADR-0008).
    fn apply(&mut self, actor: &str, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject>;

    /// Native snapshot of the state, plus the source file bytes when the
    /// format keeps unmodelled parts (ADR-0005).
    fn snapshot(&self) -> Vec<u8>;

    /// Issue session ids for nodes that do not have one yet (called on join).
    fn index_nodes(&self, ids: &mut crate::ids::NodeIds);
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
    /// Session nodes the client may address (ADR-0002).
    pub nodes: Vec<NodeRef>,
}

/// A document room.
pub struct Room<M: Model> {
    model: M,
    journal: Journal,
    ids: NodeIds,
    undo: UndoStacks,
    /// Version that last touched each session node, for undo conflicts.
    last_touch: HashMap<u32, Version>,
    /// Version of the last structural operation.
    last_structural: Version,
    /// Set by the last accepted operation when clients need a fresh node map.
    nodes_changed: bool,
}

impl<M: Model> Room<M> {
    pub fn new(model: M) -> Room<M> {
        Room {
            model,
            journal: Journal::new(10_000, 10_000),
            ids: NodeIds::new(),
            undo: UndoStacks::new(200),
            last_touch: HashMap::new(),
            last_structural: 0,
            nodes_changed: false,
        }
    }

    /// Restore a room from a snapshot: the model is authoritative at `version`.
    pub fn restore(model: M, version: Version, epoch: u64) -> Room<M> {
        let mut journal = Journal::new(10_000, 10_000);
        journal.restore(version, epoch);
        Room { model, journal, ids: NodeIds::new(), undo: UndoStacks::new(200), last_touch: HashMap::new(), last_structural: 0, nodes_changed: false }
    }

    pub fn version(&self) -> Version {
        self.journal.head()
    }

    /// Whether the room should write a snapshot now (ADR-0005: 500 operations).
    pub fn should_snapshot(&self) -> bool {
        self.journal.should_snapshot()
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

    /// Plan the catch-up for a client that saw `last_version`. Also issues
    /// session ids for nodes that have none yet.
    pub fn join(&mut self, last_version: Option<Version>, snapshot_wanted: bool) -> JoinPlan {
        self.model.index_nodes(&mut self.ids);
        let version = self.journal.head();
        let nodes = self.nodes();
        match last_version {
            Some(v) if !snapshot_wanted => match self.journal.since(v) {
                Some(tail) => JoinPlan { version, epoch: self.journal.epoch(), replay: Some(tail), snapshot: None, nodes },
                None => JoinPlan { version, epoch: self.journal.epoch(), replay: None, snapshot: Some(self.model.snapshot()), nodes },
            },
            _ => JoinPlan { version, epoch: self.journal.epoch(), replay: None, snapshot: Some(self.model.snapshot()), nodes },
        }
    }

    /// The session nodes clients can address.
    pub fn nodes(&self) -> Vec<NodeRef> {
        self.ids.iter().map(|(id, key)| NodeRef { id: id.0, story: key.story.clone(), path: key.path.clone() }).collect()
    }

    /// Submit an operation for `actor` (the JWT `sub`).
    pub fn submit(&mut self, actor: &str, role: Role, op: Op) -> Result<Applied, Reject> {
        if !role.can_write() {
            return Err(Reject::new(op.op_id.clone(), RejectCode::Forbidden, "the role may not edit this document"));
        }
        if role == Role::Commenter {
            // A commenter annotates; the document itself is not theirs (ADR-0008).
            let comment_command = op.kind == crate::wire::OpKind::Command
                && matches!(
                    op.payload.get("command").and_then(Value::as_str),
                    Some("review.newComment" | "review.reply" | "review.editComment" | "review.resolveComment" | "review.deleteComment")
                );
            let annotation = op.kind == crate::wire::OpKind::Merge
                || comment_command
                || (op.kind == crate::wire::OpKind::Primitive
                    && matches!(op.payload.get("op").and_then(Value::as_str), Some("add_comment" | "resolve_comment")));
            if !annotation {
                return Err(Reject::new(op.op_id.clone(), RejectCode::Forbidden, "a commenter may only add or resolve comments"));
            }
        }
        self.nodes_changed = false;
        if let Some(cached) = self.journal.cached(&op.op_id) {
            return Ok(cached.clone());
        }
        self.journal.check(&op)?;
        if let Some(history) = op.history.clone() {
            return self.submit_history(actor, &op, history);
        }
        let effect = self.model.apply(actor, &op, &mut self.ids)?;
        let has_delta = effect.delta.is_some();
        self.nodes_changed = effect.nodes_changed;
        let applied = self.journal.accept(&op, actor, effect.delta, effect.inverse.clone());
        self.note_touch(&op, applied.version);
        if !effect.inverse.is_empty() || !effect.forward.is_empty() || has_delta {
            // A command has no inverse yet; recording it as an empty step makes
            // it a barrier, so undo cannot silently skip it and take back an
            // older step (ADR-0010).
            let step = UndoStep::new(effect.label, applied.version, effect.inverse, effect.forward);
            self.undo.record(actor, step);
        }
        Ok(applied)
    }

    /// Undo or redo one step of `actor` (ADR-0010). The caller's operation is
    /// the unit that gets a version; the step's operations run behind it and
    /// are never journaled separately, so the document moves once.
    fn submit_history(&mut self, actor: &str, op: &Op, history: HistoryAction) -> Result<Applied, Reject> {
        self.nodes_changed = false;
        let (step, step_op_id, undo) = match &history {
            HistoryAction::Undo { op_id } => (self.undo.peek_undo(actor), op_id, true),
            HistoryAction::Redo { op_id } => (self.undo.peek_redo(actor), op_id, false),
        };
        let Some(step) = step else {
            let what = if undo { "undo" } else { "redo" };
            return Err(Reject::new(op.op_id.clone(), RejectCode::UndoConflict, format!("nothing to {what}")));
        };
        if !step.forward.iter().any(|forward| forward.op_id == *step_op_id) {
            return Err(Reject::new(
                op.op_id.clone(),
                RejectCode::UndoConflict,
                format!("step {step_op_id} is not the newest step this actor can {}", if undo { "undo" } else { "redo" }),
            ));
        }
        let operations = if undo { &step.inverse } else { &step.forward };
        if operations.len() != 1 {
            // A step of several operations needs a transaction; applying half of
            // it would leave the document between states.
            return Err(Reject::new(op.op_id.clone(), RejectCode::Unsupported, "a step with several operations cannot be replayed yet"));
        }
        if self.step_conflicts(&step) {
            return Err(Reject::new(op.op_id.clone(), RejectCode::UndoConflict, "the step's targets changed after it landed"));
        }

        let mut delta = None;
        for operation in operations {
            let mut replayed = operation.clone();
            // The step's own ids were already used by the journal; the replay is
            // part of the caller's operation, so it must not be deduplicated.
            replayed.op_id = OpId::new(format!("history:{}", op.op_id.client), op.op_id.seq);
            replayed.base_version = self.journal.head();
            replayed.epoch = None;
            replayed.history = None;
            let effect = self.model.apply(actor, &replayed, &mut self.ids)?;
            if effect.nodes_changed {
                self.nodes_changed = true;
            }
            delta = match (delta.take(), effect.delta) {
                (None, next) => next,
                (Some(previous), Some(next)) => Some(serde_json::json!({"ops": [previous, next]})),
                (previous, None) => previous,
            };
        }

        let applied = self.journal.accept(op, actor, delta, Vec::new());
        for operation in operations {
            self.note_touch(operation, applied.version);
        }
        if undo {
            self.undo.finish_undo(actor, applied.version);
        } else {
            self.undo.finish_redo(actor, applied.version);
        }
        Ok(applied)
    }

    /// Whether another operation touched what `step` changed after it landed.
    /// Offsets and block indexes make a replayed operation meaningless once a
    /// neighbour moved, so the room refuses instead of guessing (ADR-0010).
    fn step_conflicts(&self, step: &UndoStep) -> bool {
        step.forward.iter().any(|forward| {
            if forward.kind.is_structural() {
                self.last_structural > step.version
            } else {
                self.last_touch.get(&forward.target.node_id).is_some_and(|version| *version > step.version)
            }
        })
    }

    fn note_touch(&mut self, op: &Op, version: Version) {
        if op.target.node_id != 0 {
            self.last_touch.insert(op.target.node_id, version);
        }
        if op.kind.is_structural() {
            self.last_structural = version;
        }
    }

    /// Whether the last accepted operation moved block paths or added ids, so
    /// the server must broadcast a fresh node map. Reading it clears the flag.
    pub fn take_nodes_changed(&mut self) -> bool {
        std::mem::replace(&mut self.nodes_changed, false)
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
