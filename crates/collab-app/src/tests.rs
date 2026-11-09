//! Sync-state tests: no socket, just the room, the document and the maps.

use std::sync::Arc;

use serde_json::json;

use wordcraft_collab::ids::{NodeId, NodeKey};
use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{Applied, HistoryAction, NodeDelta, NodeRef, Op, OpId, OpKind, Reject, RejectCode, Target};
use wordcraft_collab_doc::DocModel;
use wordcraft_collab_engine::SessionModel;
use wordcraft_doc::{Block, Cell, CharProps, Document, Paragraph, Path, Pos, StoryRef, Table};
use wordcraft_engine::Session;

use crate::{CopyBuffer, Outcome, Outgoing, Status, SyncState};

fn server() -> (Room<DocModel>, Vec<NodeRef>) {
    let mut room = Room::new(DocModel::new(Document::new()));
    let plan = room.join(None, false);
    (room, plan.nodes)
}

fn state_with(nodes: &[NodeRef]) -> SyncState {
    let mut state = SyncState::new("alice", "Alice", Role::Editor);
    state.set_nodes(nodes);
    state
}

fn text(session: &Session) -> String {
    session.doc.body.first().and_then(|block| block.as_para()).map(|para| para.text.clone()).unwrap_or_default()
}

#[test]
fn a_local_edit_plans_a_replace_range() {
    let (mut room, nodes) = server();
    let state = state_with(&nodes);
    let node = nodes.iter().find(|node| node.story == "body").expect("a body node").id;
    let mut session = Session::new(Document::new());
    let before = session.doc.clone();
    session.run("text.insert", &json!({"text": "Hi"})).expect("type");

    let plans = state.plan(&before, &session, "text.insert", &json!({"text": "Hi"})).expect("a plan");
    assert_eq!(plans.len(), 1);
    assert!(matches!(plans[0].kind, OpKind::Primitive));
    assert_eq!(plans[0].payload, json!({"op": "replace_range", "len": 0, "text": "Hi"}));
    assert_eq!(plans[0].target, Target::new("body", node, 0));

    // The plan is an op the room accepts and it reproduces the same document.
    let applied = room
        .submit(
            "alice",
            Role::Editor,
            wordcraft_collab::wire::Op {
                op_id: OpId::new("a", 1),
                base_version: 0,
                epoch: None,
                kind: plans[0].kind,
                target: plans[0].target.clone(),
                payload: plans[0].payload.clone(),
                history: None,
            },
        )
        .expect("the room accepts");
    assert_eq!(applied.version, 1);
}

#[test]
fn a_formatting_change_falls_back_to_the_command_with_an_explicit_selection() {
    let (_room, nodes) = server();
    let state = state_with(&nodes);
    let node = nodes.iter().find(|node| node.story == "body").expect("a body node").id;
    let mut session = Session::new(Document::new());
    session.run("text.insert", &json!({"text": "Hello"})).expect("type");
    let before = session.doc.clone();
    let anchor = Pos { story: StoryRef::Body, path: Path::top(0), off: 0 };
    let focus = Pos { story: StoryRef::Body, path: Path::top(0), off: 5 };
    session.sel = wordcraft_engine::Selection { anchor, focus };
    session.run("format.bold", &json!({})).expect("bold");

    let plans = state.plan(&before, &session, "format.bold", &json!({})).expect("a plan");
    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    assert!(matches!(plan.kind, OpKind::Command));
    assert_eq!(plan.payload["command"], "format.bold");
    assert_eq!(plan.payload["selection"]["anchor"], json!({"nodeId": node, "off": 0}));
    assert_eq!(plan.payload["selection"]["focus"], json!({"nodeId": node, "off": 5}));
}

#[test]
fn a_remote_delta_applies_to_the_session() {
    let (mut room, nodes) = server();
    let mut state = state_with(&nodes);
    let node = nodes.iter().find(|node| node.story == "body").expect("a body node").id;
    let applied = room
        .submit(
            "bob",
            Role::Editor,
            wordcraft_collab::wire::Op {
                op_id: OpId::new("b", 1),
                base_version: 0,
                epoch: None,
                kind: OpKind::Primitive,
                target: Target::new("body", node, 0),
                payload: json!({"op": "insert_text", "text": "remote"}),
                history: None,
            },
        )
        .expect("bob types");
    let mut session = Session::new(Document::new());
    state.apply(&applied, &mut session).expect("the delta applies");
    assert_eq!(text(&session), "remote");
    assert_eq!(state.version, 1);
}

#[test]
fn our_own_answer_is_not_applied_twice() {
    let (_room, nodes) = server();
    let mut state = state_with(&nodes);
    let node = nodes.iter().find(|node| node.story == "body").expect("a body node").id;
    let mut session = Session::new(Document::new());
    session.run("text.insert", &json!({"text": "once"})).expect("the UI already applied it");
    let applied = Applied {
        op_id: OpId::new("alice", 1),
        version: 1,
        epoch: 0,
        delta: Some(json!({"op": "insert_text", "nodeId": node, "off": 0, "text": "once"})),
        nodes: None,
        conflict: false,
        history: None,
    };
    state.apply(&applied, &mut session).expect("no error");
    assert_eq!(text(&session), "once", "the authoritative echo did not duplicate the text");
    assert_eq!(state.version, 1);
}

#[test]
fn track_changes_forces_the_command_path_so_revisions_keep_their_author() {
    let (_room, nodes) = server();
    let state = state_with(&nodes);
    let mut session = Session::new(Document::new());
    session.doc.settings.track_changes = true;
    let before = session.doc.clone();
    session.run("text.insert", &json!({"text": "Hi"})).expect("type");

    let plans = state.plan(&before, &session, "text.insert", &json!({"text": "Hi"})).expect("a plan");
    assert_eq!(plans.len(), 1);
    assert!(matches!(plans[0].kind, OpKind::Command), "a primitive would lose the revision");
    assert_eq!(plans[0].payload["command"], "text.insert");
}

#[test]
fn a_command_that_may_allocate_ids_without_a_delta_resyncs() {
    let (_room, nodes) = server();
    let mut state = state_with(&nodes);
    let mut session = Session::new(Document::new());
    let applied = Applied {
        op_id: OpId::new("b", 1),
        version: 1,
        epoch: 0,
        delta: Some(json!({"op": "command", "command": "review.newComment"})),
        nodes: None,
        conflict: false,
        history: None,
    };
    assert_eq!(state.apply(&applied, &mut session).expect("no error"), Outcome::Resync, "an older server asks for the snapshot");
}

#[test]
fn a_node_delta_that_overflowed_resyncs_instead_of_applying_a_partial_map() {
    let (_room, nodes) = server();
    let mut state = state_with(&nodes);
    let mut session = Session::new(Document::new());
    let applied = Applied {
        op_id: OpId::new("b", 1),
        version: 1,
        epoch: 0,
        delta: Some(json!({"op": "command", "command": "review.newComment"})),
        nodes: Some(NodeDelta { overflow: true, ..NodeDelta::default() }),
        conflict: false,
        history: None,
    };
    assert_eq!(state.apply(&applied, &mut session).expect("no error"), Outcome::Resync);
}

#[test]
fn a_cursor_resolves_by_node_id() {
    let (_room, nodes) = server();
    let state = state_with(&nodes);
    let node = nodes.iter().find(|node| node.story == "body").expect("a body node").id;
    let cursor = crate::Cursor { story: "body".into(), node_id: node, off: 0 };
    let pos = state.position(&cursor).expect("a live node");
    assert_eq!(pos.story, StoryRef::Body);
    assert_eq!(pos.path, Path::top(0));
    let gone = crate::Cursor { story: "body".into(), node_id: 999, off: 0 };
    assert!(state.position(&gone).is_none());
}

#[test]
fn an_empty_diff_sends_nothing() {
    let (_room, nodes) = server();
    let state = state_with(&nodes);
    let session = Session::new(Document::new());
    let before = session.doc.clone();
    let plans = state.plan(&before, &session, "none", &json!({})).expect("no plan");
    assert!(plans.is_empty());
}

