//! Adapter tests: two clients through one room, undo through inverse operations.

use serde_json::json;
use std::sync::Arc;
use wordcraft_collab::ids::{NodeIds, NodeKey};
use wordcraft_collab::room::{Model, Role, Room};
use wordcraft_collab::wire::{HistoryAction, Op, OpId, OpKind, Target};
use wordcraft_doc::{Block, Cell, CharProps, Document, Paragraph, Path, Pos, StoryRef, Table};

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
fn inserting_a_column_diffs_to_insert_column() {
    let before = table_doc(2, 2);
    let mut after = before.clone();
    after.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").insert_col(1);
    let applied = diff_and_apply(before, after.clone());
    assert_same(&applied, &after);
}

#[test]
fn removing_a_column_diffs_to_remove_column() {
    let before = table_doc(2, 3);
    let mut after = before.clone();
    after.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").delete_col(1);
    let applied = diff_and_apply(before, after.clone());
    assert_same(&applied, &after);
}

#[test]
fn merging_cells_diffs_to_merge_cells() {
    let before = table_doc(2, 3);
    let mut after = before.clone();
    after.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").merge(0, 1, 0, 1);
    let applied = diff_and_apply(before, after.clone());
    assert_same(&applied, &after);
}

#[test]
fn splitting_a_cell_diffs_to_split_cells() {
    let before = table_doc(2, 2);
    let mut after = before.clone();
    after.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").split_cell(1, 1, 2);
    let applied = diff_and_apply(before, after.clone());
    assert_same(&applied, &after);
}

// --- columns, merges and splits on two replicas ----------------------------

/// A second replica of `doc`: the same document and a copy of the same node map,
/// which is what a client that already received the map holds.
fn replica_of(doc: &Document) -> (Document, NodeIds) {
    let mut ids = NodeIds::new();
    DocModel::new(doc.clone()).index_nodes(&mut ids);
    (doc.clone(), ids)
}

fn resolved(ids: &NodeIds, id: u32) -> Vec<u32> {
    ids.resolve(wordcraft_collab::NodeId(id)).expect("the id is live").path.clone()
}

fn cell_paragraphs(doc: &Document, row: u32, cell: u32) -> Vec<String> {
    doc.table(StoryRef::Body, &Path(vec![TABLE]))
        .and_then(|table| table.rows.get(row as usize))
        .and_then(|row| row.cells.get(cell as usize))
        .map(|cell| cell.blocks.iter().filter_map(|block| block.as_para().map(|para| para.text.clone())).collect())
        .unwrap_or_default()
}

fn table_cells(doc: &Document) -> Vec<usize> {
    doc.table(StoryRef::Body, &Path(vec![TABLE])).map_or_else(Vec::new, |table| table.rows.iter().map(|row| row.cells.len()).collect())
}

#[test]
fn a_column_insert_matches_the_documents_own_insert_col() {
    // A merged cell makes row 1 widen while rows 0 and 2 gain a cell: the plan
    // the adapter reindexes from must be the one `Table::insert_col` follows.
    let mut doc = table_doc(3, 3);
    doc.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").merge(1, 1, 0, 1);
    let mut expected = doc.clone();
    expected.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table").insert_col(1);

    let mut room = Room::new(DocModel::new(doc));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "insert_column", "index": 1})))
        .expect("the column is inserted");

    assert_eq!(
        serde_json::to_value(room.model().document().table(StoryRef::Body, &Path(vec![TABLE]))).ok(),
        serde_json::to_value(expected.table(StoryRef::Body, &Path(vec![TABLE]))).ok()
    );
}

#[test]
fn a_column_insert_moves_the_cell_ids_and_the_caret_holder_keeps_his() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let first_cell = cell_id(room.ids(), 0, 0);
    let bob_cell = cell_para_id(room.ids(), 1, 1);

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "insert_column", "index": 1})))
        .expect("the column is inserted");

    assert_eq!(table_cells(room.model().document()), vec![3, 3]);
    // Bob's paragraph moved with his cell: cell 1 became cell 2.
    assert_eq!(resolved(room.ids(), bob_cell), vec![TABLE, 1, 2, 0]);
    // The cell before the new column kept its id and its text.
    assert_eq!(resolved(room.ids(), first_cell), vec![TABLE, 0, 0]);
    assert_eq!(cell_text(room.model().document(), 0, 0), "r0c0");

    // An edit planned before the insert still lands in Bob's cell.
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, bob_cell, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("the stale address still names Bob's cell");
    assert_eq!(cell_text(room.model().document(), 1, 2), "r1c1!");
    assert_eq!(cell_text(room.model().document(), 1, 1), "", "the new column's cell is empty");
}

