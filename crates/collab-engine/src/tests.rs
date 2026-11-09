//! Command operations through `Session::run`, and primitives through the
//! document adapter behind the same room.

use serde_json::json;

use std::sync::Arc;
use wordcraft_collab::ids::{NodeIds, NodeKey};
use wordcraft_collab::room::{Model, Role, Room};
use wordcraft_collab::wire::{Op, OpId, OpKind, RejectCode, Target};

use wordcraft_doc::{Block, CharProps, Document, Paragraph, Path, StoryRef, Table};
use wordcraft_engine::Session;

use crate::SessionModel;

fn room() -> Room<SessionModel> {
    Room::new(SessionModel::from_document(Document::new()))
}

fn node(room: &mut Room<SessionModel>) -> u32 {
    room.join(None, false).nodes.first().expect("a body node").id
}

fn op(client: &str, seq: u64, base: u64, kind: OpKind, node: u32, off: usize, payload: serde_json::Value) -> Op {
    Op { op_id: OpId::new(client, seq), base_version: base, epoch: None, kind, target: Target::new("body", node, off), payload, history: None }
}

fn body_text(room: &Room<SessionModel>) -> String {
    room.model().session().doc.body.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default()
}

#[test]
fn a_command_runs_through_the_engine_at_an_explicit_caret() {
    let mut room = room();
    let node = node(&mut room);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, json!({"command": "text.insert", "params": {"text": "Hello"}})))
        .expect("command applies");
    assert_eq!(body_text(&room), "Hello");
}

#[test]
fn commands_from_forbidden_areas_are_refused() {
    let mut room = room();
    let node = node(&mut room);
    for command in ["file.new", "file.open", "tools.recordMacro", "view.zoom", "edit.undo"] {
        let err = room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, json!({"command": command}))).expect_err("refused");
        assert_eq!(err.code, RejectCode::Forbidden, "{command}");
    }
}

#[test]
fn an_unknown_command_is_invalid() {
    let mut room = room();
    let node = node(&mut room);
    let err = room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, json!({"command": "nope.nothing"}))).expect_err("unknown");
    assert_eq!(err.code, RejectCode::Invalid);
}

#[test]
fn a_read_only_command_is_refused() {
    let mut room = room();
    let node = node(&mut room);
    // A pure command (no document change) is not a replication unit.
    let err = room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, json!({"command": "edit.copy"}))).expect_err("pure");
    assert_eq!(err.code, RejectCode::Invalid);
}

#[test]
fn primitives_still_work_behind_the_session() {
    let mut room = room();
    let node = node(&mut room);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, node, 0, json!({"op": "insert_text", "text": "Hi"}))).expect("primitive");
    assert_eq!(body_text(&room), "Hi");
}

#[test]
fn a_command_on_a_deleted_node_is_gone() {
    let mut room = room();
    let node = node(&mut room);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, node, 0, json!({"op": "insert_block", "index": 1}))).expect("insert");
    let second = room.join(None, false).nodes.iter().find(|n| n.path == vec![1]).map(|n| n.id).expect("second node");
    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, second, 0, json!({"op": "remove_block"}))).expect("remove");

    let err = room
        .submit("bob", Role::Editor, op("b", 1, 2, OpKind::Command, second, 0, json!({"command": "text.insert", "params": {"text": "x"}})))
        .expect_err("gone");
    assert_eq!(err.code, RejectCode::Gone);
}

#[test]
fn the_actor_is_the_tracked_change_author() {
    let mut document = Document::new();
    document.settings.track_changes = true;
    let mut room = Room::new(SessionModel::from_document(document));
    let node = node(&mut room);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, json!({"command": "text.insert", "params": {"text": "Hi"}})))
        .expect("command applies");

    let session = room.model().session();
    assert!(session.doc.revisions.iter().any(|revision| revision.author == "alice"), "{:?}", session.doc.revisions);
}

#[test]
fn pending_formatting_travels_with_the_command() {
    let mut room = room();
    let node = node(&mut room);
    let payload = json!({"command": "text.insert", "params": {"text": "Bold"}, "pending": {"bold": true}});
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, payload)).expect("command applies");

    let session = room.model().session();
    let para = session.doc.body.first().and_then(|block| block.as_para()).expect("a paragraph");
    assert_eq!(para.runs.first().map(|run| run.props.bold), Some(Some(true)), "{:?}", para.runs);
}

#[test]
fn the_server_session_keeps_no_undo_history() {
    let mut room = room();
    let node = node(&mut room);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, json!({"command": "text.insert", "params": {"text": "Hi"}})))
        .expect("command applies");
    assert_eq!(room.model().session().undo_depth(), 0);
}

#[test]
fn a_command_is_an_undo_barrier() {
    let mut room = room();
    let node = node(&mut room);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, json!({"command": "text.insert", "params": {"text": "Hi"}})))
        .expect("command applies");
    let undo = Op {
        op_id: wordcraft_collab::wire::OpId::new("a", 2),
        base_version: 1,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Undo { op_id: wordcraft_collab::wire::OpId::new("a", 1) }),
    };
    let err = room.submit("alice", Role::Editor, undo).expect_err("a command has no inverse");
    assert_eq!(err.code, RejectCode::Unsupported);
    assert_eq!(body_text(&room), "Hi", "the refused undo changed nothing");
}

fn replica_session() -> (Session, NodeIds) {
    let session = Session::new(Document::new());
    let mut ids = NodeIds::new();
    SessionModel::from_document(session.doc.clone()).index_nodes(&mut ids);
    (session, ids)
}

#[test]
fn a_command_delta_replays_on_a_replica_session() {
    let mut room = room();
    let node = node(&mut room);
    let payload = json!({"command": "text.insert", "params": {"text": "Hello"}, "pending": {"bold": true}});
    let applied = room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, node, 0, payload)).expect("command applies");

    let (mut replica, mut ids) = replica_session();
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica replays the command");
    let text = replica.doc.body.first().and_then(|block| block.as_para()).map(|para| para.text.clone()).unwrap_or_default();
    assert_eq!(text, "Hello");
}

#[test]
fn a_primitive_delta_replays_on_a_replica_session() {
    let mut room = room();
    let node = node(&mut room);
    let applied =
        room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, node, 0, json!({"op": "insert_text", "text": "Hi"}))).expect("primitive");

    let (mut replica, mut ids) = replica_session();
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica applies the delta");
    let text = replica.doc.body.first().and_then(|block| block.as_para()).map(|para| para.text.clone()).unwrap_or_default();
    assert_eq!(text, "Hi");
}

#[test]
fn a_table_row_insert_lands_through_the_session_adapter_and_moves_cell_ids() {
    let mut doc = Document::new();
    let mut table = wordcraft_doc::Table::new(2, 2, 200.0);
    for (r, row) in table.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = wordcraft_doc::Cell::with_text(&format!("r{r}c{c}"));
        }
    }
    doc.body.push(Arc::new(Block::Table(table)));
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let table_node = plan.nodes.iter().find(|node| node.path == vec![1]).expect("the table has an id").id;
    let cell_node = plan.nodes.iter().find(|node| node.path == vec![1, 1, 1, 0]).expect("the cell paragraph has an id").id;

    let row = json!(wordcraft_doc::Table::new(1, 2, 200.0).rows[0]);
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table_node, 0, json!({"op": "insert_row", "index": 0, "row": row})))
        .expect("the row lands through the session adapter");

    assert_eq!(room.model().session().doc.table(StoryRef::Body, &Path(vec![1])).map_or(0, |t| t.rows.len()), 3);
    // The client's cell id followed its cell to row 2 instead of naming the new
    // empty row.
    let key = room.ids().resolve(wordcraft_collab::NodeId(cell_node)).expect("the id is live");
    assert_eq!(key.path, vec![1, 2, 1, 0]);
    assert_eq!(room.model().session().doc.para(StoryRef::Body, &Path(vec![1, 2, 1, 0])).map(|p| p.text.clone()), Some("r1c1".to_string()));
}

