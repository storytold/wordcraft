//! Adapter tests: two clients through one room, undo through inverse operations.

use serde_json::json;
use std::sync::Arc;
use wordcraft_collab::ids::{NodeId, NodeIds, NodeKey};
use wordcraft_collab::room::{Model, Role, Room};
use wordcraft_collab::wire::{HistoryAction, Op, OpId, OpKind, RejectCode, Target};
use wordcraft_doc::{Block, Cell, CharProps, Document, InlineObject, Paragraph, Path, Pos, StoryRef, Table};

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
fn a_body_restore_rewrites_the_text_and_objects_it_names() {
    let mut doc = Document::new();
    doc.body.clear();
    doc.body.push(Arc::new(Block::Para(Paragraph::with_text("x", CharProps::default()))));
    let mut room = Room::new(DocModel::new(doc));
    let first = node(room.ids_mut(), 0);
    let before = serde_json::to_value(room.model().document()).expect("json");
    let field = json!(InlineObject::Field {
        instr: "EQ \\o\\ac(○,x)".into(),
        result: "x".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    });

    // The inverse of `format.enclose`: the character is now the object's
    // placeholder, so the restore names the body and the objects with it.
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
                "text": "\u{FFFC}",
                "objects": [field],
                "expect": "x",
                "runs": [{"len": 3}],
            }),
        ),
    )
    .expect("the body restore lands");
    let para = || room.model().document().para(StoryRef::Body, &Path::top(0)).cloned().expect("a paragraph");
    assert_eq!(para().text, "\u{FFFC}");
    assert_eq!(para().objects.len(), 1, "the enclosed-character field is there");

    // The adapter keeps the previous body and objects as its own inverse.
    let step = room.undo("alice").expect("an undo step");
    let mut inverse = step.inverse.first().expect("an inverse operation").clone();
    inverse.base_version = room.version();
    room.submit("alice", Role::Editor, inverse).expect("the inverse lands");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the text and the objects are back");
}

#[test]
fn a_body_restore_with_a_stale_expect_is_refused_before_mutation() {
    let mut doc = Document::new();
    doc.body.clear();
    doc.body.push(Arc::new(Block::Para(Paragraph::with_text("x", CharProps::default()))));
    let mut room = Room::new(DocModel::new(doc));
    let first = node(room.ids_mut(), 0);
    let before = serde_json::to_value(room.model().document()).expect("json");
    let field = json!(InlineObject::Field {
        instr: "EQ \\o\\ac(○,x)".into(),
        result: "x".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    });

    for payload in [
        // The paragraph no longer holds the text the restore was prepared from.
        json!({"op": "set_para_format", "text": "y", "objects": [], "expect": "stale", "runs": [{"len": 1}]}),
        // A body restore must name what it expects and the objects that go with it.
        json!({"op": "set_para_format", "text": "y", "objects": [], "runs": [{"len": 1}]}),
        json!({"op": "set_para_format", "text": "y", "expect": "x", "runs": [{"len": 1}]}),
        // Objects that do not match the text's object characters.
        json!({"op": "set_para_format", "text": "x", "objects": [field], "expect": "x", "runs": [{"len": 1}]}),
    ] {
        let err =
            room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, first, 0, payload.clone())).expect_err("the body restore is refused");
        assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Invalid, "{payload}");
        assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "nothing changed");
        assert_eq!(room.version(), 0, "the room did not advance");
    }
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

// --- moving blocks past their neighbour ------------------------------------

fn move_doc(texts: &[&str]) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    for text in texts {
        doc.body.push(Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
    }
    doc
}

fn move_room(texts: &[&str]) -> Room<DocModel> {
    let mut room = Room::new(DocModel::new(move_doc(texts)));
    let _ = room.join(None, false);
    room
}

fn block_id(room: &Room<DocModel>, index: u32) -> u32 {
    room.ids().get(&NodeKey::new("body", vec![index])).expect("a block id").0
}

fn body_texts(room: &Room<DocModel>) -> Vec<String> {
    (0..paragraphs(room)).map(|index| body_text(room, index)).collect()
}

#[test]
fn moving_a_paragraph_past_its_neighbour_is_one_structural_operation() {
    let mut room = move_room(&["one", "two", "three"]);
    let (first, second, third) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, third, 0, json!({"op": "move_block", "neighbour": second})))
        .expect("the move lands");

    assert_eq!(body_texts(&room), ["one", "three", "two"]);
    assert_eq!(
        applied.delta,
        Some(json!({"op": "move_block", "nodeId": third, "count": 1, "neighbour": second})),
        "the delta names both blocks by id"
    );
    // The ids keep naming their blocks, not their old positions.
    assert_eq!(room.ids().resolve(NodeId(first)).map(|key| key.path.clone()), Ok(vec![0]));
    assert_eq!(room.ids().resolve(NodeId(third)).map(|key| key.path.clone()), Ok(vec![1]));
    assert_eq!(room.ids().resolve(NodeId(second)).map(|key| key.path.clone()), Ok(vec![2]));
}

#[test]
fn undo_of_a_two_block_move_lands_as_one_transaction_and_redo_returns_the_order() {
    let mut room = move_room(&["one", "two", "three", "four"]);
    let ids: Vec<u32> = (0..4).map(|index| block_id(&room, index)).collect();

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, ids[1], 0, json!({"op": "move_block", "count": 2, "neighbour": ids[0]})))
        .expect("the move lands");
    assert_eq!(body_texts(&room), ["two", "three", "one", "four"]);

    let undone = room.submit("alice", Role::Editor, undo_request("alice", 2, 1, "a", 1)).expect("the undo lands");
    let delta = undone.delta.clone().expect("the undo carries a delta");
    let ops = delta.get("ops").and_then(serde_json::Value::as_array).expect("one transaction");
    assert_eq!(ops.len(), 2, "the neighbour crosses the run one block at a time");
    for one in ops {
        assert_eq!(one["op"], json!("move_block"));
        assert_eq!(one["count"], json!(1));
    }
    assert_eq!(body_texts(&room), ["one", "two", "three", "four"], "the undo restores the order");

    let redo = Op {
        op_id: OpId::new("alice", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Structural,
        target: Target::new("body", ids[1], 0),
        payload: serde_json::Value::Null,
        history: Some(HistoryAction::Redo { op_id: OpId::new("a", 1) }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(body_texts(&room), ["two", "three", "one", "four"], "redo moves the run again");
}

#[test]
fn a_move_with_an_unknown_or_retired_id_is_refused_before_mutation() {
    let mut room = move_room(&["one", "two", "three"]);
    let (_first, second, third) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));
    let before = serde_json::to_value(room.model().document()).expect("json");

    // An id the session never issued.
    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, third, 0, json!({"op": "move_block", "neighbour": 999_999})))
        .expect_err("an unknown neighbour is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "nothing changed");
    assert_eq!(room.version(), 0, "the version did not move");

    // A neighbour two places above the run is not next to it.
    let err = room
        .submit("alice", Role::Editor, op("a", 2, 0, OpKind::Structural, third, 0, json!({"op": "move_block", "neighbour": block_id(&room, 0)})))
        .expect_err("a non-adjacent neighbour is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);

    // A run that leaves the container.
    let err = room
        .submit("alice", Role::Editor, op("a", 3, 0, OpKind::Structural, third, 0, json!({"op": "move_block", "count": 2, "neighbour": second})))
        .expect_err("a run past the last block is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(paragraphs(&room), 3);

    // A retired id is gone rather than invalid; the document still does not move.
    room.submit("alice", Role::Editor, op("a", 4, 0, OpKind::Structural, third, 0, json!({"op": "remove_block"}))).expect("remove the last block");
    let before_retired = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();
    let err = room
        .submit("alice", Role::Editor, op("a", 5, version, OpKind::Structural, second, 0, json!({"op": "move_block", "neighbour": third})))
        .expect_err("a retired neighbour is refused");
    assert_eq!(err.code, RejectCode::Gone);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before_retired);
    assert_eq!(room.version(), version);
}

#[test]
fn a_foreign_edit_of_a_moved_block_makes_undo_a_conflict() {
    let mut room = move_room(&["one", "two", "three"]);
    let (_first, second, third) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, third, 0, json!({"op": "move_block", "neighbour": second})))
        .expect("the move lands");

    // Bob types into the block the move displaced (it is not the step's target).
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, second, 0, json!({"op": "insert_text", "text": "X"}))).expect("bob types");
    let before = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();

    let err = room.submit("alice", Role::Editor, undo_request("alice", 2, version, "a", 1)).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the refusal changed nothing");
    assert_eq!(room.version(), version, "the version did not move");
    assert_eq!(room.undo_depth("alice"), (1, 0), "the step stays on the stack");
}

