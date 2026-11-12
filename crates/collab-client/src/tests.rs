//! Client state machine tests, including a round trip through a real room.

use serde_json::json;

use wordcraft_collab::DedupWindow;
use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{Applied, CabinetCommit, ClientMsg, HistoryAction, OpId, OpKind, Reject, RejectCode, ServerMsg, Target, Version};
use wordcraft_collab_doc::DocModel;
use wordcraft_doc::Document;

use crate::{Client, Event, State, session_client_id};

fn applied(op_id: OpId, version: Version, epoch: u64) -> ServerMsg {
    ServerMsg::Applied { applied: Applied { op_id, version, epoch, delta: None, nodes: None, conflict: false, history: None } }
}

fn welcome(version: Version, resync: bool) -> ServerMsg {
    ServerMsg::Welcome { session_id: "s".into(), role: "editor".into(), version, epoch: 0, participants: Vec::new(), resync }
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
fn a_replayed_answer_for_a_confirmed_operation_is_not_applied_twice() {
    let mut client = Client::new("alice");
    let _ = client.on(ServerMsg::Welcome {
        session_id: "s".into(),
        role: "editor".into(),
        version: 0,
        epoch: 0,
        participants: Vec::new(),
        resync: false,
    });
    // The room re-broadcasts the cached answer when the author retries an
    // operation it already applied. A peer that has it confirmed must ignore
    // the replay instead of applying its delta a second time.
    let answer = applied(OpId::new("bob", 1), 1, 0);
    assert!(matches!(client.on(answer.clone()), Event::Applied { .. }));
    assert!(matches!(client.on(answer), Event::None));
    assert_eq!(client.version(), 1);
    assert!(client.was_confirmed(&OpId::new("bob", 1)));
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
fn a_refused_stale_edit_is_not_resent_after_the_resync() {
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
    let reject = Reject::new(op.op_id.clone(), RejectCode::Stale, "the paragraph changed since this edit was made; resync");
    assert!(matches!(client.on(ServerMsg::Rejected { reject }), Event::Resync { .. }));
    assert_eq!(client.pending().len(), 0, "the same offsets are what the refusal is about");
    assert!(client.unconfirmed().is_empty(), "a reconnect must not replay it");

    // A lease refusal was never examined, so its operation waits for the retry.
    let op = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "y"}));
    let reject = Reject::new(op.op_id, RejectCode::Lease, "another session holds the file");
    let _ = client.on(ServerMsg::Rejected { reject });
    assert_eq!(client.pending().len(), 1, "the reconnect resends it");
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

