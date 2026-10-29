//! #1329: table deletion, Change Case and field unlink respect locked content controls and
//! Track Changes (Reject All restores what they changed).

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Pos, StoryRef, Table};

use crate::{Selection, Session};

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The first paragraph's text without its tracked deletions.
fn visible(s: &Session) -> String {
    let Some(p) = s.doc.para_at(&Pos::body(0, 0)) else { return String::new() };
    p.run_ranges().filter(|(_, c)| c.del.is_none()).filter_map(|(r, _)| p.text.get(r)).collect::<String>().replace(OBJ, "")
}

fn table(s: &Session) -> Option<&Table> {
    s.doc.body.iter().find_map(|b| b.as_table())
}

/// Every cell's text, row by row (deleted text included).
fn cells(s: &Session) -> Vec<Vec<String>> {
    table(s)
        .map(|t| {
            t.rows
                .iter()
                .map(|r| r.cells.iter().map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect()).collect())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether every character of every cell in `which` is a tracked deletion.
fn cells_deleted(s: &Session, which: &[(usize, usize)]) -> bool {
    let Some(t) = table(s) else { return false };
    which.iter().all(|(r, c)| {
        let cell = &t.rows[*r].cells[*c];
        cell.blocks.iter().filter_map(|b| b.as_para()).all(|p| p.run_ranges().all(|(_, props)| props.del.is_some()))
    })
}

/// A 2×2 table holding A1 B1 / A2 B2, the caret in A1 (at its start).
fn table_session() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    for (i, t) in ["A1", "B1", "A2", "B2"].iter().enumerate() {
        if i > 0 {
            run(&mut s, "text.tab", json!({}));
        }
        run(&mut s, "text.insert", json!({"text": t}));
    }
    caret_in(&mut s, 0, 0, 0);
    s
}

fn caret_in(s: &mut Session, r: u32, c: u32, off: usize) {
    s.sel = Selection::caret(Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![0, r, c, 0]), off });
}

/// [`table_session`] with "A1" in a content control tagged `ctl`, locked with `lock`.
fn locked_table(lock: &str) -> Session {
    let mut s = table_session();
    let at = |off| Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![0, 0, 0, 0]), off };
    // A partial selection inside the paragraph (not the whole paragraph) wraps inline.
    s.sel = Selection { anchor: at(0), focus: at(1) };
    run(&mut s, "developer.richText", json!({"tag": "ctl"}));
    run(&mut s, "developer.properties", json!({"control": "ctl", lock: true}));
    caret_in(&mut s, 0, 0, 0);
    s
}

#[test]
fn table_deletes_refuse_a_control_locked_against_deletion() {
    for id in ["table.deleteTable", "table.deleteRow", "table.deleteColumn", "table.deleteCells"] {
        let mut s = locked_table("lockDelete");
        let before = s.doc.clone();
        assert!(s.run(id, &json!({})).is_err(), "{id} deleted a locked control");
        assert_eq!(s.doc, before, "{id} changed the document");
        assert!(s.status.contains("can't be deleted"), "{id}: {}", s.status);
    }
}

#[test]
fn tracked_table_delete_marks_the_text_deleted_and_reject_all_restores_it() {
    let mut s = table_session();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "table.deleteTable", json!({}));
    // Like Word: the structure stays, its text is marked deleted.
    assert!(table(&s).is_some(), "the table was removed under Track Changes");
    assert_eq!(cells(&s), [["A1", "B1"], ["A2", "B2"]]);
    assert!(cells_deleted(&s, &[(0, 0), (0, 1), (1, 0), (1, 1)]));
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(cells(&s), [["A1", "B1"], ["A2", "B2"]]);
    assert!(!cells_deleted(&s, &[(0, 0)]) && s.doc.revisions.is_empty());
}

