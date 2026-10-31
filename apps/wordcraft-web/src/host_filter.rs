//! The host bridge's message filter: which incoming postMessage the editor acts on.
//!
//! Pure (no browser types), so the accept/reject decision is pinned by native tests; the bridge
//! (`host.rs`) feeds it the facts of each `message` event. A message is accepted only when
//! the page has a concrete origin of its own, the sender has that same origin, the sender is the
//! parent window (not another same-origin iframe or a `window.open`ed window), and the type is one
//! of the host → editor messages.

use serde_json::Value;

/// A host → editor message the bridge acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// `{type:"wordcraft:open", name, bytes}`
    Open,
    /// `{type:"wordcraft:save-ack", ...}` — accepted for compatibility; the upstream
    /// bridge no longer sends it, and the editor ignores it.
    SaveAck,
    /// `{type:"wordcraft:cmd", id, params?}`
    Cmd,
}

/// Why an incoming message was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The page's own origin is opaque (`"null"`, a sandboxed iframe without `allow-same-origin`)
    /// or unknown: there is nothing to vet the sender against, so the bridge stays off.
    OpaqueOrigin,
    /// The sender's origin differs from the page's.
    ForeignOrigin,
    /// The sender is not the parent window.
    NotFromParent,
    /// The message has no string `type`.
    NoType,
    /// The type is not a host → editor message.
    UnknownType,
}

/// Whether `origin` (the page's `window.origin`) is concrete enough to vet senders and to address
/// postMessage to: not empty, not opaque (`"null"`), not the `"*"` wildcard.
pub fn usable_origin(origin: &str) -> bool {
    !origin.is_empty() && origin != "null" && origin != "*"
}

/// Decide whether to act on an incoming message.
///
/// `origin` is the event's `origin`, `own_origin` the page's `window.origin`, `source_is_parent`
/// whether the event's `source` is `window.parent` (false when either is missing), `kind` the
/// message's `type` field when it is a string.
pub fn decide(origin: &str, own_origin: &str, source_is_parent: bool, kind: Option<&str>) -> Result<Incoming, Rejected> {
    if !usable_origin(own_origin) {
        return Err(Rejected::OpaqueOrigin);
    }
    if origin != own_origin {
        return Err(Rejected::ForeignOrigin);
    }
    if !source_is_parent {
        return Err(Rejected::NotFromParent);
    }
    match kind {
        None => Err(Rejected::NoType),
        Some("wordcraft:open") => Ok(Incoming::Open),
        Some("wordcraft:save-ack") => Ok(Incoming::SaveAck),
        Some("wordcraft:cmd") => Ok(Incoming::Cmd),
        Some(_) => Err(Rejected::UnknownType),
    }
}

/// Whether a successful `file.close` result posts `wordcraft:docClosed` to the host.
///
/// The host posts it on a `{"quit": true}` close and on a host `wordcraft:cmd`
/// `file.close` that reports `{"closed": true}`. A plain File › Close that keeps the
/// window (`{"closed": false}` or a dirty close) does not.
pub fn posts_doc_closed(result: &Value) -> bool {
    result.get("closed").and_then(Value::as_bool) == Some(true) || result.get("quit").and_then(Value::as_bool) == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = "https://omd.example";

    #[test]
    fn the_parent_frame_is_accepted_for_every_host_message() {
        assert_eq!(decide(OWN, OWN, true, Some("wordcraft:open")), Ok(Incoming::Open));
        assert_eq!(decide(OWN, OWN, true, Some("wordcraft:save-ack")), Ok(Incoming::SaveAck));
        assert_eq!(decide(OWN, OWN, true, Some("wordcraft:cmd")), Ok(Incoming::Cmd));
    }

    /// Review item 1: a second same-origin window (another iframe, a `window.open`ed page) passes
    /// the origin check but is not the parent, so it can't open, ack saves or run commands.
    #[test]
    fn a_same_origin_sender_that_is_not_the_parent_is_dropped() {
        for kind in ["wordcraft:open", "wordcraft:save-ack", "wordcraft:cmd"] {
            assert_eq!(decide(OWN, OWN, false, Some(kind)), Err(Rejected::NotFromParent), "{kind}");
        }
    }

    #[test]
    fn a_foreign_origin_is_dropped_even_from_the_parent() {
        assert_eq!(decide("https://evil.example", OWN, true, Some("wordcraft:cmd")), Err(Rejected::ForeignOrigin));
        assert_eq!(decide("null", OWN, true, Some("wordcraft:cmd")), Err(Rejected::ForeignOrigin));
        assert_eq!(decide("", OWN, true, Some("wordcraft:open")), Err(Rejected::ForeignOrigin));
        // Origins compare exactly: scheme and port matter.
        assert_eq!(decide("http://omd.example", OWN, true, Some("wordcraft:open")), Err(Rejected::ForeignOrigin));
        assert_eq!(decide("https://omd.example:8443", OWN, true, Some("wordcraft:open")), Err(Rejected::ForeignOrigin));
    }

    /// Review item 2: in a sandboxed iframe without `allow-same-origin` the page's origin is
    /// `"null"`, and every opaque sender reports `"null"` too; nothing is accepted.
    #[test]
    fn an_opaque_own_origin_accepts_nothing() {
        for own in ["null", "*", ""] {
            assert_eq!(decide(own, own, true, Some("wordcraft:cmd")), Err(Rejected::OpaqueOrigin), "{own:?}");
            assert_eq!(decide("null", own, true, Some("wordcraft:open")), Err(Rejected::OpaqueOrigin), "{own:?}");
            assert!(!usable_origin(own), "{own:?}");
        }
        assert!(usable_origin(OWN));
        assert!(usable_origin("http://127.0.0.1:8807"));
    }

    #[test]
    fn untyped_and_unknown_messages_are_dropped() {
        assert_eq!(decide(OWN, OWN, true, None), Err(Rejected::NoType));
        for kind in ["", "open", "wordcraft:ready", "wordcraft:save", "wordcraft:response", "wordcraft:docClosed", "WORDCRAFT:CMD"] {
            assert_eq!(decide(OWN, OWN, true, Some(kind)), Err(Rejected::UnknownType), "{kind}");
        }
    }

    #[test]
    fn doc_closed_posts_on_a_closed_or_quit_result() {
        assert!(posts_doc_closed(&serde_json::json!({"closed": true})));
        assert!(posts_doc_closed(&serde_json::json!({"quit": true})));
        assert!(!posts_doc_closed(&serde_json::json!({"closed": false})));
        assert!(!posts_doc_closed(&serde_json::json!({"closed": false, "dirty": true})));
        assert!(!posts_doc_closed(&serde_json::json!({})));
    }
}
