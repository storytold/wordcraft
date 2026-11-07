//! Commands a host page runs in the embedded editor, over its bridge.
//!
//! `?embed=1` editors take more than files from their host: a page that keeps the notes and
//! the approval state of a contract drives the editor's review features directly. While the
//! restricted rules refuse what would leave the host's document behind (see
//! [`wordcraft_ui_egui::embed`]), everything allowed is reachable: `review.comments` and
//! `review.changes` list the document's comments and tracked changes, `review.newComment`
//! adds one, `review.resolveComment` settles one, `review.nextComment` goes to it,
//! `review.acceptAll` and `review.rejectAll` resolve the markup, `file.exportPdf` hands the
//! host a PDF copy. Run through [`WordApp::execute`](wordcraft_ui_egui::WordApp::execute) —
//! the programmatic path, like the control channel's: it never asks first (no Save Changes
//! prompt stalls a host), and the embedded refusals apply to the host's commands exactly as
//! to a user's. A command on an encrypted
//! document fails with the needs-password error: open encrypted documents through
//! `wordcraft:open`, where the editor asks for the password in the iframe.
//!
//! The wire shape matches the `craft-web-host` envelope: a plain object
//! `{ type: "wordcraft:command", reqId: string, id: string, params: string }` from the page
//! (or the same as a JSON-encoded string), answered with
//! `{ type: "wordcraft:result", reqId: string, ok: bool, result?: string, error?: string }`,
//! where `params` and `result` are JSON-encoded strings (a string contract needs no extra
//! conversion dependencies on either side). Same-origin only, like the file messages, and
//! only for iframe hosts (`?host=parent`); messages that are not well-formed JSON or carry
//! another `type` are ignored, as craft-web-host ignores its unknown messages. A message
//! over [`MAX_COMMAND_BYTES`] is refused, not run.

use serde_json::Value;

/// The largest command message: a hostile or looping host cannot force unbounded JSON
/// parses. Files have their own, much larger cap (`MAX_OPEN_BYTES` in `host`).
pub(crate) const MAX_COMMAND_BYTES: usize = 1 << 20;

/// Whether a message text is over [`MAX_COMMAND_BYTES`].
pub(crate) fn over_cap(text: &str) -> bool {
    text.len() > MAX_COMMAND_BYTES
}

/// The largest number of host-delivered files waiting to open, and the largest total bytes
/// they may hold: a burst of opens between two frames cannot grow wasm memory without bound.
pub(crate) const MAX_PENDING_FILES: usize = 8;
pub(crate) const MAX_PENDING_FILE_BYTES: u64 = 512 << 20;

/// Whether a delivered file may join the open queue (`queued` files totalling `queued_bytes`).
pub(crate) fn file_queue_full(queued: usize, queued_bytes: u64) -> bool {
    queued >= MAX_PENDING_FILES || queued_bytes >= MAX_PENDING_FILE_BYTES
}

/// A command the host asked to run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HostCommand {
    /// Echoed in the reply.
    pub req_id: String,
    /// The command id, run through [`WordApp::execute`](wordcraft_ui_egui::WordApp::execute).
    pub id: String,
    /// The command's parameters, as sent (a JSON-encoded string, parsed at run time).
    pub params: String,
}

/// Whether a parsed message is addressed to this listener.
pub(crate) fn is_command(data: &Value) -> bool {
    data.get("type").and_then(Value::as_str) == Some("wordcraft:command")
}

/// The request id a message carries, as far as it can be told: a parse error still gets a
/// reply when the id is readable.
pub(crate) fn req_id_of(data: &Value) -> String {
    data.get("reqId").and_then(Value::as_str).unwrap_or_default().to_owned()
}

impl HostCommand {
    /// Parse a `wordcraft:command` message. Every field but `reqId` must be a string when it
    /// is there; an absent `params` is the common case (an empty parameter object).
    pub(crate) fn parse(data: &Value) -> Result<Self, String> {
        let object = data.as_object().ok_or("the message is not an object")?;
        let string = |name: &str| -> Result<Option<String>, String> {
            match object.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if name == "params" => {
                    // A params string must itself be JSON: checked here so the error carries
                    // the command id.
                    serde_json::from_str::<Value>(s)
                        .map_err(|e| format!("`params` of `{}` is not JSON: {e}", object.get("id").and_then(Value::as_str).unwrap_or("?")))?;
                    Ok(Some(s.clone()))
                }
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(format!("`{name}` must be a string")),
            }
        };
        let id = string("id")?.filter(|id| !id.is_empty()).ok_or("wordcraft:command needs a non-empty `id` string")?;
        let params = string("params")?.unwrap_or_else(|| "{}".to_owned());
        Ok(Self { req_id: string("reqId")?.unwrap_or_default(), id, params })
    }

    /// The parsed parameter object for the run.
    pub fn params_value(&self) -> Value {
        serde_json::from_str(&self.params).unwrap_or_else(|_| serde_json::json!({}))
    }
}

