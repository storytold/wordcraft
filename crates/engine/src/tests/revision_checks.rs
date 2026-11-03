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

/// Who changed what, in review.changes order: (kind, author, text).
fn change_list(s: &mut Session) -> Vec<(String, String, String)> {
    let list = run(s, "review.changes", json!({}));
    list.as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    let f = |k: &str| c[k].as_str().unwrap_or_default().to_string();
                    (f("kind"), f("author"), f("text"))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_deletion_inside_another_authors_insertion_is_listed_as_its_own_change() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Alice".into();
    run(&mut s, "text.insert", json!({"text": "Hello world"}));
    s.author = "Bob".into();
    s.sel = Selection { anchor: Pos::body(0, 5), focus: Pos::body(0, 11) };
    run(&mut s, "text.delete", json!({}));
    assert_eq!(visible(&s), "Hello", "Bob's deletion keeps Alice's text, struck");
    let list = change_list(&mut s);
    let has = |kind: &str, who: &str, text: &str| list.iter().any(|c| c == &(kind.into(), who.into(), text.into()));
    assert!(has("insert", "Alice", " world"), "Alice's insertion is still listed: {list:?}");
    assert!(has("delete", "Bob", " world"), "Bob's deletion is listed, by Bob: {list:?}");
    // Carol adds text after it, so there is a change to move on to.
    s.author = "Carol".into();
    s.sel = Selection::caret(Pos::body(0, 11));
    run(&mut s, "text.insert", json!({"text": "!"}));
    // Next Change still moves on past the struck text (both changes share its range): it
    // stops on it once, then goes on to Carol's.
    s.sel = Selection::caret(Pos::body(0, 0));
    run(&mut s, "review.nextChange", json!({}));
    run(&mut s, "review.nextChange", json!({}));
    assert_eq!(s.sel.ordered(), (Pos::body(0, 5), Pos::body(0, 11)));
    run(&mut s, "review.nextChange", json!({}));
    assert_eq!(s.sel.ordered().0.off, 11, "on to Carol's insertion, not stuck on the struck text");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(visible(&s), "");
}

#[test]
fn a_deletion_of_your_own_insertion_is_not_listed() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": "Hello world"}));
    s.sel = Selection { anchor: Pos::body(0, 5), focus: Pos::body(0, 11) };
    run(&mut s, "text.delete", json!({}));
    assert_eq!(visible(&s), "Hello");
    assert!(change_list(&mut s).iter().all(|(kind, ..)| kind == "insert"));
}

#[test]
fn a_deletion_inside_another_authors_move_destination_is_listed_as_its_own_change() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": "First one.\nSecond one.\nThird one."}));
    s.author = "Alice".into();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.range", json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}}));
    run(&mut s, "edit.cut", json!({}));
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 0}}));
    run(&mut s, "edit.paste", json!({}));
    // Alice moved "Second one." to the top; Bob deletes "Second" there.
    s.author = "Bob".into();
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 6) };
    run(&mut s, "text.delete", json!({}));
    let list = change_list(&mut s);
    let has = |kind: &str, who: &str, text: &str| list.iter().any(|c| c == &(kind.into(), who.into(), text.into()));
    assert!(has("moveTo", "Alice", "Second"), "Alice's move is still listed: {list:?}");
    assert!(has("delete", "Bob", "Second"), "Bob's deletion is listed, by Bob: {list:?}");
    assert!(!list.iter().any(|(k, w, _)| k == "delete" && w == "Alice"), "the move's source isn't listed twice: {list:?}");
}

