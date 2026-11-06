//! Session-based collaboration adapter.
//!
//! `wordcraft-collab-doc` edits a bare `Document`; the engine's commands also
//! need the `Session` around it (selection, preferences, protection). This
//! adapter keeps a real [`Session`](wordcraft_engine::Session) as the room
//! model, so `kind:"command"` operations (ADR-0001) run through the same
//! `Session::run` the UI, CLI and MCP use.
//!
//! Commands are dangerous on a server: they can open files, change settings or
//! quit. Only mutating commands outside a deny-list of areas that reach beyond
//! the document are accepted, and the operation must name an explicit address
//! (the client's caret is never the server's).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde_json::{Value, json};

use wordcraft_collab::ids::{NodeId, NodeIds};
use wordcraft_collab::room::{Effect, Model};
use wordcraft_collab::wire::{Op, OpKind, Reject, RejectCode};
use wordcraft_collab_doc::DocModel;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_engine::{Selection, Session};

/// Command areas that must never run from a network operation.
const DENIED_PREFIXES: &[&str] =
    &["file.", "tools.", "ui.", "view.", "caret.", "select.", "collab.", "edit.undo", "edit.redo", "edit.repeat", "review.readAloud"];

/// A room model backed by the engine.
pub struct SessionModel {
    session: Session,
}

impl SessionModel {
    pub fn new(session: Session) -> SessionModel {
        SessionModel { session }
    }

    pub fn from_document(document: Document) -> SessionModel {
        SessionModel::new(Session::new(document))
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// A command operation: `{"command": "format.bold", "params": {...}}` with
    /// the caret in `target`.
    fn apply_command(&mut self, op: &Op, ids: &NodeIds) -> Result<Effect, Reject> {
        let command = op.payload.get("command").and_then(Value::as_str).ok_or_else(|| invalid(op, "a command operation needs `command`"))?;
        if !allowed_command(command) {
            return Err(Reject::new(op.op_id.clone(), RejectCode::Forbidden, format!("command {command} is not available over the network")));
        }
        let spec = self.session.registry.get(command).ok_or_else(|| invalid(op, format!("unknown command {command}")))?;
        if !spec.mutates {
            return Err(invalid(op, format!("command {command} does not change the document")));
        }

        let key = ids.resolve(NodeId(op.target.node_id)).map_err(|why| match why {
            wordcraft_collab::ids::Resolve::Gone(version) => {
                Reject::new(op.op_id.clone(), RejectCode::Gone, format!("node {} was deleted at version {version}", op.target.node_id))
            }
            wordcraft_collab::ids::Resolve::Unknown => {
                Reject::new(op.op_id.clone(), RejectCode::Invalid, format!("unknown node {}", op.target.node_id))
            }
        })?;
        let story = parse_story(&key.story).ok_or_else(|| invalid(op, format!("unknown story {:?}", key.story)))?;
        self.session.sel = Selection::caret(Pos { story, path: Path(key.path.clone()), off: op.target.off });

        let params = op.payload.get("params").cloned().unwrap_or(Value::Null);
        self.session.run(command, &params).map_err(|err| invalid(op, err.to_string()))?;
        Ok(Effect {
            delta: Some(json!({"op": "command", "command": command, "nodeId": op.target.node_id})),
            // Engine commands have no inverse here; per-user undo covers
            // primitives and structural operations (ADR-0010).
            inverse: Vec::new(),
            forward: Vec::new(),
            label: format!("Command {command}"),
        })
    }
}

impl Model for SessionModel {
    fn apply(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        if op.kind == OpKind::Command {
            return self.apply_command(op, ids);
        }
        // Primitives and structural operations are the document adapter's; the
        // session around the document is untouched by them.
        let document = std::mem::replace(&mut self.session.doc, Document::new());
        let mut model = DocModel::new(document);
        let result = model.apply(op, ids);
        self.session.doc = model.into_document();
        result
    }

    fn snapshot(&self) -> Vec<u8> {
        serde_json::to_vec(&self.session.doc).unwrap_or_default()
    }

    fn index_nodes(&self, ids: &mut NodeIds) {
        DocModel::new(self.session.doc.clone()).index_nodes(ids);
    }
}

fn allowed_command(command: &str) -> bool {
    !DENIED_PREFIXES.iter().any(|prefix| command.starts_with(prefix))
}

fn parse_story(name: &str) -> Option<StoryRef> {
    match name {
        "body" => Some(StoryRef::Body),
        other => other.strip_prefix("part:").and_then(|id| id.parse().ok()).map(StoryRef::Part),
    }
}

fn invalid(op: &Op, message: impl Into<String>) -> Reject {
    Reject::new(op.op_id.clone(), RejectCode::Invalid, message)
}

#[cfg(test)]
mod tests;
