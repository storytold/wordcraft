//! WordCraft document adapter for the collaboration protocol.
//!
//! The room applies normalized `primitive` and `structural` operations to a
//! [`wordcraft_doc::Document`] (ADR-0001). Addresses are session node ids
//! (ADR-0002): the adapter resolves an id to the block path, applies the edit
//! through the document's own edit primitives (`insert_text`, `delete_range`,
//! `split_paragraph`, `insert_block`, `remove_block`), and rebases the id map
//! after a structural edit.
//!
//! The id map addresses every block of every container, not only top-level
//! ones: a table `[block]`, its rows `[block, row]`, its cells
//! `[block, row, cell]` and the paragraphs inside a cell `[block, row, cell,
//! block]` each get an id. Typing in a cell is therefore an ordinary
//! `insert_text`/`replace_range` on that paragraph, not a command that makes
//! every replica resync.
//!
//! Every applied operation produces an [`Effect`] with the inverse operations
//! the per-user undo stack stores (ADR-0010). Text inverses are exact
//! insert/delete pairs; structural inverses carry the removed block or row as
//! JSON, so undo restores it without a snapshot.
//!
//! What this adapter deliberately does not do (v1):
//! - it does not run `CommandSpec` commands (`kind: "command"` is refused);
//! - it does not touch styles, numbering, comments or track changes;
//! - it normalizes block insert/remove, row insert/remove and text; a column
//!   insert/remove, a cell merge or split and a text box stay commands.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_collab::ids::NodeKey;
use wordcraft_collab::ids::{NodeId, NodeIds};
use wordcraft_collab::room::{Effect, Model};
use wordcraft_collab::wire::{Op, OpId, OpKind, Reject, RejectCode, Target};
use wordcraft_doc::table::MAX_ROWS;
use wordcraft_doc::table::MAX_TABLE_DEPTH;
use wordcraft_doc::{Block, Blocks, CharProps, Document, Paragraph, Path, Pos, Row, StoryRef, Table};

/// Payload key that names the primitive (`insert_text`, `delete_range`, …).
pub const PAYLOAD_OP: &str = "op";

/// The WordCraft half of a room.
pub struct DocModel {
    doc: Document,
}

