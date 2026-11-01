//! Advanced paragraph tools (#5043): conversion and cleanup over the selection or the whole
//! document, one undo step, Track Changes respected.

use serde_json::json;
use wordcraft_doc::{Path, Pos, RevisionKind, SectionProps, StoryRef};

use crate::Session;

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn run(s: &mut Session, id: &str, v: serde_json::Value) {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

/// Select the whole document (Mod+A's command).
fn select_all(s: &mut Session) {
    run(s, "select.all", json!({}));
}

/// One paragraph with `\n` as manual line breaks, typed as text.
fn type_breaks(s: &mut Session, t: &str) {
    run(s, "text.insert", json!({"text": t}));
}

#[test]
fn line_breaks_become_paragraphs() {
    let mut s = s();
    type_breaks(&mut s, "one\ntwo\nthree");
    assert_eq!(s.doc.para_paths(StoryRef::Body).len(), 1);
    select_all(&mut s);
    run(&mut s, "para.convertBreaks", json!({"scope": "all"}));
    assert_eq!(text(&s), "one\ntwo\nthree", "the breaks are paragraph marks, not text");
    assert_eq!(s.doc.para_paths(StoryRef::Body).len(), 3);
    // One undo step brings the manual line breaks back.
    assert!(s.undo());
    let p0 = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(p0.text, "one\ntwo\nthree", "back to one paragraph with its breaks");
}

#[test]
fn blank_paragraphs_go_in_between() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a\nb\nc"}));
    select_all(&mut s);
    run(&mut s, "para.blankBetween", json!({"scope": "all"}));
    assert_eq!(text(&s), "a\n\nb\n\nc");
}

#[test]
fn blank_paragraphs_go_after_too() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a\nb\nc"}));
    select_all(&mut s);
    run(&mut s, "para.blankBetween", json!({"scope": "all", "after": true}));
    assert_eq!(text(&s), "a\n\nb\n\nc\n");
}

#[test]
fn separate_paragraphs_blanks_after_every_one() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a\nb\nc"}));
    select_all(&mut s);
    run(&mut s, "para.separateParagraphs", json!({"scope": "all"}));
    assert_eq!(text(&s), "a\n\nb\n\nc\n");
}

#[test]
fn blank_paragraphs_are_deleted() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a\n\nb\n\n\nc"}));
    select_all(&mut s);
    run(&mut s, "para.removeBlank", json!({"scope": "all"}));
    assert_eq!(text(&s), "a\nb\nc");
}

#[test]
fn a_section_break_paragraph_stays() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a\nb"}));
    // The second paragraph carries a section break and is emptied: removing it would drop the
    // section, so it stays.
    let second = Path::top(1);
    let p = s.doc.para_mut(StoryRef::Body, &second).unwrap();
    p.text.clear();
    p.runs.clear();
    p.section = Some(Box::new(SectionProps::default()));
    p.touch();
    select_all(&mut s);
    run(&mut s, "para.removeBlank", json!({"scope": "all"}));
    assert_eq!(s.doc.para_paths(StoryRef::Body).len(), 2, "the section-break paragraph stays");
}

#[test]
fn leading_spaces_are_removed() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "  a\nb\n   c"}));
    select_all(&mut s);
    run(&mut s, "para.trimLeadingSpaces", json!({"scope": "all"}));
    assert_eq!(text(&s), "a\nb\nc");
}

#[test]
fn extra_spaces_collapse_to_one() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a  b c   d \n e"}));
    select_all(&mut s);
    run(&mut s, "para.removeExtraSpaces", json!({"scope": "all"}));
    assert_eq!(text(&s), "a b c d \n e", "inner runs collapse; the leading spaces of the second paragraph stay");
}

#[test]
fn a_space_run_at_the_end_collapses() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a  "}));
    select_all(&mut s);
    run(&mut s, "para.removeExtraSpaces", json!({"scope": "all"}));
    assert_eq!(text(&s), "a ");
}

#[test]
fn indented_spaces_become_an_indent() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "   a\nb"}));
    select_all(&mut s);
    run(&mut s, "para.spacesToIndent", json!({"scope": "all"}));
    assert_eq!(text(&s), "a\nb");
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(p.props.indent_first, Some(216.0), "three spaces are 3 x 72 pt");
    assert_eq!(s.doc.para(StoryRef::Body, &Path::top(1)).unwrap().props.indent_first, None, "b has no spaces");
}

#[test]
fn indents_become_leading_spaces() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a"}));
    run(&mut s, "para.indents", json!({"left": 72.0, "firstLine": 144.0}));
    select_all(&mut s);
    run(&mut s, "para.indentToSpaces", json!({"scope": "all"}));
    assert_eq!(text(&s), "   a", "72 + 144 pt become three spaces");
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!((p.props.indent_left, p.props.indent_first), (Some(0.0), Some(0.0)));
}

#[test]
fn line_breaks_are_removed() {
    let mut s = s();
    type_breaks(&mut s, "one\ntwo\nthree");
    select_all(&mut s);
    run(&mut s, "para.removeLineBreaks", json!({"scope": "all"}));
    assert_eq!(text(&s), "onetwothree");
}

#[test]
fn the_selection_is_the_scope() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a  b\nc  d"}));
    // Only the first paragraph: select from its start to its end.
    s.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 4) };
    run(&mut s, "para.removeExtraSpaces", json!({}));
    assert_eq!(text(&s), "a b\nc  d", "the second paragraph is untouched");
}

#[test]
fn deletions_are_recorded_under_track_changes() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a  b\n c"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    select_all(&mut s);
    run(&mut s, "para.trimLeadingSpaces", json!({"scope": "all"}));
    // The text stays marked deleted, not gone, as Word records the deletion.
    assert_eq!(text(&s), "a  b\n c", "tracked deletions keep their text");
    assert!(s.doc.revisions.iter().any(|r| r.kind == RevisionKind::Delete));
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "a  b\nc", "accepting removes the leading space");
}

#[test]
fn junk_scope_is_refused() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a b"}));
    select_all(&mut s);
    let e = s.run("para.removeExtraSpaces", &json!({"scope": "junk"})).unwrap_err();
    assert!(format!("{e}").contains("scope"), "{e}");
    assert_eq!(text(&s), "a b", "the document is unchanged");
}

#[test]
fn too_many_paragraphs_are_refused_before_any_change() {
    let text = "x\n".repeat(15_000);
    let mut s = Session::new(wordcraft_doc::Document::from_text(&text));
    select_all(&mut s);
    let e = s.run("para.blankBetween", &json!({"scope": "all"})).unwrap_err();
    assert!(format!("{e}").contains("at most"), "{e}");
    let kept = s.doc.para_paths(StoryRef::Body).len();
    assert!((15_000..=15_001).contains(&kept), "nothing changed, {kept}");
}
