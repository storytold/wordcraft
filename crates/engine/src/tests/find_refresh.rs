use super::{run, s};
use serde_json::json;
use wordcraft_doc::{Document, Pos, StoryRef};

fn offsets(s: &mut crate::Session) -> Vec<(usize, usize)> {
    s.find_results().iter().map(|(a, b)| (a.off, b.off)).collect()
}

#[test]
fn result_refresh_keeps_options_caret_and_history_through_undo_and_redo() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "Cat cat scatter cat"}));
    run(&mut s, "edit.find", json!({"text": "cat", "matchCase": true, "wholeWord": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.insert", json!({"text": "é "}));
    let before = (s.sel.clone(), s.doc.clone(), s.dirty, s.rev(), s.undo_depth(), s.pending.clone());
    assert_eq!(offsets(&mut s), [(7, 10), (19, 22)]);
    assert_eq!((s.sel.clone(), s.doc.clone(), s.dirty, s.rev(), s.undo_depth(), s.pending.clone()), before);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(offsets(&mut s), [(4, 7), (16, 19)]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(offsets(&mut s), [(7, 10), (19, 22)]);
    run(&mut s, "document.setText", json!({"text": "No matches"}));
    assert!(s.find_results().is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(offsets(&mut s), [(7, 10), (19, 22)]);
}

#[test]
fn result_refresh_uses_the_original_story_and_selection_bounds() {
    let mut s = crate::Session::new(Document::from_text("cat cat cat"));
    let part = s.doc.add_part(wordcraft_doc::PartKind::Header, Document::from_text("cat").body);
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 4), "focus": Pos::body(0, 7)}));
    run(&mut s, "edit.advancedFind", json!({"text": "cat", "in": "selection"}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": " cat"}));
    run(&mut s, "caret.set", json!({"pos": {"story": {"part": part}, "path": [0], "off": 0}}));
    assert_eq!(offsets(&mut s), [(4, 7)]);
    assert_eq!(s.find_results()[0].0.story, StoryRef::Body);
    assert_eq!(s.sel.focus.story, StoryRef::Part(part));
    run(&mut s, "edit.find", json!({"text": "cat"}));
    run(&mut s, "text.insert", json!({"text": "dog"}));
    run(&mut s, "caret.set", json!({"pos": Pos::body(0, 0)}));
    assert!(s.find_results().is_empty(), "refresh does not switch from the searched header to the body");
}

#[test]
fn result_refresh_keeps_regex_wildcards_and_each_documents_search() {
    let mut s = crate::Session::new(Document::from_text("id1 ID2 id33"));
    let first = s.document_id();
    run(&mut s, "edit.find", json!({"text": "id[0-9]+", "regex": true, "matchCase": true}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    let results = s.find_results();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|(a, _)| a.path == wordcraft_doc::Path::top(1)));
    let second = s.add_document(Document::from_text("cat cot cut"));
    run(&mut s, "edit.find", json!({"text": "c?t", "wildcards": true}));
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.insert", json!({"text": " cat"}));
    assert_eq!(s.find_results().len(), 4);
    s.activate(first).unwrap();
    assert_eq!(s.find.query, "id[0-9]+");
    assert_eq!(s.find_results().len(), 2);
    s.activate(second).unwrap();
    assert_eq!(s.find_results().len(), 4);
    assert!(s.run("edit.find", &json!({"text": "(", "regex": true, "wildcards": false})).is_err());
    assert!(s.find_results().is_empty(), "an invalid changed query cannot leave stale results visible");
}
