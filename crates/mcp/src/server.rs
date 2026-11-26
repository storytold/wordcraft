//! WordCraft's MCP server: the shared `craft_mcp_server` core (JSON-RPC framing, the lifecycle,
//! `tools/*` and `resources/*`) around WordCraft's own tools, resources and instructions.

#[cfg(test)]
use std::io::Read;
use std::io::{BufRead, Write};

use craft_mcp_server::{Contents, Incoming, Info, Options, Tools, read_message};
use serde_json::{Value, json};

use crate::backend::Backend;
use crate::tools::{call_tool, tool_definitions};

/// The MCP revision we implement.
pub use craft_mcp_server::PROTOCOL_VERSION;

const INSTRUCTIONS: &str = "WordCraft is a word processor (a Microsoft Word clone). Every action is a command: find ids and \
parameters with list_commands and run them with execute (or several with batch). Positions are \
{story:\"body\", path:[block,(row,cell,block)...], off:byteOffset}. Typical flow: new_document → type_text \
\"Hello\" → execute text.newParagraph → execute para.style {style:\"Heading 1\"} → select_text {text:\"Hello\"} → \
execute format.bold → inspect_document to verify → render_page to look at the result → save {path:\"out.docx\"}. \
Several documents can be open: new_document and open_document open another one and make it active; \
list_documents gives their ids, and `doc` on execute, batch and the other document tools works on one \
without switching.";

/// Resource URIs.
pub const DOC_URI: &str = "wordcraft://document";
pub const COMMANDS_URI: &str = "wordcraft://commands";

/// Longest request line accepted on stdin.
pub const MAX_REQUEST_LINE: usize = 1 << 20;

/// One bounded line read.
#[cfg(test)]
pub(crate) enum BoundedLine {
    Eof,
    Line(String),
    /// The line exceeded the limit; the rest of it has been discarded.
    TooLong,
}