#[test]
fn tracked_row_column_and_cell_deletes_mark_their_text_deleted() {
    for (id, gone, kept) in [
        ("table.deleteRow", vec![(0, 0), (0, 1)], vec![(1, 0), (1, 1)]),
        ("table.deleteColumn", vec![(0, 0), (1, 0)], vec![(0, 1), (1, 1)]),
        ("table.deleteCells", vec![(0, 0)], vec![(0, 1), (1, 0), (1, 1)]),
    ] {
        let mut s = table_session();
        run(&mut s, "review.trackChanges", json!({"value": true}));
        run(&mut s, id, json!({}));
        assert_eq!(cells(&s), [["A1", "B1"], ["A2", "B2"]], "{id} changed the structure");
        assert!(cells_deleted(&s, &gone), "{id}: text not marked deleted");
        for k in &kept {
            assert!(!cells_deleted(&s, &[*k]), "{id}: {k:?} marked deleted");
        }
        run(&mut s, "review.rejectAll", json!({}));
        assert_eq!(cells(&s), [["A1", "B1"], ["A2", "B2"]]);
        assert!(s.doc.revisions.is_empty(), "{id}");
    }
}

#[test]
fn tracked_table_delete_over_tracked_insertions_rejects_back_to_the_original() {
    let mut s = table_session();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    // One insertion by this author, one by another.
    caret_in(&mut s, 0, 0, 2);
    run(&mut s, "text.insert", json!({"text": "x"}));
    let me = s.author.clone();
    s.author = "Other".into();
    caret_in(&mut s, 1, 1, 2);
    run(&mut s, "text.insert", json!({"text": "y"}));
    s.author = me;
    caret_in(&mut s, 0, 0, 0);
    run(&mut s, "table.deleteTable", json!({}));
    assert!(table(&s).is_some());
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(cells(&s), [["A1", "B1"], ["A2", "B2"]]);
    assert!(s.doc.revisions.is_empty());
}

/// "hello world" with "hello" in a rich text control tagged `ctl`.
fn hello_in_control(lock: Option<&str>) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "hello world"}));
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 5) };
    run(&mut s, "developer.richText", json!({"tag": "ctl"}));
    if let Some(l) = lock {
        run(&mut s, "developer.properties", json!({"control": "ctl", l: true}));
    }
    s
}

#[test]
fn change_case_refuses_content_locked_against_editing() {
    let mut s = hello_in_control(Some("lockEdit"));
    let w = OBJ.len_utf8();
    // Just the control's content, and the whole paragraph.
    for (a, b) in [(w, w + 5), (0, 11 + 2 * w)] {
        s.sel = Selection { anchor: Pos::body(0, a), focus: Pos::body(0, b) };
        let before = s.doc.clone();
        assert!(s.run("format.changeCase", &json!({"mode": "upper"})).is_err(), "{a}..{b}");
        assert_eq!(s.doc, before);
    }
    // Locked against deletion only: the content may change.
    let mut s = hello_in_control(Some("lockDelete"));
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 11 + 2 * w) };
    run(&mut s, "format.changeCase", json!({"mode": "upper"}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "HELLO WORLD");
}

#[test]
fn tracked_change_case_rejects_back_to_the_original_case() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "hello World"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.changeCase", json!({"mode": "upper"}));
    assert_eq!(visible(&s), "HELLO WORLD");
    // The selection still covers the converted text.
    let (a, b) = s.sel.ordered();
    assert_eq!((a.off, b.off), (0, s.doc.para_at(&a).map(|p| p.len()).unwrap_or(0)));
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "hello World");
    assert!(s.doc.revisions.is_empty());
    // Accept All keeps the new case.
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.changeCase", json!({"mode": "upper"}));
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "HELLO WORLD");
}

#[test]
fn tracked_change_case_of_own_insertion_stays_an_insertion() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "keep "}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": "new"}));
    s.sel = Selection { anchor: Pos::body(0, 5), focus: Pos::body(0, 8) };
    run(&mut s, "format.changeCase", json!({"mode": "upper"}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "keep NEW", "no deleted copy of text that never existed");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "keep ");
}