#[test]
fn a_column_insert_lands_through_the_session_adapter_and_keeps_the_caret_cell() {
    let mut doc = Document::new();
    let mut table = wordcraft_doc::Table::new(2, 2, 200.0);
    for (r, row) in table.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = wordcraft_doc::Cell::with_text(&format!("r{r}c{c}"));
        }
    }
    doc.body.push(Arc::new(Block::Table(table)));
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let table_node = plan.nodes.iter().find(|node| node.path == vec![1]).expect("the table has an id").id;
    let cell_node = plan.nodes.iter().find(|node| node.path == vec![1, 1, 1, 0]).expect("the cell paragraph has an id").id;

    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, table_node, 0, json!({"op": "insert_column", "index": 1})))
        .expect("the column lands through the session adapter");

    // The cell's id followed the cell past the new column; the caret holder can
    // keep typing where they were instead of resyncing.
    let key = room.ids().resolve(wordcraft_collab::NodeId(cell_node)).expect("the id is live");
    assert_eq!(key.path, vec![1, 1, 2, 0]);
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, cell_node, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("the stale address names the same cell");
    let text = room.model().session().doc.para(StoryRef::Body, &Path(vec![1, 1, 2, 0])).map(|p| p.text.clone());
    assert_eq!(text, Some("r1c1!".to_string()));
}

#[test]
fn a_merge_lands_through_the_session_adapter_and_the_caret_cell_keeps_its_id() {
    let mut doc = Document::new();
    let mut table = wordcraft_doc::Table::new(2, 2, 200.0);
    for (r, row) in table.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = wordcraft_doc::Cell::with_text(&format!("r{r}c{c}"));
        }
    }
    doc.body.push(Arc::new(Block::Table(table)));
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let table_node = plan.nodes.iter().find(|node| node.path == vec![1]).expect("the table has an id").id;
    let cell = plan.nodes.iter().find(|node| node.path == vec![1, 0, 0]).expect("the cell has an id").id;
    let caret = plan.nodes.iter().find(|node| node.path == vec![1, 0, 0, 0]).expect("the cell paragraph has an id").id;

    room.submit(
        "alice",
        Role::Editor,
        op("a", 1, 0, OpKind::Structural, table_node, 0, json!({"op": "merge_cells", "rows": [0, 0], "cols": [0, 1]})),
    )
    .expect("the merge lands through the session adapter");

    assert_eq!(room.ids().resolve(wordcraft_collab::NodeId(cell)).map(|key| key.path.clone()), Ok(vec![1, 0, 0]));
    assert_eq!(room.ids().resolve(wordcraft_collab::NodeId(caret)).map(|key| key.path.clone()), Ok(vec![1, 0, 0, 0]));
    room.submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, caret, 4, json!({"op": "insert_text", "text": "!"})))
        .expect("the caret holder keeps typing in the merged cell");
    let text = room.model().session().doc.para(StoryRef::Body, &Path(vec![1, 0, 0, 0])).map(|p| p.text.clone());
    assert_eq!(text, Some("r0c0!".to_string()));
}

#[test]
fn a_table_command_retires_the_cell_ids_it_moves() {
    let mut doc = Document::new();
    let mut table = wordcraft_doc::Table::new(2, 2, 200.0);
    for (r, row) in table.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = wordcraft_doc::Cell::with_text(&format!("r{r}c{c}"));
        }
    }
    doc.body.push(Arc::new(Block::Table(table)));
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let stale = plan.nodes.iter().find(|node| node.path == vec![1, 1, 1, 0]).expect("a cell id").id;
    let caret = plan.nodes.iter().find(|node| node.path == vec![1, 1, 1, 0]).expect("a caret node").id;

    // A command the client could not normalize (a stale client, or track
    // changes on) inserts a row above; the cell ids of that table move.
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, caret, 4, json!({"command": "table.insertRowAbove"})))
        .expect("the command runs");

    assert_eq!(room.model().session().doc.table(StoryRef::Body, &Path(vec![1])).map_or(0, |t| t.rows.len()), 3);
    // The id is retired, not re-pointed: an edit that still carries it is
    // refused instead of landing in another cell.
    let id = wordcraft_collab::NodeId(stale);
    assert!(matches!(room.ids().resolve(id), Err(wordcraft_collab::ids::Resolve::Gone(_))));
    let err = room
        .submit("bob", Role::Editor, op("b", 1, 1, OpKind::Primitive, stale, 0, json!({"op": "insert_text", "text": "!"})))
        .expect_err("the stale cell id is refused");
    assert_eq!(err.code, RejectCode::Gone);
}

#[test]
fn a_hundred_page_document_keeps_up_with_edits() {
    let mut doc = Document::new();
    doc.body.clear();
    for index in 0..3000 {
        doc.body.push(Arc::new(Block::Para(Paragraph::with_text(&format!("Paragraph {index} of a long document."), CharProps::default()))));
    }
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let nodes: Vec<u32> = (0..3000).filter_map(|index| plan.nodes.iter().find(|node| node.path == vec![index]).map(|node| node.id)).collect();
    assert_eq!(nodes.len(), 3000, "every paragraph got a session id");

    let mut version = 0;
    for (index, node) in nodes.iter().take(100).enumerate() {
        let client = if index % 2 == 0 { "alice" } else { "bob" };
        let applied = room
            .submit(
                client,
                Role::Editor,
                op(client, index as u64 + 1, version, OpKind::Primitive, *node, 0, json!({"op": "insert_text", "text": "X"})),
            )
            .expect("an edit on a long document");
        version = applied.version;
    }
    assert_eq!(room.version(), 100);
    assert!(!room.model().snapshot().is_empty());
}

fn format_room(text: &str) -> (Room<SessionModel>, u32) {
    let mut doc = Document::new();
    doc.body.clear();
    doc.body.push(Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
    let mut room = Room::new(SessionModel::from_document(doc));
    let node = node(&mut room);
    (room, node)
}

fn selection(node: u32, from: usize, to: usize) -> serde_json::Value {
    json!({"anchor": {"nodeId": node, "off": from}, "focus": {"nodeId": node, "off": to}})
}

fn runs(room: &Room<SessionModel>) -> Vec<wordcraft_doc::Run> {
    room.model().session().doc.body.first().and_then(|b| b.as_para()).map(|p| p.runs.clone()).unwrap_or_default()
}

fn format_op(client: &str, seq: u64, base: u64, node: u32, command: &str, params: serde_json::Value, selected: serde_json::Value) -> Op {
    op(client, seq, base, OpKind::Command, node, 0, json!({"command": command, "params": params, "selection": selected}))
}

fn undo_op(seq: u64, base: u64, node: u32, step: &Op) -> Op {
    Op {
        op_id: OpId::new("a", seq),
        base_version: base,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Undo { op_id: step.op_id.clone() }),
    }
}