impl DocModel {
    pub fn new(doc: Document) -> DocModel {
        DocModel { doc }
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    /// Take the document back (used by adapters that borrow it for one call).
    pub fn into_document(self) -> Document {
        self.doc
    }

    fn primitive(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let name = op.payload.get(PAYLOAD_OP).and_then(Value::as_str).unwrap_or_default();
        match name {
            "insert_text" => self.insert_text(op, ids),
            "delete_range" => self.delete_range(op, ids),
            "replace_range" => self.replace_range(op, ids),
            "split_para" => self.split_para(op, ids),
            "merge_para" => self.merge_para(op, ids),
            other => Err(invalid(op, format!("unknown primitive {other:?}"))),
        }
    }

    fn structural(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let name = op.payload.get(PAYLOAD_OP).and_then(Value::as_str).unwrap_or_default();
        match name {
            "insert_block" => self.insert_block(op, ids),
            "remove_block" => self.remove_block(op, ids),
            "insert_row" => self.insert_row(op, ids),
            "remove_row" => self.remove_row(op, ids),
            other => Err(invalid(op, format!("unknown structural operation {other:?}"))),
        }
    }

    /// Resolve a text address: the id names the paragraph, `off` is a byte
    /// offset inside its text.
    fn resolve_pos(&self, op: &Op, ids: &NodeIds) -> Result<(StoryRef, Path, usize), Reject> {
        let key = ids.resolve(NodeId(op.target.node_id)).map_err(|why| match why {
            wordcraft_collab::ids::Resolve::Gone(version) => {
                Reject::new(op.op_id.clone(), RejectCode::Gone, format!("node {} was deleted at version {version}", op.target.node_id))
            }
            wordcraft_collab::ids::Resolve::Unknown => {
                Reject::new(op.op_id.clone(), RejectCode::Invalid, format!("unknown node {}", op.target.node_id))
            }
        })?;
        let story = parse_story(&key.story).ok_or_else(|| invalid(op, format!("unknown story {:?}", key.story)))?;
        Ok((story, Path(key.path.clone()), op.target.off))
    }

    fn insert_text(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, path, off) = self.resolve_pos(op, ids)?;
        let text = op.payload.get("text").and_then(Value::as_str).ok_or_else(|| invalid(op, "insert_text needs `text`"))?;
        if text.is_empty() {
            return Ok(Effect::default());
        }
        let pos = Pos { story, path: path.clone(), off };
        self.doc.insert_text(&pos, text, &CharProps::default()).map_err(|err| invalid(op, err.to_string()))?;
        let inverse = inverse_op(OpKind::Primitive, op, story, op.target.node_id, off, json!({"op": "delete_range", "len": text.len()}));
        Ok(Effect {
            delta: Some(json!({"op": "insert_text", "nodeId": op.target.node_id, "off": off, "text": text})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Typing".into(),
            nodes_changed: false,
        })
    }

    fn delete_range(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, path, off) = self.resolve_pos(op, ids)?;
        let len = op.payload.get("len").and_then(Value::as_u64).ok_or_else(|| invalid(op, "delete_range needs `len`"))?;
        let len = usize::try_from(len).map_err(|_| invalid(op, "`len` does not fit"))?;
        if len == 0 {
            return Ok(Effect::default());
        }
        let start = Pos { story, path: path.clone(), off };
        let end = Pos { story, path: path.clone(), off: off.saturating_add(len) };
        let removed = self.doc.copy_range(&start, &end).plain_text();
        self.doc.delete_range(&start, &end).map_err(|err| invalid(op, err.to_string()))?;
        let inverse = inverse_op(OpKind::Primitive, op, story, op.target.node_id, off, json!({"op": "insert_text", "text": removed}));
        Ok(Effect {
            delta: Some(json!({"op": "delete_range", "nodeId": op.target.node_id, "off": off, "len": len})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Delete".into(),
            nodes_changed: false,
        })
    }

    fn split_para(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, path, off) = self.resolve_pos(op, ids)?;
        let pos = Pos { story, path: path.clone(), off };
        self.doc.split_paragraph(&pos).map_err(|err| invalid(op, err.to_string()))?;
        // The new paragraph sits right after this one in the same container (a
        // story's block list or a table cell): shift that container only.
        let container = path.parent().to_vec();
        let at = path.last() + 1;
        ids.shift_children(&story_name(story), &container, at, 1);
        // The new paragraph can be edited immediately: give it an id and tell
        // clients about the map (the server broadcasts nodes for this effect).
        ids.id_for(&NodeKey::new(story_name(story), path.with_last(at).0));
        let inverse = inverse_op(OpKind::Primitive, op, story, op.target.node_id, off, json!({"op": "merge_para"}));
        Ok(Effect {
            delta: Some(json!({"op": "split_para", "nodeId": op.target.node_id, "off": off})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Paragraph break".into(),
            nodes_changed: true,
        })
    }

    /// Replace `len` bytes at `off` with `text`: one user action, one undo
    /// step, even when the client's diff covers a deletion and an insertion.
    fn replace_range(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, path, off) = self.resolve_pos(op, ids)?;
        let len = op.payload.get("len").and_then(Value::as_u64).ok_or_else(|| invalid(op, "replace_range needs `len`"))?;
        let len = usize::try_from(len).map_err(|_| invalid(op, "`len` does not fit"))?;
        let text = op.payload.get("text").and_then(Value::as_str).ok_or_else(|| invalid(op, "replace_range needs `text`"))?;
        let end = off.saturating_add(len);
        let para_len = self.doc.para(story, &path).map_or(0, |para| para.text.len());
        if end > para_len {
            return Err(invalid(op, "replace_range runs past the paragraph"));
        }
        let start = Pos { story, path: path.clone(), off };
        let finish = Pos { story, path: path.clone(), off: end };
        let removed = self.doc.copy_range(&start, &finish).plain_text();
        self.doc.delete_range(&start, &finish).map_err(|err| invalid(op, err.to_string()))?;
        self.doc.insert_text(&start, text, &CharProps::default()).map_err(|err| invalid(op, err.to_string()))?;
        let inverse =
            inverse_op(OpKind::Primitive, op, story, op.target.node_id, off, json!({"op": "replace_range", "len": text.len(), "text": removed}));
        Ok(Effect {
            delta: Some(json!({"op": "replace_range", "nodeId": op.target.node_id, "off": off, "len": len, "text": text})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Replace".into(),
            nodes_changed: false,
        })
    }

    /// Join this paragraph with the next one (the inverse of `split_para`).
    fn merge_para(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, path, _off) = self.resolve_pos(op, ids)?;
        let para_len = self.doc.para(story, &path).map_or(0, |p| p.text.len());
        let next = path.with_last(path.last() + 1);
        let start = Pos { story, path: path.clone(), off: para_len };
        let end = Pos { story, path: next.clone(), off: 0 };
        self.doc.delete_range(&start, &end).map_err(|err| invalid(op, err.to_string()))?;
        let key = NodeKey::new(story_name(story), next.0.clone());
        if let Some(id) = ids.get(&key) {
            ids.remove(id, op.base_version);
        }
        // Everything after the joined paragraph moves one place up, in this
        // container only: a table cell keeps its own block numbering.
        ids.shift_children(&story_name(story), path.parent(), next.last(), -1);
        let inverse = inverse_op(OpKind::Primitive, op, story, op.target.node_id, para_len, json!({"op": "split_para"}));
        Ok(Effect {
            delta: Some(json!({"op": "merge_para", "nodeId": op.target.node_id})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Join paragraphs".into(),
            nodes_changed: true,
        })
    }

    fn insert_block(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let story = parse_story(&op.target.story).ok_or_else(|| invalid(op, format!("unknown story {:?}", op.target.story)))?;
        let (container, index) = self.block_position(op, ids, false)?;
        let block = match op.payload.get("block") {
            Some(value) if !value.is_null() => {
                serde_json::from_value::<Block>(value.clone()).map_err(|err| invalid(op, format!("bad block: {err}")))?
            }
            _ => Block::Para(Paragraph::new()),
        };
        let block_json = serde_json::to_value(&block).unwrap_or(Value::Null);
        let target = child_path(&container, index);
        self.doc.insert_block(story, &target, block).map_err(|err| invalid(op, err.to_string()))?;
        let name = story_name(story);
        ids.shift_children(&name, &container, index, 1);
        // The new block can be edited immediately, and a table brings its rows
        // and cells: issue their ids and tell clients about the map.
        if let Some(inserted) = self.doc.block(story, &target) {
            index_block(ids, &name, &mut container.clone(), index, inserted, 0);
        }
        let inverse = inverse_op(OpKind::Structural, op, story, 0, 0, block_payload("remove_block", &container, index, None));
        Ok(Effect {
            delta: Some(json!({
                "op": "insert_block",
                "story": name,
                "index": index,
                "path": container,
                "block": block_json,
            })),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Insert block".into(),
            nodes_changed: true,
        })
    }

    fn remove_block(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let story = parse_story(&op.target.story).ok_or_else(|| invalid(op, format!("unknown story {:?}", op.target.story)))?;
        let (container, index) = self.block_position(op, ids, true)?;
        let target = child_path(&container, index);
        let removed = self.doc.remove_block(story, &target).map_err(|err| invalid(op, err.to_string()))?;
        let name = story_name(story);
        // The block takes everything inside it (a table its rows and cells) out
        // of the map, so a late edit is refused instead of landing elsewhere.
        ids.remove_subtree(&name, &target.0, op.base_version);
        ids.shift_children(&name, &container, index + 1, -1);
        let block = serde_json::to_value(&removed).unwrap_or(Value::Null);
        let inverse = inverse_op(OpKind::Structural, op, story, 0, 0, block_payload("insert_block", &container, index, Some(block)));
        Ok(Effect {
            delta: Some(json!({"op": "remove_block", "story": name, "index": index, "path": container})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Delete block".into(),
            nodes_changed: true,
        })
    }

    /// Insert a row into the table the operation addresses. `payload.index` is
    /// the row position and `payload.row` the row itself: the authoritative
    /// delta and its inverse carry the body, so a replica does not have to
    /// guess which neighbour the row was copied from.
    fn insert_row(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, table_path) = self.table_path(op, ids)?;
        let index = self.row_index(op)?;
        let row = match op.payload.get("row") {
            Some(value) if !value.is_null() => serde_json::from_value::<Row>(value.clone()).map_err(|err| invalid(op, format!("bad row: {err}")))?,
            _ => return Err(invalid(op, "insert_row needs `row`")),
        };
        let name = story_name(story);
        let table = self.doc.table_mut(story, &table_path).map_err(|err| invalid(op, err.to_string()))?;
        if table.rows.len() >= MAX_ROWS {
            return Err(invalid(op, "the table is full"));
        }
        let at = index.min(table.rows.len());
        table.rows.insert(at, row.clone());
        let row_json = serde_json::to_value(&row).unwrap_or(Value::Null);
        let table_node = ids.id_for(&NodeKey::new(name.clone(), table_path.0.clone()));
        ids.shift_children(&name, &table_path.0, at, 1);
        index_row(ids, &name, &mut table_path.0.clone(), at, &row, 0);
        let inverse = inverse_op(OpKind::Structural, op, story, table_node.0, 0, json!({"op": "remove_row", "index": at}));
        Ok(Effect {
            delta: Some(json!({"op": "insert_row", "nodeId": table_node.0, "index": at, "row": row_json})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Insert row".into(),
            nodes_changed: true,
        })
    }

    fn remove_row(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, table_path) = self.table_path(op, ids)?;
        let index = self.row_index(op)?;
        let name = story_name(story);
        let table = self.doc.table_mut(story, &table_path).map_err(|err| invalid(op, err.to_string()))?;
        if index >= table.rows.len() {
            return Err(invalid(op, "the row does not exist"));
        }
        let removed = table.rows.remove(index);
        let row = serde_json::to_value(&removed).unwrap_or(Value::Null);
        let gone = child_path(&table_path.0, index);
        ids.remove_subtree(&name, &gone.0, op.base_version);
        ids.shift_children(&name, &table_path.0, index + 1, -1);
        let table_node = ids.id_for(&NodeKey::new(name, table_path.0.clone()));
        let inverse = inverse_op(OpKind::Structural, op, story, table_node.0, 0, json!({"op": "insert_row", "index": index, "row": row}));
        Ok(Effect {
            delta: Some(json!({"op": "remove_row", "nodeId": table_node.0, "index": index})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Delete row".into(),
            nodes_changed: true,
        })
    }

    /// The container and the index an insert/remove addresses.
    ///
    /// `payload.path` names the container: absent or empty is the story's block
    /// list, `[b, r, c]` the block list of a cell (a table's row list and a
    /// row's cell list are not block lists). `payload.index` is the position in
    /// it. Without an index the target node supplies both: its own container and
    /// its position (plus one for an insert, so the new block goes after it).
    fn block_position(&self, op: &Op, ids: &NodeIds, exact_node: bool) -> Result<(Vec<u32>, usize), Reject> {
        let container = match op.payload.get("path") {
            Some(Value::Array(items)) => Some(indices(op, items)?),
            Some(value) if !value.is_null() => return Err(invalid(op, "`path` must be a list of indexes")),
            _ => None,
        };
        if let Some(index) = op.payload.get("index") {
            let index = usize::try_from(index.as_u64().ok_or_else(|| invalid(op, "`index` must be a number"))?)
                .map_err(|_| invalid(op, "`index` does not fit"))?;
            return Ok((container.unwrap_or_default(), index));
        }
        if container.is_some() {
            return Err(invalid(op, "insert/remove_block with `path` needs `index`"));
        }
        let Some(key) = ids.resolve(NodeId(op.target.node_id)).ok() else {
            return Err(invalid(op, "insert/remove_block needs `index` or a known node"));
        };
        let container = key.path.get(..key.path.len().saturating_sub(1)).unwrap_or_default().to_vec();
        let at = key.path.last().copied().unwrap_or(0) as usize;
        Ok((container, if exact_node { at } else { at + 1 }))
    }

    /// The table a row operation addresses: the target node is the table itself
    /// or any node inside one.
    fn table_path(&self, op: &Op, ids: &NodeIds) -> Result<(StoryRef, Path), Reject> {
        let key = ids.resolve(NodeId(op.target.node_id)).map_err(|why| match why {
            wordcraft_collab::ids::Resolve::Gone(version) => {
                Reject::new(op.op_id.clone(), RejectCode::Gone, format!("node {} was deleted at version {version}", op.target.node_id))
            }
            wordcraft_collab::ids::Resolve::Unknown => {
                Reject::new(op.op_id.clone(), RejectCode::Invalid, format!("unknown node {}", op.target.node_id))
            }
        })?;
        let story = parse_story(&key.story).ok_or_else(|| invalid(op, format!("unknown story {:?}", key.story)))?;
        let mut path = Path(key.path.clone());
        for _ in 0..=(MAX_TABLE_DEPTH.saturating_mul(4).saturating_add(4)) {
            if self.doc.table(story, &path).is_some() {
                return Ok((story, path));
            }
            let Some(parent) = enclosing_table(&path) else { break };
            path = parent;
        }
        Err(invalid(op, "the operation is not addressed at a table or a cell of one"))
    }

    fn row_index(&self, op: &Op) -> Result<usize, Reject> {
        let index = op.payload.get("index").and_then(Value::as_u64).ok_or_else(|| invalid(op, "a row operation needs `index`"))?;
        usize::try_from(index).map_err(|_| invalid(op, "`index` does not fit"))
    }
}

impl Model for DocModel {
    fn apply(&mut self, _actor: &str, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        match op.kind {
            OpKind::Primitive => self.primitive(op, ids),
            OpKind::Structural => self.structural(op, ids),
            OpKind::Command => Err(invalid(op, "commands are not wired to the document adapter yet")),
            OpKind::Merge => Ok(Effect::default()),
        }
    }

    /// Native JSON of the model. The room stores it next to the source `.docx`
    /// bytes, because `Document::kept` is not serialized (ADR-0005).
    fn snapshot(&self) -> Vec<u8> {
        serde_json::to_vec(&self.doc).unwrap_or_default()
    }

    /// Issue ids for every block of every container: paragraphs, tables, the
    /// rows and cells of a table, and the blocks inside each cell.
    fn index_nodes(&self, ids: &mut NodeIds) {
        index_blocks(ids, "body", &mut Vec::new(), &self.doc.body, 0);
        for (part, story) in &self.doc.parts {
            let name = format!("part:{part}");
            index_blocks(ids, &name, &mut Vec::new(), &story.blocks, 0);
        }
    }
}

/// Issue ids for a block list and everything it contains. The walk is bounded
/// like the document's own paragraph walk, so a hostile snapshot cannot nest
/// tables without end.
fn index_blocks(ids: &mut NodeIds, story: &str, prefix: &mut Vec<u32>, blocks: &Blocks, depth: usize) {
    if depth > MAX_TABLE_DEPTH {
        return;
    }
    for (index, block) in blocks.iter().enumerate() {
        index_block(ids, story, prefix, index, block, depth);
    }
}

fn index_block(ids: &mut NodeIds, story: &str, container: &mut Vec<u32>, index: usize, block: &Block, depth: usize) {
    container.push(index as u32);
    ids.id_for(&NodeKey::new(story, container.clone()));
    if let Block::Table(table) = block {
        index_table(ids, story, container, table, depth.saturating_add(1));
    }
    container.pop();
}

fn index_table(ids: &mut NodeIds, story: &str, table_path: &mut Vec<u32>, table: &Table, depth: usize) {
    if depth > MAX_TABLE_DEPTH {
        return;
    }
    for (r, row) in table.rows.iter().enumerate() {
        index_row(ids, story, table_path, r, row, depth);
    }
}

fn index_row(ids: &mut NodeIds, story: &str, table_path: &mut Vec<u32>, index: usize, row: &Row, depth: usize) {
    table_path.push(index as u32);
    ids.id_for(&NodeKey::new(story, table_path.clone()));
    for (c, cell) in row.cells.iter().enumerate() {
        table_path.push(c as u32);
        ids.id_for(&NodeKey::new(story, table_path.clone()));
        index_blocks(ids, story, table_path, &cell.blocks, depth.saturating_add(1));
        table_path.pop();
    }
    table_path.pop();
}

/// The path of a child of `container` (a block list, a row list or a cell list).
fn child_path(container: &[u32], index: usize) -> Path {
    Path(container.iter().copied().chain(std::iter::once(index as u32)).collect())
}

/// The table that encloses `path`, one level up: a row id `[b, r]` and a cell id
/// `[b, r, c]` name their table directly, a block inside a cell goes through
/// the cell's table path.
fn enclosing_table(path: &Path) -> Option<Path> {
    let len = path.0.len();
    match len % 3 {
        2 => Some(Path(path.0.get(..len.checked_sub(1)?)?.to_vec())),
        0 if len > 0 => Some(Path(path.0.get(..len.checked_sub(2)?)?.to_vec())),
        _ => path.cell().map(|(table, _, _)| table),
    }
}

fn indices(op: &Op, items: &[Value]) -> Result<Vec<u32>, Reject> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let index = item.as_u64().ok_or_else(|| invalid(op, "`path` must be a list of indexes"))?;
        out.push(u32::try_from(index).map_err(|_| invalid(op, "a `path` index does not fit"))?);
    }
    Ok(out)
}

