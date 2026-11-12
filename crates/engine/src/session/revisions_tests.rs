//! #6188: edit boundaries, not clock seconds, determine tracked revision identity.

use super::{Selection, Session};
use serde_json::{Value, json};
use wordcraft_doc::{Document, Pos, RevisionKind};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn tracked(text: &str) -> Session {
    let mut s = Session::new(Document::from_text(text));
    s.author = "Alice".into();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s
}

fn insert(s: &mut Session, text: &str) {
    run(s, "text.insert", json!({"text": text, "raw": true}));
}

fn ids(s: &Session, kind: RevisionKind) -> Vec<u32> {
    s.doc.revisions.iter().enumerate().filter_map(|(i, r)| (r.kind == kind).then_some(i as u32)).collect()
}

fn text(s: &Session) -> String {
    s.doc.para_at(&Pos::body(0, 0)).unwrap().text.clone()
}

#[test]
fn continuous_revision_deletions_ignore_clock_seconds_and_keep_direction_boundaries() {
    for command in ["text.backspace", "text.delete"] {
        let mut s = tracked("abc中文");
        s.sel = Selection::caret(Pos::body(0, if command == "text.backspace" { "abc中文".len() } else { 0 }));
        run(&mut s, command, json!({}));
        let date = "2000-01-01T00:00:00Z".to_string();
        s.doc.revisions[0].date = date.clone();
        run(&mut s, command, json!({}));
        run(&mut s, command, json!({}));
        assert_eq!(ids(&s, RevisionKind::Delete), [0], "{command}");
        assert_eq!(s.doc.revisions[0].date, date);
        run(&mut s, "review.rejectAll", json!({}));
        assert_eq!(text(&s), "abc中文");
    }
    let mut s = tracked("abc");
    s.sel = Selection::caret(Pos::body(0, 1));
    run(&mut s, "text.backspace", json!({}));
    // The tracked deletion remains in the model. Delete the next live character, rather
    // than requesting a no-op deletion of the character already marked by Backspace.
    s.sel = Selection::caret(Pos::body(0, 1));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(ids(&s, RevisionKind::Delete).len(), 2);
}

#[test]
fn continuous_revision_boundaries_include_author_target_command_and_explicit_close() {
    let mut s = tracked("abc");
    insert(&mut s, "甲");
    s.author = "Bob".into();
    insert(&mut s, "乙");
    s.author = "Alice".into();
    insert(&mut s, "丙");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 3);
    assert_eq!(s.doc.revisions.iter().map(|r| r.author.as_str()).collect::<Vec<_>>(), ["Alice", "Bob", "Alice"]);
    s.sel = Selection::caret(Pos::body(0, 0));
    insert(&mut s, "丁");
    run(&mut s, "format.bold", json!({}));
    insert(&mut s, "戊");
    s.close_typing();
    insert(&mut s, "己");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 6);
}

#[test]
fn continuous_revision_caret_away_and_back_ends_the_group() {
    let mut s = tracked("");
    insert(&mut s, "abc");
    run(&mut s, "caret.left", json!({}));
    run(&mut s, "caret.right", json!({}));
    insert(&mut s, "d");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 2);
    run(&mut s, "text.newParagraph", json!({}));
    insert(&mut s, "next");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 4);
}

#[test]
fn continuous_revision_query_caret_movement_ends_the_group() {
    let mut s = tracked("");
    insert(&mut s, "abc");
    run(&mut s, "edit.goto", json!({"paragraph": 1}));
    assert_eq!(s.sel, Selection::caret(Pos::body(0, 0)));
    insert(&mut s, "def");
    assert_eq!(ids(&s, RevisionKind::Insert), [0, 1]);
    assert_eq!(text(&s), "defabc");
    // Resolving the second identity must leave the first insertion untouched.
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 3) };
    run(&mut s, "review.accept", json!({}));
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(text(&s), "def");
}