#[test]
fn a_formatting_command_carries_the_previous_value_as_its_inverse() {
    let (mut room, node) = format_room("Hello");
    let before = serde_json::to_value(&room.model().session().doc).expect("json");

    let bold = format_op("a", 1, 0, node, "format.bold", json!({"value": true}), selection(node, 0, 5));
    room.submit("alice", Role::Editor, bold.clone()).expect("bold lands");
    assert_eq!(runs(&room)[0].props.bold, Some(true), "{:?}", runs(&room));

    // The inverse is a primitive naming the previous value, not the command
    // again: a toggle would flip twice.
    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, node, &bold)).expect("the inverse lands");
    assert_eq!(serde_json::to_value(&room.model().session().doc).expect("json"), before, "the author is back at the value it had");
    assert_eq!(undone.version, 2);

    let redo = Op {
        op_id: OpId::new("a", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: bold.op_id.clone() }),
    };
    room.submit("alice", Role::Editor, redo).expect("redo replays the command");
    assert_eq!(runs(&room)[0].props.bold, Some(true), "redo restores the formatted value");
}

#[test]
fn a_paragraph_align_undo_restores_every_covered_paragraph() {
    let mut doc = Document::new();
    doc.body.clear();
    doc.body.push(Arc::new(Block::Para(Paragraph::with_text("alpha", CharProps::default()))));
    doc.body.push(Arc::new(Block::Para(Paragraph::with_text("beta", CharProps::default()))));
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let first = plan.nodes.iter().find(|n| n.path == vec![0]).expect("first node").id;
    let second = plan.nodes.iter().find(|n| n.path == vec![1]).expect("second node").id;
    let before = serde_json::to_value(&room.model().session().doc).expect("json");

    // The selection ends inside the second paragraph, so both are covered.
    let align = op(
        "a",
        1,
        0,
        OpKind::Command,
        first,
        0,
        json!({
            "command": "para.alignCenter",
            "params": {},
            "selection": {"anchor": {"nodeId": first, "off": 0}, "focus": {"nodeId": second, "off": 2}},
        }),
    );
    room.submit("alice", Role::Editor, align.clone()).expect("align lands");
    let aligned = |room: &Room<SessionModel>, path: usize| room.model().session().doc.para(StoryRef::Body, &Path::top(path)).map(|p| p.props.align);
    assert_eq!(aligned(&room, 0), Some(Some(wordcraft_doc::Align::Center)));
    assert_eq!(aligned(&room, 1), Some(Some(wordcraft_doc::Align::Center)));

    room.submit("alice", Role::Editor, undo_op(2, 1, first, &align)).expect("both inverses land as one step");
    assert_eq!(serde_json::to_value(&room.model().session().doc).expect("json"), before, "the paragraph properties are back");
}

#[test]
fn a_case_change_undo_restores_the_previous_text_and_runs() {
    let mut doc = Document::new();
    doc.body.clear();
    let mut para = Paragraph::with_text("hello", CharProps::default());
    // Two runs with different formatting: the command keeps them, so the
    // inverse must put the exact run composition back with the old text.
    para.runs = vec![
        wordcraft_doc::Run { len: 2, props: CharProps { bold: Some(true), ..CharProps::default() } },
        wordcraft_doc::Run { len: 3, props: CharProps::default() },
    ];
    doc.body.push(Arc::new(Block::Para(para)));
    let mut room = Room::new(SessionModel::from_document(doc));
    let node = node(&mut room);
    let before = serde_json::to_value(&room.model().session().doc).expect("json");

    let change = format_op("a", 1, 0, node, "format.changeCase", json!({"mode": "upper"}), selection(node, 0, 5));
    room.submit("alice", Role::Editor, change.clone()).expect("the case change lands");
    assert_eq!(body_text(&room), "HELLO");
    assert_eq!(runs(&room)[0].props.bold, Some(true), "the case change keeps the runs' formatting");

    // The inverse is a primitive naming the previous body, not the command: the
    // old text, its inline objects and its run composition come back exactly.
    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, node, &change)).expect("the body restore lands");
    assert_eq!(serde_json::to_value(&room.model().session().doc).expect("json"), before, "the text and the runs are back");
    let delta = undone.delta.expect("a delta");
    assert_eq!(delta["op"], json!("set_para_format"));
    assert_eq!(delta["text"], json!("hello"));
    assert_eq!(delta["expect"], json!("HELLO"));

    // Redo replays the command over the restored text.
    let redo = Op {
        op_id: OpId::new("a", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: change.op_id.clone() }),
    };
    room.submit("alice", Role::Editor, redo).expect("redo replays the command");
    assert_eq!(body_text(&room), "HELLO", "the case change lands again");
}

#[test]
fn an_enclose_undo_removes_the_field_and_restores_the_text() {
    let (mut room, node) = format_room("x");
    let before = serde_json::to_value(&room.model().session().doc).expect("json");

    let enclose = format_op("a", 1, 0, node, "format.enclose", json!({"shape": "circle", "text": "x"}), selection(node, 0, 1));
    room.submit("alice", Role::Editor, enclose.clone()).expect("the enclosure lands");
    let enclosed = room.model().session().doc.body.first().and_then(|b| b.as_para()).cloned().expect("a paragraph");
    assert_eq!(enclosed.objects.len(), 1, "the enclosure is an inline object");
    assert_ne!(enclosed.text, "x", "the character is now the object's placeholder");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, node, &enclose)).expect("the body restore lands");
    assert_eq!(serde_json::to_value(&room.model().session().doc).expect("json"), before, "the text and the objects are back");
    let delta = undone.delta.expect("a delta");
    assert_eq!(delta["op"], json!("set_para_format"));
    assert_eq!(delta["text"], json!("x"));
    assert_eq!(delta["objects"], json!([]), "the field is gone");
}

#[test]
fn an_undo_delta_replays_on_a_replica_session() {
    let (mut room, node) = format_room("hello");
    let before = serde_json::to_value(&room.model().session().doc).expect("json");

    // A second replica holds the same document and its own map of the ids the
    // room issued; it learns both the command and the undo only from deltas.
    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    let change = format_op("a", 1, 0, node, "format.changeCase", json!({"mode": "upper"}), selection(node, 0, 5));
    let applied = room.submit("alice", Role::Editor, change.clone()).expect("the case change lands");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica replays the command");
    let replica_text = replica.doc.body.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(replica_text, "HELLO");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, node, &change)).expect("the body restore lands");
    crate::apply_delta(&mut replica, &mut ids, undone.delta.as_ref().expect("a delta")).expect("the replica replays the restore");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), before, "the replica is back at the exact earlier document");
}

#[test]
fn a_step_over_more_than_256_paragraphs_stays_an_undo_barrier() {
    let mut doc = Document::new();
    doc.body.clear();
    for _ in 0..300 {
        doc.body.push(Arc::new(Block::Para(Paragraph::with_text("ab", CharProps::default()))));
    }
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let first = plan.nodes.iter().find(|n| n.path == vec![0]).expect("first node").id;
    let last = plan.nodes.iter().find(|n| n.path == vec![299]).expect("last node").id;

    let change = format_op(
        "a",
        1,
        0,
        first,
        "format.changeCase",
        json!({"mode": "upper"}),
        json!({"anchor": {"nodeId": first, "off": 0}, "focus": {"nodeId": last, "off": 2}}),
    );
    // The command still runs; only its inverse is not carried.
    room.submit("alice", Role::Editor, change.clone()).expect("a wide case change still lands");
    assert_eq!(body_text(&room), "AB", "the first paragraph changed case");

    let err = room.submit("alice", Role::Editor, undo_op(2, 1, first, &change)).expect_err("a step this wide has no inverse");
    assert_eq!(err.code, RejectCode::Unsupported);
    assert_eq!(body_text(&room), "AB", "the refused undo changed nothing");
}

