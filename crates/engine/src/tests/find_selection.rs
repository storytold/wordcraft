//! Replace All in Selection shares the normal replacement path, but never widens its scope.

use serde_json::{Value, json};
use wordcraft_doc::{Document, PartKind, Path, Pos, StoryRef};

use super::{body_table_cells, run, text};
use crate::{ColumnBlock, Selection, Session};

fn selected(text: &str, anchor: Pos, focus: Pos) -> Session {
    let mut s = Session::new(Document::from_text(text));
    s.sel = Selection { anchor, focus };
    s
}

fn scoped(s: &mut Session, command: &str, mut opts: Value) -> Value {
    if command == "edit.replaceAll" {
        opts["in"] = json!("selection");
    }
    run(s, command, opts)
}

#[test]
fn replace_selection_is_one_undo_step_in_either_direction_and_api() {
    for command in ["edit.replaceAll", "edit.replaceAllInSelection"] {
        for reverse in [false, true] {
            let mut s = selected("cat cat cat cat", Pos::body(0, 4), Pos::body(0, 11));
            if reverse {
                std::mem::swap(&mut s.sel.anchor, &mut s.sel.focus);
            }
            let before = s.doc.clone();
            let selection = s.sel.clone();
            let depth = s.undo_depth();
            assert_eq!(scoped(&mut s, command, json!({"text": "cat", "with": "kitten"}))["replaced"], 2);
            assert_eq!(text(&s), "cat kitten kitten cat");
            assert_eq!(s.undo_depth(), depth + 1);
            assert_eq!(s.sel, Selection::caret(Pos::body(0, 4)));
            let after = s.doc.clone();
            assert!(s.undo());
            assert_eq!(s.doc, before);
            assert_eq!(s.sel, selection);
            assert!(s.redo());
            assert_eq!(s.doc, after);
            let bytes = crate::io::save_bytes("selection.docx", &s.doc).unwrap();
            assert_eq!(crate::io::open_bytes("selection.docx", &bytes).unwrap().plain_text(StoryRef::Body), "cat kitten kitten cat");
        }
    }
}

#[test]
fn replace_selection_excludes_partial_matches_and_keeps_unicode_boundaries() {
    let original = "🙂cat cat cat猫";
    let mut s = selected(original, Pos::body(0, "🙂c".len()), Pos::body(0, "🙂cat cat ca".len()));
    assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "🐈"}))["replaced"], 1);
    assert_eq!(text(&s), "🙂cat 🐈 cat猫");
    assert!(s.undo());
    assert_eq!(text(&s), original);
}

#[test]
fn replace_selection_keeps_word_and_anchor_context() {
    let mut s = selected("scat cat", Pos::body(0, 1), Pos::body(0, 4));
    for opts in [json!({"text": "cat", "wholeWord": true}), json!({"text": "^cat$", "regex": true, "wholeWord": false})] {
        let mut opts = opts;
        opts["with"] = json!("fox");
        assert_eq!(scoped(&mut s, "edit.replaceAll", opts)["replaced"], 0);
        assert_eq!(text(&s), "scat cat");
        assert!(!s.dirty && !s.can_undo());
    }
}

#[test]
fn replace_selection_limits_greedy_and_overlapping_matches_before_searching() {
    for (original, start, end, opts, want) in [
        ("aaaa aaaa", 1, 3, json!({"text": "a+", "with": "x", "regex": true}), "axa aaaa"),
        ("catcatcat", 3, 9, json!({"text": "catcat", "with": "fox"}), "catfox"),
        ("猫猫猫", 3, 6, json!({"text": ".+", "with": "狐", "regex": true}), "猫狐猫"),
        ("Cat cat cat", 0, 7, json!({"text": "cat", "with": "fox", "matchCase": true}), "Cat fox cat"),
    ] {
        let mut s = selected(original, Pos::body(0, start), Pos::body(0, end));
        assert_eq!(scoped(&mut s, "edit.replaceAll", opts)["replaced"], 1, "{original}");
        assert_eq!(text(&s), want);
    }
}

