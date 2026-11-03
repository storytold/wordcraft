use super::*;
use wordcraft_doc::{Document, StoryRef};

fn apply(old: &str, edits: &[Edit]) -> String {
    let mut out = old.to_string();
    for e in edits.iter().rev() {
        out.replace_range(e.start..e.end, &e.ins);
    }
    out
}

#[test]
fn differences_are_small_edits_that_make_the_new_text() {
    let old = "The quiet harbour opens at dawn, and the boats leave at six.";
    let new = "The quiet harbour opens at sunrise, and the boats leave at seven.";
    let (edits, whole) = plan(old, new, false);
    assert!(!whole);
    assert_eq!(edits.iter().map(|e| (&old[e.start..e.end], e.ins.as_str())).collect::<Vec<_>>(), [("dawn", "sunrise"), ("six", "seven")]);
    assert_eq!(apply(old, &edits), new);
    // Chinese by character: 周三下午 → 周四上午 changes 三 and 下.
    let (edits, _) = plan("会议改到周三下午", "会议改到周四上午", false);
    assert_eq!(edits.len(), 1, "one replacement: {edits:?}");
    assert_eq!(apply("会议改到周三下午", &edits), "会议改到周四上午");
    // A pure insertion and a pure deletion.
    let (edits, _) = plan("a b", "a very b", false);
    assert_eq!(apply("a b", &edits), "a very b");
    let (edits, _) = plan("one two three", "one three", false);
    assert_eq!(apply("one two three", &edits), "one three");
    // Texts that share almost nothing are replaced whole.
    let (edits, whole) = plan("alpha beta gamma", "completely different words", false);
    assert!(whole && edits.len() == 1);
    // Nothing to change.
    assert!(plan("same", "same", false).0.is_empty());
}

#[test]
fn any_two_texts_edit_into_each_other() {
    let words = ["", "a", "ab cd", "x y z", "你好，世界", "it's", "e\u{301}clair", "  two  spaces ", "x.y,z"];
    for old in words {
        for new in words {
            for chars in [false, true] {
                let (edits, _) = plan(old, new, chars);
                assert_eq!(apply(old, &edits), new, "{old:?} → {new:?} (chars {chars})");
            }
        }
    }
}

#[test]
fn a_rewrite_with_track_changes_is_small_reviewable_changes() {
    let old = "The quiet harbour opens at dawn, and the boats leave at six.";
    let new = "The quiet harbour opens at sunrise, and the boats leave at seven.";
    for track in [true, false] {
        let mut s = Session::new(Document::from_text(old));
        // Bold the first words: unchanged text must keep its formatting.
        s.run("select.text", &json!({"text": "quiet harbour"})).unwrap();
        s.run("format.bold", &json!({})).unwrap();
        if track {
            s.run("review.trackChanges", &json!({"on": true})).unwrap();
        }
        let depth = s.undo_depth();
        let r = s.run("text.rewrite", &json!({"text": new, "paragraph": 0})).unwrap();
        assert_eq!(r["changes"], 2, "{r}");
        assert_eq!(s.undo_depth(), depth + 1, "one undo step");
        let bold = s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(old.find("quiet").unwrap()).bold;
        assert_eq!(bold, Some(true), "unchanged text keeps its formatting (track {track})");
        if track {
            // Only the two words that changed are deleted and the two new ones inserted; the
            // rest of the sentence carries no revision.
            let para = s.doc.para_at(&Pos::body(0, 0)).unwrap().clone();
            let text = para.text.clone();
            let marked = |word: &str, del: bool| {
                let at = text.find(word).unwrap_or_else(|| panic!("{word} in {text}"));
                (at..at + word.len())
                    .all(|i| !text.is_char_boundary(i) || if del { para.props_of_char(i).del.is_some() } else { para.props_of_char(i).ins.is_some() })
            };
            assert!(marked("dawn", true) && marked("six", true), "{text}");
            assert!(marked("sunrise", false) && marked("seven", false), "{text}");
            assert!(para.props_of_char(text.find("quiet").unwrap()).del.is_none() && para.props_of_char(text.find("quiet").unwrap()).ins.is_none());
            s.run("review.rejectAll", &json!({})).unwrap();
            assert_eq!(s.doc.plain_text(StoryRef::Body), old);
            s.run("edit.undo", &json!({})).unwrap();
            s.run("edit.undo", &json!({})).unwrap();
            s.run("review.trackChanges", &json!({"on": true})).unwrap();
            s.run("text.rewrite", &json!({"text": new, "paragraph": 0})).unwrap();
            s.run("review.acceptAll", &json!({})).unwrap();
            assert_eq!(s.doc.plain_text(StoryRef::Body), new);
        } else {
            assert_eq!(s.doc.plain_text(StoryRef::Body), new);
            assert!(s.doc.revisions.is_empty());
        }
    }
}

#[test]
fn rewrite_refuses_what_it_cannot_diff_safely_and_changes_nothing() {
    let mut s = Session::new(Document::from_text("One.\nTwo."));
    let before = s.doc.clone();
    let depth = s.undo_depth();
    for params in [
        json!({"text": "a\nb", "paragraph": 0}),
        json!({"text": 5}),
        json!({"text": "x", "paragraph": 9}),
        json!({"text": "x", "level": "line", "paragraph": 0}),
    ] {
        assert!(s.run("text.rewrite", &params).is_err(), "{params}");
    }
    s.run("select.all", &json!({})).unwrap();
    assert!(s.run("text.rewrite", &json!({"text": "x"})).is_err(), "a selection across paragraphs");
    assert_eq!(s.doc, before);
    assert_eq!(s.undo_depth(), depth);
    // The same text: no change, no undo step.
    s.run("caret.docStart", &json!({})).unwrap();
    let depth = s.undo_depth();
    assert_eq!(s.run("text.rewrite", &json!({"text": "One."})).unwrap()["changes"], 0);
    assert_eq!(s.undo_depth(), depth, "no undo step for no change");
}