#[test]
fn a_deletion_of_another_authors_paragraph_mark_is_listed_as_its_own_change() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Alice".into();
    run(&mut s, "text.insert", json!({"text": "One"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Two"}));
    // Bob deletes the paragraph mark Alice inserted.
    s.author = "Bob".into();
    s.sel = Selection { anchor: Pos::body(0, 3), focus: Pos::body(1, 0) };
    run(&mut s, "text.delete", json!({}));
    let mark = s.doc.para_at(&Pos::body(0, 0)).map(|p| (p.mark.ins.is_some(), p.mark.del.is_some()));
    assert_eq!(mark, Some((true, true)), "the mark stays, inserted by Alice and deleted by Bob");
    let ch = run(&mut s, "review.changes", json!({}));
    let list = ch.as_array().cloned().unwrap_or_default();
    let at_mark = |kind: &str, who: &str| {
        list.iter().any(|c| c["kind"] == kind && c["author"] == who && c["start"]["path"] == json!([0]) && c["start"]["off"] == 3)
    };
    assert!(at_mark("insert", "Alice"), "Alice's paragraph mark is still listed: {list:?}");
    assert!(at_mark("delete", "Bob"), "Bob's deletion of it is listed, by Bob: {list:?}");
}

#[test]
fn moving_another_authors_insertion_lists_the_move_source_by_its_mover() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": "First one."}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    // Alice adds a paragraph; Bob moves it to the top.
    s.author = "Alice".into();
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Second one."}));
    s.author = "Bob".into();
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 10}, "focus": {"block": 1, "off": 11}}));
    run(&mut s, "edit.cut", json!({}));
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 0}}));
    run(&mut s, "edit.paste", json!({}));
    let list = change_list(&mut s);
    let has = |kind: &str, who: &str| list.iter().any(|(k, w, _)| k == kind && w == who);
    assert!(has("moveTo", "Bob"), "Bob's destination is listed: {list:?}");
    assert!(has("moveFrom", "Bob"), "Bob's source, inside Alice's insertion, is listed: {list:?}");
    assert!(has("insert", "Alice"), "Alice's insertion is still listed: {list:?}");
    assert!(crate::cmd::review::move_changes(&s).iter().any(|(_, who, from)| who == "Bob" && *from), "a Moved from balloon for Bob");
}

/// Alice's tracked insertion "Hello world", " world" then deleted by Bob.
fn bob_struck_alices_words() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Alice".into();
    run(&mut s, "text.insert", json!({"text": "Hello world"}));
    s.author = "Bob".into();
    s.sel = Selection { anchor: Pos::body(0, 5), focus: Pos::body(0, 11) };
    run(&mut s, "text.delete", json!({}));
    s
}

/// The index in review.changes of `who`'s change of `kind` over `text`.
fn change_index(s: &mut Session, kind: &str, who: &str, text: &str) -> usize {
    let list = change_list(s);
    list.iter().position(|c| c == &(kind.into(), who.into(), text.into())).unwrap_or_else(|| panic!("no {kind} by {who} of {text:?} in {list:?}"))
}

#[test]
fn rejecting_a_listed_deletion_inside_an_insertion_keeps_the_insertion() {
    let mut s = bob_struck_alices_words();
    let n = change_index(&mut s, "delete", "Bob", " world");
    run(&mut s, "review.reject", json!({"change": n}));
    assert_eq!(visible(&s), "Hello world", "the words are back");
    let list = change_list(&mut s);
    assert!(list.iter().all(|(kind, who, _)| kind == "insert" && who == "Alice"), "still Alice's insertion: {list:?}");
}

#[test]
fn accepting_a_listed_insertion_keeps_the_deletion_inside_it() {
    let mut s = bob_struck_alices_words();
    let n = change_index(&mut s, "insert", "Alice", " world");
    run(&mut s, "review.accept", json!({"change": n}));
    assert_eq!(visible(&s), "Hello", "still struck by Bob");
    let list = change_list(&mut s);
    assert!(list.contains(&("delete".into(), "Bob".into(), " world".into())), "{list:?}");
    assert!(!list.contains(&("insert".into(), "Alice".into(), " world".into())), "{list:?}");
}

#[test]
fn accepting_a_listed_deletion_or_rejecting_the_insertion_removes_the_words() {
    for (kind, who, accept) in [("delete", "Bob", true), ("insert", "Alice", false)] {
        let mut s = bob_struck_alices_words();
        let n = change_index(&mut s, kind, who, " world");
        run(&mut s, if accept { "review.accept" } else { "review.reject" }, json!({"change": n}));
        let text = s.doc.para_at(&Pos::body(0, 0)).map(|p| p.plain_text()).unwrap_or_default();
        assert_eq!(text, "Hello", "{kind} by {who}");
    }
}

