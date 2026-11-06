#![allow(clippy::unwrap_used)]

use std::time::Duration;

use super::colour;
use super::{
    AgentHub, HELD, INVALID_PARAMS, LIMIT_EXCEEDED, Limits, NO_REQUEST, NOT_OWNER, OUTSIDE_SELECTION, READ_ONLY_REQUEST, Reply, Request,
    STALE_REQUEST, Status, Target,
};
use serde_json::{Value, json};

fn target(revision: u64) -> Target {
    Target { document: "document-1".into(), revision }
}

fn request(text: &str) -> Request {
    Request {
        id: 0,
        instruction: text.into(),
        history: vec![],
        selection: json!({"first": 2, "last": 4}),
        document: json!({}),
        target: target(1),
        agent: None,
    }
}

fn take(hub: &AgentHub, conn: u64) -> Request {
    hub.listen(conn, None, Duration::ZERO, || true).unwrap().unwrap()
}

fn answer(text: &str) -> Reply<'_> {
    Reply { text, ..Reply::default() }
}

#[test]
/// Checks that queue is fifo and a request is assigned only once.
fn queue_is_fifo_and_a_request_is_assigned_only_once() {
    let hub = AgentHub::default();
    let first = hub.ask(request("First")).unwrap();
    let second = hub.ask(request("Second")).unwrap();
    assert_eq!(take(&hub, 1).id, first);
    assert_eq!(take(&hub, 2).id, second);
    assert!(hub.listen(3, None, Duration::ZERO, || true).unwrap().is_none());
    hub.reply(1, first, answer("Done"), Some(&target(1))).unwrap();
    assert_eq!(hub.snapshot(first).unwrap().answer.as_deref(), Some("Done"));
}

#[test]
/// Checks that cancelling queued or working requests never resurrects them.
fn cancelling_queued_or_working_requests_never_resurrects_them() {
    let hub = AgentHub::default();
    let queued = hub.ask(request("Cancel before listen")).unwrap();
    hub.close(queued);
    assert!(hub.listen(1, None, Duration::ZERO, || true).unwrap().is_none());
    let working = hub.ask(request("Cancel after listen")).unwrap();
    take(&hub, 1);
    hub.close(working);
    assert_eq!(hub.reply(1, working, answer("Late answer"), Some(&target(1))).unwrap_err().code, NO_REQUEST);
    hub.disconnect(1);
    assert!(hub.listen(2, None, Duration::ZERO, || true).unwrap().is_none());
    assert!(hub.snapshot(working).is_none());
}

#[test]
/// Checks that only the owner may edit inside the captured selection.
fn only_the_owner_may_edit_inside_the_captured_selection() {
    let hub = AgentHub::default();
    let id = hub.ask(request("Change cells 2 through 4")).unwrap();
    take(&hub, 1);
    assert_eq!(hub.may_edit(1, None, &target(1), |_| true).unwrap_err().code, HELD);
    assert_eq!(hub.may_edit(2, Some(id), &target(1), |_| true).unwrap_err().code, NOT_OWNER);
    let proposed_cell = 5;
    let result = hub.may_edit(1, Some(id), &target(1), |selection| {
        selection["first"].as_u64().unwrap() <= proposed_cell && proposed_cell <= selection["last"].as_u64().unwrap()
    });
    assert_eq!(result.unwrap_err().code, OUTSIDE_SELECTION);
    assert_eq!(hub.snapshot(id).unwrap().status, Status::Working { conn: 1 });
    hub.may_edit(1, Some(id), &target(1), |selection| selection["first"] == 2).unwrap();
    hub.reply(1, id, answer("Done"), Some(&target(1))).unwrap();
    assert_eq!(hub.may_edit(1, Some(id), &target(1), |_| true).unwrap_err().code, READ_ONLY_REQUEST);
    assert!(hub.held().is_empty());
}

