//! The host bridge's message filter: which incoming postMessage the editor acts on.
//!
//! Pure (no browser types), so the accept/reject decision is pinned by native tests; the bridge
//! (`web/bridge.rs`) feeds it the facts of each `message` event. A message is accepted only when
//! the page has a concrete origin of its own, the sender has that same origin, the sender is the
//! parent window (not another same-origin iframe or a `window.open`ed window), and the type is one
//! of the host → editor messages.

use serde_json::Value;

/// A host → editor message the bridge acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// `{type:"wordcraft:open", name, bytes}`
    Open,
    /// `{type:"wordcraft:save-ack", ticket, ok, error?}`
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

/// The `params` field of a `wordcraft:cmd` message, as the bridge found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawParams<'a> {
    /// Missing, `undefined` or `null`.
    Absent,
    /// A string (the documented form: JSON text).
    Text(&'a str),
    /// Present but not a string (a number, an object…).
    NotText,
}

/// The engine params of a `wordcraft:cmd`: `{}` when absent, the parsed JSON when it is valid JSON
/// text, `None` (drop the command) when present but malformed.
pub fn cmd_params(raw: RawParams<'_>) -> Option<Value> {
    match raw {
        RawParams::Absent => Some(Value::Object(serde_json::Map::new())),
        RawParams::Text(s) => serde_json::from_str(s).ok(),
        RawParams::NotText => None,
    }
}

/// The `wordcraft:response` for an engine command. `id` is the command's own id (as sent), and
/// `requestId` is echoed from the host's `wordcraft:cmd` when it sent one, so a host that runs the
/// same command twice can still match each reply to its request. Results go in `value`, failures
/// in `error` (both JSON text, as before). Keys with no value are left out.
pub fn response_message(command_id: &str, request_id: Option<&str>, ok: bool, payload: &Value) -> Value {
    let mut message = serde_json::Map::new();
    message.insert("type".into(), Value::from("wordcraft:response"));
    message.insert("id".into(), Value::from(command_id));
    if let Some(request_id) = request_id {
        message.insert("requestId".into(), Value::from(request_id));
    }
    message.insert("ok".into(), Value::from(ok));
    message.insert(if ok { "value" } else { "error" }.into(), Value::from(payload.to_string()));
    Value::Object(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    /// CodeRabbit (#2919): absent params mean `{}`, malformed ones drop the command instead of
    /// running it with `{}`.
    #[test]
    fn a_response_echoes_the_request_id_so_replies_can_be_matched() {
        let ok = response_message("text.insert", Some("req-7"), true, &json!({ "inserted": 5 }));
        assert_eq!(ok["type"], "wordcraft:response");
        assert_eq!(ok["id"], "text.insert");
        assert_eq!(ok["requestId"], "req-7");
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["value"], r#"{"inserted":5}"#);
        assert!(ok.get("error").is_none());
        let failed = response_message("text.insert", Some("req-8"), false, &json!({ "error": "no selection" }));
        assert_eq!(failed["requestId"], "req-8");
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["error"], r#"{"error":"no selection"}"#);
        assert!(failed.get("value").is_none());
    }

    #[test]
    fn a_response_without_a_request_id_has_none() {
        let reply = response_message("text.insert", None, true, &json!(null));
        assert_eq!(reply["id"], "text.insert");
        assert!(reply.get("requestId").is_none());
    }

    #[test]
    fn cmd_params_are_parsed_or_refused() {
        assert_eq!(cmd_params(RawParams::Absent), Some(json!({})));
        assert_eq!(cmd_params(RawParams::Text(r#"{"text":"Hello"}"#)), Some(json!({"text": "Hello"})));
        assert_eq!(cmd_params(RawParams::Text("{}")), Some(json!({})));
        assert_eq!(cmd_params(RawParams::Text("{not json")), None);
        assert_eq!(cmd_params(RawParams::Text("")), None);
        assert_eq!(cmd_params(RawParams::NotText), None);
    }
}