#[test]
fn presence_gives_a_new_peer_its_role() {
    let mut people = Vec::new();
    crate::apply_presence(&mut people, "c1", "owner", &json!({"cursor": {"story": "body", "nodeId": 2, "off": 4}}));
    crate::apply_presence(&mut people, "c2", "editor", &json!({}));
    crate::apply_presence(&mut people, "c3", "viewer", &json!({}));
    let roles: Vec<_> = people.iter().map(|participant| (participant.client_id.as_str(), participant.role.as_str())).collect();
    assert_eq!(roles, [("c1", "owner"), ("c2", "editor"), ("c3", "viewer")]);
    assert_eq!(people[0].cursor.as_ref().map(|cursor| cursor.off), Some(4));
}

#[test]
fn a_role_from_presence_replaces_the_placeholder() {
    let mut people =
        vec![crate::ParticipantView { client_id: "c1".into(), role: "participant".into(), color: None, cursor: None, selection: None, active: true }];
    crate::apply_presence(&mut people, "c1", "viewer", &json!({"active": true}));
    assert_eq!(people[0].role, "viewer");
    assert_eq!(people.len(), 1, "the peer is updated, not duplicated");
}

#[test]
fn a_presence_without_a_role_keeps_the_known_role() {
    let mut people = Vec::new();
    crate::apply_presence(&mut people, "c1", "owner", &json!({}));
    crate::apply_presence(&mut people, "c1", "", &json!({"active": false}));
    assert_eq!(people[0].role, "owner", "an older server sends no role");
    assert!(!people[0].active);
}

/// Build the operation a client would send for one planned change.
fn op_from(client: &str, seq: u64, base: u64, plan: &Outgoing, history: Option<HistoryAction>) -> Op {
    Op {
        op_id: OpId::new(client, seq),
        base_version: base,
        epoch: None,
        kind: plan.kind,
        target: plan.target.clone(),
        payload: plan.payload.clone(),
        history,
    }
}

/// Build the undo/redo request for a confirmed step.
fn history_op(client: &str, seq: u64, base: u64, step: &Op, action: HistoryAction) -> Op {
    Op {
        op_id: OpId::new(client, seq),
        base_version: base,
        epoch: None,
        kind: step.kind,
        target: step.target.clone(),
        payload: json!({}),
        history: Some(action),
    }
}

#[test]
fn the_author_and_a_peer_apply_an_undo_answer_once() {
    let (mut room, nodes) = server();
    let mut alice = state_with(&nodes);
    let mut peer = SyncState::new("bob", "Bob", Role::Editor);
    peer.set_nodes(&nodes);
    let mut mine = Session::new(Document::new());
    let mut theirs = Session::new(Document::new());

    // The UI applies Alice's typing optimistically; its own echo is skipped.
    let before = mine.doc.clone();
    mine.run("text.insert", &json!({"text": "abc"})).expect("type");
    let plan = alice.plan(&before, &mine, "text.insert", &json!({"text": "abc"})).expect("a plan").remove(0);
    let first = op_from("alice", 1, 0, &plan, None);
    let echo = room.submit("alice", Role::Editor, first.clone()).expect("the first edit lands");
    alice.apply(&echo, &mut mine).expect("own echo is skipped");
    peer.apply(&echo, &mut theirs).expect("the peer applies");

    mine.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(0), off: 3 });
    let before = mine.doc.clone();
    mine.run("text.insert", &json!({"text": "def"})).expect("type");
    let plan = alice.plan(&before, &mine, "text.insert", &json!({"text": "def"})).expect("a plan").remove(0);
    let second = op_from("alice", 2, alice.version, &plan, None);
    let echo = room.submit("alice", Role::Editor, second.clone()).expect("the second edit lands");
    alice.apply(&echo, &mut mine).expect("own echo is skipped");
    peer.apply(&echo, &mut theirs).expect("the peer applies");
    assert_eq!(text(&mine), "abcdef");
    assert_eq!(text(&theirs), "abcdef");

    // Undo, then redo, then undo again: every answer reaches the author once
    // (no snapshot, no doubled inverse) and the peer as an ordinary delta.
    for (round, undo) in [true, false, true, false].into_iter().enumerate() {
        let expected = if undo { "abc" } else { "abcdef" };
        let action = if undo { HistoryAction::Undo { op_id: second.op_id.clone() } } else { HistoryAction::Redo { op_id: second.op_id.clone() } };
        let request = history_op("alice", 3 + round as u64, alice.version, &second, action.clone());
        let answer = room.submit("alice", Role::Editor, request).expect("the history step lands");
        assert_eq!(answer.history, Some(action), "the answer names the history operation");
        alice.apply(&answer, &mut mine).expect("the author applies its own inverse");
        peer.apply(&answer, &mut theirs).expect("the peer applies the inverse");
        assert_eq!(text(&mine), expected, "round {round}: the author is not waiting for a snapshot");
        assert_eq!(text(&theirs), expected, "round {round}: the peer converged");
    }

    // The second step undoes as well, so the first is the newest step left;
    // the later edit on the same node still blocks it. The refusal leaves both
    // documents exactly as they were.
    let undo = history_op("alice", 9, alice.version, &second, HistoryAction::Undo { op_id: second.op_id.clone() });
    let answer = room.submit("alice", Role::Editor, undo).expect("the newest step undoes");
    alice.apply(&answer, &mut mine).expect("the author applies the inverse");
    peer.apply(&answer, &mut theirs).expect("the peer applies the inverse");
    assert_eq!(text(&mine), "abc");
    let err = room
        .submit("alice", Role::Editor, history_op("alice", 10, alice.version, &first, HistoryAction::Undo { op_id: first.op_id.clone() }))
        .expect_err("a later edit on the same node blocks the older undo");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(text(&mine), "abc");
    assert_eq!(text(&theirs), "abc");
}

fn table_doc(rows: usize, cols: usize) -> Document {
    let mut doc = Document::new();
    let mut table = Table::new(rows, cols, 200.0);
    for (r, row) in table.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            *cell = Cell::with_text(&format!("r{r}c{c}"));
        }
    }
    doc.body.push(Arc::new(Block::Table(table)));
    doc
}

fn cell_text(session: &Session, row: u32, cell: u32) -> String {
    session.doc.para(StoryRef::Body, &Path(vec![1, row, cell, 0])).map(|para| para.text.clone()).unwrap_or_default()
}

#[test]
fn an_undo_in_a_table_is_refused_at_the_author_without_losing_the_peers_edit() {
    let doc = table_doc(2, 2);
    let mut room = Room::new(DocModel::new(doc.clone()));
    let plan = room.join(None, false);
    let table = room.ids().get(&NodeKey::new("body", vec![1])).expect("the table has an id").0;
    let merged = room.ids().get(&NodeKey::new("body", vec![1, 0, 0, 0])).expect("the cell paragraph has an id").0;

    let mut alice = state_with(&plan.nodes);
    let mut peer = SyncState::new("bob", "Bob", Role::Editor);
    peer.set_nodes(&plan.nodes);
    let mut mine = Session::new(doc.clone());
    let mut theirs = Session::new(doc);

    // Alice merges two cells; her document has the merge optimistically.
    let merge = Op {
        op_id: OpId::new("alice", 1),
        base_version: 0,
        epoch: None,
        kind: OpKind::Structural,
        target: Target::new("body", table, 0),
        payload: json!({"op": "merge_cells", "rows": [0, 0], "cols": [0, 1]}),
        history: None,
    };
    let answer = room.submit("alice", Role::Editor, merge.clone()).expect("the cells merge");
    let mut ids = alice.ids.clone();
    wordcraft_collab_engine::apply_delta(&mut mine, &mut ids, answer.delta.as_ref().expect("a delta")).expect("the merge applies");
    alice.ids = ids;
    alice.version = answer.version;
    peer.apply(&answer, &mut theirs).expect("the peer merges");

    // Bob types into the merged cell; the author applies that as a remote delta.
    let edit = Op {
        op_id: OpId::new("bob", 1),
        base_version: 1,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", merged, 4),
        payload: json!({"op": "insert_text", "text": "!"}),
        history: None,
    };
    let answer = room.submit("bob", Role::Editor, edit).expect("bob types");
    peer.apply(&answer, &mut theirs).expect("bob's own echo is skipped");
    alice.apply(&answer, &mut mine).expect("the author applies the peer's edit");
    let before = serde_json::to_value(&mine.doc).expect("json");
    assert_eq!(cell_text(&mine, 0, 0), "r0c0!", "the author shows the peer's edit");

    // Undoing the merge would overwrite that edit; the room refuses before
    // changing anything and the author keeps the document it had.
    let undo = Op {
        op_id: OpId::new("alice", 2),
        base_version: 2,
        epoch: None,
        kind: OpKind::Structural,
        target: Target::new("body", table, 0),
        payload: json!({}),
        history: Some(HistoryAction::Undo { op_id: merge.op_id.clone() }),
    };
    let err = room.submit("alice", Role::Editor, undo).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    let (status, resync) = crate::refusal_status(&err);
    assert_eq!(status, Status::InSync, "a refused undo leaves the session usable");
    assert!(!resync);
    assert_eq!(serde_json::to_value(&mine.doc).expect("json"), before, "the refused undo changed nothing at the author");
    assert_eq!(cell_text(&mine, 0, 0), "r0c0!", "bob's text survived");
    assert_eq!(room.version(), 2, "the room did not advance");
}