#[test]
fn a_listed_change_out_of_range_is_an_error() {
    let mut s = bob_struck_alices_words();
    assert!(s.run("review.accept", &json!({"change": 99})).is_err());
    assert!(s.run("review.reject", &json!({"change": u64::MAX})).is_err());
}

#[test]
fn rejecting_a_listed_deletion_of_an_inserted_paragraph_mark_keeps_the_paragraphs() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Alice".into();
    run(&mut s, "text.insert", json!({"text": "One"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Two"}));
    s.author = "Bob".into();
    s.sel = Selection { anchor: Pos::body(0, 3), focus: Pos::body(1, 0) };
    run(&mut s, "text.delete", json!({}));
    let list = run(&mut s, "review.changes", json!({}));
    let n = list
        .as_array()
        .and_then(|a| a.iter().position(|c| c["kind"] == "delete" && c["author"] == "Bob" && c["start"]["off"] == 3))
        .unwrap_or_else(|| panic!("Bob's mark deletion isn't listed: {list}"));
    run(&mut s, "review.reject", json!({"change": n}));
    let mark = s.doc.para_at(&Pos::body(0, 0)).map(|p| (p.mark.ins.is_some(), p.mark.del.is_some()));
    assert_eq!(mark, Some((true, false)), "Alice's inserted mark stays, no longer deleted");
    assert_eq!(s.doc.body.len(), 2);
}

#[test]
fn accept_all_and_reject_all_can_take_one_authors_changes() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": "one two three"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Alice".into();
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": " four"}));
    s.author = "Bob".into();
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 4) };
    run(&mut s, "text.delete", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": " five"}));
    run(&mut s, "review.rejectAll", json!({"author": "Bob"}));
    let list = change_list(&mut s);
    assert_eq!(list, [("insert".to_string(), "Alice".to_string(), " four".to_string())]);
    assert_eq!(visible(&s), "one two three four");
    run(&mut s, "review.acceptAll", json!({"author": "Alice"}));
    assert!(change_list(&mut s).is_empty());
    assert_eq!(visible(&s), "one two three four");
    // An author with no changes changes nothing.
    run(&mut s, "review.acceptAll", json!({"author": "Nobody"}));
}

/// The index in review.changes of the first change matching `f`.
fn listed(s: &mut Session, f: impl Fn(&Value) -> bool) -> usize {
    let list = run(s, "review.changes", json!({}));
    list.as_array().and_then(|a| a.iter().position(f)).unwrap_or_else(|| panic!("not listed: {list}"))
}

#[test]
fn rejecting_a_listed_formatting_change_keeps_another_authors_deletion_under_it() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": "one two three"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Alice".into();
    s.sel = Selection { anchor: Pos::body(0, 3), focus: Pos::body(0, 7) };
    run(&mut s, "text.delete", json!({}));
    s.author = "Bob".into();
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 13) };
    run(&mut s, "format.bold", json!({"value": true}));
    // Bob's bold over Alice's struck " two" (and on past it).
    let n = listed(&mut s, |c| c["kind"] == "format" && c["author"] == "Bob" && c["text"].as_str().is_some_and(|t| t.contains("two")));
    run(&mut s, "review.reject", json!({"change": n}));
    let list = change_list(&mut s);
    assert!(list.contains(&("delete".into(), "Alice".into(), " two".into())), "Alice's deletion stays: {list:?}");
    assert!(!list.iter().any(|(k, _, t)| k == "format" && t.contains("two")), "Bob's bold there is undone: {list:?}");
    let bold = s.doc.para_at(&Pos::body(0, 0)).is_some_and(|p| p.run_ranges().any(|(r, c)| r.start >= 3 && r.start < 13 && c.bold == Some(true)));
    assert!(!bold, "no bold left on \" two three\"");
}

