//! MCP tools: names, schemas and implementations on top of a [`Backend`].

use serde_json::{Value, json};

use crate::backend::Backend;

/// An image tool result from a reply carrying base64 PNG in `png`.
fn png_result(v: &Value, info: &Value) -> ToolResult {
    match v.get("png").and_then(Value::as_str) {
        Some(png) if !png.is_empty() => ToolResult::image(png.to_string(), info),
        _ => ToolResult::error("the reply carried no picture"),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: false }
    }
    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(v).unwrap_or_default())
    }
    pub fn error(t: impl Into<String>) -> Self {
        let message = t.into();
        let text = match message.split_once(':') {
            Some((code, _)) if !code.is_empty() && code.chars().all(|c| c.is_ascii_uppercase() || c == '_') => {
                json!({"errorCode":code,"error":message}).to_string()
            }
            _ => message,
        };
        Self { content: vec![json!({"type": "text", "text": text})], is_error: true }
    }
    pub fn image(png_base64: String, info: &Value) -> Self {
        Self {
            content: vec![json!({"type": "image", "data": png_base64, "mimeType": "image/png"}), json!({"type": "text", "text": info.to_string()})],
            is_error: false,
        }
    }
    pub fn to_value(&self) -> Value {
        json!({"content": self.content, "isError": self.is_error})
    }
}

fn s(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}
fn n(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}
fn obj(props: Value, req: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": req})
}
fn tool(name: &str, title: &str, desc: &str, schema: Value, ro: bool) -> Value {
    json!({"name": name, "title": title, "description": desc, "inputSchema": schema, "annotations": {"title": title, "readOnlyHint": ro, "destructiveHint": false, "openWorldHint": false}})
}

pub fn tool_definitions() -> Value {
    json!([
        tool(
            "list_commands",
            "List commands",
            "Every WordCraft command: id, label, ribbon location, shortcut and params. Filter with `query`.",
            obj(json!({"query": s("Case-insensitive filter on id/label/location")}), &[]),
            true
        ),
        tool(
            "execute",
            "Run a command",
            "Run one command by id with params, e.g. {command:\"format.bold\"} or {command:\"insert.table\", params:{rows:3, cols:4}}.",
            obj(json!({"command": s("Command id"), "params": {"type": "object", "description": "Command parameters"}}), &["command"]),
            false
        ),
        tool(
            "batch",
            "Run several commands",
            "Run commands in order; stops at the first error unless keepGoing.",
            obj(
                json!({"commands": {"type": "array", "items": {"type": "object", "properties": {"command": {"type": "string"}, "params": {"type": "object"}}, "required": ["command"]}}, "keepGoing": {"type": "boolean"}}),
                &["commands"]
            ),
            false
        ),
        tool(
            "new_document",
            "New document",
            "Start a document: blank, sample, letter, resume or report.",
            obj(json!({"template": s("blank|sample|letter|resume|report")}), &[]),
            false
        ),
        tool(
            "open_document",
            "Open",
            "Open a .docx, .doc, .odt, .rtf, .md, .tex, .html, .txt or .json file, or any other text file (.m3u, .log, .ini…) as plain text.",
            obj(json!({"path": s("File path")}), &["path"]),
            false
        ),
        tool(
            "save_document",
            "Save",
            "Save (format from the extension: docx, pdf, odt, rtf, html, md, tex, txt, png).",
            obj(json!({"path": s("File path; omit to save in place")}), &[]),
            false
        ),
        tool(
            "type_text",
            "Type text",
            "Type at the caret (replacing the selection). Use \\n... for line breaks inside text; use execute text.newParagraph for new paragraphs, or `paragraphs: true` to split lines into paragraphs.",
            obj(json!({"text": s("Text"), "paragraphs": {"type": "boolean"}}), &["text"]),
            false
        ),
        tool(
            "select_text",
            "Select text",
            "Select the n-th occurrence of some text.",
            obj(json!({"text": s("Text to find"), "occurrence": n("1-based")}), &["text"]),
            false
        ),
        tool("get_text", "Document text", "Body text with tracked deletions omitted.", obj(json!({}), &[]), true),
        tool(
            "inspect_document",
            "Inspect document",
            "Structure: blocks with text, styles, formatting runs, lists, tables, sections, parts, selection, pages.",
            obj(json!({}), &[]),
            true
        ),
        tool(
            "render_page",
            "Render page",
            "Render a page to PNG to look at the result.",
            obj(json!({"page": n("1-based page"), "scale": n("Pixels per point (default 1)")}), &[]),
            true
        ),
        tool("parity", "Feature parity", "WordCraft's command coverage of the word-processor feature catalog.", obj(json!({}), &[]), true),
        tool("screenshot", "Screenshot app", "Screenshot of the whole WordCraft window (desktop app only).", obj(json!({}), &[]), true),
        tool(
            "click",
            "Click",
            "Click at window coordinates (desktop app only).",
            obj(json!({"x": n("x"), "y": n("y"), "count": n("1-3 clicks"), "shift": {"type": "boolean"}, "cmd": {"type": "boolean"}}), &["x", "y"]),
            false
        ),
        tool(
            "key",
            "Press key",
            "Press a key with modifiers (desktop app only), e.g. {key:\"B\", cmd:true}.",
            obj(json!({"key": s("Key name"), "shift": {"type": "boolean"}, "alt": {"type": "boolean"}, "cmd": {"type": "boolean"}}), &["key"]),
            false
        ),
        tool(
            "ui_inspect",
            "Inspect UI",
            "UI state: tab, panes, dialog, page rectangles on screen, caret (desktop app only).",
            obj(json!({}), &[]),
            true
        ),
        tool(
            "wait_for_request",
            "Wait for an Ask Agent request",
            WAIT_DESCRIPTION,
            json!({"type":"object","properties":{"timeout_s":{"type":"number","minimum":0,"default":30,"description":"Seconds to wait; capped at 60."},"agent":{"type":"string","description":"Optional display name; otherwise initialized MCP client name."}},"required":[],"additionalProperties":false}),
            true
        ),
        tool(
            "reply_to_request",
            "Reply to an Ask Agent request",
            REPLY_DESCRIPTION,
            obj(
                json!({
                    "id": n("The request's id, from wait_for_request"),
                    "text": s("What to show the person: a progress note, or your answer"),
                    "error": s("Failure message: closes the request as failed"),
                    "done": {"type": "boolean", "description": "true (default) closes the request; false is a progress note"},
                }),
                &["id"]
            ),
            false
        ),
    ])
}