#[test]
fn two_replicas_insert_a_column_and_converge() {
    let before = table_doc(2, 2);
    let (mut replica, mut ids) = replica_of(&before);
    let mut room = Room::new(DocModel::new(before));
    let _ = room.join(None, false);
    let table = table_id(room.ids());

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "insert_column", "index": 1})))
        .expect("the column is inserted");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica inserts the column");

    assert_same(&replica, room.model().document());
    // Both replicas now address the moved cell the same way.
    let key = NodeKey::new("body", vec![TABLE, 1, 2, 0]);
    assert_eq!(ids.get(&key).map(|id| id.0), room.ids().get(&key).map(|id| id.0));
}

#[test]
fn removing_a_column_keeps_the_ids_of_the_cells_it_does_not_touch() {
    let mut room = Room::new(DocModel::new(table_doc(2, 3)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let left = cell_para_id(room.ids(), 0, 0);
    let right = cell_para_id(room.ids(), 1, 2);

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "remove_column", "index": 1})))
        .expect("the column is removed");

    assert_eq!(table_cells(room.model().document()), vec![2, 2]);
    assert_eq!(resolved(room.ids(), left), vec![TABLE, 0, 0, 0], "a cell left of the column did not move");
    assert_eq!(resolved(room.ids(), right), vec![TABLE, 1, 1, 0], "a cell right of it moved one place left");
    assert_eq!(cell_text(room.model().document(), 1, 1), "r1c2");
}

#[test]
fn merging_two_cells_keeps_the_first_cells_id_and_a_split_brings_the_cells_back() {
    let before = table_doc(2, 2);
    let (mut replica, mut ids) = replica_of(&before);
    let mut room = Room::new(DocModel::new(before));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let merged = cell_id(room.ids(), 0, 0);
    let paragraph = cell_para_id(room.ids(), 0, 0);
    let neighbour = cell_para_id(room.ids(), 0, 1);

    let applied = room
        .submit("bob", Role::Editor, op("b", 1, 0, OpKind::Structural, table, 0, json!({"op": "merge_cells", "rows": [0, 0], "cols": [0, 1]})))
        .expect("the cells merge");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica merges");
    assert_same(&replica, room.model().document());

    // The cell that absorbed the range kept its id, its paragraph id and the
    // caret in it; the absorbed cell is gone rather than re-pointed.
    assert_eq!(resolved(room.ids(), merged), vec![TABLE, 0, 0]);
    assert_eq!(resolved(room.ids(), paragraph), vec![TABLE, 0, 0, 0]);
    // The absorbed content arrived as a second paragraph of the surviving cell.
    assert_eq!(cell_paragraphs(room.model().document(), 0, 0), vec!["r0c0", "r0c1"]);
    assert!(room.ids().is_tombstoned(wordcraft_collab::NodeId(neighbour)));
    let err = room
        .submit("alice", Role::Editor, op("a", 1, 1, OpKind::Primitive, neighbour, 0, json!({"op": "insert_text", "text": "!"})))
        .expect_err("the absorbed cell is gone");
    assert_eq!(err.code, wordcraft_collab::RejectCode::Gone);

    // Splitting puts two cells back; the first keeps the id and the content.
    let applied = room
        .submit("bob", Role::Editor, op("b", 2, 1, OpKind::Structural, table, 0, json!({"op": "split_cells", "row": 0, "cell": 0, "columns": 2})))
        .expect("the merged cell splits");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica splits");
    assert_same(&replica, room.model().document());

    assert_eq!(table_cells(room.model().document()), vec![2, 2]);
    assert_eq!(resolved(room.ids(), merged), vec![TABLE, 0, 0]);
    assert_eq!(cell_paragraphs(room.model().document(), 0, 0), vec!["r0c0", "r0c1"]);
    assert_eq!(cell_text(room.model().document(), 0, 1), "", "the new part is empty");
    // The caret holder can still type where it was.
    room.submit("bob", Role::Editor, op("b", 3, 2, OpKind::Primitive, paragraph, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("the caret holder still edits its paragraph");
    assert_eq!(cell_paragraphs(room.model().document(), 0, 0), vec!["r0c0!", "r0c1"]);
}

#[test]
fn a_merge_range_that_is_not_addressable_is_refused_before_the_document_changes() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let before = serde_json::to_value(room.model().document()).expect("json");

    for payload in [
        json!({"op": "merge_cells", "rows": [0, 1], "cols": [0, 5]}),
        json!({"op": "merge_cells", "rows": [1, 0], "cols": [0, 1]}),
        json!({"op": "merge_cells", "rows": [0, 5], "cols": [0, 1]}),
        json!({"op": "merge_cells", "rows": [0, 0], "cols": [0, 0]}),
        json!({"op": "split_cells", "row": 0, "cell": 0, "columns": 1}),
    ] {
        let err = room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, payload.clone())).expect_err("the range is refused");
        assert_eq!(err.code, wordcraft_collab::RejectCode::Invalid, "{payload}");
        assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "{payload} changed the document");
    }
    assert_eq!(room.version(), 0, "a refused operation does not advance the room");
}