#[test]
fn a_refused_undo_is_an_error_and_a_stale_answer_resyncs() {
    let op = OpId::new("alice", 9);
    let conflict = Reject::new(op.clone(), RejectCode::UndoConflict, "the step's targets changed after it landed");
    assert_eq!(crate::refusal_status(&conflict), (Status::InSync, false));
    let stale = Reject::new(op, RejectCode::Stale, "baseVersion was compacted out of the journal; resync from a snapshot");
    assert_eq!(crate::refusal_status(&stale), (Status::Resyncing, true));
}

#[test]
fn a_table_undo_reaches_its_author_and_the_peer_like_a_text_undo() {
    for (name, payload) in [
        ("merge_cells", json!({"op": "merge_cells", "rows": [0, 0], "cols": [0, 1]})),
        ("split_cells", json!({"op": "split_cells", "row": 0, "cell": 0, "columns": 2})),
        ("remove_column", json!({"op": "remove_column", "index": 0})),
    ] {
        let doc = table_doc(2, 2);
        let mut room = Room::new(DocModel::new(doc.clone()));
        let plan = room.join(None, false);
        let table = room.ids().get(&NodeKey::new("body", vec![1])).expect("the table has an id").0;
        let mut alice = state_with(&plan.nodes);
        let mut peer = SyncState::new("bob", "Bob", Role::Editor);
        peer.set_nodes(&plan.nodes);
        let mut mine = Session::new(doc.clone());
        let mut theirs = Session::new(doc.clone());

        let step = Op {
            op_id: OpId::new("alice", 1),
            base_version: 0,
            epoch: None,
            kind: OpKind::Structural,
            target: Target::new("body", table, 0),
            payload,
            history: None,
        };
        let answer = room.submit("alice", Role::Editor, step.clone()).expect("the table edit lands");
        let mut ids = alice.ids.clone();
        wordcraft_collab_engine::apply_delta(&mut mine, &mut ids, answer.delta.as_ref().expect("a delta")).expect("the edit applies");
        alice.ids = ids;
        alice.version = answer.version;
        peer.apply(&answer, &mut theirs).expect("the peer applies the table edit");
        assert_ne!(serde_json::to_value(&mine.doc).expect("json"), serde_json::to_value(&doc).expect("json"), "{name}: the edit landed");

        let undo = history_op("alice", 2, alice.version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
        let answer = room.submit("alice", Role::Editor, undo).expect("the inverse lands");
        alice.apply(&answer, &mut mine).expect("the author applies its own inverse, without a snapshot");
        peer.apply(&answer, &mut theirs).expect("the peer applies the inverse");
        assert_eq!(serde_json::to_value(&mine.doc).expect("json"), serde_json::to_value(&doc).expect("json"), "{name}: the author is back");
        assert_eq!(serde_json::to_value(&theirs.doc).expect("json"), serde_json::to_value(&doc).expect("json"), "{name}: the peer converged");
    }
}

/// A room and a plain document with one paragraph per text.
fn format_doc(texts: &[&str]) -> Document {
    let mut doc = Document::new();
    doc.body.clear();
    for text in texts {
        doc.body.push(Arc::new(Block::Para(Paragraph::with_text(text, CharProps::default()))));
    }
    doc
}

fn selection(anchor: u32, from: usize, focus: u32, to: usize) -> serde_json::Value {
    json!({"anchor": {"nodeId": anchor, "off": from}, "focus": {"nodeId": focus, "off": to}})
}

/// Two replicas and the room between them, for the formatting tests: Alice
/// edits locally and sends what her client plans, Bob gets the authoritative
/// delta.
struct FormatPair {
    room: Room<SessionModel>,
    alice: SyncState,
    peer: SyncState,
    mine: Session,
    theirs: Session,
}

impl FormatPair {
    fn new(texts: &[&str]) -> FormatPair {
        let doc = format_doc(texts);
        let mut room = Room::new(SessionModel::from_document(doc.clone()));
        let nodes = room.join(None, false).nodes;
        let mut peer = SyncState::new("bob", "Bob", Role::Editor);
        peer.set_nodes(&nodes);
        FormatPair { room, alice: state_with(&nodes), peer, mine: Session::new(doc.clone()), theirs: Session::new(doc) }
    }

    /// Run a formatting command on Alice's replica, send it as her client plans
    /// it, and let the peer replay the delta. Returns the operation naming the
    /// step.
    #[allow(clippy::expect_used)]
    fn format(&mut self, seq: u64, command: &str, params: serde_json::Value) -> Op {
        let before = self.mine.doc.clone();
        self.mine.run(command, &params).expect("the UI applies the formatting");
        let plan = self.alice.plan(&before, &self.mine, command, &params).expect("a plan").remove(0);
        assert_eq!(plan.kind, OpKind::Command, "{command} travels as a command");
        let step = op_from("alice", seq, self.alice.version, &plan, None);
        let echo = self.room.submit("alice", Role::Editor, step.clone()).expect("the command lands");
        self.alice.apply(&echo, &mut self.mine).expect("own echo is skipped");
        self.peer.apply(&echo, &mut self.theirs).expect("the peer replays the command");
        step
    }

    /// The command path a client that sends commands submits (track changes on,
    /// or the CLI/MCP): the selection is the one the command was given in the
    /// pre-command document, so a restore can be addressed in that document.
    #[allow(clippy::expect_used)]
    fn rewrite(&mut self, seq: u64, command: &str, params: serde_json::Value) -> Op {
        let anchor = self.mine.sel.anchor.clone();
        let focus = self.mine.sel.focus.clone();
        let node = |pair: &FormatPair, path: usize| pair.alice.ids.get(&NodeKey::new("body", vec![path as u32])).expect("a body node").0;
        let anchor_node = node(self, anchor.path.last());
        let focus_node = node(self, focus.path.last());
        let selection = json!({
            "anchor": {"nodeId": anchor_node, "off": anchor.off},
            "focus": {"nodeId": focus_node, "off": focus.off},
        });
        self.mine.run(command, &params).expect("the UI applies the command");
        let step = Op {
            op_id: OpId::new("alice", seq),
            base_version: self.alice.version,
            epoch: None,
            kind: OpKind::Command,
            target: Target::new("body", anchor_node, anchor.off),
            payload: json!({"command": command, "params": params, "selection": selection}),
            history: None,
        };
        let echo = self.room.submit("alice", Role::Editor, step.clone()).expect("the command lands");
        self.alice.apply(&echo, &mut self.mine).expect("own echo is skipped");
        self.peer.apply(&echo, &mut self.theirs).expect("the peer replays the command");
        step
    }

    /// Run `para.removeBlank` over the whole document on Alice's replica and
    /// send it as her client plans it: a multi-block removal is not a primitive,
    /// so the plan is the command and the server normalizes it to one
    /// `remove_blank` step. A structural answer makes the server broadcast a
    /// fresh node map, as the transport does.
    #[allow(clippy::expect_used)]
    fn remove_blank(&mut self, seq: u64) -> (Op, Applied) {
        let before = self.mine.doc.clone();
        let params = json!({"scope": "all"});
        self.mine.run("para.removeBlank", &params).expect("the UI removes the blanks");
        let plan = self.alice.plan(&before, &self.mine, "para.removeBlank", &params).expect("a plan").remove(0);
        assert_eq!(plan.kind, OpKind::Command, "the command path is what the server normalizes");
        let step = op_from("alice", seq, self.alice.version, &plan, None);
        let echo = self.room.submit("alice", Role::Editor, step.clone()).expect("the command lands");
        self.alice.apply(&echo, &mut self.mine).expect("own echo is skipped");
        self.alice.set_nodes(&self.room.nodes());
        self.peer.apply(&echo, &mut self.theirs).expect("the peer replays the step");
        (step, echo)
    }
}

#[test]
fn a_comment_id_allocation_keeps_both_replicas_editing_without_a_snapshot() {
    let mut pair = FormatPair::new(&["Hello world"]);
    pair.mine.author = "Alice".into();
    pair.theirs.author = "Alice".into();
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(0), off: 5 },
    };
    let before = pair.mine.doc.clone();
    let params = json!({"text": "note"});
    pair.mine.run("review.newComment", &params).expect("the UI adds the comment");
    let plan = pair.alice.plan(&before, &pair.mine, "review.newComment", &params).expect("a plan").remove(0);
    assert_eq!(plan.kind, OpKind::Command, "a new comment part is not a primitive");

    let step = op_from("alice", 1, pair.alice.version, &plan, None);
    let echo = pair.room.submit("Alice", Role::Editor, step).expect("the comment lands");
    let delta = echo.nodes.clone().expect("the answer carries the ids of the new story");
    let new_node = delta.changed.iter().find(|node| node.story.starts_with("part:")).expect("a comment paragraph").clone();

    // Neither replica asks for a snapshot: the author adopts only the ids, the
    // peer replays the command and adopts the same ids.
    assert_eq!(pair.alice.apply(&echo, &mut pair.mine).expect("own answer"), Outcome::Quiet);
    assert_eq!(pair.peer.apply(&echo, &mut pair.theirs).expect("the peer applies"), Outcome::Quiet);
    assert!(pair.alice.ids.resolve(NodeId(new_node.id)).is_ok(), "the author can address the comment");
    assert!(pair.peer.ids.resolve(NodeId(new_node.id)).is_ok(), "the peer can address the comment");
    let part: u32 = new_node.story.trim_start_matches("part:").parse().expect("a part id");
    for session in [&pair.mine, &pair.theirs] {
        assert_eq!(session.doc.comments.len(), 1, "both replicas made one comment");
        let comment = session.doc.comments.values().next().expect("a comment");
        assert_eq!(comment.part, part, "both replicas name the same part");
    }

    // Bob types into the comment paragraph; the server accepts the id it sent,
    // so the conversation continues without a resync.
    let text = |session: &Session| {
        session
            .doc
            .parts
            .get(&part)
            .and_then(|story| story.blocks.first())
            .and_then(|block| block.as_para())
            .map(|para| para.text.clone())
            .unwrap_or_default()
    };
    assert_eq!(text(&pair.mine), "note", "the author's comment text");
    assert_eq!(text(&pair.theirs), "note", "the peer's replayed comment text");
    pair.theirs.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Part(part), path: Path::top(0), off: 4 });
    let before = pair.theirs.doc.clone();
    pair.theirs.run("text.insert", &json!({"text": "!"})).expect("bob types in the comment");
    let plan = pair.peer.plan(&before, &pair.theirs, "text.insert", &json!({"text": "!"})).expect("a plan").remove(0);
    assert_eq!(plan.target.story, new_node.story, "the primitive addresses the comment story");
    assert_eq!(plan.target.node_id, new_node.id, "the id is the server's");
    let step = op_from("bob", 1, pair.peer.version, &plan, None);
    let echo = pair.room.submit("Bob", Role::Editor, step).expect("the edit lands");
    assert_eq!(pair.peer.apply(&echo, &mut pair.theirs).expect("own echo is skipped"), Outcome::Quiet);
    assert_eq!(pair.alice.apply(&echo, &mut pair.mine).expect("the author applies bob's edit"), Outcome::Quiet);
    assert_eq!(text(&pair.theirs), "Note!", "the peer shows its edit");
    assert_eq!(text(&pair.mine), "Note!", "the author converged on it");
}

