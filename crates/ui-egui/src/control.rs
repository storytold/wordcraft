//! Programmatic control of the running app (agents, tests, MCP bridge).
//!
//! Methods (JSON lines over the host's transport, see `docs/control-protocol.md`):
//! - `engine.execute {command, params, doc?}`: run any command (engine or `ui.*`)
//! - `engine.commands {search?}`: engine and app print commands with enablement and parameters
//! - `document.inspect`: document structure; `ui.inspect`: UI state, canvas geometry, perf
//! - `ui.click`, `ui.move`, `ui.wheel`, `ui.key`, `ui.release`, `ui.type`, `ui.ime`, `ui.capsLock`:
//!   real input in screen points (or document points with `coordinates`), delivered by the
//!   control library
//! - `ui.clickText {page, x, y}`: click at a page position (points from the page's top-left)
//! - `ui.dialog.set`, `ui.dialog.confirm`, `ui.dialog.cancel`: the open dialogs' fields and answers
//! - `ui.screenshot {path?}`: PNG of the window; `ui.render {path?, page?, scale?}`: page PNG
//!   (without `path`, both reply with the PNG as base64 `png`)
//! - `ui.resize`, `ui.focus`, `ui.closeWindow`, `ui.clipboard`, `ui.replayAt`, `ui.accessTree`
//! - `ui.parity`: feature catalog parity
//!
//! `engine.execute`, `document.inspect` and `ui.render` take `doc`, the id of an open document
//! (`view.switchWindows` lists them): they work on that document while the window keeps showing
//! the active one. Without `doc` they work on the active document.

mod handler;
mod screenshots;

pub(crate) use handler::close_refused;
pub use handler::{handle, inspect};
#[cfg(test)]
mod doc_tests;

/// Writes a screenshot's PNG bytes to the path a client named. The file authority of the control
/// channel: only `.png` paths, inside the roots automation may write to, through the engine's
/// file writer (which refuses links). The web build has no files.
pub fn write_automation_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, bytes);
        Err("writing files is not available in the browser".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !path.to_ascii_lowercase().ends_with(".png") {
            return Err(format!("{path}: a screenshot path must end in .png"));
        }
        wordcraft_engine::io::write_file(path, bytes).map(|_| ()).map_err(|e| e.to_string())
    }
}
