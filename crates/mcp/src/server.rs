//! JSON-RPC 2.0 framing and the MCP lifecycle / tools / resources methods.

use std::io::{BufRead, Read, Write};

use serde_json::{Value, json};

use crate::backend::Backend;
use crate::tools::{call_tool, tool_definitions};

/// The MCP revision we implement.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const RESOURCE_NOT_FOUND: i64 = -32002;

const INSTRUCTIONS: &str = "WordCraft is a word processor (a Microsoft Word clone). Every action is a command: find ids and \
parameters with list_commands and run them with execute (or several with batch). Positions are \
{story:\"body\", path:[block,(row,cell,block)...], off:byteOffset}. Typical flow: new_document → type_text \
\"Hello\" → execute text.newParagraph → execute para.style {style:\"Heading 1\"} → select_text {text:\"Hello\"} → \
execute format.bold → inspect_document to verify → render_page to look at the result → save {path:\"out.docx\"}.";

/// Longest request line accepted on stdin.
const MAX_LINE: usize = 32 << 20;

enum Line {
    End,
    TooLong,
    Text(String),
}

/// The next line of at most `max` bytes. A longer one is read to its end and dropped, a line
/// that is not UTF-8 is replaced lossily (the JSON parser then rejects it).
fn read_line(input: &mut impl BufRead, max: usize) -> std::io::Result<Line> {
    let mut buf = Vec::new();
    let n = Read::take(&mut *input, max as u64 + 1).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(Line::End);
    }
    if buf.len() > max && !buf.ends_with(b"\n") {
        loop {
            let (used, done) = match input.fill_buf()? {
                [] => break,
                b => match b.iter().position(|&c| c == b'\n') {
                    Some(i) => (i + 1, true),
                    None => (b.len(), false),
                },
            };
            input.consume(used);
            if done {
                break;
            }
        }
        return Ok(Line::TooLong);
    }
    Ok(Line::Text(String::from_utf8_lossy(&buf).into_owned()))
}

/// Resource URIs.
pub const DOC_URI: &str = "wordcraft://document";
pub const COMMANDS_URI: &str = "wordcraft://commands";

/// An MCP server bound to one backend.
pub struct Server {
    backend: Box<dyn Backend>,
    initialized: bool,
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

impl Server {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self { backend, initialized: false }
    }

    pub fn backend(&mut self) -> &mut dyn Backend {
        self.backend.as_mut()
    }

    /// Whether the client has sent `notifications/initialized`.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Serve newline-delimited JSON-RPC until `input` closes. Logs go to stderr only (stdout is
    /// the protocol stream).
    pub fn serve(&mut self, input: impl BufRead, output: impl Write) -> std::io::Result<()> {
        self.serve_bounded(input, output, MAX_LINE)
    }

    /// [`Server::serve`] with a request line limit of `max` bytes. A longer line is dropped
    /// (without keeping it in memory) and answered with an error.
    fn serve_bounded(&mut self, mut input: impl BufRead, mut output: impl Write, max: usize) -> std::io::Result<()> {
        loop {
            let reply = match read_line(&mut input, max)? {
                Line::End => return Ok(()),
                Line::TooLong => Some(error(Value::Null, INVALID_REQUEST, format!("request line longer than {max} bytes")).to_string()),
                Line::Text(line) => self.handle_line(&line),
            };
            if let Some(reply) = reply {
                output.write_all(reply.as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
    }

    /// Handle one line; returns the reply line (None for notifications and blank lines).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) => {
                // Batches were removed in 2025-06-18; still answer older clients sensibly.
                if batch.is_empty() {
                    Some(error(Value::Null, INVALID_REQUEST, "empty batch"))
                } else {
                    let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
            }
            Ok(msg) => self.handle(msg),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// Handle one JSON-RPC message; `None` for notifications and responses.
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        let Value::Object(o) = &msg else { return Some(error(Value::Null, INVALID_REQUEST, "message must be an object")) };
        let id = o.get("id").cloned();
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // A response to a server→client request (we send none): ignore. Anything else is invalid.
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(error(id.unwrap_or(Value::Null), INVALID_REQUEST, "missing `method`"));
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            self.notification(method);
            return None;
        };
        if !(id.is_string() || id.is_number()) {
            return Some(error(Value::Null, INVALID_REQUEST, "`id` must be a string or number"));
        }
        Some(match self.request(method, &params) {
            Ok(r) => response(id, r),
            Err((code, m)) => error(id, code, m),
        })
    }

    fn notification(&mut self, method: &str) {
        match method {
            "notifications/initialized" => self.initialized = true,
            "notifications/cancelled" | "notifications/progress" | "notifications/roots/list_changed" => {}
            other => log::debug!("ignoring notification {other}"),
        }
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}, "resources": {}},
                    "serverInfo": {"name": "wordcraft", "title": "WordCraft", "version": env!("CARGO_PKG_VERSION"), "websiteUrl": "https://getartcraft.com/apps/wordcraft"},
                    "instructions": format!(
                        "{INSTRUCTIONS} Backend: {}. Community: https://discord.gg/artcraft · https://getartcraft.com/apps/wordcraft · https://github.com/storytold/wordcraft",
                        self.backend.describe()
                    ),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing tool `name`".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                Ok(call_tool(self.backend.as_mut(), name, &args).to_value())
            }
            "resources/list" => Ok(json!({"resources": [
                {"uri": DOC_URI, "name": "document", "title": "Active document", "description": "Blocks, text, formatting, sections, parts and selection of the document (document.inspect)", "mimeType": "application/json"},
                {"uri": COMMANDS_URI, "name": "commands", "title": "Commands", "description": "Every command: id, label, ribbon location, shortcut, params, enabled (engine.commands)", "mimeType": "application/json"},
            ]})),
            "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing `uri`".to_string()))?;
                let v = match uri {
                    DOC_URI => self.backend.call("document.inspect", json!({})),
                    COMMANDS_URI => self.backend.call("engine.commands", json!({})),
                    _ => return Err((RESOURCE_NOT_FOUND, format!("resource not found: {uri}"))),
                }
                .map_err(|e| (INTERNAL_ERROR, e))?;
                Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}]}))
            }
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }
}

#[cfg(test)]
mod bounded_tests {
    use super::*;

    struct Null;
    impl Backend for Null {
        fn call(&mut self, _: &str, _: Value) -> Result<Value, String> {
            Ok(Value::Null)
        }
        fn has_ui(&self) -> bool {
            false
        }
        fn describe(&self) -> String {
            "test".into()
        }
    }

    #[test]
    fn an_oversize_line_is_refused_and_the_next_one_still_served() {
        let mut input = vec![b'x'; 200];
        input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        let mut out = Vec::new();
        Server::new(Box::new(Null)).serve_bounded(std::io::Cursor::new(input), &mut out, 64).unwrap();
        let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0]["error"]["code"], INVALID_REQUEST);
        assert_eq!(lines[1]["id"], 1);
    }

    #[test]
    fn a_line_without_a_newline_is_bounded() {
        let mut out = Vec::new();
        Server::new(Box::new(Null)).serve_bounded(std::io::Cursor::new(vec![b'x'; 10_000]), &mut out, 64).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("longer than 64"));
    }
}