#[test]
fn a_char_format_undo_returns_the_previous_value_at_its_author_and_the_peer() {
    let mut pair = FormatPair::new(&["Hello"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(0), off: 5 },
    };
    let before = serde_json::to_value(&pair.mine.doc).expect("json");

    let bold = pair.format(1, "format.bold", json!({"value": true}));
    let formatted = serde_json::to_value(&pair.mine.doc).expect("json");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), formatted, "both replicas are bold");
    assert_ne!(formatted, before, "the bold is in the document");

    // Undo: the answer is a primitive naming the previous value, so the author
    // applies its own inverse (ADR-0010) instead of waiting for a snapshot.
    let undo = history_op("alice", 2, pair.alice.version, &bold, HistoryAction::Undo { op_id: bold.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the inverse lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), before, "the author is back at the previous value");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), before, "the peer is back at the previous value");

    // Redo replays the command itself, as before.
    let redo = history_op("alice", 3, pair.alice.version, &bold, HistoryAction::Redo { op_id: bold.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, redo).expect("the redo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies the redo");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the redo");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), formatted, "the author is bold again");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), formatted, "the peer converged");
}

#[test]
fn a_foreign_format_change_refuses_the_undo_without_touching_the_document() {
    let mut pair = FormatPair::new(&["Hello"]);
    let node = pair.alice.ids.get(&NodeKey::new("body", vec![0])).expect("a body node").0;
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(0), off: 5 },
    };
    let bold = pair.format(1, "format.bold", json!({"value": true}));

    // Bob formats the same fields; his replica applies it optimistically.
    pair.theirs.sel = pair.mine.sel.clone();
    pair.theirs.run("format.italic", &json!({"value": true})).expect("bob's UI applies the italic");
    let italic = Op {
        op_id: OpId::new("bob", 1),
        base_version: 1,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: json!({
            "command": "format.italic",
            "params": {"value": true},
            "selection": selection(node, 0, node, 5),
        }),
        history: None,
    };
    let answer = pair.room.submit("bob", Role::Editor, italic).expect("bob's formatting lands");
    pair.peer.apply(&answer, &mut pair.theirs).expect("bob's own echo is skipped");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies bob's change");
    let at_author = serde_json::to_value(&pair.mine.doc).expect("json");
    let at_peer = serde_json::to_value(&pair.theirs.doc).expect("json");
    assert_eq!(at_author, at_peer, "both replicas hold bob's change");

    // Alice's undo would overwrite bob's formatting of the same paragraph: the
    // room refuses it before changing anything.
    let version = pair.room.version();
    let undo = history_op("alice", 2, pair.alice.version, &bold, HistoryAction::Undo { op_id: bold.op_id.clone() });
    let err = pair.room.submit("alice", Role::Editor, undo).expect_err("a later format change blocks the undo");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(crate::refusal_status(&err), (Status::InSync, false), "the session stays usable");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), at_author, "nothing changed at the author");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), at_peer, "nothing changed at the peer");
    assert_eq!(pair.room.version(), version, "the room did not advance");
}

#[test]
fn a_two_paragraph_align_undo_lands_as_one_transaction_on_both_replicas() {
    let mut pair = FormatPair::new(&["alpha", "beta"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(1), off: 2 },
    };
    let before = serde_json::to_value(&pair.mine.doc).expect("json");

    let align = pair.format(1, "para.alignCenter", json!({}));
    let aligned = serde_json::to_value(&pair.mine.doc).expect("json");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), aligned, "both replicas are aligned");
    assert_ne!(aligned, before);

    let undo = history_op("alice", 2, pair.alice.version, &align, HistoryAction::Undo { op_id: align.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the inverses land");
    assert_eq!(
        answer.delta.as_ref().and_then(|delta| delta.get("op")).and_then(|op| op.as_str()),
        Some("transaction"),
        "one step, several paragraphs"
    );
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverses");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverses");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), before, "the author's paragraph properties are back");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), before, "the peer's paragraph properties are back");
}

fn para_texts(session: &Session) -> Vec<String> {
    session.doc.body.iter().filter_map(|block| block.as_para()).map(|para| para.text.clone()).collect()
}