#[test]
fn moving_a_paragraph_diffs_to_move_block() {
    let room = move_room(&["one", "two", "three"]);
    let ids = room.ids().clone();
    let before = room.model().document().clone();
    let mut after = before.clone();
    after.body.swap(1, 2);

    let changes = crate::diff(&before, &after, &ids).expect("the diff names the move");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, OpKind::Structural);
    assert_eq!(changes[0].target.node_id, block_id(&room, 2), "the block that moved up is the target");
    assert_eq!(changes[0].payload, json!({"op": "move_block", "count": 1, "neighbour": block_id(&room, 1)}));
}

#[test]
fn moving_two_paragraphs_diffs_to_one_move_with_the_same_addresses() {
    let room = move_room(&["one", "two", "three", "four"]);
    let ids = room.ids().clone();
    let before = room.model().document().clone();
    let mut after = before.clone();
    let blocks = after.body.clone();
    after.body = vec![blocks[1].clone(), blocks[2].clone(), blocks[0].clone(), blocks[3].clone()];

    let changes = crate::diff(&before, &after, &ids).expect("the diff names the move");
    assert_eq!(changes.len(), 1, "one run, one operation");
    assert_eq!(changes[0].target.node_id, block_id(&room, 1), "the run starts at the block that moved");
    assert_eq!(changes[0].payload, json!({"op": "move_block", "count": 2, "neighbour": block_id(&room, 0)}));
}

#[test]
fn a_reorder_wider_than_the_bound_is_refused_instead_of_becoming_text_edits() {
    let texts: Vec<String> = (0..70).map(|index| format!("p{index}")).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let room = move_room(&refs);
    let ids = room.ids().clone();
    let before = room.model().document().clone();
    let mut after = before.clone();
    let blocks = after.body.clone();
    let mut order: Vec<Arc<Block>> = blocks.iter().skip(1).take(66).cloned().collect();
    order.push(blocks[0].clone());
    order.extend(blocks.iter().skip(67).cloned());
    after.body = order;

    let err = crate::diff(&before, &after, &ids).expect_err("66 blocks are wider than a move");
    assert!(matches!(err, crate::DiffError::Unsupported(_)), "{err:?}");
}

#[test]
fn a_replica_applies_a_move_and_keeps_the_ids_of_the_blocks_it_moved() {
    let mut room = move_room(&["one", "two", "three"]);
    let (_first, second, third) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));
    let mut replica = room.model().document().clone();
    let mut replica_ids = room.ids().clone();

    submit_and_mirror(
        &mut room,
        op("a", 1, 0, OpKind::Structural, third, 0, json!({"op": "move_block", "neighbour": second})),
        &mut replica,
        &mut replica_ids,
    );
    assert_eq!(room.ids().resolve(NodeId(third)).map(|key| key.path.clone()), Ok(vec![1]));
    assert_eq!(replica_ids.resolve(NodeId(third)).map(|key| key.path.clone()), Ok(vec![1]), "the replica follows the block");
    assert_eq!(replica_ids.resolve(NodeId(second)).map(|key| key.path.clone()), Ok(vec![2]));
}

// --- sorting a run of paragraphs by an explicit id order --------------------

fn permute_payload(order: &[u32]) -> serde_json::Value {
    json!({"op": "permute", "nodeId": order.first().copied().unwrap_or(0), "order": order})
}

#[test]
fn sorting_blocks_is_one_transactional_permute() {
    let mut room = move_room(&["gamma", "alpha", "beta"]);
    let (gamma, alpha, beta) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, alpha, 0, permute_payload(&[alpha, beta, gamma])))
        .expect("the sort lands");

    assert_eq!(body_texts(&room), ["alpha", "beta", "gamma"]);
    assert_eq!(
        applied.delta,
        Some(json!({"op": "transaction", "ops": [{"op": "permute", "nodeId": alpha, "order": [alpha, beta, gamma]}]})),
        "the sort lands as one permute inside the step's transaction"
    );
    // The ids keep naming their blocks, not their old positions.
    assert_eq!(room.ids().resolve(NodeId(alpha)).map(|key| key.path.clone()), Ok(vec![0]));
    assert_eq!(room.ids().resolve(NodeId(beta)).map(|key| key.path.clone()), Ok(vec![1]));
    assert_eq!(room.ids().resolve(NodeId(gamma)).map(|key| key.path.clone()), Ok(vec![2]));
}

#[test]
fn undo_of_a_sort_lands_as_one_transaction_and_redo_returns_the_order() {
    let mut room = move_room(&["gamma", "alpha", "beta"]);
    let (gamma, alpha, beta) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, alpha, 0, permute_payload(&[alpha, beta, gamma]))).expect("the sort lands");

    let undone = room.submit("alice", Role::Editor, undo_request("alice", 2, 1, "a", 1)).expect("the undo lands");
    assert_eq!(
        undone.delta,
        Some(json!({"op": "transaction", "ops": [{"op": "permute", "nodeId": gamma, "order": [gamma, alpha, beta]}]})),
        "the inverse is one permute with the old order"
    );
    assert_eq!(body_texts(&room), ["gamma", "alpha", "beta"], "the undo restores the order");

    let redo = Op {
        op_id: OpId::new("alice", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Structural,
        target: Target::new("body", alpha, 0),
        payload: serde_json::Value::Null,
        history: Some(HistoryAction::Redo { op_id: OpId::new("a", 1) }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(body_texts(&room), ["alpha", "beta", "gamma"], "redo sorts again");
}

#[test]
fn a_permute_with_an_unknown_or_retired_id_is_refused_before_mutation() {
    let mut room = move_room(&["one", "two", "three"]);
    let (first, _second, third) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));
    let before = serde_json::to_value(room.model().document()).expect("json");

    // An id the session never issued.
    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, first, 0, permute_payload(&[first, 999_999])))
        .expect_err("an unknown id is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "nothing changed");
    assert_eq!(room.version(), 0, "the version did not move");

    // A list that skips a block names no contiguous run.
    let err = room
        .submit("alice", Role::Editor, op("a", 2, 0, OpKind::Structural, first, 0, permute_payload(&[third, first])))
        .expect_err("a gap is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);

    // The same id twice.
    let err = room
        .submit("alice", Role::Editor, op("a", 3, 0, OpKind::Structural, first, 0, permute_payload(&[first, first])))
        .expect_err("a duplicate is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);

    // A block from another container.
    let mut other = table_doc(1, 1);
    other.body.insert(0, Arc::new(Block::Para(Paragraph::with_text("gap", CharProps::default()))));
    let mut room_other = Room::new(DocModel::new(other));
    let _ = room_other.join(None, false);
    let cell_para = room_other.ids().iter().find(|(_, key)| key.path.len() == 4).map(|(id, _)| id.0).expect("a cell paragraph id");
    let outer = block_id(&room_other, 0);
    let before_other = serde_json::to_value(room_other.model().document()).expect("json");
    let err = room_other
        .submit("alice", Role::Editor, op("a", 4, 0, OpKind::Structural, outer, 0, permute_payload(&[outer, cell_para])))
        .expect_err("blocks in different containers are refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room_other.model().document()).expect("json"), before_other);

    // A retired id is gone rather than invalid; the document still does not move.
    room.submit("alice", Role::Editor, op("a", 5, 0, OpKind::Structural, third, 0, json!({"op": "remove_block"}))).expect("remove the last block");
    let before_retired = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();
    let err = room
        .submit("alice", Role::Editor, op("a", 6, version, OpKind::Structural, first, 0, permute_payload(&[third, first])))
        .expect_err("a retired id is refused");
    assert_eq!(err.code, RejectCode::Gone);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before_retired);
    assert_eq!(room.version(), version);
}

#[test]
fn a_permute_with_a_table_in_the_run_is_refused_before_mutation() {
    let mut room = Room::new(DocModel::new(table_doc(1, 1)));
    let _ = room.join(None, false);
    let paragraph = block_id(&room, 0);
    let table = table_id(room.ids());
    let before = serde_json::to_value(room.model().document()).expect("json");

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, paragraph, 0, permute_payload(&[table, paragraph])))
        .expect_err("a table cannot be reordered as a paragraph");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 0);
    assert_eq!(paragraphs(&room), 2, "the table is still there");
}

