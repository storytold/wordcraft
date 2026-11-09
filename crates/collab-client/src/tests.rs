//! Client state machine tests, including a round trip through a real room.

use serde_json::json;

use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{Applied, CabinetCommit, ClientMsg, HistoryAction, OpId, OpKind, Reject, RejectCode, ServerMsg, Target, Version};
use wordcraft_collab_doc::DocModel;
use wordcraft_doc::Document;

use crate::{Client, Event, State, session_client_id};

fn applied(op_id: OpId, version: Version, epoch: u64) -> ServerMsg {
    ServerMsg::Applied { applied: Applied { op_id, version, epoch, delta: None, conflict: false, history: None } }
}

/// Two windows on the same document must not share an op id space: with one id
/// the server treats one client's operations as the other's duplicates and the
/// participant list collapses to a single peer (found by the two-client host
/// test, docs/collab/STATUS.md §5.1).
#[test]
fn two_sessions_on_one_document_get_different_client_ids() {
    let first = session_client_id("web", 1);
    let second = session_client_id("web", 1);
    assert_ne!(first, second, "two clients must not share an op id space");
    assert!(first.starts_with("web-1-"), "{first}");
    assert!(session_client_id("ui", 7).starts_with("ui-7-"));
}

#[test]
fn operation_ids_are_per_client_and_queue_until_confirmed() {
    let mut client = Client::new("alice");
    let first = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "a"}));
    let second = client.op(OpKind::Primitive, Target::new("body", 1, 1), json!({"op": "insert_text", "text": "b"}));

    assert_eq!(first.op_id, OpId::new("alice", 1));
    assert_eq!(second.op_id, OpId::new("alice", 2));
    assert_eq!(client.pending().len(), 2);

    let event = client.on(applied(first.op_id.clone(), 1, 0));
    assert!(matches!(event, Event::Applied { .. }));
    assert_eq!(client.pending().len(), 1);
    assert_eq!(client.version(), 1);
    assert!(client.was_confirmed(&first.op_id));
}

#[test]
fn a_structural_operation_carries_the_epoch() {
    let mut client = Client::new("alice");
    let _ = client.op(OpKind::Structural, Target::new("body", 1, 0), json!({"op": "insert_block", "index": 1}));
    assert_eq!(client.pending()[0].op.epoch, Some(0));
    let _ = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "x"}));
    assert_eq!(client.pending()[1].op.epoch, None);
}

#[test]
fn welcome_with_resync_asks_for_a_snapshot_then_joins() {
    let mut client = Client::new("alice");
    let event = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: 41,
        epoch: 3,
        participants: Vec::new(),
        resync: true,
    });
    assert!(matches!(event, Event::Welcome { version: 41, resync: true, .. }));
    assert_eq!(client.state(), State::Resyncing);

    let event = client.on(ServerMsg::Snapshot { version: 41, bytes: b"{}".to_vec(), last: true });
    assert!(matches!(event, Event::Snapshot { last: true, .. }));
    assert_eq!(client.state(), State::Joined);
    assert_eq!(client.version(), 41);
}

#[test]
fn presence_carries_the_sender_role() {
    let mut client = Client::new("alice");
    let event = client.on(ServerMsg::Presence {
        client_id: "bob".into(),
        role: "viewer".into(),
        state: json!({"cursor": {"story": "body", "nodeId": 1, "off": 2}}),
    });
    match event {
        Event::Presence { client_id, role, state } => {
            assert_eq!(client_id, "bob");
            assert_eq!(role, "viewer");
            assert_eq!(state["cursor"]["off"], 2);
        }
        other => panic!("expected presence, got {other:?}"),
    }
}

#[test]
fn a_version_gap_asks_for_a_snapshot() {
    let mut client = Client::new("alice");
    let _ = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: 5,
        epoch: 0,
        participants: Vec::new(),
        resync: false,
    });
    let event = client.on(applied(OpId::new("bob", 1), 7, 0));
    match event {
        Event::Resync { reason } => assert!(reason.contains("gap"), "{reason}"),
        other => panic!("expected a resync, got {other:?}"),
    }
    assert_eq!(client.state(), State::Resyncing);
}

#[test]
fn a_stale_rejection_asks_for_a_resync() {
    let mut client = Client::new("alice");
    let _ = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: 7,
        epoch: 0,
        participants: Vec::new(),
        resync: false,
    });
    let op = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "x"}));
    let reject = Reject::new(op.op_id, RejectCode::Stale, "baseVersion was compacted");
    let event = client.on(ServerMsg::Rejected { reject });
    assert!(matches!(event, Event::Resync { .. }));
    assert_eq!(client.state(), State::Resyncing);
}

#[test]
fn a_non_retryable_rejection_is_reported_and_keeps_the_queue() {
    let mut client = Client::new("alice");
    let _ = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: 7,
        epoch: 0,
        participants: Vec::new(),
        resync: false,
    });
    let op = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "x"}));
    let reject = Reject::new(op.op_id, RejectCode::Forbidden, "viewer");
    let event = client.on(ServerMsg::Rejected { reject });
    assert!(matches!(event, Event::Rejected { .. }));
    assert_eq!(client.pending().len(), 1, "the app decides what to do with an unconfirmed operation");
}

#[test]
fn unconfirmed_operations_resend_with_the_same_ids() {
    let mut client = Client::new("alice");
    let op = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "x"}));
    let resend = client.unconfirmed();
    match &resend[0] {
        ClientMsg::Op { op: resent } => assert_eq!(resent.op_id, op.op_id),
        other => panic!("expected an op, got {other:?}"),
    }
}

