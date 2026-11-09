//! One document room: journal, ids, roles and undo over an application model.
//!
//! The server binary owns a `Room` per open document. The model itself (a
//! `wordcraft_engine::Session` in the app adapter) is hidden behind [`Model`],
//! so the protocol can be tested with a fake model and without a network.

use std::collections::HashMap;

use serde_json::Value;

use crate::ids::NodeIds;
use crate::journal::{DedupWindow, Journal};
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
    /// Nodes beyond [`Op::target`] whose content this operation's inverse would
    /// overwrite or delete. A later edit on one of them makes the step
    /// `undo-conflict` instead of silently losing that edit (ADR-0010): a merge
    /// reports the merged cells, a split its cell, a column insert the cells it
    /// created. Empty when the inverse only inserts or removes what the target
    /// already names.
    pub touched: Vec<u32>,
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

    /// Top-level block count per story (`body`, `part:N`). A room uses it to
    /// tell a command that only appended a new story — safe to index for an id
    /// delta — from one that changed an existing story, where every later path
    /// shifted and only a snapshot can rebuild the map. The default suits a
    /// model with no stories.
    fn story_shape(&self) -> Vec<(String, usize)> {
        Vec::new()
    }
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
    /// `dedup` is the `opId` window saved with that snapshot, so a client that
    /// retries an operation from before the restart is answered from it instead
    /// of applying it a second time.
    pub fn restore(model: M, version: Version, epoch: u64, dedup: DedupWindow) -> Room<M> {
        let mut journal = Journal::new(10_000, 10_000);
        journal.restore_with(version, epoch, dedup);
        Room { model, journal, ids: NodeIds::new(), undo: UndoStacks::new(200), last_touch: HashMap::new(), last_structural: 0, nodes_changed: false }
    }

    pub fn version(&self) -> Version {
        self.journal.head()
    }

    /// The `opId` window to save with the next snapshot (ADR-0003, ADR-0005).
    pub fn dedup(&self) -> DedupWindow {
        self.journal.dedup()
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
        // A command that allocates ids must land on the head: the author ran it
        // optimistically on its own state, and only when both states are the
        // same do the comment/part ids it minted locally match the server's.
        // Otherwise the author asks for a snapshot instead of diverging.
        let allocates =
            op.kind == crate::wire::OpKind::Command && op.payload.get("command").and_then(Value::as_str).is_some_and(crate::wire::may_allocate_ids);
        if allocates && op.base_version != self.journal.head() {
            return Err(Reject::new(op.op_id.clone(), RejectCode::Stale, "a command that allocates ids needs baseVersion == head; resync"));
        }
        let shape_before = allocates.then(|| self.model.story_shape());
        if allocates {
            self.ids.start_delta();
        }
        let effect = match self.model.apply(actor, &op, &mut self.ids) {
            Ok(effect) => effect,
            Err(reject) => {
                if allocates {
                    self.ids.cancel_delta();
                }
                return Err(reject);
            }
        };
        // The delta names the ids the command issued. It is only safe when no
        // existing story changed size: a block inserted into a story shifts
        // every later path, and indexing on top of that would leave two ids
        // naming the same node. Such a command keeps the snapshot path.
        let node_delta = if allocates {
            let appended_only = shape_before.as_deref().is_some_and(|before| stories_appended_only(before, &self.model.story_shape()));
            if appended_only {
                if effect.nodes_changed {
                    self.model.index_nodes(&mut self.ids);
                }
                // An empty delta is still an answer: it certifies the map did
                // not move, so the replica does not take a snapshot.
                Some(self.ids.take_delta().unwrap_or_default())
            } else {
                self.ids.cancel_delta();
                None
            }
        } else {
            None
        };
        let has_delta = effect.delta.is_some();
        self.nodes_changed = effect.nodes_changed;
        let touched = effect.touched.clone();
        let mut applied = self.journal.accept(&op, actor, effect.delta, effect.inverse.clone());
        // Only the live answer carries the delta: a reconnecting replica gets
        // the whole map with `ServerMsg::Nodes`, so the journal and the `opId`
        // window store no node lists.
        applied.nodes = node_delta;
        self.note_touch(&op, applied.version);
        for id in &touched {
            self.last_touch.insert(*id, applied.version);
        }
        if !effect.inverse.is_empty() || !effect.forward.is_empty() || has_delta {
            // A command has no inverse yet; recording it as an empty step makes
            // it a barrier, so undo cannot silently skip it and take back an
            // older step (ADR-0010).
            let step = UndoStep::new(effect.label, applied.version, effect.inverse, effect.forward).with_touched(touched);
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
        if operations.is_empty() {
            return Err(Reject::new(op.op_id.clone(), RejectCode::Unsupported, "the step has no operations to replay"));
        }
        if self.step_conflicts(&step) {
            return Err(Reject::new(op.op_id.clone(), RejectCode::UndoConflict, "the step's targets changed after it landed"));
        }

        // A step may hold several operations (a replaced range, a block swap, a
        // future command inverse). It is one transaction: every operation lands
        // or none does, and the document moves once (ADR-0010).
        let mut deltas = Vec::with_capacity(operations.len());
        let mut landed: Vec<(Op, Effect)> = Vec::with_capacity(operations.len());
        for operation in operations {
            let mut replayed = operation.clone();
            // The step's own ids were already used by the journal; the replay is
            // part of the caller's operation, so it must not be deduplicated.
            replayed.op_id = OpId::new(format!("history:{}", op.op_id.client), op.op_id.seq);
            replayed.base_version = self.journal.head();
            replayed.epoch = None;
            replayed.history = None;
            match self.model.apply(actor, &replayed, &mut self.ids) {
                Ok(effect) => {
                    if effect.nodes_changed {
                        self.nodes_changed = true;
                    }
                    if let Some(delta) = effect.delta.clone() {
                        deltas.push(delta);
                    }
                    landed.push((replayed, effect));
                }
                Err(mut reject) => {
                    // All or none: take back what already landed before telling
                    // the caller no. The inverses come from the same model, so a
                    // rollback only fails on a broken adapter.
                    let effects: Vec<Effect> = landed.into_iter().map(|(_, effect)| effect).collect();
                    let rolled_back = self.roll_back(actor, &effects);
                    if !rolled_back {
                        reject.message.push_str("; the step could not be rolled back, the room needs a snapshot");
                    }
                    return Err(reject);
                }
            }
        }
        let delta = match deltas.len() {
            0 => None,
            1 => deltas.pop(),
            _ => Some(serde_json::json!({"op": "transaction", "ops": deltas})),
        };

        let applied = self.journal.accept(op, actor, delta, Vec::new());
        for (operation, effect) in &landed {
            self.note_touch(operation, applied.version);
            for id in &effect.touched {
                self.last_touch.insert(*id, applied.version);
            }
        }
        if undo {
            self.undo.finish_undo(actor, applied.version);
        } else {
            self.undo.finish_redo(actor, applied.version);
        }
        Ok(applied)
    }

    /// Take back operations of a step that landed before a later one was
    /// refused, newest first, so the step is all or none. Returns whether the
    /// whole rollback succeeded; the inverses come from the same model, so a
    /// failure means a broken adapter, never hostile input.
    fn roll_back(&mut self, actor: &str, landed: &[Effect]) -> bool {
        let mut seq = 0_u64;
        for effect in landed.iter().rev() {
            if effect.inverse.is_empty() {
                // Nothing was changed, or the operation has no inverse (a
                // command); the latter cannot be part of a transaction.
                if effect.delta.is_some() {
                    return false;
                }
                continue;
            }
            for inverse in &effect.inverse {
                seq = seq.saturating_add(1);
                let mut replayed = inverse.clone();
                replayed.op_id = OpId::new("history:rollback", seq);
                replayed.base_version = self.journal.head();
                replayed.epoch = None;
                replayed.history = None;
                if self.model.apply(actor, &replayed, &mut self.ids).is_err() {
                    return false;
                }
            }
        }
        true
    }

    /// Whether another operation touched what `step` changed after it landed.
    /// Offsets and block indexes make a replayed operation meaningless once a
    /// neighbour moved, so the room refuses instead of guessing (ADR-0010).
    /// `touched` covers nodes an operation changed beside its target: a text
    /// edit inside a merged cell makes the merge's inverse unsafe, and undo
    /// answers `undo-conflict` instead of overwriting that edit.
    fn step_conflicts(&self, step: &UndoStep) -> bool {
        let touched_later = |id: &u32| self.last_touch.get(id).is_some_and(|version| *version > step.version);
        step.touched.iter().any(touched_later)
            || step.forward.iter().any(|forward| {
                if forward.kind.is_structural() { self.last_structural > step.version } else { touched_later(&forward.target.node_id) }
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

/// Whether every story the room already knew kept its size: only new stories
/// appeared, so indexing their blocks cannot move an existing node's path.
fn stories_appended_only(before: &[(String, usize)], after: &[(String, usize)]) -> bool {
    before.iter().all(|(story, count)| after.iter().any(|(name, now)| name == story && now == count))
}