#[test]
/// Checks that a request cannot edit a different document.
fn a_request_cannot_edit_a_different_document() {
    let hub = AgentHub::default();
    let id = hub.ask(request("Stay in document 1")).unwrap();
    take(&hub, 1);
    let other = Target { document: "document-2".into(), revision: 1 };
    assert_eq!(hub.may_edit(1, Some(id), &other, |_| true).unwrap_err().code, STALE_REQUEST);
    assert_eq!(hub.snapshot(id).unwrap().status, Status::Failed);
}

#[test]
/// Checks that changed or closed documents drop late replies and release holds.
fn changed_or_closed_documents_drop_late_replies_and_release_holds() {
    for current in [None, Some(target(2))] {
        let hub = AgentHub::default();
        let id = hub.ask(request("Old document contents")).unwrap();
        take(&hub, 1);
        assert_eq!(hub.reply(1, id, answer("Must not appear"), current.as_ref()).unwrap_err().code, STALE_REQUEST);
        let snapshot = hub.snapshot(id).unwrap();
        assert_eq!(snapshot.status, Status::Failed);
        assert!(snapshot.answer.is_none());
        assert!(snapshot.notes.is_empty());
        assert!(hub.held().is_empty());
        hub.disconnect(1);
        assert!(hub.listen(2, None, Duration::ZERO, || true).unwrap().is_none());
    }
}

#[test]
/// Checks that invalidation cancels queued and working requests but not other documents.
fn invalidation_cancels_queued_and_working_requests_but_not_other_documents() {
    let hub = AgentHub::default();
    let working = hub.ask(request("Working")).unwrap();
    let queued = hub.ask(request("Queued")).unwrap();
    take(&hub, 1);
    let mut other = request("Other document");
    other.target.document = "document-2".into();
    let other = hub.ask(other).unwrap();
    hub.invalidate(&target(2));
    for id in [working, queued] {
        assert_eq!(hub.snapshot(id).unwrap().status, Status::Failed);
    }
    assert_eq!(take(&hub, 2).id, other);
    assert_eq!(hub.reply(1, working, answer("Stale"), Some(&target(2))).unwrap_err().code, READ_ONLY_REQUEST);
}

#[test]
/// Checks that an authorized edit advances its request and invalidates other old work.
fn an_authorized_edit_advances_its_request_and_invalidates_other_old_work() {
    let hub = AgentHub::default();
    let own = hub.ask(request("Edit")).unwrap();
    let other = hub.ask(request("Other old request")).unwrap();
    take(&hub, 1);
    hub.may_edit(1, Some(own), &target(1), |_| true).unwrap();
    hub.advance(1, own, &target(1), target(2), json!({"first": 2, "last": 5})).unwrap();
    assert_eq!(hub.snapshot(other).unwrap().status, Status::Failed);
    hub.may_edit(1, Some(own), &target(2), |selection| selection["last"] == 5).unwrap();
    hub.advance(1, own, &target(2), target(3), json!({"first": 2, "last": 6})).unwrap();
    hub.may_edit(1, Some(own), &target(3), |selection| selection["last"] == 6).unwrap();
    hub.reply(1, own, answer("Applied twice"), Some(&target(3))).unwrap();
}

#[test]
/// Checks that advance cannot switch document or skip ownership.
fn advance_cannot_switch_document_or_skip_ownership() {
    let hub = AgentHub::default();
    let id = hub.ask(request("Edit")).unwrap();
    take(&hub, 1);
    assert_eq!(hub.advance(2, id, &target(1), target(2), Value::Null).unwrap_err().code, NOT_OWNER);
    let next = Target { document: "document-2".into(), revision: 2 };
    assert_eq!(hub.advance(1, id, &target(1), next, Value::Null).unwrap_err().code, INVALID_PARAMS);
    assert_eq!(hub.advance(1, id, &target(1), target(1), Value::Null).unwrap_err().code, INVALID_PARAMS);
    hub.may_edit(1, Some(id), &target(1), |_| true).unwrap();
}