#[test]
fn continuous_revision_queries_without_caret_movement_keep_the_group() {
    let mut s = tracked("");
    insert(&mut s, "first");
    let before = s.sel.clone();
    run(&mut s, "edit.goto", json!({}));
    run(&mut s, "review.changes", json!({}));
    assert_eq!(s.sel, before);
    insert(&mut s, "second");
    assert_eq!(ids(&s, RevisionKind::Insert), [0]);
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(text(&s), "");
}

#[test]
fn continuous_revision_allocations_outside_dispatch_end_the_group() {
    for kind in [RevisionKind::Insert, RevisionKind::Delete] {
        let mut s = tracked("abc");
        if kind == RevisionKind::Insert {
            insert(&mut s, "first");
        } else {
            s.sel = Selection::caret(Pos::body(0, 3));
            run(&mut s, "text.backspace", json!({}));
        }
        let before = s.sel.clone();
        assert_eq!(crate::cmd::new_revision(&mut s, kind), 1);
        assert_eq!(crate::cmd::new_revision(&mut s, kind), 2);
        assert_eq!(s.sel, before);
        if kind == RevisionKind::Insert {
            insert(&mut s, "second");
        } else {
            run(&mut s, "text.backspace", json!({}));
        }
        assert_eq!(ids(&s, kind), [0, 1, 2, 3]);
    }
}

#[test]
fn continuous_revision_undo_redo_restore_and_document_switch_end_the_group() {
    let mut s = tracked("");
    insert(&mut s, "first");
    let snap = s.edit_snapshot();
    insert(&mut s, " more");
    s.restore(snap);
    insert(&mut s, " second");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 2);
    assert!(s.undo());
    assert!(s.redo());
    insert(&mut s, " third");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 3);
    let home = s.documents()[0].id;
    let other = s.add_document(Document::from_text("other"));
    s.switch_document(other).unwrap();
    s.switch_document(home).unwrap();
    insert(&mut s, " fourth");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 4);
    s.doc = Document::from_text("replacement");
    s.doc.settings.track_changes = true;
    s.note_body_replaced();
    s.sel = Selection::caret(Pos::body(0, 0));
    insert(&mut s, "new ");
    assert_eq!(ids(&s, RevisionKind::Insert), [0]);
    assert_eq!(text(&s), "new replacement");
}

#[test]
fn continuous_revision_failed_nested_command_rolls_back_and_forgets_allocation() {
    fn fail(s: &mut Session, _: &Value) -> crate::CmdResult {
        s.run("text.insert", &json!({"text": "discarded", "raw": true}))?;
        Err(crate::CmdError::Failed("synthetic rollback".into()))
    }
    let mut specs = crate::cmd::text::specs();
    specs.extend(crate::cmd::review::specs());
    specs.push(crate::CommandSpec::new("test.fail", "Failed edit", "Test", fail));
    let mut s = Session::with_registry(Document::new(), std::sync::Arc::new(crate::Registry::new(specs)));
    s.doc.settings.track_changes = true;
    insert(&mut s, "first");
    let before = s.doc.clone();
    assert!(s.run("test.fail", &json!({})).is_err());
    assert_eq!(s.doc, before);
    insert(&mut s, "second");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 2);
    assert_eq!(text(&s), "firstsecond");
}

#[test]
fn continuous_revision_selection_replacement_and_targeted_review_keep_other_edits() {
    let mut s = tracked("original");
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, "original".len()) };
    insert(&mut s, "replacement");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 1);
    assert_eq!(ids(&s, RevisionKind::Delete).len(), 1);
    insert(&mut s, " continued");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 2);
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(text(&s), "original");

    let mut s = tracked("");
    insert(&mut s, "Alice");
    s.author = "Bob".into();
    insert(&mut s, "Bob");
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 5) };
    run(&mut s, "review.accept", json!({}));
    assert_eq!(text(&s), "AliceBob");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(text(&s), "Alice");
}

