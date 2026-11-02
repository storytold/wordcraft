//! `wordcraft-cli mcp --allow-commands <list>`: through the MCP protocol, a client runs only the
//! commands on the list. `execute`, every command of `batch`, the commands behind the dedicated
//! tools (`open_document`, `save_document`…) and every step of a macro are held to it, so none of
//! them is a way around it; `list_commands` shows only what may run; reading the document
//! (`inspect_document`, `render_page`) needs no command and keeps working.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_engine::command_access::CommandAllowlist;
use wordcraft_mcp::{Headless, Server};

/// A headless server held to `list`.
fn allowed(list: &str) -> Server {
    let headless = Headless::default();
    let commands = CommandAllowlist::parse(list, &headless.session.registry).unwrap();
    Server::new(Box::new(headless.with_command_allowlist(Some(Arc::new(commands)))))
}

/// `tools/call` -> (isError, the first text block).
fn call(server: &mut Server, name: &str, args: Value) -> (bool, String) {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string();
    let reply: Value = serde_json::from_str(&server.handle_line(&line).unwrap()).unwrap();
    let result = &reply["result"];
    (result["isError"] == true, result["content"][0]["text"].as_str().unwrap_or("").to_string())
}

/// `tools/call` that must succeed; returns its text.
fn ok(server: &mut Server, name: &str, args: Value) -> String {
    let (err, text) = call(server, name, args.clone());
    assert!(!err, "{name} {args}: {text}");
    text
}

/// `tools/call` that must be refused for naming `command`, which is off the list.
fn refused(server: &mut Server, name: &str, args: Value, command: &str) {
    let (err, text) = call(server, name, args.clone());
    assert!(err, "{name} {args} should be refused: {text}");
    assert!(text.contains(command) && text.contains("not allowed"), "{name}: {text}");
}

/// The text of the active document.
fn body(server: &mut Server) -> String {
    let doc: Value = serde_json::from_str(&ok(server, "inspect_document", json!({}))).unwrap();
    doc["blocks"].as_array().unwrap().iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")
}

const EDITING: &str = "text.*, format.*, para.*, select.text, caret.*, file.new, view.switchWindows, document.text";

#[test]
fn listed_commands_run_and_the_rest_are_refused() {
    let mut s = allowed(EDITING);
    ok(&mut s, "new_document", json!({}));
    ok(&mut s, "type_text", json!({"text": "Report"}));
    ok(&mut s, "execute", json!({"command": "para.heading1"}));
    ok(&mut s, "select_text", json!({"text": "Report"}));
    ok(&mut s, "execute", json!({"command": "format.bold"}));
    let doc: Value = serde_json::from_str(&ok(&mut s, "inspect_document", json!({}))).unwrap();
    assert_eq!(doc["blocks"][0]["style"], "Heading1", "a listed command runs the commands it needs itself (para.style)");

    refused(&mut s, "execute", json!({"command": "insert.table", "params": {"rows": 2, "cols": 2}}), "insert.table");
    refused(&mut s, "execute", json!({"command": "file.saveAs", "params": {"path": "x.docx"}}), "file.saveAs");
    assert!(!body(&mut s).contains('\t'), "nothing ran");
}

#[test]
fn a_batch_stops_at_the_first_command_off_the_list() {
    let mut s = allowed(EDITING);
    let (err, text) = call(
        &mut s,
        "batch",
        json!({"commands": [{"command": "text.insert", "params": {"text": "one"}}, {"command": "file.open", "params": {"path": "x.docx"}}, {"command": "text.insert", "params": {"text": "two"}}]}),
    );
    assert!(err && text.contains("file.open") && text.contains("not allowed"), "{text}");
    let b = body(&mut s);
    assert!(b.contains("one") && !b.contains("two"), "{b}");
    // keepGoing runs the others but still refuses the one off the list.
    let (_, text) = call(
        &mut s,
        "batch",
        json!({"keepGoing": true, "commands": [{"command": "tools.macros"}, {"command": "text.insert", "params": {"text": "three"}}]}),
    );
    assert!(text.contains("tools.macros") && text.contains("not allowed"), "{text}");
    assert!(body(&mut s).contains("three"));
}

#[test]
fn a_macro_is_not_a_way_around_the_list() {
    // The macro command itself is allowed here, the step it would play is not.
    let mut s = allowed("text.insert, tools.macros, document.text");
    ok(
        &mut s,
        "execute",
        json!({"command": "tools.macros", "params": {"define": {"name": "sneak", "steps": [{"command": "text.insert", "params": {"text": "ok "}}, {"command": "insert.table", "params": {"rows": 1, "cols": 1}}]}}}),
    );
    refused(&mut s, "execute", json!({"command": "tools.macros", "params": {"run": "sneak"}}), "insert.table");
    let doc: Value = serde_json::from_str(&ok(&mut s, "inspect_document", json!({}))).unwrap();
    assert!(!doc["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "table"), "the table step never ran: {doc}");
}

#[test]
fn the_tools_built_on_commands_are_held_to_the_list() {
    let mut s = allowed("text.insert, document.text");
    refused(&mut s, "open_document", json!({"path": "x.docx"}), "file.open");
    refused(&mut s, "save_document", json!({"path": "x.docx"}), "file.save");
    refused(&mut s, "save_document", json!({"path": "x.png"}), "file.exportPng");
    refused(&mut s, "new_document", json!({}), "file.new");
    refused(&mut s, "select_text", json!({"text": "x"}), "select.text");
    ok(&mut s, "type_text", json!({"text": "still typing"}));
    assert!(ok(&mut s, "get_text", json!({})).contains("still typing"));
    // Reading needs no command.
    ok(&mut s, "inspect_document", json!({}));
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "render_page", "arguments": {"page": 1, "scale": 0.25}}})
        .to_string();
    let reply: Value = serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap();
    assert_eq!(reply["result"]["content"][0]["type"], "image");
}

#[test]
fn list_commands_shows_only_what_may_run() {
    let mut s = allowed("format.*, text.insert");
    let list: Value = serde_json::from_str(&ok(&mut s, "list_commands", json!({}))).unwrap();
    let ids: Vec<&str> = list.as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"format.bold") && ids.contains(&"text.insert"), "{ids:?}");
    assert!(ids.iter().all(|id| id.starts_with("format.") || *id == "text.insert"), "{ids:?}");
}

#[test]
fn without_a_list_nothing_changes() {
    let mut s = Server::new(Box::new(Headless::default()));
    ok(&mut s, "execute", json!({"command": "insert.table", "params": {"rows": 1, "cols": 1}}));
    let list: Value = serde_json::from_str(&ok(&mut s, "list_commands", json!({}))).unwrap();
    assert!(list.as_array().unwrap().len() > 400);
}
