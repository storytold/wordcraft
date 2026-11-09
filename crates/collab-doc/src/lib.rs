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
//! insert/delete pairs; structural inverses carry the removed block, row or cell
//! as JSON, so undo restores it without a snapshot.
//!
//! Structural table edits keep the ids of the cells they do not move: a column
//! insert shifts the cell ids of the rows that gained a cell and leaves the ones
//! that widened alone (`insert_column`/`remove_column`), a merge keeps the id of
//! the cell that absorbs the range (`merge_cells`), and a split keeps the id of
//! the cell it cuts (`split_cells`). A range that cannot be addressed cell by
//! cell is refused before the document is touched.
//!
//! What this adapter deliberately does not do (v1):
//! - it does not run `CommandSpec` commands (`kind: "command"` is refused);
//! - it does not touch styles, numbering, comments or track changes;
//! - it does not normalize formatting or several rearranged blocks; the
//!   forward effect of a formatting command is still the command itself, and
//!   only its inverse is a primitive (`set_para_format`, see below).
//!
//! `move_block` swaps a run of blocks with the block next to it, addressed by
//! the run's first session id, a block count (at most [`MAX_MOVE_BLOCKS`]) and
//! the neighbour's session id. The operation is its own inverse, every id below
//! the container follows the block it names, and a step whose run is wider than
//! the bound is not a move the adapter normalizes: the client's diff reports it
//! as unsupported and the engine command path keeps it as a command step.
//!
//! `set_para_format` restores one paragraph's body and formatting to a named
//! earlier value: it is the inverse of a character or paragraph command, whose
//! forward effect a replica replays as a command (a toggle or a relative step
//! cannot be replayed backwards). The payload names only the fields the command
//! changed. A formatting-only restore carries `runs`/`mark`/`props`; a command
//! that rewrites characters (`format.changeCase`) or replaces them with an
//! enclosed-character field (`format.enclose`) also carries the paragraph's
//! `text` and `objects`, with `expect` naming the text the paragraph holds
//! after the command. The address must be a paragraph, a run list must cover the
//! target text on character boundaries, and `expect` must match the current
//! text, or the operation is refused before the document is touched.
//!
//! A column removal, a merge and a split carry an exact inverse: the removed
//! bodies, the original cell boundaries, the grid and the session ids of the
//! cells they took away travel in the inverse operation, so undo restores the
//! table instead of reporting the step as unsupported. The inverse refuses to
//! run when the table no longer has the shape the forward operation left
//! (`undo-conflict`), and a step reports the ids whose content its inverse would
//! overwrite, so an edit made after it is never silently dropped (ADR-0010).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_collab::ids::NodeKey;
use wordcraft_collab::ids::{NodeId, NodeIds};
use wordcraft_collab::room::{Effect, Model};
use wordcraft_collab::wire::{Op, OpId, OpKind, Reject, RejectCode, Target};
use wordcraft_doc::props::CellProps;
use wordcraft_doc::props::VMerge;
use wordcraft_doc::table::MAX_COLS;
use wordcraft_doc::table::MAX_ROWS;
use wordcraft_doc::table::MAX_TABLE_DEPTH;
use wordcraft_doc::{Block, Blocks, Cell, CharProps, Document, InlineObject, ParaProps, Paragraph, Path, Pos, Row, Run, StoryRef, Table};

/// The most runs one `set_para_format` restores. A longer paragraph is refused
/// before the document changes: a hostile payload cannot make the adapter walk
/// an unbounded list.
pub const MAX_FORMAT_RUNS: usize = 100_000;

/// The most blocks one adjacent move carries. A wider run is not a move the
/// adapter captures: the client's diff reports the step as unsupported (it
/// travels as the command), and the command's undo step stays a barrier. The
/// bound also keeps an inverse small: undoing a move of `count` blocks is
/// `count` one-block moves in one transaction.
pub const MAX_MOVE_BLOCKS: usize = 64;

/// Payload key that names the primitive (`insert_text`, `delete_range`, …).
pub const PAYLOAD_OP: &str = "op";

/// Why an adjacent-block move cannot be addressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveError {
    /// The id was never issued in this session.
    Unknown(u32),
    /// The block was deleted at this version.
    Gone(u32, u64),
    /// The addresses exist but do not describe one adjacent move.
    Bad(String),
}

impl std::fmt::Display for MoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MoveError::Unknown(id) => write!(f, "unknown node {id}"),
            MoveError::Gone(id, version) => write!(f, "node {id} was deleted at version {version}"),
            MoveError::Bad(why) => write!(f, "{why}"),
        }
    }
}

/// One adjacent-block move, resolved against a document and the session map:
/// the `count` blocks of the run that starts at `run` and the block at
/// `neighbour` swap places, so the run ends up above the neighbour (`up`) or
/// below it. Both addresses are node ids, so the same plan names the same
/// content on every replica.
#[derive(Clone, Debug, PartialEq)]
pub struct MovePlan {
    pub story: StoryRef,
    pub story_name: String,
    pub container: Vec<u32>,
    pub run: usize,
    pub count: usize,
    /// The run's ids, in document order (the inverse moves the neighbour back
    /// past each of them).
    pub run_ids: Vec<u32>,
    pub neighbour: u32,
    pub neighbour_index: usize,
    pub up: bool,
}

/// Resolve `run_id` (the first block of the run), `count` and `neighbour_id`
/// into one adjacent move, or say why the addresses do not describe one.
///
/// Every check runs before the document or the id map is touched: a run that
/// leaves the container, a block that is not a paragraph (a table, or a
/// paragraph of a section break, cannot be moved as the command refuses them
/// too), a neighbour in another container or not directly beside the run, and a
/// `count` outside `1..=MAX_MOVE_BLOCKS` are all refused here.
pub fn plan_move(doc: &Document, ids: &NodeIds, run_id: u32, count: usize, neighbour_id: u32) -> Result<MovePlan, MoveError> {
    if count == 0 || count > MAX_MOVE_BLOCKS {
        return Err(MoveError::Bad(format!("a move carries 1 to {MAX_MOVE_BLOCKS} blocks, not {count}")));
    }
    let run_key = ids.resolve(NodeId(run_id)).map_err(|why| match why {
        wordcraft_collab::ids::Resolve::Unknown => MoveError::Unknown(run_id),
        wordcraft_collab::ids::Resolve::Gone(version) => MoveError::Gone(run_id, version),
    })?;
    let story_name = run_key.story.clone();
    let run_path = run_key.path.clone();
    if run_path.is_empty() {
        return Err(MoveError::Bad(format!("node {run_id} is not a block")));
    }
    let story = parse_story(&story_name).ok_or_else(|| MoveError::Bad(format!("unknown story {story_name:?}")))?;
    let container = run_path.get(..run_path.len().saturating_sub(1)).unwrap_or_default().to_vec();
    let run = run_path.last().copied().unwrap_or(0) as usize;
    let blocks = doc.container(story, &Path(run_path.clone())).ok_or_else(|| MoveError::Bad(format!("node {run_id} is not in a block list")))?;
    let len = blocks.len();
    if run.saturating_add(count) > len {
        return Err(MoveError::Bad("the moved blocks run past the container".to_string()));
    }
    for index in run..run.saturating_add(count) {
        match blocks.get(index).map(|block| &**block) {
            Some(Block::Para(para)) if para.section.is_none() => {}
            Some(_) => return Err(MoveError::Bad("only paragraphs that do not carry a section break can be moved".to_string())),
            None => return Err(MoveError::Bad("the moved blocks run past the container".to_string())),
        }
    }
    let n_key = ids.resolve(NodeId(neighbour_id)).map_err(|why| match why {
        wordcraft_collab::ids::Resolve::Unknown => MoveError::Unknown(neighbour_id),
        wordcraft_collab::ids::Resolve::Gone(version) => MoveError::Gone(neighbour_id, version),
    })?;
    if n_key.story != story_name
        || n_key.path.len() != run_path.len()
        || n_key.path.get(..container.len()).unwrap_or_default() != container.as_slice()
    {
        return Err(MoveError::Bad("the neighbour is in another container".to_string()));
    }
    if neighbour_id == run_id {
        return Err(MoveError::Bad("a block cannot be moved past itself".to_string()));
    }
    let neighbour_index = n_key.path.last().copied().unwrap_or(0) as usize;
    let up = if neighbour_index.saturating_add(1) == run {
        true
    } else if neighbour_index == run.saturating_add(count) {
        false
    } else {
        return Err(MoveError::Bad("the neighbour is not next to the moved blocks".to_string()));
    };
    match blocks.get(neighbour_index).map(|block| &**block) {
        Some(Block::Para(_)) => {}
        Some(_) => return Err(MoveError::Bad("a table is in the way".to_string())),
        None => return Err(MoveError::Bad("the neighbour does not exist".to_string())),
    }
    let mut run_ids = Vec::with_capacity(count);
    for index in run..run.saturating_add(count) {
        let key = NodeKey::new(story_name.clone(), child_path(&container, index).0);
        let id = ids.get(&key).ok_or_else(|| MoveError::Bad(format!("a moved block has no session id: {story_name:?} {:?}", key.path)))?;
        run_ids.push(id.0);
    }
    Ok(MovePlan { story, story_name, container, run, count, run_ids, neighbour: neighbour_id, neighbour_index, up })
}

/// Apply a resolved move: the run and its neighbour swap places in their
/// container, and every session id below it follows the block it names.
pub fn apply_move(doc: &mut Document, ids: &mut NodeIds, plan: &MovePlan) -> Result<(), String> {
    let path = child_path(&plan.container, plan.run);
    let blocks = doc.container_mut(plan.story, &path).map_err(|err| err.to_string())?;
    let end = plan.run.saturating_add(plan.count);
    if plan.up {
        let start = plan.run.saturating_sub(1);
        let window = blocks.get_mut(start..end).ok_or_else(|| "the move range is gone".to_string())?;
        window.rotate_left(1);
    } else {
        let window = blocks.get_mut(plan.run..=end).ok_or_else(|| "the move range is gone".to_string())?;
        window.rotate_right(1);
    }
    rebase_ids_after_move(ids, plan);
    Ok(())
}