#[test]
fn a_body_restore_on_a_retired_node_is_refused_before_mutation() {
    let (mut room, node) = format_room("hello");
    room.ids_mut().remove(wordcraft_collab::ids::NodeId(node), 0);
    let before = serde_json::to_value(&room.model().session().doc).expect("json");

    let err = room
        .submit(
            "alice",
            Role::Editor,
            op(
                "a",
                1,
                0,
                OpKind::Primitive,
                node,
                0,
                json!({"op": "set_para_format", "text": "HELLO", "objects": [], "expect": "hello", "runs": [{"len": 5}]}),
            ),
        )
        .expect_err("a retired id cannot name the paragraph");
    assert_eq!(err.code, RejectCode::Gone);
    assert_eq!(serde_json::to_value(&room.model().session().doc).expect("json"), before, "nothing changed");
    assert_eq!(room.version(), 0, "the room did not advance");
}

#[test]
fn a_format_restore_on_a_node_that_is_not_a_paragraph_is_refused_before_mutation() {
    let mut doc = Document::new();
    let mut table = wordcraft_doc::Table::new(1, 1, 200.0);
    table.rows[0].cells[0] = wordcraft_doc::Cell::with_text("cell");
    doc.body.push(Arc::new(Block::Table(table)));
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let table_node = plan.nodes.iter().find(|n| n.path == vec![1]).expect("the table has an id").id;
    let before = serde_json::to_value(&room.model().session().doc).expect("json");

    let err = room
        .submit(
            "alice",
            Role::Editor,
            op("a", 1, 0, OpKind::Primitive, table_node, 0, json!({"op": "set_para_format", "props": {"align": "center"}})),
        )
        .expect_err("a table is not a paragraph");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(&room.model().session().doc).expect("json"), before, "nothing changed");
    assert_eq!(room.version(), 0, "the room did not advance");
}

#[test]
fn a_format_restore_that_does_not_cover_the_text_is_refused_before_mutation() {
    let (mut room, node) = format_room("Hello");
    let before = serde_json::to_value(&room.model().session().doc).expect("json");
    for runs in [json!([{"len": 999, "props": {"bold": true}}]), json!([{"len": 2, "props": {"bold": true}}])] {
        let err = room
            .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Primitive, node, 0, json!({"op": "set_para_format", "runs": runs})))
            .expect_err("the runs do not cover the paragraph");
        assert_eq!(err.code, RejectCode::Invalid, "{runs}");
        assert_eq!(serde_json::to_value(&room.model().session().doc).expect("json"), before, "nothing changed");
    }
}

#[test]
fn a_formatting_command_over_a_paragraph_without_a_session_id_is_refused_before_mutation() {
    let mut doc = Document::new();
    doc.body.clear();
    for text in ["alpha", "beta", "gamma"] {
        doc.body.push(Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
    }
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let first = plan.nodes.iter().find(|n| n.path == vec![0]).expect("first node").id;
    let middle = plan.nodes.iter().find(|n| n.path == vec![1]).expect("middle node").id;
    let last = plan.nodes.iter().find(|n| n.path == vec![2]).expect("last node").id;
    // The inverse needs an id for every covered paragraph; without one the
    // command is refused instead of applying a change undo cannot name.
    room.ids_mut().remove(wordcraft_collab::ids::NodeId(middle), 0);
    let before = serde_json::to_value(&room.model().session().doc).expect("json");

    let err = room
        .submit(
            "alice",
            Role::Editor,
            op(
                "a",
                1,
                0,
                OpKind::Command,
                first,
                0,
                json!({
                    "command": "para.alignCenter",
                    "params": {},
                    "selection": {"anchor": {"nodeId": first, "off": 0}, "focus": {"nodeId": last, "off": 2}},
                }),
            ),
        )
        .expect_err("a covered paragraph with no id cannot be undone");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(serde_json::to_value(&room.model().session().doc).expect("json"), before, "nothing changed");
    assert_eq!(room.version(), 0, "the room did not advance");
}

// --- moving paragraphs past their neighbour --------------------------------

fn paragraphs_doc(texts: &[&str]) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    for text in texts {
        doc.body.push(Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
    }
    doc
}

fn texts(room: &Room<SessionModel>) -> Vec<String> {
    room.model().session().doc.body.iter().filter_map(|block| block.as_para().map(|para| para.text.clone())).collect()
}

fn document_json(room: &Room<SessionModel>) -> serde_json::Value {
    serde_json::to_value(&room.model().session().doc).expect("json")
}

#[test]
fn a_paragraph_move_lands_as_one_structural_operation_and_undoes() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["one", "two", "three"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (first, second) = (node(0), node(1));

    // A replica learns the move and its undo only from the deltas.
    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    let command = op("a", 1, 0, OpKind::Command, second, 0, json!({"command": "para.moveUp", "params": {}}));
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the move lands");
    assert_eq!(
        applied.delta,
        Some(json!({"op": "move_block", "nodeId": second, "count": 1, "neighbour": first})),
        "the command lands as a structural operation, not as a command"
    );
    assert_eq!(texts(&room), ["two", "one", "three"]);

    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica applies the move");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica converged");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, second, &command)).expect("the undo lands");
    assert_eq!(texts(&room), ["one", "two", "three"], "the undo restores the order");
    crate::apply_delta(&mut replica, &mut ids, undone.delta.as_ref().expect("a delta")).expect("the replica undoes");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica undid too");

    let redo = Op {
        op_id: OpId::new("a", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", second, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: command.op_id.clone() }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(texts(&room), ["two", "one", "three"], "redo moves the paragraph again");
}

#[test]
fn a_move_command_at_the_top_of_a_story_is_refused_before_mutation() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["one", "two"])));
    let plan = room.join(None, false);
    let first = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;
    let before = document_json(&room);

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.moveUp", "params": {}})))
        .expect_err("the first paragraph cannot move up");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(document_json(&room), before, "the refusal changed nothing");
    assert_eq!(room.version(), 0, "the version did not move");
    assert_eq!(room.model().session().undo_depth(), 0, "no undo step was recorded");
}

#[test]
fn a_move_on_an_unknown_node_is_refused_before_mutation() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["one", "two"])));
    let plan = room.join(None, false);
    let first = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;
    let before = document_json(&room);

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, 4_242, 0, json!({"op": "move_block", "neighbour": first})))
        .expect_err("an unknown target is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(document_json(&room), before);
    assert_eq!(room.version(), 0);
}

