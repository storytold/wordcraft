//! Command operations through `Session::run`, and primitives through the
//! document adapter behind the same room.

use serde_json::json;

use std::sync::Arc;
use wordcraft_collab::ids::NodeIds;
use wordcraft_collab::room::{Model, Role, Room};
use wordcraft_collab::wire::{Op, OpId, OpKind, RejectCode, Target};

use wordcraft_doc::{Block, CharProps, Document, Paragraph, Path, StoryRef};
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
