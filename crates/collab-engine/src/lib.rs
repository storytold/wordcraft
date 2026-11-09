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
use wordcraft_doc::props::CharProps;
use wordcraft_doc::table::MAX_TABLE_DEPTH;
use wordcraft_doc::{Block, Blocks, Document, Path, Pos, StoryRef};
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

    /// A command operation: `{"command": "format.bold", "params": {...}, "pending": {...}}`
    /// with the caret in `target`. The author is the JWT actor, never the
    /// operation's payload (ADR-0008).
    fn apply_command(&mut self, actor: &str, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let command = op.payload.get("command").and_then(Value::as_str).ok_or_else(|| invalid(op, "a command operation needs `command`"))?;
        if !allowed_command(command) {
            return Err(Reject::new(op.op_id.clone(), RejectCode::Forbidden, format!("command {command} is not available over the network")));
        }
        let spec = self.session.registry.get(command).ok_or_else(|| invalid(op, format!("unknown command {command}")))?;
        if !spec.mutates {
            return Err(invalid(op, format!("command {command} does not change the document")));
        }

        // The caret or selection is explicit: the client's own caret is never
        // the server's. A command that formats a range sends both ends.
        let selection = match op.payload.get("selection") {
            Some(value) if !value.is_null() => {
                let anchor = resolve_address(op, ids, value.get("anchor"))?;
                let focus = resolve_address(op, ids, value.get("focus"))?;
                Selection { anchor, focus }
            }
            _ => Selection::caret(resolve_address(op, ids, None)?),
        };
        self.session.sel = selection;
        self.session.author = actor.to_string();

        // Formatting picked with a collapsed caret travels with the operation:
        // the server has no user interface to pick it (RFC §3.1).
        let pending = match op.payload.get("pending") {
            Some(value) if !value.is_null() => {
                Some(serde_json::from_value::<CharProps>(value.clone()).map_err(|err| invalid(op, format!("bad `pending`: {err}")))?)
            }
            _ => None,
        };
        self.session.pending = pending.clone();

        let params = op.payload.get("params").cloned().unwrap_or(Value::Null);
        let before = doc_shape(&self.session.doc);
        let tables_before = touch_tables(command).then(|| table_shapes(&self.session.doc));
        self.session.run(command, &params).map_err(|err| invalid(op, err.to_string()))?;
        let mut nodes_changed = before != doc_shape(&self.session.doc);
        if let Some(tables_before) = tables_before {
            let tables_after = table_shapes(&self.session.doc);
            if tables_before != tables_after {
                // The command moved rows or cells. Their ids name paths that
                // mean something else now, so a stale id would edit the wrong
                // cell; retire the moved tables' ids instead. A client that
                // still holds one is refused as `Gone` and resyncs, which the
                // command's node-map broadcast already asks it to do.
                for shape in tables_before.iter().filter(|old| !tables_after.iter().any(|new| same_shape(old, new))) {
                    ids.remove_subtree(&shape.story, &shape.path, op.base_version);
                }
                for shape in tables_after.iter().filter(|new| !tables_before.iter().any(|old| same_shape(old, new))) {
                    ids.remove_subtree(&shape.story, &shape.path, op.base_version);
                }
                self.index_nodes(ids);
                nodes_changed = true;
            }
        }

        // Per-user undo is the protocol's (ADR-0010); the room must not keep a
        // parallel history of full document snapshots for every command.
        self.session.reset_history();

        Ok(Effect {
            delta: Some(json!({
                "op": "command",
                "command": command,
                "params": params,
                "story": op.target.story,
                "nodeId": op.target.node_id,
                "off": op.target.off,
                "pending": pending,
                "selection": op.payload.get("selection").cloned(),
            })),
            // Engine commands have no inverse here; per-user undo covers
            // primitives and structural operations (ADR-0010). The forward
            // operation names the step so undo reports `unsupported` instead of
            // skipping over it to an older step.
            inverse: Vec::new(),
            forward: vec![op.clone()],
            label: format!("Command {command}"),
            nodes_changed,
            touched: Vec::new(),
        })
    }
}

/// Block counts per story: a command that changes one can move paths or add
/// ids, so replicas need a fresh node map. Text and formatting leave it equal.
fn doc_shape(doc: &Document) -> (usize, Vec<(u32, usize)>) {
    (doc.body.len(), doc.parts.iter().map(|(id, part)| (*id, part.blocks.len())).collect())
}

/// The table structure of a document: for every table, its path and how many
/// cells each row holds. A command that changes this moves cells, so the ids of
/// those tables cannot simply be re-indexed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TableShape {
    story: String,
    path: Vec<u32>,
    cells: Vec<usize>,
}

fn same_shape(a: &TableShape, b: &TableShape) -> bool {
    a.story == b.story && a.path == b.path && a.cells == b.cells
}

/// Whether a command can change table structure; the walk below is skipped for
/// typing and formatting, which are the hot commands.
fn touch_tables(command: &str) -> bool {
    command.starts_with("table.") || command.starts_with("insert.")
}

fn table_shapes(doc: &Document) -> Vec<TableShape> {
    let mut out = Vec::new();
    collect_tables("body", &doc.body, &mut Vec::new(), &mut out, 0);
    for (part, story) in &doc.parts {
        collect_tables(&format!("part:{part}"), &story.blocks, &mut Vec::new(), &mut out, 0);
    }
    out
}

