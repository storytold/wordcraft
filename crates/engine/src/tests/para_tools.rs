//! Advanced paragraph tools (#5043): conversion and cleanup over the selection or the whole
//! document, one undo step, Track Changes respected.

use serde_json::json;
use wordcraft_doc::{InlineObject, Path, Pos, RevisionKind, SectionProps, StoryRef};

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
fn extra_spaces_collapse_without_removing_tabs_or_page_breaks() {
    use wordcraft_doc::para::PAGE_BREAK;

    let mut s = s();
    run(&mut s, "document.setText", json!({"text": format!("a  \t  b  {PAGE_BREAK}  c\t  ")}));
    select_all(&mut s);
    run(&mut s, "para.removeExtraSpaces", json!({"scope": "all"}));
    assert_eq!(text(&s), format!("a \t b {PAGE_BREAK} c\t "), "only repeated spaces collapse");
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
fn spaces_to_indent_keeps_spaces_the_indent_limit_cannot_represent() {
    let mut many = s();
    run(&mut many, "document.setText", json!({"text": format!("{}a", " ".repeat(24))}));
    select_all(&mut many);
    run(&mut many, "para.spacesToIndent", json!({"scope": "all"}));
    assert_eq!(text(&many), "  a", "22 spaces fit in the 1584 pt maximum; two stay as text");
    assert_eq!(many.doc.para(StoryRef::Body, &Path::top(0)).unwrap().props.indent_first, Some(1584.0));

    let mut nearly_full = s();
    run(&mut nearly_full, "document.setText", json!({"text": "  a"}));
    run(&mut nearly_full, "para.indents", json!({"firstLine": 1548.0}));
    select_all(&mut nearly_full);
    run(&mut nearly_full, "para.spacesToIndent", json!({"scope": "all"}));
    assert_eq!(text(&nearly_full), "  a", "less than 72 pt of room cannot replace one space");
    assert_eq!(nearly_full.doc.para(StoryRef::Body, &Path::top(0)).unwrap().props.indent_first, Some(1548.0));
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
fn a_hanging_indent_that_sums_to_zero_is_still_removed() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a"}));
    run(&mut s, "para.indents", json!({"left": 72.0, "firstLine": -72.0}));
    select_all(&mut s);
    run(&mut s, "para.indentToSpaces", json!({"scope": "all"}));
    assert_eq!(text(&s), "a", "the cancelling indents need no leading spaces");
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!((p.props.indent_left, p.props.indent_first), (Some(0.0), Some(0.0)));

    let mut tracked = Session::new(wordcraft_doc::Document::new());
    run(&mut tracked, "document.setText", json!({"text": "a"}));
    run(&mut tracked, "para.indents", json!({"left": 72.0, "firstLine": -72.0}));
    run(&mut tracked, "review.trackChanges", json!({"value": true}));
    select_all(&mut tracked);
    run(&mut tracked, "para.indentToSpaces", json!({"scope": "all"}));
    assert!(!tracked.doc.revisions.iter().any(|r| r.kind == RevisionKind::Insert), "no inserted text means no insert revision");
    assert!(tracked.doc.revisions.iter().any(|r| r.kind == RevisionKind::Format), "the removed hanging indent is tracked as formatting");
}

#[test]
fn cleanup_keeps_fields_footnote_anchors_and_mixed_formatting() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "aa  bb  "}));
    s.sel = crate::Selection { anchor: Pos::body(0, 4), focus: Pos::body(0, 6) };
    run(&mut s, "format.bold", json!({"value": true}));
    s.sel = crate::Selection::caret(s.doc.end_of(StoryRef::Body));
    run(&mut s, "insert.field", json!({"instr": "AUTHOR", "result": "Ada"}));
    let note = s.run("references.footnote", &json!({"text": "keep this note"})).unwrap()["id"].as_u64().unwrap() as u32;
    s.sel = crate::Selection::caret(s.doc.end_of(StoryRef::Body));
    run(&mut s, "text.insert", json!({"text": "  tail"}));

    select_all(&mut s);
    run(&mut s, "para.removeExtraSpaces", json!({"scope": "all"}));

    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!((p.props_of_char(3).bold, p.props_of_char(4).bold), (Some(true), Some(true)), "the bold run moved but kept its formatting");
    assert!(p.object_offsets().iter().any(|off| matches!(p.object_at(*off), Some(InlineObject::Field { .. }))), "the field stays");
    assert!(p.object_offsets().iter().any(|off| matches!(p.object_at(*off), Some(InlineObject::NoteRef { .. }))), "the footnote anchor stays");
    assert!(s.doc.parts.contains_key(&note), "the anchored footnote story stays");
}