#[test]
fn replace_selection_supports_regex_wildcards_and_special_replacements() {
    for (opts, want) in [
        (json!({"text": "John", "with": "Jack"}), "Doe, Jane; Smith, Jack; Doe, Jane"),
        (json!({"text": r"(\w+), (\w+)", "with": "$2 $1", "regex": true}), "Doe, Jane; John Smith; Doe, Jane"),
        (json!({"text": "([A-Z][a-z]@), ([A-Z][a-z]@)", "with": r"\2 \1", "wildcards": true}), "Doe, Jane; John Smith; Doe, Jane"),
        (json!({"text": "Smith, John", "with": "[^&]^t^c"}), "Doe, Jane; [Smith, John]\tOK; Doe, Jane"),
    ] {
        let mut s = selected("Doe, Jane; Smith, John; Doe, Jane", Pos::body(0, 11), Pos::body(0, 22));
        s.clipboard_text = "OK".into();
        assert_eq!(scoped(&mut s, "edit.replaceAllInSelection", opts)["replaced"], 1);
        assert_eq!(text(&s), want);
    }
}

#[test]
fn replace_selection_splits_and_joins_only_selected_paragraphs() {
    for (query, with, want, count) in [
        ("cat", "a^pb", "cat\na\nb\na\nb\ncat", 2),
        ("^p", "|", "cat\ncat|cat\ncat", 1),
        ("cat^pcat", "fox", "cat\nfox\ncat", 1),
        ("cat", "", "cat\n\n\ncat", 2),
    ] {
        let mut s = selected("cat\ncat\ncat\ncat", Pos::body(1, 0), Pos::body(2, 3));
        let before = s.doc.clone();
        assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": query, "with": with}))["replaced"], count);
        assert_eq!(text(&s), want);
        assert!(s.undo());
        assert_eq!(s.doc, before);
    }
}

#[test]
fn replace_selection_tracks_paragraph_splits_and_joins() {
    for (query, with, want) in [("cat", "a^pb", "cat\na\nb\na\nb\ncat"), ("^p", "|", "cat\ncat|cat\ncat"), ("cat^pcat", "fox", "cat\nfox\ncat")] {
        let mut s = selected("cat\ncat\ncat\ncat", Pos::body(1, 0), Pos::body(2, 3));
        s.doc.settings.track_changes = true;
        scoped(&mut s, "edit.replaceAll", json!({"text": query, "with": with}));
        run(&mut s, "review.acceptAll", json!({}));
        assert_eq!(text(&s), want, "{query} → {with}");
    }
}

#[test]
fn replace_selection_preserves_character_formatting() {
    let mut s = selected("cat cat cat", Pos::body(0, 4), Pos::body(0, 7));
    run(&mut s, "format.bold", json!({"value": true}));
    let before = s.doc.clone();
    assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "kitten"}))["replaced"], 1);
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(4).bold, Some(true));
    assert_ne!(p.props_of_char(0).bold, Some(true));
    assert_ne!(p.props_of_char(11).bold, Some(true));
    assert!(s.undo());
    assert_eq!(s.doc, before);
}

#[test]
fn replace_selection_tracks_only_selected_matches_and_skips_deleted_text() {
    let mut s = selected("cat cat cat", Pos::body(0, 4), Pos::body(0, 7));
    s.doc.settings.track_changes = true;
    let before = s.doc.clone();
    scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "kitten"}));
    assert_eq!(s.doc.final_text(StoryRef::Body), "cat kitten cat");
    assert!(!run(&mut s, "review.changes", json!({})).as_array().unwrap().is_empty());
    let after = s.doc.clone();
    assert!(s.undo());
    assert_eq!(s.doc, before);
    assert!(s.redo());
    assert_eq!(s.doc, after);
    run(&mut s, "select.all", json!({}));
    assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox"}))["replaced"], 2, "the deleted middle cat is not a match");
    assert_eq!(s.doc.final_text(StoryRef::Body), "fox kitten fox");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(text(&s), "cat cat cat");
}

