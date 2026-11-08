//! Room-level tests with a fake model: ordering, dedup, roles, undo, catch-up.

use serde_json::json;

use crate::room::{Effect, Model, Role, Room};
use crate::wire::{HistoryAction, Op, OpId, OpKind, RejectCode, Target};

/// A one-string document, enough to observe ordering and deltas.
#[derive(Default)]
struct TextModel {
    text: String,
}

impl TextModel {
    fn apply_insert(&mut self, op: &Op) -> Result<Effect, crate::wire::Reject> {
        let text = op.payload.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        if op.target.off > self.text.len() || !self.text.is_char_boundary(op.target.off) {
            return Err(crate::wire::Reject::new(op.op_id.clone(), RejectCode::Invalid, "offset is not a char boundary"));
        }
        let inverse = Op {
            op_id: OpId::new("server", 0),
            base_version: 0,
            epoch: None,
            kind: OpKind::Primitive,
            target: Target::new("body", op.target.node_id, op.target.off),
            payload: json!({"delete": text.len()}),
            history: None,
        };
        self.text.insert_str(op.target.off, text);
        Ok(Effect {
            delta: Some(json!({"insert": text, "off": op.target.off})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Typing".into(),
            nodes_changed: false,
        })
    }

    fn apply_delete(&mut self, op: &Op) -> Result<Effect, crate::wire::Reject> {
        let len = op.payload.get("delete").and_then(|v| v.as_u64()).unwrap_or_default();
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        let end = op.target.off.saturating_add(len);
        if end > self.text.len() || !self.text.is_char_boundary(op.target.off) || !self.text.is_char_boundary(end) {
            return Err(crate::wire::Reject::new(op.op_id.clone(), RejectCode::Invalid, "range is not a char boundary"));
        }
        let removed = self.text[op.target.off..end].to_string();
        self.text.replace_range(op.target.off..end, "");
        let inverse = Op {
            op_id: OpId::new("server", 0),
            base_version: 0,
            epoch: None,
            kind: OpKind::Primitive,
            target: Target::new("body", op.target.node_id, op.target.off),
            payload: json!({"text": removed}),
            history: None,
        };
        Ok(Effect {
            delta: Some(json!({"delete": len, "off": op.target.off})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Delete".into(),
            nodes_changed: false,
        })
    }
}

impl Model for TextModel {
    fn apply(&mut self, _actor: &str, op: &Op, _ids: &mut crate::ids::NodeIds) -> Result<Effect, crate::wire::Reject> {
        match op.kind {
            OpKind::Primitive if op.payload.get("delete").is_some() => self.apply_delete(op),
            OpKind::Primitive => self.apply_insert(op),
            OpKind::Structural => {
                self.text.push('#');
                Ok(Effect { delta: None, inverse: Vec::new(), forward: vec![op.clone()], label: "Structural".into(), nodes_changed: true })
            }
            OpKind::Command | OpKind::Merge => {
                Ok(Effect { delta: None, inverse: Vec::new(), forward: Vec::new(), label: "Merge".into(), nodes_changed: false })
            }
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        self.text.clone().into_bytes()
    }

    fn index_nodes(&self, _ids: &mut crate::ids::NodeIds) {}
}

fn insert(client: &str, seq: u64, base: u64, off: usize, text: &str) -> Op {
    Op {
        op_id: OpId::new(client, seq),
        base_version: base,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", 1, off),
        payload: json!({"text": text}),
        history: None,
    }
}

#[test]
fn two_clients_converge_in_server_order() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "Hello")).expect("alice types");
    room.submit("bob", Role::Editor, insert("b", 1, 1, 5, " world")).expect("bob types");

    assert_eq!(room.model().text, "Hello world");
    assert_eq!(room.version(), 2);
}

