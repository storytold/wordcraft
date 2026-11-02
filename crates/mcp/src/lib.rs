//! WordCraft's MCP server (Model Context Protocol, JSON-RPC 2.0 over stdio).
//!
//! Two backends: [`Headless`] runs an in-process editing session (no window); [`backend::Remote`]
//! talks to a running WordCraft app through its control channel (`wordcraft --control PORT`), so
//! agents can also click, type and screenshot the real UI.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod backend;
pub mod server;
pub mod tools;

pub use backend::{Backend, Remote};
pub use server::Server;

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_engine::Session;
use wordcraft_engine::command_access::{self, CommandAllowlist};
use wordcraft_engine::file_access::{self, AutomationRoots};

/// Methods that need the desktop app.
pub const NEEDS_APP: &str = "needs the desktop app: start `wordcraft --control 7981` and run `wordcraft-cli mcp --connect 127.0.0.1:7981`";

/// An in-process session.
pub struct Headless {
    pub session: Session,
    roots: Option<Arc<AutomationRoots>>,
    commands: Option<Arc<CommandAllowlist>>,
}

impl Headless {
    /// Confine every file the session's commands read and write to `roots`
    /// (`wordcraft-cli mcp --automation-read-root … --automation-write-root …`).
    pub fn with_automation_roots(mut self, roots: Option<Arc<AutomationRoots>>) -> Self {
        self.roots = roots;
        self
    }

    /// Hold every command the session's clients run to `commands`
    /// (`wordcraft-cli mcp --allow-commands …`; see [`command_access`]).
    pub fn with_command_allowlist(mut self, commands: Option<Arc<CommandAllowlist>>) -> Self {
        self.commands = commands;
        self
    }

    /// Answer `method`; a document method addressed to another open document (`doc`) runs on
    /// that document, and the active one stays active.
    fn answer(&mut self, method: &str, mut params: Value) -> Result<Value, String> {
        if !DOC_METHODS.contains(&method) {
            return answer_on(&mut self.session, method, params);
        }
        let doc = doc_param(&params)?;
        if let Some(o) = params.as_object_mut() {
            o.remove("doc");
        }
        match doc {
            None => answer_on(&mut self.session, method, params),
            Some(id) => self.session.with_document(id, |s| answer_on(s, method, params)).map_err(|e| e.to_string())?,
        }
    }
}

/// The methods that work on one document, and so take `doc`: the id of the open document to work
/// on (`view.switchWindows` lists them), the active one when absent.
pub const DOC_METHODS: &[&str] = &["engine.execute", "command", "document.inspect", "ui.render", "render.page"];

/// The document a request addresses with `doc`: `None` (absent or null) is the active document.
/// Anything but a whole number from 0 up is an error; whether it names an open document is for
/// the session to say.
pub fn doc_param(params: &Value) -> Result<Option<u64>, String> {
    match params.get("doc") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| "`doc` must be the id of an open document (list_documents lists them)".to_string()),
    }
}

/// Answer `method` on `session` (the active document, or the one `doc` picked). A command the
/// client names (`engine.execute`, `command`, or a command id as the method) must be on the
/// command list in force on this thread, and `engine.commands` lists only those; reading the
/// document (`document.inspect`, page renders, parity) stays available.
fn answer_on(session: &mut Session, method: &str, params: Value) -> Result<Value, String> {
    match method {
        "engine.execute" | "command" => {
            let id = params.get("command").and_then(Value::as_str).ok_or("missing `command`")?.to_string();
            command_access::check(&id)?;
            let p = params.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            session.run(&id, &p).map_err(|e| e.to_string())
        }
        // With --allow-commands, the list shows only what the client may run.
        "engine.commands" => Ok(match (session.registry.describe(), command_access::current()) {
            (Value::Array(all), Some(list)) => {
                Value::Array(all.into_iter().filter(|c| c.get("id").and_then(Value::as_str).is_some_and(|id| list.allows(id))).collect())
            }
            (all, _) => all,
        }),
        "document.inspect" => session.run("document.inspect", &params).map_err(|e| e.to_string()),
        "ui.parity" => Ok(wordcraft_engine::catalog::parity(&session.registry)),
        "ui.render" | "render.page" => {
            let page = params.get("page").and_then(Value::as_u64).unwrap_or(1).max(1);
            let page = usize::try_from(page).unwrap_or(usize::MAX).saturating_sub(1);
            let scale = params.get("scale").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.1, 4.0) as f32;
            let l = session.layout();
            let pg = l.pages.get(page).ok_or_else(|| format!("no page {}", page.saturating_add(1)))?;
            let img = wordcraft_render::render_page(&session.doc, pg, scale, &Default::default());
            let png = img.to_png();
            Ok(json!({"png": wordcraft_engine::cmd::insert::base64_encode(&png), "width": img.width, "height": img.height, "pages": l.pages.len()}))
        }
        m if m.starts_with("ui.") => Err(format!("`{m}` {NEEDS_APP}")),
        other => {
            command_access::check(other)?;
            session.run(other, &params).map_err(|e| e.to_string())
        }
    }
}

impl Default for Headless {
    fn default() -> Self {
        Headless { session: Session::new(wordcraft_doc::Document::new()), roots: None, commands: None }
    }
}

impl Backend for Headless {
    /// Answer one request with this server's command list and automation roots in force on the
    /// calling thread, whichever thread that is, and the previous ones restored afterwards.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let roots = self.roots.clone();
        let commands = self.commands.clone();
        command_access::confine(commands.as_ref(), || file_access::confine(roots.as_ref(), || self.answer(method, params)))
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "headless session (no window)".into()
    }
}

#[cfg(test)]
mod tests;
