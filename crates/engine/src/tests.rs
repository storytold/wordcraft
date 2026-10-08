use serde_json::json;
use wordcraft_doc::{Pos, StoryRef};

use crate::{Session, cmd};

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

#[test]
fn every_command_has_unique_id() {
    let reg = cmd::registry();
    let mut ids: Vec<&str> = reg.all().iter().map(|c| c.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate command ids");
    assert!(n > 150, "{n} commands");
}

#[test]
fn typing_enter_undo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "text.insert", json!({"text": " world"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Second"}));
    assert_eq!(text(&s), "Hello world\nSecond");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "Hello world\n");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "Hello world");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "Hello world");
}

#[test]
fn backspace_and_delete() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "abc"}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(text(&s), "ab");
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "cd"}));
    run(&mut s, "caret.home", json!({}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(text(&s), "abcd");
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(text(&s), "bcd");
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.deleteWordBack", json!({}));
    assert_eq!(text(&s), "");
}

#[test]
fn bold_toggles_selection_and_caret_word() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "make this bold"}));
    run(&mut s, "select.text", json!({"text": "this"}));
    run(&mut s, "format.bold", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(5).bold, Some(true));
    assert_eq!(p.props_of_char(0).bold, None);
    run(&mut s, "format.bold", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(5).bold, Some(false));
    // Caret inside a word formats the word.
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [0], "off": 11}}));
    run(&mut s, "format.italic", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(10).italic, Some(true));
    assert_eq!(p.props_of_char(13).italic, Some(true));
    assert_eq!(p.props_of_char(9).italic, None);
}

#[test]
fn pending_format_applies_to_typing() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "a "}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "text.insert", json!({"text": "b"}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(2).bold, Some(true));
    assert_eq!(p.props_of_char(0).bold, None);
}

#[test]
fn styles_and_lists() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Title"}));
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    // Heading's next style is Normal.
    assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().props.style.as_deref(), Some("Normal"));
    run(&mut s, "para.bullets", json!({}));
    run(&mut s, "text.insert", json!({"text": "one"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "two"}));
    let n1 = s.doc.para_at(&Pos::body(1, 0)).unwrap().props.numbering.unwrap();
    let n2 = s.doc.para_at(&Pos::body(2, 0)).unwrap().props.numbering.unwrap();
    assert_eq!(n1.num, n2.num);
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.newParagraph", json!({})); // empty item ends the list
    assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.map(|n| n.num), Some(0));
    // "1. " autoformat.
    run(&mut s, "text.insert", json!({"text": "1."}));
    run(&mut s, "text.insert", json!({"text": " "}));
    assert!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.is_some_and(|n| n.num != 0));
}

#[test]
fn find_replace() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "cat dog cat\nCat bird"}));
    let r = run(&mut s, "edit.find", json!({"text": "cat"}));
    assert_eq!(r["count"], 3);
    let r = run(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox", "matchCase": true}));
    assert_eq!(r["replaced"], 2);
    assert_eq!(text(&s), "fox dog fox\nCat bird");
    let r = run(&mut s, "edit.find", json!({"text": "\\b\\w{3}\\b", "regex": true, "matchCase": false}));
    assert_eq!(r["count"], 4);
}

#[test]
fn clipboard_round_trip() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "alpha beta\ngamma"}));
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 1, "off": 2}}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard_text, "beta\nga");
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(text(&s), "alpha beta\ngammabeta\nga");
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(text(&s), "");
    run(&mut s, "edit.paste", json!({"text": "plain\ntext"}));
    assert_eq!(text(&s), "plain\ntext");
}

#[test]
fn tables_commands() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 3}));
    assert!(s.sel.focus.path.cell().is_some());
    run(&mut s, "text.insert", json!({"text": "A1"}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.insert", json!({"text": "B1"}));
    run(&mut s, "table.insertRowBelow", json!({}));
    run(&mut s, "table.insertColumnRight", json!({}));
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 3);
    assert_eq!(t.cols(), 4);
    run(&mut s, "table.deleteTable", json!({}));
    assert!(s.doc.body.iter().all(|b| b.as_table().is_none()));
    assert!(s.run("table.merge", &json!({})).is_err());
}

#[test]
fn page_setup_and_breaks() {
    let mut s = s();
    run(&mut s, "layout.orientation", json!({"value": "landscape"}));
    assert!(s.doc.last_section.landscape);
    run(&mut s, "layout.size", json!({"name": "A4"}));
    assert!((s.doc.last_section.page_w - 841.89).abs() < 0.1);
    run(&mut s, "layout.margins", json!({"preset": "narrow"}));
    assert_eq!(s.doc.last_section.margin_left, 36.0);
    run(&mut s, "text.insert", json!({"text": "one"}));
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "text.insert", json!({"text": "two"}));
    assert_eq!(s.doc.sections().len(), 2);
    assert_eq!(s.layout().pages.len(), 2);
    assert!(s.run("layout.margins", &json!({"left": 1000.0})).is_err());
}

