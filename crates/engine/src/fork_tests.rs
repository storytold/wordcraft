//! Engine behaviour the chat add-in relies on (rejected member commands are rolled
//! back completely; tracked edits never rewrite another author's revision marks).

use serde_json::json;
use wordcraft_doc::{Pos, StoryRef};

use crate::Session;

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn author_of(s: &Session, rid: Option<u32>) -> Option<String> {
    rid.and_then(|r| s.doc.revisions.get(r as usize)).map(|r| r.author.clone())
}

#[test]
fn restore_undoes_a_command_completely() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Alfa beta"}));
    run(&mut s, "text.insert", json!({"text": " gama"}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.can_redo());
    let labels = s.undo_labels();
    let doc = s.doc.clone();
    let sel = s.sel.clone();
    let rev = s.rev();
    s.dirty = false;
    let snap = s.edit_snapshot();
    assert_eq!(snap.doc(), &doc);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    assert_ne!(s.doc, doc);
    assert!(!s.can_redo(), "a new command clears redo");
    s.restore(snap);
    assert_eq!(s.doc, doc);
    assert_eq!(s.sel, sel);
    assert_eq!(s.undo_labels(), labels, "the rejected command's undo step is gone");
    assert!(s.can_redo(), "the owner's redo is back");
    assert!(!s.dirty);
    assert!(s.rev() > rev, "layout must be recomputed");
}

#[test]
fn tracked_delete_keeps_another_authors_deletion() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Alfa beta gama"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Owner".into();
    run(&mut s, "select.text", json!({"text": "beta"}));
    run(&mut s, "text.delete", json!({}));
    s.author = "@claude".into();
    run(&mut s, "select.text", json!({"text": "Alfa beta gama"}));
    run(&mut s, "text.delete", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).cloned().unwrap_or_default();
    let mut by: Vec<(String, Option<String>)> = Vec::new();
    for (r, c) in p.run_ranges() {
        by.push((p.text[r].to_string(), author_of(&s, c.del)));
    }
    assert!(by.iter().any(|(t, a)| t == "beta" && a.as_deref() == Some("Owner")), "{by:?}");
    assert!(by.iter().any(|(t, a)| t.contains("Alfa") && a.as_deref() == Some("@claude")), "{by:?}");
}

#[test]
fn tracked_split_inside_another_authors_insertion_keeps_the_original_mark() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Alfa"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Owner".into();
    run(&mut s, "text.insert", json!({"text": " beta"}));
    s.author = "@claude".into();
    run(&mut s, "text.newParagraph", json!({}));
    let head = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).cloned().unwrap_or_default();
    let tail = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(1)).cloned().unwrap_or_default();
    assert_eq!(author_of(&s, head.mark.ins).as_deref(), Some("@claude"));
    assert_eq!(tail.mark.ins, None, "the original paragraph mark was not inserted by anyone");
    assert_eq!(tail.mark.del, None);
}

#[test]
fn restore_is_exact_at_a_full_undo_stack() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Alfa"}));
    for i in 0..510 {
        run(&mut s, if i % 2 == 0 { "para.alignCenter" } else { "para.alignLeft" }, json!({}));
    }
    let labels = s.undo_labels();
    assert_eq!(labels.len(), 500);
    let snap = s.edit_snapshot();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    assert_eq!(s.undo_labels().first().map(String::as_str), Some("Table"));
    s.restore(snap);
    assert_eq!(s.undo_labels(), labels, "the oldest owner step is back and the refused step is gone");
    // Undo still walks the owner's own steps, down to the oldest one.
    let mut n = 0;
    while s.undo() {
        n += 1;
    }
    assert_eq!(n, 500);
}

#[test]
fn version_restore_keeps_the_document_identity() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "Alfa"}));
    run(&mut s, "file.versions", json!({"save": "v1"}));
    run(&mut s, "text.insert", json!({"text": " beta"}));
    let (generation, replaced) = (s.doc_generation, s.doc_replaced);
    run(&mut s, "file.versions", json!({"restore": 0}));
    assert_eq!(s.doc_generation, generation, "same document: the chat must not switch");
    assert!(s.doc_replaced > replaced, "saved positions are no longer valid");
    s.set_document(wordcraft_doc::Document::new());
    assert!(s.doc_generation > generation);
}
