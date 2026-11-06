//! Command operations through `Session::run`, and primitives through the
//! document adapter behind the same room.

use serde_json::json;

use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{Op, OpId, OpKind, RejectCode, Target};
use wordcraft_doc::Document;

use crate::SessionModel;

fn room() -> Room<SessionModel> {
    Room::new(SessionModel::from_document(Document::new()))
}

fn node(room: &mut Room<SessionModel>) -> u32 {
    room.join(None, false).nodes.first().expect("a body node").id
}

fn op(client: &str, seq: u64, base: u64, kind: OpKind, node: u32, off: usize, payload: serde_json::Value) -> Op {
    Op { op_id: OpId::new(client, seq), base_version: base, epoch: None, kind, target: Target::new("body", node, off), payload }
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
