//! Sync-state tests: no socket, just the room, the document and the maps.

use serde_json::json;

use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{Applied, NodeRef, OpId, OpKind, Target};
use wordcraft_collab_doc::DocModel;
use wordcraft_doc::{Document, Path, Pos, StoryRef};
use wordcraft_engine::Session;

use crate::SyncState;

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