#[test]
fn cleanup_inside_a_table_keeps_the_table_and_its_cell() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    run(&mut s, "text.insert", json!({"text": "a  b"}));
    select_all(&mut s);
    run(&mut s, "para.removeExtraSpaces", json!({"scope": "all"}));
    let table = s.doc.body.iter().find_map(|block| block.as_table()).expect("the table stays");
    let cell = table.rows[0].cells[0].blocks.iter().find_map(|block| block.as_para()).expect("the cell paragraph stays");
    assert_eq!(cell.text, "a b");
}

#[test]
fn whole_document_scope_from_a_table_cell_reaches_the_body() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "body  text"}));
    s.sel = crate::Selection::caret(s.doc.end_of(StoryRef::Body));
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 2}));
    let table_index = s.doc.body.iter().position(|block| block.as_table().is_some()).expect("the table was inserted") as u32;
    s.sel = crate::Selection::caret(Pos { story: StoryRef::Body, path: Path(vec![table_index, 0, 0, 0]), off: 0 });
    run(&mut s, "text.insert", json!({"text": "cell  text"}));
    assert!(s.sel.focus.path.depth() > 0, "the caret is inside the table");
    run(&mut s, "para.removeExtraSpaces", json!({"scope": "all"}));
    assert!(text(&s).contains("body text"), "body text outside the table was cleaned too: {}", text(&s));
    let table = s.doc.body.iter().find_map(|block| block.as_table()).expect("the table stays");
    let cell = table.rows[0].cells[0].blocks.iter().find_map(|block| block.as_para()).expect("the cell stays");
    assert_eq!(cell.text, "cell text");
}

#[test]
fn paragraph_tools_refuse_content_locked_controls() {
    for (text, command) in [("a  b", "para.removeExtraSpaces"), ("a\nb", "para.convertBreaks")] {
        let mut s = s();
        type_breaks(&mut s, text);
        select_all(&mut s);
        run(&mut s, "developer.richText", json!({"tag": "locked"}));
        run(&mut s, "developer.properties", json!({"control": "locked", "lockEdit": true}));
        select_all(&mut s);
        let before = s.doc.clone();
        let error = s.run(command, &json!({"scope": "all"})).unwrap_err();
        assert!(error.to_string().contains("can't be edited"), "{command}: {error}");
        assert_eq!(s.doc, before, "{command} changed locked content");
    }
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
fn structural_tools_do_not_cross_the_selection() {
    let mut breaks = s();
    type_breaks(&mut breaks, "one\ntwo");
    run(&mut breaks, "text.newParagraph", json!({}));
    run(&mut breaks, "text.insert", json!({"text": "outside"}));
    breaks.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 7) };
    run(&mut breaks, "para.convertBreaks", json!({}));
    let last = breaks.doc.para_paths(StoryRef::Body).last().cloned().unwrap();
    assert_eq!(breaks.doc.para(StoryRef::Body, &last).unwrap().text, "outside");

    let mut between = s();
    run(&mut between, "document.setText", json!({"text": "a\nb\nc"}));
    between.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(1, 1) };
    run(&mut between, "para.blankBetween", json!({}));
    assert_eq!(text(&between), "a\n\nb\nc", "only a and b get a blank between them");

    let mut separate = s();
    run(&mut separate, "document.setText", json!({"text": "a\nb\nc"}));
    separate.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(1, 1) };
    run(&mut separate, "para.separateParagraphs", json!({}));
    assert_eq!(text(&separate), "a\n\nb\n\nc", "the blank after b stays before the unselected c");

    let mut blank = s();
    run(&mut blank, "document.setText", json!({"text": "a\n\nb\n\nc"}));
    blank.sel = crate::Selection::caret(Pos::body(1, 0));
    run(&mut blank, "para.removeBlank", json!({}));
    assert_eq!(text(&blank), "a\nb\n\nc", "the other blank paragraph is outside the selection");
}

#[test]
fn indent_space_conversions_are_reversible_track_changes() {
    let mut to_indent = s();
    run(&mut to_indent, "document.setText", json!({"text": "  a"}));
    run(&mut to_indent, "review.trackChanges", json!({"value": true}));
    select_all(&mut to_indent);
    run(&mut to_indent, "para.spacesToIndent", json!({"scope": "all"}));
    assert_eq!(to_indent.doc.para(StoryRef::Body, &Path::top(0)).unwrap().props.indent_first, Some(144.0));
    run(&mut to_indent, "review.rejectAll", json!({}));
    let p = to_indent.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!((p.text.as_str(), p.props.indent_first), ("  a", None));

    let mut to_spaces = s();
    run(&mut to_spaces, "document.setText", json!({"text": "a"}));
    run(&mut to_spaces, "para.indents", json!({"firstLine": 144.0}));
    run(&mut to_spaces, "review.trackChanges", json!({"value": true}));
    select_all(&mut to_spaces);
    run(&mut to_spaces, "para.indentToSpaces", json!({"scope": "all"}));
    run(&mut to_spaces, "review.rejectAll", json!({}));
    let p = to_spaces.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!((p.text.as_str(), p.props.indent_first), ("a", Some(144.0)));

    let mut accepted = s();
    run(&mut accepted, "document.setText", json!({"text": "a"}));
    run(&mut accepted, "para.indents", json!({"firstLine": 144.0}));
    run(&mut accepted, "review.trackChanges", json!({"value": true}));
    select_all(&mut accepted);
    run(&mut accepted, "para.indentToSpaces", json!({"scope": "all"}));
    run(&mut accepted, "review.acceptAll", json!({}));
    let p = accepted.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!((p.text.as_str(), p.props.indent_first), ("  a", Some(0.0)));
}

