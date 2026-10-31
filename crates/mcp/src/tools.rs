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
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: true }
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
/// Adds the optional `doc` property to a tool's schema.
fn with_doc(mut schema: Value) -> Value {
    if let Some(p) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        p.insert("doc".into(), json!({"type": "integer", "minimum": 0, "description": DOC_DESC}));
    }
    schema
}

const DOC_DESC: &str = "Id of the open document to work on (list_documents); the active document when left out. The active document stays active.";

fn tool(name: &str, title: &str, desc: &str, schema: Value, ro: bool) -> Value {
    json!({"name": name, "title": title, "description": desc, "inputSchema": schema, "annotations": {"title": title, "readOnlyHint": ro, "openWorldHint": false}})
}

/// Every MCP tool with its title, description, input schema and annotations (`tools/list`).
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
            "Run one command by id with params, e.g. {command:\"format.bold\"} or {command:\"insert.table\", params:{rows:3, cols:4}}. Commands work on the active document, or on the open document `doc` names. file.new and file.open open another document beside the open ones and make it active; view.switchWindows {id} makes another one active and file.closeDocument {id, force?} closes one. `author` signs this command's comments and tracked changes, so an agent's edits are told apart from the person's.",
            with_doc(obj(
                json!({"command": s("Command id"), "params": {"type": "object", "description": "Command parameters"}, "author": s("Name that signs this command's comments and tracked changes (the document's author when absent)")}),
                &["command"]
            )),
            false
        ),
        tool(
            "batch",
            "Run several commands",
            "Run commands in order; stops at the first error unless keepGoing. `doc` sends every command to that open document.",
            with_doc(obj(
                json!({"commands": {"type": "array", "items": {"type": "object", "properties": {"command": {"type": "string"}, "params": {"type": "object"}}, "required": ["command"]}}, "keepGoing": {"type": "boolean"}}),
                &["commands"]
            )),
            false
        ),
        tool(
            "new_document",
            "New document",
            "Start a document: blank, sample, letter, resume or report. It opens beside the documents already open and becomes the active one; the reply's `id` addresses it (`doc`).",
            obj(json!({"template": s("blank|sample|letter|resume|report")}), &[]),
            false
        ),
        tool(
            "open_document",
            "Open",
            "Open a .docx, .doc, .odt, .rtf, .md, .tex, .html, .txt or .json file, or any other text file (.m3u, .log, .ini…) as plain text. Give `password` for a protected file; without it the call fails asking for one. The file opens beside the documents already open (it replaces a blank one nobody has used) and becomes the active one; a file that is already open becomes active again (`alreadyOpen`). The reply's `id` addresses it (`doc`).",
            obj(json!({"path": s("File path"), "password": s("Password, if the file is protected")}), &["path"]),
            false
        ),
        tool(
            "save_document",
            "Save",
            "Save (format from the extension: docx, pdf, odt, rtf, html, md, tex, txt, png).",
            with_doc(obj(json!({"path": s("File path; omit to save in place")}), &[])),
            false
        ),
        tool(
            "type_text",
            "Type text",
            &format!(
                "Type at the caret (replacing the selection). Use \\n... for line breaks inside text; use execute text.newParagraph for new paragraphs, or `paragraphs: true` to split lines into paragraphs (one undo step, at most {} lines).",
                wordcraft_engine::cmd::text::MAX_TYPED_PARAGRAPHS
            ),
            with_doc(obj(json!({"text": s("Text"), "paragraphs": {"type": "boolean"}}), &["text"])),
            false
        ),
        tool(
            "select_text",
            "Select text",
            "Select the n-th occurrence of some text.",
            with_doc(obj(json!({"text": s("Text to find"), "occurrence": n("1-based")}), &["text"])),
            false
        ),
        tool("get_text", "Document text", "Body text with tracked deletions omitted.", with_doc(obj(json!({}), &[])), true),
        tool(
            "inspect_document",
            "Inspect document",
            "Structure: blocks with text, styles, formatting runs, lists, tables, sections, parts, selection, pages.",
            with_doc(obj(json!({}), &[])),
            true
        ),
        tool(
            "render_page",
            "Render page",
            "Render a page to PNG to look at the result.",
            with_doc(obj(json!({"page": n("1-based page"), "scale": n("Pixels per point (default 1)")}), &[])),
            true
        ),
        tool(
            "list_documents",
            "Open documents",
            "The open documents in the order they were opened: id, title, path, dirty (unsaved changes) and active (the one commands work on by default). new_document and open_document open another document beside these. Give a document's id as `doc` to execute, batch, type_text, select_text, get_text, inspect_document, render_page or save_document to work on it while it stays in the background; execute view.switchWindows {id} makes it the active one, file.closeDocument {id, force?} closes it.",
            obj(json!({}), &[]),
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
    ])
}