#[test]
fn track_changes_and_accept() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "original"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": " added"}));
    run(&mut s, "select.text", json!({"text": "orig"}));
    s.author = "Someone Else".into();
    run(&mut s, "text.delete", json!({}));
    // Deleted text stays (marked) until accepted.
    assert_eq!(text(&s), "original added");
    let ch = run(&mut s, "review.changes", json!({}));
    assert_eq!(ch.as_array().unwrap().len(), 2);
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "inal added");
}

#[test]
fn comments() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Some text here"}));
    run(&mut s, "select.text", json!({"text": "text"}));
    let r = run(&mut s, "review.newComment", json!({"text": "Nice"}));
    let id = r["id"].as_u64().unwrap();
    let l = run(&mut s, "review.comments", json!({}));
    assert_eq!(l[0]["text"], "Nice");
    assert_eq!(text(&s), "Some text here");
    run(&mut s, "review.deleteComment", json!({"id": id}));
    assert!(s.doc.comments.is_empty());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().objects.len(), 0);
}

#[test]
fn toc_and_fields() {
    let mut s = Session::new(crate::sample::report());
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({}));
    let t = text(&s);
    assert!(t.contains("Contents"), "{t}");
    assert!(t.contains("Summary\t1"), "{t}");
    run(&mut s, "references.updateToc", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body).matches("Summary\t").count(), 1, "one TOC entry after update");
}

#[test]
fn failed_command_leaves_document_unchanged() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "keep"}));
    let before = text(&s);
    assert!(s.run("format.size", &json!({"size": -3})).is_err());
    assert!(s.run("no.such", &json!({})).is_err());
    assert!(s.run("text.insert", &json!({})).is_err());
    assert_eq!(text(&s), before);
}

#[test]
fn hostile_params_never_panic() {
    let reg = cmd::registry();
    let junk = [
        json!(null),
        json!({}),
        json!({"text": 5, "size": "x", "path": [], "pos": {"block": 9999, "off": 99999}}),
        json!({"value": -1e308, "rows": 1e9}),
    ];
    for spec in reg.all() {
        if spec.id.starts_with("file.") || spec.id == "insert.picture" || spec.id == "insert.textFromFile" {
            continue;
        }
        for j in &junk {
            let mut s = Session::new(crate::sample::sample_document());
            let _ = s.run(spec.id, j);
            s.clamp_selection();
            let _ = s.layout();
        }
    }
}

#[test]
fn inspect_reports_structure() {
    let mut s = Session::new(crate::sample::sample_document());
    let r = run(&mut s, "document.inspect", json!({}));
    assert!(r["pages"].as_u64().unwrap() >= 1);
    assert!(r["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "table"));
    let f = run(&mut s, "format.state", json!({}));
    assert_eq!(f["styleName"], "Title");
}

#[test]
fn caret_navigation() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "first line\nsecond line"}));
    run(&mut s, "caret.down", json!({}));
    assert_eq!(s.sel.focus.path.last(), 1);
    run(&mut s, "caret.end", json!({}));
    assert_eq!(s.sel.focus.off, 11);
    run(&mut s, "caret.up", json!({"extend": true}));
    assert_eq!(s.sel.focus.path.last(), 0);
    assert!(!s.sel.is_collapsed());
    run(&mut s, "caret.wordLeft", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    assert_eq!(s.sel.focus, Pos::body(1, 11));
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, 10);
}

#[test]
fn text_box_insert_puts_the_caret_inside() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Before "}));
    let id = run(&mut s, "insert.textBox", json!({}))["story"].as_u64().unwrap() as u32;
    let boxed = StoryRef::Part(id);
    assert_eq!(s.sel.focus.story, boxed);
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    assert_eq!(s.doc.plain_text(boxed), "Hello");
    assert!(!text(&s).contains("Hello"), "{}", text(&s));
    // Leaving the box lands just after it in the body.
    let anchor = s.doc.text_box_anchor(id).unwrap();
    assert_eq!(anchor, Pos::body(0, "Before \u{FFFC}".len()));
    run(&mut s, "caret.set", json!({"pos": anchor}));
    run(&mut s, "text.insert", json!({"text": " after"}));
    assert_eq!(s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).map(|p| p.text.as_str()), Some("Before \u{FFFC} after"));
    // Undo goes back through the typing to before the box, in the body.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.sel.focus.story, StoryRef::Body);
    assert_eq!(text(&s), "Before ");
    // Given text: the caret ends after it.
    let id = run(&mut s, "insert.textBox", json!({"text": "Note"}))["story"].as_u64().unwrap() as u32;
    assert_eq!(s.sel.focus, Pos { story: StoryRef::Part(id), path: wordcraft_doc::Path::top(0), off: 4 });
}

#[test]
fn text_box_in_a_table_cell_keeps_the_caret_in_the_cell() {
    // Layout doesn't show the text of boxes inside tables yet, so don't move into one.
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    let cell = s.sel.focus.clone();
    assert!(cell.path.0.len() > 1, "{cell:?}");
    run(&mut s, "insert.textBox", json!({}));
    assert_eq!(s.sel.focus.story, StoryRef::Body);
    assert_eq!(s.sel.focus.path, cell.path);
}