#[test]
fn a_two_paragraph_case_change_undo_lands_as_one_transaction_on_both_replicas() {
    let mut pair = FormatPair::new(&["alpha", "beta"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(1), off: 4 },
    };
    let before = serde_json::to_value(&pair.mine.doc).expect("json");

    // The characters of both paragraphs are rewritten; the command travels as a
    // command, not as a text diff that would drop the run structure.
    let change = pair.rewrite(1, "format.changeCase", json!({"mode": "upper"}));
    let changed = serde_json::to_value(&pair.mine.doc).expect("json");
    assert_eq!(para_texts(&pair.mine), vec!["ALPHA".to_string(), "BETA".to_string()]);
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), changed, "the peer replays the same command");

    // The inverse names each previous body: one `set_para_format` per paragraph,
    // sent to every replica as a single transaction.
    let undo = history_op("alice", 2, pair.alice.version, &change, HistoryAction::Undo { op_id: change.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the body restores land");
    let delta = answer.delta.as_ref().expect("a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, several paragraphs");
    assert_eq!(delta["ops"].as_array().map(Vec::len), Some(2), "{delta}");
    assert_eq!(delta["ops"][0]["text"], json!("alpha"), "{delta}");
    assert_eq!(delta["ops"][0]["expect"], json!("ALPHA"), "{delta}");

    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverses");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverses");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), before, "the author's text and runs are back");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), before, "the peer's text and runs are back");
}

#[test]
fn an_enclose_undo_removes_the_field_at_its_author_and_the_peer() {
    let mut pair = FormatPair::new(&["x"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(0), off: 1 },
    };
    let before = serde_json::to_value(&pair.mine.doc).expect("json");
    let objects = |session: &Session| session.doc.body.first().and_then(|b| b.as_para()).map(|p| p.objects.len()).unwrap_or(0);

    let enclose = pair.rewrite(1, "format.enclose", json!({"shape": "circle", "text": "x"}));
    let enclosed = serde_json::to_value(&pair.mine.doc).expect("json");
    assert_eq!(objects(&pair.mine), 1, "the author holds the field");
    assert_eq!(objects(&pair.theirs), 1, "the peer replays the enclosure");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), enclosed);

    // The inverse restores the body and the objects: the field goes away and the
    // character comes back at both replicas.
    let undo = history_op("alice", 2, pair.alice.version, &enclose, HistoryAction::Undo { op_id: enclose.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the body restore lands");
    let delta = answer.delta.as_ref().expect("a delta");
    assert_eq!(delta["op"], json!("set_para_format"));
    assert_eq!(delta["objects"], json!([]), "the field is gone");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(objects(&pair.mine), 0);
    assert_eq!(objects(&pair.theirs), 0);
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), before, "the author is back at the plain character");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), before, "the peer is back at the plain character");
}

#[test]
fn a_foreign_edit_of_a_rewritten_paragraph_refuses_the_undo_without_touching_the_document() {
    let mut pair = FormatPair::new(&["hello"]);
    let node = pair.alice.ids.get(&NodeKey::new("body", vec![0])).expect("a body node").0;
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(0), off: 5 },
    };
    let change = pair.rewrite(1, "format.changeCase", json!({"mode": "upper"}));

    // Bob types into the same paragraph; his replica applies it optimistically.
    pair.theirs.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(0), off: 5 });
    pair.theirs.run("text.insert", &json!({"text": "!"})).expect("bob's UI types");
    let typed = Op {
        op_id: OpId::new("bob", 1),
        base_version: 1,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", node, 5),
        payload: json!({"op": "insert_text", "text": "!"}),
        history: None,
    };
    let answer = pair.room.submit("bob", Role::Editor, typed).expect("bob's edit lands");
    pair.peer.apply(&answer, &mut pair.theirs).expect("bob's own echo is skipped");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies bob's edit");
    let at_author = serde_json::to_value(&pair.mine.doc).expect("json");
    let at_peer = serde_json::to_value(&pair.theirs.doc).expect("json");
    assert_eq!(at_author, at_peer, "both replicas hold bob's edit");

    // Alice's undo would overwrite bob's editing of the paragraph: the room
    // refuses it before changing anything and without moving the version.
    let version = pair.room.version();
    let undo = history_op("alice", 2, pair.alice.version, &change, HistoryAction::Undo { op_id: change.op_id.clone() });
    let err = pair.room.submit("alice", Role::Editor, undo).expect_err("a later edit of the paragraph blocks the undo");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(crate::refusal_status(&err), (Status::InSync, false), "the session stays usable");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), at_author, "nothing changed at the author");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), at_peer, "nothing changed at the peer");
    assert_eq!(pair.room.version(), version, "the room did not advance");
}

// --- moving paragraphs past their neighbour --------------------------------

/// Alice moves the third paragraph up past the second: her client plans a
/// structural operation with both ids, Bob applies the delta, and the step's
/// inverse (one `move_block` back) reaches both. Returns the operation naming
/// the step and the pair.
#[allow(clippy::expect_used)]
fn moved_pair() -> (FormatPair, Op) {
    let mut pair = FormatPair::new(&["one", "two", "three"]);
    pair.mine.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(2), off: 0 });
    let before = pair.mine.doc.clone();
    pair.mine.run("para.moveUp", &json!({})).expect("the UI moves the paragraph");
    let plan = pair.alice.plan(&before, &pair.mine, "para.moveUp", &json!({})).expect("a plan").remove(0);
    assert_eq!(plan.kind, OpKind::Structural, "a move travels as a structural operation");
    assert_eq!(plan.payload["op"], json!("move_block"));
    let step = op_from("alice", 1, pair.alice.version, &plan, None);
    let echo = pair.room.submit("alice", Role::Editor, step.clone()).expect("the move lands");
    pair.alice.apply(&echo, &mut pair.mine).expect("own echo is skipped");
    pair.peer.apply(&echo, &mut pair.theirs).expect("the peer applies the move");
    assert_eq!(para_texts(&pair.mine), ["one", "three", "two"]);
    assert_eq!(para_texts(&pair.theirs), ["one", "three", "two"]);
    (pair, step)
}

#[test]
fn a_paragraph_move_undo_returns_the_order_at_its_author_and_the_peer() {
    let (mut pair, step) = moved_pair();

    let undo = history_op("alice", 2, pair.alice.version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the undo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(para_texts(&pair.mine), ["one", "two", "three"], "the author is back at the earlier order");
    assert_eq!(para_texts(&pair.theirs), ["one", "two", "three"], "the peer converged");

    let redo = history_op("alice", 3, pair.alice.version, &step, HistoryAction::Redo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, redo).expect("the redo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies the redo");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the redo");
    assert_eq!(para_texts(&pair.mine), ["one", "three", "two"], "redo returns the moved order");
    assert_eq!(para_texts(&pair.theirs), ["one", "three", "two"]);
}

#[test]
fn a_foreign_edit_of_a_moved_paragraph_refuses_the_undo_without_touching_the_document() {
    let (mut pair, step) = moved_pair();

    // Bob types into the block the move displaced (not the step's own target).
    pair.theirs.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(2), off: 0 });
    let before = pair.theirs.doc.clone();
    pair.theirs.run("text.insert", &json!({"text": "X"})).expect("bob types");
    let plan = pair.peer.plan(&before, &pair.theirs, "text.insert", &json!({"text": "X"})).expect("a plan").remove(0);
    let edit = op_from("bob", 1, pair.peer.version, &plan, None);
    let echo = pair.room.submit("bob", Role::Editor, edit).expect("bob's edit lands");
    pair.peer.apply(&echo, &mut pair.theirs).expect("own echo is skipped");
    pair.alice.apply(&echo, &mut pair.mine).expect("alice applies bob's edit");

    let mine_before = serde_json::to_value(&pair.mine.doc).expect("json");
    let theirs_before = serde_json::to_value(&pair.theirs.doc).expect("json");
    let version = pair.alice.version;
    let undo = history_op("alice", 2, version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let err = pair.room.submit("alice", Role::Editor, undo).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), mine_before, "the author's document is unchanged");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), theirs_before, "the peer's document is unchanged");
    assert_eq!(pair.room.version(), version, "the version did not move");
}

// --- sorting a run of paragraphs --------------------------------------------

