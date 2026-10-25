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

/// The document's paragraphs as (text, in a list) from `inspect_document`.
fn paragraphs(s: &mut Server) -> Vec<(String, bool)> {
    let doc = tool(s, "inspect_document", json!({}));
    doc["blocks"].as_array().unwrap().iter().map(|b| (b["text"].as_str().unwrap_or("").to_string(), !b["list"].is_null())).collect()
}

/// `type_text` with `paragraphs` is one command: it used to be a `text.newParagraph` and a
/// `text.insert` per line, each snapshotting the whole document for Undo (quadratic: 10,000 lines
/// took 2.8 s; one command takes 7 ms) and each its own Undo step (#3973). The document it makes
/// is the one the per-line way made, a CRLF is one break, and one Undo takes all of it back.
#[test]
fn type_text_paragraphs_is_one_step_and_makes_the_same_paragraphs() {
    let per_line = |s: &mut Server, text: &str| {
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                tool(s, "execute", json!({"command": "text.newParagraph"}));
            }
            if !line.is_empty() {
                tool(s, "type_text", json!({"text": line}));
            }
        }
    };
    let setup = |s: &mut Server, listed: bool| {
        tool(s, "new_document", json!({}));
        tool(s, "type_text", json!({"text": "Start"}));
        if listed {
            tool(s, "execute", json!({"command": "para.bullets"}));
        }
    };
    for listed in [false, true] {
        // Text AutoCorrect has nothing to change, so only the paragraphs are compared.
        for text in ["One\nTwo\nThree", "One\n\n\nTwo", "\nLead", "Trail\n", "A b  c\n  Indented\n\tTabbed", "X"] {
            let mut old = Server::new(Box::new(Headless::default()));
            setup(&mut old, listed);
            per_line(&mut old, text);
            let mut new = Server::new(Box::new(Headless::default()));
            setup(&mut new, listed);
            tool(&mut new, "type_text", json!({"text": text, "paragraphs": true}));
            assert_eq!(paragraphs(&mut new), paragraphs(&mut old), "{text:?} (in a list: {listed})");
        }
    }

    // The lines are typed as they are: Enter's AutoCorrect (a capital for the next sentence, a
    // fixed typo) doesn't run on the breaks. The per-line way capitalised "two" (and not the last
    // line, which no Enter followed), so what was written depended on the line's place.
    let mut s = Server::new(Box::new(Headless::default()));
    tool(&mut s, "new_document", json!({}));
    tool(&mut s, "type_text", json!({"text": "one\ntwo teh\nthree", "paragraphs": true}));
    let texts: Vec<String> = paragraphs(&mut s).into_iter().map(|p| p.0).collect();
    assert_eq!(texts, ["one", "two teh", "three"]);

    // CRLF is one break per line end.
    let mut s = Server::new(Box::new(Headless::default()));
    tool(&mut s, "new_document", json!({}));
    tool(&mut s, "type_text", json!({"text": "a\r\nb\r\nc", "paragraphs": true}));
    let texts: Vec<String> = paragraphs(&mut s).into_iter().map(|p| p.0).collect();
    assert_eq!(texts, ["a", "b", "c"]);

    // One Undo step takes all the lines back.
    let mut s = Server::new(Box::new(Headless::default()));
    tool(&mut s, "new_document", json!({}));
    tool(&mut s, "type_text", json!({"text": "keep"}));
    // Moving the caret ends the typing that "keep" was part of, so it isn't undone with the rest.
    tool(&mut s, "execute", json!({"command": "caret.docEnd"}));
    tool(&mut s, "type_text", json!({"text": "\none\ntwo\nthree\nfour", "paragraphs": true}));
    assert_eq!(paragraphs(&mut s).len(), 5);
    tool(&mut s, "execute", json!({"command": "edit.undo"}));
    let texts: Vec<String> = paragraphs(&mut s).into_iter().map(|p| p.0).collect();
    assert_eq!(texts, ["keep"], "one Undo removes every line");
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

/// A backend that refuses every text insertion (a protected document, say).
struct RefusesInsert {
    calls: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
}

impl crate::backend::Backend for RefusesInsert {
    fn call(&mut self, _method: &str, params: Value) -> Result<Value, String> {
        let command = params["command"].as_str().unwrap_or("").to_string();
        self.calls.lock().unwrap().push((command.clone(), params["params"].clone()));
        match command.as_str() {
            "text.insert" => Err("editing refused".into()),
            _ => Ok(json!({"ok": true})),
        }
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "refuses edits".into()
    }
}

/// #1356: a refused edit is reported and nothing is typed after it. The lines are one
/// `text.insert` now (#3973), so a protected document refuses all of them at once, never a
/// line at a time with the next one typed after the break that failed.
#[test]
fn type_text_paragraphs_reports_a_refusal_and_types_nothing_after_it() {
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut s = Server::new(Box::new(RefusesInsert { calls: calls.clone() }));
    let r = call(&mut s, 11, "tools/call", json!({"name": "type_text", "arguments": {"text": "one\ntwo", "paragraphs": true}}));
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("editing refused"), "{r}");
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "one command for all the lines: {calls:?}");
    assert_eq!(calls[0].0, "text.insert");
    assert_eq!(calls[0].1["text"], "one\rtwo", "the line break is a paragraph break");
}

/// #3685: an over-long request line gets an error reply and the server keeps serving.
#[test]
fn over_long_request_line_is_rejected_and_serving_continues() {
    let mut s = Server::new(Box::new(Headless::default()));
    let mut input = vec![b'a'; (1 << 20) + 10];
    input.push(b'\n');
    input.extend_from_slice(br#"{"jsonrpc":"2.0","id":5,"method":"ping"}"#);
    input.push(b'\n');
    let mut out = Vec::new();
    s.serve(std::io::Cursor::new(input), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains("too long"), "{}", lines[0]);
    assert!(lines[1].contains(r#""id":5"#), "{}", lines[1]);
}
