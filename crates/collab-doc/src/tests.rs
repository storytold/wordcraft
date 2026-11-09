//! Adapter tests: two clients through one room, undo through inverse operations.

use serde_json::json;
use std::sync::Arc;
use wordcraft_collab::ids::{NodeIds, NodeKey};
use wordcraft_collab::room::{Model, Role, Room};
use wordcraft_collab::wire::{Op, OpId, OpKind, Target};
use wordcraft_doc::{Block, Cell, CharProps, Document, Path, Pos, StoryRef, Table};

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

fn body_line(doc: &Document) -> String {
    doc.body.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default()
}

#[test]
fn a_transaction_delta_lands_all_or_nothing() {
    let mut doc = typed("abcdef");
    let mut ids = NodeIds::new();
    DocModel::new(doc.clone()).index_nodes(&mut ids);
    let first = node(&mut ids, 0);

    crate::apply_delta(
        &mut doc,
        &mut ids,
        &json!({"op": "transaction", "ops": [
            {"op": "delete_range", "nodeId": first, "off": 0, "len": 1},
            {"op": "delete_range", "nodeId": first, "off": 0, "len": 1}
        ]}),
    )
    .expect("a step of two deletes replays as one delta");
    assert_eq!(body_line(&doc), "cdef");

    let err = crate::apply_delta(
        &mut doc,
        &mut ids,
        &json!({"op": "transaction", "ops": [
            {"op": "delete_range", "nodeId": first, "off": 0, "len": 1},
            {"op": "insert_text", "nodeId": 9999, "off": 0, "text": "x"}
        ]}),
    )
    .expect_err("the second operation addresses an unknown node");
    assert!(err.contains("unknown node"), "{err}");
    assert_eq!(body_line(&doc), "cdef", "the first operation was taken back with the second");
}

#[test]
fn a_nested_transaction_is_refused() {
    let mut doc = Document::new();
    let mut ids = NodeIds::new();
    let err = crate::apply_delta(&mut doc, &mut ids, &json!({"op": "transaction", "ops": [{"op": "transaction", "ops": []}]}))
        .expect_err("nested transactions are not a wire form");
    assert!(err.contains("nested"), "{err}");
}

// --- tables: ids for tables, rows, cells and their paragraphs -------------

/// A document whose second block is a `rows × cols` table; every cell reads
/// `r{row}c{col}`, so a misplaced edit is visible.
fn table_doc(rows: usize, cols: usize) -> Document {
    let mut doc = Document::new();
    let mut table = Table::new(rows, cols, 200.0);
    for (r, row) in table.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = Cell::with_text(&format!("r{r}c{c}"));
        }
    }
    doc.body.push(Arc::new(Block::Table(table)));
    doc
}

const TABLE: u32 = 1;

fn table_id(ids: &NodeIds) -> u32 {
    ids.get(&NodeKey::new("body", vec![TABLE])).expect("the table has an id").0
}

fn row_id(ids: &NodeIds, row: u32) -> u32 {
    ids.get(&NodeKey::new("body", vec![TABLE, row])).expect("the row has an id").0
}

fn cell_id(ids: &NodeIds, row: u32, cell: u32) -> u32 {
    ids.get(&NodeKey::new("body", vec![TABLE, row, cell])).expect("the cell has an id").0
}

fn cell_para_id(ids: &NodeIds, row: u32, cell: u32) -> u32 {
    ids.get(&NodeKey::new("body", vec![TABLE, row, cell, 0])).expect("the cell paragraph has an id").0
}

fn cell_pos(row: u32, cell: u32, off: usize) -> Pos {
    Pos { story: StoryRef::Body, path: Path(vec![TABLE, row, cell, 0]), off }
}

fn cell_text(doc: &Document, row: u32, cell: u32) -> String {
    doc.para(StoryRef::Body, &Path(vec![TABLE, row, cell, 0])).map(|para| para.text.clone()).unwrap_or_default()
}

fn table_rows(doc: &Document) -> usize {
    doc.table(StoryRef::Body, &Path(vec![TABLE])).map_or(0, |table| table.rows.len())
}

#[test]
fn every_table_node_gets_an_id_of_its_own() {
    let doc = table_doc(2, 3);
    let mut ids = NodeIds::new();
    DocModel::new(doc).index_nodes(&mut ids);

    let table = table_id(&ids);
    assert_ne!(table, row_id(&ids, 0));
    assert_ne!(row_id(&ids, 0), row_id(&ids, 1));
    assert_ne!(cell_id(&ids, 0, 0), cell_id(&ids, 0, 1));
    assert_ne!(cell_id(&ids, 0, 0), cell_para_id(&ids, 0, 0));
    // The paragraphs of a cell belong to the cell, not to the story's block list.
    assert_eq!(ids.resolve(wordcraft_collab::NodeId(cell_para_id(&ids, 1, 2))).map(|key| key.path.clone()), Ok(vec![TABLE, 1, 2, 0]));
    // The block before the table and the table itself are distinct nodes.
    assert!(ids.get(&NodeKey::new("body", vec![0])).is_some());
    assert_ne!(ids.get(&NodeKey::new("body", vec![0])).map(|id| id.0), Some(table));
}