/// An inverse structural payload; the container travels only when it is not the
/// story's block list, so a flat document's operations stay as they were.
fn block_payload(name: &str, container: &[u32], index: usize, block: Option<Value>) -> Value {
    let mut payload = json!({"op": name, "index": index});
    if !container.is_empty() {
        payload["path"] = json!(container);
    }
    if let Some(block) = block {
        payload["block"] = block;
    }
    payload
}

/// Apply the authoritative delta of one operation to a replica's document and
/// its session-node map (RFC §3.1, ADR-0001). The server already validated the
/// operation; a replica only mirrors the result, so this path never rejects for
/// policy reasons. `history` is not touched: remote edits must not enter the
/// user's local undo stack.
///
/// A `command` delta needs a `Session` (engine commands), so it is refused here
/// and the caller replays it with `Session::run` on a session-aware path.
pub fn apply_delta(doc: &mut Document, ids: &mut NodeIds, delta: &Value) -> Result<(), String> {
    let name = delta.get("op").and_then(Value::as_str).ok_or_else(|| "delta has no `op`".to_string())?;
    match name {
        "insert_text" => {
            let (story, path, off) = delta_pos(delta, ids)?;
            let text = delta.get("text").and_then(Value::as_str).ok_or_else(|| "insert_text has no `text`".to_string())?;
            doc.insert_text(&Pos { story, path, off }, text, &CharProps::default()).map_err(|err| err.to_string())?;
        }
        "delete_range" => {
            let (story, path, off) = delta_pos(delta, ids)?;
            let len = delta_len(delta)?;
            let start = Pos { story, path: path.clone(), off };
            let end = Pos { story, path, off: off.saturating_add(len) };
            doc.delete_range(&start, &end).map_err(|err| err.to_string())?;
        }
        "replace_range" => {
            let (story, path, off) = delta_pos(delta, ids)?;
            let len = delta_len(delta)?;
            let text = delta.get("text").and_then(Value::as_str).ok_or_else(|| "replace_range has no `text`".to_string())?;
            let start = Pos { story, path: path.clone(), off };
            let end = Pos { story, path, off: off.saturating_add(len) };
            doc.delete_range(&start, &end).map_err(|err| err.to_string())?;
            doc.insert_text(&start, text, &CharProps::default()).map_err(|err| err.to_string())?;
        }
        "split_para" => {
            let (story, path, off) = delta_pos(delta, ids)?;
            doc.split_paragraph(&Pos { story, path: path.clone(), off }).map_err(|err| err.to_string())?;
            let container = path.parent().to_vec();
            ids.shift_children(&story_name(story), &container, path.last() + 1, 1);
        }
        "merge_para" => {
            let (story, path, _) = delta_pos(delta, ids)?;
            let para_len = doc.para(story, &path).map_or(0, |para| para.text.len());
            let next = path.with_last(path.last() + 1);
            let start = Pos { story, path: path.clone(), off: para_len };
            let end = Pos { story, path: next.clone(), off: 0 };
            doc.delete_range(&start, &end).map_err(|err| err.to_string())?;
            ids.remove_subtree(&story_name(story), &next.0, 0);
            ids.shift_children(&story_name(story), path.parent(), next.last(), -1);
        }
        "insert_block" => {
            let story =
                delta.get("story").and_then(Value::as_str).and_then(parse_story).ok_or_else(|| "insert_block has no known `story`".to_string())?;
            let container = delta_indices(delta)?;
            let index = usize::try_from(delta.get("index").and_then(Value::as_u64).ok_or_else(|| "insert_block has no `index`".to_string())?)
                .map_err(|_| "`index` does not fit".to_string())?;
            let block = match delta.get("block") {
                Some(value) if !value.is_null() => serde_json::from_value::<Block>(value.clone()).map_err(|err| format!("bad block: {err}"))?,
                _ => Block::Para(Paragraph::new()),
            };
            let target = child_path(&container, index);
            doc.insert_block(story, &target, block).map_err(|err| err.to_string())?;
            let name = story_name(story);
            ids.shift_children(&name, &container, index, 1);
            // A block that allocates ids (a table) gets them here, so the
            // replica can address what it just drew before the map arrives.
            if let Some(inserted) = doc.block(story, &target) {
                index_block(ids, &name, &mut container.clone(), index, inserted, 0);
            }
        }
        "remove_block" => {
            let story =
                delta.get("story").and_then(Value::as_str).and_then(parse_story).ok_or_else(|| "remove_block has no known `story`".to_string())?;
            let container = delta_indices(delta)?;
            let index = usize::try_from(delta.get("index").and_then(Value::as_u64).ok_or_else(|| "remove_block has no `index`".to_string())?)
                .map_err(|_| "`index` does not fit".to_string())?;
            let target = child_path(&container, index);
            doc.remove_block(story, &target).map_err(|err| err.to_string())?;
            let name = story_name(story);
            ids.remove_subtree(&name, &target.0, 0);
            ids.shift_children(&name, &container, index + 1, -1);
        }
        "insert_row" => {
            let (story, table_path) = delta_table(delta, doc, ids)?;
            let index = usize::try_from(delta.get("index").and_then(Value::as_u64).ok_or_else(|| "insert_row has no `index`".to_string())?)
                .map_err(|_| "`index` does not fit".to_string())?;
            let row = match delta.get("row") {
                Some(value) if !value.is_null() => serde_json::from_value::<Row>(value.clone()).map_err(|err| format!("bad row: {err}"))?,
                _ => return Err("insert_row has no `row`".to_string()),
            };
            let table = doc.table_mut(story, &table_path).map_err(|err| err.to_string())?;
            if table.rows.len() >= MAX_ROWS {
                return Err("the table is full".to_string());
            }
            let at = index.min(table.rows.len());
            table.rows.insert(at, row.clone());
            let name = story_name(story);
            ids.shift_children(&name, &table_path.0, at, 1);
            index_row(ids, &name, &mut table_path.0.clone(), at, &row, 0);
        }
        "remove_row" => {
            let (story, table_path) = delta_table(delta, doc, ids)?;
            let index = usize::try_from(delta.get("index").and_then(Value::as_u64).ok_or_else(|| "remove_row has no `index`".to_string())?)
                .map_err(|_| "`index` does not fit".to_string())?;
            let table = doc.table_mut(story, &table_path).map_err(|err| err.to_string())?;
            if index >= table.rows.len() {
                return Err("the row does not exist".to_string());
            }
            table.rows.remove(index);
            let name = story_name(story);
            let gone = child_path(&table_path.0, index);
            ids.remove_subtree(&name, &gone.0, 0);
            ids.shift_children(&name, &table_path.0, index + 1, -1);
        }
        "command" => return Err("a command delta needs a Session, not a bare Document".to_string()),
        // One undo step may change several places; replicas must land all of
        // them or none, so the whole transaction runs on a scratch copy.
        "transaction" => {
            let ops = delta.get("ops").and_then(Value::as_array).ok_or_else(|| "transaction has no `ops`".to_string())?;
            if ops.is_empty() {
                return Err("transaction has no operations".to_string());
            }
            let mut scratch_doc = doc.clone();
            let mut scratch_ids = ids.clone();
            for op in ops {
                if op.get("op").and_then(Value::as_str) == Some("transaction") {
                    return Err("a transaction cannot be nested".to_string());
                }
                apply_delta(&mut scratch_doc, &mut scratch_ids, op)?;
            }
            *doc = scratch_doc;
            *ids = scratch_ids;
        }
        other => return Err(format!("unknown delta {other:?}")),
    }
    Ok(())
}

