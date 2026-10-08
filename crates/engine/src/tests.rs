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

#[test]
fn copy_paste_text_box_makes_an_independent_copy() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "A"}));
    let id = run(&mut s, "insert.textBox", json!({"text": "Original"}))["story"].as_u64().unwrap() as u32;
    // Select "A" and the box in the body, copy, paste at the end.
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 1 + 3)}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 4)}));
    let parts = s.doc.parts.len();
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.doc.parts.len(), parts + 1);
    let stories: Vec<u32> = s
        .doc
        .para(StoryRef::Body, &wordcraft_doc::Path::top(0))
        .unwrap()
        .objects
        .iter()
        .filter_map(|o| if let wordcraft_doc::InlineObject::Shape { story, .. } = o { *story } else { None })
        .collect();
    assert_eq!(stories.len(), 2);
    assert_eq!(stories[0], id);
    let copy = stories[1];
    assert_ne!(copy, id);
    // Typing in the copy leaves the original alone.
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Part(copy), path: wordcraft_doc::Path::top(0), off: 0 }}));
    run(&mut s, "text.insert", json!({"text": "Copy of "}));
    assert_eq!(s.doc.plain_text(StoryRef::Part(copy)), "Copy of Original");
    assert_eq!(s.doc.plain_text(StoryRef::Part(id)), "Original");
    // Undo the typing and the paste: the copy's story goes too.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.parts.len(), parts);
}

#[test]
fn word_count_includes_text_boxes_by_default() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "one two "}));
    run(&mut s, "insert.textBox", json!({"text": "three four five"}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
    assert_eq!(s.word_count(), 5);
    let v = run(&mut s, "review.wordCount", json!({}));
    assert_eq!((v["words"].as_u64(), v["includeTextBoxes"].as_bool()), (Some(5), Some(true)));
    assert_eq!(v["paragraphs"].as_u64(), Some(2));
    // Turned off: body only, and it sticks.
    let v = run(&mut s, "review.wordCount", json!({"includeTextBoxes": false}));
    assert_eq!(v["words"].as_u64(), Some(2));
    assert_eq!(s.word_count(), 2);
    assert_eq!(run(&mut s, "review.wordCount", json!({}))["words"].as_u64(), Some(2));
    // Deleting the box drops its words (its story stays behind, unused).
    run(&mut s, "review.wordCount", json!({"includeTextBoxes": true}));
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 8), "focus": Pos::body(0, 8 + 3)}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(s.word_count(), 2);
}

/// The laid-out area of the only text box.
fn box_area(s: &mut Session) -> wordcraft_layout::hit::ObjectHit {
    let l = s.layout();
    l.find_object(0, |o| o.text_box.is_some()).unwrap()
}

fn select_box(s: &mut Session) {
    let o = box_area(s);
    let end = Pos { off: o.off + 3, ..o.pos() };
    run(s, "select.range", json!({"anchor": o.pos(), "focus": end}));
}

#[test]
fn arrange_bounds_resizes_and_moves_objects() {
    use wordcraft_doc::para::{Anchor, Wrap};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Some text "}));
    run(&mut s, "insert.textBox", json!({"text": "Box"}));
    select_box(&mut s);
    // Resize: stays inline, keeps a text box's minimum.
    run(&mut s, "arrange.bounds", json!({"width": 200, "height": 5}));
    let o = box_area(&mut s);
    assert_eq!((o.rect.w, o.rect.h, o.wrap), (200.0, 18.0, Wrap::Inline));
    assert!(s.run("arrange.nudge", &json!({"dx": 5})).is_err(), "inline objects don't nudge");
    // Move: floats exactly where it was dropped, still selected.
    let r = run(&mut s, "arrange.bounds", json!({"page": 0, "x": 300, "y": 400}));
    assert_eq!(r["object"]["float"]["wrap"], "square");
    let o = box_area(&mut s);
    assert_eq!((o.page, o.rect.x, o.rect.y), (0, 300.0, 400.0));
    assert_eq!(s.doc.plain_text(StoryRef::Part(o.text_box.unwrap())), "Box");
    // Moving a floating object on its page keeps its anchors and shifts its offsets.
    let set = |s: &mut Session, f: &dyn Fn(&mut wordcraft_doc::para::Float)| {
        let o = box_area(s);
        let p = s.doc.para_mut(StoryRef::Body, &o.path).unwrap();
        if let Some(wordcraft_doc::InlineObject::Shape { float, .. }) = p.object_at_mut(o.off) {
            f(float);
        }
        p.touch();
        s.touch();
    };
    set(&mut s, &|f| {
        f.h_rel = Anchor::Column;
        f.v_rel = Anchor::Paragraph;
        f.x = 10.0;
        f.y = 20.0;
    });
    let before = box_area(&mut s).rect;
    select_box(&mut s);
    run(&mut s, "arrange.bounds", json!({"x": before.x + 30.0, "y": before.y - 5.0}));
    let o = box_area(&mut s);
    assert!((o.rect.x - before.x - 30.0).abs() < 0.01 && (o.rect.y - before.y + 5.0).abs() < 0.01, "{:?} → {:?}", before, o.rect);
    let p = s.doc.para(StoryRef::Body, &o.path).unwrap();
    let Some(wordcraft_doc::InlineObject::Shape { float, .. }) = p.object_at(o.off) else { panic!() };
    assert_eq!((float.h_rel, float.v_rel, float.x, float.y), (Anchor::Column, Anchor::Paragraph, 40.0, 15.0));
    // Nudge.
    run(&mut s, "arrange.nudge", json!({"dx": 6, "dy": -1}));
    let n = box_area(&mut s).rect;
    assert!((n.x - o.rect.x - 6.0).abs() < 0.01 && (n.y - o.rect.y + 1.0).abs() < 0.01);
    // Hostile values stay on the page.
    run(&mut s, "arrange.bounds", json!({"x": -1e9, "y": 1e9, "width": 1e9}));
    let o = box_area(&mut s);
    assert!(o.rect.right() >= 12.0 && o.rect.y <= 792.0 && o.rect.w <= 4000.0, "{:?}", o.rect);
    // One undo per change.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(box_area(&mut s).rect, n);
}

#[test]
fn arrange_bounds_moves_an_object_to_another_page() {
    let mut s = s();
    for i in 0..60 {
        run(&mut s, "text.insert", json!({"text": format!("Paragraph {i} of filler text.")}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
    run(&mut s, "insert.textBox", json!({"text": "Traveller"}));
    select_box(&mut s);
    assert!(s.layout().pages.len() >= 2);
    run(&mut s, "arrange.bounds", json!({"page": 1, "x": 100, "y": 200}));
    let o = box_area(&mut s);
    assert_eq!((o.page, o.rect.x, o.rect.y), (1, 100.0, 200.0));
    assert!(o.path.0[0] > 0, "anchored to text on page 2: {:?}", o.path);
    assert_eq!(s.doc.plain_text(StoryRef::Part(o.text_box.unwrap())), "Traveller");
    // The selection follows the object.
    assert_eq!(s.sel.anchor, o.pos());
    assert!(s.run("arrange.bounds", &json!({"page": 99, "x": 1, "y": 1})).is_err());
}