/// Two clients, each with its own replica and map, type in different cells of
/// one table through the room; both replicas end up with the room's document.
#[test]
fn two_replicas_edit_different_cells_of_one_table_and_converge() {
    let doc = table_doc(2, 2);
    let mut room = Room::new(DocModel::new(doc.clone()));
    let _ = room.join(None, false);

    let mut alice = doc.clone();
    let mut alice_ids = NodeIds::new();
    DocModel::new(alice.clone()).index_nodes(&mut alice_ids);
    let mut bob = doc.clone();
    let mut bob_ids = NodeIds::new();
    DocModel::new(bob.clone()).index_nodes(&mut bob_ids);

    // Alice types at the end of cell (0, 0); her replica already holds the edit.
    let alice_cell = cell_para_id(&alice_ids, 0, 0);
    alice.insert_text(&cell_pos(0, 0, 4), "A", &CharProps::default()).expect("alice types");
    let accepted = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, alice_cell, 4, json!({"op": "insert_text", "text": "A"})))
        .expect("alice's edit lands");
    if let Some(delta) = &accepted.delta {
        crate::apply_delta(&mut bob, &mut bob_ids, delta).expect("bob mirrors alice");
    }

    // Bob answers in another cell, on the version Alice's edit produced.
    let bob_cell = cell_para_id(&bob_ids, 1, 1);
    bob.insert_text(&cell_pos(1, 1, 4), "B", &CharProps::default()).expect("bob types");
    let accepted = room
        .submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, bob_cell, 4, json!({"op": "insert_text", "text": "B"})))
        .expect("bob's edit lands");
    if let Some(delta) = &accepted.delta {
        crate::apply_delta(&mut alice, &mut alice_ids, delta).expect("alice mirrors bob");
    }

    assert_eq!(cell_text(&alice, 0, 0), "r0c0A");
    assert_eq!(cell_text(&alice, 1, 1), "r1c1B");
    assert_eq!(cell_text(&bob, 0, 0), "r0c0A");
    assert_eq!(cell_text(&bob, 1, 1), "r1c1B");
    assert_same(&alice, room.model().document());
    assert_same(&bob, room.model().document());
    assert_eq!(room.version(), 2);
}

#[test]
fn a_repeated_cell_edit_after_a_reconnect_is_applied_once() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let cell = cell_para_id(room.ids(), 0, 0);
    let edit = op("a", 1, 0, OpKind::Primitive, cell, 4, json!({"op": "insert_text", "text": "!"}));

    let first = room.submit("alice", Role::Editor, edit.clone()).expect("the edit lands");
    // The client never saw the answer and resends the same operation after a
    // reconnect: the room answers from its `opId` window instead of applying it.
    let again = room.submit("alice", Role::Editor, edit).expect("the retry is answered from the window");

    assert_eq!(again.version, first.version);
    assert_eq!(room.version(), 1);
    assert_eq!(cell_text(room.model().document(), 0, 0), "r0c0!");
}

#[test]
fn a_row_insert_moves_the_cell_ids_and_an_active_edit_lands_in_its_own_cell() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);

    // Bob reads the id of his cell before Alice restructures the table.
    let bob_cell = cell_para_id(room.ids(), 1, 1);
    let table = table_id(room.ids());
    let empty_row = json!(Table::new(1, 2, 200.0).rows[0]);

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "insert_row", "index": 0, "row": empty_row})))
        .expect("the row is inserted");

    assert_eq!(table_rows(room.model().document()), 3);
    // Bob's id moved with his cell: row 1 became row 2, the cell is the same.
    assert_eq!(room.ids().resolve(wordcraft_collab::NodeId(bob_cell)).map(|key| key.path.clone()), Ok(vec![TABLE, 2, 1, 0]));

    // His edit arrives after the insert and still lands in the cell he was in.
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, bob_cell, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("the stale address still names Bob's cell");

    let doc = room.model().document();
    assert_eq!(cell_text(doc, 2, 1), "r1c1!");
    assert_eq!(cell_text(doc, 1, 1), "r0c1", "the shifted row kept its own text");
    assert_eq!(cell_text(doc, 0, 1), "", "the new row is empty");
}

#[test]
fn a_removed_row_tombstones_its_cells_and_a_late_edit_is_gone() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let doomed = cell_para_id(room.ids(), 0, 1);

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "remove_row", "index": 0})))
        .expect("the row is removed");

    assert_eq!(table_rows(room.model().document()), 1);
    assert_eq!(cell_text(room.model().document(), 0, 1), "r1c1", "the remaining row moved up");
    let err = room
        .submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, doomed, 4, json!({"op": "insert_text", "text": "!"})))
        .expect_err("the cell is gone");
    assert_eq!(err.code, wordcraft_collab::RejectCode::Gone);
}