#[test]
fn a_foreign_edit_of_a_sorted_block_makes_undo_a_conflict() {
    let mut room = move_room(&["gamma", "alpha", "beta"]);
    let (gamma, alpha, beta) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, alpha, 0, permute_payload(&[alpha, beta, gamma]))).expect("the sort lands");

    // Bob types into the block the sort moved to the front (not the step's address).
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, alpha, 0, json!({"op": "insert_text", "text": "X"}))).expect("bob types");
    let before = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();

    let err = room.submit("alice", Role::Editor, undo_request("alice", 2, version, "a", 1)).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the refusal changed nothing");
    assert_eq!(room.version(), version, "the version did not move");
    assert_eq!(room.undo_depth("alice"), (1, 0), "the step stays on the stack");
}

#[test]
fn sorting_blocks_diffs_to_one_permute_with_explicit_ids() {
    let room = move_room(&["delta", "bravo", "alpha", "charlie"]);
    let (delta, bravo, alpha, charlie) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2), block_id(&room, 3));
    let ids = room.ids().clone();
    let before = room.model().document().clone();
    let mut after = before.clone();
    let blocks = after.body.clone();
    after.body = vec![blocks[2].clone(), blocks[1].clone(), blocks[3].clone(), blocks[0].clone()];

    let changes = crate::diff(&before, &after, &ids).expect("the diff names the sort");
    assert_eq!(changes.len(), 1, "one run, one operation");
    assert_eq!(changes[0].kind, OpKind::Structural);
    assert_eq!(changes[0].target.node_id, alpha, "the block now at the front is the address");
    assert_eq!(changes[0].payload, permute_payload(&[alpha, bravo, charlie, delta]));
}

#[test]
fn a_sort_wider_than_the_bound_is_refused_instead_of_becoming_text_edits() {
    let texts: Vec<String> = (0..70).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let room = move_room(&refs);
    let ids = room.ids().clone();
    let before = room.model().document().clone();
    let mut after = before.clone();
    let blocks = after.body.clone();
    let mut reversed: Vec<Arc<Block>> = blocks.iter().take(67).rev().cloned().collect();
    reversed.extend(blocks.iter().skip(67).cloned());
    after.body = reversed;

    let err = crate::diff(&before, &after, &ids).expect_err("67 blocks are wider than a permute");
    assert!(matches!(err, crate::DiffError::Unsupported(_)), "{err:?}");
}

#[test]
fn a_replica_applies_a_permute_and_keeps_the_ids_of_the_blocks_it_moved() {
    let mut room = move_room(&["gamma", "alpha", "beta"]);
    let (gamma, alpha, beta) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));
    let mut replica = room.model().document().clone();
    let mut replica_ids = room.ids().clone();

    submit_and_mirror(&mut room, op("a", 1, 0, OpKind::Structural, alpha, 0, permute_payload(&[alpha, beta, gamma])), &mut replica, &mut replica_ids);
    assert_eq!(body_texts(&room), ["alpha", "beta", "gamma"]);
    assert_eq!(replica_ids.resolve(NodeId(alpha)).map(|key| key.path.clone()), Ok(vec![0]), "the replica follows the block");
    assert_eq!(replica_ids.resolve(NodeId(beta)).map(|key| key.path.clone()), Ok(vec![1]));
    assert_eq!(replica_ids.resolve(NodeId(gamma)).map(|key| key.path.clone()), Ok(vec![2]));
}

// --- promoting and demoting paragraphs --------------------------------------

/// A document whose paragraphs carry the styles named: `Some("Heading2")` is a
/// heading, `None` body text.
fn styled_doc(texts: &[(&str, Option<&str>)]) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    for (text, style) in texts {
        let mut para = Paragraph::with_text(text, CharProps::default());
        para.props.style = style.map(str::to_string);
        doc.body.push(Arc::new(Block::Para(para)));
    }
    doc
}

fn styled_room(texts: &[(&str, Option<&str>)]) -> Room<DocModel> {
    let mut room = Room::new(DocModel::new(styled_doc(texts)));
    let _ = room.join(None, false);
    room
}

fn level_step(items: serde_json::Value) -> serde_json::Value {
    json!({"op": "set_para_level", "items": items})
}

fn para_style(room: &Room<DocModel>, block: usize) -> Option<String> {
    room.model().document().body.get(block).and_then(|b| b.as_para()).and_then(|p| p.props.style.clone())
}

#[test]
fn levelling_paragraphs_is_one_transactional_level_step() {
    let mut room = styled_room(&[("Title", Some("Heading1")), ("One", None), ("Two", Some("Heading2"))]);
    let (one, two) = (block_id(&room, 1), block_id(&room, 2));

    let applied = room
        .submit(
            "alice",
            Role::Editor,
            op("a", 1, 0, OpKind::Structural, one, 0, level_step(json!([{"id": one, "level": 1}, {"id": two, "level": 3}]))),
        )
        .expect("the level step lands");

    assert_eq!(para_style(&room, 1).as_deref(), Some("Heading1"), "body text becomes the heading the step names");
    assert_eq!(para_style(&room, 2).as_deref(), Some("Heading3"), "a heading goes down a level");
    assert_eq!(
        applied.delta,
        Some(json!({
            "op": "transaction",
            "ops": [{"op": "set_para_level", "items": [{"id": one, "level": 1}, {"id": two, "level": 3}]}],
        })),
        "the step lands as one level operation inside the step's transaction"
    );
    // The ids keep naming their paragraphs: a level change moves no block.
    assert_eq!(room.ids().resolve(NodeId(one)).map(|key| key.path.clone()), Ok(vec![1]));
    assert_eq!(room.ids().resolve(NodeId(two)).map(|key| key.path.clone()), Ok(vec![2]));
}

