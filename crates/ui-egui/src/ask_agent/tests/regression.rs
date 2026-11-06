#![allow(clippy::unwrap_used)]

use std::time::Duration;

use super::{AgentHub, Request};
use serde_json::json;

/// Build the original dead-listener fixture with the current fallible bounded-queue API.
/// Direct unwraps assert that contract; obsolete compatibility traits must not swallow its errors.
fn request() -> Request {
    // Keep the original dead-listener reproduction with the current bounded queue API.
    serde_json::from_value(json!({
        "id": 0,
        "instruction": "Do not assign this request to a dead socket",
        "history": [],
        "selection": {},
        "document": {},
        "target": {"document": "repro-document", "revision": 1},
        "agent": null
    }))
    .unwrap()
}

#[test]
/// A socket already dead before listen must neither register nor consume the FIFO request;
/// the same queued request must remain claimable by the next live connection.
fn a_dead_listener_cannot_claim_work_queued_before_listen() {
    let hub = AgentHub::default();
    let id = hub.ask(request()).unwrap();
    let dead = hub.listen(1, None, Duration::ZERO, || false).unwrap();
    assert!(dead.is_none(), "a closed socket must not consume the queued request");
    assert!(hub.agents().is_empty(), "a closed socket must not appear as an available agent");
    let live = hub.listen(2, None, Duration::ZERO, || true).unwrap().unwrap();
    assert_eq!(live.id, id, "the same request must remain available for a live connection");
}