#[test]
/// Checks that disconnect requeues work and rejects the previous owners reply.
fn disconnect_requeues_work_and_rejects_the_previous_owners_reply() {
    let hub = AgentHub::default();
    let mut req = request("Review");
    req.agent = Some(" Codex ".into());
    let id = hub.ask(req).unwrap();
    assert!(hub.listen(1, Some("Claude"), Duration::ZERO, || true).unwrap().is_none());
    assert_eq!(hub.listen(2, Some("Codex"), Duration::ZERO, || true).unwrap().unwrap().id, id);
    hub.disconnect(2);
    assert_eq!(hub.reply(2, id, answer("Old connection"), Some(&target(1))).unwrap_err().code, NOT_OWNER);
    assert_eq!(hub.listen(3, Some("Codex"), Duration::ZERO, || true).unwrap().unwrap().id, id);
    assert_eq!(hub.reply(2, id, answer("Still old"), Some(&target(1))).unwrap_err().code, NOT_OWNER);
}

#[test]
/// Checks that a new attempt does not inherit the previous owner's progress or limits.
fn requeued_work_gets_fresh_progress_notes_and_budgets() {
    for (progress_bytes, progress_notes) in [(1, 2), (2, 1)] {
        let hub = AgentHub::with_limits(Limits { progress_bytes, progress_notes, ..Limits::default() });
        let id = hub.ask(request("Try again")).unwrap();
        take(&hub, 1);
        hub.reply(1, id, Reply { text: "x", done: false, ..Reply::default() }, Some(&target(1))).unwrap();
        hub.disconnect(1);
        assert!(hub.snapshot(id).unwrap().notes.is_empty());
        take(&hub, 2);
        hub.reply(2, id, Reply { text: "y", done: false, ..Reply::default() }, Some(&target(1))).unwrap();
        assert_eq!(hub.snapshot(id).unwrap().notes, ["y"]);
    }
}

#[test]
/// Checks that a dead connection does not take queued work or register itself.
fn a_dead_connection_does_not_take_queued_work_or_register_itself() {
    let hub = AgentHub::default();
    let id = hub.ask(request("Do not lose me")).unwrap();
    assert!(hub.listen(1, None, Duration::ZERO, || false).unwrap().is_none());
    assert!(hub.agents().is_empty());
    assert_eq!(take(&hub, 2).id, id);
}

#[test]
/// Checks that queue capacity includes finished requests until explicitly closed.
fn queue_capacity_includes_finished_requests_until_explicitly_closed() {
    let hub = AgentHub::with_limits(Limits { requests: 1, ..Limits::default() });
    let id = hub.ask(request("First")).unwrap();
    assert_eq!(hub.ask(request("Second")).unwrap_err().code, LIMIT_EXCEEDED);
    take(&hub, 1);
    hub.reply(1, id, answer("Finished"), Some(&target(1))).unwrap();
    assert_eq!(hub.ask(request("Third")).unwrap_err().code, LIMIT_EXCEEDED);
    hub.close(id);
    assert!(hub.ask(request("After close")).unwrap() > id);
}

#[test]
/// Checks that oversized requests and target updates do not partially change the queue.
fn oversized_requests_and_target_updates_do_not_partially_change_the_queue() {
    let hub = AgentHub::with_limits(Limits { request_bytes: 512, ..Limits::default() });
    let mut oversized = request("Too large");
    oversized.document = json!({"text": "x".repeat(512)});
    assert_eq!(hub.ask(oversized).unwrap_err().code, LIMIT_EXCEEDED);
    let id = hub.ask(request("Small")).unwrap();
    assert_eq!(take(&hub, 1).id, id);
    assert_eq!(hub.advance(1, id, &target(1), target(2), json!("x".repeat(512))).unwrap_err().code, LIMIT_EXCEEDED);
    hub.may_edit(1, Some(id), &target(1), |_| true).unwrap();
}

