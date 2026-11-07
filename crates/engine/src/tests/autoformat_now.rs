//! AutoFormat Now (Word's Format › AutoFormat, Alt+Ctrl+K): the AutoFormat-as-you-type replacements
//! applied to text that is already in the document, in one undo step.

use serde_json::{Value, json};
use wordcraft_doc::StoryRef;

use crate::{Session, cmd};

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn texts(s: &Session) -> Vec<String> {
    s.doc.para_paths(StoryRef::Body).iter().map(|p| s.doc.para(StoryRef::Body, p).unwrap().text.clone()).collect()
}

#[test]
fn ctrl_alt_k_runs_autoformat_now() {
    assert_eq!(cmd::registry().by_shortcut("Ctrl+Alt+K").map(|c| c.id), Some("format.autoFormatNow"));
}

#[test]
fn quotes_and_dashes_in_existing_text_become_typographic() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("He said \"hi\" and 'bye'\nwell--done\nA -- B"));
    run(&mut s, "caret.docStart", json!({}));
    let depth = s.undo_depth();
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s), ["He said \u{201C}hi\u{201D} and \u{2018}bye\u{2019}", "well\u{2014}done", "A \u{2013} B"]);
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(texts(&s), ["He said \"hi\" and 'bye'", "well--done", "A -- B"]);
}

#[test]
fn list_markers_typed_as_text_become_list_items() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("* apples\n1. first\nplain"));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s), ["apples", "first", "plain"]);
    let paths = s.doc.para_paths(StoryRef::Body);
    let num = |i: usize| s.doc.styles.resolve_para(&s.doc.para(StoryRef::Body, &paths[i]).unwrap().props).numbering.is_some_and(|n| n.num != 0);
    assert!(num(0) && num(1) && !num(2));
}

#[test]
fn only_the_selected_paragraph_changes() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("\"one\"\n\"two\""));
    s.run("select.text", &json!({"text": "\"two\""})).unwrap();
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s), ["\"one\"", "\u{201C}two\u{201D}"]);
}
