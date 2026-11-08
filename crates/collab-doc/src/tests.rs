//! Adapter tests: two clients through one room, undo through inverse operations.

use serde_json::json;
use wordcraft_collab::ids::{NodeIds, NodeKey};
use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{Op, OpId, OpKind, Target};
use wordcraft_doc::Document;

use crate::DocModel;

fn node(ids: &mut NodeIds, block: u32) -> u32 {
    ids.id_for(&NodeKey::new("body", vec![block])).0
}

fn op(client: &str, seq: u64, base: u64, kind: OpKind, node_id: u32, off: usize, payload: serde_json::Value) -> Op {
    Op { op_id: OpId::new(client, seq), base_version: base, epoch: None, kind, target: Target::new("body", node_id, off), payload, history: None }
}

fn room() -> Room<DocModel> {
    Room::new(DocModel::new(Document::new()))
}

fn body_text(room: &Room<DocModel>, block: usize) -> String {
    room.model().document().body.get(block).and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default()
}

fn paragraphs(room: &Room<DocModel>) -> usize {
    room.model().document().body.len()
}

#[test]
fn two_clients_type_into_one_paragraph() {
    let mut room = room();
    let first = node(room.ids_mut(), 0);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, first, 0, json!({"op": "insert_text", "text": "Hello"})))
        .expect("alice types");
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, first, 5, json!({"op": "insert_text", "text": " world"}))).expect("bob types");

    assert_eq!(body_text(&room, 0), "Hello world");
    assert_eq!(room.version(), 2);
}

#[test]
fn undoing_an_insert_removes_exactly_that_text() {
    let mut room = room();
    let first = node(room.ids_mut(), 0);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, first, 0, json!({"op": "insert_text", "text": "Hello"}))).expect("type");

    let step = room.undo("alice").expect("undo step");
    assert_eq!(step.label, "Typing");
    let inverse = step.inverse.first().expect("inverse operation").clone();
    let mut undo_op = inverse;
    undo_op.base_version = room.version();
    room.submit("alice", Role::Editor, undo_op).expect("undo lands");

    assert_eq!(body_text(&room, 0), "");
}

#[test]
fn deleting_a_range_can_be_undone_with_the_removed_text() {
    let mut room = room();
    let first = node(room.ids_mut(), 0);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, first, 0, json!({"op": "insert_text", "text": "abcdef"}))).expect("type");
    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Primitive, first, 2, json!({"op": "delete_range", "len": 2}))).expect("delete");

    assert_eq!(body_text(&room, 0), "abef");

    let step = room.undo("alice").expect("undo step");
    let mut undo_op = step.inverse.first().expect("inverse").clone();
    undo_op.base_version = room.version();
    room.submit("alice", Role::Editor, undo_op).expect("undo lands");
    assert_eq!(body_text(&room, 0), "abcdef");
}

#[test]
fn splitting_a_paragraph_moves_ids_and_undo_joins_them() {
    let mut room = room();
    let first = node(room.ids_mut(), 0);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, first, 0, json!({"op": "insert_text", "text": "Hello"}))).expect("type");

    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Primitive, first, 2, json!({"op": "split_para"}))).expect("split");

    assert_eq!(paragraphs(&room), 2);
    assert_eq!(body_text(&room, 0), "He");
    assert_eq!(body_text(&room, 1), "llo");

    let step = room.undo("alice").expect("undo step");
    let mut undo_op = step.inverse.first().expect("inverse").clone();
    undo_op.base_version = room.version();
    room.submit("alice", Role::Editor, undo_op).expect("undo lands");

    assert_eq!(paragraphs(&room), 1);
    assert_eq!(body_text(&room, 0), "Hello");
}

#[test]
fn inserting_a_block_shifts_later_ids() {
    let mut room = room();
    let first = node(room.ids_mut(), 0);
    let inserted = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, first, 0, json!({"op": "insert_block", "index": 1})))
        .expect("insert block");

    assert_eq!(inserted.epoch, 1);
    assert_eq!(paragraphs(&room), 2);
    // The old paragraph keeps id 1; the new one is a different block.
    assert_eq!(room.ids().resolve(wordcraft_collab::NodeId(first)).map(|k| k.path.clone()), Ok(vec![0]));
}

#[test]
fn removing_a_block_is_a_tombstone_and_undo_restores_it() {
    let mut room = room();
    let first = node(room.ids_mut(), 0);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, first, 0, json!({"op": "insert_text", "text": "keep"}))).expect("type");

    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, first, 0, json!({"op": "remove_block"}))).expect("remove block");
    assert_eq!(paragraphs(&room), 1);
    assert!(room.ids().is_tombstoned(wordcraft_collab::NodeId(first)));

    let step = room.undo("alice").expect("undo step");
    let mut undo_op = step.inverse.first().expect("inverse").clone();
    undo_op.base_version = room.version();
    room.submit("alice", Role::Editor, undo_op).expect("undo lands");
    assert_eq!(paragraphs(&room), 2);
    assert_eq!(body_text(&room, 0), "keep");
}

#[test]
fn a_late_edit_on_a_deleted_paragraph_is_gone() {
    let mut room = room();
    let first = node(room.ids_mut(), 0);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, first, 0, json!({"op": "insert_block", "index": 1}))).expect("insert");
    let second = node(room.ids_mut(), 1);
    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, second, 0, json!({"op": "remove_block"}))).expect("remove");

    let err = room
        .submit("bob", Role::Editor, op("b", 1, 2, OpKind::Primitive, second, 0, json!({"op": "insert_text", "text": "x"})))
        .expect_err("the paragraph is gone");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Gone);
}

#[test]
fn commands_are_not_wired_to_the_document_adapter() {
    let mut room = room();
    let first = node(room.ids_mut(), 0);
    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "format.bold"})))
        .expect_err("command is refused");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Invalid);
}