/// A `409` from the cabinet is its own event: the panel uses it to offer the
/// room's file as a copy, while every replica still resyncs (TASK.md §6).
#[test]
fn a_cabinet_conflict_is_a_separate_event_and_resyncs() {
    let mut client = Client::new("alice");
    let event =
        client.on(ServerMsg::Error { code: "cabinetConflict".into(), message: "the file changed outside the session".into(), retryable: true });
    match event {
        Event::Conflict { reason } => assert!(reason.contains("changed outside"), "{reason}"),
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(client.state(), State::Resyncing, "the stale base must be replaced");
}

/// A conflict frame still reaches an app that has no copy support: a plain
/// retryable error keeps its old resync meaning.
#[test]
fn any_other_error_still_resyncs() {
    let mut client = Client::new("alice");
    let event = client.on(ServerMsg::Error { code: "lease".into(), message: "held".into(), retryable: true });
    assert!(matches!(event, Event::Resync { .. }), "got {event:?}");
    assert_eq!(client.state(), State::Resyncing);
}

/// The room's refused file arrives chunked, and the owner can ask for it.
#[test]
fn a_save_copy_chunk_parses_and_the_request_names_an_operation() {
    let message: ServerMsg = serde_json::from_value(json!({
        "type": "saveCopy",
        "version": 6,
        "sha256": "ab",
        "warnings": ["kept"],
        "bytes": [80, 75],
        "last": true,
    }))
    .expect("the save-copy frame");
    match Client::new("alice").on(message) {
        Event::SaveCopy { version, sha256, warnings, bytes, last } => {
            assert_eq!(version, 6);
            assert_eq!(sha256, "ab");
            assert_eq!(warnings, vec!["kept".to_string()]);
            assert_eq!(bytes, [80, 75]);
            assert!(last);
        }
        other => panic!("expected a copy chunk, got {other:?}"),
    }

    let mut client = Client::new("alice");
    let request = client.save_copy();
    match request {
        ClientMsg::SaveCopy { op_id } => assert_eq!(op_id, OpId::new("alice", 1)),
        other => panic!("expected saveCopy, got {other:?}"),
    }
}

/// The document a joining client offers travels in chunks inside one frame
/// each, and the server is told which one ends the stream (ADR WC-0004).
#[test]
fn seed_frames_split_the_document_and_mark_exactly_one_last_chunk() {
    let model = vec![b'x'; crate::SEED_CHUNK * 2 + 7];
    let frames = crate::seed_frames(&model);
    assert_eq!(frames.len(), 3);
    let mut rebuilt = Vec::new();
    let mut lasts = 0;
    for frame in &frames {
        match frame {
            ClientMsg::Seed { bytes, last } => {
                assert!(!bytes.is_empty(), "a chunk is never empty");
                assert!(bytes.len() <= crate::SEED_CHUNK);
                rebuilt.extend_from_slice(bytes);
                if *last {
                    lasts += 1;
                }
            }
            other => panic!("expected a seed frame, got {other:?}"),
        }
    }
    assert_eq!(rebuilt, model);
    assert_eq!(lasts, 1, "one frame ends the stream");

    // An empty document still sends its last frame: the server waits for it.
    assert!(matches!(crate::seed_frames(b"").as_slice(), [ClientMsg::Seed { bytes, last: true }] if bytes.is_empty()));
}

/// A server restored from an older snapshot answers a welcome whose version is
/// **below** the base this replica had. The base follows the head and the
/// queue built above it is reported instead of resent: the old
/// `version.max(...)` left the base above the head, so every operation after
/// the restore was refused `Stale` (review 5853542478; ADR WC-0008).
#[test]
fn a_welcome_below_the_base_lowers_it_and_reports_the_queue_it_drops() {
    let mut client = Client::new("alice");
    let _ = client.on(welcome(7, false));
    let first = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "a"}));
    let second = client.op(OpKind::Primitive, Target::new("body", 1, 1), json!({"op": "insert_text", "text": "b"}));

    match client.on(welcome(3, true)) {
        Event::Welcome { version, dropped, .. } => {
            assert_eq!(version, 3);
            assert_eq!(dropped, vec![first.op_id.clone(), second.op_id.clone()], "the app is told which edits the restore took");
        }
        other => panic!("expected a welcome, got {other:?}"),
    }
    assert_eq!(client.version(), 3, "the base is the room's head, not a maximum");
    assert!(client.pending().is_empty());
    assert!(client.unconfirmed().is_empty(), "a restore must not replay an edit the document no longer has");
    assert_eq!(client.state(), State::Resyncing, "the snapshot that follows replaces the document");
}