#[test]
fn undo_of_a_level_step_lands_as_one_transaction_and_redo_returns_the_levels() {
    let mut room = styled_room(&[("Title", Some("Heading1")), ("One", None), ("Two", Some("Heading2"))]);
    let (one, two) = (block_id(&room, 1), block_id(&room, 2));
    room.submit(
        "alice",
        Role::Editor,
        op("a", 1, 0, OpKind::Structural, one, 0, level_step(json!([{"id": one, "level": 1}, {"id": two, "level": 3}]))),
    )
    .expect("the level step lands");

    let undone = room.submit("alice", Role::Editor, undo_request("alice", 2, 1, "a", 1)).expect("the undo lands");
    assert_eq!(
        undone.delta,
        Some(json!({
            "op": "transaction",
            "ops": [{"op": "set_para_level", "items": [{"id": one, "level": null}, {"id": two, "level": 2}]}],
        })),
        "the inverse is one level operation naming the levels before the step"
    );
    assert_eq!(para_style(&room, 1), None, "the promoted paragraph is body text again");
    assert_eq!(para_style(&room, 2).as_deref(), Some("Heading2"), "the demoted heading is back at level 2");

    let redo = Op {
        op_id: OpId::new("alice", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Structural,
        target: Target::new("body", one, 0),
        payload: serde_json::Value::Null,
        history: Some(HistoryAction::Redo { op_id: OpId::new("a", 1) }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(para_style(&room, 1).as_deref(), Some("Heading1"), "redo levels again");
    assert_eq!(para_style(&room, 2).as_deref(), Some("Heading3"));
}

#[test]
fn undo_restores_a_style_the_level_does_not_imply() {
    let mut room = styled_room(&[("Quoted", Some("Quote"))]);
    let one = block_id(&room, 0);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, level_step(json!([{"id": one, "level": 2}]))))
        .expect("the promote lands");
    assert_eq!(para_style(&room, 0).as_deref(), Some("Heading2"));

    let undone = room.submit("alice", Role::Editor, undo_request("alice", 2, 1, "a", 1)).expect("the undo lands");
    assert_eq!(
        undone.delta,
        Some(json!({
            "op": "transaction",
            "ops": [{"op": "set_para_level", "items": [{"id": one, "level": null, "style": "Quote"}]}],
        })),
        "the inverse names the style the level does not imply"
    );
    assert_eq!(para_style(&room, 0).as_deref(), Some("Quote"), "the paragraph keeps the style it had");
}

#[test]
fn undo_restores_a_direct_outline_level_the_style_does_not_imply() {
    let mut para = Paragraph::with_text("Odd", CharProps::default());
    para.props.style = Some("Heading2".into());
    para.props.outline_level = Some(0);
    let mut doc = Document::new();
    doc.body = vec![Arc::new(Block::Para(para))];
    let mut room = Room::new(DocModel::new(doc));
    let _ = room.join(None, false);
    let one = block_id(&room, 0);

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, level_step(json!([{"id": one, "level": 3}]))))
        .expect("the level step lands");
    assert_eq!(para_style(&room, 0).as_deref(), Some("Heading3"));

    let undone = room.submit("alice", Role::Editor, undo_request("alice", 2, 1, "a", 1)).expect("the undo lands");
    assert_eq!(
        undone.delta,
        Some(json!({
            "op": "transaction",
            "ops": [{"op": "set_para_level", "items": [{"id": one, "level": 1, "style": "Heading2", "outline": 0}]}],
        })),
        "the inverse names the style and the direct outline level"
    );
    let restored = room.model().document().body.first().and_then(|b| b.as_para()).expect("a paragraph");
    assert_eq!(restored.props.style.as_deref(), Some("Heading2"));
    assert_eq!(restored.props.outline_level, Some(0), "the direct outline level is back");
}

#[test]
fn a_level_step_with_an_unknown_retired_or_unaddressable_id_is_refused_before_mutation() {
    let mut room = styled_room(&[("One", None), ("Two", None)]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    let before = serde_json::to_value(room.model().document()).expect("json");

    // An id the session never issued.
    let err = room
        .submit(
            "alice",
            Role::Editor,
            op("a", 1, 0, OpKind::Structural, one, 0, level_step(json!([{"id": one, "level": 1}, {"id": 999_999, "level": 1}]))),
        )
        .expect_err("an unknown id is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "nothing changed");
    assert_eq!(room.version(), 0, "the version did not move");

    // The same id twice: the inverse could not name one level per paragraph.
    let err = room
        .submit(
            "alice",
            Role::Editor,
            op("a", 2, 0, OpKind::Structural, one, 0, level_step(json!([{"id": one, "level": 1}, {"id": one, "level": 2}]))),
        )
        .expect_err("a duplicate is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);

    // A level outside 1–9, and an outline level without a named style.
    for payload in [json!([{"id": one, "level": 0}]), json!([{"id": one, "level": 10}]), json!([{"id": one, "level": 1, "outline": 0}])] {
        let err = room
            .submit("alice", Role::Editor, op("a", 3, 0, OpKind::Structural, one, 0, level_step(payload)))
            .expect_err("a malformed level is refused");
        assert_eq!(err.code, RejectCode::Invalid);
        assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    }

    // A stale id that names a path outside the container (a map from an older
    // document): there is no paragraph there, so the step is refused.
    let stale = room.ids_mut().id_for(&NodeKey::new("body", vec![9])).0;
    let err = room
        .submit("alice", Role::Editor, op("a", 4, 0, OpKind::Structural, stale, 0, level_step(json!([{"id": stale, "level": 1}]))))
        .expect_err("a path outside the container is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);

    // A retired id is gone rather than invalid; the document still does not move.
    room.submit("alice", Role::Editor, op("a", 5, 0, OpKind::Structural, two, 0, json!({"op": "remove_block", "index": 1})))
        .expect("remove the last block");
    let before_retired = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();
    let err = room
        .submit("alice", Role::Editor, op("a", 6, version, OpKind::Structural, two, 0, level_step(json!([{"id": two, "level": 1}]))))
        .expect_err("a retired id is refused");
    assert_eq!(err.code, RejectCode::Gone);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before_retired);
    assert_eq!(room.version(), version);
}

#[test]
fn a_level_step_on_a_table_is_refused_before_mutation() {
    let mut room = Room::new(DocModel::new(table_doc(1, 1)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let before = serde_json::to_value(room.model().document()).expect("json");

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table, 0, level_step(json!([{"id": table, "level": 1}]))))
        .expect_err("a table has no outline level");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 0);
}

#[test]
fn a_level_step_wider_than_the_bound_is_refused_before_mutation() {
    let names: Vec<String> = (0..65).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<(&str, Option<&str>)> = names.iter().map(|name| (name.as_str(), None)).collect();
    let mut room = styled_room(&refs);
    let items: Vec<serde_json::Value> = (0..65).map(|index| json!({"id": block_id(&room, index), "level": 1})).collect();
    let before = serde_json::to_value(room.model().document()).expect("json");
    let first = block_id(&room, 0);

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, first, 0, level_step(serde_json::Value::Array(items))))
        .expect_err("65 paragraphs are wider than a level step");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 0);
}

#[test]
fn a_foreign_edit_of_a_levelled_block_makes_undo_a_conflict() {
    let mut room = styled_room(&[("One", None), ("Two", None)]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    room.submit(
        "alice",
        Role::Editor,
        op("a", 1, 0, OpKind::Structural, one, 0, level_step(json!([{"id": one, "level": 1}, {"id": two, "level": 2}]))),
    )
    .expect("the level step lands");

    // Bob types into the second paragraph, which the step levelled.
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, two, 0, json!({"op": "insert_text", "text": "X"}))).expect("bob types");
    let before = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();

    let err = room.submit("alice", Role::Editor, undo_request("alice", 2, version, "a", 1)).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the refusal changed nothing");
    assert_eq!(room.version(), version, "the version did not move");
    assert_eq!(room.undo_depth("alice"), (1, 0), "the step stays on the stack");
}