/// Alice sorts three paragraphs: her client plans one structural `permute` with
/// the ids in their new order, Bob applies the authoritative transaction delta,
/// and the step's inverse (the old order) reaches both.
#[allow(clippy::expect_used)]
fn sorted_pair() -> (FormatPair, Op) {
    let mut pair = FormatPair::new(&["delta", "bravo", "alpha", "charlie"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(3), off: 0 },
    };
    let before = pair.mine.doc.clone();
    pair.mine.run("para.sort", &json!({})).expect("the UI sorts the paragraphs");
    let plan = pair.alice.plan(&before, &pair.mine, "para.sort", &json!({})).expect("a plan").remove(0);
    assert_eq!(plan.kind, OpKind::Structural, "a sort travels as a structural operation");
    assert_eq!(plan.payload["op"], json!("permute"), "the plan names the ids in their new order");
    let step = op_from("alice", 1, pair.alice.version, &plan, None);
    let echo = pair.room.submit("alice", Role::Editor, step.clone()).expect("the sort lands");
    pair.alice.apply(&echo, &mut pair.mine).expect("own echo is skipped");
    // A structural answer makes the server broadcast a fresh node map (ADR-0002).
    pair.alice.set_nodes(&pair.room.nodes());
    pair.peer.apply(&echo, &mut pair.theirs).expect("the peer applies the sort");
    assert_eq!(para_texts(&pair.mine), ["alpha", "bravo", "charlie", "delta"]);
    assert_eq!(para_texts(&pair.theirs), ["alpha", "bravo", "charlie", "delta"]);
    (pair, step)
}

#[test]
fn a_paragraph_sort_undo_lands_as_one_transaction_at_its_author_and_the_peer() {
    let (mut pair, step) = sorted_pair();

    let undo = history_op("alice", 2, pair.alice.version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the undo lands");
    let delta = answer.delta.clone().expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("permute"), "the inverse is the old order");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(para_texts(&pair.mine), ["delta", "bravo", "alpha", "charlie"], "the author is back at the earlier order");
    assert_eq!(para_texts(&pair.theirs), ["delta", "bravo", "alpha", "charlie"], "the peer converged");

    let redo = history_op("alice", 3, pair.alice.version, &step, HistoryAction::Redo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, redo).expect("the redo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies the redo");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the redo");
    assert_eq!(para_texts(&pair.mine), ["alpha", "bravo", "charlie", "delta"], "redo sorts again");
    assert_eq!(para_texts(&pair.theirs), ["alpha", "bravo", "charlie", "delta"]);
}

#[test]
fn a_foreign_edit_of_a_sorted_paragraph_refuses_the_undo_without_touching_the_document() {
    let (mut pair, step) = sorted_pair();

    // Bob types into the block the sort moved to the front (not the step's address).
    pair.theirs.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(0), off: 0 });
    let before = pair.theirs.doc.clone();
    pair.theirs.run("text.insert", &json!({"text": "X"})).expect("bob types");
    let plan = pair.peer.plan(&before, &pair.theirs, "text.insert", &json!({"text": "X"})).expect("a plan").remove(0);
    let edit = op_from("bob", 1, pair.peer.version, &plan, None);
    let echo = pair.room.submit("bob", Role::Editor, edit).expect("bob's edit lands");
    pair.peer.apply(&echo, &mut pair.theirs).expect("own echo is skipped");
    pair.alice.apply(&echo, &mut pair.mine).expect("alice applies bob's edit");

    let mine_before = serde_json::to_value(&pair.mine.doc).expect("json");
    let theirs_before = serde_json::to_value(&pair.theirs.doc).expect("json");
    let version = pair.alice.version;
    let undo = history_op("alice", 2, version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let err = pair.room.submit("alice", Role::Editor, undo).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), mine_before, "the author's document is unchanged");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), theirs_before, "the peer's document is unchanged");
    assert_eq!(pair.room.version(), version, "the version did not move");
}

// --- promoting and demoting paragraphs --------------------------------------

/// Two replicas on a document whose paragraphs carry the styles named.
#[allow(clippy::expect_used)]
fn styled_pair(texts: &[(&str, Option<&str>)]) -> FormatPair {
    let mut doc = Document::new();
    doc.body.clear();
    for (text, style) in texts {
        let mut para = Paragraph::with_text(text, CharProps::default());
        para.props.style = style.map(str::to_string);
        doc.body.push(Arc::new(Block::Para(para)));
    }
    let mut room = Room::new(SessionModel::from_document(doc.clone()));
    let nodes = room.join(None, false).nodes;
    let mut peer = SyncState::new("bob", "Bob", Role::Editor);
    peer.set_nodes(&nodes);
    FormatPair { room, alice: state_with(&nodes), peer, mine: Session::new(doc.clone()), theirs: Session::new(doc) }
}

fn para_styles(session: &Session) -> Vec<Option<String>> {
    session.doc.body.iter().filter_map(|block| block.as_para()).map(|para| para.props.style.clone()).collect()
}

#[test]
fn a_promote_undo_lands_as_one_transaction_at_its_author_and_the_peer() {
    let mut pair = styled_pair(&[("Title", Some("Heading1")), ("Body", None)]);
    pair.mine.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(1), off: 0 });
    let before = pair.mine.doc.clone();
    pair.mine.run("para.promote", &json!({})).expect("the UI promotes the paragraph");
    let plan = pair.alice.plan(&before, &pair.mine, "para.promote", &json!({})).expect("a plan").remove(0);
    assert_eq!(plan.kind, OpKind::Command, "the client sends the command; the server names the level");
    let step = op_from("alice", 1, pair.alice.version, &plan, None);
    let echo = pair.room.submit("alice", Role::Editor, step.clone()).expect("the promote lands");
    let delta = echo.delta.clone().expect("the promote carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_level"), "the forward effect is a level operation");
    assert_eq!(delta["ops"][0]["items"][0]["level"], json!(1));
    pair.alice.apply(&echo, &mut pair.mine).expect("own echo is skipped");
    pair.peer.apply(&echo, &mut pair.theirs).expect("the peer applies the level");
    assert_eq!(para_styles(&pair.mine), vec![Some("Heading1".to_string()), Some("Heading1".to_string())]);
    assert_eq!(
        serde_json::to_value(&pair.mine.doc).expect("json"),
        serde_json::to_value(&pair.theirs.doc).expect("json"),
        "the author and the peer hold the same document"
    );

    let undo = history_op("alice", 2, pair.alice.version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the undo lands");
    let delta = answer.delta.clone().expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "the inverse is one delta too");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_level"), "the inverse names the earlier level");
    assert_eq!(delta["ops"][0]["items"][0]["level"], serde_json::Value::Null);
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(para_styles(&pair.mine), vec![Some("Heading1".to_string()), None], "the author is back at body text");
    assert_eq!(para_styles(&pair.theirs), vec![Some("Heading1".to_string()), None], "the peer converged");

    let redo = history_op("alice", 3, pair.alice.version, &step, HistoryAction::Redo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, redo).expect("the redo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies the redo");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the redo");
    assert_eq!(para_styles(&pair.mine), vec![Some("Heading1".to_string()), Some("Heading1".to_string())], "redo promotes again");
    assert_eq!(para_styles(&pair.theirs), vec![Some("Heading1".to_string()), Some("Heading1".to_string())]);
}

#[test]
fn a_foreign_edit_of_a_promoted_paragraph_refuses_the_undo_without_touching_the_document() {
    let mut pair = styled_pair(&[("Title", Some("Heading1")), ("Body", None)]);
    pair.mine.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(1), off: 0 });
    let before = pair.mine.doc.clone();
    pair.mine.run("para.promote", &json!({})).expect("the UI promotes the paragraph");
    let plan = pair.alice.plan(&before, &pair.mine, "para.promote", &json!({})).expect("a plan").remove(0);
    let step = op_from("alice", 1, pair.alice.version, &plan, None);
    let echo = pair.room.submit("alice", Role::Editor, step.clone()).expect("the promote lands");
    pair.alice.apply(&echo, &mut pair.mine).expect("own echo is skipped");
    pair.peer.apply(&echo, &mut pair.theirs).expect("the peer applies the level");

    // Bob types into the paragraph Alice levelled.
    pair.theirs.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(1), off: 0 });
    let before = pair.theirs.doc.clone();
    pair.theirs.run("text.insert", &json!({"text": "X"})).expect("bob types");
    let plan = pair.peer.plan(&before, &pair.theirs, "text.insert", &json!({"text": "X"})).expect("a plan").remove(0);
    let edit = op_from("bob", 1, pair.peer.version, &plan, None);
    let echo = pair.room.submit("bob", Role::Editor, edit).expect("bob's edit lands");
    pair.peer.apply(&echo, &mut pair.theirs).expect("own echo is skipped");
    pair.alice.apply(&echo, &mut pair.mine).expect("alice applies bob's edit");

    let mine_before = serde_json::to_value(&pair.mine.doc).expect("json");
    let theirs_before = serde_json::to_value(&pair.theirs.doc).expect("json");
    let version = pair.alice.version;
    let undo = history_op("alice", 2, version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let err = pair.room.submit("alice", Role::Editor, undo).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), mine_before, "the author's document is unchanged");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), theirs_before, "the peer's document is unchanged");
    assert_eq!(pair.room.version(), version, "the version did not move");
}

