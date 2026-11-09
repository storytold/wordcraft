//! Sync-state tests: no socket, just the room, the document and the maps.

use std::sync::Arc;

use serde_json::json;

use wordcraft_collab::ids::NodeKey;
use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{Applied, HistoryAction, NodeRef, Op, OpId, OpKind, Reject, RejectCode, Target};
use wordcraft_collab_doc::DocModel;
use wordcraft_collab_engine::SessionModel;
use wordcraft_doc::{Block, Cell, CharProps, Document, Paragraph, Path, Pos, StoryRef, Table};
use wordcraft_engine::Session;

use crate::{Outgoing, Status, SyncState};

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