#[test]
fn splitting_a_paragraph_inside_a_cell_moves_only_that_cell() {
    let mut room = Room::new(DocModel::new(table_doc(1, 2)));
    let _ = room.join(None, false);
    let in_cell = cell_para_id(room.ids(), 0, 0);
    let other = cell_para_id(room.ids(), 0, 1);

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, in_cell, 2, json!({"op": "split_para"}))).expect("the cell paragraph splits");

    let doc = room.model().document();
    assert_eq!(doc.para(StoryRef::Body, &Path(vec![TABLE, 0, 0, 0])).map(|p| p.text.clone()), Some("r0".to_string()));
    assert_eq!(doc.para(StoryRef::Body, &Path(vec![TABLE, 0, 0, 1])).map(|p| p.text.clone()), Some("c0".to_string()));
    // The other cell and the story's own blocks did not move.
    assert_eq!(room.ids().resolve(wordcraft_collab::NodeId(other)).map(|key| key.path.clone()), Ok(vec![TABLE, 0, 1, 0]));
    assert!(room.ids().get(&NodeKey::new("body", vec![TABLE, 0, 0, 1])).is_some());
}

#[test]
fn typing_in_a_cell_diffs_to_a_replace_range() {
    let before = table_doc(2, 2);
    let mut after = before.clone();
    after.insert_text(&cell_pos(1, 0, 4), "!", &CharProps::default()).expect("type in the cell");
    let applied = diff_and_apply(before, after.clone());
    assert_same(&applied, &after);
    assert_eq!(cell_text(&applied, 1, 0), "r1c0!");
}

#[test]
fn inserting_a_row_diffs_to_insert_row() {
    let before = table_doc(2, 2);
    let mut after = before.clone();
    after.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").insert_row(1, 0);
    let applied = diff_and_apply(before, after.clone());
    assert_same(&applied, &after);
}

#[test]
fn deleting_a_row_diffs_to_remove_row() {
    let before = table_doc(3, 2);
    let mut after = before.clone();
    after.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").delete_row(1);
    let applied = diff_and_apply(before, after.clone());
    assert_same(&applied, &after);
}

#[test]
fn a_column_insert_is_still_reported_as_unsupported() {
    let before = table_doc(2, 2);
    let mut after = before.clone();
    after.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").insert_col(1);
    let mut ids = NodeIds::new();
    DocModel::new(before.clone()).index_nodes(&mut ids);
    assert!(matches!(crate::diff(&before, &after, &ids), Err(crate::DiffError::Unsupported(_))));
}

#[test]
fn inserting_a_table_diffs_to_insert_block_and_its_cells_get_ids() {
    let before = typed("Hello");
    let mut after = before.clone();
    let mut table = Table::new(2, 2, 200.0);
    for (r, row) in table.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = Cell::with_text(&format!("r{r}c{c}"));
        }
    }
    after.insert_block(StoryRef::Body, &Path::top(1), Block::Table(table)).expect("the table is inserted");
    let applied = diff_and_apply(before, after.clone());
    assert_same(&applied, &after);
    // The inserted table is addressable right away, so the next edit in a cell
    // is a primitive and does not resync.
    let mut ids = NodeIds::new();
    DocModel::new(applied.clone()).index_nodes(&mut ids);
    assert!(ids.get(&NodeKey::new("body", vec![1, 1, 1, 0])).is_some());
}

#[test]
fn undo_of_a_row_insert_removes_the_row_again() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    room.submit(
        "alice",
        Role::Editor,
        op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "insert_row", "index": 1, "row": json!(Table::new(1, 2, 200.0).rows[0])})),
    )
    .expect("insert");

    let step = room.undo("alice").expect("undo step");
    assert_eq!(step.label, "Insert row");
    let mut undo_op = step.inverse.first().expect("inverse").clone();
    undo_op.base_version = room.version();
    room.submit("alice", Role::Editor, undo_op).expect("undo lands");

    assert_eq!(table_rows(room.model().document()), 2);
    assert_eq!(cell_text(room.model().document(), 1, 0), "r1c0");
}

#[test]
fn a_replica_applies_a_row_insert_and_a_row_removal() {
    let before = table_doc(2, 2);
    let mut recent = before.clone();
    recent.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").insert_row(1, 0);
    let mut replica = before.clone();
    let mut ids = NodeIds::new();
    DocModel::new(before).index_nodes(&mut ids);

    let row = serde_json::to_value(&recent.table(StoryRef::Body, &Path(vec![TABLE])).expect("table").rows[1]).expect("row json");
    let table = table_id(&ids);
    crate::apply_delta(&mut replica, &mut ids, &json!({"op": "insert_row", "nodeId": table, "index": 1, "row": row}))
        .expect("the replica inserts the row");
    assert_same(&replica, &recent);

    crate::apply_delta(&mut replica, &mut ids, &json!({"op": "remove_row", "nodeId": table, "index": 1})).expect("the replica removes it");
    assert_same(&replica, &table_doc(2, 2));
}