/// Read one line of at most `max` bytes (excluding the newline) without buffering more than
/// `max + 1` bytes; an over-long line is skipped up to its newline.
#[cfg(test)]
pub(crate) fn read_bounded_line(reader: &mut impl BufRead, max: usize) -> std::io::Result<BoundedLine> {
    let mut buf = Vec::new();
    reader.by_ref().take(max as u64 + 1).read_until(b'\n', &mut buf)?;
    if buf.is_empty() {
        return Ok(BoundedLine::Eof);
    }
    if buf.last() != Some(&b'\n') && buf.len() > max {
        loop {
            let chunk = reader.fill_buf()?;
            if chunk.is_empty() {
                break;
            }
            let found = chunk.iter().position(|&b| b == b'\n');
            let used = found.map_or(chunk.len(), |i| i + 1);
            reader.consume(used);
            if found.is_some() {
                break;
            }
        }
        return Ok(BoundedLine::TooLong);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    String::from_utf8(buf).map(BoundedLine::Line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// WordCraft's tools, resources and instructions over one backend, for the shared server.
struct WordcraftTools {
    backend: Box<dyn Backend>,
}

impl Tools for WordcraftTools {
    fn instructions(&self) -> String {
        format!(
            "{INSTRUCTIONS} Backend: {}. Community: https://discord.gg/artcraft · https://getartcraft.com/apps/wordcraft · https://github.com/storytold/wordcraft",
            self.backend.describe()
        )
    }

    fn list(&mut self) -> Vec<Value> {
        match tool_definitions() {
            Value::Array(tools) => tools,
            _ => Vec::new(),
        }
    }

    fn call(&mut self, name: &str, args: &Value) -> Value {
        call_tool(self.backend.as_mut(), name, args).to_value()
    }

    fn resources(&self) -> Vec<Value> {
        vec![
            json!({"uri": DOC_URI, "name": "document", "title": "Active document", "description": "Blocks, text, formatting, sections, parts and selection of the document (document.inspect)", "mimeType": "application/json"}),
            json!({"uri": COMMANDS_URI, "name": "commands", "title": "Commands", "description": "Every command: id, label, ribbon location, shortcut, params, enabled (engine.commands)", "mimeType": "application/json"}),
        ]
    }

    fn read_resource(&mut self, uri: &str) -> Option<Result<Contents, String>> {
        let method = match uri {
            DOC_URI => "document.inspect",
            COMMANDS_URI => "engine.commands",
            _ => return None,
        };
        Some(
            self.backend
                .call(method, json!({}))
                .map(|v| Contents::Text { mime_type: "application/json".into(), text: serde_json::to_string_pretty(&v).unwrap_or_default() }),
        )
    }
}

/// An MCP server bound to one backend.
pub struct Server {
    inner: craft_mcp_server::Server<WordcraftTools>,
}

impl Server {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        let info = Info { name: "wordcraft", title: "WordCraft", version: env!("CARGO_PKG_VERSION"), max_line: MAX_REQUEST_LINE };
        let options = Options {
            website_url: Some("https://getartcraft.com/apps/wordcraft".into()),
            // Clients read every tool failure from the result, as before the shared core.
            tool_errors_as_results: true,
            strict_request_ids: true,
        };
        Self { inner: craft_mcp_server::Server::new(info, WordcraftTools { backend }).with_options(options) }
    }

    pub fn backend(&mut self) -> &mut dyn Backend {
        self.inner.tools().backend.as_mut()
    }

    /// Whether the client has sent `notifications/initialized`.
    pub fn is_initialized(&self) -> bool {
        self.inner.is_initialized()
    }

    /// Serve newline-delimited JSON-RPC until `input` closes. Logs go to stderr only (stdout is
    /// the protocol stream).
    pub fn serve(&mut self, mut input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        // The shared core's own loop, with every line through [`Self::handle_line`] so that the
        // tool-call compatibility rule below applies to a served session too.
        while let Some(incoming) = read_message(&mut input, MAX_REQUEST_LINE)? {
            let reply = match incoming {
                Incoming::Line(line) => self.handle_line(&line),
                Incoming::Refused(reply) => Some(reply),
            };
            if let Some(reply) = reply {
                output.write_all(reply.as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Handle one line; returns the reply line (None for notifications and blank lines).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        // The shared core checks the received line's length before parsing; keep that check on the
        // line as received, not on anything re-serialised from it.
        if line.len() > MAX_REQUEST_LINE {
            return self.inner.handle_line(line);
        }
        // The shared core trims the line before parsing (Rust's `trim`, so a no-break space too);
        // parse the same text, so a call it would accept is filtered rather than refused.
        match serde_json::from_str::<Value>(line.trim()) {
            Ok(mut msg) => {
                // Only a message that loses an argument is re-serialised; every other line reaches
                // the shared core as received, so the length it checks is the length that arrived.
                if drop_unknown_arguments(&mut msg) { self.dispatch(msg) } else { self.inner.handle_line(line) }
            }
            // Not JSON: the shared core's own parse error, unchanged.
            Err(_) => self.inner.handle_line(line),
        }
    }

    /// Handle one JSON-RPC message; `None` for notifications and responses.
    pub fn handle(&mut self, mut msg: Value) -> Option<Value> {
        drop_unknown_arguments(&mut msg);
        self.inner.handle(msg)
    }

    /// Dispatch a message whose unknown arguments were dropped, replying in the shared core's own
    /// format (`handle_line` is `handle` plus framing). A batch goes back through the core's line
    /// handling, which alone knows its rules for an empty batch and one over its cap.
    fn dispatch(&mut self, msg: Value) -> Option<String> {
        match msg {
            batch @ Value::Array(_) => self.inner.handle_line(&batch.to_string()),
            msg => self.inner.handle(msg).map(|reply| reply.to_string()),
        }
    }
}

/// Drop the top-level `tools/call` arguments a tool's schema does not declare, as WordCraft's
/// hand-written server did before the shared core (#6972). The shared core refuses them, but the
/// advertised schemas leave the argument object open, so a client following the schema could send
/// one and have a call that worked before refused. Arguments the schema does declare are still
/// checked there: required, type and enum. Only the top level is touched: `execute`'s `params`
/// are the command's own and stay as sent. Returns whether anything was dropped.
fn drop_unknown_arguments(msg: &mut Value) -> bool {
    match msg {
        Value::Array(batch) => {
            let mut dropped = false;
            for item in batch {
                dropped = drop_unknown_arguments(item) || dropped;
            }
            dropped
        }
        Value::Object(o) => {
            if o.get("method").and_then(Value::as_str) != Some("tools/call") {
                return false;
            }
            let Some(params) = o.get_mut("params").and_then(Value::as_object_mut) else { return false };
            let Some(name) = params.get("name").and_then(Value::as_str) else { return false };
            let defs = tool_definitions();
            let Some(declared) =
                defs.as_array().and_then(|d| d.iter().find(|t| t["name"] == name)).and_then(|t| t["inputSchema"]["properties"].as_object())
            else {
                return false;
            };
            let Some(Value::Object(args)) = params.get_mut("arguments") else { return false };
            let before = args.len();
            args.retain(|k, _| declared.contains_key(k));
            args.len() != before
        }
        _ => false,
    }
}