// --- bullets and numbering ---------------------------------------------------

impl FormatPair {
    /// Run a list command on Alice's replica, send it the way her client plans
    /// it, and let the peer apply the authoritative delta. Returns the step and
    /// the answer (whose `delta` is the list operation).
    #[allow(clippy::expect_used)]
    fn list(&mut self, seq: u64, command: &str, params: serde_json::Value) -> (Op, Applied) {
        let before = self.mine.doc.clone();
        self.mine.run(command, &params).expect("the UI applies the list command");
        let plan = self.alice.plan(&before, &self.mine, command, &params).expect("a plan").remove(0);
        assert_eq!(plan.kind, OpKind::Command, "{command} travels as a command");
        let step = op_from("alice", seq, self.alice.version, &plan, None);
        let echo = self.room.submit("alice", Role::Editor, step.clone()).expect("the command lands");
        self.alice.apply(&echo, &mut self.mine).expect("own echo is skipped");
        self.peer.apply(&echo, &mut self.theirs).expect("the peer applies the list delta");
        (step, echo)
    }
}

fn para_numbering(session: &Session, block: usize) -> Option<wordcraft_doc::props::NumRef> {
    session.doc.body.get(block).and_then(|block| block.as_para()).and_then(|para| para.props.numbering)
}

#[test]
fn a_bullets_undo_lands_as_one_transaction_at_its_author_and_the_peer() {
    let mut pair = FormatPair::new(&["One", "Two"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(1), off: 3 },
    };
    let before = serde_json::to_value(&pair.mine.doc).expect("json");

    let (step, echo) = pair.list(1, "para.bullets", json!({}));
    let delta = echo.delta.clone().expect("the bullets carry a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_list"), "the forward effect is a list operation");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "both selected paragraphs are named");
    let num = items[0]["num"].as_u64().expect("a list id");
    assert_eq!(items[1]["num"], json!(num));
    assert_eq!(delta["ops"][0]["define"][0]["num"]["id"], json!(num), "the definition travels with the delta");
    let listed = serde_json::to_value(&pair.mine.doc).expect("json");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), listed, "the author and the peer hold the same document");
    assert_eq!(para_numbering(&pair.mine, 0), para_numbering(&pair.theirs, 0), "both replicas name the same numbering id");

    let undo = history_op("alice", 2, pair.alice.version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the undo lands");
    let delta = answer.delta.clone().expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_list"), "the inverse names the earlier list state");
    assert_eq!(delta["ops"][0]["items"][0]["num"], serde_json::Value::Null);
    assert_eq!(delta["ops"][0]["undefine"], json!([num]), "the definition the step created is dropped");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), before, "the author is back at the earlier document");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), before, "the peer is back at the earlier document");

    let redo = history_op("alice", 3, pair.alice.version, &step, HistoryAction::Redo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, redo).expect("the redo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies the redo");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the redo");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), listed, "redo lists again with the same numbering id");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), listed);
}

#[test]
fn a_foreign_edit_of_a_listed_paragraph_refuses_the_undo_without_touching_the_document() {
    let mut pair = FormatPair::new(&["One", "Two"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(1), off: 3 },
    };
    let (step, _echo) = pair.list(1, "para.bullets", json!({}));

    // Bob types into the second paragraph Alice listed.
    pair.theirs.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(1), off: 0 });
    let before = pair.theirs.doc.clone();
    pair.theirs.run("text.insert", &json!({"text": "X"})).expect("bob types");
    let plan = pair.peer.plan(&before, &pair.theirs, "text.insert", &json!({"text": "X"})).expect("a plan").remove(0);
    let edit = op_from("bob", 1, pair.peer.version, &plan, None);
    let echo = pair.room.submit("bob", Role::Editor, edit).expect("bob's edit lands");
    pair.peer.apply(&echo, &mut pair.theirs).expect("own echo is skipped");
    pair.alice.apply(&echo, &mut pair.mine).expect("alice applies bob's edit");

    let mine_before = serde_json::to_value(&pair.mine.doc).expect("json");
    let theirs_before = serde_json::to_value(&pair.theirs.doc).expect("json");
    let version = pair.alice.version;
    let undo = history_op("alice", 2, version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let err = pair.room.submit("alice", Role::Editor, undo).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), mine_before, "the author's document is unchanged");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), theirs_before, "the peer's document is unchanged");
    assert_eq!(pair.room.version(), version, "the version did not move");
}

#[test]
fn a_multilevel_undo_lands_as_one_transaction_at_its_author_and_the_peer() {
    let mut pair = FormatPair::new(&["One", "Two"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(1), off: 3 },
    };
    let before = serde_json::to_value(&pair.mine.doc).expect("json");

    let (step, echo) = pair.list(1, "para.multilevel", json!({}));
    let delta = echo.delta.clone().expect("the multilevel list carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_multilevel"), "the forward effect keeps the multilevel op");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "both selected paragraphs are named");
    let num = items[0]["num"].as_u64().expect("a list id");
    assert_eq!(items[1]["num"], json!(num), "one definition for the step");
    assert_eq!(delta["ops"][0]["define"][0]["num"]["id"], json!(num), "the definition travels with the delta");
    let listed = serde_json::to_value(&pair.mine.doc).expect("json");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), listed, "the author and the peer hold the same document");
    assert_eq!(para_numbering(&pair.mine, 0), para_numbering(&pair.theirs, 0), "both replicas name the same numbering id");

    let undo = history_op("alice", 2, pair.alice.version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the undo lands");
    let delta = answer.delta.clone().expect("the undo carries a delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_multilevel"), "the inverse keeps the multilevel op");
    assert_eq!(delta["ops"][0]["items"][0]["num"], serde_json::Value::Null);
    assert_eq!(delta["ops"][0]["undefine"], json!([num]), "the definition the step created is dropped");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), before, "the author is back at the earlier document");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), before, "the peer is back at the earlier document");

    let redo = history_op("alice", 3, pair.alice.version, &step, HistoryAction::Redo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, redo).expect("the redo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies the redo");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the redo");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), listed, "redo lists again with the same numbering id");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), listed);
}

#[test]
fn a_restart_numbering_undo_lands_as_one_transaction_at_its_author_and_the_peer() {
    let mut pair = FormatPair::new(&["One", "Two"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(1), off: 3 },
    };
    let (listing, _echo) = pair.list(1, "para.numbering", json!({}));
    let source = para_numbering(&pair.mine, 0).expect("a list").num;
    let listed = serde_json::to_value(&pair.mine.doc).expect("json");

    // Restart numbering at the caret of the first paragraph.
    pair.mine.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(0), off: 0 });
    let (step, echo) = pair.list(2, "para.restartNumbering", json!({}));
    let delta = echo.delta.clone().expect("the restart carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_restart_numbering"), "the forward effect keeps the restart op");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    let num = items[0]["num"].as_u64().expect("a restarted list");
    assert_ne!(num, u64::from(source), "a fresh instance");
    assert_eq!(items[1]["num"], json!(num), "the following paragraphs of the list move too");
    assert_eq!(delta["ops"][0]["define"][0]["num"]["id"], json!(num), "the instance travels with the delta");
    let restarted = serde_json::to_value(&pair.mine.doc).expect("json");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), restarted, "the author and the peer hold the same document");
    assert_eq!(para_numbering(&pair.mine, 0), para_numbering(&pair.theirs, 0), "both replicas name the same instance");

    let undo = history_op("alice", 3, pair.alice.version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the undo lands");
    let delta = answer.delta.clone().expect("the undo carries a delta");
    assert_eq!(delta["ops"][0]["op"], json!("set_para_restart_numbering"), "the inverse keeps the restart op");
    assert_eq!(delta["ops"][0]["items"][0]["num"], json!(source), "the earlier instance is back");
    assert_eq!(delta["ops"][0]["undefine"], json!([num]), "the instance the step created is dropped");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), listed, "the author is back at the listed document");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), listed, "the peer is back at the listed document");
    assert_eq!(para_numbering(&pair.mine, 0).map(|n| n.num), Some(source), "the listing step is still on the stack underneath");

    let redo = history_op("alice", 4, pair.alice.version, &step, HistoryAction::Redo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, redo).expect("the redo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies the redo");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the redo");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), restarted, "redo restarts again with the same instance");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), restarted);
    let _ = listing;
}

