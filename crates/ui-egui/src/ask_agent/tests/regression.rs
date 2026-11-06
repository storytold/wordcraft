#![allow(clippy::unwrap_used)]

use std::time::Duration;

use super::{AgentHub, HubError, Request};
use serde_json::json;

// These two adapters let the same behavioral reproduction run against the previous infallible
// queue API and the new bounded API. A compiler error is not proof of the dead-listener bug.
trait Enqueued {
    fn into_id(self) -> u64;
}

impl Enqueued for u64 {
    fn into_id(self) -> u64 {
        self
    }
}

impl Enqueued for Result<u64, HubError> {
    fn into_id(self) -> u64 {
        self.unwrap()
    }
}

trait Listened {
    fn into_request(self) -> Option<Request>;
}

impl Listened for Option<Request> {
    fn into_request(self) -> Option<Request> {
        self
    }
}

impl Listened for Result<Option<Request>, HubError> {
    fn into_request(self) -> Option<Request> {
        self.unwrap()
    }
}

fn request() -> Request {
    // The old API ignores the additional target field; the new API validates and uses it.
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
/// Checks that a dead listener cannot claim work queued before listen.
fn a_dead_listener_cannot_claim_work_queued_before_listen() {
    let hub = AgentHub::default();
    let id = hub.ask(request()).into_id();
    let dead = hub.listen(1, None, Duration::ZERO, || false).into_request();
    assert!(dead.is_none(), "a closed socket must not consume the queued request");
    assert!(hub.agents().is_empty(), "a closed socket must not appear as an available agent");
    let live = hub.listen(2, None, Duration::ZERO, || true).into_request().unwrap();
    assert_eq!(live.id, id, "the same request must remain available for a live connection");
}