/// One local change the client sends as an operation, derived from the
/// difference between the document before and after a user's command.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalChange {
    pub kind: OpKind,
    pub target: Target,
    pub payload: Value,
}

/// Why a document difference cannot be expressed as primitives.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffError {
    /// A change the primitives do not cover (tables, formatting, several moves).
    /// The caller can fall back to a command operation or ask for a resync.
    Unsupported(String),
    /// A paragraph the server has not issued an id for yet.
    MissingId { story: String, block: usize },
}

impl std::fmt::Display for DiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffError::Unsupported(what) => write!(f, "unsupported change: {what}"),
            DiffError::MissingId { story, block } => write!(f, "no session id for {story} block {block}"),
        }
    }
}

/// Turn the difference between `before` and `after` into the operations the
/// server understands. Cloning a `Document` shares its blocks (`Arc`), so a
/// command's before-image is cheap and untouched blocks are skipped by pointer.
/// The walk descends into table cells, so typing in a cell is a `replace_range`
/// on that paragraph's id and a row insert/remove is an `insert_row`/
/// `remove_row`; formatting-only changes, a cell count or column change and
/// multi-block rearrangements are reported as unsupported.
pub fn diff(before: &Document, after: &Document, ids: &NodeIds) -> Result<Vec<LocalChange>, DiffError> {
    let mut changes = Vec::new();
    diff_blocks("body", &mut Vec::new(), &before.body, &after.body, ids, &mut changes)?;
    for (part, old) in &before.parts {
        let Some(new) = after.parts.get(part) else { return Err(DiffError::Unsupported(format!("story part:{part} removed"))) };
        diff_blocks(&format!("part:{part}"), &mut Vec::new(), &old.blocks, &new.blocks, ids, &mut changes)?;
    }
    for part in after.parts.keys() {
        if !before.parts.contains_key(part) {
            return Err(DiffError::Unsupported(format!("story part:{part} added")));
        }
    }
    Ok(changes)
}