#[test]
fn concurrent_inserts_in_one_paragraph_both_land() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "AC")).expect("alice types");
    // Bob saw version 0: his operation is behind, but text primitives are merge
    // class and land on head with the authoritative delta.
    let applied = room.submit("bob", Role::Editor, insert("b", 1, 0, 1, "B")).expect("bob types");
    assert!(applied.conflict, "bob's base was behind");
    assert_eq!(room.model().text, "ABC");
}

#[test]
fn a_repeated_op_id_is_not_applied_twice() {
    let mut room = Room::new(TextModel::default());
    let op = insert("a", 7, 0, 0, "x");
    let first = room.submit("alice", Role::Editor, op.clone()).expect("first");
    let again = room.submit("alice", Role::Editor, op).expect("dedup");

    assert_eq!(first, again);
    assert_eq!(room.version(), 1);
    assert_eq!(room.model().text, "x");
}

#[test]
fn viewer_cannot_write() {
    let mut room = Room::new(TextModel::default());
    let err = room.submit("carol", Role::Viewer, insert("c", 1, 0, 0, "x")).expect_err("viewer is refused");
    assert_eq!(err.code, RejectCode::Forbidden);
    assert_eq!(room.version(), 0);
    assert!(room.model().text.is_empty());
}

#[test]
fn structural_operations_serialize_on_the_head() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "x")).expect("alice");

    let mut structural = insert("b", 1, 0, 0, "y");
    structural.kind = OpKind::Structural;
    let err = room.submit("bob", Role::Editor, structural).expect_err("stale structural");
    assert_eq!(err.code, RejectCode::Conflict);

    let mut fresh = insert("b", 2, 1, 1, "y");
    fresh.kind = OpKind::Structural;
    room.submit("bob", Role::Editor, fresh).expect("head structural");
    assert_eq!(room.epoch(), 1);
}

#[test]
fn undo_is_per_user() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "a")).expect("alice");
    room.submit("bob", Role::Editor, insert("b", 1, 1, 1, "b")).expect("bob");

    assert_eq!(room.undo_depth("alice"), (1, 0));
    let step = room.undo("alice").expect("alice has a step");
    assert_eq!(step.label, "Typing");
    assert_eq!(step.inverse.len(), 1);
    assert_eq!(room.undo_depth("bob"), (1, 0), "bob's undo stack is separate");
}

fn history_op(client: &str, seq: u64, base: u64, action: HistoryAction) -> Op {
    Op {
        op_id: OpId::new(client, seq),
        base_version: base,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", 1, 0),
        payload: json!({}),
        history: Some(action),
    }
}

fn undo_op(client: &str, seq: u64, base: u64, step_client: &str, step_seq: u64) -> Op {
    history_op(client, seq, base, HistoryAction::Undo { op_id: OpId::new(step_client, step_seq) })
}

fn redo_op(client: &str, seq: u64, base: u64, step_client: &str, step_seq: u64) -> Op {
    history_op(client, seq, base, HistoryAction::Redo { op_id: OpId::new(step_client, step_seq) })
}

#[test]
fn undo_replays_the_inverse_as_one_versioned_operation() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "Hi")).expect("type");

    let applied = room.submit("alice", Role::Editor, undo_op("a", 2, 1, "a", 1)).expect("undo");
    assert_eq!(applied.version, 2, "undo is one operation, not one per replayed step");
    assert_eq!(applied.delta, Some(json!({"delete": 2, "off": 0})));
    assert_eq!(room.model().text, "");
    assert_eq!(room.undo_depth("alice"), (0, 1));
    assert_eq!(room.version(), 2);
}

#[test]
fn redo_replays_the_original_operation() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "Hi")).expect("type");
    room.submit("alice", Role::Editor, undo_op("a", 2, 1, "a", 1)).expect("undo");

    let applied = room.submit("alice", Role::Editor, redo_op("a", 3, 2, "a", 1)).expect("redo");
    assert_eq!(applied.version, 3);
    assert_eq!(room.model().text, "Hi");
    assert_eq!(room.undo_depth("alice"), (1, 0));
}