/// The acceptance of the rule: after a restore the client's next operation
/// lands. Resending the queued one with the base it was built on is exactly
/// what the room refuses `Stale`, and that refusal is the loop this removes.
#[test]
fn a_restored_room_does_not_refuse_the_next_operation_stale() {
    let mut room: Room<DocModel> = Room::new(DocModel::new(Document::new()));
    let plan = room.join(None, false);
    let node = plan.nodes.first().expect("a body node").id;
    let mut client = Client::new("alice");
    let _ = client.on(welcome(plan.version, false));
    let first = client.op(OpKind::Primitive, Target::new("body", node, 0), json!({"op": "insert_text", "text": "Hello"}));
    let applied = room.submit("alice", Role::Editor, first).expect("insert");
    let _ = client.on(ServerMsg::Applied { applied });

    // The edit that was in flight when the room went down.
    let in_flight = client.op(OpKind::Primitive, Target::new("body", node, 5), json!({"op": "insert_text", "text": "!"}));

    // The room comes back from a snapshot taken before either edit.
    let mut restored: Room<DocModel> = Room::restore(DocModel::new(Document::new()), 0, 0, DedupWindow::default());
    let restored_plan = restored.join(None, false);
    assert_eq!(restored_plan.version, 0);
    match client.on(welcome(restored.version(), true)) {
        Event::Welcome { version: 0, dropped, .. } => assert_eq!(dropped, vec![in_flight.op_id.clone()]),
        other => panic!("expected a welcome, got {other:?}"),
    }
    assert_eq!(client.version(), 0, "the base is the restored head");

    // What the old client did on reconnect: resend the queued operation with
    // the base it was built on.
    let refused = restored.submit("alice", Role::Editor, in_flight).expect_err("base above the head");
    assert_eq!(refused.code, RejectCode::Stale, "the refusal the restore used to produce");

    // The new client has nothing to resend, and what it sends now is accepted.
    assert!(client.unconfirmed().is_empty());
    let next = client.op(OpKind::Primitive, Target::new("body", node, 0), json!({"op": "insert_text", "text": "Hello"}));
    let applied = restored.submit("alice", Role::Editor, next).expect("the new base is the head");
    assert_eq!(applied.version, 1);
    let _ = client.on(ServerMsg::Applied { applied });
    assert_eq!(client.version(), restored.version(), "the replica is back on the room's head");
}

/// A restore takes the `opId` window with it: answers at or below the head
/// stay deduplicated, the ones above it describe state the room no longer has,
/// and `seq` keeps counting so no id is ever reused for a different edit.
#[test]
fn a_restore_keeps_the_dedup_window_below_the_head_and_the_id_counter() {
    let mut client = Client::new("alice");
    let _ = client.on(welcome(0, false));
    let kept = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "a"}));
    let _ = client.on(applied(kept.op_id.clone(), 1, 0));
    let lost = client.op(OpKind::Primitive, Target::new("body", 1, 1), json!({"op": "insert_text", "text": "b"}));
    let _ = client.on(applied(lost.op_id.clone(), 2, 0));
    assert_eq!(client.history_depth(), (2, 0));

    let _ = client.on(welcome(1, true));
    assert!(client.was_confirmed(&kept.op_id), "an answer the restored room still holds stays deduplicated");
    assert!(!client.was_confirmed(&lost.op_id), "an answer above the head is not in the window the snapshot ships");
    assert_eq!(client.history_depth(), (1, 0), "the step that is gone from the room leaves the stack");

    let next = client.op(OpKind::Primitive, Target::new("body", 1, 1), json!({"op": "insert_text", "text": "c"}));
    assert_eq!(next.op_id, OpId::new("alice", 3), "the counter never rewinds: an id names one edit for the session");
    assert_eq!(next.base_version, 1);
}

/// The snapshot carries the room's state, so the base follows it downwards
/// too; the queue it replaces is reconciled the same way, and the welcome that
/// announced the restore is where the drop is reported.
#[test]
fn a_snapshot_below_the_base_lowers_it_and_clears_the_queue() {
    let mut client = Client::new("alice");
    let _ = client.on(welcome(9, false));
    let op = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "a"}));

    let event = client.on(ServerMsg::Snapshot { version: 4, bytes: b"{}".to_vec(), last: true });
    assert!(matches!(event, Event::Snapshot { version: 4, last: true, .. }));
    assert_eq!(client.version(), 4, "the snapshot is the document's version");
    assert!(client.pending().is_empty(), "the queue was built on a document that is gone");
    assert_eq!(client.state(), State::Joined);
    assert!(client.unconfirmed().is_empty());
    assert!(!client.was_confirmed(&op.op_id));
}

/// A rollback the server does not follow with a snapshot cannot be joined: the
/// client asks for the document instead of reporting a base the room cannot
/// address.
#[test]
fn a_rollback_without_a_snapshot_asks_for_one() {
    let mut client = Client::new("alice");
    let _ = client.on(welcome(5, false));
    let _ = client.op(OpKind::Primitive, Target::new("body", 1, 0), json!({"op": "insert_text", "text": "a"}));

    match client.on(welcome(2, false)) {
        Event::Resync { reason } => assert!(reason.contains("below"), "{reason}"),
        other => panic!("expected a resync, got {other:?}"),
    }
    assert_eq!(client.version(), 2);
    assert!(client.pending().is_empty());
    assert_eq!(client.state(), State::Resyncing);
}