#[test]
fn removing_a_column_through_a_replica_matches_the_room() {
    let before = table_doc(2, 3);
    let (mut replica, mut ids) = replica_of(&before);
    let mut room = Room::new(DocModel::new(before));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "remove_column", "index": 0})))
        .expect("the column is removed");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica removes the column");
    assert_same(&replica, room.model().document());
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
fn undo_of_a_column_insert_removes_the_column_again() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "insert_column", "index": 1}))).expect("insert");

    let step = room.undo("alice").expect("undo step");
    assert_eq!(step.label, "Insert column");
    let mut undo_op = step.inverse.first().expect("inverse").clone();
    undo_op.base_version = room.version();
    room.submit("alice", Role::Editor, undo_op).expect("undo lands");

    assert_eq!(table_cells(room.model().document()), vec![2, 2]);
    assert_same(room.model().document(), &table_doc(2, 2));
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

// --- undo of the structural table edits ------------------------------------

/// An undo request for `client`'s step, as the app sends it: the step is named
/// by the id of the operation that created it.
fn undo_request(client: &str, seq: u64, base: u64, step_client: &str, step_seq: u64) -> Op {
    Op {
        op_id: OpId::new(client, seq),
        base_version: base,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", 0, 0),
        payload: json!({}),
        history: Some(HistoryAction::Undo { op_id: OpId::new(step_client, step_seq) }),
    }
}

fn grid_len(doc: &Document) -> usize {
    doc.table(StoryRef::Body, &Path(vec![TABLE])).map_or(0, |table| table.grid.len())
}