#[test]
fn replace_selection_respects_locks_without_checking_matches_outside_scope() {
    for track_changes in [false, true] {
        let mut s = Session::new(Document::from_text("cat cat cat"));
        run(&mut s, "edit.find", json!({"text": "cat"}));
        run(&mut s, "developer.richText", json!({"tag": "protected"}));
        run(&mut s, "developer.properties", json!({"control": "protected", "lockEdit": true}));
        s.doc.settings.track_changes = track_changes;
        run(&mut s, "select.all", json!({}));
        let before = s.doc.clone();
        let selection = s.sel.clone();
        let depth = s.undo_depth();
        assert!(s.run("edit.replaceAllInSelection", &json!({"text": "cat", "with": "kitten"})).is_err());
        assert_eq!(s.doc, before);
        assert_eq!(s.sel, selection);
        assert_eq!(s.undo_depth(), depth);
        let at = s.doc.para_at(&Pos::body(0, 0)).unwrap().text.rfind("cat").unwrap();
        s.sel = Selection { anchor: Pos::body(0, at), focus: Pos::body(0, at + 3) };
        assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "kitten"}))["replaced"], 1);
        assert_eq!(s.doc.final_text(StoryRef::Body), "cat cat kitten");
    }
}

#[test]
fn replace_selection_uses_actual_block_segments_and_undo_restores_them() {
    for command in ["edit.replaceAll", "edit.replaceAllInSelection"] {
        let mut s = selected("cat cat\ncat cat", Pos::body(0, 0), Pos::body(1, 3));
        s.column = Some(ColumnBlock {
            sel: s.sel.clone(),
            anchor_x: 0.0,
            focus_x: 20.0,
            segments: vec![(Pos::body(0, 0), Pos::body(0, 3)), (Pos::body(1, 0), Pos::body(1, 3))],
            cells: None,
        });
        let block = s.column.clone();
        assert_eq!(scoped(&mut s, command, json!({"text": "cat", "with": "kitten"}))["replaced"], 2);
        assert_eq!(text(&s), "kitten cat\nkitten cat");
        assert!(s.column.is_none());
        assert!(s.undo());
        assert_eq!(text(&s), "cat cat\ncat cat");
        assert_eq!(s.column, block);
    }
}

#[test]
fn replace_selection_leaves_other_table_columns_alone() {
    let mut s = Session::new(Document::new());
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    for i in 0..4 {
        if i > 0 {
            run(&mut s, "text.tab", json!({}));
        }
        run(&mut s, "text.insert", json!({"text": "cat"}));
    }
    run(&mut s, "table.selectColumn", json!({}));
    assert_eq!(s.column_segments().unwrap().len(), 2);
    let before = s.doc.clone();
    let column = s.column.clone();
    assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox"}))["replaced"], 2);
    assert_eq!(body_table_cells(&s), vec![vec!["cat", "fox", "cat", "fox"]]);
    assert!(s.undo());
    assert_eq!(s.doc, before);
    assert_eq!(s.column, column);
}

#[test]
fn replace_selection_can_edit_a_secondary_story_without_changing_the_body() {
    for kind in [PartKind::Header, PartKind::Footer, PartKind::Footnote, PartKind::Endnote] {
        let mut s = Session::new(Document::from_text("cat cat cat"));
        let part = s.doc.add_part(kind, Document::from_text("cat cat cat").body);
        let story = StoryRef::Part(part);
        let pos = |off| Pos { story, path: Path::top(0), off };
        s.sel = Selection { anchor: pos(4), focus: pos(7) };
        assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox"}))["replaced"], 1);
        assert_eq!(s.doc.plain_text(story), "cat fox cat");
        assert_eq!(text(&s), "cat cat cat");
    }
}

