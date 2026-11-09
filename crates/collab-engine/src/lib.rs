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

use wordcraft_collab::ids::{NodeId, NodeIds, NodeKey};
use wordcraft_collab::room::{Effect, Model};
use wordcraft_collab::wire::{Op, OpId, OpKind, Reject, RejectCode, Target};
use wordcraft_collab_doc::DocModel;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::table::MAX_TABLE_DEPTH;
use wordcraft_doc::{Block, Blocks, Document, ParaProps, Path, Pos, Run, StoryRef};
use wordcraft_engine::{Selection, Session};

/// Command areas that must never run from a network operation.
const DENIED_PREFIXES: &[&str] =
    &["file.", "tools.", "ui.", "view.", "caret.", "select.", "collab.", "edit.undo", "edit.redo", "edit.repeat", "review.readAloud"];

/// The frequent character-formatting commands whose result can be undone
/// exactly (ADR-0010). Every one of them only changes character properties —
/// a direct property, a character style, or a rebuild of the cleared ones. A
/// command that also moves text (`format.changeCase`) or replaces it
/// (`format.enclose` with `text`) is deliberately absent: its `set_para_format`
/// inverse would put formatting back over different text, so it stays an undo
/// barrier instead.
const CHAR_FORMAT_COMMANDS: &[&str] = &[
    "format.allCaps",
    "format.bold",
    "format.border",
    "format.charStyle",
    "format.clear",
    "format.color",
    "format.doubleStrikethrough",
    "format.doubleUnderline",
    "format.emboss",
    "format.emphasisMark",
    "format.engrave",
    "format.font",
    "format.growFont",
    "format.growFont1",
    "format.hidden",
    "format.highlight",
    "format.italic",
    "format.outline",
    "format.position",
    "format.resetChar",
    "format.scale",
    "format.set",
    "format.shading",
    "format.shadow",
    "format.shrinkFont",
    "format.shrinkFont1",
    "format.size",
    "format.smallCaps",
    "format.spacing",
    "format.strikethrough",
    "format.subscript",
    "format.superscript",
    "format.underline",
    "format.underlineColor",
    "format.wordUnderline",
];

/// The frequent paragraph-formatting commands with an exact inverse: they only
/// change paragraph properties (alignment, indents, spacing, keep rules, tabs,
/// borders, shading, direction, the paragraph style). Commands that move,
/// reorder or rewrite paragraphs (`para.sort`, `para.promote`, `para.moveUp`),
/// strip text (`para.removeBlank`) or allocate numbering definitions
/// (`para.bullets`) stay barriers.
const PARA_FORMAT_COMMANDS: &[&str] = &[
    "para.addSpaceBefore",
    "para.align",
    "para.alignCenter",
    "para.alignLeft",
    "para.alignRight",
    "para.asianTypography",
    "para.borders",
    "para.distribute",
    "para.double",
    "para.hangingIndent",
    "para.heading1",
    "para.heading2",
    "para.heading3",
    "para.indent",
    "para.indents",
    "para.justify",
    "para.keepLines",
    "para.keepNext",
    "para.lineSpacing",
    "para.listLevel",
    "para.ltr",
    "para.normal",
    "para.oneAndHalf",
    "para.outdent",
    "para.outlineLevel",
    "para.pageBreakBefore",
    "para.removeHanging",
    "para.reset",
    "para.rtl",
    "para.set",
    "para.shading",
    "para.single",
    "para.spacing",
    "para.style",
    "para.tabs",
    "para.widowControl",
];

/// The most paragraphs one formatting step restores in its inverse. A wider
/// selection is not carried as primitives: the step stays a barrier instead of
/// sending an unbounded transaction to every replica.
const MAX_FORMAT_PARAGRAPHS: usize = 256;