#[test]
/// Checks that progress and answer limits reject whole messages without truncation.
fn progress_and_answer_limits_reject_whole_messages_without_truncation() {
    let hub = AgentHub::with_limits(Limits { reply_bytes: 8, progress_bytes: 5, progress_notes: 2, ..Limits::default() });
    let id = hub.ask(request("Bounded replies")).unwrap();
    take(&hub, 1);
    for text in ["abc", "de"] {
        hub.reply(1, id, Reply { text, done: false, ..Reply::default() }, Some(&target(1))).unwrap();
    }
    assert_eq!(hub.reply(1, id, Reply { text: "f", done: false, ..Reply::default() }, Some(&target(1))).unwrap_err().code, LIMIT_EXCEEDED);
    assert_eq!(hub.reply(1, id, answer("123456789"), Some(&target(1))).unwrap_err().code, LIMIT_EXCEEDED);
    assert_eq!(hub.snapshot(id).unwrap().notes, ["abc", "de"]);
    assert_eq!(hub.snapshot(id).unwrap().status, Status::Working { conn: 1 });
    hub.reply(1, id, answer("12345678"), Some(&target(1))).unwrap();
    assert_eq!(hub.snapshot(id).unwrap().answer.as_deref(), Some("12345678"));
}

#[test]
/// Checks that listener names and count are bounded and disconnect releases capacity.
fn listener_names_and_count_are_bounded_and_disconnect_releases_capacity() {
    let hub = AgentHub::with_limits(Limits { listeners: 1, ..Limits::default() });
    hub.listen(1, None, Duration::ZERO, || true).unwrap();
    assert_eq!(hub.listen(2, None, Duration::ZERO, || true).unwrap_err().code, LIMIT_EXCEEDED);
    hub.disconnect(1);
    assert!(hub.listen(2, None, Duration::ZERO, || true).is_ok());
    assert_eq!(hub.listen(2, Some(&"x".repeat(257)), Duration::ZERO, || true).unwrap_err().code, INVALID_PARAMS);
}

#[test]
/// Checks that wire validation and current document resolution are enforced.
fn wire_validation_and_current_document_resolution_are_enforced() {
    let hub = AgentHub::default();
    let id = hub.ask(request("Wire")).unwrap();
    let resolve = |_: &Target| Some(target(1));
    assert_eq!(hub.handle(1, "agent.listen", &json!({"timeoutMs": 0}), || true, resolve).unwrap()["result"]["id"], id);
    for params in [json!([]), json!({}), json!({"id": id}), json!({"id": id, "text": 3}), json!({"id": id, "text": "ok", "done": "yes"})] {
        assert_eq!(hub.handle(1, "agent.reply", &params, || true, resolve).unwrap()["errorCode"], INVALID_PARAMS);
    }
    let reply = hub.handle(1, "agent.reply", &json!({"id": id, "text": "stale answer"}), || true, |_| Some(target(2))).unwrap();
    assert_eq!(reply["errorCode"], STALE_REQUEST);
    assert!(hub.snapshot(id).unwrap().answer.is_none());
    assert!(hub.handle(1, "file.save", &json!({}), || true, resolve).is_none());
}

#[test]
/// Checks that wire error response is terminal even if done was false.
fn wire_error_response_is_terminal_even_if_done_was_false() {
    let hub = AgentHub::default();
    let id = hub.ask(request("Wire error")).unwrap();
    take(&hub, 1);
    let response = hub.handle(1, "agent.reply", &json!({"id": id, "error": "cannot edit", "done": false}), || true, |_| Some(target(1))).unwrap();
    assert_eq!(response["result"]["done"], true);
    assert_eq!(hub.snapshot(id).unwrap().status, Status::Failed);
}