#[test]
fn undo_is_refused_when_another_user_touched_the_same_node() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "a")).expect("alice");
    room.submit("bob", Role::Editor, insert("b", 1, 1, 1, "b")).expect("bob");

    let err = room.submit("alice", Role::Editor, undo_op("a", 2, 2, "a", 1)).expect_err("conflict");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(room.model().text, "ab", "the refused undo changed nothing");
    assert_eq!(room.version(), 2);
    assert_eq!(room.undo_depth("alice"), (1, 0));
}

#[test]
fn undo_only_takes_the_newest_step() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "a")).expect("first");
    room.submit("alice", Role::Editor, insert("a", 2, 1, 1, "b")).expect("second");

    let err = room.submit("alice", Role::Editor, undo_op("a", 3, 2, "a", 1)).expect_err("not newest");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(room.model().text, "ab");
}

#[test]
fn a_repeated_undo_is_deduplicated() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "Hi")).expect("type");
    let undo = undo_op("a", 2, 1, "a", 1);
    let first = room.submit("alice", Role::Editor, undo.clone()).expect("undo");
    let again = room.submit("alice", Role::Editor, undo).expect("dedup");

    assert_eq!(first, again);
    assert_eq!(room.model().text, "");
    assert_eq!(room.version(), 2);
    assert_eq!(room.undo_depth("alice"), (0, 1), "the stack did not move twice");
}

#[test]
fn undo_without_a_step_is_refused() {
    let mut room = Room::new(TextModel::default());
    let err = room.submit("alice", Role::Editor, undo_op("a", 1, 0, "a", 99)).expect_err("nothing to undo");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(room.version(), 0);
}

#[test]
fn a_viewer_cannot_undo() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "a")).expect("alice");
    let err = room.submit("carol", Role::Viewer, undo_op("c", 1, 1, "a", 1)).expect_err("viewer");
    assert_eq!(err.code, RejectCode::Forbidden);
    assert_eq!(room.model().text, "a");
}

#[test]
fn join_replays_the_tail_or_hands_out_a_snapshot() {
    let mut room = Room::new(TextModel::default());
    for seq in 1..=3 {
        room.submit("alice", Role::Editor, insert("a", seq, seq - 1, (seq as usize) - 1, "x")).expect("op");
    }

    let tail = room.join(Some(1), false);
    assert_eq!(tail.replay.as_ref().map(Vec::len), Some(2));
    assert!(tail.snapshot.is_none());

    let fresh = room.join(None, false);
    assert_eq!(fresh.snapshot, Some(b"xxx".to_vec()));
    assert!(fresh.replay.is_none());
}

#[test]
fn an_offset_inside_a_character_is_refused() {
    let mut room = Room::new(TextModel::default());
    room.submit("alice", Role::Editor, insert("a", 1, 0, 0, "ё")).expect("two-byte character");
    // Byte 1 is inside "ё" (2 bytes in UTF-8).
    let err = room.submit("alice", Role::Editor, insert("a", 2, 1, 1, "x")).expect_err("split character");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(room.model().text, "ё");
}

#[test]
fn roles_parse_and_gate_commits() {
    assert_eq!(Role::parse("editor"), Some(Role::Editor));
    assert_eq!(Role::parse("admin"), None);
    assert!(Role::Owner.can_commit());
    assert!(!Role::Editor.can_commit());
    assert!(!Role::Viewer.can_write());
    assert!(Role::Commenter.can_write());
}

#[test]
fn a_commenter_may_only_annotate() {
    let mut room = Room::new(TextModel::default());
    let err = room.submit("carol", Role::Commenter, insert("c", 1, 0, 0, "text")).expect_err("a commenter cannot type");
    assert_eq!(err.code, RejectCode::Forbidden);

    let mut comment = insert("c", 1, 0, 0, "");
    comment.payload = json!({"op": "add_comment", "text": "please check"});
    room.submit("carol", Role::Commenter, comment).expect("a comment is allowed");
}
