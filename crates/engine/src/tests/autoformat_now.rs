//! AutoFormat Now (Word's Format › AutoFormat, Alt+Ctrl+K): the AutoFormat-as-you-type replacements
//! applied to text that is already in the document, in one undo step.

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{CharProps, ListKind, Path, Pos, StoryRef};

use crate::{Selection, Session, cmd};

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn texts(s: &Session) -> Vec<String> {
    s.doc.para_paths(StoryRef::Body).iter().map(|p| s.doc.para(StoryRef::Body, p).unwrap().text.clone()).collect()
}

#[cfg(not(target_os = "macos"))]
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

// ---- PR #5893 review ----------------------------------------------------------------------

/// The paragraphs' text without their tracked deletions.
fn visible(s: &Session) -> Vec<String> {
    s.doc
        .para_paths(StoryRef::Body)
        .iter()
        .map(|path| {
            let p = s.doc.para(StoryRef::Body, path).unwrap();
            p.run_ranges().filter(|(_, c)| c.del.is_none()).filter_map(|(r, _)| p.text.get(r)).collect::<String>().replace(OBJ, "")
        })
        .collect()
}

/// The text of paragraph `i` that is marked as a tracked deletion.
fn deleted(s: &Session, i: usize) -> String {
    let p = s.doc.para(StoryRef::Body, &Path::top(i)).unwrap();
    p.run_ranges().filter(|(_, c)| c.del.is_some()).filter_map(|(r, _)| p.text.get(r)).collect()
}

fn numbered(s: &Session, i: usize) -> Option<u32> {
    let p = s.doc.para(StoryRef::Body, &Path::top(i)).unwrap();
    s.doc.styles.resolve_para(&p.props).numbering.filter(|n| n.num != 0).map(|n| n.num)
}

#[test]
fn tracked_autoformat_now_rejects_back_to_the_original_text() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("He said \"hi\"\nA -- B\n* apples"));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(visible(&s), ["He said \u{201C}hi\u{201D}", "A \u{2013} B", "apples"]);
    assert!(!s.doc.revisions.is_empty(), "the rewrite is tracked");
    assert_eq!(deleted(&s, 0), "\"\"", "the straight quotes stay, deleted");
    assert_eq!(deleted(&s, 1), "--");
    assert!(numbered(&s, 2).is_some());
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(texts(&s), ["He said \"hi\"", "A -- B", "* apples"]);
    assert!(numbered(&s, 2).is_none(), "Reject All takes the list back");
    assert!(s.doc.revisions.is_empty());
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "format.autoFormatNow", json!({}));
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(texts(&s), ["He said \u{201C}hi\u{201D}", "A \u{2013} B", "apples"]);
    assert!(numbered(&s, 2).is_some());
}

#[test]
fn tracked_changes_protection_records_the_rewrite() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("say \"hi\""));
    run(&mut s, "review.restrict", json!({"mode": "trackedChanges", "password": "pw"}));
    s.doc.settings.track_changes = false;
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(visible(&s), ["say \u{201C}hi\u{201D}"]);
    assert_eq!(deleted(&s, 0), "\"\"", "protection forces tracking: the old quotes stay as deletions");
}

#[test]
fn another_authors_insertion_becomes_a_deletion_and_an_insertion() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "file.setAuthor", json!({"name": "Ana"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": "say \"hi\"", "raw": true}));
    assert_eq!(texts(&s), ["say \"hi\""], "typed raw: no AutoFormat as you type");
    run(&mut s, "file.setAuthor", json!({"name": "Ben"}));
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(visible(&s), ["say \u{201C}hi\u{201D}"]);
    assert_eq!(deleted(&s, 0), "\"\"", "Ana's quotes are marked deleted, not dropped");
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    let ben = |c: &CharProps| c.ins.and_then(|i| s.doc.revisions.get(i as usize)).is_some_and(|r| r.author == "Ben");
    assert!(p.run_ranges().any(|(r, c)| &p.text[r] == "\u{201C}" && ben(c)), "Ben inserted the curly quote");
}

#[test]
fn own_tracked_insertion_is_rewritten_in_place() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": "say \"hi\"", "raw": true}));
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s), ["say \u{201C}hi\u{201D}"]);
    assert_eq!(deleted(&s, 0), "", "the author's own insertion stays one insertion");
}

/// `"a" "b"` with `"b"` in a rich text control tagged `ctl`, locked with `lock`.
fn quotes_with_control(lock: &str) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "\"a\" \"b\"", "raw": true}));
    s.sel = Selection { anchor: Pos::body(0, 4), focus: Pos::body(0, 7) };
    run(&mut s, "developer.richText", json!({"tag": "ctl"}));
    run(&mut s, "developer.properties", json!({"control": "ctl", lock: true}));
    run(&mut s, "caret.docStart", json!({}));
    s
}

#[test]
fn content_locked_against_editing_is_left_alone() {
    let mut s = quotes_with_control("lockEdit");
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(visible(&s), ["\u{201C}a\u{201D} \"b\""], "only the text outside the locked control changes");
    // Locked against deletion only: the content may change, as with Change Case.
    let mut s = quotes_with_control("lockDelete");
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(visible(&s), ["\u{201C}a\u{201D} \u{201C}b\u{201D}"]);
}