/// Move every session id of the affected container to the block it names after
/// the swap. The subtree follows: a moved table keeps the ids of its rows, cells
/// and the paragraphs inside them.
pub fn rebase_ids_after_move(ids: &mut NodeIds, plan: &MovePlan) {
    let run = plan.run as u32;
    let count = plan.count as u32;
    let neighbour = plan.neighbour_index as u32;
    ids.permute_children(&plan.story_name, &plan.container, |index| {
        if plan.up {
            if index == neighbour {
                neighbour.saturating_add(count)
            } else if index > neighbour && index <= neighbour.saturating_add(count) {
                index.saturating_sub(1)
            } else {
                index
            }
        } else if index == neighbour {
            run
        } else if index >= run && index < neighbour {
            index.saturating_add(1)
        } else {
            index
        }
    });
}

/// The ids whose position the move changes: every block of the run and the
/// neighbour with their subtrees. An edit another user makes in one of them
/// after the move makes undo a conflict instead of moving the block back on top
/// of it.
pub fn move_touched(ids: &NodeIds, plan: &MovePlan) -> Vec<u32> {
    let mut touched = Vec::new();
    for index in plan.run..plan.run.saturating_add(plan.count) {
        touched.extend(live_ids(ids, &plan.story_name, &[], &child_path(&plan.container, index).0));
    }
    touched.extend(live_ids(ids, &plan.story_name, &[], &child_path(&plan.container, plan.neighbour_index).0));
    touched.sort_unstable();
    touched.dedup();
    touched
}

/// The operations that undo a move: the neighbour crosses the run one block at
/// a time, in the opposite direction, so each inverse is again one adjacent
/// move with explicit addresses. Several of them land as one `transaction`
/// delta (the room wraps a step's deltas), and a one-block move is a single
/// `move_block`.
pub fn move_inverse_ops(op: &Op, plan: &MovePlan) -> Vec<Op> {
    let mut out = Vec::with_capacity(plan.count);
    for step in 0..plan.count {
        let neighbour = if plan.up { plan.run_ids.get(plan.count - 1 - step) } else { plan.run_ids.get(step) };
        let Some(neighbour) = neighbour else { continue };
        out.push(inverse_op(OpKind::Structural, op, plan.story, plan.neighbour, 0, json!({"op": "move_block", "count": 1, "neighbour": neighbour})));
    }
    out
}