fn exec(b: &mut dyn Backend, id: &str, params: Value, doc: Option<&Value>) -> Result<Value, String> {
    b.call("engine.execute", addressed(json!({"command": id, "params": params}), doc))
}

/// `params` with the tool's `doc`, when it gave one. The backend checks it: a session says
/// whether it names an open document.
fn addressed(mut params: Value, doc: Option<&Value>) -> Value {
    if let (Some(d), Some(o)) = (doc, params.as_object_mut()) {
        o.insert("doc".into(), d.clone());
    }
    params
}

/// The active document's id, added to a reply that opened or made one.
fn with_active_id(b: &mut dyn Backend, r: Result<Value, String>) -> Result<Value, String> {
    let mut v = r?;
    if v.get("id").is_none()
        && let Ok(list) = exec(b, "view.switchWindows", json!({}), None)
        && let Some(id) =
            list.get("documents").and_then(Value::as_array).and_then(|d| d.iter().find(|d| d["active"] == true)).and_then(|d| d.get("id"))
        && let Some(o) = v.as_object_mut()
    {
        o.insert("id".into(), id.clone());
    }
    Ok(v)
}

fn wrap(r: Result<Value, String>) -> ToolResult {
    match r {
        Ok(v) => ToolResult::json(&v),
        Err(e) => ToolResult::error(e),
    }
}