#[test]
fn a_foreign_edit_of_a_multilevel_paragraph_refuses_the_undo_without_touching_the_document() {
    let mut pair = FormatPair::new(&["One", "Two"]);
    pair.mine.sel = wordcraft_engine::Selection {
        anchor: Pos { story: StoryRef::Body, path: Path::top(0), off: 0 },
        focus: Pos { story: StoryRef::Body, path: Path::top(1), off: 3 },
    };
    let (step, _echo) = pair.list(1, "para.multilevel", json!({}));

    pair.theirs.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(1), off: 0 });
    let before = pair.theirs.doc.clone();
    pair.theirs.run("text.insert", &json!({"text": "X"})).expect("bob types");
    let plan = pair.peer.plan(&before, &pair.theirs, "text.insert", &json!({"text": "X"})).expect("a plan").remove(0);
    let edit = op_from("bob", 1, pair.peer.version, &plan, None);
    let echo = pair.room.submit("bob", Role::Editor, edit).expect("bob's edit lands");
    pair.peer.apply(&echo, &mut pair.theirs).expect("own echo is skipped");
    pair.alice.apply(&echo, &mut pair.mine).expect("alice applies bob's edit");

    let mine_before = serde_json::to_value(&pair.mine.doc).expect("json");
    let theirs_before = serde_json::to_value(&pair.theirs.doc).expect("json");
    let version = pair.alice.version;
    let undo = history_op("alice", 2, version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let err = pair.room.submit("alice", Role::Editor, undo).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), mine_before, "the author's document is unchanged");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), theirs_before, "the peer's document is unchanged");
    assert_eq!(pair.room.version(), version, "the version did not move");
}

// --- deleting blank paragraphs ----------------------------------------------

#[test]
fn a_blank_undo_lands_as_one_transaction_at_its_author_and_the_peer() {
    let mut pair = FormatPair::new(&["One", "", "Two", "", "Three"]);
    let before = serde_json::to_value(&pair.mine.doc).expect("json");

    let (step, echo) = pair.remove_blank(1);
    let delta = echo.delta.clone().expect("the step carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("remove_blank"), "the forward effect is a blank step");
    let items = delta["ops"][0]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "both empty paragraphs are named");
    let blank1 = items[1]["id"].as_u64().expect("an id") as u32;
    assert_eq!(items[1]["index"], json!(1));
    let blank2 = items[0]["id"].as_u64().expect("an id") as u32;
    assert_eq!(items[0]["index"], json!(3));

    let listed = serde_json::to_value(&pair.mine.doc).expect("json");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), listed, "the author and the peer hold the same document");
    assert_eq!(para_texts(&pair.mine), ["One", "Two", "Three"]);
    assert_eq!(para_texts(&pair.theirs), ["One", "Two", "Three"]);

    let undo = history_op("alice", 2, pair.alice.version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, undo).expect("the undo lands");
    let delta = answer.delta.clone().expect("the undo carries a delta");
    assert_eq!(delta["op"], json!("transaction"), "one step, one delta");
    assert_eq!(delta["ops"][0]["op"], json!("remove_blank"), "the inverse names the bodies it puts back");
    assert_eq!(delta["ops"][0]["restore"].as_array().expect("restore").len(), 2);
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies its own inverse");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the inverse");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), before, "the exact earlier paragraphs are back at the author");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), before, "the exact earlier paragraphs are back at the peer");
    pair.alice.set_nodes(&pair.room.nodes());
    pair.peer.set_nodes(&pair.room.nodes());
    for (state, who) in [(&pair.alice, "the author"), (&pair.peer, "the peer")] {
        assert_eq!(state.ids.get(&NodeKey::new("body", vec![1])).map(|id| id.0), Some(blank1), "{who} revived the blank paragraph's id");
        assert_eq!(state.ids.get(&NodeKey::new("body", vec![3])).map(|id| id.0), Some(blank2), "{who} revived the second id");
    }

    let redo = history_op("alice", 3, pair.alice.version, &step, HistoryAction::Redo { op_id: step.op_id.clone() });
    let answer = pair.room.submit("alice", Role::Editor, redo).expect("the redo lands");
    pair.alice.apply(&answer, &mut pair.mine).expect("the author applies the redo");
    pair.peer.apply(&answer, &mut pair.theirs).expect("the peer applies the redo");
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), listed, "redo removes the same paragraphs again");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), listed);
}

#[test]
fn a_foreign_edit_of_a_neighbouring_paragraph_refuses_the_blank_undo_without_touching_the_document() {
    let mut pair = FormatPair::new(&["One", "", "Two", "", "Three"]);
    let (step, _echo) = pair.remove_blank(1);
    assert_eq!(para_texts(&pair.mine), ["One", "Two", "Three"]);

    // Bob types into a paragraph the inverse would put a blank beside.
    pair.theirs.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(1), off: 0 });
    let before = pair.theirs.doc.clone();
    pair.theirs.run("text.insert", &json!({"text": "X"})).expect("bob types");
    let plan = pair.peer.plan(&before, &pair.theirs, "text.insert", &json!({"text": "X"})).expect("a plan").remove(0);
    let edit = op_from("bob", 1, pair.peer.version, &plan, None);
    let echo = pair.room.submit("bob", Role::Editor, edit).expect("bob's edit lands");
    pair.peer.apply(&echo, &mut pair.theirs).expect("own echo is skipped");
    pair.alice.apply(&echo, &mut pair.mine).expect("alice applies bob's edit");

    let mine_before = serde_json::to_value(&pair.mine.doc).expect("json");
    let theirs_before = serde_json::to_value(&pair.theirs.doc).expect("json");
    let version = pair.alice.version;
    let undo = history_op("alice", 2, version, &step, HistoryAction::Undo { op_id: step.op_id.clone() });
    let err = pair.room.submit("alice", Role::Editor, undo).expect_err("the undo is refused");
    assert_eq!(err.code, RejectCode::UndoConflict);
    assert_eq!(serde_json::to_value(&pair.mine.doc).expect("json"), mine_before, "the author's document is unchanged");
    assert_eq!(serde_json::to_value(&pair.theirs.doc).expect("json"), theirs_before, "the peer's document is unchanged");
    assert_eq!(pair.room.version(), version, "the version did not move");
}

/// The refused commit's file arrives in chunks; a chunk from an older answer
/// never joins a newer one, and only `last` releases the copy (TASK.md §6).
#[test]
fn refused_copy_chunks_join_until_last_and_restart_on_a_new_version() {
    let mut buffer = CopyBuffer::default();
    assert!(buffer.push(4, "aa".into(), vec!["kept".into()], b"PK", false).is_none());
    assert!(buffer.push(4, "aa".into(), vec!["kept".into()], b"\x03\x04", false).is_none());
    let copy = buffer.push(4, "aa".into(), vec!["kept".into()], b"body", true).expect("the last chunk completes the copy");
    assert_eq!(copy.version, 4);
    assert_eq!(copy.sha256, "aa");
    assert_eq!(copy.warnings, vec!["kept".to_string()]);
    assert_eq!(copy.bytes, b"PK\x03\x04body");

    // A late chunk of the finished copy does not leak into the next answer.
    assert!(buffer.push(4, "aa".into(), vec![], b"late", false).is_none());
    let next = buffer.push(5, "bb".into(), vec![], b"new", true).expect("a new version is a new copy");
    assert_eq!(next.version, 5);
    assert_eq!(next.bytes, b"new", "the older chunk is dropped when a new version opens the copy");
    assert_eq!(next.sha256, "bb", "the metadata follows the version that opened the copy");
}