/// The authoritative delta of a move; the inverse carries the same key names.
pub fn move_delta(run_id: u32, count: usize, neighbour_id: u32) -> Value {
    json!({"op": "move_block", "nodeId": run_id, "count": count, "neighbour": neighbour_id})
}

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
            "set_para_format" => self.set_para_format(op, ids),
            other => Err(invalid(op, format!("unknown primitive {other:?}"))),
        }
    }

    fn structural(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let name = op.payload.get(PAYLOAD_OP).and_then(Value::as_str).unwrap_or_default();
        match name {
            "insert_block" => self.insert_block(op, ids),
            "remove_block" => self.remove_block(op, ids),
            "move_block" => self.move_block(op, ids),
            "insert_row" => self.insert_row(op, ids),
            "remove_row" => self.remove_row(op, ids),
            "insert_column" => self.insert_column(op, ids),
            "remove_column" => self.remove_column(op, ids),
            "merge_cells" => self.merge_cells(op, ids),
            "split_cells" => self.split_cells(op, ids),
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
            touched: Vec::new(),
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
            touched: Vec::new(),
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
            touched: Vec::new(),
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
            touched: Vec::new(),
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
            touched: Vec::new(),
        })
    }

    /// Restore the character and paragraph formatting of one paragraph to an
    /// exact earlier value: the inverse of a formatting command (ADR-0010).
    ///
    /// The command itself travels as `kind:"command"` and every replica replays
    /// it, but no command can be replayed backwards: a toggle would flip again
    /// and a relative step (grow font) would move twice. The inverse is
    /// therefore a primitive that names the previous value byte for byte. The
    /// payload may carry the paragraph's `runs`, its `mark` (the formatting of
    /// the paragraph mark) and its paragraph `props`; a field that is absent is
    /// left as it is, so the inverse carries only what the command changed.
    ///
    /// A command that rewrites characters (`format.changeCase`) or replaces
    /// them with an enclosed-character field (`format.enclose`) cannot be undone
    /// by formatting alone: its inverse also carries `text` and `objects`, and
    /// `expect` names the text the paragraph holds after the command. Without
    /// that body a restore would put the old formatting over different text.
    ///
    /// The address must name a paragraph. Anything else — a table, a row, a
    /// cell, an unknown or deleted node — is refused before the document is
    /// touched, and so is a run list that does not cover the target text on
    /// character boundaries, a body restore whose `expect` does not match, and
    /// inline objects that do not match the text's object characters.
    fn set_para_format(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, path, _) = self.resolve_pos(op, ids)?;
        let Paragraph { text, runs, mark, props, objects, .. } =
            self.doc.para(story, &path).cloned().ok_or_else(|| invalid(op, format!("node {} does not name a paragraph", op.target.node_id)))?;
        let patch = format_patch(&op.payload, &text).map_err(|why| invalid(op, why))?;
        let inverse_payload = json!({
            "op": "set_para_format",
            "text": patch.text.as_ref().map(|_| json!(text)),
            "objects": patch.objects.as_ref().map(|_| json!(objects)),
            "expect": patch.text.as_ref().map(|new| json!(new)),
            "runs": patch.runs.as_ref().map(|_| json!(runs)),
            "mark": patch.mark.as_ref().map(|_| json!(mark)),
            "props": patch.props.as_ref().map(|_| json!(props)),
        });
        let delta = json!({
            "op": "set_para_format",
            "nodeId": op.target.node_id,
            "text": patch.text.as_ref().map(|text| json!(text)),
            "objects": patch.objects.as_ref().map(|objects| json!(objects)),
            "expect": patch.text.as_ref().map(|_| json!(text)),
            "runs": patch.runs.as_ref().map(|runs| json!(runs)),
            "mark": patch.mark.as_ref().map(|mark| json!(mark)),
            "props": patch.props.as_ref().map(|props| json!(props)),
        });
        let para = self.doc.para_mut(story, &path).map_err(|err| invalid(op, err.to_string()))?;
        apply_format_patch(para, patch);
        let inverse = inverse_op(OpKind::Primitive, op, story, op.target.node_id, 0, inverse_payload);
        Ok(Effect {
            delta: Some(delta),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Formatting".into(),
            nodes_changed: false,
            touched: Vec::new(),
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
        // Undoing the insert removes the block again, so an edit another user
        // made inside it after the insert must block the undo rather than be
        // dropped with the block.
        let touched = live_ids(ids, &name, &[], &target.0);
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
            touched,
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
            touched: Vec::new(),
        })
    }

    /// Move `payload.count` blocks starting at the target past the block named
    /// by `payload.neighbour`. Both addresses are session ids, so every replica
    /// names the same content and recomputes the same plan from the same
    /// document; the operation is its own inverse (the neighbour crosses the run
    /// instead).
    ///
    /// An address that does not describe one adjacent move — an unknown or
    /// deleted id, a neighbour in another container or not beside the run, a
    /// non-paragraph in the way, a run that leaves the container or carries more
    /// than [`MAX_MOVE_BLOCKS`] blocks — is refused before the document or the
    /// id map changes.
    fn move_block(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let count = match op.payload.get("count") {
            Some(value) => usize::try_from(value.as_u64().ok_or_else(|| invalid(op, "`count` must be a number"))?)
                .map_err(|_| invalid(op, "`count` does not fit"))?,
            None => 1,
        };
        let neighbour = op.payload.get("neighbour").and_then(Value::as_u64).ok_or_else(|| invalid(op, "move_block needs `neighbour`"))?;
        let neighbour = u32::try_from(neighbour).map_err(|_| invalid(op, "`neighbour` does not fit"))?;
        let plan = plan_move(&self.doc, ids, op.target.node_id, count, neighbour).map_err(|err| match err {
            MoveError::Unknown(id) => invalid(op, format!("unknown node {id}")),
            MoveError::Gone(id, version) => Reject::new(op.op_id.clone(), RejectCode::Gone, format!("node {id} was deleted at version {version}")),
            MoveError::Bad(why) => invalid(op, why),
        })?;
        // Undo swaps the same two sides back, so an edit another user makes in
        // any block the move displaced must refuse the undo.
        let touched = move_touched(ids, &plan);
        apply_move(&mut self.doc, ids, &plan).map_err(|err| invalid(op, err))?;
        Ok(Effect {
            delta: Some(move_delta(op.target.node_id, plan.count, plan.neighbour)),
            inverse: move_inverse_ops(op, &plan),
            forward: vec![op.clone()],
            label: "Move paragraphs".into(),
            nodes_changed: true,
            touched,
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
        // Undo removes the row again: text another user put into it after the
        // insert must make the undo a conflict, not vanish with the row.
        let mut row_path = table_path.0.clone();
        row_path.push(at as u32);
        let touched = live_ids(ids, &name, &table_path.0, &row_path);
        Ok(Effect {
            delta: Some(json!({"op": "insert_row", "nodeId": table_node.0, "index": at, "row": row_json})),
            inverse: vec![inverse],
            forward: vec![op.clone()],
            label: "Insert row".into(),
            nodes_changed: true,
            touched,
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
            touched: Vec::new(),
        })
    }

    /// Insert a grid column at grid index `index` into the table the operation
    /// addresses. The rows whose cells do not span the new column get one empty
    /// cell and their later cell ids move one place right; a row whose cell spans
    /// the column only widens it and keeps every id.
    fn insert_column(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, table_path) = self.table_path(op, ids)?;
        let index = self.column_index(op)?;
        let name = story_name(story);
        let table_node = ids.id_for(&NodeKey::new(name.clone(), table_path.0.clone()));
        if let Some(restore) = op.payload.get("restore").filter(|value| !value.is_null()) {
            restore_insert_column(&mut self.doc, ids, story, &table_path, index, restore).map_err(|err| err.reject(op))?;
            return Ok(Effect {
                delta: Some(json!({"op": "insert_column", "nodeId": table_node.0, "index": index, "restore": restore})),
                inverse: Vec::new(),
                forward: vec![op.clone()],
                label: "Undo column removal".into(),
                nodes_changed: true,
                touched: Vec::new(),
            });
        }
        let table = self.doc.table(story, &table_path).ok_or_else(|| invalid(op, "the node is not a table"))?;
        if table.grid.len() >= MAX_COLS {
            return Err(invalid(op, "the table is full"));
        }
        if index > table.grid.len() {
            return Err(invalid(op, format!("column {index} is past the grid")));
        }
        let before_columns = table.grid.len();
        let plan = column_insert_plan(table, index);
        insert_column_at(&mut self.doc, ids, story, &table_path, index).map_err(|err| invalid(op, err))?;
        // The cells the insert created are what its inverse (the same column
        // taken out again) would delete: typing into one of them after the
        // insert makes the undo `undo-conflict` instead of losing that text.
        let mut touched = Vec::new();
        for (r, at) in plan.into_iter().enumerate() {
            let Some(at) = at else { continue };
            let mut cell = table_path.0.clone();
            cell.push(r as u32);
            cell.push(at as u32);
            touched.extend(live_ids(ids, &name, &table_path.0, &cell));
        }
        // Undo of an insert is the same column taken out again; it is exact
        // because nothing existed there before. A one-column table cannot lose
        // its column, so such an insert carries no inverse.
        let inverse = if before_columns >= 2 {
            vec![inverse_op(OpKind::Structural, op, story, table_node.0, 0, json!({"op": "remove_column", "index": index}))]
        } else {
            Vec::new()
        };
        Ok(Effect {
            delta: Some(json!({"op": "insert_column", "nodeId": table_node.0, "index": index})),
            inverse,
            forward: vec![op.clone()],
            label: "Insert column".into(),
            nodes_changed: true,
            touched,
        })
    }

    /// Remove grid column `index`: a cell that spans it shrinks and keeps its id,
    /// a cell that only covered it is tombstoned, and the cells to its right
    /// move one place left.
    fn remove_column(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, table_path) = self.table_path(op, ids)?;
        let index = self.column_index(op)?;
        let name = story_name(story);
        let table_node = ids.id_for(&NodeKey::new(name.clone(), table_path.0.clone()));
        // The bodies the column takes away travel in the inverse, with their
        // session ids and the grid width, so undo puts the column back with its
        // content instead of reporting the step as unsupported.
        let restore = remove_column_restore(&self.doc, ids, story, &table_path, index).map_err(|err| invalid(op, err))?;
        remove_column_at(&mut self.doc, ids, story, &table_path, index, op.base_version).map_err(|err| invalid(op, err))?;
        let table = self.doc.table(story, &table_path).ok_or_else(|| invalid(op, "the table vanished"))?;
        let inverse = restore.map(|mut restore| {
            restore["shape"] = json!(table_shape(table));
            inverse_op(OpKind::Structural, op, story, table_node.0, 0, json!({"op": "insert_column", "index": index, "restore": restore}))
        });
        Ok(Effect {
            delta: Some(json!({"op": "remove_column", "nodeId": table_node.0, "index": index})),
            inverse: inverse.into_iter().collect(),
            forward: vec![op.clone()],
            label: "Delete column".into(),
            nodes_changed: true,
            touched: Vec::new(),
        })
    }

    /// Merge the cells covering `payload.rows` (`[first, last]`) and
    /// `payload.cols` (`[first, last]`), grid indexes both. The first cell of
    /// each merged row keeps its id; the cells it absorbs are tombstoned, and the
    /// content of the continued rows moves into the first row's cell.
    fn merge_cells(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, table_path) = self.table_path(op, ids)?;
        let name = story_name(story);
        let table_node = ids.id_for(&NodeKey::new(name.clone(), table_path.0.clone()));
        if let Some(restore) = op.payload.get("restore").filter(|value| !value.is_null()) {
            restore_merge(&mut self.doc, ids, story, &table_path, restore, op.base_version).map_err(|err| err.reject(op))?;
            return Ok(Effect {
                delta: Some(json!({"op": "merge_cells", "nodeId": table_node.0, "restore": restore})),
                inverse: Vec::new(),
                forward: vec![op.clone()],
                label: "Undo split".into(),
                nodes_changed: true,
                touched: Vec::new(),
            });
        }
        let (r0, r1, g0, g1) = self.merge_range(op)?;
        let plan = {
            let table = self.doc.table(story, &table_path).ok_or_else(|| invalid(op, "the node is not a table"))?;
            merge_plan(table, (r0, r1, g0, g1)).map_err(|err| invalid(op, err))?
        };
        let restore = merge_restore(&self.doc, ids, story, &table_path, r0, &plan).map_err(|err| invalid(op, err))?;
        merge_cells_at(&mut self.doc, ids, story, &table_path, (r0, r1, g0, g1), op.base_version).map_err(|err| invalid(op, err))?;
        let table = self.doc.table(story, &table_path).ok_or_else(|| invalid(op, "the table vanished"))?;
        // The merged cells and their blocks are what the inverse rewrites: an
        // edit inside the merged cell after the merge makes undo a conflict.
        let mut touched = Vec::new();
        for (i, (at, _)) in plan.iter().enumerate() {
            let mut cell = table_path.0.clone();
            cell.push((r0 + i) as u32);
            cell.push(*at as u32);
            touched.extend(live_ids(ids, &name, &table_path.0, &cell));
        }
        let inverse = restore.map(|mut restore| {
            restore["shape"] = json!(table_shape(table));
            inverse_op(OpKind::Structural, op, story, table_node.0, 0, json!({"op": "split_cells", "restore": restore}))
        });
        Ok(Effect {
            delta: Some(json!({"op": "merge_cells", "nodeId": table_node.0, "rows": [r0, r1], "cols": [g0, g1]})),
            inverse: inverse.into_iter().collect(),
            forward: vec![op.clone()],
            label: "Merge cells".into(),
            nodes_changed: true,
            touched,
        })
    }

    /// Split cell `payload.cell` of row `payload.row` into `payload.columns`
    /// cells. The cell keeps its id and its content; the parts to its right are
    /// new cells, and the cells after it move right by that many.
    ///
    /// With `payload.restore` the operation is an inverse instead: it puts the
    /// cells a merge absorbed back with their original boundaries, grid and ids.
    fn split_cells(&mut self, op: &Op, ids: &mut NodeIds) -> Result<Effect, Reject> {
        let (story, table_path) = self.table_path(op, ids)?;
        let name = story_name(story);
        let table_node = ids.id_for(&NodeKey::new(name.clone(), table_path.0.clone()));
        if let Some(restore) = op.payload.get("restore").filter(|value| !value.is_null()) {
            restore_split(&mut self.doc, ids, story, &table_path, restore, op.base_version).map_err(|err| err.reject(op))?;
            return Ok(Effect {
                delta: Some(json!({"op": "split_cells", "nodeId": table_node.0, "restore": restore})),
                inverse: Vec::new(),
                forward: vec![op.clone()],
                label: "Undo merge".into(),
                nodes_changed: true,
                touched: Vec::new(),
            });
        }
        let row = self.small_index(op, "row")?;
        let cell = self.small_index(op, "cell")?;
        let columns = self.small_index(op, "columns")?;
        let (added, restore) = split_restore(&self.doc, story, &table_path, row, cell, columns).map_err(|err| invalid(op, err))?;
        split_cells_at(&mut self.doc, ids, story, &table_path, row, cell, columns).map_err(|err| invalid(op, err))?;
        let table = self.doc.table(story, &table_path).ok_or_else(|| invalid(op, "the table vanished"))?;
        // The cell and the parts cut from it are what the inverse merges back:
        // text typed into either after the split makes undo a conflict.
        let mut touched = Vec::new();
        for k in 0..=added {
            let mut path = table_path.0.clone();
            path.push(row as u32);
            path.push((cell + k) as u32);
            touched.extend(live_ids(ids, &name, &table_path.0, &path));
        }
        let inverse = restore.map(|mut restore| {
            restore["shape"] = json!(table_shape(table));
            inverse_op(OpKind::Structural, op, story, table_node.0, 0, json!({"op": "merge_cells", "restore": restore}))
        });
        Ok(Effect {
            delta: Some(json!({"op": "split_cells", "nodeId": table_node.0, "row": row, "cell": cell, "columns": columns})),
            inverse: inverse.into_iter().collect(),
            forward: vec![op.clone()],
            label: "Split cells".into(),
            nodes_changed: true,
            touched,
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

    fn column_index(&self, op: &Op) -> Result<usize, Reject> {
        let index = op.payload.get("index").and_then(Value::as_u64).ok_or_else(|| invalid(op, "a column operation needs `index`"))?;
        usize::try_from(index).map_err(|_| invalid(op, "`index` does not fit"))
    }

    /// A small row, cell or column-count field; bounded so a hostile payload
    /// cannot ask for an allocation.
    fn small_index(&self, op: &Op, key: &str) -> Result<usize, Reject> {
        let value = op.payload.get(key).and_then(Value::as_u64).ok_or_else(|| invalid(op, format!("a cell operation needs `{key}`")))?;
        let value = usize::try_from(value).map_err(|_| invalid(op, format!("`{key}` does not fit")))?;
        if value > MAX_ROWS {
            return Err(invalid(op, format!("`{key}` is out of range")));
        }
        Ok(value)
    }

    /// `payload.rows` and `payload.cols`, each `[first, last]`.
    fn merge_range(&self, op: &Op) -> Result<(usize, usize, usize, usize), Reject> {
        let pair = |key: &str| -> Result<(usize, usize), Reject> {
            let items =
                op.payload.get(key).and_then(Value::as_array).ok_or_else(|| invalid(op, format!("a merge needs `{key}` as [first, last]")))?;
            if items.len() != 2 {
                return Err(invalid(op, format!("`{key}` must hold two indexes")));
            }
            let first = items.first().and_then(Value::as_u64).ok_or_else(|| invalid(op, format!("`{key}` must hold two indexes")))?;
            let last = items.get(1).and_then(Value::as_u64).ok_or_else(|| invalid(op, format!("`{key}` must hold two indexes")))?;
            Ok((
                usize::try_from(first).map_err(|_| invalid(op, "an index does not fit"))?,
                usize::try_from(last).map_err(|_| invalid(op, "an index does not fit"))?,
            ))
        };
        let (r0, r1) = pair("rows")?;
        let (g0, g1) = pair("cols")?;
        Ok((r0, r1, g0, g1))
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

/// What inserting grid column `g` does to one row's cell list: `Some(index)` is
/// the cell a new empty cell appears in front of, `None` says a cell spanning the
/// column widens instead. It mirrors `Table::insert_col`, which cannot report
/// what it did; a test pins the two together.
fn column_insert_plan(table: &Table, g: usize) -> Vec<Option<usize>> {
    table
        .rows
        .iter()
        .map(|row| {
            let mut x = 0usize;
            for (i, cell) in row.cells.iter().enumerate() {
                if g <= x {
                    return Some(i);
                }
                if g < x.saturating_add(cell.span()) {
                    return None;
                }
                x = x.saturating_add(cell.span());
            }
            Some(row.cells.len())
        })
        .collect()
}

/// What deleting grid column `g` does to one row's cell list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColumnRemoval {
    /// The row does not reach the column.
    Kept,
    /// A cell spanning the column shrinks and keeps its id.
    Shrunk(usize),
    /// The cell at this index disappears.
    Removed(usize),
}

fn column_removal_plan(table: &Table, g: usize) -> Vec<ColumnRemoval> {
    table
        .rows
        .iter()
        .map(|row| {
            let mut x = 0usize;
            for (i, cell) in row.cells.iter().enumerate() {
                if g < x.saturating_add(cell.span()) {
                    return if cell.span() > 1 { ColumnRemoval::Shrunk(i) } else { ColumnRemoval::Removed(i) };
                }
                x = x.saturating_add(cell.span());
            }
            ColumnRemoval::Kept
        })
        .collect()
}

/// Insert grid column `g`: the document's own `Table::insert_col` does the edit,
/// and the plan above says which rows gained a cell. Every cell of such a row at
/// or after the new index moves one place right with its whole subtree, so a
/// client editing one of them keeps its id; the widened cells keep theirs too.
fn insert_column_at(doc: &mut Document, ids: &mut NodeIds, story: StoryRef, table_path: &Path, g: usize) -> Result<(), String> {
    let name = story_name(story);
    let table = doc.table(story, table_path).ok_or_else(|| "the target is not a table".to_string())?;
    if table.grid.len() >= MAX_COLS {
        return Err("the table is full".to_string());
    }
    if g > table.grid.len() {
        return Err(format!("column {g} is past the grid"));
    }
    let plan = column_insert_plan(table, g);
    doc.table_mut(story, table_path).map_err(|err| err.to_string())?.insert_col(g);
    let table = doc.table(story, table_path).ok_or_else(|| "the table vanished".to_string())?;
    for (r, at) in plan.into_iter().enumerate() {
        let Some(at) = at else { continue };
        let mut container = table_path.0.clone();
        container.push(r as u32);
        ids.shift_children(&name, &container, at, 1);
        let Some(cell) = table.rows.get(r).and_then(|row| row.cells.get(at)) else { continue };
        let mut path = container;
        path.push(at as u32);
        ids.id_for(&NodeKey::new(name.clone(), path.clone()));
        index_blocks(ids, &name, &mut path, &cell.blocks, 0);
    }
    Ok(())
}

/// Remove grid column `g`. The rows that lose a cell tombstone it and move their
/// later cells left; the cells it does not touch keep their ids. A row that held
/// only that cell disappears from the table, and the rows below move up.
fn remove_column_at(doc: &mut Document, ids: &mut NodeIds, story: StoryRef, table_path: &Path, g: usize, at: u64) -> Result<(), String> {
    let name = story_name(story);
    let (plan, dropped) = {
        let table = doc.table(story, table_path).ok_or_else(|| "the target is not a table".to_string())?;
        if table.grid.len() <= 1 {
            return Err("the table has one column".to_string());
        }
        if g >= table.grid.len() {
            return Err(format!("column {g} does not exist"));
        }
        let plan = column_removal_plan(table, g);
        let dropped: Vec<usize> = plan
            .iter()
            .enumerate()
            .filter(|(r, step)| matches!(step, ColumnRemoval::Removed(0)) && table.rows.get(*r).is_some_and(|row| row.cells.len() == 1))
            .map(|(r, _)| r)
            .collect();
        (plan, dropped)
    };
    doc.table_mut(story, table_path).map_err(|err| err.to_string())?.delete_col(g);
    for (r, step) in plan.iter().enumerate() {
        if dropped.contains(&r) {
            continue;
        }
        let ColumnRemoval::Removed(c) = step else { continue };
        let mut container = table_path.0.clone();
        container.push(r as u32);
        let mut gone = container.clone();
        gone.push(*c as u32);
        ids.remove_subtree(&name, &gone, at);
        ids.shift_children(&name, &container, c + 1, -1);
    }
    let mut removed_before = 0usize;
    for r in dropped {
        let at_row = r.saturating_sub(removed_before);
        let mut row_path = table_path.0.clone();
        row_path.push(at_row as u32);
        ids.remove_subtree(&name, &row_path, at);
        ids.shift_children(&name, &table_path.0, at_row, -1);
        removed_before = removed_before.saturating_add(1);
    }
    Ok(())
}

/// The cell index of the cell covering grid column `g` in row `r`, with the row
/// reachable at all.
fn cell_at_grid(table: &Table, r: usize, g: usize) -> Option<usize> {
    table.cell_at_grid(r, g)
}

/// The vertical-merge state of the cell that covers grid column `g` in row `r`.
fn vmerge_at(table: &Table, r: usize, g: usize) -> Option<VMerge> {
    table.cell_at_grid(r, g).and_then(|c| table.rows.get(r)?.cells.get(c)).map(|cell| cell.props.vmerge)
}

/// The row and cell indexes a merge of `range` takes on: per row, the first cell
/// (`a`) and the last one (`b`) the range covers. Every refusal happens here,
/// before the document is touched.
fn merge_plan(table: &Table, range: (usize, usize, usize, usize)) -> Result<Vec<(usize, usize)>, String> {
    let (r0, r1, g0, g1) = range;
    if r0 > r1 || g0 > g1 {
        return Err("the range is inverted".to_string());
    }
    if r1 >= table.rows.len() {
        return Err(format!("row {r1} does not exist"));
    }
    if g1 >= table.grid.len() {
        return Err(format!("column {g1} does not exist"));
    }
    let mut plan = Vec::with_capacity(r1.saturating_sub(r0).saturating_add(1));
    for r in r0..=r1 {
        let (Some(a), Some(b)) = (cell_at_grid(table, r, g0), cell_at_grid(table, r, g1)) else {
            return Err(format!("row {r} has no cell at the range's edge"));
        };
        plan.push((a, b));
    }
    if vmerge_at(table, r0, g0) == Some(VMerge::Continue) {
        return Err("the range starts inside a vertical merge".to_string());
    }
    if r1.saturating_add(1) < table.rows.len() && vmerge_at(table, r1 + 1, g0) == Some(VMerge::Continue) {
        return Err("the range ends inside a vertical merge".to_string());
    }
    if r0 == r1 && plan.first().is_some_and(|(a, b)| a == b) {
        return Err("the range is one cell".to_string());
    }
    Ok(plan)
}

/// Merge the rectangle `rows r0..=r1 × grid columns g0..=g1` with `Table::merge`,
/// which keeps the first cell of each merged row. That cell keeps its id; the
/// cells it absorbs are tombstoned; a continued row's content moves into the
/// first row's cell, so the ids of its blocks are retired too.
///
/// Every refusal happens before the document is touched: a range that is not on
/// cell boundaries of every row it covers, or that starts or ends inside a
/// vertical merge, cannot be addressed cell by cell and is refused.
fn merge_cells_at(
    doc: &mut Document,
    ids: &mut NodeIds,
    story: StoryRef,
    table_path: &Path,
    range: (usize, usize, usize, usize),
    at: u64,
) -> Result<(), String> {
    let (r0, r1, g0, g1) = range;
    let name = story_name(story);
    let plan: Vec<(usize, usize)> = {
        let table = doc.table(story, table_path).ok_or_else(|| "the target is not a table".to_string())?;
        merge_plan(table, range)?
    };
    let before: Vec<Vec<Arc<Block>>> = plan
        .iter()
        .enumerate()
        .map(|(i, (a, _))| {
            doc.table(story, table_path)
                .and_then(|t| t.rows.get(r0 + i))
                .and_then(|row| row.cells.get(*a))
                .map(|cell| cell.blocks.clone())
                .unwrap_or_default()
        })
        .collect();
    doc.table_mut(story, table_path).map_err(|err| err.to_string())?.merge(r0, r1, g0, g1);
    let table = doc.table(story, table_path).ok_or_else(|| "the table vanished".to_string())?;
    for (i, (a, b)) in plan.iter().enumerate() {
        let r = r0 + i;
        let mut container = table_path.0.clone();
        container.push(r as u32);
        for c in (a + 1)..=*b {
            let mut gone = container.clone();
            gone.push(c as u32);
            ids.remove_subtree(&name, &gone, at);
        }
        ids.shift_children(&name, &container, a + 1, -i32::try_from(*b - *a).unwrap_or(i32::MAX));
        // The first cell kept its id; everything below it can have moved, so
        // keep the blocks that are still the same object at the same index and
        // reissue the rest (a continued row was drained, the first row gained
        // blocks from the rows below).
        let mut cell = container;
        cell.push(*a as u32);
        let old = before.get(i).cloned().unwrap_or_default();
        let blocks = table.rows.get(r).and_then(|row| row.cells.get(*a)).map(|c| c.blocks.clone()).unwrap_or_default();
        for (k, block) in blocks.iter().enumerate() {
            if old.get(k).is_some_and(|was| Arc::ptr_eq(was, block)) {
                continue;
            }
            let mut block_path = cell.clone();
            block_path.push(k as u32);
            ids.remove_descendants(&name, &block_path, at);
            if let Some(id) = ids.get(&NodeKey::new(name.clone(), block_path.clone())) {
                ids.remove(id, at);
            }
            ids.id_for(&NodeKey::new(name.clone(), block_path.clone()));
            if let Block::Table(nested) = &**block {
                index_table(ids, &name, &mut block_path, nested, 0);
            }
        }
        // Blocks that left the cell (the merged-away tail) are tombstoned: a
        // late edit is refused instead of landing on a block that moved up.
        for k in blocks.len()..old.len() {
            let mut block_path = cell.clone();
            block_path.push(k as u32);
            ids.remove_subtree(&name, &block_path, at);
        }
    }
    Ok(())
}

/// Split cell `c` of row `r` into `columns` cells with `Table::split_cell`: the
/// cell keeps its id and its content, the new parts appear to its right, and the
/// cells after it move right. The split can refine the grid inside the cell
/// (other rows' cells widen over the new lines), which `split_cell` does itself.
fn split_cells_at(
    doc: &mut Document,
    ids: &mut NodeIds,
    story: StoryRef,
    table_path: &Path,
    r: usize,
    c: usize,
    columns: usize,
) -> Result<(), String> {
    let name = story_name(story);
    if columns < 2 {
        return Err("a split needs at least two cells".to_string());
    }
    if columns > MAX_COLS {
        return Err("a split asks for too many cells".to_string());
    }
    let mut probe = {
        let table = doc.table(story, table_path).ok_or_else(|| "the target is not a table".to_string())?;
        if r >= table.rows.len() {
            return Err(format!("row {r} does not exist"));
        }
        if table.rows.get(r).is_none_or(|row| c >= row.cells.len()) {
            return Err(format!("cell {c} does not exist"));
        }
        table.clone()
    };
    let parts = probe.split_cell(r, c, columns);
    if parts.len() < 2 {
        return Err("the cell cannot be split that way".to_string());
    }
    let added = parts.len().saturating_sub(1);
    *doc.table_mut(story, table_path).map_err(|err| err.to_string())? = probe;
    let table = doc.table(story, table_path).ok_or_else(|| "the table vanished".to_string())?;
    let mut container = table_path.0.clone();
    container.push(r as u32);
    ids.shift_children(&name, &container, c + 1, i32::try_from(added).unwrap_or(i32::MAX));
    for k in 1..=added {
        let Some(cell) = table.rows.get(r).and_then(|row| row.cells.get(c + k)) else { continue };
        let mut path = container.clone();
        path.push((c + k) as u32);
        ids.id_for(&NodeKey::new(name.clone(), path.clone()));
        index_blocks(ids, &name, &mut path, &cell.blocks, 0);
    }
    Ok(())
}

/// The path of a child of `container` (a block list, a row list or a cell list).
fn child_path(container: &[u32], index: usize) -> Path {
    Path(container.iter().copied().chain(std::iter::once(index as u32)).collect())
}

// --- inverses of the structural table edits -------------------------------
//
// A merge, a split and a column removal change the table's shape, so the
// operations that put them back cannot be computed from the table as it is
// afterwards: the removed bodies and the original boundaries are gone. Each of
// the three therefore carries its inverse's data — the cells, their props, the
// grid and the session ids — captured before the forward edit, and the inverse
// refuses to run when the table no longer has the shape the forward edit left.

/// Most bytes an inverse's restore data may take. A region too large to carry
/// keeps no inverse (undo reports the step as unsupported), so a hostile or huge
/// table cannot grow the undo stack and the journal without bound.
const MAX_RESTORE_BYTES: usize = 256 * 1024;

/// Most node ids one inverse may put back (a merge of a very tall table).
const MAX_RESTORE_IDS: usize = 16_384;

/// Why a restore was not applied: the payload is wrong, or the table moved on.
enum RestoreRefusal {
    /// The payload itself is malformed or out of range.
    Bad(String),
    /// The table no longer has the shape the forward edit left, so the inverse
    /// would land on something else. The room answers `undo-conflict`.
    Changed(String),
}

impl RestoreRefusal {
    /// The refusal as a `Reject` for an operation a client sent.
    fn reject(self, op: &Op) -> Reject {
        match self {
            RestoreRefusal::Bad(message) => invalid(op, message),
            RestoreRefusal::Changed(message) => Reject::new(op.op_id.clone(), RejectCode::UndoConflict, message),
        }
    }
}

/// The spans of every row's cells: the shape guard an inverse compares the
/// table against before it puts anything back.
fn table_shape(table: &Table) -> Vec<Vec<u32>> {
    table.rows.iter().map(|row| row.cells.iter().map(|cell| u32::try_from(cell.span()).unwrap_or(u32::MAX)).collect()).collect()
}

fn parse_shape(value: Option<&Value>) -> Result<Vec<Vec<u32>>, RestoreRefusal> {
    let rows = value.and_then(Value::as_array).ok_or_else(|| RestoreRefusal::Bad("`restore.shape` is missing".to_string()))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let spans = row.as_array().ok_or_else(|| RestoreRefusal::Bad("`restore.shape` is malformed".to_string()))?;
        let mut cells = Vec::with_capacity(spans.len());
        for span in spans {
            let span = span.as_u64().ok_or_else(|| RestoreRefusal::Bad("`restore.shape` holds a non-number".to_string()))?;
            cells.push(u32::try_from(span).map_err(|_| RestoreRefusal::Bad("a span does not fit".to_string()))?);
        }
        out.push(cells);
    }
    Ok(out)
}

/// A small non-negative field of a restore payload.
fn restore_index(entry: &Value, key: &str) -> Result<usize, RestoreRefusal> {
    let value = entry.get(key).and_then(Value::as_u64).ok_or_else(|| RestoreRefusal::Bad(format!("a restore entry needs `{key}`")))?;
    usize::try_from(value).map_err(|_| RestoreRefusal::Bad(format!("`{key}` does not fit")))
}

/// Every live id at or below `path`, as `(relative path, id)` and sorted, so an
/// inverse's payload is deterministic.
fn live_id_pairs(ids: &NodeIds, story: &str, base: &[u32], path: &[u32]) -> Vec<(Vec<u32>, u32)> {
    let mut found: Vec<(Vec<u32>, u32)> = ids
        .iter()
        .filter(|(_, key)| key.story == story && key.path.len() >= path.len() && key.path.starts_with(path))
        .map(|(id, key)| (key.path.iter().skip(base.len()).copied().collect(), id.0))
        .collect();
    found.sort();
    found
}

/// The ids of a subtree, for [`Effect::touched`].
fn live_ids(ids: &NodeIds, story: &str, base: &[u32], path: &[u32]) -> Vec<u32> {
    live_id_pairs(ids, story, base, path).into_iter().map(|(_, id)| id).collect()
}

/// The ids of a subtree as restore entries (`{"path": [relative…], "id": n}`).
fn id_entries(ids: &NodeIds, story: &str, base: &[u32], path: &[u32]) -> Vec<Value> {
    live_id_pairs(ids, story, base, path).into_iter().map(|(path, id)| json!({"path": path, "id": id})).collect()
}

/// Put captured ids back on their paths. An id the room tombstoned is revived —
/// the node's content comes back with it — so a client that held the id (the
/// caret holder of a cell a merge absorbed) addresses its own cell again.
fn restore_ids(ids: &mut NodeIds, story: &str, base: &[u32], entries: Option<&Value>) -> Result<(), RestoreRefusal> {
    let Some(items) = entries.and_then(Value::as_array) else { return Ok(()) };
    if items.len() > MAX_RESTORE_IDS {
        return Err(RestoreRefusal::Bad("too many ids to restore".to_string()));
    }
    for item in items {
        let path = item.get("path").and_then(Value::as_array).ok_or_else(|| RestoreRefusal::Bad("an id entry has no `path`".to_string()))?;
        let id = item.get("id").and_then(Value::as_u64).ok_or_else(|| RestoreRefusal::Bad("an id entry has no `id`".to_string()))?;
        let id = u32::try_from(id).map_err(|_| RestoreRefusal::Bad("an id does not fit".to_string()))?;
        let mut key_path = base.to_vec();
        for index in path {
            let index = index.as_u64().ok_or_else(|| RestoreRefusal::Bad("a path index is not a number".to_string()))?;
            key_path.push(u32::try_from(index).map_err(|_| RestoreRefusal::Bad("a path index does not fit".to_string()))?);
        }
        let key = NodeKey::new(story, key_path);
        // A replica whose map diverged keeps its own id; the fresh map the
        // server sends after a structural operation corrects it.
        if ids.get(&key).is_some_and(|existing| existing != NodeId(id)) {
            continue;
        }
        ids.rebase(NodeId(id), key);
    }
    Ok(())
}

/// Whether a restore payload still fits the bound above.
fn fits(restore: &Value) -> bool {
    serde_json::to_vec(restore).is_ok_and(|bytes| bytes.len() <= MAX_RESTORE_BYTES)
}

/// What a merge takes away, as the payload of its inverse (`split_cells` with
/// `restore`): per row, the cells it absorbs with their props and bodies, and
/// the session ids of everything below them. `None` when the region is too large
/// to carry.
fn merge_restore(
    doc: &Document,
    ids: &NodeIds,
    story: StoryRef,
    table_path: &Path,
    first_row: usize,
    plan: &[(usize, usize)],
) -> Result<Option<Value>, String> {
    let name = story_name(story);
    let table = doc.table(story, table_path).ok_or_else(|| "the target is not a table".to_string())?;
    let mut rows = Vec::with_capacity(plan.len());
    let mut entries = Vec::new();
    for (i, (a, b)) in plan.iter().enumerate() {
        let r = first_row + i;
        let Some(cells) = table.rows.get(r).and_then(|row| row.cells.get(*a..=*b)) else { return Ok(None) };
        rows.push(json!({"row": r, "at": a, "cells": cells}));
        for c in *a..=*b {
            let mut path = table_path.0.clone();
            path.push(r as u32);
            path.push(c as u32);
            entries.extend(id_entries(ids, &name, &table_path.0, &path));
        }
    }
    let restore = json!({"rows": rows, "ids": entries});
    Ok(fits(&restore).then_some(restore))
}

/// What a split takes away, as the payload of its inverse (`merge_cells` with
/// `restore`): the grid and the cell before the cut, plus the props of the cells
/// of the other rows that widened over the refined grid. Returns how many parts
/// the split adds and, when the payload fits, the restore data.
fn split_restore(doc: &Document, story: StoryRef, table_path: &Path, r: usize, c: usize, columns: usize) -> Result<(usize, Option<Value>), String> {
    let table = doc.table(story, table_path).ok_or_else(|| "the target is not a table".to_string())?;
    if !(2..=MAX_COLS).contains(&columns) {
        return Ok((0, None));
    }
    if r >= table.rows.len() || table.rows.get(r).is_none_or(|row| c >= row.cells.len()) {
        return Ok((0, None));
    }
    let mut probe = table.clone();
    let parts = probe.split_cell(r, c, columns);
    if parts.len() < 2 {
        return Ok((0, None));
    }
    let cell = table.rows.get(r).and_then(|row| row.cells.get(c)).cloned();
    let mut props = Vec::new();
    for (rr, row) in table.rows.iter().enumerate() {
        if rr == r {
            continue;
        }
        let Some(after) = probe.rows.get(rr) else { continue };
        for (ci, cell) in row.cells.iter().enumerate() {
            if after.cells.get(ci).is_some_and(|wider| wider.props != cell.props) {
                props.push(json!({"row": rr, "at": ci, "props": cell.props}));
            }
        }
    }
    let added = parts.len().saturating_sub(1);
    let restore = json!({"grid": table.grid, "cell": {"row": r, "at": c, "cell": cell}, "added": added, "cells": props});
    Ok((added, fits(&restore).then_some(restore)))
}

/// What a column removal takes away, as the payload of its inverse
/// (`insert_column` with `restore`): the grid width, the body of every cell that
/// vanished, the props of the cells that only shrank, the rows that disappeared
/// with the column, and the session ids of all of it. `None` when the region is
/// too large to carry.
fn remove_column_restore(doc: &Document, ids: &NodeIds, story: StoryRef, table_path: &Path, g: usize) -> Result<Option<Value>, String> {
    let name = story_name(story);
    let table = doc.table(story, table_path).ok_or_else(|| "the target is not a table".to_string())?;
    if g >= table.grid.len() {
        return Ok(None);
    }
    let plan = column_removal_plan(table, g);
    let dropped: Vec<usize> = plan
        .iter()
        .enumerate()
        .filter(|(r, step)| matches!(step, ColumnRemoval::Removed(0)) && table.rows.get(*r).is_some_and(|row| row.cells.len() == 1))
        .map(|(r, _)| r)
        .collect();
    let mut rows = Vec::new();
    let mut entries = Vec::new();
    for (r, step) in plan.iter().enumerate() {
        let Some(row) = table.rows.get(r) else { continue };
        let mut row_path = table_path.0.clone();
        row_path.push(r as u32);
        if dropped.contains(&r) {
            rows.push(json!({"row": r, "kind": "dropped", "body": row}));
            entries.extend(id_entries(ids, &name, &table_path.0, &row_path));
            continue;
        }
        match step {
            ColumnRemoval::Removed(c) => {
                let Some(cell) = row.cells.get(*c) else { continue };
                rows.push(json!({"row": r, "kind": "removed", "at": c, "cell": cell}));
                row_path.push(*c as u32);
                entries.extend(id_entries(ids, &name, &table_path.0, &row_path));
            }
            ColumnRemoval::Shrunk(c) => {
                let Some(cell) = row.cells.get(*c) else { continue };
                rows.push(json!({"row": r, "kind": "shrunk", "at": c, "props": cell.props}));
            }
            ColumnRemoval::Kept => {}
        }
    }
    let width = table.grid.get(g).copied().unwrap_or(72.0);
    let restore = json!({"width": width, "rows": rows, "ids": entries});
    Ok(fits(&restore).then_some(restore))
}

/// The inverse of a merge: put the cells it absorbed back, with their original
/// boundaries, bodies and ids. The merged cell of each row is replaced by the
/// list of cells that row had before, and the cells after it move right again.
fn restore_split(
    doc: &mut Document,
    ids: &mut NodeIds,
    story: StoryRef,
    table_path: &Path,
    restore: &Value,
    version: u64,
) -> Result<(), RestoreRefusal> {
    let name = story_name(story);
    let table = doc.table(story, table_path).ok_or_else(|| RestoreRefusal::Changed("the table is gone".to_string()))?;
    if table_shape(table) != parse_shape(restore.get("shape"))? {
        return Err(RestoreRefusal::Changed("the table changed after the merge landed".to_string()));
    }
    let entries = restore.get("rows").and_then(Value::as_array).ok_or_else(|| RestoreRefusal::Bad("`restore.rows` is missing".to_string()))?;
    if entries.is_empty() || entries.len() > MAX_ROWS {
        return Err(RestoreRefusal::Bad("`restore.rows` is empty or too long".to_string()));
    }
    let mut probe = table.clone();
    let mut plan: Vec<(usize, usize, Vec<Cell>)> = Vec::with_capacity(entries.len());
    for entry in entries {
        let r = restore_index(entry, "row")?;
        let at = restore_index(entry, "at")?;
        let cells: Vec<Cell> = serde_json::from_value(entry.get("cells").cloned().unwrap_or(Value::Null))
            .map_err(|err| RestoreRefusal::Bad(format!("bad cells: {err}")))?;
        if cells.is_empty() || cells.len() > MAX_COLS {
            return Err(RestoreRefusal::Bad("a restored row holds no cells or too many".to_string()));
        }
        let row = probe.rows.get_mut(r).ok_or_else(|| RestoreRefusal::Changed(format!("row {r} does not exist")))?;
        if at >= row.cells.len() {
            return Err(RestoreRefusal::Changed(format!("row {r} has no cell {at}")));
        }
        row.cells.splice(at..=at, cells.clone());
        plan.push((r, at, cells));
    }
    *doc.table_mut(story, table_path).map_err(|err| RestoreRefusal::Changed(err.to_string()))? = probe;
    for (r, at, cells) in &plan {
        let mut container = table_path.0.clone();
        container.push(*r as u32);
        // The cells after the merged one moved left when it absorbed them; they
        // go back to their own indexes first, so their ids name them again.
        ids.shift_children(&name, &container, at + 1, i32::try_from(cells.len().saturating_sub(1)).unwrap_or(i32::MAX));
        let mut merged = container;
        merged.push(*at as u32);
        // The merged cell kept its own id; the blocks it gained from the rows it
        // drained are new, so they are retired before the originals come back.
        ids.remove_descendants(&name, &merged, version);
    }
    restore_ids(ids, &name, &table_path.0, restore.get("ids"))
}

/// The inverse of a split: merge the parts back into the cell it was cut from,
/// with the grid and the props of the widened cells of the other rows restored.
fn restore_merge(
    doc: &mut Document,
    ids: &mut NodeIds,
    story: StoryRef,
    table_path: &Path,
    restore: &Value,
    version: u64,
) -> Result<(), RestoreRefusal> {
    let name = story_name(story);
    let table = doc.table(story, table_path).ok_or_else(|| RestoreRefusal::Changed("the table is gone".to_string()))?;
    if table_shape(table) != parse_shape(restore.get("shape"))? {
        return Err(RestoreRefusal::Changed("the table changed after the split landed".to_string()));
    }
    let grid: Vec<f32> =
        serde_json::from_value(restore.get("grid").cloned().unwrap_or(Value::Null)).map_err(|err| RestoreRefusal::Bad(format!("bad grid: {err}")))?;
    if grid.len() > MAX_COLS {
        return Err(RestoreRefusal::Bad("the restored grid is too wide".to_string()));
    }
    let cell = restore.get("cell").ok_or_else(|| RestoreRefusal::Bad("`restore.cell` is missing".to_string()))?;
    let r = restore_index(cell, "row")?;
    let at = restore_index(cell, "at")?;
    let added = restore_index(restore, "added")?;
    if added >= MAX_COLS {
        return Err(RestoreRefusal::Bad("`restore.added` is out of range".to_string()));
    }
    let body: Cell =
        serde_json::from_value(cell.get("cell").cloned().unwrap_or(Value::Null)).map_err(|err| RestoreRefusal::Bad(format!("bad cell: {err}")))?;
    let widened = restore.get("cells").and_then(Value::as_array).cloned().unwrap_or_default();
    if widened.len() > MAX_ROWS {
        return Err(RestoreRefusal::Bad("`restore.cells` is too long".to_string()));
    }
    let mut probe = table.clone();
    probe.grid = grid;
    for entry in &widened {
        let row = restore_index(entry, "row")?;
        let at = restore_index(entry, "at")?;
        let props: CellProps = serde_json::from_value(entry.get("props").cloned().unwrap_or(Value::Null))
            .map_err(|err| RestoreRefusal::Bad(format!("bad cell props: {err}")))?;
        let cell = probe
            .rows
            .get_mut(row)
            .and_then(|row| row.cells.get_mut(at))
            .ok_or_else(|| RestoreRefusal::Changed(format!("row {row} has no cell {at}")))?;
        cell.props = props;
    }
    let row = probe.rows.get_mut(r).ok_or_else(|| RestoreRefusal::Changed(format!("row {r} does not exist")))?;
    if at.saturating_add(added) >= row.cells.len() {
        return Err(RestoreRefusal::Changed(format!("row {r} has no cells {at}..{}", at.saturating_add(added))));
    }
    row.cells.splice(at..=at + added, [body]);
    *doc.table_mut(story, table_path).map_err(|err| RestoreRefusal::Changed(err.to_string()))? = probe;
    // The parts the split cut are new cells: they never existed before, so their
    // ids are retired rather than reused for the cell that takes them back.
    let mut container = table_path.0.clone();
    container.push(r as u32);
    for k in 1..=added {
        let mut part = container.clone();
        part.push((at + k) as u32);
        ids.remove_subtree(&name, &part, version);
    }
    restore_ids(ids, &name, &table_path.0, restore.get("ids"))
}

/// The inverse of a column removal: put the grid column back with the bodies it
/// took away, the rows that vanished with it, and the ids of all of them. The
/// cells that only shrank get their props back.
fn restore_insert_column(
    doc: &mut Document,
    ids: &mut NodeIds,
    story: StoryRef,
    table_path: &Path,
    index: usize,
    restore: &Value,
) -> Result<(), RestoreRefusal> {
    let name = story_name(story);
    let table = doc.table(story, table_path).ok_or_else(|| RestoreRefusal::Changed("the table is gone".to_string()))?;
    if table_shape(table) != parse_shape(restore.get("shape"))? {
        return Err(RestoreRefusal::Changed("the table changed after the column was removed".to_string()));
    }
    let width = restore.get("width").and_then(Value::as_f64).ok_or_else(|| RestoreRefusal::Bad("`restore.width` is missing".to_string()))?;
    let entries = restore.get("rows").and_then(Value::as_array).ok_or_else(|| RestoreRefusal::Bad("`restore.rows` is missing".to_string()))?;
    if entries.len() > MAX_ROWS {
        return Err(RestoreRefusal::Bad("`restore.rows` is too long".to_string()));
    }
    if index > table.grid.len() || table.grid.len() >= MAX_COLS {
        return Err(RestoreRefusal::Changed(format!("column {index} does not fit the grid")));
    }
    let mut probe = table.clone();
    probe.grid.insert(index, width as f32);
    enum Step {
        Row(usize),
        Cell(usize, usize),
    }
    let mut steps = Vec::with_capacity(entries.len());
    for entry in entries {
        let r = restore_index(entry, "row")?;
        match entry.get("kind").and_then(Value::as_str) {
            Some("dropped") => {
                let body: Row = serde_json::from_value(entry.get("body").cloned().unwrap_or(Value::Null))
                    .map_err(|err| RestoreRefusal::Bad(format!("bad row: {err}")))?;
                if r > probe.rows.len() {
                    return Err(RestoreRefusal::Changed(format!("row {r} does not fit the table")));
                }
                probe.rows.insert(r, body);
                steps.push(Step::Row(r));
            }
            Some("removed") => {
                let at = restore_index(entry, "at")?;
                let body: Cell = serde_json::from_value(entry.get("cell").cloned().unwrap_or(Value::Null))
                    .map_err(|err| RestoreRefusal::Bad(format!("bad cell: {err}")))?;
                let row = probe.rows.get_mut(r).ok_or_else(|| RestoreRefusal::Changed(format!("row {r} does not exist")))?;
                if at > row.cells.len() {
                    return Err(RestoreRefusal::Changed(format!("row {r} has no place for cell {at}")));
                }
                row.cells.insert(at, body);
                steps.push(Step::Cell(r, at));
            }
            Some("shrunk") => {
                let at = restore_index(entry, "at")?;
                let props: CellProps = serde_json::from_value(entry.get("props").cloned().unwrap_or(Value::Null))
                    .map_err(|err| RestoreRefusal::Bad(format!("bad cell props: {err}")))?;
                let cell = probe
                    .rows
                    .get_mut(r)
                    .and_then(|row| row.cells.get_mut(at))
                    .ok_or_else(|| RestoreRefusal::Changed(format!("row {r} has no cell {at}")))?;
                cell.props = props;
            }
            _ => return Err(RestoreRefusal::Bad("a restore entry has no known `kind`".to_string())),
        }
    }
    *doc.table_mut(story, table_path).map_err(|err| RestoreRefusal::Changed(err.to_string()))? = probe;
    for step in steps {
        match step {
            Step::Row(r) => {
                // The rows the removal dropped moved the ones below up; they go
                // back down before the restored row's ids are put back.
                ids.shift_children(&name, &table_path.0, r, 1);
            }
            Step::Cell(r, at) => {
                let mut container = table_path.0.clone();
                container.push(r as u32);
                ids.shift_children(&name, &container, at, 1);
            }
        }
    }
    restore_ids(ids, &name, &table_path.0, restore.get("ids"))
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
        "set_para_format" => {
            let (story, path, _) = delta_pos(delta, ids)?;
            let text = doc.para(story, &path).map(|para| para.text.clone()).ok_or_else(|| "set_para_format addresses no paragraph".to_string())?;
            let patch = format_patch(delta, &text)?;
            apply_format_patch(doc.para_mut(story, &path).map_err(|err| err.to_string())?, patch);
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
        "move_block" => {
            let node = delta.get("nodeId").and_then(Value::as_u64).ok_or_else(|| "move_block has no `nodeId`".to_string())?;
            let node = u32::try_from(node).map_err(|_| "`nodeId` does not fit".to_string())?;
            let count = match delta.get("count") {
                Some(value) => usize::try_from(value.as_u64().ok_or_else(|| "`count` must be a number".to_string())?)
                    .map_err(|_| "`count` does not fit".to_string())?,
                None => 1,
            };
            let neighbour = delta.get("neighbour").and_then(Value::as_u64).ok_or_else(|| "move_block has no `neighbour`".to_string())?;
            let neighbour = u32::try_from(neighbour).map_err(|_| "`neighbour` does not fit".to_string())?;
            let plan = plan_move(doc, ids, node, count, neighbour).map_err(|err| err.to_string())?;
            apply_move(doc, ids, &plan)?;
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
        "insert_column" => {
            let (story, table_path) = delta_table(delta, doc, ids)?;
            let index = usize::try_from(delta.get("index").and_then(Value::as_u64).ok_or_else(|| "insert_column has no `index`".to_string())?)
                .map_err(|_| "`index` does not fit".to_string())?;
            match delta.get("restore").filter(|value| !value.is_null()) {
                Some(restore) => restore_insert_column(doc, ids, story, &table_path, index, restore).map_err(|err| match err {
                    RestoreRefusal::Bad(message) | RestoreRefusal::Changed(message) => message,
                })?,
                None => insert_column_at(doc, ids, story, &table_path, index)?,
            }
        }
        "remove_column" => {
            let (story, table_path) = delta_table(delta, doc, ids)?;
            let index = usize::try_from(delta.get("index").and_then(Value::as_u64).ok_or_else(|| "remove_column has no `index`".to_string())?)
                .map_err(|_| "`index` does not fit".to_string())?;
            remove_column_at(doc, ids, story, &table_path, index, 0)?;
        }
        "merge_cells" => {
            let (story, table_path) = delta_table(delta, doc, ids)?;
            if let Some(restore) = delta.get("restore").filter(|value| !value.is_null()) {
                restore_merge(doc, ids, story, &table_path, restore, 0).map_err(|err| match err {
                    RestoreRefusal::Bad(message) | RestoreRefusal::Changed(message) => message,
                })?;
                return Ok(());
            }
            let pair = |key: &str| -> Result<(usize, usize), String> {
                let items = delta.get(key).and_then(Value::as_array).ok_or_else(|| format!("merge_cells has no `{key}`"))?;
                let first = items.first().and_then(Value::as_u64).ok_or_else(|| format!("`{key}` must hold two indexes"))?;
                let last = items.get(1).and_then(Value::as_u64).ok_or_else(|| format!("`{key}` must hold two indexes"))?;
                Ok((
                    usize::try_from(first).map_err(|_| "an index does not fit".to_string())?,
                    usize::try_from(last).map_err(|_| "an index does not fit".to_string())?,
                ))
            };
            let (r0, r1) = pair("rows")?;
            let (g0, g1) = pair("cols")?;
            merge_cells_at(doc, ids, story, &table_path, (r0, r1, g0, g1), 0)?;
        }
        "split_cells" => {
            let (story, table_path) = delta_table(delta, doc, ids)?;
            if let Some(restore) = delta.get("restore").filter(|value| !value.is_null()) {
                restore_split(doc, ids, story, &table_path, restore, 0).map_err(|err| match err {
                    RestoreRefusal::Bad(message) | RestoreRefusal::Changed(message) => message,
                })?;
                return Ok(());
            }
            let field = |key: &str| -> Result<usize, String> {
                let value = delta.get(key).and_then(Value::as_u64).ok_or_else(|| format!("split_cells has no `{key}`"))?;
                usize::try_from(value).map_err(|_| format!("`{key}` does not fit"))
            };
            split_cells_at(doc, ids, story, &table_path, field("row")?, field("cell")?, field("columns")?)?;
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
/// `remove_row`; a block that swapped places with its neighbour is a
/// `move_block`, while formatting-only changes, a cell count or column change and
/// a wider rearrangement are reported as unsupported.
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
        // A move leaves every block where it was, so the pointers name it: try
        // one adjacent move before the per-block walk (which would mistake the
        // reordered text for an edit). A reorder the primitives cannot express
        // is refused instead of being sent as a chain of text replacements.
        if let Some(change) = adjacent_move(story, prefix, old, new, ids)? {
            out.push(change);
            return Ok(());
        }
        if reordered(old, new) {
            return Err(DiffError::Unsupported(format!("several blocks were reordered in {}", block_where(story, prefix))));
        }
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

/// One adjacent move in a block list: `count` blocks and the block next to them
/// swapped places. A move shares every block with the document before it, so
/// the `Arc` pointers say which block went where and no text has to be compared.
fn adjacent_move(story: &str, prefix: &[u32], old: &[Arc<Block>], new: &[Arc<Block>], ids: &NodeIds) -> Result<Option<LocalChange>, DiffError> {
    let Some(first) = (0..old.len()).find(|index| !Arc::ptr_eq(&old[*index], &new[*index])) else { return Ok(None) };
    let limit = MAX_MOVE_BLOCKS.min(old.len().saturating_sub(first).saturating_sub(1));
    // The run starts just below the first difference and moved up past it.
    if limit > 0 && Arc::ptr_eq(&old[first + 1], &new[first]) {
        for count in 1..=limit {
            if !Arc::ptr_eq(&old[first], &new[first + count]) || !(1..count).all(|at| Arc::ptr_eq(&old[first + 1 + at], &new[first + at])) {
                continue;
            }
            if !(first + count + 1..old.len()).all(|at| Arc::ptr_eq(&old[at], &new[at])) {
                continue;
            }
            let run = node_id(ids, story, prefix, first + 1)?;
            let neighbour = node_id(ids, story, prefix, first)?;
            return Ok(Some(LocalChange {
                kind: OpKind::Structural,
                target: Target::new(story, run, 0),
                payload: json!({"op": "move_block", "count": count, "neighbour": neighbour}),
            }));
        }
    }
    // The run starts at the first difference and moved down past the block after it.
    for count in 1..=limit {
        if !Arc::ptr_eq(&old[first + count], &new[first]) || !(0..count).all(|at| Arc::ptr_eq(&old[first + at], &new[first + 1 + at])) {
            continue;
        }
        if !(first + count + 1..old.len()).all(|at| Arc::ptr_eq(&old[at], &new[at])) {
            continue;
        }
        let run = node_id(ids, story, prefix, first)?;
        let neighbour = node_id(ids, story, prefix, first + count)?;
        return Ok(Some(LocalChange {
            kind: OpKind::Structural,
            target: Target::new(story, run, 0),
            payload: json!({"op": "move_block", "count": count, "neighbour": neighbour}),
        }));
    }
    Ok(None)
}

/// Whether `new` holds the same blocks as `old` in another order: the pointers
/// are the same, only their positions differ. A reorder the primitives cannot
/// name must be refused, not sent as a chain of text replacements.
fn reordered(old: &[Arc<Block>], new: &[Arc<Block>]) -> bool {
    // A move changes at least two positions; one differing block is an edit.
    // Checking that first keeps the pointer multiset below off the typing path,
    // where every keystroke diffs the whole story.
    let mut differing = 0_usize;
    for (a, b) in old.iter().zip(new) {
        if !Arc::ptr_eq(a, b) {
            differing = differing.saturating_add(1);
            if differing > 1 {
                break;
            }
        }
    }
    if differing < 2 {
        return false;
    }
    let mut counts: HashMap<usize, isize> = HashMap::with_capacity(old.len());
    for block in old {
        *counts.entry(Arc::as_ptr(block) as usize).or_default() += 1;
    }
    for block in new {
        let count = counts.entry(Arc::as_ptr(block) as usize).or_default();
        *count -= 1;
        if *count < 0 {
            return false;
        }
    }
    counts.values().all(|count| *count == 0)
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
        if let Some(change) = table_shape_change(story, table_path, old, new, ids) {
            out.push(change);
            return Ok(());
        }
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

/// A column insert or removal, a cell merge or a cell split: replayed on a copy
/// of the old table with the document's own primitive, the one that reproduces
/// the new table names the operation. `None` leaves the change to the per-cell
/// walk or to the command fallback.
fn table_shape_change(story: &str, table_path: &[u32], old: &Table, new: &Table, ids: &NodeIds) -> Option<LocalChange> {
    let table = node_id(ids, story, table_path, usize::MAX).ok()?;
    if old.grid.len().saturating_add(1) == new.grid.len() {
        for g in 0..=old.grid.len() {
            let mut probe = old.clone();
            probe.insert_col(g);
            if &probe == new {
                return Some(LocalChange {
                    kind: OpKind::Structural,
                    target: Target::new(story, table, 0),
                    payload: json!({"op": "insert_column", "index": g}),
                });
            }
        }
    }
    if old.grid.len() == new.grid.len().saturating_add(1) {
        for g in 0..old.grid.len() {
            let mut probe = old.clone();
            probe.delete_col(g);
            if &probe == new {
                return Some(LocalChange {
                    kind: OpKind::Structural,
                    target: Target::new(story, table, 0),
                    payload: json!({"op": "remove_column", "index": g}),
                });
            }
        }
    }
    let changed: Vec<usize> = (0..old.rows.len()).filter(|&r| !same_row(&old.rows[r], &new.rows[r])).collect();
    let first = *changed.first()?;
    let last = *changed.last()?;
    let contiguous = changed.len() == last.saturating_sub(first).saturating_add(1);
    // A merge takes cells away from every row it covers and leaves the grid
    // alone; a split adds cells to one row and may widen cells of the others.
    if contiguous && old.grid == new.grid && changed.iter().all(|&r| old.rows[r].cells.len() > new.rows[r].cells.len()) {
        let k = old.rows[first].cells.len().checked_sub(new.rows[first].cells.len())?;
        if !(first..=last).all(|r| old.rows[r].cells.len().checked_sub(new.rows[r].cells.len()) == Some(k)) {
            return None;
        }
        for a in 0..=old.rows[first].cells.len().saturating_sub(k.saturating_add(1)) {
            let g0 = old.grid_col(first, a);
            let g1 = old.grid_col(first, a.saturating_add(k));
            let mut probe = old.clone();
            probe.merge(first, last, g0, g1);
            if &probe == new {
                return Some(LocalChange {
                    kind: OpKind::Structural,
                    target: Target::new(story, table, 0),
                    payload: json!({"op": "merge_cells", "rows": [first, last], "cols": [g0, g1]}),
                });
            }
        }
        return None;
    }
    let grown: Vec<usize> = changed.iter().copied().filter(|&r| new.rows[r].cells.len() > old.rows[r].cells.len()).collect();
    if let [r] = grown[..] {
        let columns = new.rows[r].cells.len().checked_sub(old.rows[r].cells.len())?.saturating_add(1);
        if columns > MAX_COLS {
            return None;
        }
        for c in 0..old.rows[r].cells.len() {
            let mut probe = old.clone();
            if probe.split_cell(r, c, columns).len() < 2 {
                continue;
            }
            if &probe == new {
                return Some(LocalChange {
                    kind: OpKind::Structural,
                    target: Target::new(story, table, 0),
                    payload: json!({"op": "split_cells", "row": r, "cell": c, "columns": columns}),
                });
            }
        }
    }
    None
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

/// The body and formatting fields a `set_para_format` restores. Each is
/// optional so an inverse carries only what the forward command changed;
/// `None` = leave it. `text` names the paragraph body and `objects` the inline
/// objects that live in it: a command that rewrites characters
/// (`format.changeCase`) or replaces them with an enclosed-character field
/// (`format.enclose` with `text`) cannot be undone by formatting alone.
#[derive(Clone, Debug, Default, PartialEq)]
struct FormatPatch {
    text: Option<String>,
    objects: Option<Vec<InlineObject>>,
    runs: Option<Vec<Run>>,
    mark: Option<CharProps>,
    props: Option<ParaProps>,
}

/// Parse and validate a `set_para_format` payload (or delta) against the
/// paragraph's current text and objects. Every check runs before the caller
/// mutates anything: a run list that does not cover the target text exactly, a
/// run that ends inside a character, or a text restore whose `expect` does not
/// match the paragraph is refused, not normalized.
fn format_patch(payload: &Value, text: &str) -> Result<FormatPatch, String> {
    // A body restore names the text the paragraph must hold *now* (`expect`), so
    // a stale inverse cannot overwrite an edit the room did not see.
    let text_patch = match payload.get("text") {
        Some(value) if !value.is_null() => {
            let expect = payload.get("expect").and_then(Value::as_str).ok_or_else(|| "a paragraph body restore needs `expect`".to_string())?;
            if expect != text {
                return Err("the paragraph text changed since the restore was prepared".to_string());
            }
            Some(serde_json::from_value::<String>(value.clone()).map_err(|err| format!("bad `text`: {err}"))?)
        }
        _ => None,
    };
    let objects_patch = match payload.get("objects") {
        Some(value) if !value.is_null() => {
            Some(serde_json::from_value::<Vec<InlineObject>>(value.clone()).map_err(|err| format!("bad `objects`: {err}"))?)
        }
        _ => None,
    };
    if text_patch.is_none() && objects_patch.is_some() {
        return Err("`objects` needs the `text` it belongs to".to_string());
    }
    if text_patch.is_some() && objects_patch.is_none() {
        return Err("a paragraph body restore needs its `objects`".to_string());
    }
    let target_text = text_patch.as_deref().unwrap_or(text);
    if let Some(objects_patch) = &objects_patch
        && objects_patch.len() != target_text.chars().filter(|c| *c == wordcraft_doc::para::OBJ).count()
    {
        return Err("the restored objects do not match the text's object characters".to_string());
    }
    let runs = match payload.get("runs") {
        Some(value) if !value.is_null() => {
            let runs: Vec<Run> = serde_json::from_value(value.clone()).map_err(|err| format!("bad `runs`: {err}"))?;
            if runs.len() > MAX_FORMAT_RUNS {
                return Err(format!("more than {MAX_FORMAT_RUNS} runs to restore"));
            }
            let mut end = 0usize;
            for run in &runs {
                if run.len == 0 {
                    return Err("a restored run is empty".to_string());
                }
                end = end.checked_add(run.len).ok_or_else(|| "the restored runs are too long".to_string())?;
                if end > target_text.len() {
                    return Err("the restored runs are longer than the paragraph".to_string());
                }
                if !target_text.is_char_boundary(end) {
                    return Err("a restored run ends inside a character".to_string());
                }
            }
            if end != target_text.len() {
                return Err(format!("the restored runs cover {end} of {} bytes", target_text.len()));
            }
            Some(runs)
        }
        _ => None,
    };
    if text_patch.is_some() && runs.is_none() {
        return Err("a paragraph body restore needs its `runs`".to_string());
    }
    let mark = match payload.get("mark") {
        Some(value) if !value.is_null() => Some(serde_json::from_value::<CharProps>(value.clone()).map_err(|err| format!("bad `mark`: {err}"))?),
        _ => None,
    };
    let props = match payload.get("props") {
        Some(value) if !value.is_null() => Some(serde_json::from_value::<ParaProps>(value.clone()).map_err(|err| format!("bad `props`: {err}"))?),
        _ => None,
    };
    if text_patch.is_none() && runs.is_none() && mark.is_none() && props.is_none() {
        return Err("set_para_format changes nothing".to_string());
    }
    Ok(FormatPatch { text: text_patch, objects: objects_patch, runs, mark, props })
}

fn apply_format_patch(para: &mut Paragraph, patch: FormatPatch) {
    if let Some(text) = patch.text {
        para.text = text;
    }
    if let Some(objects) = patch.objects {
        para.objects = objects;
    }
    if let Some(runs) = patch.runs {
        para.runs = runs;
    }
    if let Some(mark) = patch.mark {
        para.mark = mark;
    }
    if let Some(props) = patch.props {
        para.props = props;
    }
    para.touch();
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