/// "Made by {AUTHOR} on " with "by {AUTHOR} on" in a rich text control tagged `ctl`.
fn field_in_control(lock: &str) -> (Session, Pos) {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.doc.core.creator = "Ada".into();
    run(&mut s, "text.insert", json!({"text": "Made by "}));
    run(&mut s, "insert.field", json!({"instr": "AUTHOR", "result": "Ada"}));
    run(&mut s, "text.insert", json!({"text": " on "}));
    let w = OBJ.len_utf8();
    s.sel = Selection { anchor: Pos::body(0, 5), focus: Pos::body(0, 8 + w + 3) };
    run(&mut s, "developer.richText", json!({"tag": "ctl"}));
    run(&mut s, "developer.properties", json!({"control": "ctl", lock: true}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let off = p.object_offsets().into_iter().find(|o| matches!(p.object_at(*o), Some(InlineObject::Field { .. }))).unwrap();
    (s, Pos::body(0, off))
}

#[test]
fn unlink_refuses_a_field_in_content_locked_against_editing() {
    let (mut s, at) = field_in_control("lockEdit");
    s.sel = Selection::caret(at);
    let before = s.doc.clone();
    assert!(s.run("fields.unlink", &json!({})).is_err());
    assert_eq!(s.doc, before);
    // Locked against deletion only: unlinking is an edit of the content, allowed.
    let (mut s, at) = field_in_control("lockDelete");
    s.sel = Selection::caret(at);
    run(&mut s, "fields.unlink", json!({}));
    assert!(s.doc.plain_text(StoryRef::Body).contains("by Ada on"));
}

#[test]
fn tracked_unlink_keeps_the_result_as_plain_text() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.doc.core.creator = "Ada".into();
    run(&mut s, "text.insert", json!({"text": "Made by "}));
    run(&mut s, "insert.field", json!({"instr": "AUTHOR", "result": "Ada"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.sel = Selection::caret(Pos::body(0, 8));
    run(&mut s, "fields.unlink", json!({}));
    // Like Word, not tracked: the result is ordinary text, shown and kept by Reject All.
    assert_eq!(visible(&s), "Made by Ada");
    assert!(s.doc.para_at(&Pos::body(0, 0)).unwrap().objects.is_empty());
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Made by Ada");
}

#[test]
fn tracked_delete_table_removes_a_table_with_no_live_text() {
    // Delete, Accept All (an empty grid is left), Delete again: the table goes.
    let mut s = table_session();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "table.deleteTable", json!({}));
    run(&mut s, "review.acceptAll", json!({}));
    assert!(table(&s).is_some());
    caret_in(&mut s, 0, 0, 0);
    run(&mut s, "table.deleteTable", json!({}));
    assert!(table(&s).is_none(), "an empty table can't be deleted under Track Changes");
    assert!(s.doc.revisions.is_empty(), "no unused revision");
    // A fresh empty table too.
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let revs = s.doc.revisions.len();
    run(&mut s, "table.deleteTable", json!({}));
    assert!(table(&s).is_none());
    assert_eq!(s.doc.revisions.len(), revs);
}

/// A 2×2 table whose first row is one cell "Title" across both columns; A | B below; caret in A.
fn merged_title_table(lock: Option<&str>) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    let at = |r, c| Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![0, r, c, 0]), off: 0 };
    s.sel = Selection { anchor: at(0, 0), focus: at(0, 1) };
    run(&mut s, "table.merge", json!({}));
    assert_eq!(table(&s).unwrap().rows[0].cells.len(), 1);
    caret_in(&mut s, 0, 0, 0);
    run(&mut s, "text.insert", json!({"text": "Title"}));
    caret_in(&mut s, 1, 0, 0);
    run(&mut s, "text.insert", json!({"text": "A"}));
    caret_in(&mut s, 1, 1, 0);
    run(&mut s, "text.insert", json!({"text": "B"}));
    if let Some(l) = lock {
        let p = |off| Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![0, 0, 0, 0]), off };
        s.sel = Selection { anchor: p(0), focus: p(2) };
        run(&mut s, "developer.richText", json!({"tag": "ctl"}));
        run(&mut s, "developer.properties", json!({"control": "ctl", l: true}));
    }
    caret_in(&mut s, 1, 0, 0);
    s
}