#[test]
fn a_move_over_more_than_64_blocks_stays_an_undo_barrier() {
    let names: Vec<String> = (0..67).map(|index| format!("p{index}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&refs)));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (first, last) = (node(0), node(65));

    // 66 paragraphs move down past the 67th: wider than one captured move.
    let selection = json!({"anchor": {"nodeId": first, "off": 0}, "focus": {"nodeId": last, "off": 1}});
    let command = op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.moveDown", "params": {}, "selection": selection}));
    room.submit("alice", Role::Editor, command.clone()).expect("a wide move still lands");
    assert_eq!(texts(&room).first().map(String::as_str), Some("p66"), "the last paragraph crossed the run");

    let err = room.submit("alice", Role::Editor, undo_op(2, 1, first, &command)).expect_err("a step this wide has no inverse");
    assert_eq!(err.code, RejectCode::Unsupported);
}

// --- sorting a run of paragraphs --------------------------------------------

fn sort_command(first: u32, last: u32) -> Op {
    let selection = json!({"anchor": {"nodeId": first, "off": 0}, "focus": {"nodeId": last, "off": 1}});
    op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.sort", "params": {}, "selection": selection}))
}

#[test]
fn a_paragraph_sort_lands_as_one_structural_operation_and_undoes() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["gamma", "alpha", "beta"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (gamma, alpha, beta) = (node(0), node(1), node(2));

    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    let command = sort_command(gamma, beta);
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the sort lands");
    assert_eq!(texts(&room), ["alpha", "beta", "gamma"]);
    assert_eq!(
        applied.delta,
        Some(json!({"op": "transaction", "ops": [{"op": "permute", "nodeId": alpha, "order": [alpha, beta, gamma]}]})),
        "the sort lands as one permute inside the step's transaction"
    );
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica applies the sort");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica converged");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, gamma, &command)).expect("the undo lands");
    assert_eq!(texts(&room), ["gamma", "alpha", "beta"], "the undo restores the order");
    crate::apply_delta(&mut replica, &mut ids, undone.delta.as_ref().expect("a delta")).expect("the replica undoes");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica undid too");

    let redo = Op {
        op_id: OpId::new("a", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", gamma, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: command.op_id.clone() }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(texts(&room), ["alpha", "beta", "gamma"], "redo sorts again");
}

#[test]
fn a_sort_that_changes_nothing_is_still_an_addressable_step() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["alpha", "beta", "gamma"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (first, last) = (node(0), node(2));

    let command = sort_command(first, last);
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the sort lands");
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["op"], json!("transaction"), "the step is still one transaction");
    assert_eq!(delta["ops"][0]["op"], json!("permute"));
    assert_eq!(texts(&room), ["alpha", "beta", "gamma"]);

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, first, &command)).expect("the no-op undo lands");
    assert_eq!(undone.delta.as_ref().and_then(|delta| delta.get("op")), Some(&json!("transaction")));
    assert_eq!(texts(&room), ["alpha", "beta", "gamma"]);
}

#[test]
fn a_sort_over_more_than_64_blocks_stays_an_undo_barrier() {
    let names: Vec<String> = (0..67).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&refs)));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (first, last) = (node(0), node(66));

    // The run is already in order: the command changes nothing, but it is wider
    // than a permute may name, so the step stays a command barrier.
    let command = sort_command(first, last);
    room.submit("alice", Role::Editor, command.clone()).expect("a wide sort still lands");
    let err = room.submit("alice", Role::Editor, undo_op(2, 1, first, &command)).expect_err("a step this wide has no inverse");
    assert_eq!(err.code, RejectCode::Unsupported);
}

#[test]
fn a_permute_on_an_unknown_node_is_refused_before_mutation() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["one", "two"])));
    let plan = room.join(None, false);
    let first = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;
    let before = document_json(&room);

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, 4_242, 0, json!({"op": "permute", "order": [4_242, first]})))
        .expect_err("an unknown target is refused");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(document_json(&room), before);
    assert_eq!(room.version(), 0);
}

// --- promoting and demoting paragraphs --------------------------------------

/// A document whose paragraphs carry the styles named: `Some("Heading2")` is a
/// heading, `None` body text.
fn heading_doc(texts: &[(&str, Option<&str>)]) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    for (text, style) in texts {
        let mut para = Paragraph::with_text(text, CharProps::default());
        para.props.style = style.map(str::to_string);
        doc.body.push(Arc::new(Block::Para(para)));
    }
    doc
}

fn heading(room: &Room<SessionModel>, block: usize) -> Option<String> {
    room.model().session().doc.body.get(block).and_then(|b| b.as_para()).and_then(|p| p.props.style.clone())
}

fn selected(anchor: u32, focus: u32) -> serde_json::Value {
    json!({"anchor": {"nodeId": anchor, "off": 0}, "focus": {"nodeId": focus, "off": 1}})
}

#[test]
fn a_promote_lands_as_one_level_operation_and_undoes() {
    let mut room = Room::new(SessionModel::from_document(heading_doc(&[("Title", Some("Heading1")), ("Body", None)])));
    let plan = room.join(None, false);
    let body = plan.nodes.iter().find(|node| node.path == vec![1]).expect("a block").id;

    // A replica learns the step and its undo only from the deltas.
    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    let command = op("a", 1, 0, OpKind::Command, body, 0, json!({"command": "para.promote", "params": {}}));
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the promote lands");
    assert_eq!(heading(&room, 1).as_deref(), Some("Heading1"), "body text becomes a heading at the level above it");
    assert_eq!(
        applied.delta,
        Some(json!({"op": "transaction", "ops": [{"op": "set_para_level", "items": [{"id": body, "level": 1}]}]})),
        "the promote lands as one level operation inside the step's transaction"
    );
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica applies the level");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica converged");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, body, &command)).expect("the undo lands");
    let delta = undone.delta.as_ref().expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_level"), "the inverse names the earlier level");
    assert_eq!(heading(&room, 1), None, "the paragraph is body text again");
    crate::apply_delta(&mut replica, &mut ids, delta).expect("the replica undoes");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica undid too");

    let redo = Op {
        op_id: OpId::new("a", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", body, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: command.op_id.clone() }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(heading(&room, 1).as_deref(), Some("Heading1"), "redo promotes again");
}

#[test]
fn a_promote_over_a_selection_levels_every_paragraph_in_one_transaction() {
    let mut room = Room::new(SessionModel::from_document(heading_doc(&[("Title", Some("Heading1")), ("One", None), ("Two", None)])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (one, two) = (node(1), node(2));

    let command = op("a", 1, 0, OpKind::Command, one, 0, json!({"command": "para.promote", "params": {}, "selection": selected(one, two)}));
    let applied = room.submit("alice", Role::Editor, command).expect("the promote lands");
    assert_eq!(heading(&room, 1).as_deref(), Some("Heading1"));
    assert_eq!(heading(&room, 2).as_deref(), Some("Heading1"));
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(
        delta["ops"][0],
        json!({"op": "set_para_level", "items": [{"id": one, "level": 1}, {"id": two, "level": 1}]}),
        "every changed paragraph is named, in document order"
    );
}

#[test]
fn a_promote_over_more_than_64_paragraphs_stays_an_undo_barrier() {
    let names: Vec<String> = (0..65).map(|index| format!("p{index:02}")).collect();
    let mut texts: Vec<(&str, Option<&str>)> = vec![("Title", Some("Heading1"))];
    texts.extend(names.iter().map(|name| (name.as_str(), None)));
    let mut room = Room::new(SessionModel::from_document(heading_doc(&texts)));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (first, last) = (node(1), node(65));

    let command = op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.promote", "params": {}, "selection": selected(first, last)}));
    room.submit("alice", Role::Editor, command.clone()).expect("a wide promote still lands");
    assert_eq!(heading(&room, 65).as_deref(), Some("Heading1"), "the last paragraph was promoted too");

    let err = room.submit("alice", Role::Editor, undo_op(2, 1, first, &command)).expect_err("a step this wide has no inverse");
    assert_eq!(err.code, RejectCode::Unsupported);
}

#[test]
fn a_promote_with_nothing_to_promote_is_refused_before_mutation() {
    let mut room = Room::new(SessionModel::from_document(heading_doc(&[("Title", Some("Heading1"))])));
    let plan = room.join(None, false);
    let title = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;
    let before = document_json(&room);

    let err = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, title, 0, json!({"command": "para.promote", "params": {}})))
        .expect_err("Heading 1 cannot be promoted");
    assert_eq!(err.code, RejectCode::Invalid);
    assert_eq!(document_json(&room), before, "the refusal changed nothing");
    assert_eq!(room.version(), 0, "the version did not move");
    assert_eq!(room.model().session().undo_depth(), 0, "no undo step was recorded");
}

#[test]
fn a_level_step_that_changes_nothing_is_still_an_addressable_step() {
    let mut room = Room::new(SessionModel::from_document(heading_doc(&[("Body", None)])));
    let plan = room.join(None, false);
    let body = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;
    let before = document_json(&room);

    let step = op("a", 1, 0, OpKind::Structural, body, 0, json!({"op": "set_para_level", "items": [{"id": body, "level": null}]}));
    let applied = room.submit("alice", Role::Editor, step.clone()).expect("an identity level step still lands");
    assert_eq!(
        applied.delta,
        Some(json!({"op": "transaction", "ops": [{"op": "set_para_level", "items": [{"id": body, "level": null}]}]})),
        "the step is addressable even when it changes nothing"
    );
    assert_eq!(document_json(&room), before, "the document is unchanged");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, body, &step)).expect("the identity undo lands");
    assert!(undone.delta.is_some(), "the undo carries a delta too");
    assert_eq!(document_json(&room), before, "the undo changes nothing either");
}