#[test]
fn undo_and_redo_move_the_client_stack_only_when_confirmed() {
    let mut client = Client::new("alice");
    let _ = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: 0,
        epoch: 0,
        participants: Vec::new(),
        resync: false,
    });
    let op = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "a"}));
    let _ = client.on(applied(op.op_id.clone(), 1, 0));
    assert_eq!(client.history_depth(), (1, 0));

    let undo = client.undo().expect("a step");
    assert_eq!(undo.history, Some(HistoryAction::Undo { op_id: op.op_id.clone() }));
    assert!(undo.payload.is_null());
    assert_eq!(client.history_depth(), (1, 0), "nothing moves before the server answers");

    let _ = client.on(applied(undo.op_id, 2, 0));
    assert_eq!(client.history_depth(), (0, 1));

    // A refused redo leaves the step where it was.
    let refused = client.redo().expect("a redo step");
    let reject = Reject::new(refused.op_id.clone(), RejectCode::UndoConflict, "touched later");
    let _ = client.on(ServerMsg::Rejected { reject });
    assert_eq!(client.history_depth(), (0, 1));

    let redo = client.redo().expect("a redo step");
    assert_eq!(redo.history, Some(HistoryAction::Redo { op_id: op.op_id }));
    let _ = client.on(applied(redo.op_id, 3, 0));
    assert_eq!(client.history_depth(), (1, 0));
}

#[test]
fn a_new_local_edit_clears_the_redo_branch() {
    let mut client = Client::new("alice");
    let _ = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: 0,
        epoch: 0,
        participants: Vec::new(),
        resync: false,
    });
    let first = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "a"}));
    let _ = client.on(applied(first.op_id.clone(), 1, 0));
    let undo = client.undo().expect("a step");
    let _ = client.on(applied(undo.op_id, 2, 0));
    assert_eq!(client.history_depth(), (0, 1));

    let second = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "b"}));
    let _ = client.on(applied(second.op_id, 3, 0));
    assert_eq!(client.history_depth(), (1, 0), "a new edit drops redo");
    assert!(client.redo().is_none());
}

#[test]
fn undo_round_trips_through_the_room() {
    let mut room: Room<DocModel> = Room::new(DocModel::new(Document::new()));
    let plan = room.join(None, false);
    let node = plan.nodes.first().expect("a body node").id;
    let mut client = Client::new("alice");
    let _ = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: plan.version,
        epoch: plan.epoch,
        participants: Vec::new(),
        resync: false,
    });
    let text_of = |room: &Room<DocModel>| -> String {
        room.model().document().body.first().and_then(|block| block.as_para()).map(|para| para.text.clone()).unwrap_or_default()
    };

    let op = client.op(OpKind::Primitive, Target::new("body", node, 0), json!({"op": "insert_text", "text": "Hello"}));
    let applied = room.submit("alice", Role::Editor, op).expect("insert");
    let _ = client.on(ServerMsg::Applied { applied });
    assert_eq!(text_of(&room), "Hello");

    let undo = client.undo().expect("a step");
    let applied = room.submit("alice", Role::Editor, undo).expect("undo");
    assert_eq!(applied.version, 2);
    let _ = client.on(ServerMsg::Applied { applied });
    assert_eq!(text_of(&room), "");

    let redo = client.redo().expect("a step");
    let applied = room.submit("alice", Role::Editor, redo).expect("redo");
    assert_eq!(applied.version, 3);
    let _ = client.on(ServerMsg::Applied { applied });
    assert_eq!(text_of(&room), "Hello");
}

#[test]
fn a_round_trip_through_a_room_applies_the_operation() {
    let mut room: Room<DocModel> = Room::new(DocModel::new(Document::new()));
    let plan = room.join(None, false);
    let node = plan.nodes.first().expect("a body node").id;

    let mut client = Client::new("alice");
    let _ = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: plan.version,
        epoch: plan.epoch,
        participants: Vec::new(),
        resync: false,
    });
    let op = client.op(OpKind::Primitive, Target::new("body", node, 0), json!({"op": "insert_text", "text": "Hello"}));
    let applied = room.submit("alice", Role::Editor, op).expect("the room accepts");
    let event = client.on(ServerMsg::Applied { applied });
    assert!(matches!(event, Event::Applied { .. }));
    assert!(client.pending().is_empty());

    let text = room.model().document().body.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(text, "Hello");
}

/// The cabinet's copy of a commit (M87) travels in `committed` when the room
/// published the file itself; an older server's answer, without the field,
/// still parses as no cabinet copy.
#[test]
fn a_commit_answer_carries_the_cabinet_copy_when_the_room_published_it() {
    let older: ServerMsg = serde_json::from_value(json!({"type": "committed", "version": 3, "sha256": "ab", "warnings": []})).expect("an older wire");
    let event = Client::new("alice").on(older);
    match event {
        Event::Committed { version, cabinet, .. } => {
            assert_eq!(version, 3);
            assert_eq!(cabinet, None, "the server only built the file");
        }
        other => panic!("expected committed, got {other:?}"),
    }

    let published: ServerMsg = serde_json::from_value(json!({
        "type": "committed",
        "version": 4,
        "sha256": "cd",
        "warnings": [],
        "cabinet": {"version": 1794186677u64, "etag": "1794186677"},
    }))
    .expect("the cabinet fields");
    let event = Client::new("alice").on(published);
    match event {
        Event::Committed { version, cabinet, .. } => {
            assert_eq!(version, 4);
            assert_eq!(cabinet, Some(CabinetCommit { version: 1_794_186_677, etag: "1794186677".into() }));
        }
        other => panic!("expected committed, got {other:?}"),
    }
}
