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
use wordcraft_doc::{Block, Blocks, Document, InlineObject, ParaProps, Paragraph, Path, Pos, Run, StoryRef, Table};
use wordcraft_engine::{Selection, Session};

/// Command areas that must never run from a network operation.
const DENIED_PREFIXES: &[&str] =
    &["file.", "tools.", "ui.", "view.", "caret.", "select.", "collab.", "edit.undo", "edit.redo", "edit.repeat", "review.readAloud"];

/// The frequent character-formatting commands whose result can be undone
/// exactly (ADR-0010). Most of them only change character properties — a direct
/// property, a character style, or a rebuild of the cleared ones — so the
/// inverse carries `runs`/`mark`/`props`. Two of them rewrite the characters
/// themselves: `format.changeCase` (the same text in another case) and
/// `format.enclose` (an enclosed-character field replaces or wraps the
/// selection). Their inverse carries the paragraph's `text` and `objects` as
/// well, so undo restores the runs the command rebuilt instead of putting the
/// old formatting over new text.
const CHAR_FORMAT_COMMANDS: &[&str] = &[
    "format.allCaps",
    "format.bold",
    "format.border",
    "format.changeCase",
    "format.charStyle",
    "format.clear",
    "format.color",
    "format.doubleStrikethrough",
    "format.doubleUnderline",
    "format.emboss",
    "format.emphasisMark",
    "format.enclose",
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
/// borders, shading, direction, the paragraph style). Commands that strip text
/// (`para.removeBlank`) stay barriers. `para.sort` is normalized to `permute` by
/// [`SessionModel::normalized_permute`], the outline level commands
/// (`para.promote`/`demote` and the Outlining tab's levels) to
/// `set_para_level` by [`SessionModel::normalized_level`], and the Bullets and
/// Numbering buttons (`para.bullets`/`para.numbering`) to `set_para_list` by
/// [`SessionModel::normalized_list`].
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
/// selection is not captured: the command still runs, but the step has no
/// inverse, so undo answers `unsupported` instead of sending an unbounded
/// transaction to every replica.
const MAX_FORMAT_PARAGRAPHS: usize = 256;

/// One paragraph before a formatting command, captured so the command's
/// inverse can name the previous value exactly. `text` and `objects` are only
/// used when the command rewrote the characters (`format.changeCase`,
/// `format.enclose`), where formatting alone would not restore the paragraph.
struct FormatSnapshot {
    story: StoryRef,
    path: Path,
    node_id: u32,
    text: String,
    objects: Vec<InlineObject>,
    runs: Vec<Run>,
    mark: CharProps,
    props: ParaProps,
}

/// The contiguous run of paragraphs `para.sort` is about to reorder, captured
/// before it runs so the result can be named by session ids even when the sort
/// changes nothing. `None` when the run is wider than
/// [`MAX_PERMUTE_BLOCKS`](wordcraft_collab_doc::MAX_PERMUTE_BLOCKS), crosses a
/// container, is not made of paragraphs or has a block without a session id:
/// such a step stays a command and its undo answers `unsupported`.
struct SortRange {
    story: StoryRef,
    container: Vec<u32>,
    start: usize,
    count: usize,
    ids: Vec<u32>,
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
        // A command that moves blocks is normalized after it runs: the document
        // difference says whether it was one adjacent move, which every replica
        // can replay as a structural operation and undo exactly. A sort is
        // normalized from the selected run it ran over, so even a sort that
        // changes nothing is an addressable step.
        let sort_range = if command == "para.sort" { self.sort_range(ids) } else { None };
        let before_doc = (is_move_command(command) || command == "para.sort" || is_level_command(command) || is_list_command(command))
            .then(|| self.session.doc.clone());
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

        // A block move lands as `move_block`, addressed by ids: replicas apply
        // the same structural delta instead of replaying the command, and the
        // step's inverse puts the order back. A move wider than the bound stays
        // a command step and is reported as `unsupported` by undo.
        if let Some(before_doc) = before_doc {
            // A sort is always a `permute`, even when its result happens to be
            // one adjacent move: the order list is what the user asked for and
            // the undo step stays one transaction.
            if let Some(range) = sort_range.as_ref()
                && let Some(effect) = self.normalized_permute(op, &before_doc, range, ids)
            {
                return Ok(effect);
            }
            if is_move_command(command)
                && let Some(effect) = self.normalized_move(op, &before_doc, ids)
            {
                return Ok(effect);
            }
            // A level command names the level of every paragraph it changed, so
            // its forward effect travels as `set_para_level` instead of a
            // command every replica replays.
            if is_level_command(command)
                && let Some(effect) = self.normalized_level(op, &before_doc, ids, label)
            {
                return Ok(effect);
            }
            // The Bullets and Numbering buttons name the exact list state of
            // every paragraph they changed, so their forward effect travels as
            // `set_para_list` instead of a command every replica replays.
            if is_list_command(command)
                && let Some(effect) = self.normalized_list(op, &before_doc, ids, label, list_op_name(command))
            {
                return Ok(effect);
            }
        }

        // A step that moved blocks in a way the adapter could not normalize (a
        // selection wider than the bound) cannot be restored by paragraph
        // paths; formatting commands never move blocks.
        let (inverse, touched) = match format {
            Some(Some(captured)) if !nodes_changed => self.format_inverse(op, &captured).unwrap_or_default(),
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
    fn capture_format(&self, op: &Op, ids: &NodeIds) -> Result<Option<Vec<FormatSnapshot>>, Reject> {
        let (a, b) = self.session.sel.ordered();
        if a.story != b.story {
            return Ok(Some(Vec::new()));
        }
        let name = story_name(a.story);
        let mut out = Vec::new();
        for path in self.session.doc.paths_between(&a, &b) {
            let Some(para) = self.session.doc.para(a.story, &path) else { continue };
            // A selection wider than the limit is not captured: the command
            // still runs, but the step stays a barrier instead of sending an
            // unbounded transaction to every replica.
            if out.len() >= MAX_FORMAT_PARAGRAPHS {
                return Ok(None);
            }
            let node = ids
                .get(&NodeKey::new(name.clone(), path.0.clone()))
                .ok_or_else(|| invalid(op, format!("the paragraph {} has no session id, so its formatting cannot be undone", path_key(&path))))?;
            out.push(FormatSnapshot {
                story: a.story,
                path,
                node_id: node.0,
                text: para.text.clone(),
                objects: para.objects.clone(),
                runs: para.runs.clone(),
                mark: para.mark.clone(),
                props: para.props.clone(),
            });
        }
        Ok(Some(out))
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
            // A captured path that no longer holds a paragraph means the command
            // moved blocks: the whole step becomes a barrier rather than a
            // partial restore.
            let text = para.text != before.text;
            let objects = para.objects != before.objects;
            let runs = para.runs != before.runs;
            let mark = para.mark != before.mark;
            let props = para.props != before.props;
            if !(text || objects || runs || mark || props) {
                continue;
            }
            if inverse.len() >= MAX_FORMAT_PARAGRAPHS {
                return None;
            }
            let mut payload = json!({"op": "set_para_format"});
            if text || objects {
                // The command rewrote the characters: the inverse names the
                // exact previous text, its inline objects and its run
                // composition, and `expect` names what the paragraph holds now
                // so a stale restore cannot overwrite a later edit.
                payload["text"] = json!(before.text);
                payload["objects"] = serde_json::to_value(&before.objects).unwrap_or(Value::Null);
                payload["expect"] = json!(para.text);
                payload["runs"] = serde_json::to_value(&before.runs).unwrap_or(Value::Null);
            } else if runs {
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

    /// A block move as one structural operation: the command already reordered
    /// the document (the engine owns the rules for a selection, a table in the
    /// way and a section break), so the document difference is read back. When
    /// it is exactly one adjacent move of at most
    /// [`MAX_MOVE_BLOCKS`](wordcraft_collab_doc::MAX_MOVE_BLOCKS) blocks, the
    /// step's delta becomes `move_block` with two explicit ids and its inverse
    /// is the same move back, one block at a time. Anything else (a selection
    /// wider than the bound) keeps the command delta and stays a barrier.
    fn normalized_move(&self, op: &Op, before: &Document, ids: &mut NodeIds) -> Option<Effect> {
        let changes = wordcraft_collab_doc::diff(before, &self.session.doc, ids).ok()?;
        if changes.len() != 1 {
            return None;
        }
        let change = changes.first()?;
        if change.kind != OpKind::Structural || change.payload.get("op").and_then(Value::as_str) != Some("move_block") {
            return None;
        }
        let count = usize::try_from(change.payload.get("count")?.as_u64()?).ok()?;
        let neighbour = u32::try_from(change.payload.get("neighbour")?.as_u64()?).ok()?;
        let run = change.target.node_id;
        let plan = wordcraft_collab_doc::plan_move(before, ids, run, count, neighbour).ok()?;
        // An edit in any block the move displaced blocks the undo.
        let touched = wordcraft_collab_doc::move_touched(ids, &plan);
        wordcraft_collab_doc::rebase_ids_after_move(ids, &plan);
        Some(Effect {
            delta: Some(wordcraft_collab_doc::move_delta(run, count, neighbour)),
            inverse: wordcraft_collab_doc::move_inverse_ops(op, &plan),
            forward: vec![op.clone()],
            label: "Move paragraphs".into(),
            nodes_changed: true,
            touched,
        })
    }

    /// The run `para.sort` will reorder: the paragraphs between the selection
    /// ends in one container, with their session ids.
    fn sort_range(&self, ids: &NodeIds) -> Option<SortRange> {
        let (a, b) = self.session.sel.ordered();
        if a.story != b.story || a.path.parent() != b.path.parent() {
            return None;
        }
        let container = a.path.parent().to_vec();
        let first = a.path.last();
        let last = b.path.last();
        let (start, end) = (first.min(last), first.max(last));
        let count = end.checked_sub(start)?.checked_add(1)?;
        if count == 0 || count > wordcraft_collab_doc::MAX_PERMUTE_BLOCKS {
            return None;
        }
        let blocks = self.session.doc.container(a.story, &Path(block_path(&container, start)))?;
        let story_name = story_name(a.story);
        let mut range_ids = Vec::with_capacity(count);
        for index in start..=end {
            match blocks.get(index).map(|block| &**block) {
                Some(Block::Para(_)) => {}
                _ => return None,
            }
            let key = NodeKey::new(story_name.clone(), block_path(&container, index));
            range_ids.push(ids.get(&key)?.0);
        }
        Some(SortRange { story: a.story, container, start, count, ids: range_ids })
    }

    /// A `para.sort` as one structural `permute`: the command already reordered
    /// the document, so the run is named by the ids of the blocks in their new
    /// order. The inverse carries the old order; both travel as one
    /// `transaction` delta, and an edit in any block of the run blocks the undo.
    fn normalized_permute(&self, op: &Op, before: &Document, range: &SortRange, ids: &mut NodeIds) -> Option<Effect> {
        let before_blocks = before.container(range.story, &Path(block_path(&range.container, range.start)))?;
        let after_blocks = self.session.doc.container(range.story, &Path(block_path(&range.container, range.start)))?;
        if before_blocks.len() < range.start.saturating_add(range.count) || after_blocks.len() < range.start.saturating_add(range.count) {
            return None;
        }
        let mut order = Vec::with_capacity(range.count);
        for position in 0..range.count {
            let block = after_blocks.get(range.start.saturating_add(position))?;
            let mut found = None;
            for old in 0..range.count {
                if std::sync::Arc::ptr_eq(before_blocks.get(range.start.saturating_add(old))?, block) {
                    found = range.ids.get(old).copied();
                    break;
                }
            }
            order.push(found?);
        }
        let plan = wordcraft_collab_doc::plan_permute(before, ids, &order).ok()?;
        // An edit in any block the sort moved blocks the undo.
        let touched = wordcraft_collab_doc::permute_touched(ids, &plan);
        wordcraft_collab_doc::rebase_ids_after_permute(ids, &plan);
        Some(Effect {
            delta: Some(wordcraft_collab_doc::permute_delta(&plan.order_ids)),
            inverse: wordcraft_collab_doc::permute_inverse_ops(op, &plan),
            forward: vec![op.clone()],
            label: "Sort paragraphs".into(),
            nodes_changed: true,
            touched,
        })
    }

    /// A level command as one structural `set_para_level`: the command already
    /// changed the document, so the difference is read back. When the only
    /// change is the level of at most
    /// [`MAX_LEVEL_PARAGRAPHS`](wordcraft_collab_doc::MAX_LEVEL_PARAGRAPHS)
    /// paragraphs, the step's delta names every changed paragraph by session id
    /// and its level, and its inverse names the levels before it; both travel as
    /// one `transaction` delta, and an edit in any levelled paragraph blocks the
    /// undo. A command that also changed text, numbering or the block order
    /// keeps its command delta and stays a barrier.
    fn normalized_level(&self, op: &Op, before: &Document, ids: &NodeIds, label: &str) -> Option<Effect> {
        let changes = level_changes(before, &self.session.doc)?;
        if changes.is_empty() || changes.len() > wordcraft_collab_doc::MAX_LEVEL_PARAGRAPHS {
            return None;
        }
        let mut items = Vec::with_capacity(changes.len());
        for change in &changes {
            let id = ids.get(&NodeKey::new(change.story.clone(), change.path.clone()))?.0;
            let implied = wordcraft_collab_doc::implied_style(change.after_level);
            let style = if change.after_style == implied && change.after_outline.is_none() {
                wordcraft_collab_doc::LevelStyle::Implied
            } else {
                wordcraft_collab_doc::LevelStyle::Named(change.after_style.clone())
            };
            let outline = if style == wordcraft_collab_doc::LevelStyle::Implied { None } else { change.after_outline };
            items.push(wordcraft_collab_doc::LevelItem { id, level: change.after_level, style, outline });
        }
        // A level change moves no path, so the same ids resolve in the document
        // before the command and the plan names the levels undo puts back.
        let plan = wordcraft_collab_doc::plan_level(before, ids, &items).ok()?;
        let touched = wordcraft_collab_doc::level_touched(&plan);
        Some(Effect {
            delta: Some(wordcraft_collab_doc::level_delta(&items)),
            inverse: wordcraft_collab_doc::level_inverse_ops(op, &plan),
            forward: vec![op.clone()],
            label: label.to_string(),
            nodes_changed: false,
            touched,
        })
    }

    /// A list command as one structural `set_para_list`: the command already
    /// changed the document, so the difference is read back. When the only
    /// change is the list-facing state of at most
    /// [`MAX_LIST_PARAGRAPHS`](wordcraft_collab_doc::MAX_LIST_PARAGRAPHS)
    /// paragraphs — the numbering reference, the paragraph style and the four
    /// indents the list commands clear — the step's delta names every changed
    /// paragraph by session id with its exact state, and the definitions the
    /// command allocated travel with it. The inverse names the state before the
    /// step and drops the definitions the step created (only when nothing uses
    /// them any more), so undo and redo land on the same document at every
    /// replica. A command that also changed text, runs, objects, the mark, the
    /// section, another paragraph property or the block order — or a changed
    /// paragraph inside a table or without a session id — is not captured: the
    /// step keeps its command delta and stays a barrier.
    fn normalized_list(&self, op: &Op, before: &Document, ids: &NodeIds, label: &str, name: &str) -> Option<Effect> {
        let (items, define) = list_changes(before, &self.session.doc, ids)?;
        if items.is_empty() || items.len() > wordcraft_collab_doc::MAX_LIST_PARAGRAPHS {
            return None;
        }
        let step = wordcraft_collab_doc::ListStep::Exact { items, define, undefine: Vec::new() };
        let plan = wordcraft_collab_doc::plan_list(before, ids, &step).ok()?;
        let touched = wordcraft_collab_doc::list_touched(&plan);
        Some(Effect {
            delta: Some(wordcraft_collab_doc::list_delta(&plan, name)),
            inverse: wordcraft_collab_doc::list_inverse_ops(op, &plan, name),
            forward: vec![op.clone()],
            label: label.to_string(),
            nodes_changed: false,
            touched,
        })
    }
}

/// One paragraph a level command changed: where it lives and the level, style
/// and direct outline level it carries after the command.
struct LevelChangeAt {
    story: String,
    path: Vec<u32>,
    after_level: Option<u8>,
    after_style: Option<String>,
    after_outline: Option<u8>,
}

/// The paragraphs whose outline level changed between two documents, or `None`
/// when the difference is more than a level: a different number of blocks or
/// parts, a block or table shape that moved, or a paragraph whose text, runs,
/// mark, objects, section or other properties changed. A level command leaves
/// every path in place, so the two documents are walked side by side; blocks the
/// command did not touch share their `Arc` and are skipped by pointer.
fn level_changes(before: &Document, after: &Document) -> Option<Vec<LevelChangeAt>> {
    if before.parts.len() != after.parts.len() {
        return None;
    }
    let mut out = Vec::new();
    if !collect_level_changes("body", &mut Vec::new(), &before.body, &after.body, after, &mut out) {
        return None;
    }
    for (part, old) in &before.parts {
        let new = after.parts.get(part)?;
        if !collect_level_changes(&format!("part:{part}"), &mut Vec::new(), &old.blocks, &new.blocks, after, &mut out) {
            return None;
        }
    }
    Some(out)
}

fn collect_level_changes(story: &str, prefix: &mut Vec<u32>, old: &Blocks, new: &Blocks, after: &Document, out: &mut Vec<LevelChangeAt>) -> bool {
    if old.len() != new.len() {
        return false;
    }
    for (index, (a, b)) in old.iter().zip(new.iter()).enumerate() {
        if std::sync::Arc::ptr_eq(a, b) {
            continue;
        }
        prefix.push(index as u32);
        let ok = match (a.as_ref(), b.as_ref()) {
            (Block::Para(old_para), Block::Para(new_para)) => {
                if !same_para_except_level(old_para, new_para) {
                    false
                } else if old_para.props.style != new_para.props.style || old_para.props.outline_level != new_para.props.outline_level {
                    out.push(LevelChangeAt {
                        story: story.to_string(),
                        path: prefix.clone(),
                        after_level: wordcraft_collab_doc::para_level(after, &new_para.props),
                        after_style: new_para.props.style.clone(),
                        after_outline: new_para.props.outline_level,
                    });
                    true
                } else {
                    // Touched but unchanged (a command that set the level it
                    // already had): not part of the step's address.
                    true
                }
            }
            (Block::Table(old_table), Block::Table(new_table)) => {
                let mut cells_ok = same_table_shape(old_table, new_table);
                for (r, (old_row, new_row)) in old_table.rows.iter().zip(&new_table.rows).enumerate() {
                    if !cells_ok {
                        break;
                    }
                    for (c, (old_cell, new_cell)) in old_row.cells.iter().zip(&new_row.cells).enumerate() {
                        prefix.push(r as u32);
                        prefix.push(c as u32);
                        cells_ok = collect_level_changes(story, prefix, &old_cell.blocks, &new_cell.blocks, after, out);
                        prefix.pop();
                        prefix.pop();
                        if !cells_ok {
                            break;
                        }
                    }
                }
                cells_ok
            }
            _ => false,
        };
        prefix.pop();
        if !ok {
            return false;
        }
    }
    true
}

/// Whether two paragraphs differ only in their style and direct outline level:
/// a level command changes nothing else, so any other difference means the
/// step cannot be named by levels alone.
fn same_para_except_level(old: &Paragraph, new: &Paragraph) -> bool {
    if old.text != new.text || old.objects != new.objects || old.runs != new.runs || old.mark != new.mark || old.section != new.section {
        return false;
    }
    let strip = |props: &ParaProps| ParaProps { style: None, outline_level: None, ..props.clone() };
    strip(&old.props) == strip(&new.props)
}

/// Whether two tables have the same rows and the same cells per row: a level
/// command never changes a table's shape, so a different shape is a structural
/// change.
fn same_table_shape(old: &Table, new: &Table) -> bool {
    old.rows.len() == new.rows.len() && old.rows.iter().zip(&new.rows).all(|(old_row, new_row)| old_row.cells.len() == new_row.cells.len())
}

/// The path of a child of `container`.
fn block_path(container: &[u32], index: usize) -> Vec<u32> {
    container.iter().copied().chain(std::iter::once(index as u32)).collect()
}

/// The paragraphs whose list-facing state changed between two documents, with
/// the numbering definitions the change added, or `None` when the difference is
/// more than a list: a different number of blocks or parts, a table that
/// changed (a list inside a cell cannot be addressed by `set_para_list`), a
/// paragraph whose text, runs, mark, objects, section or other properties
/// changed, or a paragraph the session map cannot name. A list command leaves
/// every path in place, so the two documents are walked side by side; blocks
/// the command did not touch share their `Arc` and are skipped by pointer.
fn list_changes(
    before: &Document,
    after: &Document,
    ids: &NodeIds,
) -> Option<(Vec<wordcraft_collab_doc::ListItem>, Vec<wordcraft_collab_doc::ListDefinition>)> {
    if before.parts.len() != after.parts.len() {
        return None;
    }
    let mut out = Vec::new();
    if !collect_list_changes("body", &before.body, &after.body, ids, &mut out) {
        return None;
    }
    for (part, old) in &before.parts {
        let new = after.parts.get(part)?;
        if !collect_list_changes(&format!("part:{part}"), &old.blocks, &new.blocks, ids, &mut out) {
            return None;
        }
    }
    let define = list_definitions_of(before, after)?;
    Some((out, define))
}

fn collect_list_changes(story: &str, old: &Blocks, new: &Blocks, ids: &NodeIds, out: &mut Vec<wordcraft_collab_doc::ListItem>) -> bool {
    if old.len() != new.len() {
        return false;
    }
    for (index, (a, b)) in old.iter().zip(new.iter()).enumerate() {
        if std::sync::Arc::ptr_eq(a, b) {
            continue;
        }
        let ok = match (a.as_ref(), b.as_ref()) {
            (Block::Para(old_para), Block::Para(new_para)) => {
                if !same_para_except_list(old_para, new_para) {
                    false
                } else {
                    let before = wordcraft_collab_doc::ListState::of(&old_para.props);
                    let after = wordcraft_collab_doc::ListState::of(&new_para.props);
                    if before == after {
                        // Touched but unchanged (a command that set the list
                        // it already had): not part of the step's address.
                        true
                    } else {
                        // A paragraph inside a table has a deeper path; the
                        // list step does not address it, so the step stays a
                        // command barrier.
                        let key = NodeKey::new(story.to_string(), vec![index as u32]);
                        match ids.get(&key) {
                            Some(id) => {
                                out.push(wordcraft_collab_doc::ListItem { id: id.0, state: after });
                                true
                            }
                            None => false,
                        }
                    }
                }
            }
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Whether two paragraphs differ only in the fields a list command writes: the
/// numbering reference, the style and the four indents it clears.
fn same_para_except_list(old: &Paragraph, new: &Paragraph) -> bool {
    if old.text != new.text || old.objects != new.objects || old.runs != new.runs || old.mark != new.mark || old.section != new.section {
        return false;
    }
    let strip = |props: &ParaProps| ParaProps {
        style: None,
        numbering: None,
        indent_left: None,
        indent_first: None,
        indent_left_chars: None,
        indent_first_chars: None,
        ..props.clone()
    };
    strip(&old.props) == strip(&new.props)
}

/// The numbering definitions a command added. A list command only adds a
/// definition; a change to an existing one (or a removal) is not a list step,
/// so the command stays a barrier.
fn list_definitions_of(before: &Document, after: &Document) -> Option<Vec<wordcraft_collab_doc::ListDefinition>> {
    let old_abstracts = &before.numbering.abstracts;
    let new_abstracts = &after.numbering.abstracts;
    if old_abstracts.iter().any(|old| new_abstracts.iter().find(|new| new.id == old.id).is_none_or(|new| new != old)) {
        return None;
    }
    let old_nums = &before.numbering.nums;
    let new_nums = &after.numbering.nums;
    if old_nums.iter().any(|old| new_nums.iter().find(|new| new.id == old.id).is_none_or(|new| new != old)) {
        return None;
    }
    let mut out = Vec::new();
    for num in new_nums.iter().filter(|num| old_nums.iter().all(|old| old.id != num.id)) {
        let abstract_num = new_abstracts.iter().find(|abstract_num| abstract_num.id == num.abstract_id)?.clone();
        out.push(wordcraft_collab_doc::ListDefinition { abstract_num, num: num.clone() });
    }
    if out.len() > wordcraft_collab_doc::MAX_LIST_DEFINITIONS {
        return None;
    }
    Some(out)
}

fn is_format_command(command: &str) -> bool {
    CHAR_FORMAT_COMMANDS.contains(&command) || PARA_FORMAT_COMMANDS.contains(&command)
}

/// Commands that reorder blocks. Their effect is normalized to `move_block`
/// after the run, so replicas converge on a structural delta and the step has an
/// exact inverse (ADR-0010).
const MOVE_COMMANDS: &[&str] = &["para.moveUp", "para.moveDown", "outline.moveUp", "outline.moveDown"];

fn is_move_command(command: &str) -> bool {
    MOVE_COMMANDS.contains(&command)
}

/// Commands whose only effect is an outline level: promote and demote (Word's
/// Alt+Shift+←/→), and the Outlining tab's absolute levels. They are normalized
/// to `set_para_level` after the run — the levels the command left, named by
/// session ids — so every replica applies the same structural delta and the
/// step's inverse puts the earlier levels back (ADR-0010). A command that also
/// changed a list level, moved a block or rewrote text stays a command barrier.
const LEVEL_COMMANDS: &[&str] =
    &["para.promote", "para.demote", "outline.promote", "outline.demote", "outline.promoteToHeading1", "outline.demoteToBody", "outline.level"];

fn is_level_command(command: &str) -> bool {
    LEVEL_COMMANDS.contains(&command)
}

/// Commands whose only document effect is list membership: the Home ›
/// Paragraph Bullets and Numbering buttons. They are normalized to
/// `set_para_list` after the run — the exact list state the command left, named
/// by session ids — so every replica applies the same structural delta and the
/// step's inverse puts the earlier list references and definitions back
/// (ADR-0010). A command that also changed a list definition another way, a
/// paragraph inside a table or a step wider than the bound stays a command
/// barrier.
const LIST_COMMANDS: &[&str] = &["para.bullets", "para.numbering", "para.multilevel", "para.restartNumbering"];

fn is_list_command(command: &str) -> bool {
    LIST_COMMANDS.contains(&command)
}

/// The structural operation a list command normalizes to. The Multilevel button
/// and Restart numbering keep their own op name so a step (and its inverse) is
/// readable as what the user asked for; the two single-level buttons share
/// `set_para_list`.
fn list_op_name(command: &str) -> &'static str {
    match command {
        "para.multilevel" => "set_para_multilevel",
        "para.restartNumbering" => "set_para_restart_numbering",
        _ => "set_para_list",
    }
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
