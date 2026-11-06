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
        })
    }

    fn split_para(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, path, off) = self.resolve_pos(op, ids)?;
        let pos = Pos { story, path: path.clone(), off };
        self.doc.split_paragraph(&pos).map_err(|err| invalid(op, err.to_string()))?;
        // The new paragraph sits right after this one: shift every later block.
        let block = path.0.first().copied().unwrap_or(0) as usize;
        ids.shift_top_blocks(&story_name(story), block + 1, 1);
        let inverse = inverse_op(OpKind::Primitive, op, story, op.target.node_id, off, json!({"op": "merge_para"}));
        Ok(Effect {
            delta: Some(json!({"op": "split_para", "nodeId": op.target.node_id, "off": off})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Paragraph break".into(),
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
        ids.shift_top_blocks(&story_name(story), block + 1, -1);
        let inverse = inverse_op(OpKind::Primitive, op, story, op.target.node_id, para_len, json!({"op": "split_para"}));
        Ok(Effect {
            delta: Some(json!({"op": "merge_para", "nodeId": op.target.node_id})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Join paragraphs".into(),
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
        self.doc.insert_block(story, &Path::top(index), block).map_err(|err| invalid(op, err.to_string()))?;
        ids.shift_top_blocks(&story_name(story), index, 1);
        let inverse = inverse_op(OpKind::Structural, op, story, 0, 0, json!({"op": "remove_block", "index": index}));
        Ok(Effect {
            delta: Some(json!({"op": "insert_block", "index": index})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Insert block".into(),
        })
    }

    fn remove_block(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let story = parse_story(&op.target.story).ok_or_else(|| invalid(op, format!("unknown story {:?}", op.target.story)))?;
        let index = self.block_index(op, ids, true)?;
        let removed = self.doc.remove_block(story, &Path::top(index)).map_err(|err| invalid(op, err.to_string()))?;
        ids.shift_top_blocks(&story_name(story), index + 1, -1);
        if op.target.node_id != 0 {
            ids.remove(NodeId(op.target.node_id), op.base_version);
        }
        let block = serde_json::to_value(&removed).unwrap_or(Value::Null);
        let inverse = inverse_op(OpKind::Structural, op, story, 0, 0, json!({"op": "insert_block", "index": index, "block": block}));
        Ok(Effect {
            delta: Some(json!({"op": "remove_block", "index": index})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Delete block".into(),
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
    }
}

#[cfg(test)]
mod tests;