/// The `wordcraft:result` reply: the request id echoed, `ok` a real boolean, the run's
/// outcome as `result` (JSON-encoded) or `error`.
pub(crate) fn reply_message(req_id: &str, outcome: &Result<Value, String>) -> Value {
    match outcome {
        Ok(v) => serde_json::json!({
            "type": "wordcraft:result",
            "reqId": req_id,
            "ok": true,
            "result": serde_json::to_string(v).unwrap_or_else(|_| "null".to_owned()),
        }),
        Err(e) => serde_json::json!({
            "type": "wordcraft:result",
            "reqId": req_id,
            "ok": false,
            "error": e,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{HostCommand, is_command, over_cap, reply_message, req_id_of};
    use serde_json::{Value, json};

    fn parse_command_text(text: &str) -> Result<HostCommand, String> {
        let data: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        if !is_command(&data) {
            return Err("not a command".to_owned());
        }
        HostCommand::parse(&data)
    }

    /// A full command parses: request id, command id, JSON parameters.
    #[test]
    fn parses_a_command() {
        let c = parse_command_text(r#"{"type":"wordcraft:command","reqId":"r7","id":"review.newComment","params":"{\"text\":\"Fix this clause\"}"}"#)
            .unwrap();
        assert_eq!(c.req_id, "r7");
        assert_eq!(c.id, "review.newComment");
        assert_eq!(c.params_value().get("text").and_then(Value::as_str), Some("Fix this clause"));
    }

    /// No parameters is the common case: an empty object.
    #[test]
    fn parses_without_params() {
        let c = parse_command_text(r#"{"type":"wordcraft:command","id":"review.comments"}"#).unwrap();
        assert_eq!(c.id, "review.comments");
        assert_eq!(c.params_value(), json!({}));
    }

    /// Messages that are not for this listener are told apart from broken commands: a foreign
    /// type or unparseable JSON is ignored, a command with bad fields errors (and its reply
    /// echoes whatever request id it carried).
    #[test]
    fn dispatches_and_rejects() {
        let data: Value = serde_json::from_str(r#"{"type":"wordcraft:save","id":"x"}"#).unwrap();
        assert!(!is_command(&data), "another app's message is not a command");
        assert!(serde_json::from_str::<Value>("{oops").is_err(), "unparseable is not a command");
        for (text, why) in [
            (r#"{"type":"wordcraft:command"}"#, "no id"),
            (r#"{"type":"wordcraft:command","id":""}"#, "empty id"),
            (r#"{"type":"wordcraft:command","id":42}"#, "id not a string"),
            (r#"{"type":"wordcraft:command","id":"review.comments","params":{}}"#, "params not a string"),
            (r#"{"type":"wordcraft:command","id":"review.comments","params":"{oops"}"#, "params not JSON"),
        ] {
            let data: Value = serde_json::from_str(text).unwrap();
            assert!(parse_command_text(text).is_err(), "{why}");
            assert_eq!(req_id_of(&data), "", "no request id to echo");
        }
        let data: Value = serde_json::from_str(r#"{"type":"wordcraft:command","reqId":"kept","id":42}"#).unwrap();
        assert_eq!(req_id_of(&data), "kept", "a readable request id is echoed on errors");
    }

    /// The delivered-file queue refuses past either bound.
    #[test]
    fn caps_the_file_queue() {
        assert!(!super::file_queue_full(0, 0));
        assert!(super::file_queue_full(super::MAX_PENDING_FILES, 0));
        assert!(super::file_queue_full(1, super::MAX_PENDING_FILE_BYTES));
    }

    /// A message over the size cap is refused before any parse.
    #[test]
    fn caps_message_size() {
        let big = format!("{{\"type\":\"wordcraft:command\",\"pad\":\"{}\"}}", "x".repeat(1 << 21));
        assert!(over_cap(&big));
        assert!(!over_cap(r#"{"type":"wordcraft:command","id":"review.comments"}"#));
    }

    /// The reply is the documented envelope with a real boolean `ok`.
    #[test]
    fn replies() {
        let ok = reply_message("r1", &Ok(json!({"comments": []})));
        assert_eq!(ok.get("ok"), Some(&Value::Bool(true)), "ok is a boolean");
        assert_eq!(ok.get("reqId"), Some(&json!("r1")));
        assert_eq!(ok.get("result").and_then(Value::as_str), Some(r#"{"comments":[]}"#));
        let failed = reply_message("r2", &Err("refused".to_owned()));
        assert_eq!(failed.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(failed.get("error").and_then(Value::as_str), Some("refused"));
        assert!(failed.get("result").is_none(), "an error reply carries no result");
    }
}