#[test]
fn a_level_step_that_names_the_current_levels_is_an_identity_step() {
    let mut room = styled_room(&[("One", Some("Heading2"))]);
    let one = block_id(&room, 0);
    let before = serde_json::to_value(room.model().document()).expect("json");

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, level_step(json!([{"id": one, "level": 2}]))))
        .expect("an identity level step still lands");
    assert_eq!(
        applied.delta,
        Some(json!({"op": "transaction", "ops": [{"op": "set_para_level", "items": [{"id": one, "level": 2}]}]})),
        "the step is addressable even when it changes nothing"
    );
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the document is unchanged");
    assert_eq!(room.version(), 1, "the step still moves the version");

    room.submit("alice", Role::Editor, undo_request("alice", 2, 1, "a", 1)).expect("the identity undo lands");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the undo changes nothing either");
    assert_eq!(para_style(&room, 0).as_deref(), Some("Heading2"));
}

#[test]
fn a_replica_applies_a_level_step_and_keeps_the_styles() {
    let mut room = styled_room(&[("Title", Some("Heading1")), ("Body", None)]);
    let body = block_id(&room, 1);
    let mut replica = room.model().document().clone();
    let mut replica_ids = room.ids().clone();

    submit_and_mirror(
        &mut room,
        op("a", 1, 0, OpKind::Structural, body, 0, level_step(json!([{"id": body, "level": 1}]))),
        &mut replica,
        &mut replica_ids,
    );

    let para = replica.body.get(1).and_then(|block| block.as_para()).expect("the replica kept the paragraph");
    assert_eq!(para.props.style.as_deref(), Some("Heading1"), "the replica applied the same level");
    assert_eq!(replica_ids.resolve(NodeId(body)).map(|key| key.path.clone()), Ok(vec![1]), "the id still names its paragraph");
}

// --- bullets and numbering ---------------------------------------------------

/// A document of plain paragraphs, the kind the Bullets and Numbering buttons
/// are pressed on.
fn list_doc(texts: &[&str]) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    for text in texts {
        doc.body.push(Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
    }
    doc
}

fn list_room(texts: &[&str]) -> Room<DocModel> {
    let mut room = Room::new(DocModel::new(list_doc(texts)));
    let _ = room.join(None, false);
    room
}

fn list_request(ids: &[u32], kind: &str, level: u8) -> serde_json::Value {
    json!({"op": "set_para_list", "ids": ids, "kind": kind, "level": level})
}

fn para_numbering(room: &Room<DocModel>, block: usize) -> Option<wordcraft_doc::props::NumRef> {
    room.model().document().body.get(block).and_then(|b| b.as_para()).and_then(|p| p.props.numbering)
}

fn list_style(room: &Room<DocModel>, block: usize) -> Option<String> {
    room.model().document().body.get(block).and_then(|b| b.as_para()).and_then(|p| p.props.style.clone())
}

#[test]
fn listing_paragraphs_is_one_transactional_list_step() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, list_request(&[one, two], "bullets", 1)))
        .expect("the list step lands");

    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_list"));
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    let num = items[0]["num"].as_u64().expect("a list id");
    assert_eq!(items[1]["num"], json!(num), "both paragraphs join the same list");
    assert_eq!(items[0]["level"], json!(1), "the level is explicit");
    assert_eq!(items[1]["level"], json!(1));
    assert_eq!(items[0]["style"], json!("ListParagraph"));
    assert_eq!(items[0]["indentLeft"], serde_json::Value::Null, "the list command clears the indents");
    assert_eq!(delta["ops"][0]["define"][0]["num"]["id"], json!(num), "the definition the step created travels with the delta");
    let num = u32::try_from(num).expect("a list id");
    assert_eq!(para_numbering(&room, 0), Some(wordcraft_doc::props::NumRef { num, level: 1 }));
    assert_eq!(para_numbering(&room, 1), Some(wordcraft_doc::props::NumRef { num, level: 1 }));
    assert_eq!(list_style(&room, 0).as_deref(), Some("ListParagraph"));
    let level = room.model().document().numbering.level(num, 1).expect("the definition has level 1");
    assert_eq!(level.format, wordcraft_doc::section::NumFormat::Bullet);
    // A list change moves no block: the ids keep naming their paragraphs.
    assert_eq!(room.ids().resolve(NodeId(one)).map(|key| key.path.clone()), Ok(vec![0]));
    assert_eq!(room.ids().resolve(NodeId(two)).map(|key| key.path.clone()), Ok(vec![1]));
}

#[test]
fn two_replicas_that_apply_the_same_list_payload_allocate_the_same_numbering_id() {
    let mut first = list_room(&["One", "Two"]);
    let mut second = list_room(&["One", "Two"]);
    let ids = |room: &Room<DocModel>| (block_id(room, 0), block_id(room, 1));
    let (one, two) = ids(&first);

    first
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, list_request(&[one, two], "numbering", 0)))
        .expect("the first replica lists");
    second
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, list_request(&[one, two], "numbering", 0)))
        .expect("the second replica lists");

    assert_eq!(
        serde_json::to_value(first.model().document()).expect("json"),
        serde_json::to_value(second.model().document()).expect("json"),
        "the same payload on the same document creates the same definition"
    );
    assert_eq!(para_numbering(&first, 0), para_numbering(&second, 0), "the paragraphs name the same numbering id");
    assert_eq!(para_numbering(&first, 0).map(|n| n.num), Some(1), "the first free list id");
}

#[test]
fn a_repeated_list_step_is_an_identity_step() {
    let mut room = list_room(&["One"]);
    let one = block_id(&room, 0);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, list_request(&[one], "bullets", 0)))
        .expect("the first list step lands");
    let listed = serde_json::to_value(room.model().document()).expect("json");
    let definitions = room.model().document().numbering.nums.len();

    let applied = room
        .submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, list_request(&[one], "bullets", 0)))
        .expect("a repeated step still lands");
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_list"), "the step stays addressable");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), listed, "a repeated step changes nothing");
    assert_eq!(room.model().document().numbering.nums.len(), definitions, "no second definition is allocated");
}

#[test]
fn undo_of_a_list_step_lands_as_one_transaction_and_redo_returns_the_list() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    let before = serde_json::to_value(room.model().document()).expect("json");
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, list_request(&[one, two], "numbering", 2)))
        .expect("the list step lands");
    let listed = serde_json::to_value(room.model().document()).expect("json");
    let num = para_numbering(&room, 0).expect("a list reference").num;

    let undone = room.submit("alice", Role::Editor, undo_request("alice", 2, 1, "a", 1)).expect("the undo lands");
    let delta = undone.delta.expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"));
    assert_eq!(delta["ops"][0]["op"], json!("set_para_list"));
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items[0]["num"], serde_json::Value::Null, "the previous state was no list");
    assert_eq!(items[1]["num"], serde_json::Value::Null);
    assert_eq!(delta["ops"][0]["undefine"], json!([num]), "the definition the step created is dropped");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "undo returns the exact earlier document, definitions included");

    let redo = Op {
        op_id: OpId::new("alice", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Structural,
        target: Target::new("body", one, 0),
        payload: serde_json::Value::Null,
        history: Some(HistoryAction::Redo { op_id: OpId::new("a", 1) }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), listed, "redo lists again with the same numbering id");
}

