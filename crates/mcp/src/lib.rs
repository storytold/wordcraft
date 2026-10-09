//! WordCraft's MCP server (Model Context Protocol, JSON-RPC 2.0 over stdio).
//!
//! Two backends: [`Headless`] runs an in-process editing session (no window); [`backend::Remote`]
//! talks to a running WordCraft app through its control channel (`wordcraft --control PORT`), so
//! agents can also click, type and screenshot the real UI.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod backend;
pub mod control_key;
pub mod server;
pub mod tools;

pub use backend::{Backend, Remote};
pub use server::Server;

use serde_json::{Value, json};
use wordcraft_engine::Session;

/// Methods that need the desktop app.
pub const NEEDS_APP: &str = "needs the desktop app: start `wordcraft --control 7981` and run `wordcraft-cli mcp --connect 127.0.0.1:7981`";

/// An in-process session.
pub struct Headless {
    pub session: Session,
}

impl Default for Headless {
    fn default() -> Self {
        Headless { session: Session::new(wordcraft_doc::Document::new()) }
    }
}

impl Backend for Headless {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "engine.execute" | "command" => {
                let id = params.get("command").and_then(Value::as_str).ok_or("missing `command`")?.to_string();
                let p = params.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
                self.session.run(&id, &p).map_err(|e| e.to_string())
            }
            "engine.commands" => Ok(self.session.registry.describe()),
            "document.inspect" => self.session.run("document.inspect", &params).map_err(|e| e.to_string()),
            "ui.parity" => Ok(wordcraft_engine::catalog::parity(&self.session.registry)),
            "ui.render" | "render.page" => {
                let page = params.get("page").and_then(Value::as_u64).unwrap_or(1).max(1) as usize - 1;
                let scale = params.get("scale").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.1, 4.0) as f32;
                let l = self.session.layout();
                let pg = l.pages.get(page).ok_or_else(|| format!("no page {}", page + 1))?;
                let img = wordcraft_render::render_page(&self.session.doc, pg, scale, &Default::default());
                let png = img.to_png();
                Ok(
                    json!({"png": wordcraft_engine::cmd::insert::base64_encode(&png), "width": img.width, "height": img.height, "pages": l.pages.len()}),
                )
            }
            m if m.starts_with("ui.") => Err(format!("`{m}` {NEEDS_APP}")),
            other => self.session.run(other, &params).map_err(|e| e.to_string()),
        }
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