/// Diff one block list (a story, a table cell, a nested cell).
fn diff_blocks(
    story: &str,
    prefix: &mut Vec<u32>,
    old: &[Arc<Block>],
    new: &[Arc<Block>],
    ids: &NodeIds,
    out: &mut Vec<LocalChange>,
) -> Result<(), DiffError> {
    if old.len() == new.len() {
        for (block, (a, b)) in old.iter().zip(new).enumerate() {
            if Arc::ptr_eq(a, b) {
                continue;
            }
            prefix.push(block as u32);
            let result = diff_block(story, prefix, a, b, ids, out);
            prefix.pop();
            result?;
        }
        return Ok(());
    }

    let where_ = block_where(story, prefix);

    if new.len() == old.len() + 1 {
        if let Some(at) = split_point(old, new) {
            let id = node_id(ids, story, prefix, at)?;
            let off = new[at].as_para().map(|para| para.text.len()).unwrap_or(0);
            out.push(LocalChange { kind: OpKind::Primitive, target: Target::new(story, id, off), payload: json!({"op": "split_para"}) });
            return Ok(());
        }
        let Some(at) = one_insertion(old, new) else {
            return Err(DiffError::Unsupported(format!("several blocks changed in {where_}")));
        };
        let block = serde_json::to_value(new[at].as_ref()).map_err(|err| DiffError::Unsupported(err.to_string()))?;
        out.push(LocalChange {
            kind: OpKind::Structural,
            target: Target::new(story, 0, 0),
            payload: block_payload("insert_block", prefix, at, Some(block)),
        });
        return Ok(());
    }

    if old.len() == new.len() + 1 {
        if let Some(at) = merge_point(old, new) {
            let id = node_id(ids, story, prefix, at)?;
            let off = new[at].as_para().map(|para| para.text.len()).unwrap_or(0);
            out.push(LocalChange { kind: OpKind::Primitive, target: Target::new(story, id, off), payload: json!({"op": "merge_para"}) });
            return Ok(());
        }
        let Some(at) = one_removal(old, new) else {
            return Err(DiffError::Unsupported(format!("several blocks changed in {where_}")));
        };
        out.push(LocalChange {
            kind: OpKind::Structural,
            target: Target::new(story, 0, 0),
            payload: block_payload("remove_block", prefix, at, None),
        });
        return Ok(());
    }

    Err(DiffError::Unsupported(format!("{} blocks became {} in {where_}", old.len(), new.len())))
}