#[test]
fn accepting_a_listed_move_keeps_another_authors_deletion_inside_it() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": "First one.\nSecond one.\nThird one."}));
    s.author = "Alice".into();
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.range", json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}}));
    run(&mut s, "edit.cut", json!({}));
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 0}}));
    run(&mut s, "edit.paste", json!({}));
    s.author = "Bob".into();
    s.sel = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 6) };
    run(&mut s, "text.delete", json!({}));
    let n = change_index(&mut s, "moveTo", "Alice", "Second");
    run(&mut s, "review.accept", json!({"change": n}));
    let list = change_list(&mut s);
    assert!(list.iter().all(|(_, who, _)| who == "Bob"), "Alice's move is resolved, both ends: {list:?}");
    assert!(list.contains(&("delete".into(), "Bob".into(), "Second".into())), "Bob's deletion stays: {list:?}");
}

#[test]
fn a_listed_paragraph_mark_formatting_change_leaves_the_insertion_before_it() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": "One\nTwo"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Alice".into();
    run(&mut s, "caret.set", json!({"pos": {"block": 0, "off": 3}}));
    run(&mut s, "text.insert", json!({"text": " more"}));
    // Bob's formatting change on the paragraph mark alone.
    s.author = "Bob".into();
    let rev = {
        s.doc.revisions.push(wordcraft_doc::Revision { kind: wordcraft_doc::RevisionKind::Format, author: "Bob".into(), ..Default::default() });
        u32::try_from(s.doc.revisions.len() - 1).unwrap_or_default()
    };
    if let Ok(p) = s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)) {
        let old = p.mark.formatting();
        p.mark.bold = Some(true);
        p.mark.fmt_change = wordcraft_doc::props::PropChange::boxed(rev, old);
        p.touch();
    }
    let n = listed(&mut s, |c| c["kind"] == "format" && c["author"] == "Bob" && c["start"] == c["end"]);
    run(&mut s, "review.reject", json!({"change": n}));
    let list = change_list(&mut s);
    assert_eq!(list, [("insert".to_string(), "Alice".to_string(), " more".to_string())], "Alice's insertion stays");
    let mark = s.doc.para_at(&Pos::body(0, 0)).map(|p| (p.mark.bold, p.mark.fmt_change.is_some()));
    assert_eq!(mark, Some((None, false)), "the mark's bold is undone");
}

#[test]
fn a_change_or_author_of_the_wrong_type_is_an_error() {
    let mut s = bob_struck_alices_words();
    let before = change_list(&mut s);
    for bad in [json!(1.5), json!(-1), json!("0"), json!(true)] {
        assert!(s.run("review.accept", &json!({"change": bad})).is_err(), "change {bad}");
    }
    for bad in [json!(5), json!(["Bob"]), json!(false)] {
        assert!(s.run("review.acceptAll", &json!({"author": bad})).is_err(), "author {bad}");
    }
    assert_eq!(change_list(&mut s), before, "nothing was resolved");
    // null is the same as leaving it out.
    run(&mut s, "review.rejectAll", json!({"author": null}));
    assert!(change_list(&mut s).is_empty());
}

#[test]
fn a_listed_paragraph_formatting_change_keeps_another_revisions_numbering_change() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    run(&mut s, "document.setText", json!({"text": "One"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    // Alice's numbering change on the paragraph, then Bob centres it.
    let alice = {
        s.doc.revisions.push(wordcraft_doc::Revision { kind: wordcraft_doc::RevisionKind::Format, author: "Alice".into(), ..Default::default() });
        u32::try_from(s.doc.revisions.len() - 1).unwrap_or_default()
    };
    if let Ok(p) = s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)) {
        p.props.num_change = Some(Box::new(wordcraft_doc::props::NumChange { rev: alice, original: String::new() }));
        p.touch();
    }
    s.author = "Bob".into();
    run(&mut s, "para.align", json!({"value": "center"}));
    let n = listed(&mut s, |c| c["kind"] == "format" && c["author"] == "Bob");
    run(&mut s, "review.reject", json!({"change": n}));
    let p = s.doc.para_at(&Pos::body(0, 0)).cloned().unwrap_or_default();
    assert!(p.props.fmt_change.is_none(), "Bob's centring is resolved");
    assert_eq!(p.props.align, None, "and undone");
    assert_eq!(p.props.num_change.map(|n| n.rev), Some(alice), "Alice's numbering change stays");
}