#[test]
fn a_list_step_with_an_unknown_retired_or_unaddressable_id_is_refused_before_mutation() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    let before = serde_json::to_value(room.model().document()).expect("json");

    // An id the session never issued.
    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, list_request(&[one, 999_999], "bullets", 0)))
        .expect_err("an unknown id is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 0);

    // The same id twice.
    let err = room
        .submit("alice", Role::Editor, op("a", 2, 0, OpKind::Structural, one, 0, list_request(&[one, one], "bullets", 0)))
        .expect_err("a duplicate is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);

    // An empty set, a level outside 0–8 and an unknown kind.
    for payload in [
        list_request(&[], "bullets", 0),
        list_request(&[one], "bullets", 9),
        list_request(&[one], "outline", 0),
        json!({"op": "set_para_list", "items": []}),
    ] {
        let err =
            room.submit("alice", Role::Editor, op("a", 3, 0, OpKind::Structural, one, 0, payload)).expect_err("a malformed list step is refused");
        assert_eq!(err.code, RejectCode::Invalid);
        assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    }

    // A stale id that names a path outside the container (a map from an older
    // document): there is no paragraph there, so the step is refused.
    let stale = room.ids_mut().id_for(&NodeKey::new("body", vec![9])).0;
    let err = room
        .submit("alice", Role::Editor, op("a", 4, 0, OpKind::Structural, stale, 0, list_request(&[stale], "bullets", 0)))
        .expect_err("a path outside the container is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);

    // A retired id is gone rather than invalid; the document still does not move.
    room.submit("alice", Role::Editor, op("a", 5, 0, OpKind::Structural, two, 0, json!({"op": "remove_block", "index": 1})))
        .expect("remove the last block");
    let before_retired = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();
    let err = room
        .submit("alice", Role::Editor, op("a", 6, version, OpKind::Structural, two, 0, list_request(&[two], "bullets", 0)))
        .expect_err("a retired id is refused");
    assert_eq!(err.code, RejectCode::Gone);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before_retired);
    assert_eq!(room.version(), version);
}

#[test]
fn a_list_step_on_a_table_or_inside_one_is_refused_before_mutation() {
    let mut room = Room::new(DocModel::new(table_doc(1, 1)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let cell = cell_para_id(room.ids(), 0, 0);
    let before = serde_json::to_value(room.model().document()).expect("json");

    for id in [table, cell] {
        let err = room
            .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, id, 0, list_request(&[id], "numbering", 0)))
            .expect_err("a table or a paragraph inside one is refused");
        assert_eq!(err.code, RejectCode::Invalid);
        assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
        assert_eq!(room.version(), 0);
    }
}

#[test]
fn a_list_step_wider_than_the_bound_is_refused_before_mutation() {
    let names: Vec<String> = (0..65).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut room = list_room(&refs);
    let ids: Vec<u32> = (0..65).map(|index| block_id(&room, index)).collect();
    let before = serde_json::to_value(room.model().document()).expect("json");

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, ids[0], 0, list_request(&ids, "bullets", 0)))
        .expect_err("65 paragraphs are wider than a list step");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 0);
}

#[test]
fn a_foreign_edit_of_a_listed_block_makes_undo_a_conflict() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, list_request(&[one, two], "bullets", 0)))
        .expect("the list step lands");

    // Bob types into the second paragraph, which the step listed.
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, two, 0, json!({"op": "insert_text", "text": "X"}))).expect("bob types");
    let before = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();

    let err = room.submit("alice", Role::Editor, undo_request("alice", 2, version, "a", 1)).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the refusal changed nothing");
    assert_eq!(room.version(), version, "the version did not move");
    assert_eq!(room.undo_depth("alice"), (1, 0), "the step stays on the stack");
}

#[test]
fn a_replica_applies_a_list_step_and_creates_the_definition() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    let mut replica = room.model().document().clone();
    let mut replica_ids = room.ids().clone();
    assert!(replica.numbering.nums.is_empty(), "the replica starts without the definition");

    submit_and_mirror(
        &mut room,
        op("a", 1, 0, OpKind::Structural, one, 0, list_request(&[one, two], "numbering", 1)),
        &mut replica,
        &mut replica_ids,
    );
    assert_eq!(replica.numbering.nums.len(), 1, "the replica created the definition from the delta");
    assert_eq!(replica.numbering.nums[0].id, para_numbering(&room, 0).expect("a list").num);
}

// --- multilevel lists and restarted numbering --------------------------------

fn multilevel_request(levels: &[(u32, u8)], kind: &str) -> serde_json::Value {
    let levels: Vec<serde_json::Value> = levels.iter().map(|(id, level)| json!({"id": id, "level": level})).collect();
    json!({"op": "set_para_multilevel", "kind": kind, "levels": levels})
}

fn restart_request(ids: &[u32], start: u32) -> serde_json::Value {
    json!({"op": "set_para_restart_numbering", "ids": ids, "start": start})
}

/// Put the first `count` paragraphs into one numbered list, so a restart step
/// has a list to restart. Returns their ids.
fn list_first(room: &mut Room<DocModel>, count: u32, seq: u64) -> Vec<u32> {
    let ids: Vec<u32> = (0..count).map(|index| block_id(room, index)).collect();
    let version = room.version();
    room.submit("alice", Role::Editor, op("a", seq, version, OpKind::Structural, ids[0], 0, list_request(&ids, "numbering", 0)))
        .expect("the first list step lands");
    ids
}

#[test]
fn a_multilevel_step_lists_each_paragraph_at_its_level() {
    let mut room = list_room(&["One", "Two", "Three"]);
    let (one, two, three) = (block_id(&room, 0), block_id(&room, 1), block_id(&room, 2));

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, multilevel_request(&[(one, 0), (two, 1), (three, 2)], "legal")))
        .expect("the multilevel step lands");
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_multilevel"), "the step keeps its own op name");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 3);
    let num = items[0]["num"].as_u64().expect("a list id");
    for (item, level) in items.iter().zip([0, 1, 2]) {
        assert_eq!(item["num"], json!(num), "every paragraph joins the same definition");
        assert_eq!(item["level"], json!(level), "each paragraph keeps the level it named");
    }
    assert_eq!(delta["ops"][0]["define"][0]["num"]["id"], json!(num), "the definition travels with the delta");
    let definition = delta["ops"][0]["define"][0]["abstract"]["levels"].as_array().expect("levels");
    assert_eq!(definition.len(), 9, "a multilevel definition carries nine levels");
    let num = u32::try_from(num).expect("a list id");
    assert_eq!(room.model().document().numbering.level(num, 2).map(|level| level.text.as_str()), Some("%1.%2.%3."), "legal levels join");
    assert_eq!(para_numbering(&room, 0), Some(wordcraft_doc::props::NumRef { num, level: 0 }));
    assert_eq!(para_numbering(&room, 2), Some(wordcraft_doc::props::NumRef { num, level: 2 }));
}

#[test]
fn two_replicas_that_apply_the_same_multilevel_payload_allocate_the_same_numbering_id() {
    let mut first = list_room(&["One", "Two"]);
    let mut second = list_room(&["One", "Two"]);
    let ids = |room: &Room<DocModel>| (block_id(room, 0), block_id(room, 1));
    let (one, two) = ids(&first);
    let payload = multilevel_request(&[(one, 0), (two, 1)], "outline");

    first.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, payload.clone())).expect("the first replica lists");
    second.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, payload)).expect("the second replica lists");

    assert_eq!(
        serde_json::to_value(first.model().document()).expect("json"),
        serde_json::to_value(second.model().document()).expect("json"),
        "the same payload on the same document creates the same definition"
    );
    assert_eq!(para_numbering(&first, 0), para_numbering(&second, 0), "the paragraphs name the same numbering id");
    assert_eq!(para_numbering(&first, 1), para_numbering(&second, 1));
}