#[test]
fn delete_columns_leaves_a_merged_cell_that_keeps_a_column_alone() {
    // Tracked: only A is marked deleted, not the merged title.
    let mut s = merged_title_table(None);
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "table.deleteColumn", json!({}));
    assert!(cells_deleted(&s, &[(1, 0)]));
    assert!(!cells_deleted(&s, &[(0, 0)]), "the merged title was struck");
    assert!(!cells_deleted(&s, &[(1, 1)]));
    // Untracked: a lockDelete control in the title doesn't stop the column going.
    let mut s = merged_title_table(Some("lockDelete"));
    run(&mut s, "table.deleteColumn", json!({}));
    assert_eq!(table(&s).map(|t| t.grid.len()), Some(1));
    assert_eq!(s.doc.control_ranges(StoryRef::Body).len(), 1);
}

#[test]
fn tracked_row_and_column_deletes_over_tracked_changes_reject_back_to_the_original() {
    for id in ["table.deleteRow", "table.deleteColumn"] {
        let mut s = table_session();
        run(&mut s, "review.trackChanges", json!({"value": true}));
        caret_in(&mut s, 0, 0, 2);
        run(&mut s, "text.insert", json!({"text": "x"}));
        let me = s.author.clone();
        s.author = "Other".into();
        caret_in(&mut s, 0, 1, 2);
        run(&mut s, "text.insert", json!({"text": "y"}));
        caret_in(&mut s, 1, 0, 2);
        run(&mut s, "text.insert", json!({"text": "z"}));
        // And a deletion already there.
        s.sel = Selection {
            anchor: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![0, 0, 0, 0]), off: 0 },
            focus: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![0, 0, 0, 0]), off: 1 },
        };
        run(&mut s, "text.delete", json!({}));
        s.author = me;
        caret_in(&mut s, 0, 0, 0);
        run(&mut s, id, json!({}));
        run(&mut s, "review.rejectAll", json!({}));
        assert_eq!(cells(&s), [["A1", "B1"], ["A2", "B2"]], "{id}");
        assert!(s.doc.revisions.is_empty(), "{id}");
    }
}

fn tracked_hello() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "hello world"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.changeCase", json!({"mode": "upper"}));
    assert_eq!(visible(&s), "HELLO WORLD");
    s
}

#[test]
fn tracked_change_case_twice_reads_only_the_live_text() {
    let mut s = tracked_hello();
    run(&mut s, "format.changeCase", json!({"mode": "title"}));
    assert_eq!(visible(&s), "Hello World");
    // Shift+F3 goes on from UPPERCASE as it does untracked.
    let mut plain = Session::new(wordcraft_doc::Document::new());
    run(&mut plain, "text.insert", json!({"text": "HELLO WORLD"}));
    run(&mut plain, "select.all", json!({}));
    run(&mut plain, "format.changeCase", json!({}));
    let mut s = tracked_hello();
    run(&mut s, "format.changeCase", json!({}));
    assert_eq!(visible(&s), plain.doc.plain_text(StoryRef::Body));
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "hello world");
}

#[test]
fn tracked_change_case_handles_sharp_s_and_mixed_formatting() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "text.insert", json!({"text": "straße ab"}));
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 3) };
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.changeCase", json!({"mode": "upper"}));
    assert_eq!(visible(&s), "STRASSE AB");
    // Each inserted stretch keeps its run's formatting.
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let live: Vec<(String, bool)> =
        p.run_ranges().filter(|(_, c)| c.del.is_none()).map(|(r, c)| (p.text[r].to_string(), c.bold == Some(true))).collect();
    let bold: String = live.iter().filter(|x| x.1).map(|x| x.0.as_str()).collect();
    assert_eq!(bold, "STR");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "straße ab");
}