const WAIT_DESCRIPTION: &str = "Ask Agent: wait for the next request from the WordCraft window. \
Returns {id, instruction, history, selection, document}, or {none: true} when nothing came within timeout_s. \
Edit the document with normal commands tagged with request id, and answer with reply_to_request.";

const REPLY_DESCRIPTION: &str = "Ask Agent: show a note or your answer for a request in the WordCraft window. \
Send a progress note with done: false, or your final answer with done: true (the default), which closes the request.";

/// Longest `wait_for_request` wait, in seconds (the app's own cap on `agent.listen`).
const MAX_WAIT_S: f64 = 60.0;

fn exec(b: &mut dyn Backend, id: &str, params: Value) -> Result<Value, String> {
    b.call("engine.execute", json!({"command": id, "params": params}))
}

fn wrap(r: Result<Value, String>) -> ToolResult {
    match r {
        Ok(v) => ToolResult::json(&v),
        Err(e) => ToolResult::error(e),
    }
}

pub fn call_tool(b: &mut dyn Backend, name: &str, a: &Value) -> ToolResult {
    let st = |k: &str| a.get(k).and_then(Value::as_str);
    match name {
        "list_commands" => {
            let q = st("query").unwrap_or("").to_lowercase();
            match b.call("engine.commands", json!({})) {
                Ok(Value::Array(v)) => {
                    let f: Vec<Value> = v
                        .into_iter()
                        .filter(|c| {
                            q.is_empty()
                                || ["id", "label", "location"]
                                    .iter()
                                    .any(|k| c.get(*k).and_then(Value::as_str).is_some_and(|x| x.to_lowercase().contains(&q)))
                        })
                        .collect();
                    ToolResult::json(&Value::Array(f))
                }
                Ok(v) => ToolResult::json(&v),
                Err(e) => ToolResult::error(e),
            }
        }
        "execute" => match st("command") {
            Some(c) => wrap(exec(b, c, a.get("params").cloned().unwrap_or(json!({})))),
            None => ToolResult::error("missing `command`"),
        },
        "batch" => {
            let keep = a.get("keepGoing").and_then(Value::as_bool).unwrap_or(false);
            let mut out = Vec::new();
            for c in a.get("commands").and_then(Value::as_array).cloned().unwrap_or_default() {
                let id = c.get("command").and_then(Value::as_str).unwrap_or("");
                let r = exec(b, id, c.get("params").cloned().unwrap_or(json!({})));
                let failed = r.is_err();
                out.push(match r {
                    Ok(v) => json!({"command": id, "ok": true, "result": v}),
                    Err(e) => json!({"command": id, "ok": false, "error": e}),
                });
                if failed && !keep {
                    return ToolResult { content: vec![json!({"type": "text", "text": Value::Array(out).to_string()})], is_error: true };
                }
            }
            ToolResult::json(&Value::Array(out))
        }
        "new_document" => wrap(exec(b, "file.new", json!({"template": st("template").unwrap_or("blank")}))),
        "open_document" => wrap(exec(b, "file.open", json!({"path": st("path").unwrap_or("")}))),
        "save_document" => {
            let p = st("path");
            let ext = p.and_then(|x| x.rsplit('.').next()).unwrap_or("").to_ascii_lowercase();
            if ext == "png" {
                wrap(exec(b, "file.exportPng", json!({"path": p})))
            } else {
                wrap(exec(
                    b,
                    "file.save",
                    match p {
                        Some(p) => json!({"path": p}),
                        None => json!({}),
                    },
                ))
            }
        }
        "type_text" => {
            let t = st("text").unwrap_or("");
            if a.get("paragraphs").and_then(Value::as_bool).unwrap_or(false) {
                let mut last = Ok(Value::Null);
                for (i, line) in t.split('\n').enumerate() {
                    if i > 0 {
                        last = exec(b, "text.newParagraph", json!({}));
                        if last.is_err() {
                            break;
                        }
                    }
                    if !line.is_empty() {
                        last = exec(b, "text.insert", json!({"text": line, "raw": true}));
                    }
                    if last.is_err() {
                        break;
                    }
                }
                wrap(last)
            } else {
                wrap(exec(b, "text.insert", json!({"text": t, "raw": true})))
            }
        }
        "select_text" => {
            wrap(exec(b, "select.text", json!({"text": st("text").unwrap_or(""), "occurrence": a.get("occurrence").cloned().unwrap_or(json!(1))})))
        }
        "get_text" => wrap(exec(b, "document.text", json!({}))),
        "inspect_document" => wrap(b.call("document.inspect", json!({}))),
        "parity" => wrap(b.call("ui.parity", json!({}))),
        // Pictures travel inside the reply (`png`), headless or through the app: no file with a
        // predictable name in the shared temporary folder.
        "render_page" => {
            let params = json!({"page": a.get("page").cloned().unwrap_or(json!(1)), "scale": a.get("scale").cloned().unwrap_or(json!(1.0))});
            match b.call("ui.render", params) {
                Ok(v) => png_result(&v, &json!({"width": v["width"], "height": v["height"], "pages": v["pages"]})),
                Err(e) => ToolResult::error(e),
            }
        }
        "screenshot" => match b.call("ui.screenshot", json!({})) {
            Ok(v) => png_result(&v, &json!({"width": v["width"], "height": v["height"]})),
            Err(e) => ToolResult::error(e),
        },
        "click" => wrap(b.call("ui.click", a.clone())),
        "key" => wrap(b.call("ui.key", a.clone())),
        "ui_inspect" => wrap(b.call("ui.inspect", json!({}))),
        "wait_for_request" | "reply_to_request" if !b.has_ui() => ToolResult::error(crate::ASK_NEEDS_APP),
        "wait_for_request" => {
            if a.as_object().is_none_or(|o| o.keys().any(|k| !["timeout_s", "agent"].contains(&k.as_str()))) {
                return ToolResult::error("INVALID_PARAMS: unknown wait parameter");
            }
            let secs = match a.get("timeout_s") {
                None => 30.0,
                Some(v) => match v.as_f64().filter(|n| n.is_finite() && *n >= 0.0) {
                    Some(n) => n.min(MAX_WAIT_S),
                    None => return ToolResult::error("INVALID_PARAMS: timeout_s must be nonnegative"),
                },
            };
            let mut params = json!({"timeoutMs":(secs*1000.0).floor() as u64});
            if let Some(agent) = a.get("agent") {
                if !agent.is_string() {
                    return ToolResult::error("INVALID_PARAMS: agent must be a string");
                }
                params["agent"] = agent.clone();
            }
            wrap(b.call("agent.listen", params))
        }
        "reply_to_request" => {
            let Some(id) = a.get("id").and_then(Value::as_u64) else {
                return ToolResult::error("missing `id`: the request's id from wait_for_request");
            };
            let mut params = json!({"id": id});
            if let Some(text) = st("text") {
                params["text"] = json!(text);
            }
            if let Some(error) = st("error") {
                params["error"] = json!(error);
            }
            if params.get("text").is_none() && params.get("error").is_none() {
                return ToolResult::error("missing `text` (what to show the person) or `error`");
            }
            params["done"] = json!(a.get("done").and_then(Value::as_bool).unwrap_or(true));
            wrap(b.call("agent.reply", params))
        }
        other => ToolResult::error(format!("unknown tool `{other}`")),
    }
}
