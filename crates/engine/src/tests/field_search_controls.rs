use super::{run, s, text};
use serde_json::json;
use wordcraft_doc::{CharProps, InlineObject, Pos, StoryRef};

fn field(s: &mut crate::Session, off: usize, result: &str) {
    s.doc
        .para_mut(StoryRef::Body, &Pos::body(0, 0).path)
        .unwrap()
        .insert_object(
            off,
            InlineObject::Field {
                instr: "CITATION OriginalSource".into(),
                result: result.into(),
                locked: true,
                dirty: true,
                code: false,
                form: String::new(),
            },
            &CharProps::default(),
        )
        .unwrap();
}

#[test]
fn field_search_controls_maps_results_to_atomic_ranges() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "é lead  tail [7]"}));
    field(&mut s, 8, "[7] [7]");
    let before = serde_json::to_value(&s.doc).unwrap();
    let r = run(&mut s, "edit.find", json!({"text": "[7]"}));
    assert_eq!(r["count"], 2, "a matching atomic field is one navigable result");
    assert_eq!(r["matches"][0]["start"]["off"], 8);
    assert_eq!(r["matches"][0]["end"]["off"], 11);
    assert_eq!(s.sel.ordered(), (Pos::body(0, 8), Pos::body(0, 11)));
    assert_eq!(r["matches"][1]["start"]["off"], 17);
    run(&mut s, "edit.findNext", json!({}));
    assert_eq!(s.sel.ordered(), (Pos::body(0, 17), Pos::body(0, 20)));
    run(&mut s, "edit.findPrevious", json!({}));
    assert_eq!(s.sel.ordered(), (Pos::body(0, 8), Pos::body(0, 11)));
    assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "OriginalSource"}))["count"], 0);
}

#[test]
fn field_search_controls_obeys_options_neighbors_and_object_search() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "prepost"}));
    field(&mut s, 3, "Cat é");
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat"}))["count"], 1);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat", "matchCase": true}))["count"], 0);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "Cat", "wholeWord": true}))["count"], 0, "the preceding live text joins the word");
    let r = run(&mut s, "edit.find", json!({"text": "preCat épost", "wholeWord": false}));
    assert_eq!(r["count"], 1);
    assert_eq!(r["matches"][0]["end"]["off"], 10);
    let r = run(&mut s, "edit.find", json!({"text": "é", "special": false}));
    assert_eq!(r["count"], 1);
    assert_eq!(r["matches"][0]["start"]["off"], 3);
    assert_eq!(r["matches"][0]["end"]["off"], 6);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "C.t", "regex": true}))["count"], 1);
}

#[test]
fn field_search_controls_skipped_replacements_preserve_history() {
    for track in [false, true] {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": "plain   "}));
        field(&mut s, 8, "[7]");
        run(&mut s, "review.trackChanges", json!({"value": track}));
        run(&mut s, "caret.docEnd", json!({}));
        run(&mut s, "text.insert", json!({"text": " edit"}));
        run(&mut s, "edit.undo", json!({}));
        let before = serde_json::to_value(&s.doc).unwrap();
        let undo = s.undo_depth();
        let result = run(&mut s, "edit.replaceAll", json!({"text": "[7]", "with": "changed"}));
        assert_eq!(result["replaced"], 0);
        assert_eq!(result["skippedInFields"], 1);
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
        assert_eq!(s.undo_depth(), undo);
        run(&mut s, "select.range", json!({"anchor":{"block":0,"off":8},"focus":{"block":0,"off":8}}));
        run(&mut s, "edit.find", json!({"text": "[7]"}));
        assert_eq!(s.sel.ordered(), (Pos::body(0, 8), Pos::body(0, 11)));
        let selection = s.sel.clone();
        run(&mut s, "edit.replace", json!({"with": "changed"}));
        assert_eq!(s.sel, selection);
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
        run(&mut s, "edit.redo", json!({}));
        assert!(text(&s).ends_with(" edit"), "the skipped replacement preserves Redo");
    }
}

#[test]
fn field_search_controls_skips_tracked_deletions_and_bounds_selection() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "live "}));
    field(&mut s, 5, "hidden");
    run(&mut s, "select.range", json!({"anchor":{"block":0,"off":5},"focus":{"block":0,"off":8}}));
    assert_eq!(run(&mut s, "edit.advancedFind", json!({"text":"hidden","in":"selection"}))["count"], 1);
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "live hidden");
    assert_eq!(run(&mut s, "edit.find", json!({"text":"hidden"}))["count"], 0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "edit.find", json!({"text":"hidden"}))["count"], 1);
}