#[test]
fn labels_in_a_field_result_stay_text() {
    // A TOC-like field whose result spans paragraphs: "1. Intro" and "2. Body" are its result.
    let mut doc = wordcraft_doc::Document::from_text("x\n1. Intro\n2. Body\n1. real list");
    let c = CharProps::default();
    let p = doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.insert_object(1, InlineObject::FieldStart { instr: "TOC \\o \"1-3\"".into(), locked: false, dirty: false }, &c).unwrap();
    let p = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    let end = p.len();
    p.insert_object(end, InlineObject::FieldEnd, &c).unwrap();
    let mut s = Session::new(doc);
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "format.autoFormatNow", json!({}));
    let shown = visible(&s);
    assert_eq!(shown[1], "1. Intro", "field result keeps its label");
    assert_eq!(shown[2], "2. Body");
    assert!(numbered(&s, 1).is_none() && numbered(&s, 2).is_none());
    assert_eq!(shown[3], "real list");
    assert!(numbered(&s, 3).is_some());
}

#[test]
fn a_label_with_a_tracked_deletion_is_not_a_label() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("1. first"));
    run(&mut s, "file.setAuthor", json!({"name": "Ana"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.sel = Selection { anchor: Pos::body(0, 1), focus: Pos::body(0, 2) };
    run(&mut s, "text.delete", json!({}));
    assert_eq!(deleted(&s, 0), ".");
    run(&mut s, "review.trackChanges", json!({"value": false}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "format.autoFormatNow", json!({}));
    assert!(numbered(&s, 0).is_none(), "\"1 first\" as read has no label");
    assert_eq!(texts(&s), ["1. first"]);
}

#[test]
fn dash_rules() {
    for (before, after) in [
        ("A - B", "A \u{2013} B"),
        ("A -- B", "A \u{2013} B"),
        ("well--done", "well\u{2014}done"),
        ("a---b", "a---b"),
        ("re-use", "re-use"),
        ("end -", "end -"),
    ] {
        let mut s = Session::new(wordcraft_doc::Document::from_text(before));
        run(&mut s, "caret.docStart", json!({}));
        run(&mut s, "format.autoFormatNow", json!({}));
        assert_eq!(texts(&s), [after], "{before:?}");
    }
}

#[test]
fn each_switch_alone() {
    let fresh = || {
        let mut s = Session::new(wordcraft_doc::Document::from_text("* a\n1. b\nx -- y \"z\""));
        run(&mut s, "caret.docStart", json!({}));
        s
    };
    let mut s = fresh();
    s.prefs.autocorrect.bullets = false;
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s), ["* a", "b", "x \u{2013} y \u{201C}z\u{201D}"]);
    assert!(numbered(&s, 0).is_none() && numbered(&s, 1).is_some());

    let mut s = fresh();
    s.prefs.autocorrect.numbering = false;
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s), ["a", "1. b", "x \u{2013} y \u{201C}z\u{201D}"]);
    assert!(numbered(&s, 0).is_some() && numbered(&s, 1).is_none());

    let mut s = fresh();
    s.prefs.autocorrect.smart_quotes = false;
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s)[2], "x \u{2013} y \"z\"");

    let mut s = fresh();
    s.prefs.autocorrect.dashes = false;
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s)[2], "x -- y \u{201C}z\u{201D}");
}

#[test]
fn every_label_gives_its_list_kind() {
    for (label, kind) in [
        ("* ", ListKind::Bullet),
        ("- ", ListKind::BulletChar('-')),
        ("> ", ListKind::BulletChar('\u{27A2}')),
        ("1. ", ListKind::Numbered),
        ("1) ", ListKind::NumberedParen),
    ] {
        let mut s = Session::new(wordcraft_doc::Document::from_text(&format!("{label}item\n{label}\n{label}  ")));
        run(&mut s, "caret.docStart", json!({}));
        run(&mut s, "format.autoFormatNow", json!({}));
        assert_eq!(texts(&s)[0], "item", "{label:?}");
        assert_eq!(numbered(&s, 0), s.doc.numbering.find_kind(kind), "{label:?}");
        // A label with nothing after it stays text.
        assert_eq!(texts(&s)[1..], [label.to_string(), format!("{label}  ")], "{label:?}");
        assert!(numbered(&s, 1).is_none() && numbered(&s, 2).is_none(), "{label:?}");
    }
}

#[test]
fn a_selection_across_two_paragraphs_changes_both_and_no_more() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("\"one\"\n\"two\"\n\"three\""));
    s.sel = Selection { anchor: Pos::body(0, 2), focus: Pos::body(1, 2) };
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(texts(&s), ["\u{201C}one\u{201D}", "\u{201C}two\u{201D}", "\"three\""]);
}

#[test]
fn undo_takes_a_list_conversion_back_whole() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("* apples\n1. first"));
    let numbering = s.doc.numbering.clone();
    run(&mut s, "caret.docStart", json!({}));
    let depth = s.undo_depth();
    run(&mut s, "format.autoFormatNow", json!({}));
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(texts(&s), ["* apples", "1. first"]);
    assert!(numbered(&s, 0).is_none() && numbered(&s, 1).is_none());
    assert_eq!(s.doc.numbering, numbering, "the list definitions it added are gone");
}