#[test]
fn a_repeated_multilevel_step_is_an_identity_step() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    let payload = multilevel_request(&[(one, 0), (two, 2)], "legal");
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, payload.clone())).expect("the first step lands");
    let listed = serde_json::to_value(room.model().document()).expect("json");
    let definitions = room.model().document().numbering.nums.len();

    let applied = room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, payload)).expect("a repeated step still lands");
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_multilevel"), "the step stays addressable");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), listed, "a repeated step changes nothing");
    assert_eq!(room.model().document().numbering.nums.len(), definitions, "no second definition is allocated");
    assert!(delta["ops"][0].get("define").is_none(), "the repeated step defines nothing");
}

#[test]
fn undo_of_a_multilevel_step_lands_as_one_transaction_and_redo_returns_the_levels() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    let before = serde_json::to_value(room.model().document()).expect("json");
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, multilevel_request(&[(one, 0), (two, 1)], "legal")))
        .expect("the multilevel step lands");
    let listed = serde_json::to_value(room.model().document()).expect("json");
    let num = para_numbering(&room, 0).expect("a list reference").num;

    let undone = room.submit("alice", Role::Editor, undo_request("alice", 2, 1, "a", 1)).expect("the undo lands");
    let delta = undone.delta.expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"));
    assert_eq!(delta["ops"][0]["op"], json!("set_para_multilevel"), "the inverse keeps the step's op name");
    assert_eq!(delta["ops"][0]["items"][0]["num"], serde_json::Value::Null, "the previous state was no list");
    assert_eq!(delta["ops"][0]["items"][1]["num"], serde_json::Value::Null);
    assert_eq!(delta["ops"][0]["undefine"], json!([num]), "the definition the step created is dropped");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "undo returns the exact earlier document");

    let redo = Op {
        op_id: OpId::new("alice", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Structural,
        target: Target::new("body", one, 0),
        payload: serde_json::Value::Null,
        history: Some(HistoryAction::Redo { op_id: OpId::new("a", 1) }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), listed, "redo returns the same levels and numbering id");
}

#[test]
fn a_multilevel_step_with_a_bad_level_or_id_is_refused_before_mutation() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    let before = serde_json::to_value(room.model().document()).expect("json");

    for payload in [
        multilevel_request(&[], "legal"),
        multilevel_request(&[(one, 9)], "legal"),
        multilevel_request(&[(one, 0), (one, 1)], "legal"),
        multilevel_request(&[(one, 0)], "bullets"),
        json!({"op": "set_para_multilevel", "kind": "legal", "levels": [{"id": one}]}),
        multilevel_request(&[(one, 0), (999_999, 1)], "legal"),
    ] {
        let err: wordcraft_collab::wire::Reject = room
            .submit("alice", Role::Editor, op("a", 2, 0, OpKind::Structural, one, 0, payload))
            .expect_err("a malformed multilevel step is refused");
        assert_eq!(err.code, RejectCode::Invalid);
        assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
        assert_eq!(room.version(), 0);
    }

    // A retired id is gone rather than invalid; the document still does not move.
    room.submit("alice", Role::Editor, op("a", 5, 0, OpKind::Structural, two, 0, json!({"op": "remove_block", "index": 1})))
        .expect("remove the last block");
    let before_retired = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();
    let err = room
        .submit("alice", Role::Editor, op("a", 6, version, OpKind::Structural, two, 0, multilevel_request(&[(two, 0)], "legal")))
        .expect_err("a retired id is refused");
    assert_eq!(err.code, RejectCode::Gone);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before_retired);
    assert_eq!(room.version(), version);
}

#[test]
fn a_multilevel_step_on_a_table_is_refused_before_mutation() {
    let mut room = Room::new(DocModel::new(table_doc(1, 1)));
    let _ = room.join(None, false);
    let table = table_id(room.ids());
    let cell = cell_para_id(room.ids(), 0, 0);
    let before = serde_json::to_value(room.model().document()).expect("json");

    for id in [table, cell] {
        let err = room
            .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, id, 0, multilevel_request(&[(id, 0)], "legal")))
            .expect_err("a table or a paragraph inside one is refused");
        assert_eq!(err.code, RejectCode::Invalid);
        assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
        assert_eq!(room.version(), 0);
    }
}

#[test]
fn a_multilevel_step_wider_than_the_bound_is_refused_before_mutation() {
    let names: Vec<String> = (0..65).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut room = list_room(&refs);
    let ids: Vec<u32> = (0..65).map(|index| block_id(&room, index)).collect();
    let levels: Vec<(u32, u8)> = ids.iter().map(|id| (*id, 0)).collect();
    let before = serde_json::to_value(room.model().document()).expect("json");

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, ids[0], 0, multilevel_request(&levels, "legal")))
        .expect_err("65 paragraphs are wider than a multilevel step");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 0);
}

#[test]
fn a_foreign_edit_of_a_multilevel_block_makes_undo_a_conflict() {
    let mut room = list_room(&["One", "Two"]);
    let (one, two) = (block_id(&room, 0), block_id(&room, 1));
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, multilevel_request(&[(one, 0), (two, 1)], "legal")))
        .expect("the multilevel step lands");

    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, two, 0, json!({"op": "insert_text", "text": "X"}))).expect("bob types");
    let before = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();

    let err = room.submit("alice", Role::Editor, undo_request("alice", 2, version, "a", 1)).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the refusal changed nothing");
    assert_eq!(room.version(), version, "the version did not move");
    assert_eq!(room.undo_depth("alice"), (1, 0), "the step stays on the stack");
}

#[test]
fn a_restart_step_moves_the_paragraphs_to_a_new_instance_with_an_explicit_start() {
    let mut room = list_room(&["One", "Two"]);
    let ids = list_first(&mut room, 2, 1);
    let one = ids[0];
    let source = para_numbering(&room, 0).expect("a list").num;

    let applied = room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, restart_request(&ids, 4))).expect("the restart lands");
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_restart_numbering"), "the step keeps its own op name");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    let num = items[0]["num"].as_u64().expect("a restarted list");
    assert_ne!(num, u64::from(source), "a fresh instance");
    assert_eq!(items[1]["num"], json!(num), "both named paragraphs move");
    assert_eq!(items[0]["level"], json!(0), "the level is kept");
    assert_eq!(delta["ops"][0]["define"][0]["num"]["startOverrides"], json!([[0, 4]]), "the explicit start travels with the delta");
    let document = room.model().document();
    let instance = document.numbering.num(u32::try_from(num).expect("a list id")).expect("the new instance");
    assert_eq!(instance.abstract_id, document.numbering.num(source).expect("the source").abstract_id, "the same list, restarted");
    assert_eq!(instance.start_overrides, vec![(0, 4)]);
    assert_eq!(para_numbering(&room, 1), Some(wordcraft_doc::props::NumRef { num: u32::try_from(num).expect("a list id"), level: 0 }));
}

#[test]
fn two_replicas_that_apply_the_same_restart_payload_allocate_the_same_numbering_id() {
    let mut first = list_room(&["One", "Two"]);
    let mut second = list_room(&["One", "Two"]);
    let first_ids = list_first(&mut first, 2, 1);
    let second_ids = list_first(&mut second, 2, 1);
    assert_eq!(first_ids, second_ids, "the two replicas address the same paragraphs");
    let (one, two) = (first_ids[0], first_ids[1]);
    let payload = restart_request(&[one, two], 7);

    first.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, payload.clone())).expect("the first replica restarts");
    second.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, payload)).expect("the second replica restarts");

    assert_eq!(
        serde_json::to_value(first.model().document()).expect("json"),
        serde_json::to_value(second.model().document()).expect("json"),
        "the same payload on the same document restarts into the same instance"
    );
    assert_eq!(para_numbering(&first, 0), para_numbering(&second, 0));
    assert_eq!(para_numbering(&first, 0).map(|n| n.num), Some(2), "the first free list id");
}

