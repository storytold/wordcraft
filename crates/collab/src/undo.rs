//! Per-user undo stacks (ADR-0010).
//!
//! Undo is a **new operation**, not a rollback of the shared journal: the
//! server keeps, per actor, the inverse operations of the steps that actor
//! performed, and sends them as a normal operation when that actor asks to
//! undo. Another user's edits are never reverted, and the document version
//! only moves forward.
//!
//! A step is one user action (a typed word, a table insertion) and holds the
//! inverse operations for it. Redo replays the original operations, so the
//! stacks are two lists of steps per actor.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::wire::Op;

/// One undoable user action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoStep {
    /// Human label for the undo menu (`Typing`, `Insert table`).
    pub label: String,
    /// Document version right after this step landed. An undo is refused when
    /// another operation touched the same targets later (ADR-0010).
    pub version: crate::wire::Version,
    /// Operations that undo this step, already addressed and ready to send.
    pub inverse: Vec<Op>,
    /// Operations that redo it (the originals), kept for the redo stack.
    pub forward: Vec<Op>,
}

impl UndoStep {
    pub fn new(label: impl Into<String>, version: crate::wire::Version, inverse: Vec<Op>, forward: Vec<Op>) -> UndoStep {
        UndoStep { label: label.into(), version, inverse, forward }
    }
}

#[derive(Default)]
struct ActorHistory {
    done: Vec<UndoStep>,
    undone: Vec<UndoStep>,
}

/// Undo/redo stacks for every actor in a room.
pub struct UndoStacks {
    actors: HashMap<String, ActorHistory>,
    limit: usize,
}

impl Default for UndoStacks {
    fn default() -> Self {
        UndoStacks::new(200)
    }
}

impl UndoStacks {
    /// `limit` caps the steps kept per actor; older steps fall off the bottom.
    pub fn new(limit: usize) -> UndoStacks {
        UndoStacks { actors: HashMap::new(), limit: limit.max(1) }
    }

    /// Record a step after its operations were accepted. Recording a new step
    /// clears that actor's redo stack, as every editor does.
    pub fn record(&mut self, actor: &str, step: UndoStep) {
        let history = self.actors.entry(actor.to_string()).or_default();
        history.undone.clear();
        history.done.push(step);
        while history.done.len() > self.limit {
            history.done.remove(0);
        }
    }

    /// The newest step `actor` can undo, without moving the stacks. The caller
    /// applies its inverse and then calls [`UndoStacks::finish_undo`].
    pub fn peek_undo(&self, actor: &str) -> Option<UndoStep> {
        self.actors.get(actor)?.done.last().cloned()
    }

    /// Move the newest step to the redo stack after its inverse landed at
    /// `version`. That version becomes the step's new baseline: a redo is
    /// allowed until something else touches the same targets.
    pub fn finish_undo(&mut self, actor: &str, version: crate::wire::Version) {
        let Some(history) = self.actors.get_mut(actor) else { return };
        if let Some(mut step) = history.done.pop() {
            step.version = version;
            history.undone.push(step);
        }
    }

    /// Take the newest step to undo (the caller sends [`UndoStep::inverse`] as
    /// a new operation and then calls [`UndoStacks::finish_undo`]).
    pub fn undo(&mut self, actor: &str) -> Option<UndoStep> {
        let step = self.peek_undo(actor)?;
        self.finish_undo(actor, step.version);
        Some(step)
    }

    /// The newest step `actor` can redo, without moving the stacks.
    pub fn peek_redo(&self, actor: &str) -> Option<UndoStep> {
        self.actors.get(actor)?.undone.last().cloned()
    }

    /// Move the newest undone step back to the done stack after it replayed at
    /// `version`.
    pub fn finish_redo(&mut self, actor: &str, version: crate::wire::Version) {
        let Some(history) = self.actors.get_mut(actor) else { return };
        if let Some(mut step) = history.undone.pop() {
            step.version = version;
            history.done.push(step);
        }
    }

    /// Take the newest undone step to redo (the caller sends
    /// [`UndoStep::forward`], then calls [`UndoStacks::finish_redo`]).
    pub fn redo(&mut self, actor: &str) -> Option<UndoStep> {
        let step = self.peek_redo(actor)?;
        self.finish_redo(actor, step.version);
        Some(step)
    }

    /// `(undoable, redoable)` for the menu and the UI state.
    pub fn depth(&self, actor: &str) -> (usize, usize) {
        self.actors.get(actor).map_or((0, 0), |h| (h.done.len(), h.undone.len()))
    }

    /// The labels the undo menu shows, newest first.
    pub fn labels(&self, actor: &str) -> Vec<&str> {
        self.actors.get(actor).map_or_else(Vec::new, |h| h.done.iter().rev().map(|s| s.label.as_str()).collect())
    }

    /// Drop one actor's history (leave, revoke, commit).
    pub fn clear(&mut self, actor: &str) {
        self.actors.remove(actor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{OpId, OpKind, Target};

    fn op(client: &str, seq: u64) -> Op {
        Op {
            op_id: OpId::new(client, seq),
            base_version: seq,
            epoch: None,
            kind: OpKind::Primitive,
            target: Target::new("body", 1, 0),
            payload: serde_json::Value::Null,
            history: None,
        }
    }

    fn step(label: &str, client: &str, seq: u64) -> UndoStep {
        UndoStep::new(label, seq, vec![op(client, seq)], vec![op(client, seq + 100)])
    }

    #[test]
    fn undo_is_per_actor() {
        let mut stacks = UndoStacks::new(10);
        stacks.record("alice", step("Typing", "a", 1));
        stacks.record("bob", step("Insert table", "b", 2));

        let alice = stacks.undo("alice").expect("alice has a step");
        assert_eq!(alice.label, "Typing");
        assert_eq!(stacks.depth("alice"), (0, 1));
        assert_eq!(stacks.depth("bob"), (1, 0), "bob's step is untouched");
    }

    #[test]
    fn redo_replays_the_forward_operations() {
        let mut stacks = UndoStacks::new(10);
        stacks.record("alice", step("Typing", "a", 1));
        let undone = stacks.undo("alice").expect("undo");
        assert_eq!(undone.inverse.len(), 1);

        let redone = stacks.redo("alice").expect("redo");
        assert_eq!(redone.forward.len(), 1);
        assert_eq!(stacks.depth("alice"), (1, 0));
    }

    #[test]
    fn a_new_step_clears_redo() {
        let mut stacks = UndoStacks::new(10);
        stacks.record("alice", step("Typing", "a", 1));
        let _ = stacks.undo("alice");
        assert_eq!(stacks.depth("alice"), (0, 1));

        stacks.record("alice", step("Bold", "a", 2));
        assert_eq!(stacks.depth("alice"), (1, 0));
    }

    #[test]
    fn the_limit_drops_the_oldest_step() {
        let mut stacks = UndoStacks::new(2);
        stacks.record("alice", step("One", "a", 1));
        stacks.record("alice", step("Two", "a", 2));
        stacks.record("alice", step("Three", "a", 3));
        assert_eq!(stacks.depth("alice").0, 2);
        assert_eq!(stacks.labels("alice"), vec!["Three", "Two"]);
    }

    #[test]
    fn unknown_actor_has_no_history() {
        let stacks = UndoStacks::new(10);
        assert_eq!(stacks.depth("nobody"), (0, 0));
        assert!(stacks.labels("nobody").is_empty());
    }
}
