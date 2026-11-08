//! Adapter tests: two clients through one room, undo through inverse operations.

use serde_json::json;
use wordcraft_collab::ids::{NodeIds, NodeKey};
use wordcraft_collab::room::{Model, Role, Room};
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

fn submit_and_mirror(room: &mut Room<DocModel>, op: Op, replica: &mut Document, ids: &mut NodeIds) {
    let applied = room.submit("alice", Role::Editor, op).expect("the room accepts");
    if let Some(delta) = &applied.delta {
        crate::apply_delta(replica, ids, delta).expect("the replica applies the delta");
    }
    assert_eq!(
        serde_json::to_value(&*replica).expect("replica json"),
        serde_json::to_value(room.model().document()).expect("server json"),
        "the replica converged after version {}",
        applied.version
    );
}

#[test]
fn a_replica_applies_the_deltas_and_converges() {
    let mut room = Room::new(DocModel::new(Document::new()));
    let _ = room.join(None, false);
    let first = node(room.ids_mut(), 0);

    // The replica builds its own map from the same document; the ids are
    // deterministic, so both sides agree.
    let mut replica = Document::new();
    let mut replica_ids = NodeIds::new();
    DocModel::new(replica.clone()).index_nodes(&mut replica_ids);

    submit_and_mirror(
        &mut room,
        op("a", 1, 0, OpKind::Primitive, first, 0, json!({"op": "insert_text", "text": "Hello"})),
        &mut replica,
        &mut replica_ids,
    );
    submit_and_mirror(
        &mut room,
        op("a", 2, 1, OpKind::Primitive, first, 0, json!({"op": "replace_range", "len": 5, "text": "Hi"})),
        &mut replica,
        &mut replica_ids,
    );
    submit_and_mirror(&mut room, op("a", 3, 2, OpKind::Primitive, first, 2, json!({"op": "split_para"})), &mut replica, &mut replica_ids);
    submit_and_mirror(&mut room, op("a", 4, 3, OpKind::Structural, 0, 0, json!({"op": "insert_block", "index": 1})), &mut replica, &mut replica_ids);
    submit_and_mirror(&mut room, op("a", 5, 4, OpKind::Structural, 0, 0, json!({"op": "remove_block", "index": 1})), &mut replica, &mut replica_ids);
    submit_and_mirror(&mut room, op("a", 6, 5, OpKind::Primitive, first, 2, json!({"op": "merge_para"})), &mut replica, &mut replica_ids);
}

#[test]
fn a_command_delta_is_refused_by_the_document_replica() {
    let mut doc = Document::new();
    let mut ids = NodeIds::new();
    let err = crate::apply_delta(&mut doc, &mut ids, &json!({"op": "command", "command": "text.insert"})).expect_err("needs a session");
    assert!(err.contains("Session"), "{err}");
}

fn typed(text: &str) -> Document {
    let mut doc = Document::new();
    let pos = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
    doc.insert_text(&pos, text, &wordcraft_doc::CharProps::default()).expect("type");
    doc
}

fn diff_and_apply(before: Document, after: Document) -> Document {
    let mut ids = NodeIds::new();
    DocModel::new(before.clone()).index_nodes(&mut ids);
    let changes = crate::diff(&before, &after, &ids).expect("a supported difference");
    assert!(!changes.is_empty(), "a difference produced no operation");
    let mut room = Room::new(DocModel::new(before));
    let _ = room.join(None, false);
    for (index, change) in changes.into_iter().enumerate() {
        let op = Op {
            op_id: OpId::new("a", index as u64 + 1),
            base_version: index as u64,
            epoch: None,
            kind: change.kind,
            target: change.target,
            payload: change.payload,
            history: None,
        };
        room.submit("alice", Role::Editor, op).expect("the derived operation is accepted");
    }
    room.model().document().clone()
}

fn assert_same(actual: &Document, expected: &Document) {
    assert_eq!(serde_json::to_value(actual).expect("json"), serde_json::to_value(expected).expect("json"));
}

#[test]
fn typing_diffs_to_a_replace_range() {
    let before = typed("Hello");
    let mut after = before.clone();
    let pos = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 5 };
    after.insert_text(&pos, " world", &wordcraft_doc::CharProps::default()).expect("type more");
    assert_same(&diff_and_apply(before, after.clone()), &after);
}

#[test]
fn a_deletion_diffs_to_a_replace_range() {
    let before = typed("Hello world");
    let mut after = before.clone();
    let start = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 5 };
    let end = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 11 };
    after.delete_range(&start, &end).expect("delete");
    assert_same(&diff_and_apply(before, after.clone()), &after);
}

#[test]
fn an_enter_diffs_to_split_para() {
    let before = typed("Hello");
    let mut after = before.clone();
    let pos = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 2 };
    after.split_paragraph(&pos).expect("split");
    assert_same(&diff_and_apply(before, after.clone()), &after);
}

#[test]
fn a_backspace_at_the_paragraph_start_diffs_to_merge_para() {
    let mut before = typed("Hello");
    let pos = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 2 };
    before.split_paragraph(&pos).expect("split");
    let mut after = before.clone();
    let start = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 2 };
    let end = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(1), off: 0 };
    after.delete_range(&start, &end).expect("merge");
    assert_same(&diff_and_apply(before, after.clone()), &after);
}

#[test]
fn an_inserted_paragraph_diffs_to_insert_block() {
    let before = typed("Hello");
    let mut after = before.clone();
    after
        .insert_block(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(1), wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::new()))
        .expect("insert");
    assert_same(&diff_and_apply(before, after.clone()), &after);
}

#[test]
fn a_removed_paragraph_diffs_to_remove_block() {
    let mut before = typed("Hello");
    before
        .insert_block(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(1), wordcraft_doc::Block::Para(wordcraft_doc::Paragraph::new()))
        .expect("insert");
    let mut after = before.clone();
    after.remove_block(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(1)).expect("remove");
    assert_same(&diff_and_apply(before, after.clone()), &after);
}

#[test]
fn a_formatting_only_change_is_reported_as_unsupported() {
    let before = typed("Hello");
    let mut after = before.clone();
    let start = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
    let end = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 5 };
    after.format_range(&start, &end, &|props| props.bold = Some(true)).expect("format");
    let ids = NodeIds::new();
    assert!(matches!(crate::diff(&before, &after, &ids), Err(crate::DiffError::Unsupported(_))));
    // Keep the variable used: the before document is untouched by the diff.
    let _ = before.body.len();
}