/// Diff one block that changed in place: its text, or everything inside it when
/// the block is a table.
fn diff_block(story: &str, path: &[u32], old: &Block, new: &Block, ids: &NodeIds, out: &mut Vec<LocalChange>) -> Result<(), DiffError> {
    match (old, new) {
        (Block::Para(before), Block::Para(after)) => {
            let Some((off, len, text)) = text_change(&before.text, &after.text) else {
                return Err(DiffError::Unsupported(format!("block {} in {story} changed without text (formatting)", path_text(path))));
            };
            let id = ids
                .get(&NodeKey::new(story, path.to_vec()))
                .ok_or_else(|| DiffError::MissingId { story: story.to_string(), block: path.last().copied().unwrap_or(0) as usize })?;
            out.push(LocalChange {
                kind: OpKind::Primitive,
                target: Target::new(story, id.0, off),
                payload: json!({"op": "replace_range", "len": len, "text": text}),
            });
            Ok(())
        }
        (Block::Table(before), Block::Table(after)) => diff_table(story, path, before, after, ids, out),
        _ => Err(DiffError::Unsupported(format!("block {} in {story} is a table or changed kind", path_text(path)))),
    }
}

/// Diff a table: edits inside its cells paragraph by paragraph, and a row that
/// was added or removed as `insert_row`/`remove_row`.
fn diff_table(story: &str, table_path: &[u32], old: &Table, new: &Table, ids: &NodeIds, out: &mut Vec<LocalChange>) -> Result<(), DiffError> {
    if old.rows.len() == new.rows.len() {
        for (r, (a, b)) in old.rows.iter().zip(&new.rows).enumerate() {
            if same_row(a, b) {
                continue;
            }
            if a.cells.len() != b.cells.len() || a.props != b.props || a.controls != b.controls {
                return Err(DiffError::Unsupported(format!("row {r} of the table at {} changed shape", path_text(table_path))));
            }
            for (c, (before, after)) in a.cells.iter().zip(&b.cells).enumerate() {
                if same_cell(before, after) {
                    continue;
                }
                let mut cell_path = table_path.to_vec();
                cell_path.push(r as u32);
                cell_path.push(c as u32);
                diff_blocks(story, &mut cell_path, &before.blocks, &after.blocks, ids, out)?;
            }
        }
        return Ok(());
    }

    let where_ = format!("the table at {}", path_text(table_path));

    if new.rows.len() == old.rows.len() + 1 {
        let Some(at) = one_row_insertion(old, new) else {
            return Err(DiffError::Unsupported(format!("several rows changed in {where_}")));
        };
        let id = node_id(ids, story, table_path, usize::MAX)?;
        let row = serde_json::to_value(&new.rows[at]).map_err(|err| DiffError::Unsupported(err.to_string()))?;
        out.push(LocalChange {
            kind: OpKind::Structural,
            target: Target::new(story, id, 0),
            payload: json!({"op": "insert_row", "index": at, "row": row}),
        });
        return Ok(());
    }

    if old.rows.len() == new.rows.len() + 1 {
        let Some(at) = one_row_removal(old, new) else {
            return Err(DiffError::Unsupported(format!("several rows changed in {where_}")));
        };
        let id = node_id(ids, story, table_path, usize::MAX)?;
        out.push(LocalChange { kind: OpKind::Structural, target: Target::new(story, id, 0), payload: json!({"op": "remove_row", "index": at}) });
        return Ok(());
    }

    Err(DiffError::Unsupported(format!("{} rows became {} in {where_}", old.rows.len(), new.rows.len())))
}