/// The formatting of one paragraph before a formatting command, captured so
/// the command's inverse can name the previous value exactly.
struct FormatSnapshot {
    story: StoryRef,
    path: Path,
    node_id: u32,
    text_len: usize,
    runs: Vec<Run>,
    mark: CharProps,
    props: ParaProps,
}

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
        // A formatting command gets the previous value of every paragraph it may
        // touch before it runs: its inverse is a primitive naming that value, not
        // a second command (ADR-0010). A paragraph the inverse cannot address is
        // refused here, before the document changes.
        let label = spec.label;
        let format = if is_format_command(command) { Some(self.capture_format(op, ids)?) } else { None };
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

        // A step that moved blocks (a command the adapter could not normalize)
        // cannot be restored by paragraph paths; formatting commands never do.
        let (inverse, touched) = match format {
            Some(captured) if !nodes_changed => self.format_inverse(op, &captured).unwrap_or_default(),
            _ => (Vec::new(), Vec::new()),
        };

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
            // A primitive (text, table, structural) has an exact inverse, and a
            // formatting command names its previous value field by field. Any
            // other command has none here, so the step is a barrier: undo
            // reports `unsupported` instead of skipping to an older step.
            inverse,
            forward: vec![op.clone()],
            label: if is_format_command(command) { label.to_string() } else { format!("Command {command}") },
            nodes_changed,
            touched,
        })
    }

    /// The formatting of every paragraph a formatting command may change,
    /// captured before it runs (ADR-0010). The command formats the selection,
    /// or the word at a collapsed caret, or the paragraph mark of an empty
    /// paragraph: all of them live in the paragraphs between the selection
    /// ends, so that range is the capture. A paragraph that has no session id
    /// cannot be named by the inverse, so the command is refused before it
    /// touches the document.
    fn capture_format(&self, op: &Op, ids: &NodeIds) -> Result<Vec<FormatSnapshot>, Reject> {
        let (a, b) = self.session.sel.ordered();
        if a.story != b.story {
            return Ok(Vec::new());
        }
        let name = story_name(a.story);
        let mut out = Vec::new();
        for path in self.session.doc.paths_between(&a, &b) {
            let Some(para) = self.session.doc.para(a.story, &path) else { continue };
            let node = ids
                .get(&NodeKey::new(name.clone(), path.0.clone()))
                .ok_or_else(|| invalid(op, format!("the paragraph {} has no session id, so its formatting cannot be undone", path_key(&path))))?;
            if out.len() >= MAX_FORMAT_PARAGRAPHS {
                return Err(invalid(op, "the selection covers too many paragraphs to undo its formatting"));
            }
            out.push(FormatSnapshot {
                story: a.story,
                path,
                node_id: node.0,
                text_len: para.text.len(),
                runs: para.runs.clone(),
                mark: para.mark.clone(),
                props: para.props.clone(),
            });
        }
        Ok(out)
    }

    /// The inverse of a formatting command: one `set_para_format` per paragraph
    /// whose formatting changed, carrying only the changed fields' previous
    /// values. `None` when the change cannot be restored exactly — the command
    /// moved text or the capture is too wide — so the step stays a barrier.
    fn format_inverse(&self, op: &Op, captured: &[FormatSnapshot]) -> Option<(Vec<Op>, Vec<u32>)> {
        let mut inverse = Vec::new();
        let mut touched = Vec::new();
        for before in captured {
            let para = self.session.doc.para(before.story, &before.path)?;
            if para.text.len() != before.text_len {
                return None;
            }
            let runs = para.runs != before.runs;
            let mark = para.mark != before.mark;
            let props = para.props != before.props;
            if !(runs || mark || props) {
                continue;
            }
            if inverse.len() >= MAX_FORMAT_PARAGRAPHS {
                return None;
            }
            let mut payload = json!({"op": "set_para_format"});
            if runs {
                payload["runs"] = serde_json::to_value(&before.runs).unwrap_or(Value::Null);
            }
            if mark {
                payload["mark"] = serde_json::to_value(&before.mark).unwrap_or(Value::Null);
            }
            if props {
                payload["props"] = serde_json::to_value(&before.props).unwrap_or(Value::Null);
            }
            inverse.push(Op {
                op_id: OpId::new(format!("inverse:{}", op.op_id.client), op.op_id.seq),
                base_version: 0,
                epoch: None,
                kind: OpKind::Primitive,
                target: Target::new(story_name(before.story), before.node_id, 0),
                payload,
                history: None,
            });
            touched.push(before.node_id);
        }
        Some((inverse, touched))
    }
}

fn is_format_command(command: &str) -> bool {
    CHAR_FORMAT_COMMANDS.contains(&command) || PARA_FORMAT_COMMANDS.contains(&command)
}

fn path_key(path: &Path) -> String {
    path.0.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
}

fn story_name(story: StoryRef) -> String {
    match story {
        StoryRef::Body => "body".to_string(),
        StoryRef::Part(id) => format!("part:{id}"),
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
