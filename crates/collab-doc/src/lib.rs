//! WordCraft document adapter for the collaboration protocol.
//!
//! The room applies normalized `primitive` and `structural` operations to a
//! [`wordcraft_doc::Document`] (ADR-0001). Addresses are session node ids
//! (ADR-0002): the adapter resolves an id to the paragraph path, applies the
//! edit through the document's own edit primitives (`insert_text`,
//! `delete_range`, `split_paragraph`, `insert_block`, `remove_block`), and
//! rebases the id map after a block insertion or removal.
//!
//! Every applied operation produces an [`Effect`] with the inverse operations
//! the per-user undo stack stores (ADR-0010). Text inverses are exact
//! insert/delete pairs; structural inverses carry the removed block as JSON, so
//! undo restores it without a snapshot.
//!
//! What this adapter deliberately does not do (v1):
//! - it does not run `CommandSpec` commands (`kind: "command"` is refused);
//! - it does not touch styles, numbering, comments or track changes;
//! - it addresses top-level blocks only, not table cells or text boxes.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde_json::{Value, json};
use wordcraft_collab::ids::NodeKey;
use wordcraft_collab::ids::{NodeId, NodeIds};
use wordcraft_collab::room::{Effect, Model};
use wordcraft_collab::wire::{Op, OpId, OpKind, Reject, RejectCode, Target};
use wordcraft_doc::{Block, CharProps, Document, Paragraph, Path, Pos, StoryRef};

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
        // The new paragraph sits right after this one: shift every later block.
        let block = path.0.first().copied().unwrap_or(0) as usize;
        ids.shift_top_blocks(&story_name(story), block + 1, 1);
        // The new paragraph can be edited immediately: give it an id and tell
        // clients about the map (the server broadcasts nodes for this effect).
        ids.id_for(&NodeKey::new(story_name(story), vec![(block + 1) as u32]));
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
        let block = path.0.first().copied().unwrap_or(0) as usize;
        let para_len = self.doc.para(story, &path).map_or(0, |p| p.text.len());
        let next = Path::top(block + 1);
        let start = Pos { story, path: path.clone(), off: para_len };
        let end = Pos { story, path: next, off: 0 };
        self.doc.delete_range(&start, &end).map_err(|err| invalid(op, err.to_string()))?;
        let removed_key = NodeKey::new(story_name(story), vec![(block + 1) as u32]);
        if let Some(id) = ids.get(&removed_key) {
            ids.remove(id, op.base_version);
        }
        ids.shift_top_blocks(&story_name(story), block + 1, -1);
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
        let index = self.block_index(op, ids, false)?;
        let block = match op.payload.get("block") {
            Some(value) if !value.is_null() => {
                serde_json::from_value::<Block>(value.clone()).map_err(|err| invalid(op, format!("bad block: {err}")))?
            }
            _ => Block::Para(Paragraph::new()),
        };
        let block_json = serde_json::to_value(&block).unwrap_or(Value::Null);
        self.doc.insert_block(story, &Path::top(index), block).map_err(|err| invalid(op, err.to_string()))?;
        ids.shift_top_blocks(&story_name(story), index, 1);
        ids.id_for(&NodeKey::new(story_name(story), vec![index as u32]));
        let inverse = inverse_op(OpKind::Structural, op, story, 0, 0, json!({"op": "remove_block", "index": index}));
        Ok(Effect {
            delta: Some(json!({"op": "insert_block", "story": story_name(story), "index": index, "block": block_json})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Insert block".into(),
            nodes_changed: true,
        })
    }

    fn remove_block(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let story = parse_story(&op.target.story).ok_or_else(|| invalid(op, format!("unknown story {:?}", op.target.story)))?;
        let index = self.block_index(op, ids, true)?;
        let removed = self.doc.remove_block(story, &Path::top(index)).map_err(|err| invalid(op, err.to_string()))?;
        if op.target.node_id != 0 {
            ids.remove(NodeId(op.target.node_id), op.base_version);
        } else if let Some(id) = ids.get(&NodeKey::new(story_name(story), vec![index as u32])) {
            ids.remove(id, op.base_version);
        }
        ids.shift_top_blocks(&story_name(story), index + 1, -1);
        let block = serde_json::to_value(&removed).unwrap_or(Value::Null);
        let inverse = inverse_op(OpKind::Structural, op, story, 0, 0, json!({"op": "insert_block", "index": index, "block": block}));
        Ok(Effect {
            delta: Some(json!({"op": "remove_block", "story": story_name(story), "index": index})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Delete block".into(),
            nodes_changed: true,
        })
    }

    /// The block index an insert/remove addresses: `payload.index` when given,
    /// otherwise the position of the node (`remove`) or right after it
    /// (`insert`).
    fn block_index(&self, op: &Op, ids: &NodeIds, exact_node: bool) -> Result<usize, Reject> {
        if let Some(index) = op.payload.get("index").and_then(Value::as_u64) {
            return usize::try_from(index).map_err(|_| invalid(op, "`index` does not fit"));
        }
        let key = ids.resolve(NodeId(op.target.node_id)).map_err(|_| invalid(op, "insert/remove_block needs `index` or a known node"))?;
        let block = key.path.first().copied().unwrap_or(0) as usize;
        Ok(if exact_node { block } else { block + 1 })
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

    /// Issue ids for every paragraph of the body and of the secondary stories.
    fn index_nodes(&self, ids: &mut NodeIds) {
        for (index, block) in self.doc.body.iter().enumerate() {
            if block.as_para().is_some() {
                ids.id_for(&NodeKey::new("body", vec![index as u32]));
            }
        }
        for (part, story) in &self.doc.parts {
            let name = format!("part:{part}");
            for (index, block) in story.blocks.iter().enumerate() {
                if block.as_para().is_some() {
                    ids.id_for(&NodeKey::new(name.clone(), vec![index as u32]));
                }
            }
        }
    }
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
            let block = path.0.first().copied().unwrap_or(0) as usize;
            ids.shift_top_blocks(&story_name(story), block + 1, 1);
        }
        "merge_para" => {
            let (story, path, _) = delta_pos(delta, ids)?;
            let block = path.0.first().copied().unwrap_or(0) as usize;
            let para_len = doc.para(story, &path).map_or(0, |para| para.text.len());
            let start = Pos { story, path: path.clone(), off: para_len };
            let end = Pos { story, path: Path::top(block + 1), off: 0 };
            doc.delete_range(&start, &end).map_err(|err| err.to_string())?;
            ids.shift_top_blocks(&story_name(story), block + 1, -1);
        }
        "insert_block" => {
            let story =
                delta.get("story").and_then(Value::as_str).and_then(parse_story).ok_or_else(|| "insert_block has no known `story`".to_string())?;
            let index = usize::try_from(delta.get("index").and_then(Value::as_u64).ok_or_else(|| "insert_block has no `index`".to_string())?)
                .map_err(|_| "`index` does not fit".to_string())?;
            let block = match delta.get("block") {
                Some(value) if !value.is_null() => serde_json::from_value::<Block>(value.clone()).map_err(|err| format!("bad block: {err}"))?,
                _ => Block::Para(Paragraph::new()),
            };
            doc.insert_block(story, &Path::top(index), block).map_err(|err| err.to_string())?;
            ids.shift_top_blocks(&story_name(story), index, 1);
        }
        "remove_block" => {
            let story =
                delta.get("story").and_then(Value::as_str).and_then(parse_story).ok_or_else(|| "remove_block has no known `story`".to_string())?;
            let index = usize::try_from(delta.get("index").and_then(Value::as_u64).ok_or_else(|| "remove_block has no `index`".to_string())?)
                .map_err(|_| "`index` does not fit".to_string())?;
            doc.remove_block(story, &Path::top(index)).map_err(|err| err.to_string())?;
            ids.shift_top_blocks(&story_name(story), index + 1, -1);
        }
        "command" => return Err("a command delta needs a Session, not a bare Document".to_string()),
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

/// Turn the difference between `before` and `after` into the primitives the
/// server understands. Cloning a `Document` shares its blocks (`Arc`), so a
/// command's before-image is cheap and untouched blocks are skipped by
/// pointer. Table cells, formatting-only changes and multi-block rearrangements
/// are reported as unsupported.
pub fn diff(before: &Document, after: &Document, ids: &NodeIds) -> Result<Vec<LocalChange>, DiffError> {
    let mut changes = Vec::new();
    diff_story("body", &before.body, &after.body, ids, &mut changes)?;
    for (part, old) in &before.parts {
        let Some(new) = after.parts.get(part) else { return Err(DiffError::Unsupported(format!("story part:{part} removed"))) };
        diff_story(&format!("part:{part}"), &old.blocks, &new.blocks, ids, &mut changes)?;
    }
    for part in after.parts.keys() {
        if !before.parts.contains_key(part) {
            return Err(DiffError::Unsupported(format!("story part:{part} added")));
        }
    }
    Ok(changes)
}

fn diff_story(
    story: &str,
    old: &[std::sync::Arc<Block>],
    new: &[std::sync::Arc<Block>],
    ids: &NodeIds,
    out: &mut Vec<LocalChange>,
) -> Result<(), DiffError> {
    if old.len() == new.len() {
        for (block, (a, b)) in old.iter().zip(new).enumerate() {
            if std::sync::Arc::ptr_eq(a, b) {
                continue;
            }
            match (a.as_ref(), b.as_ref()) {
                (Block::Para(before), Block::Para(after)) => {
                    let Some((off, len, text)) = text_change(&before.text, &after.text) else {
                        return Err(DiffError::Unsupported(format!("block {block} in {story} changed without text (formatting)")));
                    };
                    let id = ids.get(&NodeKey::new(story, vec![block as u32])).ok_or(DiffError::MissingId { story: story.to_string(), block })?;
                    out.push(LocalChange {
                        kind: OpKind::Primitive,
                        target: Target::new(story, id.0, off),
                        payload: json!({"op": "replace_range", "len": len, "text": text}),
                    });
                }
                _ => return Err(DiffError::Unsupported(format!("block {block} in {story} is a table or changed kind"))),
            }
        }
        return Ok(());
    }

    if new.len() == old.len() + 1 {
        if let Some(at) = split_point(old, new) {
            let id = ids.get(&NodeKey::new(story, vec![at as u32])).ok_or(DiffError::MissingId { story: story.to_string(), block: at })?;
            let off = new[at].as_para().map(|para| para.text.len()).unwrap_or(0);
            out.push(LocalChange { kind: OpKind::Primitive, target: Target::new(story, id.0, off), payload: json!({"op": "split_para"}) });
            return Ok(());
        }
        let Some(at) = one_insertion(old, new) else {
            return Err(DiffError::Unsupported(format!("several blocks changed in {story}")));
        };
        let Some(Block::Para(_)) = new.get(at).map(|b| b.as_ref()) else {
            return Err(DiffError::Unsupported(format!("a table was inserted in {story}")));
        };
        let block = serde_json::to_value(new[at].as_ref()).map_err(|err| DiffError::Unsupported(err.to_string()))?;
        out.push(LocalChange {
            kind: OpKind::Structural,
            target: Target::new(story, 0, 0),
            payload: json!({"op": "insert_block", "index": at, "block": block}),
        });
        return Ok(());
    }

    if old.len() == new.len() + 1 {
        if let Some(at) = merge_point(old, new) {
            let id = ids.get(&NodeKey::new(story, vec![at as u32])).ok_or(DiffError::MissingId { story: story.to_string(), block: at })?;
            let off = new[at].as_para().map(|para| para.text.len()).unwrap_or(0);
            out.push(LocalChange { kind: OpKind::Primitive, target: Target::new(story, id.0, off), payload: json!({"op": "merge_para"}) });
            return Ok(());
        }
        let Some(at) = one_removal(old, new) else {
            return Err(DiffError::Unsupported(format!("several blocks changed in {story}")));
        };
        out.push(LocalChange { kind: OpKind::Structural, target: Target::new(story, 0, 0), payload: json!({"op": "remove_block", "index": at}) });
        return Ok(());
    }

    Err(DiffError::Unsupported(format!("{} blocks became {} in {story}", old.len(), new.len())))
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
