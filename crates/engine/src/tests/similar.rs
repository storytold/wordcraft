use super::{run, text};
use crate::{Selection, Session};
use serde_json::json;
use wordcraft_doc::{Document, Pos, StoryRef};

fn fixture() -> Session {
    let mut s = Session::new(Document::from_text("alpha beta gamma delta"));
    for (start, end) in [(6, 10), (17, 22)] {
        run(&mut s, "select.range", json!({"anchor": Pos::body(0, start), "focus": Pos::body(0, end)}));
        run(&mut s, "format.bold", json!({"value": true}));
    }
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 7)}));
    s
}

#[test]
fn select_similar_selects_only_first_match_and_returns_every_range() {
    let mut s = fixture();
    let before = s.doc.clone();
    let result = run(&mut s, "select.similar", json!({}));
    assert_eq!(s.sel, Selection { anchor: Pos::body(0, 6), focus: Pos::body(0, 10) });
    assert_eq!(
        result,
        json!({"runs": 2, "ranges": [
            {"anchor": Pos::body(0, 6), "focus": Pos::body(0, 10)},
            {"anchor": Pos::body(0, 17), "focus": Pos::body(0, 22)}
        ]})
    );
    assert!(s.doc.same_content(&before), "selection is not a document edit");
    run(&mut s, "format.italic", json!({"value": true}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(6).italic, Some(true));
    assert_ne!(p.props_of_char(11).italic, Some(true), "gamma must stay plain");
    assert_ne!(p.props_of_char(17).italic, Some(true), "other matches are returned, not selected");
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.same_content(&before));
    run(&mut s, "edit.redo", json!({}));
    for range in result["ranges"].as_array().unwrap() {
        run(&mut s, "select.range", range.clone());
        run(&mut s, "format.italic", json!({"value": true}));
    }
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(6).italic, Some(true));
    assert_eq!(p.props_of_char(17).italic, Some(true));
    assert_ne!(p.props_of_char(11).italic, Some(true));
}

#[test]
fn select_similar_copy_and_delete_leave_nonmatching_text() {
    let mut s = fixture();
    run(&mut s, "select.similar", json!({}));
    assert_eq!(run(&mut s, "edit.copy", json!({}))["text"], "beta");
    run(&mut s, "text.delete", json!({}));
    assert_eq!(text(&s), "alpha  gamma delta");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "alpha beta gamma delta");
}

#[test]
fn select_similar_ranges_keep_unicode_bytes_paragraph_order_and_story() {
    let mut s = Session::new(Document::from_text("αβ gap\nγδ gap"));
    for path in [0, 1] {
        run(&mut s, "select.range", json!({"anchor": Pos::body(path, 0), "focus": Pos::body(path, 4)}));
        run(&mut s, "format.bold", json!({"value": true}));
    }
    // A reversed, mixed selection samples its ordered start, as before.
    run(&mut s, "select.range", json!({"anchor": Pos::body(1, 7), "focus": Pos::body(0, 2)}));
    let r = run(&mut s, "select.similar", json!({}));
    assert_eq!(r["runs"], 2);
    assert_eq!(
        r["ranges"],
        json!([
            {"anchor": Pos::body(0, 0), "focus": Pos::body(0, 4)},
            {"anchor": Pos::body(1, 0), "focus": Pos::body(1, 4)}
        ])
    );
    assert_eq!(s.sel, Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 4) });
    let header = s.doc.add_part(wordcraft_doc::PartKind::Header, Document::from_text("Header only").body);
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Part(header), path: wordcraft_doc::Path::top(0), off: 0 }}));
    let r = run(&mut s, "select.similar", json!({}));
    assert_eq!(r["runs"], 1);
    assert_eq!(r["ranges"][0]["anchor"]["story"], serde_json::to_value(StoryRef::Part(header)).unwrap());
    assert_eq!(s.sel.focus.story, StoryRef::Part(header));
}

#[test]
fn select_similar_empty_story_keeps_caret_and_returns_no_ranges() {
    let mut s = Session::new(Document::new());
    let before = s.sel.clone();
    assert_eq!(run(&mut s, "select.similar", json!({})), json!({"runs": 0, "ranges": []}));
    assert_eq!(s.sel, before);
}

#[test]
fn select_similar_replaces_column_state_even_when_the_first_range_has_the_same_corners() {
    let mut s = fixture();
    s.sel = Selection { anchor: Pos::body(0, 6), focus: Pos::body(0, 10) };
    run(&mut s, "select.column", json!({"anchor": Pos::body(0, 6), "focus": Pos::body(0, 10)}));
    s.column.as_mut().unwrap().segments = vec![(Pos::body(0, 6), Pos::body(0, 10)), (Pos::body(0, 17), Pos::body(0, 22))];
    s.column_mode = true;
    s.pending = Some(wordcraft_doc::props::CharProps { italic: Some(true), ..Default::default() });
    run(&mut s, "select.similar", json!({}));
    assert!(s.column.is_none());
    assert!(!s.column_mode);
    assert!(s.pending.is_none());
    run(&mut s, "format.italic", json!({"value": true}));
    assert_ne!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(17).italic, Some(true));
}

#[test]
fn select_similar_returns_body_and_table_matches_in_story_order() {
    let mut doc = Document::from_text("body");
    let mut table = wordcraft_doc::Table::new(1, 1, 200.0);
    table.rows[0].cells[0] = wordcraft_doc::Cell::with_text("cell");
    doc.body.push(std::sync::Arc::new(wordcraft_doc::Block::Table(table)));
    let cell = Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, 0, 0, 0]), off: 1 };
    let mut s = Session::new(doc);
    s.sel = Selection::caret(cell.clone());
    let r = run(&mut s, "select.similar", json!({}));
    assert_eq!(r["runs"], 2);
    assert_eq!(r["ranges"][0], json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 4)}));
    assert_eq!(r["ranges"][1], json!({"anchor": Pos { off: 0, ..cell.clone() }, "focus": Pos { off: 4, ..cell }}));
    assert_eq!(s.sel, Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 4) });
}

#[test]
fn select_similar_ranges_can_be_deleted_in_reverse_without_touching_nonmatches() {
    let mut s = fixture();
    let result = run(&mut s, "select.similar", json!({}));
    for range in result["ranges"].as_array().unwrap().iter().rev() {
        run(&mut s, "select.range", range.clone());
        run(&mut s, "text.delete", json!({}));
    }
    assert_eq!(text(&s), "alpha  gamma ");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "alpha beta gamma ");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "alpha  gamma ");
}
