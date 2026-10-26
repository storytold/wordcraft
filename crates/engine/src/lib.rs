//! WordCraft engine: the editing session and its commands.
//!
//! **Everything is a command.** Each user-visible action is a [`CommandSpec`] with a stable id
//! (`format.bold`, `insert.table`), a label, the ribbon/menu location, a default shortcut, a
//! params description and a `run` function. Menus, ribbon, shortcuts, the command palette, the
//! CLI, the JSON control channel and the MCP server all call [`Session::run`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod catalog;
pub mod cmd;
pub mod file_access;
pub mod io;
mod io_ext;
pub mod keymap;
pub mod math_gallery;
pub mod print;
pub mod recovery;
pub mod ribbon;
pub mod sample;
mod session;
pub mod speech;

use std::collections::HashMap;

use serde_json::Value;

pub use io::Password;
pub use keymap::KeyMap;
pub use ribbon::RibbonLayout;
pub use session::{
    CellRange, ColumnBlock, DocInfo, DocState, EditSnapshot, FindState, MathEdit, Prefs, Selection, Session, StylesPaneDock, UNDO_LIST_MAX,
    ViewState, WindowView, blank_document, next_math_session, set_system_locale,
};
/// The outcome a command reports for IME receipt classification.
///
/// A command that can be called by an input method must classify its committed effects. Only a
/// classified content change without an automatic rewrite may mint an IME receipt; any other
/// committed state change still advances the IME state revision and therefore makes a mixed
/// frame `Other` rather than certifying it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImeEditOutcome {
    /// The handler explicitly classified the committed effects.
    pub classified: bool,
    /// IME-visible content changed. Only this gates an IME receipt.
    pub content_changed: bool,
    /// Any committed document state changed, including style or representation changes.
    pub state_changed: bool,
    /// An automatic engine rewrite was applied (AutoCorrect, build-up, AutoFormat).
    pub transformed: bool,
    /// The caret or selection changed. Filled in by [`Session::run`].
    pub selection_changed: bool,
}

/// The largest text one `equation.type` call inserts, in characters. The engine truncates to this
/// in both linear and built-up modes; the UI refuses an IME commit above it instead of inserting a
/// prefix.
pub const MAX_EQUATION_TYPE_CHARS: usize = 10_000;

/// The largest number of unit steps one `equation.deleteSurrounding` request may take on either
/// side of the caret. A hostile IME count must not become an unbounded command.
pub const MAX_EQUATION_DELETE_UNITS: usize = 64;

pub use wordcraft_doc as doc;
pub use wordcraft_geom::Region;
pub use wordcraft_layout as layout;
pub use wordcraft_proof as proof;
pub use wordcraft_render as render;

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum CmdError {
    #[error("unknown command `{0}`")]
    Unknown(String),
    #[error("bad parameters: {0}")]
    Params(String),
    #[error("not available: {0}")]
    Disabled(String),
    #[error("{0}")]
    Failed(String),
}

impl From<wordcraft_doc::DocError> for CmdError {
    fn from(e: wordcraft_doc::DocError) -> Self {
        CmdError::Failed(e.to_string())
    }
}

pub type CmdResult = Result<Value, CmdError>;

/// A registered command.
#[derive(Clone)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Where it lives in the UI: `Home › Font`, `Insert › Tables`, `File`…
    pub location: &'static str,
    /// Default shortcut in a portable form (`Mod+B`, `Mod+Shift+>`, `F7`). `Mod` = Ctrl, or ⌘ on macOS.
    pub shortcut: &'static str,
    /// Parameter description for agents (`{"size": number}`).
    pub params: &'static str,
    pub mutates: bool,
    /// `None` = enabled; `Some(reason)` = disabled.
    pub enabled: fn(&Session) -> Option<&'static str>,
    pub run: fn(&mut Session, &Value) -> CmdResult,
}

fn always(_: &Session) -> Option<&'static str> {
    None
}

impl CommandSpec {
    pub const fn new(id: &'static str, label: &'static str, location: &'static str, run: fn(&mut Session, &Value) -> CmdResult) -> Self {
        CommandSpec { id, label, location, shortcut: "", params: "{}", mutates: true, enabled: always, run }
    }
    pub const fn key(mut self, s: &'static str) -> Self {
        self.shortcut = s;
        self
    }
    pub const fn params(mut self, p: &'static str) -> Self {
        self.params = p;
        self
    }
    /// Doesn't change the document (navigation, view, export).
    pub const fn pure(mut self) -> Self {
        self.mutates = false;
        self
    }
    pub const fn when(mut self, f: fn(&Session) -> Option<&'static str>) -> Self {
        self.enabled = f;
        self
    }
}

/// All commands by id.
pub struct Registry {
    specs: Vec<CommandSpec>,
    by_id: HashMap<&'static str, usize>,
}

impl Registry {
    pub fn new(specs: Vec<CommandSpec>) -> Self {
        let mut by_id = HashMap::new();
        for (i, s) in specs.iter().enumerate() {
            by_id.insert(s.id, i);
        }
        Registry { specs, by_id }
    }
    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.by_id.get(id).and_then(|i| self.specs.get(*i))
    }
    pub fn all(&self) -> &[CommandSpec] {
        &self.specs
    }
    /// Command list as JSON (for agents: `commands` / `list_commands`).
    pub fn describe(&self) -> Value {
        Value::Array(
            self.specs
                .iter()
                .map(|s| serde_json::json!({"id": s.id, "label": s.label, "location": s.location, "shortcut": s.shortcut, "params": s.params, "mutates": s.mutates}))
                .collect(),
        )
    }
    /// The command bound to a shortcut, if any.
    pub fn by_shortcut(&self, key: &str) -> Option<&CommandSpec> {
        self.specs.iter().find(|s| !s.shortcut.is_empty() && s.shortcut.split(" / ").any(|k| k.eq_ignore_ascii_case(key)))
    }
}

/// Parameter helpers.
pub mod p {
    use serde_json::Value;

    use crate::CmdError;

    pub fn str<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
        v.get(k).and_then(Value::as_str)
    }
    pub fn req_str<'a>(v: &'a Value, k: &str) -> Result<&'a str, CmdError> {
        str(v, k).ok_or_else(|| CmdError::Params(format!("`{k}` (string) is required")))
    }
    pub fn f32(v: &Value, k: &str) -> Option<f32> {
        v.get(k).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite())
    }
    pub fn req_f32(v: &Value, k: &str) -> Result<f32, CmdError> {
        f32(v, k).ok_or_else(|| CmdError::Params(format!("`{k}` (number) is required")))
    }
    pub fn u64(v: &Value, k: &str) -> Option<u64> {
        v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_f64().filter(|f| *f >= 0.0 && f.is_finite()).map(|f| f as u64)))
    }
    pub fn bool(v: &Value, k: &str) -> Option<bool> {
        v.get(k).and_then(Value::as_bool)
    }
    /// The `value` param, or the first positional-ish string.
    pub fn value(v: &Value) -> Option<&Value> {
        v.get("value")
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_bidi;
#[cfg(test)]
mod tests_documents;
#[cfg(test)]
mod tests_fonts;
#[cfg(test)]
mod tests_ime_noop;
#[cfg(test)]
mod tests_revision_checks;
#[cfg(test)]
mod tests_typing;
#[cfg(test)]
mod tests_wordart;