#[test]
fn a_level_step_and_a_permute_land_as_one_transaction_on_a_replica() {
    let mut room = Room::new(SessionModel::from_document(heading_doc(&[("One", None), ("Two", Some("Heading2"))])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (one, two) = (node(0), node(1));
    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    // A command that levels and reorders in one step: both operations land or
    // neither does.
    let delta = json!({"op": "transaction", "ops": [
        {"op": "set_para_level", "items": [{"id": one, "level": 2}]},
        {"op": "permute", "nodeId": one, "order": [two, one]},
    ]});
    crate::apply_delta(&mut replica, &mut ids, &delta).expect("the replica applies the step");
    assert_eq!(replica.doc.body[1].as_para().and_then(|para| para.props.style.clone()).as_deref(), Some("Heading2"), "the levelled paragraph moved");
    assert_eq!(
        replica.doc.body.iter().filter_map(|block| block.as_para().map(|para| para.text.clone())).collect::<Vec<_>>(),
        ["Two", "One"],
        "the order changed too"
    );

    let before = serde_json::to_value(&replica.doc).expect("json");
    let refused = json!({"op": "transaction", "ops": [
        {"op": "set_para_level", "items": [{"id": one, "level": 1}]},
        {"op": "permute", "nodeId": one, "order": [one, 4_242]},
    ]});
    let err = crate::apply_delta(&mut replica, &mut ids, &refused).expect_err("the second operation is unknown");
    assert!(err.contains("unknown node"), "{err}");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), before, "a refused transaction changes nothing");
}

// --- bullets and numbering ---------------------------------------------------

fn para_numbering(session: &Session, block: usize) -> Option<wordcraft_doc::props::NumRef> {
    session.doc.body.get(block).and_then(|block| block.as_para()).and_then(|para| para.props.numbering)
}

#[test]
fn a_bullets_command_lands_as_one_list_operation_and_undoes() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One", "Two"])));
    let plan = room.join(None, false);
    let one = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;

    // A replica learns the step and its undo only from the deltas.
    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    let command = op("a", 1, 0, OpKind::Command, one, 0, json!({"command": "para.bullets", "params": {}}));
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the bullets land");
    let delta = applied.delta.clone().expect("the step carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_list"), "the forward effect is a list operation");
    assert_eq!(delta["ops"][0]["items"][0]["id"], json!(one));
    let num = delta["ops"][0]["items"][0]["num"].as_u64().expect("a list id");
    assert_eq!(delta["ops"][0]["define"][0]["num"]["id"], json!(num), "the definition the command allocated travels with the delta");
    assert_eq!(para_numbering(room.model().session(), 0), Some(wordcraft_doc::props::NumRef { num: num as u32, level: 0 }));
    crate::apply_delta(&mut replica, &mut ids, &delta).expect("the replica applies the list");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica converged");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, one, &command)).expect("the undo lands");
    let delta = undone.delta.as_ref().expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_list"), "the inverse names the earlier list state");
    assert_eq!(delta["ops"][0]["items"][0]["num"], serde_json::Value::Null);
    assert_eq!(delta["ops"][0]["undefine"], json!([num]));
    assert_eq!(para_numbering(room.model().session(), 0), None, "the paragraph is not a list item any more");
    crate::apply_delta(&mut replica, &mut ids, delta).expect("the replica undoes");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica undid too");

    let redo = Op {
        op_id: OpId::new("a", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", one, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: command.op_id.clone() }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(para_numbering(room.model().session(), 0), Some(wordcraft_doc::props::NumRef { num: num as u32, level: 0 }), "redo lists again");
}

#[test]
fn a_numbering_command_over_a_selection_lists_every_paragraph_in_one_transaction() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One", "Two"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (one, two) = (node(0), node(1));

    let command = op("a", 1, 0, OpKind::Command, one, 0, json!({"command": "para.numbering", "params": {}, "selection": selected(one, two)}));
    let applied = room.submit("alice", Role::Editor, command).expect("the numbering lands");
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_list"));
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "every selected paragraph is named, in document order");
    assert_eq!(items[0]["id"], json!(one));
    assert_eq!(items[1]["id"], json!(two));
    assert_eq!(items[0]["num"], items[1]["num"], "one list instance for the step");
    assert_eq!(items[0]["level"], json!(0));
}

#[test]
fn a_list_command_over_more_than_64_paragraphs_stays_an_undo_barrier() {
    let names: Vec<String> = (0..65).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&refs)));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (first, last) = (node(0), node(64));

    let command = op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.bullets", "params": {}, "selection": selected(first, last)}));
    room.submit("alice", Role::Editor, command.clone()).expect("a wide list step still lands");
    assert!(para_numbering(room.model().session(), 64).is_some(), "the last paragraph was listed too");

    let err = room.submit("alice", Role::Editor, undo_op(2, 1, first, &command)).expect_err("a step this wide has no inverse");
    assert_eq!(err.code, RejectCode::Unsupported);
}

#[test]
fn a_list_step_on_a_table_is_refused_before_mutation() {
    let mut doc = Document::new();
    doc.body.clear();
    let mut table = wordcraft_doc::Table::new(1, 1, 200.0);
    table.rows[0].cells[0] = wordcraft_doc::Cell::with_text("cell");
    doc.body.push(Arc::new(Block::Table(table)));
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let table_node = plan.nodes.iter().find(|node| node.path == vec![0]).expect("the table has an id").id;
    let cell_node = plan.nodes.iter().find(|node| node.path == vec![0, 0, 0, 0]).expect("the cell paragraph has an id").id;
    let before = document_json(&room);

    for id in [table_node, cell_node] {
        let err = room
            .submit(
                "alice",
                Role::Editor,
                op("a", 1, 0, OpKind::Structural, id, 0, json!({"op": "set_para_list", "ids": [id], "kind": "bullets", "level": 0})),
            )
            .expect_err("a table or a paragraph inside one has no list step");
        assert_eq!(err.code, RejectCode::Invalid);
        assert_eq!(document_json(&room), before, "the refusal changed nothing");
        assert_eq!(room.version(), 0, "the version did not move");
    }
}