#[test]
fn replace_selection_in_a_text_box_leaves_its_anchor_and_body_alone() {
    let mut s = Session::new(Document::from_text("cat cat cat"));
    let part = run(&mut s, "insert.textBox", json!({"text": "cat cat cat"}))["story"].as_u64().unwrap() as u32;
    let body = s.doc.body.clone();
    let story = StoryRef::Part(part);
    s.sel = Selection { anchor: Pos { story, path: Path::top(0), off: 4 }, focus: Pos { story, path: Path::top(0), off: 7 } };
    assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox"}))["replaced"], 1);
    assert_eq!(s.doc.plain_text(story), "cat fox cat");
    assert_eq!(s.doc.body, body);
}

#[test]
fn replace_selection_does_not_spend_the_match_limit_on_unselected_text() {
    let original = "cat ".repeat(100_010);
    let end = original.len() - 1;
    let mut s = selected(&original, Pos::body(0, end - 3), Pos::body(0, end));
    assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox"}))["replaced"], 1);
    assert_eq!(text(&s), format!("{}fox ", "cat ".repeat(100_009)));
}

#[test]
fn replace_selection_empty_invalid_and_no_match_never_widen_the_edit() {
    let mut s = selected("é cat cat", Pos::body(0, 3), Pos::body(0, 6));
    scoped(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox"}));
    assert!(s.undo());
    let before = s.doc.clone();
    let selection = s.sel.clone();
    let rev = s.rev();
    let dirty = s.dirty;
    assert_eq!(scoped(&mut s, "edit.replaceAll", json!({"text": "dog", "with": "fox"}))["replaced"], 0);
    assert_eq!(s.sel, selection);
    assert_eq!(s.rev(), rev);
    assert_eq!(s.dirty, dirty);
    assert!(s.can_redo());
    for scope in [json!(null), json!(true), json!(0), json!([]), json!("elsewhere")] {
        assert!(s.run("edit.replaceAll", &json!({"text": "cat", "with": "fox", "in": scope})).is_err());
        assert_eq!(s.doc, before);
        assert!(s.can_redo());
    }
    for selection in [
        Selection::caret(Pos::body(0, 0)),
        Selection { anchor: Pos::body(0, 1), focus: Pos::body(0, 6) }, // middle of é
        Selection { anchor: Pos::body(999, 0), focus: Pos::body(0, 6) },
        Selection { anchor: Pos::body(0, 0), focus: Pos { story: StoryRef::Part(999), path: Path::top(0), off: 6 } },
    ] {
        s.sel = selection;
        assert!(s.run("edit.replaceAll", &json!({"text": "cat", "with": "fox", "in": "selection"})).is_err());
        assert_eq!(s.doc, before);
        assert!(s.can_redo());
    }
}

#[test]
fn replace_selection_refuses_an_empty_column_instead_of_using_its_corners() {
    let mut s = selected("cat\ncat", Pos::body(0, 0), Pos::body(1, 0));
    s.column = Some(ColumnBlock {
        sel: s.sel.clone(),
        anchor_x: 0.0,
        focus_x: 0.0,
        segments: vec![(Pos::body(0, 0), Pos::body(0, 0)), (Pos::body(1, 0), Pos::body(1, 0))],
        cells: None,
    });
    let before = s.doc.clone();
    assert!(s.why_disabled("edit.replaceAllInSelection").is_some());
    assert!(s.run("edit.replaceAllInSelection", &json!({"text": "cat", "with": "fox"})).is_err());
    assert!(s.run("edit.replaceAll", &json!({"text": "cat", "with": "fox", "in": "selection"})).is_err());
    assert_eq!(s.doc, before);
    assert!(!s.dirty && !s.can_undo());
}

#[test]
fn replace_all_without_scope_keeps_its_existing_story_wide_behavior() {
    for scope in [None, Some("story")] {
        let mut s = selected("cat cat cat", Pos::body(0, 4), Pos::body(0, 7));
        let mut opts = json!({"text": "cat", "with": "fox"});
        if let Some(scope) = scope {
            opts["in"] = json!(scope);
        }
        assert_eq!(run(&mut s, "edit.replaceAll", opts)["replaced"], 3);
        assert_eq!(text(&s), "fox fox fox");
    }
}
