//! An MCP client doing realistic tasks through the protocol only, checking results via MCP.

use serde_json::{Value, json};

use crate::{Headless, Server};

fn call(s: &mut Server, id: u64, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let r = s.handle_line(&line).expect("reply");
    serde_json::from_str(&r).expect("json")
}

fn tool(s: &mut Server, name: &str, args: Value) -> Value {
    let r = call(s, 9, "tools/call", json!({"name": name, "arguments": args}));
    let res = &r["result"];
    assert_eq!(res["isError"], false, "{name}: {res}");
    let text = res["content"][0]["text"].as_str().unwrap_or("null");
    serde_json::from_str(text).unwrap_or(Value::String(text.to_string()))
}

#[test]
fn lifecycle_and_tools() {
    let mut s = Server::new(Box::new(Headless::default()));
    let init = call(&mut s, 1, "initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(init["result"]["serverInfo"]["name"], "wordcraft");
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    let tools = call(&mut s, 2, "tools/list", json!({}));
    assert!(tools["result"]["tools"].as_array().unwrap().len() >= 12);
    let bad = call(&mut s, 3, "nope", json!({}));
    assert!(bad["error"].is_object());
    assert!(s.handle_line("{not json").unwrap().contains("parse error"));
}

#[test]
fn agent_writes_a_formatted_document() {
    let mut s = Server::new(Box::new(Headless::default()));
    tool(&mut s, "new_document", json!({}));
    tool(&mut s, "type_text", json!({"text": "Quarterly Report"}));
    tool(&mut s, "execute", json!({"command": "para.style", "params": {"style": "Heading 1"}}));
    tool(&mut s, "execute", json!({"command": "text.newParagraph"}));
    tool(&mut s, "type_text", json!({"text": "Sales grew strongly this quarter.\nCosts fell.", "paragraphs": true}));
    tool(&mut s, "select_text", json!({"text": "strongly"}));
    tool(&mut s, "execute", json!({"command": "format.bold"}));
    tool(&mut s, "batch", json!({"commands": [{"command": "caret.docEnd"}, {"command": "insert.table", "params": {"rows": 2, "cols": 2}}]}));
    let doc = tool(&mut s, "inspect_document", json!({}));
    let blocks = doc["blocks"].as_array().unwrap();
    assert_eq!(blocks[0]["style"], "Heading1");
    assert_eq!(blocks[0]["text"], "Quarterly Report");
    let runs = blocks[1]["runs"].as_array().unwrap();
    assert!(runs.iter().any(|r| r["props"]["bold"] == true));
    assert!(blocks.iter().any(|b| b["type"] == "table"));
    let text = tool(&mut s, "get_text", json!({}));
    assert!(text["text"].as_str().unwrap().contains("Costs fell."));
    let r = call(&mut s, 10, "tools/call", json!({"name": "render_page", "arguments": {"page": 1, "scale": 0.5}}));
    assert_eq!(r["result"]["content"][0]["type"], "image");
    let r = call(&mut s, 11, "tools/call", json!({"name": "screenshot", "arguments": {}}));
    assert_eq!(r["result"]["isError"], true);
    let p = tool(&mut s, "parity", json!({}));
    assert!(p["percent"].as_f64().unwrap() > 50.0);
}

#[test]
fn resources() {
    let mut s = Server::new(Box::new(Headless::default()));
    let l = call(&mut s, 1, "resources/list", json!({}));
    assert_eq!(l["result"]["resources"].as_array().unwrap().len(), 2);
    let r = call(&mut s, 2, "resources/read", json!({"uri": "wordcraft://document"}));
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("blocks"));
}

/// A backend standing in for a running app: it answers `ui.render` and `ui.screenshot` with a
/// PNG in the reply and records what it was asked.
struct FakeApp {
    calls: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
}

impl crate::backend::Backend for FakeApp {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.calls.lock().unwrap().push((method.to_string(), params.clone()));
        match method {
            "ui.render" | "ui.screenshot" => Ok(json!({"png": wordcraft_engine::cmd::insert::base64_encode(b"\x89PNG"), "width": 1, "height": 1})),
            other => Err(format!("unexpected {other}")),
        }
    }
    fn has_ui(&self) -> bool {
        true
    }
    fn describe(&self) -> String {
        "fake app".into()
    }
}