#[test]
fn a_list_step_that_names_the_current_list_is_an_identity_step() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One"])));
    let plan = room.join(None, false);
    let one = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;
    room.submit(
        "alice",
        Role::Editor,
        op("a", 1, 0, OpKind::Structural, one, 0, json!({"op": "set_para_list", "ids": [one], "kind": "bullets", "level": 0})),
    )
    .expect("the first list step lands");
    let listed = document_json(&room);

    let step = op("a", 2, 1, OpKind::Structural, one, 0, json!({"op": "set_para_list", "ids": [one], "kind": "bullets", "level": 0}));
    let applied = room.submit("alice", Role::Editor, step.clone()).expect("an identity list step still lands");
    assert_eq!(applied.delta.as_ref().and_then(|delta| delta.get("op")).and_then(|op| op.as_str()), Some("transaction"));
    assert_eq!(document_json(&room), listed, "the document is unchanged");

    let undone = room.submit("alice", Role::Editor, undo_op(3, 2, one, &step)).expect("the identity undo lands");
    assert!(undone.delta.is_some(), "the undo carries a delta too");
    assert_eq!(document_json(&room), listed, "the undo changes nothing either");
}

#[test]
fn a_multilevel_command_lands_as_one_multilevel_operation_and_undoes() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One", "Two"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (one, two) = (node(0), node(1));

    // A replica learns the step and its undo only from the deltas.
    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    let command = op("a", 1, 0, OpKind::Command, one, 0, json!({"command": "para.multilevel", "params": {}, "selection": selected(one, two)}));
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the multilevel list lands");
    let delta = applied.delta.clone().expect("the step carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_multilevel"), "the forward effect keeps the multilevel op");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "every selected paragraph is named");
    let num = items[0]["num"].as_u64().expect("a list id");
    assert_eq!(items[1]["num"], json!(num), "both paragraphs join one definition");
    assert_eq!(delta["ops"][0]["define"][0]["abstract"]["levels"].as_array().expect("levels").len(), 9, "nine levels travel");
    crate::apply_delta(&mut replica, &mut ids, &delta).expect("the replica applies the multilevel list");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica converged");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, one, &command)).expect("the undo lands");
    let delta = undone.delta.as_ref().expect("the undo carries a delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_multilevel"), "the inverse keeps the multilevel op");
    assert_eq!(delta["ops"][0]["items"][0]["num"], serde_json::Value::Null);
    assert_eq!(delta["ops"][0]["undefine"], json!([num]), "the definition the step created is dropped");
    crate::apply_delta(&mut replica, &mut ids, delta).expect("the replica undoes");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica undid too");

    let redo = Op {
        op_id: OpId::new("a", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", one, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: command.op_id.clone() }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(para_numbering(room.model().session(), 0).map(|n| n.num), Some(u32::try_from(num).expect("a list id")), "redo lists again");
}

#[test]
fn a_restart_numbering_command_lands_as_one_restart_operation_and_undoes() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One", "Two"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (one, two) = (node(0), node(1));

    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    // The paragraphs are one numbered list first; the replica learns it from
    // the listing delta.
    let listing = op("a", 1, 0, OpKind::Command, one, 0, json!({"command": "para.numbering", "params": {}, "selection": selected(one, two)}));
    let applied = room.submit("alice", Role::Editor, listing).expect("the numbering lands");
    let delta = applied.delta.clone().expect("a delta");
    crate::apply_delta(&mut replica, &mut ids, &delta).expect("the replica applies the listing");
    let source = para_numbering(room.model().session(), 0).expect("a list").num;

    let command = op("a", 2, 1, OpKind::Command, one, 0, json!({"command": "para.restartNumbering", "params": {}}));
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the restart lands");
    let delta = applied.delta.clone().expect("the step carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_restart_numbering"), "the forward effect keeps the restart op");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    let num = items[0]["num"].as_u64().expect("a restarted list");
    assert_ne!(num, u64::from(source), "a fresh instance");
    assert_eq!(items[0]["level"], json!(0), "the level is kept");
    assert_eq!(delta["ops"][0]["define"][0]["num"]["startOverrides"][0][0], json!(0), "the restart names its level");
    crate::apply_delta(&mut replica, &mut ids, &delta).expect("the replica applies the restart");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica converged");

    let undone = room.submit("alice", Role::Editor, undo_op(3, 2, one, &command)).expect("the undo lands");
    let delta = undone.delta.as_ref().expect("the undo carries a delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_restart_numbering"), "the inverse keeps the restart op");
    assert_eq!(delta["ops"][0]["items"][0]["num"], json!(source), "the earlier instance is back");
    assert_eq!(delta["ops"][0]["undefine"], json!([num]), "the instance the step created is dropped");
    crate::apply_delta(&mut replica, &mut ids, delta).expect("the replica undoes");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica undid too");

    let redo = Op {
        op_id: OpId::new("a", 4),
        base_version: 3,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", one, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: command.op_id.clone() }),
    };
    room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(para_numbering(room.model().session(), 0).map(|n| n.num), Some(u32::try_from(num).expect("a list id")), "redo restarts again");
}

#[test]
fn a_multilevel_step_that_names_the_current_levels_is_an_identity_step() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One"])));
    let plan = room.join(None, false);
    let one = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;
    let payload = json!({"op": "set_para_multilevel", "kind": "legal", "levels": [{"id": one, "level": 0}]});
    room.submit("alice", Role::Editor, op("a", 1, 0, OpKind::Structural, one, 0, payload.clone())).expect("the first multilevel step lands");
    let listed = document_json(&room);
    let definitions = room.model().session().doc.numbering.nums.len();

    let applied =
        room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, payload.clone())).expect("an identity step still lands");
    assert_eq!(applied.delta.as_ref().and_then(|delta| delta.get("op")).and_then(|op| op.as_str()), Some("transaction"));
    assert_eq!(document_json(&room), listed, "the document is unchanged");
    assert_eq!(room.model().session().doc.numbering.nums.len(), definitions, "no second definition is allocated");

    let undone =
        room.submit("alice", Role::Editor, undo_op(3, 2, one, &op("a", 2, 1, OpKind::Structural, one, 0, payload))).expect("the identity undo lands");
    assert!(undone.delta.is_some(), "the undo carries a delta too");
    assert_eq!(document_json(&room), listed, "the undo changes nothing either");
}

#[test]
fn a_restart_step_that_names_the_current_start_is_an_identity_step() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One", "Two"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (one, two) = (node(0), node(1));
    room.submit(
        "alice",
        Role::Editor,
        op("a", 1, 0, OpKind::Structural, one, 0, json!({"op": "set_para_list", "ids": [one, two], "kind": "numbering", "level": 0})),
    )
    .expect("the listing lands");
    let payload = json!({"op": "set_para_restart_numbering", "ids": [one, two], "start": 4});
    room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, payload.clone())).expect("the first restart lands");
    let restarted = document_json(&room);
    let definitions = room.model().session().doc.numbering.nums.len();

    let applied = room.submit("alice", Role::Editor, op("a", 3, 2, OpKind::Structural, one, 0, payload)).expect("an identity restart still lands");
    assert_eq!(applied.delta.as_ref().and_then(|delta| delta.get("op")).and_then(|op| op.as_str()), Some("transaction"));
    assert_eq!(document_json(&room), restarted, "the document is unchanged");
    assert_eq!(room.model().session().doc.numbering.nums.len(), definitions, "no second instance is allocated");
}