#[test]
fn continuous_revision_json_and_docx_destroy_reopen_edit_save() {
    for docx in [false, true] {
        let mut s = tracked("");
        insert(&mut s, "是吗");
        s.doc.revisions[0].date = "2000-01-01T00:00:00Z".into();
        insert(&mut s, "？我总觉得有点问题");
        let expected = text(&s);
        let date = s.doc.revisions[0].date.clone();
        let bytes = if docx { wordcraft_docx::write(&s.doc).unwrap() } else { serde_json::to_vec(&s.doc).unwrap() };
        drop(s);
        let doc = if docx { wordcraft_docx::read(&bytes).unwrap() } else { serde_json::from_slice(&bytes).unwrap() };
        let mut reopened = Session::new(doc);
        reopened.author = "Alice".into();
        assert_eq!(text(&reopened), expected);
        assert_eq!(ids(&reopened, RevisionKind::Insert).len(), 1);
        assert_eq!(reopened.doc.revisions[0].author, "Alice");
        assert_eq!(reopened.doc.revisions[0].date, date);
        assert!(reopened.doc.settings.track_changes);
        reopened.sel = Selection::caret(Pos::body(0, expected.len()));
        insert(&mut reopened, "，继续");
        insert(&mut reopened, "编辑");
        assert_eq!(ids(&reopened, RevisionKind::Insert).len(), 2);
        let expected = text(&reopened);
        let bytes = if docx { wordcraft_docx::write(&reopened.doc).unwrap() } else { serde_json::to_vec(&reopened.doc).unwrap() };
        drop(reopened);
        let doc = if docx { wordcraft_docx::read(&bytes).unwrap() } else { serde_json::from_slice(&bytes).unwrap() };
        let mut again = Session::new(doc);
        assert_eq!(text(&again), expected);
        assert_eq!(ids(&again, RevisionKind::Insert).len(), 2);
        run(&mut again, "review.rejectAll", json!({}));
        assert_eq!(text(&again), "");
    }
}

#[test]
fn continuous_revision_hidden_markup_records_but_disabled_tracking_does_not() {
    let mut s = tracked("");
    run(&mut s, "review.markup", json!({"mode": "noMarkup"}));
    insert(&mut s, "tracked");
    insert(&mut s, " text");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 1);
    run(&mut s, "review.trackChanges", json!({"value": false}));
    insert(&mut s, " ordinary");
    insert(&mut s, " text");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 1);
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(text(&s), " ordinary text");
}

#[test]
fn continuous_revision_tracking_protection_and_nested_insertion_allocate_once() {
    let mut s = tracked("");
    s.doc.settings.track_changes = false;
    s.doc.settings.protection = Some("trackedChanges".into());
    insert(&mut s, "one");
    insert(&mut s, "two");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 1);
    assert_eq!(text(&s), "onetwo");

    fn nested(s: &mut Session, _: &Value) -> crate::CmdResult {
        s.run("text.insert", &json!({"text": "one", "raw": true}))?;
        s.run("text.insert", &json!({"text": "two", "raw": true}))
    }
    let mut specs = crate::cmd::text::specs();
    specs.push(crate::CommandSpec::new("test.nested", "Nested edit", "Test", nested));
    let mut s = Session::with_registry(Document::new(), std::sync::Arc::new(crate::Registry::new(specs)));
    s.doc.settings.track_changes = true;
    run(&mut s, "test.nested", json!({}));
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 1);
    assert_eq!(text(&s), "onetwo");
    insert(&mut s, "three");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 2);
}

#[test]
fn continuous_revision_missing_or_reclassified_id_is_not_reused() {
    let mut s = tracked("");
    insert(&mut s, "one");
    // Hosts must signal body replacements, but even a stale slot cannot refer to a
    // removed revision or to an ID whose author/kind or move role has changed.
    s.doc.revisions[0].move_name = Some("move".into());
    insert(&mut s, "two");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 2);
    s.doc.revisions[1].author = "Bob".into();
    insert(&mut s, "three");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 3);
    s.doc.revisions[2].kind = RevisionKind::Delete;
    insert(&mut s, "four");
    assert_eq!(ids(&s, RevisionKind::Insert).len(), 3);
}