#[test]
fn tracked_delete_of_struck_table_text_keeps_it_for_reject_all() {
    for id in ["table.deleteTable", "table.deleteRow", "table.deleteColumn", "table.deleteCells"] {
        let mut s = table_session();
        run(&mut s, "review.trackChanges", json!({"value": true}));
        run(&mut s, id, json!({}));
        caret_in(&mut s, 0, 0, 0);
        // Pressed again: the text is already struck, so nothing is removed outright.
        let before = s.doc.clone();
        assert!(s.run(id, &json!({})).is_err(), "{id}");
        assert_eq!(s.doc, before, "{id}");
        assert!(s.status.contains("already marked deleted"), "{id}: {}", s.status);
        run(&mut s, "review.rejectAll", json!({}));
        assert_eq!(cells(&s), [["A1", "B1"], ["A2", "B2"]], "{id}");
    }
}

#[derive(Clone, Copy, Debug)]
enum Slot {
    FootnoteSeparator,
    EndnoteSeparator,
    FootnoteContinuationSeparator,
    EndnoteContinuationNotice,
}

const SLOTS: [Slot; 4] = [Slot::FootnoteSeparator, Slot::EndnoteSeparator, Slot::FootnoteContinuationSeparator, Slot::EndnoteContinuationNotice];

fn slot_mut(doc: &mut wordcraft_doc::Document, slot: Slot) -> &mut Option<wordcraft_doc::Blocks> {
    match slot {
        Slot::FootnoteSeparator => &mut doc.footnote_separators.separator,
        Slot::EndnoteSeparator => &mut doc.endnote_separators.separator,
        Slot::FootnoteContinuationSeparator => &mut doc.footnote_separators.continuation_separator,
        Slot::EndnoteContinuationNotice => &mut doc.endnote_separators.continuation_notice,
    }
}

/// A session whose note separator `slot` holds the text "SEP" as a tracked insertion (revision 0).
fn tracked_separator_session(slot: Slot) -> Session {
    let mut doc = wordcraft_doc::Document::new();
    doc.revisions.push(wordcraft_doc::Revision {
        kind: wordcraft_doc::RevisionKind::Insert,
        author: "Alice".into(),
        date: String::new(),
        move_name: None,
    });
    let props = wordcraft_doc::CharProps { ins: Some(0), ..Default::default() };
    let p = wordcraft_doc::Paragraph::with_text("SEP", props);
    *slot_mut(&mut doc, slot) = Some(vec![wordcraft_doc::para_block(p)]);
    Session::new(doc)
}

fn separator_para(s: &mut Session, slot: Slot) -> wordcraft_doc::Paragraph {
    slot_mut(&mut s.doc, slot).as_ref().and_then(|b| b.iter().find_map(|b| b.as_para())).expect("separator").clone()
}

#[test]
fn accept_all_resolves_note_separator_revisions() {
    for slot in SLOTS {
        let mut s = tracked_separator_session(slot);
        run(&mut s, "review.acceptAll", json!({}));
        let p = separator_para(&mut s, slot);
        assert_eq!(p.plain_text(), "SEP", "{slot:?}");
        assert!(p.run_ranges().all(|(_, c)| c.ins.is_none() && c.del.is_none()), "{slot:?}");
    }
}

#[test]
fn reject_all_resolves_note_separator_revisions() {
    for slot in SLOTS {
        let mut s = tracked_separator_session(slot);
        run(&mut s, "review.rejectAll", json!({}));
        assert_eq!(separator_para(&mut s, slot).plain_text(), "", "{slot:?}");
    }
}

#[test]
fn new_tracked_change_does_not_alias_separator_revision() {
    for slot in SLOTS {
        let mut s = tracked_separator_session(slot);
        run(&mut s, "review.acceptAll", json!({}));
        s.author = "Bob".into();
        run(&mut s, "review.trackChanges", json!({"value": true}));
        run(&mut s, "text.insert", json!({"text": "NEW "}));
        let p = separator_para(&mut s, slot);
        assert!(p.run_ranges().all(|(_, c)| c.ins.is_none() && c.del.is_none()), "{slot:?}");
    }
}