#[test]
fn a_multilevel_command_over_more_than_64_paragraphs_stays_an_undo_barrier() {
    let names: Vec<String> = (0..65).map(|index| format!("p{index:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&refs)));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (first, last) = (node(0), node(64));

    let command = op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.multilevel", "params": {}, "selection": selected(first, last)}));
    room.submit("alice", Role::Editor, command.clone()).expect("a wide multilevel step still lands");
    assert!(para_numbering(room.model().session(), 64).is_some(), "the last paragraph was listed too");

    let err = room.submit("alice", Role::Editor, undo_op(2, 1, first, &command)).expect_err("a step this wide has no inverse");
    assert_eq!(err.code, RejectCode::Unsupported);
}

#[test]
fn a_restart_step_with_a_zero_start_is_refused_before_mutation() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One", "Two"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (one, two) = (node(0), node(1));
    room.submit(
        "alice",
        Role::Editor,
        op("a", 1, 0, OpKind::Structural, one, 0, json!({"op": "set_para_list", "ids": [one, two], "kind": "numbering", "level": 0})),
    )
    .expect("the listing lands");
    let before = document_json(&room);

    for payload in [
        json!({"op": "set_para_restart_numbering", "ids": [one], "start": 0}),
        json!({"op": "set_para_restart_numbering", "ids": [one], "start": -3}),
    ] {
        let err = room.submit("alice", Role::Editor, op("a", 2, 1, OpKind::Structural, one, 0, payload)).expect_err("a bad start is refused");
        assert_eq!(err.code, RejectCode::Invalid);
        assert_eq!(document_json(&room), before, "the refusal changed nothing");
        assert_eq!(room.version(), 1, "the version did not move");
    }
}

// --- deleting blank paragraphs ----------------------------------------------

#[test]
fn a_remove_blank_command_lands_as_one_blank_step_and_undoes() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One", "", "Two", "", "Three"])));
    let plan = room.join(None, false);
    let node = |path: u32| plan.nodes.iter().find(|node| node.path == vec![path]).expect("a block").id;
    let (first, blank1, blank2) = (node(0), node(1), node(3));

    // A replica learns the step and its undo only from the deltas.
    let mut replica = Session::new(room.model().session().doc.clone());
    let mut ids = NodeIds::new();
    SessionModel::from_document(replica.doc.clone()).index_nodes(&mut ids);

    let command = op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.removeBlank", "params": {"scope": "all"}}));
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the command lands");
    let delta = applied.delta.clone().expect("the step carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("remove_blank"), "the forward effect is a blank step");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "both empty paragraphs are named");
    assert_eq!(items[0], json!({"id": blank2, "index": 3}), "the highest index comes first");
    assert_eq!(items[1], json!({"id": blank1, "index": 1}));
    assert_eq!(texts(&room), ["One", "Two", "Three"]);
    crate::apply_delta(&mut replica, &mut ids, &delta).expect("the replica applies the step");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica converged");

    let undone = room.submit("alice", Role::Editor, undo_op(2, 1, first, &command)).expect("the undo lands");
    let delta = undone.delta.as_ref().expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("remove_blank"), "the inverse names the bodies it puts back");
    assert_eq!(delta["ops"][0]["restore"].as_array().expect("restore").len(), 2);
    assert_eq!(texts(&room), ["One", "", "Two", "", "Three"], "the paragraphs are back");
    crate::apply_delta(&mut replica, &mut ids, delta).expect("the replica undoes");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica undid too");
    for (path, wanted) in [(1, blank1), (3, blank2)] {
        assert_eq!(ids.get(&NodeKey::new("body", vec![path])).map(|id| id.0), Some(wanted), "the replica revived the id at its own path");
        assert_eq!(room.ids().get(&NodeKey::new("body", vec![path])).map(|id| id.0), Some(wanted), "the room revived it too");
    }

    let redo = Op {
        op_id: OpId::new("a", 3),
        base_version: 2,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", first, 0),
        payload: serde_json::Value::Null,
        history: Some(wordcraft_collab::wire::HistoryAction::Redo { op_id: command.op_id.clone() }),
    };
    let applied = room.submit("alice", Role::Editor, redo).expect("the redo lands");
    assert_eq!(texts(&room), ["One", "Two", "Three"], "redo removes the same paragraphs again");
    crate::apply_delta(&mut replica, &mut ids, applied.delta.as_ref().expect("a delta")).expect("the replica redoes");
    assert_eq!(serde_json::to_value(&replica.doc).expect("json"), document_json(&room), "the replica redid too");
}

#[test]
fn a_remove_blank_command_with_no_blank_paragraph_changes_nothing_and_is_no_barrier() {
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&["One", "Two"])));
    let plan = room.join(None, false);
    let first = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;
    let before = document_json(&room);

    let applied = room
        .submit("alice", Role::Editor, op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.removeBlank", "params": {"scope": "all"}})))
        .expect("the command lands");
    assert_eq!(document_json(&room), before, "the document did not change");
    assert!(applied.delta.is_none(), "there is nothing for a replica to apply");
    assert!(room.undo("alice").is_none(), "a no-op step is not a barrier either");
}

#[test]
fn a_remove_blank_command_over_more_than_64_paragraphs_stays_an_undo_barrier() {
    let mut texts: Vec<String> = Vec::new();
    for _ in 0..65 {
        texts.push("keep".to_string());
        texts.push(String::new());
    }
    texts.push("end".to_string());
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let mut room = Room::new(SessionModel::from_document(paragraphs_doc(&refs)));
    let plan = room.join(None, false);
    let first = plan.nodes.iter().find(|node| node.path == vec![0]).expect("a block").id;

    let command = op("a", 1, 0, OpKind::Command, first, 0, json!({"command": "para.removeBlank", "params": {"scope": "all"}}));
    let applied = room.submit("alice", Role::Editor, command.clone()).expect("the command lands");
    let delta = applied.delta.expect("a delta");
    assert_eq!(delta["op"], json!("command"), "a wider step keeps the command delta");

    let err = room.submit("alice", Role::Editor, undo_op(2, 1, first, &command)).expect_err("the undo is unsupported");
    assert_eq!(err.code, RejectCode::Unsupported);
}

#[test]
fn a_blank_step_on_a_table_is_refused_before_mutation() {
    // A body paragraph whose next block is a table cannot be removed (the
    // engine leaves it alone), and a paragraph inside a cell is not addressable.
    let mut doc = paragraphs_doc(&["One", ""]);
    doc.body.push(Arc::new(Block::Table(Table::new(1, 1, 200.0))));
    let mut room = Room::new(SessionModel::from_document(doc));
    let plan = room.join(None, false);
    let blank = plan.nodes.iter().find(|node| node.path == vec![1]).expect("a block").id;
    let cell = plan.nodes.iter().find(|node| node.path == vec![2, 0, 0, 0]).expect("a cell paragraph").id;

    for (seq, id) in [(1, blank), (2, cell)] {
        let before = document_json(&room);
        let version = room.version();
        let err = room
            .submit("alice", Role::Editor, op("a", seq, version, OpKind::Structural, 0, 0, json!({"op": "remove_blank", "ids": [id]})))
            .expect_err("the step is refused");
        assert_eq!(err.code, RejectCode::Invalid, "{}", err.message);
        assert_eq!(document_json(&room), before, "the refusal changed nothing");
        assert_eq!(room.version(), version, "the version did not move");
    }
}