#[test]
/// Checks that proposal-only replies retain all review fields without performing an edit.
fn proposal_only_replies_are_retained_as_unapplied_review_text() {
    let hub = AgentHub::default();
    let id = hub.ask(request("Propose a change")).unwrap();
    take(&hub, 1);
    let proposal = "{\"tool\":\"delete_everything\"}";
    let response = hub
        .handle(
            1,
            "agent.reply",
            &json!({"id": id, "proposal": proposal, "format": "json", "comment": "<b>review this</b>"}),
            || true,
            |_| Some(target(1)),
        )
        .unwrap();
    assert_eq!(response["ok"], true);
    let snapshot = hub.snapshot(id).unwrap();
    assert_eq!(snapshot.status, Status::Answered);
    assert_eq!(snapshot.proposal.as_deref(), Some(proposal));
    assert_eq!(snapshot.format.as_deref(), Some("json"));
    assert_eq!(snapshot.comment.as_deref(), Some("<b>review this</b>"));
    let note = snapshot.review_text();
    assert!(note.contains("Proposal (not applied)"));
    assert!(note.contains(proposal));
    assert!(note.contains("Format: json"));
    assert!(note.contains("<b>review this</b>"));
}

#[test]
/// Checks that proposal fields are typed and bounded and refusals store no partial answer.
fn proposal_fields_are_validated_before_any_response_is_retained() {
    let hub = AgentHub::default();
    let id = hub.ask(request("Validate proposed text")).unwrap();
    take(&hub, 1);
    for extra in [json!({"proposal": true}), json!({"proposal": ""}), json!({"text": "ok", "comment": []})] {
        let mut params = extra;
        params["id"] = json!(id);
        assert_eq!(hub.handle(1, "agent.reply", &params, || true, |_| Some(target(1))).unwrap()["errorCode"], INVALID_PARAMS);
    }
    for extra in [
        json!({"proposal": "x".repeat(200_001)}),
        json!({"text": "x".repeat(4_001)}),
        json!({"text": "ok", "comment": "x".repeat(2_001)}),
        json!({"proposal": "ok", "format": "x".repeat(65)}),
    ] {
        let mut params = extra;
        params["id"] = json!(id);
        assert_eq!(hub.handle(1, "agent.reply", &params, || true, |_| Some(target(1))).unwrap()["errorCode"], LIMIT_EXCEEDED);
        let snapshot = hub.snapshot(id).unwrap();
        assert_eq!(snapshot.status, Status::Working { conn: 1 });
        assert!(snapshot.answer.is_none() && snapshot.proposal.is_none() && snapshot.comment.is_none());
    }
    let response = hub.handle(1, "agent.reply", &json!({"id": id, "proposal": "x".repeat(200_000)}), || true, |_| Some(target(1))).unwrap();
    assert_eq!(response["ok"], true);
}

#[test]
/// Checks that a stale proposal is dropped and a configured envelope limit includes metadata.
fn stale_or_oversized_proposals_do_not_survive_as_review_notes() {
    let hub = AgentHub::with_limits(Limits { reply_bytes: 8, ..Limits::default() });
    let id = hub.ask(request("Small envelope")).unwrap();
    take(&hub, 1);
    let reply = Reply { text: "1234", proposal: Some("56789"), ..Reply::default() };
    assert_eq!(hub.reply(1, id, reply, Some(&target(1))).unwrap_err().code, LIMIT_EXCEEDED);
    assert!(hub.snapshot(id).unwrap().proposal.is_none());
    let reply = Reply { proposal: Some("123"), ..Reply::default() };
    assert_eq!(hub.reply(1, id, reply, Some(&target(2))).unwrap_err().code, STALE_REQUEST);
    assert!(hub.snapshot(id).unwrap().review_text().is_empty());
}

#[test]
/// Preserves the original stable-agent-colour and sorted-list contract.
fn agent_colours_and_names_are_stable() {
    assert_eq!(colour("Claude"), colour("Claude"));
    assert_ne!(colour("Claude"), colour("Codex"));
    let hub = AgentHub::default();
    for (conn, name) in [(1, "Codex"), (2, "Claude"), (3, "Codex")] {
        hub.listen(conn, Some(name), Duration::ZERO, || true).unwrap();
    }
    assert_eq!(hub.agents(), ["Claude", "Codex"]);
}