/// The id of the child at `index` of `prefix`, or of `prefix` itself when
/// `index` is [`usize::MAX`] (a table, addressed by its own node).
fn node_id(ids: &NodeIds, story: &str, prefix: &[u32], index: usize) -> Result<u32, DiffError> {
    let mut path = prefix.to_vec();
    if index != usize::MAX {
        path.push(index as u32);
    }
    ids.get(&NodeKey::new(story, path))
        .map(|id| id.0)
        .ok_or_else(|| DiffError::MissingId { story: story.to_string(), block: prefix.last().copied().unwrap_or(0) as usize })
}

fn block_where(story: &str, prefix: &[u32]) -> String {
    if prefix.is_empty() { story.to_string() } else { format!("{} of {story}", path_text(prefix)) }
}

fn path_text(path: &[u32]) -> String {
    path.iter().map(ToString::to_string).collect::<Vec<_>>().join("/")
}

/// Same row, judged without walking its text: props, controls, and the cell
/// blocks by pointer (an untouched block is shared through `Arc`).
fn same_row(a: &Row, b: &Row) -> bool {
    a.props == b.props && a.controls == b.controls && a.cells.len() == b.cells.len() && a.cells.iter().zip(&b.cells).all(|(x, y)| same_cell(x, y))
}

fn same_cell(a: &wordcraft_doc::Cell, b: &wordcraft_doc::Cell) -> bool {
    a.props == b.props
        && a.controls == b.controls
        && a.blocks.len() == b.blocks.len()
        && a.blocks.iter().zip(&b.blocks).all(|(x, y)| Arc::ptr_eq(x, y))
}

fn one_row_insertion(old: &Table, new: &Table) -> Option<usize> {
    (0..new.rows.len()).find(|&at| {
        old.rows.iter().take(at).zip(&new.rows).all(|(a, b)| same_row(a, b))
            && old.rows.iter().skip(at).zip(new.rows.iter().skip(at + 1)).all(|(a, b)| same_row(a, b))
    })
}

fn one_row_removal(old: &Table, new: &Table) -> Option<usize> {
    (0..old.rows.len()).find(|&at| {
        old.rows.iter().take(at).zip(&new.rows).all(|(a, b)| same_row(a, b))
            && old.rows.iter().skip(at + 1).zip(new.rows.iter().skip(at)).all(|(a, b)| same_row(a, b))
    })
}

/// A paragraph split in two: the neighbours are untouched and the two new
/// paragraphs concatenate to the old text.
fn split_point(old: &[std::sync::Arc<Block>], new: &[std::sync::Arc<Block>]) -> Option<usize> {
    (0..old.len()).find(|&at| {
        let (Some(Block::Para(before)), Some(Block::Para(first)), Some(Block::Para(second))) =
            (old.get(at).map(|b| b.as_ref()), new.get(at).map(|b| b.as_ref()), new.get(at + 1).map(|b| b.as_ref()))
        else {
            return false;
        };
        before.text == format!("{}{}", first.text, second.text)
            && before.props == first.props
            && before.props == second.props
            && old.iter().take(at).zip(new).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
            && old.iter().skip(at + 1).zip(new.iter().skip(at + 2)).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
    })
}