fn collect_tables(story: &str, blocks: &Blocks, prefix: &mut Vec<u32>, out: &mut Vec<TableShape>, depth: usize) {
    if depth > MAX_TABLE_DEPTH {
        return;
    }
    for (index, block) in blocks.iter().enumerate() {
        prefix.push(index as u32);
        if let Block::Table(table) = &**block {
            out.push(TableShape { story: story.to_string(), path: prefix.clone(), cells: table.rows.iter().map(|row| row.cells.len()).collect() });
            for (r, row) in table.rows.iter().enumerate() {
                for (c, cell) in row.cells.iter().enumerate() {
                    prefix.push(r as u32);
                    prefix.push(c as u32);
                    collect_tables(story, &cell.blocks, prefix, out, depth.saturating_add(1));
                    prefix.pop();
                    prefix.pop();
                }
            }
        }
        prefix.pop();
    }
}

/// Apply an authoritative delta to a replica's `Session` (RFC §3.1). Primitive
/// and structural deltas go through the document adapter and re-run layout;
/// a `command` delta replays the same engine command at the explicit address
/// and pending formatting, outside this user's undo history.
pub fn apply_delta(session: &mut Session, ids: &mut NodeIds, delta: &Value) -> Result<(), String> {
    if delta.get("op").and_then(Value::as_str) == Some("command") {
        let command = delta.get("command").and_then(Value::as_str).ok_or_else(|| "a command delta has no `command`".to_string())?;
        let node = delta.get("nodeId").and_then(Value::as_u64).ok_or_else(|| "a command delta has no `nodeId`".to_string())?;
        let node = u32::try_from(node).map_err(|_| "`nodeId` does not fit".to_string())?;
        let key = ids.resolve(NodeId(node)).map_err(|_| format!("unknown node {node}"))?;
        let story = parse_story(&key.story).ok_or_else(|| format!("unknown story {:?}", key.story))?;
        let off = usize::try_from(delta.get("off").and_then(Value::as_u64).unwrap_or(0)).map_err(|_| "`off` does not fit".to_string())?;
        session.sel = match delta.get("selection") {
            Some(value) if !value.is_null() => {
                let anchor = delta_address(delta, ids, value.get("anchor"))?;
                let focus = delta_address(delta, ids, value.get("focus"))?;
                Selection { anchor, focus }
            }
            _ => Selection::caret(Pos { story, path: Path(key.path.clone()), off }),
        };
        session.pending = match delta.get("pending") {
            Some(value) if !value.is_null() => {
                Some(serde_json::from_value::<CharProps>(value.clone()).map_err(|err| format!("bad `pending`: {err}"))?)
            }
            _ => None,
        };
        let params = delta.get("params").cloned().unwrap_or(Value::Null);
        session.run_remote(command, &params).map_err(|err| err.to_string())?;
        return Ok(());
    }
    wordcraft_collab_doc::apply_delta(&mut session.doc, ids, delta)?;
    session.touch();
    session.relayout();
    Ok(())
}

fn delta_address(_delta: &Value, ids: &NodeIds, address: Option<&Value>) -> Result<Pos, String> {
    let node =
        address.and_then(|value| value.get("nodeId")).and_then(Value::as_u64).ok_or_else(|| "selection address has no `nodeId`".to_string())?;
    let node = u32::try_from(node).map_err(|_| "`nodeId` does not fit".to_string())?;
    let off = address.and_then(|value| value.get("off")).and_then(Value::as_u64).unwrap_or(0);
    let off = usize::try_from(off).map_err(|_| "`off` does not fit".to_string())?;
    let key = ids.resolve(NodeId(node)).map_err(|_| format!("unknown node {node}"))?;
    let story = parse_story(&key.story).ok_or_else(|| format!("unknown story {:?}", key.story))?;
    Ok(Pos { story, path: Path(key.path.clone()), off })
}

impl Model for SessionModel {
    fn apply(&mut self, actor: &str, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        if op.kind == OpKind::Command {
            return self.apply_command(actor, op, ids);
        }
        // Primitives and structural operations are the document adapter's; the
        // session around the document is untouched by them.
        let document = std::mem::replace(&mut self.session.doc, Document::new());
        let mut model = DocModel::new(document);
        let result = model.apply(actor, op, ids);
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

/// Resolve `{nodeId, off}` (or the operation's target when absent) to a
/// document position.
fn resolve_address(op: &Op, ids: &NodeIds, address: Option<&Value>) -> Result<Pos, Reject> {
    let node = address.and_then(|value| value.get("nodeId")).and_then(Value::as_u64);
    let node = match node {
        Some(node) => u32::try_from(node).map_err(|_| invalid(op, "`nodeId` does not fit"))?,
        None => op.target.node_id,
    };
    let off = match address.and_then(|value| value.get("off")) {
        Some(value) => {
            usize::try_from(value.as_u64().ok_or_else(|| invalid(op, "`off` must be a number"))?).map_err(|_| invalid(op, "`off` does not fit"))?
        }
        None => op.target.off,
    };
    let key = ids.resolve(NodeId(node)).map_err(|why| match why {
        wordcraft_collab::ids::Resolve::Gone(version) => {
            Reject::new(op.op_id.clone(), RejectCode::Gone, format!("node {node} was deleted at version {version}"))
        }
        wordcraft_collab::ids::Resolve::Unknown => Reject::new(op.op_id.clone(), RejectCode::Invalid, format!("unknown node {node}")),
    })?;
    let story = parse_story(&key.story).ok_or_else(|| invalid(op, format!("unknown story {:?}", key.story)))?;
    Ok(Pos { story, path: Path(key.path.clone()), off })
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