/// Run tool `name` with arguments `a` through backend `b` (`tools/call`); unknown tools and
/// failed calls are error results, never panics.
pub fn call_tool(b: &mut dyn Backend, name: &str, a: &Value) -> ToolResult {
    let st = |k: &str| a.get(k).and_then(Value::as_str);
    let doc = a.get("doc").filter(|d| !d.is_null());
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
            Some(c) => {
                let mut call = json!({"command": c, "params": a.get("params").cloned().unwrap_or(json!({}))});
                if let (Some(author), Some(o)) = (a.get("author"), call.as_object_mut()) {
                    o.insert("author".into(), author.clone());
                }
                wrap(b.call("engine.execute", addressed(call, doc)))
            }
            None => ToolResult::error("missing `command`"),
        },
        "batch" => {
            let keep = a.get("keepGoing").and_then(Value::as_bool).unwrap_or(false);
            let mut out = Vec::new();
            for c in a.get("commands").and_then(Value::as_array).cloned().unwrap_or_default() {
                let id = c.get("command").and_then(Value::as_str).unwrap_or("");
                let r = exec(b, id, c.get("params").cloned().unwrap_or(json!({})), doc);
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
        "new_document" => {
            let r = exec(b, "file.new", json!({"template": st("template").unwrap_or("blank")}), None);
            wrap(with_active_id(b, r))
        }
        "open_document" => wrap(exec(b, "file.open", json!({"path": st("path").unwrap_or(""), "password": st("password")}), None)),
        "list_documents" => wrap(exec(b, "view.switchWindows", json!({}), None)),
        "save_document" => {
            let p = st("path");
            let ext = p.and_then(|x| x.rsplit('.').next()).unwrap_or("").to_ascii_lowercase();
            if ext == "png" {
                wrap(exec(b, "file.exportPng", json!({"path": p}), doc))
            } else {
                wrap(exec(
                    b,
                    "file.save",
                    match p {
                        Some(p) => json!({"path": p}),
                        None => json!({}),
                    },
                    doc,
                ))
            }
        }
        "type_text" => {
            let t = st("text").unwrap_or("");
            if a.get("paragraphs").and_then(Value::as_bool).unwrap_or(false) {
                // One `text.insert` with `paragraphs`: each break is Enter's own (its AutoFormat,
                // and its refusal in a one-line control), but the lines go in as one edit and one
                // undo step, all or nothing, instead of a command (and a whole-document undo
                // snapshot) per line. The engine splits the lines and holds them to its cap
                // (`MAX_TYPED_PARAGRAPHS`), for this tool and every other caller alike.
                wrap(exec(b, "text.insert", json!({"text": t, "raw": true, "paragraphs": true}), doc))
            } else {
                wrap(exec(b, "text.insert", json!({"text": t, "raw": true}), doc))
            }
        }
        "select_text" => wrap(exec(
            b,
            "select.text",
            json!({"text": st("text").unwrap_or(""), "occurrence": a.get("occurrence").cloned().unwrap_or(json!(1))}),
            doc,
        )),
        "get_text" => wrap(exec(b, "document.text", json!({}), doc)),
        "inspect_document" => wrap(b.call("document.inspect", addressed(json!({}), doc))),
        "parity" => wrap(b.call("ui.parity", json!({}))),
        // Pictures travel inside the reply (`png`), headless or through the app: no file with a
        // predictable name in the shared temporary folder.
        "render_page" => {
            let params =
                addressed(json!({"page": a.get("page").cloned().unwrap_or(json!(1)), "scale": a.get("scale").cloned().unwrap_or(json!(1.0))}), doc);
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
        other => ToolResult::error(format!("unknown tool `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that keeps what it was asked for, so a test can read the forwarded params.
    struct Recording {
        seen: Option<(String, Value)>,
    }

    impl Backend for Recording {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.seen = Some((method.to_string(), params));
            Ok(json!({"encrypted": true}))
        }
        fn has_ui(&self) -> bool {
            false
        }
        fn describe(&self) -> String {
            "recording".into()
        }
    }

    /// What the backend was asked for: `file.open` runs through `engine.execute`, so the
    /// command and its parameters are nested one level down.
    fn forwarded(args: Value) -> Value {
        let mut b = Recording { seen: None };
        let r = call_tool(&mut b, "open_document", &args);
        assert!(!r.is_error, "open_document failed: {:?}", r);
        let (method, mut call) = b.seen.expect("the backend was called");
        assert_eq!(method, "engine.execute");
        assert_eq!(call["command"], json!("file.open"));
        call["params"].take()
    }

    #[test]
    fn open_document_forwards_the_password_to_file_open() {
        let p = forwarded(json!({"path": "/tmp/a.docx", "password": "s3cret"}));
        assert_eq!(p["path"], json!("/tmp/a.docx"));
        assert_eq!(p["password"], json!("s3cret"));
    }

    /// Without a password the parameter stays null rather than an empty string: `file.open` reads
    /// an empty one as "no password given", so a protected file would come back unprotected.
    #[test]
    fn open_document_without_a_password_passes_null() {
        assert_eq!(forwarded(json!({"path": "/tmp/a.docx"}))["password"], Value::Null);
    }

    /// `doc` travels beside the command, not inside its params, so the app's control channel can
    /// run the command on that document; without one nothing is added.
    #[test]
    fn doc_is_forwarded_beside_the_command() {
        let mut b = Recording { seen: None };
        call_tool(&mut b, "execute", &json!({"command": "text.insert", "params": {"text": "x"}, "doc": 7}));
        let (_, call) = b.seen.take().expect("the backend was called");
        assert_eq!(call["doc"], json!(7));
        assert!(call["params"].get("doc").is_none(), "{call}");
        call_tool(&mut b, "inspect_document", &json!({"doc": 7}));
        assert_eq!(b.seen.take().expect("called").1["doc"], json!(7));
        call_tool(&mut b, "execute", &json!({"command": "text.insert", "params": {"text": "x"}}));
        assert!(b.seen.take().expect("called").1.get("doc").is_none());
    }

    /// The password has to be in the schema, or a client has no way to know it can be sent.
    #[test]
    fn the_schema_advertises_the_password_as_optional() {
        let tools = tool_definitions();
        let open = tools.as_array().expect("the tools are a list").iter().find(|t| t["name"] == "open_document").expect("open_document is offered");
        let schema = &open["inputSchema"];
        assert!(schema["properties"]["password"].is_object(), "the schema offers no password");
        assert_eq!(schema["required"], json!(["path"]), "the password stays optional");
    }
}