/// Two paragraphs joined: the neighbours are untouched and the old texts
/// concatenate to the new one.
fn merge_point(old: &[std::sync::Arc<Block>], new: &[std::sync::Arc<Block>]) -> Option<usize> {
    (0..new.len()).find(|&at| {
        let (Some(Block::Para(first)), Some(Block::Para(second)), Some(Block::Para(merged))) =
            (old.get(at).map(|b| b.as_ref()), old.get(at + 1).map(|b| b.as_ref()), new.get(at).map(|b| b.as_ref()))
        else {
            return false;
        };
        merged.text == format!("{}{}", first.text, second.text)
            && first.props == merged.props
            && first.props == second.props
            && old.iter().take(at).zip(new).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
            && old.iter().skip(at + 2).zip(new.iter().skip(at + 1)).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
    })
}

fn one_insertion(old: &[std::sync::Arc<Block>], new: &[std::sync::Arc<Block>]) -> Option<usize> {
    (0..new.len()).find(|&at| {
        old.iter().take(at).zip(new).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
            && old.iter().skip(at).zip(new.iter().skip(at + 1)).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
    })
}

fn one_removal(old: &[std::sync::Arc<Block>], new: &[std::sync::Arc<Block>]) -> Option<usize> {
    (0..old.len()).find(|&at| {
        old.iter().take(at).zip(new).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
            && old.iter().skip(at + 1).zip(new.iter().skip(at)).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
    })
}

/// The single byte range that differs between two strings, cut on character
/// boundaries: `(offset, removed_len, inserted_text)`.
fn text_change(before: &str, after: &str) -> Option<(usize, usize, String)> {
    if before == after {
        return None;
    }
    let mut prefix = before.bytes().zip(after.bytes()).take_while(|(a, b)| a == b).count();
    while prefix > 0 && (!before.is_char_boundary(prefix) || !after.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let mut suffix =
        before.bytes().rev().zip(after.bytes().rev()).take_while(|(a, b)| a == b).count().min(before.len() - prefix).min(after.len() - prefix);
    while suffix > 0 && (!before.is_char_boundary(before.len() - suffix) || !after.is_char_boundary(after.len() - suffix)) {
        suffix -= 1;
    }
    Some((prefix, before.len() - prefix - suffix, after[prefix..after.len() - suffix].to_string()))
}

fn delta_pos(delta: &Value, ids: &NodeIds) -> Result<(StoryRef, Path, usize), String> {
    let node = delta.get("nodeId").and_then(Value::as_u64).ok_or_else(|| "delta has no `nodeId`".to_string())?;
    let node = u32::try_from(node).map_err(|_| "`nodeId` does not fit".to_string())?;
    let key = ids.resolve(NodeId(node)).map_err(|_| format!("unknown node {node}"))?;
    let story = parse_story(&key.story).ok_or_else(|| format!("unknown story {:?}", key.story))?;
    let off = usize::try_from(delta.get("off").and_then(Value::as_u64).unwrap_or(0)).map_err(|_| "`off` does not fit".to_string())?;
    Ok((story, Path(key.path.clone()), off))
}

/// The container a block delta names: an absent or empty `path` is the story's
/// block list.
fn delta_indices(delta: &Value) -> Result<Vec<u32>, String> {
    let Some(items) = delta.get("path").filter(|value| !value.is_null()) else { return Ok(Vec::new()) };
    let items = items.as_array().ok_or_else(|| "`path` must be a list of indexes".to_string())?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let index = item.as_u64().ok_or_else(|| "`path` must be a list of indexes".to_string())?;
        out.push(u32::try_from(index).map_err(|_| "a `path` index does not fit".to_string())?);
    }
    Ok(out)
}

/// The table a row delta addresses: the delta's node is the table itself or a
/// cell of it, resolved the same way the server resolved the operation.
fn delta_table(delta: &Value, doc: &Document, ids: &NodeIds) -> Result<(StoryRef, Path), String> {
    let node = delta.get("nodeId").and_then(Value::as_u64).ok_or_else(|| "a row delta has no `nodeId`".to_string())?;
    let node = u32::try_from(node).map_err(|_| "`nodeId` does not fit".to_string())?;
    let key = ids.resolve(NodeId(node)).map_err(|_| format!("unknown node {node}"))?;
    let story = parse_story(&key.story).ok_or_else(|| format!("unknown story {:?}", key.story))?;
    let mut path = Path(key.path.clone());
    for _ in 0..=(MAX_TABLE_DEPTH.saturating_mul(4).saturating_add(4)) {
        if doc.table(story, &path).is_some() {
            return Ok((story, path));
        }
        let Some(parent) = enclosing_table(&path) else { break };
        path = parent;
    }
    Err(format!("node {node} is not in a table"))
}

fn delta_len(delta: &Value) -> Result<usize, String> {
    usize::try_from(delta.get("len").and_then(Value::as_u64).ok_or_else(|| "delta has no `len`".to_string())?)
        .map_err(|_| "`len` does not fit".to_string())
}

fn parse_story(name: &str) -> Option<StoryRef> {
    match name {
        "body" => Some(StoryRef::Body),
        other => other.strip_prefix("part:").and_then(|n| n.parse().ok()).map(StoryRef::Part),
    }
}

fn story_name(story: StoryRef) -> String {
    match story {
        StoryRef::Body => "body".to_string(),
        StoryRef::Part(id) => format!("part:{id}"),
    }
}

fn invalid(op: &Op, message: impl Into<String>) -> Reject {
    Reject::new(op.op_id.clone(), RejectCode::Invalid, message)
}

/// An inverse operation issued by the server, not by a client.
fn inverse_op(kind: OpKind, op: &Op, story: StoryRef, node_id: u32, off: usize, payload: Value) -> Op {
    Op {
        op_id: OpId::new(format!("inverse:{}", op.op_id.client), op.op_id.seq),
        base_version: 0,
        epoch: None,
        kind,
        target: Target { story: story_name(story), node_id, off },
        payload,
        history: None,
    }
}

#[cfg(test)]
mod tests;