#[test]
fn undo_of_a_merge_splits_the_cells_back_with_their_own_text_and_ids() {
    let before = table_doc(2, 2);
    let (mut replica, mut ids) = replica_of(&before);
    let mut room = Room::new(DocModel::new(before.clone()));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let absorbed = cell_id(room.ids(), 0, 1);
    let absorbed_para = cell_para_id(room.ids(), 0, 1);

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "merge_cells", "rows": [0, 1], "cols": [0, 1]})))
        .expect("the cells merge");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica merges");
    assert_same(&replica, room.model().document());
    assert_eq!(cell_paragraphs(room.model().document(), 0, 0), vec!["r0c0", "r0c1", "r1c0", "r1c1"]);

    // The inverse is a split that carries the cells the merge took away.
    let applied = room.submit("alice", Role::Editor, undo_request("a", 2, 1, "a", 1)).expect("the undo lands");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica splits back");

    assert_same(room.model().document(), &before);
    assert_same(&replica, &before);
    // Every cell came back with its own text, not the merged run.
    assert_eq!(cell_text(room.model().document(), 0, 1), "r0c1");
    assert_eq!(cell_text(room.model().document(), 1, 0), "r1c0");
    // The absorbed cell and its paragraph are addressable again under their ids.
    assert_eq!(resolved(room.ids(), absorbed), vec![TABLE, 0, 1]);
    assert_eq!(resolved(room.ids(), absorbed_para), vec![TABLE, 0, 1, 0]);
    assert_eq!(resolved(&ids, absorbed), vec![TABLE, 0, 1], "the replica put the same id back");
    // The caret holder keeps his cell and types where he was.
    room.submit("alice", Role::Editor, op("a", 3, 2, OpKind::Primitive, absorbed_para, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("the caret holder still edits his own cell");
    assert_eq!(cell_text(room.model().document(), 0, 1), "r0c1!");
}

#[test]
fn undo_of_a_split_merges_the_parts_back_and_restores_the_grid() {
    let before = table_doc(2, 2);
    let (mut replica, mut ids) = replica_of(&before);
    let mut room = Room::new(DocModel::new(before.clone()));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let cell = cell_id(room.ids(), 0, 0);
    let para = cell_para_id(room.ids(), 0, 0);

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "split_cells", "row": 0, "cell": 0, "columns": 2})))
        .expect("the cell splits");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica splits");
    assert_same(&replica, room.model().document());
    assert_eq!(table_cells(room.model().document()), vec![3, 2]);
    assert_eq!(grid_len(room.model().document()), 3, "the split refined the grid inside the cell");

    let applied = room.submit("alice", Role::Editor, undo_request("a", 2, 1, "a", 1)).expect("the undo lands");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica merges back");

    assert_same(room.model().document(), &before);
    assert_same(&replica, &before);
    assert_eq!(grid_len(room.model().document()), 2, "the grid line the split added is gone");
    assert_eq!(resolved(room.ids(), cell), vec![TABLE, 0, 0]);
    room.submit("alice", Role::Editor, op("a", 3, 2, OpKind::Primitive, para, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("the caret holder still edits his cell");
    assert_eq!(cell_text(room.model().document(), 0, 0), "r0c0!");
}

#[test]
fn undo_of_a_column_removal_puts_the_column_back_with_its_text_and_ids() {
    let before = table_doc(2, 3);
    let (mut replica, mut ids) = replica_of(&before);
    let mut room = Room::new(DocModel::new(before.clone()));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let gone_cell = cell_id(room.ids(), 1, 1);
    let gone_para = cell_para_id(room.ids(), 1, 1);

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "remove_column", "index": 1})))
        .expect("the column is removed");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica removes the column");
    assert_same(&replica, room.model().document());
    assert_eq!(table_cells(room.model().document()), vec![2, 2]);
    assert_eq!(cell_text(room.model().document(), 1, 1), "r1c2");

    let applied = room.submit("alice", Role::Editor, undo_request("a", 2, 1, "a", 1)).expect("the undo lands");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica puts the column back");

    assert_same(room.model().document(), &before);
    assert_same(&replica, &before);
    assert_eq!(cell_text(room.model().document(), 1, 1), "r1c1", "the removed cell's text is back");
    // The cells the removal took away are addressable again.
    assert_eq!(resolved(room.ids(), gone_cell), vec![TABLE, 1, 1]);
    assert_eq!(resolved(room.ids(), gone_para), vec![TABLE, 1, 1, 0]);
    assert_eq!(resolved(&ids, gone_para), vec![TABLE, 1, 1, 0], "the replica put the same id back");
    // The caret holder in the removed column keeps his cell.
    room.submit("alice", Role::Editor, op("a", 3, 2, OpKind::Primitive, gone_para, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("the caret holder still edits the cell that came back");
    assert_eq!(cell_text(room.model().document(), 1, 1), "r1c1!");
}

#[test]
fn undo_of_a_column_removal_restores_a_row_that_disappeared_with_the_column() {
    // Row 1 holds a single one-column cell, so removing column 0 drops the whole
    // row; the inverse has to put the row back, not just the cell.
    let mut before = table_doc(2, 2);
    {
        let table = before.table_mut(StoryRef::Body, &Path(vec![TABLE])).expect("table");
        table.rows[1].cells.truncate(1);
        table.rows[1].props.grid_after = 1;
    }
    let mut room = Room::new(DocModel::new(before.clone()));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let row = row_id(room.ids(), 1);

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "remove_column", "index": 0})))
        .expect("the column is removed");
    assert_eq!(table_rows(room.model().document()), 1, "the row went with its only cell");

    room.submit("alice", Role::Editor, undo_request("a", 2, 1, "a", 1)).expect("the undo lands");
    assert_same(room.model().document(), &before);
    assert_eq!(resolved(room.ids(), row), vec![TABLE, 1], "the row's id names the row again");
}

