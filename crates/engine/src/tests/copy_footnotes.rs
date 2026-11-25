//! Copy, Cut and Paste keep footnotes (#6713): the copy brings the notes along and a paste makes
//! notes of its own, in one undo step.

use serde_json::json;
use wordcraft_doc::{InlineObject, Path, Pos, StoryRef};

use crate::{Selection, Session};

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// "Alpha" + a footnote reference + " beta gamma"; returns the session and the note's story id.
fn with_note() -> (Session, u32) {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Alpha beta gamma"}));
    s.sel = Selection::caret(Pos::body(0, 5));
    let id = run(&mut s, "references.footnote", json!({"text": "first note"}))["id"].as_u64().unwrap() as u32;
    (s, id)
}

/// The note stories the body's references name, in reading order.
fn body_notes(s: &Session) -> Vec<u32> {
    let mut out = Vec::new();
    s.doc.objects_in_reading_order(&mut |o| {
        if let InlineObject::NoteRef { id, .. } = o {
            out.push(*id);
        }
    });
    out
}

fn note_text(s: &Session, id: u32) -> String {
    s.doc.plain_text(StoryRef::Part(id))
}

fn select_all_body(s: &mut Session) {
    let end = s.doc.end_of(StoryRef::Body);
    s.sel = Selection { anchor: Pos::body(0, 0), focus: end };
}

#[test]
fn copy_and_paste_keep_the_footnote_and_undo_removes_it() {
    let (mut s, a) = with_note();
    select_all_body(&mut s);
    run(&mut s, "edit.copy", json!({}));
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    let parts_before = s.doc.parts.len();
    run(&mut s, "edit.paste", json!({}));
    let ids = body_notes(&s);
    assert_eq!(ids.len(), 2, "the pasted text keeps its footnote reference");
    assert_eq!(ids[0], a);
    assert_ne!(ids[1], a, "the pasted reference has a note of its own");
    assert!(note_text(&s, ids[1]).contains("first note"), "and the note's text");
    assert!(note_text(&s, a).contains("first note"));
    // One undo takes the paste, its reference and its note back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(body_notes(&s), vec![a]);
    assert_eq!(s.doc.parts.len(), parts_before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(body_notes(&s).len(), 2);
}

#[test]
fn cut_and_paste_move_the_footnote() {
    let (mut s, a) = with_note();
    select_all_body(&mut s);
    run(&mut s, "edit.cut", json!({}));
    assert!(body_notes(&s).is_empty());
    let parts_before = s.doc.parts.len();
    run(&mut s, "edit.paste", json!({}));
    let ids = body_notes(&s);
    assert_eq!(ids, vec![a], "the footnote came back with the text, on its own story");
    assert!(note_text(&s, a).contains("first note"));
    assert_eq!(s.doc.parts.len(), parts_before, "no second, unreferenced copy is left behind");
}

#[test]
fn pasting_the_same_copy_twice_makes_two_notes() {
    let (mut s, _) = with_note();
    select_all_body(&mut s);
    run(&mut s, "edit.copy", json!({}));
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    run(&mut s, "edit.paste", json!({}));
    run(&mut s, "edit.paste", json!({}));
    let ids = body_notes(&s);
    assert_eq!(ids.len(), 3);
    assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2], "every reference has its own note: {ids:?}");
    assert!(ids.iter().all(|id| note_text(&s, *id).contains("first note")));
}

#[test]
fn a_selection_cutting_through_the_reference_copies_no_note() {
    let (mut s, _) = with_note();
    // Only "Alpha": the reference is the next character.
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 5) };
    run(&mut s, "edit.copy", json!({}));
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    let parts_before = s.doc.parts.len();
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(body_notes(&s).len(), 1);
    assert_eq!(s.doc.parts.len(), parts_before, "no note was made");
}

#[test]
fn copy_with_a_note_pasted_under_track_changes_marks_the_note_inserted() {
    let (mut s, a) = with_note();
    select_all_body(&mut s);
    run(&mut s, "edit.copy", json!({}));
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "edit.paste", json!({}));
    let ids = body_notes(&s);
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[1], a);
    let note = s.doc.para(StoryRef::Part(ids[1]), &Path::top(0)).unwrap();
    assert!(!note.runs.is_empty() && note.runs.iter().all(|r| r.props.ins.is_some()), "the new note's text is a tracked insertion");
    let old = s.doc.para(StoryRef::Part(a), &Path::top(0)).unwrap();
    assert!(old.runs.iter().all(|r| r.props.ins.is_none()), "the original note is untouched");
}

#[test]
fn note_pasted_into_another_document_keeps_no_revision_of_the_source() {
    let (mut s, a) = with_note();
    s.doc.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "Source", ""));
    let deleted = s.doc.para(StoryRef::Part(a), &Path::top(0)).unwrap().text.find("first ").unwrap();
    s.doc.para_mut(StoryRef::Part(a), &Path::top(0)).unwrap().format(deleted, deleted + "first ".len(), &|c| c.del = Some(0)).unwrap();
    select_all_body(&mut s);
    run(&mut s, "edit.copy", json!({}));
    let mut other = wordcraft_doc::Document::new();
    other.revisions.push(wordcraft_doc::Revision::new(wordcraft_doc::RevisionKind::Delete, "Unrelated", ""));
    s.add_document(other);
    run(&mut s, "review.trackChanges", json!({"value": true}));

    run(&mut s, "edit.paste", json!({}));

    let ids = body_notes(&s);
    assert_eq!(ids.len(), 1);
    let note = s.doc.para(StoryRef::Part(ids[0]), &Path::top(0)).unwrap();
    assert!(note.runs.iter().all(|r| r.props.del.is_none() && r.props.ins != Some(0)), "no run binds to the destination's revision 0");
    assert!(note.text.contains("note") && !note.text.contains("first"), "the note pastes as it reads after review: {:?}", note.text);
}

#[test]
fn copying_inside_a_note_makes_no_note_when_pasted() {
    let (mut s, a) = with_note();
    let story = StoryRef::Part(a);
    s.sel = Selection { anchor: Pos { story, path: Path::top(0), off: 0 }, focus: s.doc.end_of(story) };
    run(&mut s, "edit.copy", json!({}));
    s.sel = Selection::caret(s.doc.end_of(StoryRef::Body));
    let parts_before = s.doc.parts.len();
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.doc.parts.len(), parts_before);
    assert_eq!(body_notes(&s), vec![a], "and no reference to the note it came from");
}