#[test]
fn a_repeated_restart_step_is_an_identity_step() {
    let mut room = list_room(&["One", "Two"]);
    let ids = list_first(&mut room, 2, 1);
    let one = ids[0];
    let payload = restart_request(&ids, 3);
    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, payload.clone())).expect("the first restart lands");
    let restarted = serde_json::to_value(room.model().document()).expect("json");
    let definitions = room.model().document().numbering.nums.len();

    let applied = room.submit("alice", Role::Editor, op("a", 3, 2, OpKind::Structural, one, 0, payload)).expect("a repeated restart still lands");
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_restart_numbering"), "the step stays addressable");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), restarted, "a repeated step changes nothing");
    assert_eq!(room.model().document().numbering.nums.len(), definitions, "no second instance is allocated");
    assert!(delta["ops"][0].get("define").is_none(), "the repeated step defines nothing");
}

#[test]
fn undo_of_a_restart_step_lands_as_one_transaction_and_redo_returns_the_start() {
    let mut room = list_room(&["One", "Two"]);
    let ids = list_first(&mut room, 2, 1);
    let one = ids[0];
    let source = para_numbering(&room, 0).expect("a list").num;
    let before = serde_json::to_value(room.model().document()).expect("json");
    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, restart_request(&ids, 5))).expect("the restart lands");
    let restarted = serde_json::to_value(room.model().document()).expect("json");
    let num = para_numbering(&room, 0).expect("a list reference").num;

    let undone = room.submit("alice", Role::Editor, undo_request("alice", 3, 2, "a", 2)).expect("the undo lands");
    let delta = undone.delta.expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"));
    assert_eq!(delta["ops"][0]["op"], json!("set_para_restart_numbering"), "the inverse keeps the step's op name");
    assert_eq!(delta["ops"][0]["items"][0]["num"], json!(source), "the earlier instance is back");
    assert_eq!(delta["ops"][0]["undefine"], json!([num]), "the instance the step created is dropped");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "undo returns the exact earlier document");

    let redo = Op {
        op_id: OpId::new("alice", 4),
        base_version: 3,
        epoch: None,
        kind: OpKind::Structural,
        target: Target::new("body", one, 0),
        payload: serde_json::Value::Null,
        history: Some(HistoryAction::Redo { op_id: OpId::new("a", 2) }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), restarted, "redo recreates the same instance and start");
}

#[test]
fn a_restart_step_with_a_bad_start_or_address_is_refused_before_mutation() {
    let mut room = list_room(&["One", "Two"]);
    let ids = list_first(&mut room, 2, 1);
    let one = ids[0];
    let before = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();

    for payload in [
        restart_request(&ids, 0),
        json!({"op": "set_para_restart_numbering", "ids": ids, "start": -1}),
        restart_request(&ids, 100_001),
        restart_request(&[], 1),
        restart_request(&[one, one], 1),
        restart_request(&[one, 999_999], 1),
        json!({"op": "set_para_restart_numbering", "ids": [one]}),
    ] {
        let err = room
            .submit("alice", Role::Editor, op("a", 9, version, OpKind::Structural, one, 0, payload))
            .expect_err("a malformed restart step is refused");
        assert_eq!(err.code, RejectCode::Invalid);
        assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
        assert_eq!(room.version(), version);
    }

    // A paragraph outside a list cannot be restarted.
    let mut plain = list_room(&["Plain"]);
    let block = block_id(&plain, 0);
    let err = plain
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, block, 0, restart_request(&[block], 2)))
        .expect_err("a paragraph outside a list is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(plain.version(), 0);

    // A restart restarts one list: two different lists in one step are refused.
    let mut mixed = list_room(&["A", "B"]);
    let (a, b) = (block_id(&mixed, 0), block_id(&mixed, 1));
    mixed.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, a, 0, list_request(&[a], "bullets", 0))).expect("the first list lands");
    mixed.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, b, 0, list_request(&[b], "numbering", 0))).expect("the second list lands");
    let mixed_before = serde_json::to_value(mixed.model().document()).expect("json");
    let err = mixed
        .submit("alice", Role::Editor, op("a", 3, 2, OpKind::Structural, a, 0, restart_request(&[a, b], 2)))
        .expect_err("a restart of two lists is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(mixed.model().document()).expect("json"), mixed_before);
    assert_eq!(mixed.version(), 2);

    // A retired id is gone; the document still does not move.
    let mut retiring = list_room(&["One", "Two"]);
    let ids = list_first(&mut retiring, 2, 1);
    let two = ids[1];
    retiring
        .submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, two, 0, json!({"op": "remove_block", "index": 1})))
        .expect("remove the last block");
    let retired_before = serde_json::to_value(retiring.model().document()).expect("json");
    let version = retiring.version();
    let err = retiring
        .submit("alice", Role::Editor, op("a", 3, version, OpKind::Structural, two, 0, restart_request(&[two], 2)))
        .expect_err("a retired id is refused");
    assert_eq!(err.code, RejectCode::Gone);
    assert_eq!(serde_json::to_value(retiring.model().document()).expect("json"), retired_before);
    assert_eq!(retiring.version(), version);
}

#[test]
fn a_restart_step_on_a_table_is_refused_before_mutation() {
    let mut room = Room::new(DocModel::new(table_doc(1, 1)));
    let _ = room.join(None, false);
    let cell = cell_para_id(room.ids(), 0, 0);
    let before = serde_json::to_value(room.model().document()).expect("json");

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, cell, 0, restart_request(&[cell], 2)))
        .expect_err("a paragraph inside a table is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 0);
}

#[test]
fn a_restart_step_wider_than_the_bound_is_refused_before_mutation() {
    let names: Vec<String> = (0..65).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut room = list_room(&refs);
    let ids: Vec<u32> = (0..65).map(|index| block_id(&room, index)).collect();
    // The bound is a list-step bound too, so list the paragraphs in two steps.
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, ids[0], 0, list_request(&ids[..33], "numbering", 0)))
        .expect("the first half lists");
    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, ids[33], 0, list_request(&ids[33..], "numbering", 0)))
        .expect("the second half lists");
    let before = serde_json::to_value(room.model().document()).expect("json");

    let err = room
        .submit("alice", Role::Editor, op("a", 3, 2, OpKind::Structural, ids[0], 0, restart_request(&ids, 2)))
        .expect_err("65 paragraphs are wider than a restart step");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before);
    assert_eq!(room.version(), 2);
}

#[test]
fn a_foreign_edit_of_a_restarted_block_makes_undo_a_conflict() {
    let mut room = list_room(&["One", "Two"]);
    let ids = list_first(&mut room, 2, 1);
    let (one, two) = (ids[0], ids[1]);
    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, restart_request(&ids, 4))).expect("the restart lands");

    room.submit("bob", Role::Editor, op("b", 1, 2, OpKind::Primitive, two, 0, json!({"op": "insert_text", "text": "X"}))).expect("bob types");
    let before = serde_json::to_value(room.model().document()).expect("json");
    let version = room.version();

    let err = room.submit("alice", Role::Editor, undo_request("alice", 3, version, "a", 2)).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(room.model().document()).expect("json"), before, "the refusal changed nothing");
    assert_eq!(room.version(), version, "the version did not move");
    assert_eq!(room.undo_depth("alice"), (2, 0), "the restart step stays on the stack above the listing");
}
