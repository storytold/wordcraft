//! Real-time collaboration settings (Review › Collaborate, ADR-0001…0011).
//!
//! These commands only flip the feature flag and remember the server address;
//! the WebSocket itself belongs to the front end (`wordcraft-collab-client` and
//! the host bridge), so the engine stays free of networking and the single-user
//! mode makes no calls while `collab.enabled` is false.

use serde_json::{Value, json};

use crate::{CmdError, CmdResult, CommandSpec, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("collab.enable", "Collaborate", "Review › Collaborate", enable).params(r#"{"server": string, "enabled"?: bool}"#),
        CommandSpec::new("collab.disable", "Stop Collaborating", "Review › Collaborate", disable),
        CommandSpec::new("collab.status", "Collaboration Status", "Review › Collaborate", status).pure(),
    ]
}

fn valid_server(server: &str) -> bool {
    (server.starts_with("ws://") || server.starts_with("wss://")) && !server.contains(char::is_whitespace)
}

fn enable(s: &mut Session, v: &Value) -> CmdResult {
    let server = v.get("server").and_then(Value::as_str).ok_or_else(|| CmdError::Params("collab.enable needs `server`".into()))?;
    if !valid_server(server) {
        return Err(CmdError::Params("`server` must be a ws:// or wss:// URL".into()));
    }
    s.prefs.collab.server = server.to_string();
    s.prefs.collab.enabled = true;
    s.status = "Collaboration enabled".into();
    Ok(json!({"enabled": true, "server": server}))
}

fn disable(s: &mut Session, _v: &Value) -> CmdResult {
    s.prefs.collab.enabled = false;
    s.status = "Collaboration off".into();
    Ok(json!({"enabled": false, "server": s.prefs.collab.server}))
}

fn status(s: &mut Session, _v: &Value) -> CmdResult {
    Ok(json!({
        "enabled": s.prefs.collab.enabled,
        "server": s.prefs.collab.server,
        "author": s.author,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::Document;

    fn session() -> Session {
        Session::new(Document::new())
    }

    #[test]
    fn collaboration_is_off_by_default() {
        let mut s = session();
        let result = status(&mut s, &Value::Null).expect("status");
        assert_eq!(result["enabled"], false);
    }

    #[test]
    fn enabling_records_the_server() {
        let mut s = session();
        let result = enable(&mut s, &json!({"server": "wss://collab.example/ws"})).expect("enable");
        assert_eq!(result["enabled"], true);
        assert_eq!(s.prefs.collab.server, "wss://collab.example/ws");
        assert!(s.prefs.collab.enabled);
    }

    #[test]
    fn a_bad_server_is_refused() {
        let mut s = session();
        let err = enable(&mut s, &json!({"server": "http://collab.example"})).expect_err("http is not a socket URL");
        assert!(matches!(err, CmdError::Params(_)));
        assert!(!s.prefs.collab.enabled, "a refused command leaves the flag alone");
    }

    #[test]
    fn disabling_keeps_the_address() {
        let mut s = session();
        enable(&mut s, &json!({"server": "ws://127.0.0.1:9999/ws"})).expect("enable");
        let result = disable(&mut s, &Value::Null).expect("disable");
        assert_eq!(result["enabled"], false);
        assert_eq!(s.prefs.collab.server, "ws://127.0.0.1:9999/ws");
    }

    #[test]
    fn a_remote_command_does_not_enter_the_local_undo_stack() {
        let mut s = session();
        s.run("text.insert", &json!({"text": "a"})).expect("local typing");
        let local_steps = s.undo_depth();
        s.run_remote("text.insert", &json!({"text": "b"})).expect("a command from another replica");
        assert_eq!(s.undo_depth(), local_steps, "the remote edit is not a step of this user's undo");
        let text = s.doc.body.first().and_then(|block| block.as_para()).map(|para| para.text.clone()).unwrap_or_default();
        assert_eq!(text, "ab");
    }
}