/// With an app attached, pictures travel inside the reply: the bridge never writes a file with a
/// predictable name into the shared temporary folder (another user could plant a link there).
#[test]
fn render_and_screenshot_through_the_app_use_no_temp_file() {
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut s = Server::new(Box::new(FakeApp { calls: calls.clone() }));
    for (tool, method) in [("render_page", "ui.render"), ("screenshot", "ui.screenshot")] {
        let r = call(&mut s, 10, "tools/call", json!({"name": tool, "arguments": {"page": 1}}));
        let content = &r["result"]["content"];
        assert_eq!(r["result"]["isError"], false, "{tool}: {r}");
        assert_eq!(content[0]["type"], "image", "{tool}");
        assert_eq!(content[0]["data"], wordcraft_engine::cmd::insert::base64_encode(b"\x89PNG"), "{tool}");
        let (m, params) = calls.lock().unwrap().pop().expect("one call");
        assert_eq!(m, method);
        assert!(params.get("path").is_none(), "{tool} asked for a file: {params}");
    }
}

/// #40: a headless session never reads or writes the person's custom dictionary file: an agent's
/// Add to Dictionary stays in memory (only the desktop app saves the dictionary).
#[test]
fn headless_sessions_keep_the_dictionary_in_memory() {
    let mut h = Headless::default();
    assert!(h.session.user_dict_path.is_none());
    let mut s = Server::new(Box::new(Headless::default()));
    tool(&mut s, "execute", json!({"command": "review.addToDictionary", "params": {"word": "zxqmcpword"}}));
    h.session.run("review.addToDictionary", &json!({"word": "zxqmcpword2"})).expect("added");
    assert!(h.session.user_dict_path.is_none());
}

/// A backend whose paragraph break is refused while text insertion would succeed.
struct RefusesParagraph {
    calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl crate::backend::Backend for RefusesParagraph {
    fn call(&mut self, _method: &str, params: Value) -> Result<Value, String> {
        let command = params["command"].as_str().unwrap_or("").to_string();
        self.calls.lock().unwrap().push(command.clone());
        match command.as_str() {
            "text.newParagraph" => Err("paragraph break refused".into()),
            _ => Ok(json!({"ok": true})),
        }
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "refuses paragraphs".into()
    }
}

/// #1356: a refused paragraph break is reported and the next line is not typed after it.
#[test]
fn type_text_stops_at_a_refused_paragraph_break() {
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut s = Server::new(Box::new(RefusesParagraph { calls: calls.clone() }));
    let r = call(&mut s, 11, "tools/call", json!({"name": "type_text", "arguments": {"text": "one\ntwo", "paragraphs": true}}));
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("paragraph break refused"), "{r}");
    assert_eq!(*calls.lock().unwrap(), vec!["text.insert", "text.newParagraph"]);
}

/// #2242: `insert_table` fills a table in one call; the header row and style survive a DOCX round trip.
#[test]
fn insert_table_fills_a_table_and_survives_docx() {
    let mut s = Server::new(Box::new(Headless::default()));
    tool(&mut s, "new_document", json!({}));
    tool(&mut s, "insert_table", json!({"values": [["Month", "Cases"], ["January", 12], ["February", 7]], "header": true, "style": "TableGrid"}));
    let path = std::env::temp_dir().join(format!("wordcraft-mcp-table-{}.docx", std::process::id()));
    let path_str = path.to_str().expect("utf-8 path").to_string();
    tool(&mut s, "save_document", json!({"path": path_str}));
    let mut s2 = Server::new(Box::new(Headless::default()));
    tool(&mut s2, "open_document", json!({"path": path_str}));
    let doc = tool(&mut s2, "inspect_document", json!({}));
    let table = doc["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "table").expect("a table");
    let text = table.to_string();
    for cell in ["Month", "Cases", "January", "12", "February", "7"] {
        assert!(text.contains(cell), "{cell} missing in {text}");
    }
    assert!(text.contains("TableGrid"), "{text}");
    // The header flag is not in `inspect_document`; read it from the reopened document.
    let mut h = Headless::default();
    h.session.run("file.open", &json!({"path": path_str})).expect("reopened");
    let _ = std::fs::remove_file(&path);
    let table = h.session.doc.body.iter().find_map(|b| match &**b {
        wordcraft_doc::Block::Table(t) => Some(t.clone()),
        _ => None,
    });
    let table = table.expect("a table");
    assert!(table.rows[0].props.header && !table.rows[1].props.header);
}

/// #2242: malformed `insert_table` arguments are tool errors, not panics.
#[test]
fn insert_table_rejects_hostile_values() {
    let mut s = Server::new(Box::new(Headless::default()));
    let wide: Vec<Value> = (0..64).map(|i| json!(i)).collect();
    for args in [json!({}), json!({"values": "x"}), json!({"values": [["a", "b"], ["c"]]}), json!({"values": [wide]}), json!({"values": [[{"a": 1}]]}), json!({"values": []})] {
        let r = call(&mut s, 12, "tools/call", json!({"name": "insert_table", "arguments": args}));
        assert_eq!(r["result"]["isError"], true, "{args}: {r}");
    }
}