/// Two replicas and a room restored to an older snapshot: each lowers its base
/// to the head, the answers they exchange land without a gap, and both end on
/// the room's head with an empty queue.
#[test]
fn two_clients_converge_after_the_room_is_restored() {
    let mut room: Room<DocModel> = Room::new(DocModel::new(Document::new()));
    let plan = room.join(None, false);
    let node = plan.nodes.first().expect("a body node").id;
    let mut alice = Client::new("alice");
    let mut bob = Client::new("bob");
    let _ = alice.on(welcome(0, false));
    let _ = bob.on(welcome(0, false));

    let first = alice.op(OpKind::Primitive, Target::new("body", node, 0), json!({"op": "insert_text", "text": "A"}));
    let applied = room.submit("alice", Role::Editor, first).expect("insert");
    // The state the snapshot was taken at: after Alice's first edit.
    let at_version_one = DocModel::new(room.model().document().clone());
    let _ = alice.on(ServerMsg::Applied { applied: applied.clone() });
    let _ = bob.on(ServerMsg::Applied { applied });

    let second = bob.op(OpKind::Primitive, Target::new("body", node, 1), json!({"op": "insert_text", "text": "B"}));
    let applied = room.submit("bob", Role::Editor, second).expect("insert");
    let _ = alice.on(ServerMsg::Applied { applied: applied.clone() });
    let _ = bob.on(ServerMsg::Applied { applied });
    let third = alice.op(OpKind::Primitive, Target::new("body", node, 2), json!({"op": "insert_text", "text": "C"}));
    let applied = room.submit("alice", Role::Editor, third).expect("insert");
    let _ = alice.on(ServerMsg::Applied { applied: applied.clone() });
    let _ = bob.on(ServerMsg::Applied { applied });
    assert_eq!((alice.version(), bob.version()), (3, 3));

    // The room comes back from the snapshot taken at version 1.
    let mut restored: Room<DocModel> = Room::restore(at_version_one, 1, 0, DedupWindow::default());
    let plan = restored.join(None, false);
    assert_eq!(plan.version, 1);
    for client in [&mut alice, &mut bob] {
        match client.on(welcome(plan.version, true)) {
            Event::Welcome { version: 1, dropped, .. } => assert!(dropped.is_empty(), "nothing was in flight"),
            other => panic!("expected a welcome, got {other:?}"),
        }
        assert_eq!(client.version(), 1);
    }

    // Alice writes from the restored head, Bob receives the answer, then writes
    // himself: no `Stale`, no gap, and both end on the room's head.
    let from_alice = alice.op(OpKind::Primitive, Target::new("body", node, 1), json!({"op": "insert_text", "text": "D"}));
    let applied = restored.submit("alice", Role::Editor, from_alice).expect("the lowered base is the head");
    assert_eq!(applied.version, 2);
    assert!(matches!(alice.on(ServerMsg::Applied { applied: applied.clone() }), Event::Applied { .. }));
    assert!(matches!(bob.on(ServerMsg::Applied { applied: applied.clone() }), Event::Applied { .. }));
    let from_bob = bob.op(OpKind::Primitive, Target::new("body", node, 2), json!({"op": "insert_text", "text": "E"}));
    let applied = restored.submit("bob", Role::Editor, from_bob).expect("bob is on the head too");
    assert_eq!(applied.version, 3);
    assert!(matches!(bob.on(ServerMsg::Applied { applied: applied.clone() }), Event::Applied { .. }));
    assert!(matches!(alice.on(ServerMsg::Applied { applied }), Event::Applied { .. }));

    assert_eq!(restored.version(), 3);
    assert_eq!((alice.version(), bob.version()), (3, 3), "both replicas end on the room's head");
    assert!(alice.pending().is_empty() && bob.pending().is_empty());
}