#[test]
fn undo_of_a_merge_after_another_users_edit_in_the_cell_is_a_conflict() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "merge_cells", "rows": [0, 0], "cols": [0, 1]})))
        .expect("the cells merge");

    // Bob types into the cell the merge kept; undoing the merge would replace
    // that body with the captured one and lose his text.
    let merged_para = cell_para_id(room.ids(), 0, 0);
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, merged_para, 4, json!({"op": "insert_text", "text": "!"}))).expect("bob types");
    let before = serde_json::to_value(room.model().document()).expect("json");

    let err = room.submit("alice", Role::Editor, undo_request("a", 2, 2, "a", 1)).expect_err("the undo is refused");
    assert_eq!(err.code, wordcraft_collab::RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the refused undo changed nothing");
    assert_eq!(room.version(), 2, "a refused undo does not advance the room");
    assert_eq!(room.undo_depth("alice"), (1, 0), "the step stays on the stack");
    assert_eq!(cell_paragraphs(room.model().document(), 0, 0), vec!["r0c0!", "r0c1"], "bob's text survived");
}

#[test]
fn undo_of_a_merge_lands_after_an_edit_in_another_cell() {
    // The touched set is the merged region, not the whole table: an edit in a
    // cell the merge did not take must not block the undo.
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "merge_cells", "rows": [0, 0], "cols": [0, 1]})))
        .expect("the cells merge");
    let other = cell_para_id(room.ids(), 1, 1);
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, other, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("bob types in another row");

    room.submit("alice", Role::Editor, undo_request("a", 2, 2, "a", 1)).expect("the undo lands");
    assert_eq!(table_cells(room.model().document()), vec![2, 2]);
    assert_eq!(cell_text(room.model().document(), 0, 1), "r0c1");
    assert_eq!(cell_text(room.model().document(), 1, 1), "r1c1!", "bob's edit is still there");
}

#[test]
fn undo_of_a_row_insert_after_an_edit_in_the_new_row_is_a_conflict() {
    let mut room = Room::new(DocModel::new(table_doc(2, 2)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    room.submit(
        "alice",
        Role::Editor,
        op("a", 1, 0, OpKind::Structural, table, 0, json!({"op": "insert_row", "index": 1, "row": json!(Table::new(1, 2, 200.0).rows[0])})),
    )
    .expect("the row is inserted");

    // Bob types into the row Alice inserted; undoing the insert would take the
    // row — and his text — away.
    let new_cell = cell_para_id(room.ids(), 1, 0);
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, new_cell, 0, json!({"op": "insert_text", "text": "mine"}))).expect("bob types");
    let before = serde_json::to_value(room.model().document()).expect("json");

    let err = room.submit("alice", Role::Editor, undo_request("a", 2, 2, "a", 1)).expect_err("the undo is refused");
    assert_eq!(err.code, wordcraft_collab::RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the refused undo changed nothing");
    assert_eq!(cell_text(room.model().document(), 1, 0), "mine");
}

#[test]
fn a_format_restore_rewrites_the_runs_and_properties_it_names() {
    let mut doc = Document::new();
    doc.body.clear();
    doc.body.push(Arc::new(Block::Para(Paragraph::with_text("Hello", CharProps::default()))));
    let mut room = Room::new(DocModel::new(doc));
    let first = node(room.ids_mut(), 0);
    let before = serde_json::to_value(room.model().document()).expect("json");

    room.submit(
        "alice",
        Role::Editor,
        op(
            "a",
            1,
            0,
            OpKind::Primitive,
            first,
            0,
            json!({
                "op": "set_para_format",
                "runs": [{"len": 2, "props": {"bold": true}}, {"len": 3, "props": {}}],
                "mark": {"bold": true},
                "props": {"align": "center"},
            }),
        ),
    )
    .expect("the restore lands");
    let para = || room.model().document().para(StoryRef::Body, &Path::top(0)).cloned().expect("a paragraph");
    assert_eq!(para().runs.first().map(|run| run.props.bold), Some(Some(true)));
    assert_eq!(para().props.align, Some(wordcraft_doc::Align::Center));

    // The adapter keeps the previous runs and properties as its own inverse.
    let step = room.undo("alice").expect("an undo step");
    assert_eq!(step.label, "Formatting");
    let mut inverse = step.inverse.first().expect("an inverse operation").clone();
    inverse.base_version = room.version();
    room.submit("alice", Role::Editor, inverse).expect("the inverse lands");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the paragraph is back");
}

#[test]
fn a_format_restore_on_an_unknown_node_is_refused_before_mutation() {
    let mut room = room();
    let before = serde_json::to_value(room.model().document()).expect("json");
    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, 999, 0, json!({"op": "set_para_format", "props": {"align": "center"}})))
        .expect_err("an id the room never issued is refused");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 0);
}
