//! Client state machine tests, including a round trip through a real room.

use serde_json::json;

use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{Applied, ClientMsg, OpId, OpKind, Reject, RejectCode, ServerMsg, Target, Version};
use wordcraft_collab_doc::DocModel;
use wordcraft_doc::Document;

use crate::{Client, Event, State};

fn applied(op_id: OpId, version: Version, epoch: u64) -> ServerMsg {
    ServerMsg::Applied { applied: Applied { op_id, version, epoch, delta: None, conflict: false } }
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