#[test]
fn indent_to_spaces_does_not_inherit_revision_marks() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 1) };
    run(&mut s, "format.bold", json!({"value": true}));
    run(&mut s, "text.delete", json!({}));
    run(&mut s, "review.trackChanges", json!({"value": false}));
    run(&mut s, "para.indents", json!({"firstLine": 72.0}));
    select_all(&mut s);
    run(&mut s, "para.indentToSpaces", json!({"scope": "all"}));
    let space = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().props_of_char(0);
    assert_eq!((space.ins, space.del, space.fmt_change.as_ref().map(|_| ())), (None, None, None));
}

#[test]
fn converting_too_many_line_breaks_is_atomic() {
    let original = format!("{}end", "x\n".repeat(10_001));
    let mut s = s();
    type_breaks(&mut s, &original);
    select_all(&mut s);
    let before = s.doc.clone();
    let error = s.run("para.convertBreaks", &json!({"scope": "all"})).unwrap_err();
    assert!(error.to_string().contains("at most"), "{error}");
    assert_eq!(s.doc, before, "the refused conversion is all-or-nothing");
}

#[test]
fn repeating_a_tracked_deletion_creates_no_empty_revision() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "  a"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    select_all(&mut s);
    run(&mut s, "para.trimLeadingSpaces", json!({"scope": "all"}));
    let revisions = s.doc.revisions.len();
    run(&mut s, "para.trimLeadingSpaces", json!({"scope": "all"}));
    assert_eq!(s.doc.revisions.len(), revisions, "already-deleted spaces create no revision");
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

/// Tracked-delete the bytes `a..b` of the first body paragraph (Track Changes must be on).
fn track_delete_in_first(s: &mut Session, a: usize, b: usize) {
    let path = s.doc.para_paths(StoryRef::Body)[0].clone();
    let at = |off: usize| Pos { story: StoryRef::Body, path: path.clone(), off };
    crate::cmd::track_delete(s, &at(a), &at(b)).expect("tracked delete");
}

fn first_indent(s: &Session) -> Option<f32> {
    let path = s.doc.para_paths(StoryRef::Body)[0].clone();
    s.doc.para(StoryRef::Body, &path).expect("paragraph").props.indent_first
}

fn first_deleted(s: &Session, off: usize) -> bool {
    let path = s.doc.para_paths(StoryRef::Body)[0].clone();
    s.doc.para(StoryRef::Body, &path).expect("paragraph").props_of_char(off).del.is_some()
}

#[test]
fn remove_extra_spaces_skips_tracked_deleted_spaces() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "a  b"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    track_delete_in_first(&mut s, 2, 3); // the second space is already deleted
    select_all(&mut s);
    run(&mut s, "para.removeExtraSpaces", json!({"scope": "all"}));
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "a b", "the one live space survives");

    // Three spaces, the middle one already deleted: of the two live ones the last is kept.
    let mut s = self::s();
    run(&mut s, "document.setText", json!({"text": "a   b"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    track_delete_in_first(&mut s, 2, 3);
    select_all(&mut s);
    run(&mut s, "para.removeExtraSpaces", json!({"scope": "all"}));
    assert!(first_deleted(&s, 1) && first_deleted(&s, 2), "the first and the already deleted space are deleted");
    assert!(!first_deleted(&s, 3), "the last live space stays");
}

#[test]
fn spaces_to_indent_twice_does_not_count_deleted_spaces_again() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "  x"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    select_all(&mut s);
    run(&mut s, "para.spacesToIndent", json!({"scope": "all"}));
    assert_eq!(first_indent(&s), Some(144.0));
    run(&mut s, "para.spacesToIndent", json!({"scope": "all"}));
    assert_eq!(first_indent(&s), Some(144.0), "the deleted spaces are not indented a second time");
}

#[test]
fn convert_breaks_leaves_a_tracked_deleted_break_alone() {
    let mut s = s();
    type_breaks(&mut s, "a\nb\nc");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    track_delete_in_first(&mut s, 1, 2);
    select_all(&mut s);
    run(&mut s, "para.convertBreaks", json!({"scope": "all"}));
    assert_eq!(s.doc.para_paths(StoryRef::Body).len(), 2, "only the live break becomes a paragraph break");
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
