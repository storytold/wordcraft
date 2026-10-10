use serde_json::json;
use wordcraft_doc::props::BorderStyle;
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
fn doc_extension_dispatches_to_docbin() {
    // Garbage bytes with a .doc name must produce an error through the Word 97-2003
    // reader — never a panic and never a silently empty document.
    for name in ["x.doc", "x.dot"] {
        assert!(crate::io::open_bytes(name, &[0u8; 64]).is_err(), "{name} should fail");
    }
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
fn joined_commands_are_one_undo_step() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "para.indents", json!({"left": 9.0}));
    run(&mut s, "para.indents", json!({"left": 18.0}));
    for x in [27.0, 36.0] {
        s.join_next_undo();
        run(&mut s, "para.indents", json!({"left": x}));
    }
    let indent = |s: &Session| s.doc.para_at(&s.sel.focus).and_then(|p| p.props.indent_left);
    assert_eq!(indent(&s), Some(36.0));
    // One undo reverts the whole second drag, not just its last frame.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(indent(&s), Some(9.0));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(indent(&s), None);
    assert_eq!(text(&s), "Hello");
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(indent(&s), Some(36.0));
}

#[test]
fn restore_takes_a_command_back_exactly() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "alpha beta"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "gamma"}));
    run(&mut s, "edit.undo", json!({}));
    assert!(s.can_redo());
    let (doc, sel, labels, depth, rev) = (s.doc.clone(), s.sel.clone(), s.undo_labels(), s.undo_depth(), s.rev());
    s.dirty = false;
    let snap = s.edit_snapshot();
    assert_eq!(snap.doc(), &doc);
    assert_eq!(snap.undo_depth(), depth);
    assert!(!snap.dirty());
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({}));
    assert_ne!(s.doc, doc);
    assert_eq!(s.undo_depth(), depth + 1);
    assert!(!s.can_redo(), "a new command clears redo");
    s.restore(snap);
    assert_eq!(s.doc, doc);
    assert_eq!(s.sel, sel);
    assert_eq!(s.undo_labels(), labels, "the command's undo step is gone");
    assert!(s.can_redo(), "redo is back");
    assert!(!s.dirty);
    assert!(s.rev() > rev, "the layout is recomputed");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "alpha beta\ngamma");
}

#[test]
fn restore_is_exact_at_the_undo_limit() {
    // 510 steps, each with its own indent: the stack holds the newest 500.
    let build = || {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "alpha"}));
        for i in 0..510 {
            run(&mut s, "para.indents", json!({"left": f64::from(i)}));
        }
        s
    };
    // What undo walks through: (label, indent) for every step, newest first.
    let walk = |s: &mut Session| {
        let mut steps = Vec::new();
        while let Some(label) = s.undo_label().map(str::to_string) {
            s.undo();
            steps.push((label, s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props.indent_left)));
        }
        steps
    };
    let expected = walk(&mut build());
    assert_eq!(expected.len(), 500);
    // One command, and several (each pushes the oldest step out of the full stack).
    for n in [1, 2, 4] {
        let mut s = build();
        let snap = s.edit_snapshot();
        for _ in 0..n {
            run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
        }
        assert_eq!(s.undo_label(), Some("Table"));
        assert_eq!(s.undo_depth(), 500);
        s.restore(snap);
        assert_eq!(walk(&mut s), expected, "{n} command(s): the oldest steps are back, the commands' steps are gone");
    }
}

#[test]
fn restore_keeps_the_open_undo_step() {
    // Typing in progress: text typed after the restore joins the same undo step.
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "alpha"}));
    let depth = s.undo_depth();
    let snap = s.edit_snapshot();
    run(&mut s, "para.alignCenter", json!({}));
    s.restore(snap);
    run(&mut s, "text.insert", json!({"text": " beta"}));
    assert_eq!(s.undo_depth(), depth, "still one typing step");
    // A drag in progress (`join_next_undo`): its next frame joins the same undo step.
    run(&mut s, "para.indents", json!({"left": 9.0}));
    s.join_next_undo();
    let depth = s.undo_depth();
    let snap = s.edit_snapshot();
    run(&mut s, "para.indents", json!({"left": 18.0}));
    s.restore(snap);
    run(&mut s, "para.indents", json!({"left": 27.0}));
    assert_eq!(s.undo_depth(), depth, "still one drag step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props.indent_left), None);
}

#[test]
fn failed_command_keeps_undo_and_redo() {
    // A command that fails after its undo checkpoint leaves the history exactly as it was:
    // the redo stack survives, and no empty undo step is added.
    for (id, params) in [("text.insert", json!({})), ("para.align", json!({"value": "bogus"}))] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "Hello"}));
        run(&mut s, "edit.undo", json!({}));
        assert!(s.can_redo());
        assert!(s.run(id, &params).is_err(), "{id} should fail");
        assert!(s.can_redo(), "{id}: redo lost");
        assert!(!s.can_undo(), "{id}: failed command left an undo step");
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(text(&s), "Hello", "{id}");
    }

    // At the undo limit, the oldest step is not evicted by a command that fails.
    let mut s = s();
    let steps = 600;
    for i in 1..=steps {
        run(&mut s, "para.indents", json!({"left": i as f32}));
    }
    let kept = s.undo_labels().len();
    assert!(kept < steps, "the history should be at its limit");
    assert!(s.run("para.align", &json!({"value": "bogus"})).is_err());
    assert_eq!(s.undo_labels().len(), kept);
    while s.can_undo() {
        run(&mut s, "edit.undo", json!({}));
    }
    let indent = s.doc.para_at(&s.sel.focus).and_then(|p| p.props.indent_left);
    assert_eq!(indent, Some((steps - kept) as f32));
}

#[test]
fn failed_command_restores_history_changed_by_nested_commands() {
    // `file.inspect` runs `review.deleteComment` (allowed in a comments-only document, and it
    // checkpoints), then `review.acceptAll` (refused). The refusal must leave the undo and
    // redo stacks exactly as they were, including at and next to the history limit, where the
    // outer and nested checkpoints each evict the oldest step.
    fn timeline(s: &mut Session) -> Vec<(Vec<String>, wordcraft_doc::Document)> {
        let mut out = Vec::new();
        while s.can_redo() {
            run(s, "edit.redo", json!({}));
        }
        loop {
            // Comments are stamped with the time to the second, and the two sessions compared
            // are built a moment apart, so leave the stamp out of the comparison.
            let mut doc = s.doc.clone();
            doc.comments.values_mut().for_each(|c| c.date.clear());
            out.push((s.undo_labels(), doc));
            if !s.can_undo() {
                return out;
            }
            run(s, "edit.undo", json!({}));
        }
    }
    let kept = {
        let mut s = s();
        for i in 1..=600 {
            run(&mut s, "para.indents", json!({"left": i as f32}));
        }
        s.undo_labels().len()
    };
    // Below the limit there is room for a redo step too; at the limit an Undo would leave one free.
    for (len, redo) in [(kept - 1, Some("New Comment")), (kept, None)] {
        let session = || {
            let mut s = s();
            run(&mut s, "text.insert", json!({"text": "note"}));
            for i in 1..=len + usize::from(redo.is_some()) - 4 {
                run(&mut s, "para.indents", json!({"left": i as f32}));
            }
            run(&mut s, "review.restrict", json!({"mode": "comments"}));
            run(&mut s, "review.newComment", json!({"text": "one"}));
            run(&mut s, "review.newComment", json!({"text": "two"}));
            if redo.is_some() {
                run(&mut s, "edit.undo", json!({}));
            }
            assert_eq!(s.undo_labels().len(), len);
            assert_eq!(s.redo_label(), redo);
            s
        };
        let mut failed = session();
        let err = failed.run("file.inspect", &json!({"remove": ["comments", "revisions"]}));
        assert!(matches!(err, Err(crate::CmdError::Disabled(_))), "{len}: {err:?}");
        assert_eq!(failed.undo_labels().len(), len, "{len}: undo steps");
        assert_eq!(failed.redo_label(), redo, "{len}: redo step");
        assert!(timeline(&mut failed) == timeline(&mut session()), "{len}: undo history changed");
    }
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
    // Enter capitalised the first word, as Word's AutoCorrect does.
    assert_eq!(text(&s), "Abcd");
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
fn border_toggles_and_saves() {
    use wordcraft_doc::props::Border;
    let border = |s: &Session| s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(5).border;
    // Saved .docx carries the border iff it is on (read back: the reader maps only `w:bdr` to `border`).
    let docx_has_bdr = |s: &Session| {
        let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
        back.para_at(&Pos::body(0, 0)).unwrap().props_of_char(5).border.is_some()
    };
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "make this boxed"}));
    run(&mut s, "select.text", json!({"text": "this"}));
    run(&mut s, "format.border", json!({}));
    assert_eq!(border(&s), Some(Border::single(0.5)));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(0).border, None);
    assert!(docx_has_bdr(&s));
    run(&mut s, "format.border", json!({}));
    assert_eq!(border(&s), None);
    run(&mut s, "format.border", json!({"value": true}));
    run(&mut s, "format.border", json!({"value": true}));
    assert_eq!(border(&s), Some(Border::single(0.5)));
    run(&mut s, "format.border", json!({"value": false}));
    assert_eq!(border(&s), None);
    assert!(!docx_has_bdr(&s));
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
    assert!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.is_none_or(|n| n.num == 0));
    // "1. " autoformat.
    run(&mut s, "text.insert", json!({"text": "1."}));
    run(&mut s, "text.insert", json!({"text": " "}));
    assert!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.is_some_and(|n| n.num != 0));
}

#[test]
fn applying_a_heading_clears_list_numbering() {
    // Applying a heading (or Title/Normal) to a numbered paragraph removes its direct list
    // numbering. A dead `&& id == "Normal"` term used to let only Normal clear it, so headings
    // kept the numbering.
    for style_cmd in ["para.heading1", "para.heading2", "para.normal"] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "item"}));
        run(&mut s, "para.bullets", json!({}));
        assert!(
            s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.is_some_and(|n| n.num != 0),
            "{style_cmd}: expected list numbering before applying the style"
        );
        run(&mut s, style_cmd, json!({}));
        assert!(
            s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.is_none_or(|n| n.num == 0),
            "{style_cmd}: should clear direct list numbering"
        );
    }
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
fn replace_under_track_changes_skips_deleted_text() {
    // Replace marks the match deleted and leaves it in the text; the next
    // step must not find it again, or Replace never advances.
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "one cat two cat"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat"}))["count"], 2);
    let mut remaining = Vec::new();
    for _ in 0..4 {
        remaining.push(run(&mut s, "edit.replace", json!({"text": "cat", "with": "dog"}))["remaining"].clone());
    }
    assert_eq!(remaining, [json!(1), json!(0), json!(0), json!(0)]);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat"}))["count"], 0);
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "one dog two dog");

    // A match that runs across tracked-deleted text is not in the document.
    let mut s = self::s();
    run(&mut s, "document.setText", json!({"text": "cat"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 1}, "focus": {"block": 0, "off": 2}}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(text(&s), "cat", "the deletion is tracked, not applied");
    assert_eq!(run(&mut s, "edit.find", json!({"text": "cat"}))["count"], 0);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "t"}))["count"], 1);

    // Without Track Changes, Replace steps through every match as before.
    let mut s = self::s();
    run(&mut s, "document.setText", json!({"text": "one cat two cat"}));
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "edit.find", json!({"text": "cat"}));
    run(&mut s, "edit.replace", json!({"text": "cat", "with": "dog"}));
    let r = run(&mut s, "edit.replace", json!({"text": "cat", "with": "dog"}));
    assert_eq!(r["remaining"], 0);
    assert_eq!(text(&s), "one dog two dog");
}

#[test]
fn find_reads_the_text_around_tracked_deletions() {
    // `edits` are (start, end) byte ranges deleted with Track Changes on, last first.
    fn tracked(text: &str, edits: &[(usize, usize)]) -> Session {
        let mut s = self::s();
        run(&mut s, "document.setText", json!({"text": text}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        for (a, b) in edits {
            run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": a}, "focus": {"block": 0, "off": b}}));
            run(&mut s, "text.delete", json!({}));
        }
        assert_eq!(text, self::text(&s), "the deletions are tracked, not applied");
        s
    }
    let offs = |r: &serde_json::Value| -> Vec<(u64, u64)> {
        r["matches"].as_array().unwrap().iter().map(|m| (m["start"]["off"].as_u64().unwrap(), m["end"]["off"].as_u64().unwrap())).collect()
    };

    // "aaa" with the first "a" deleted reads "aa": the rejected raw match 0..2 must not hide
    // the live one at 1..3.
    let mut s = tracked("aaa", &[(0, 1)]);
    assert_eq!(offs(&run(&mut s, "edit.find", json!({"text": "aa"}))), [(1, 3)]);
    assert_eq!(offs(&run(&mut s, "edit.find", json!({"text": "^a", "regex": true}))), [(1, 2)]);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "aa", "with": "b"}))["replaced"], 1);

    // Whole words follow the live text: deleting the space joins "cat" and "fish"...
    let mut s = tracked("cat fish", &[(3, 4)]);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "fish", "wholeWord": true}))["count"], 0);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "catfish", "wholeWord": true}))["count"], 0, "a match may not span a deletion");
    assert_eq!(run(&mut s, "edit.find", json!({"text": "fish", "wholeWord": false}))["count"], 1);
    // ...and deleting the "X" leaves "fish" a word of its own.
    let mut s = tracked("cat Xfish", &[(4, 5)]);
    assert_eq!(offs(&run(&mut s, "edit.find", json!({"text": "FISH", "wholeWord": true}))), [(5, 9)]);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "FISH", "wholeWord": true, "matchCase": true}))["count"], 0);

    // Replace and the Find Next/Previous steps agree with Find.
    let mut s = tracked("aaa aaa", &[(4, 5), (0, 1)]);
    run(&mut s, "caret.docStart", json!({}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "aa"}))["count"], 2);
    assert_eq!(run(&mut s, "edit.findNext", json!({}))["index"], 1);
    assert_eq!(run(&mut s, "edit.findPrevious", json!({}))["index"], 0);
    let r = run(&mut s, "edit.replace", json!({"text": "aa", "with": "b"}));
    assert_eq!(r["remaining"], 1);
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
fn table_border_presets_mask_the_sides_they_clear() {
    // On a default (TableGrid) table the "outside"/"inside" presets must write nil over the
    // sides they clear; without it those sides inherit the style's grid (#143) and every
    // preset renders like "all".
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    // Distinct vertical/horizontal rule positions on page 0.
    let rule_pos = |s: &mut Session| {
        let (mut vx, mut hy) = (Vec::new(), Vec::new());
        for it in &s.layout().pages[0].items {
            if let crate::layout::Placed::Rule { x0, y0, x1, y1, .. } = it {
                if (x0 - x1).abs() < 1e-3 && !vx.contains(x0) {
                    vx.push(*x0);
                }
                if (y0 - y1).abs() < 1e-3 && !hy.contains(y0) {
                    hy.push(*y0);
                }
            }
        }
        (vx, hy)
    };
    // Style default: a full 2x2 grid — 3 vertical + 3 horizontal lines.
    let (vx, hy) = rule_pos(&mut s);
    assert_eq!((vx.len(), hy.len()), (3, 3), "baseline grid: {vx:?} {hy:?}");
    run(&mut s, "table.borders", json!({"kind": "outside"}));
    let (vx, hy) = rule_pos(&mut s);
    assert_eq!((vx.len(), hy.len()), (2, 2), "outside keeps only the frame: {vx:?} {hy:?}");
    let b = s.doc.body.iter().find_map(|x| x.as_table()).unwrap().props.borders.unwrap();
    assert!(b.between.is_some_and(|x| x.style == BorderStyle::None));
    assert!(b.inside_v.is_some_and(|x| x.style == BorderStyle::None));
    run(&mut s, "table.borders", json!({"kind": "inside"}));
    let (vx, hy) = rule_pos(&mut s);
    assert_eq!((vx.len(), hy.len()), (1, 1), "inside keeps only the inner rules: {vx:?} {hy:?}");
    let b = s.doc.body.iter().find_map(|x| x.as_table()).unwrap().props.borders.unwrap();
    for side in [b.top, b.bottom, b.left, b.right] {
        assert!(side.is_some_and(|x| x.style == BorderStyle::None));
    }
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

/// The author of the revision `rid` points to.
fn author_of(s: &Session, rid: Option<u32>) -> Option<String> {
    rid.and_then(|r| s.doc.revisions.get(r as usize)).map(|r| r.author.clone())
}

#[test]
fn tracked_delete_keeps_another_authors_deletion() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "alpha beta gamma"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    s.author = "Ana".into();
    run(&mut s, "select.text", json!({"text": "beta"}));
    run(&mut s, "text.delete", json!({}));
    // A second author deletes the whole line, including the text Ana already deleted.
    s.author = "Ben".into();
    run(&mut s, "select.text", json!({"text": "alpha beta gamma"}));
    run(&mut s, "text.delete", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let by: Vec<(String, Option<String>)> = p.run_ranges().map(|(r, c)| (p.text[r].to_string(), author_of(&s, c.del))).collect();
    assert!(by.iter().any(|(t, a)| t == "beta" && a.as_deref() == Some("Ana")), "{by:?}");
    assert!(by.iter().any(|(t, a)| t.contains("alpha") && a.as_deref() == Some("Ben")), "{by:?}");
    assert!(by.iter().any(|(t, a)| t.contains("gamma") && a.as_deref() == Some("Ben")), "{by:?}");
}

#[test]
fn tracked_split_gives_the_new_paragraph_mark_to_its_author() {
    // Ben presses Enter at the end of Ana's insertion, and in the middle of it.
    for off in [None, Some(8)] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "alpha"}));
        run(&mut s, "review.trackChanges", json!({"value": true}));
        s.author = "Ana".into();
        run(&mut s, "text.insert", json!({"text": " beta"}));
        s.author = "Ben".into();
        if let Some(off) = off {
            run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [0], "off": off}}));
        }
        run(&mut s, "text.newParagraph", json!({}));
        let head = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let tail = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap();
        assert_eq!(author_of(&s, head.mark.ins).as_deref(), Some("Ben"), "split at {off:?}: the new paragraph mark is Ben's");
        // The second paragraph ends with the original mark, which nobody inserted (not Ana).
        assert_eq!(tail.mark.ins, None, "split at {off:?}");
        assert_eq!(tail.mark.del, None, "split at {off:?}");
    }
}

#[test]
fn no_markup_view_lays_out_the_final_text() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "original"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "select.text", json!({"text": "orig"}));
    run(&mut s, "text.delete", json!({}));
    let gap = |s: &mut Session| {
        let l = s.layout();
        let x = |off| l.caret(&Pos::body(0, off)).map(|c| c.x).unwrap_or(f32::NAN);
        x(4) - x(0)
    };
    assert!(gap(&mut s) > 5.0, "markup shows the deletion");
    run(&mut s, "review.markup", json!({"value": "noMarkup"}));
    assert!(gap(&mut s).abs() < 0.01, "No Markup leaves it out");
    run(&mut s, "review.showMarkup", json!({"value": true}));
    assert!(gap(&mut s) > 5.0);
    run(&mut s, "review.showMarkup", json!({"value": false}));
    assert!(gap(&mut s).abs() < 0.01);
}

#[test]
fn replace_all_is_tracked() {
    let mut s = s();
    let original = "We walked towards the light, then towards home.";
    run(&mut s, "document.setText", json!({"text": original}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    let r = run(&mut s, "edit.replaceAll", json!({"text": "towards", "with": "toward", "matchCase": true}));
    assert_eq!(r["replaced"], 2);
    let ch = run(&mut s, "review.changes", json!({}));
    assert!(!ch.as_array().unwrap().is_empty(), "replace all must leave tracked revisions");
    run(&mut s, "review.rejectAll", json!({}));
    assert_eq!(text(&s), original);
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
fn toc_page_numbers_follow_headings() {
    let mut s = s();
    run(&mut s, "references.toc", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    for name in ["Alpha", "Bravo", "Charlie"] {
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.insert", json!({"text": name}));
        run(&mut s, "para.heading1", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "references.updateToc", json!({}));
    let t = text(&s);
    for (name, page) in [("Alpha", 2), ("Bravo", 3), ("Charlie", 4)] {
        assert!(t.contains(&format!("{name}\t{page}")), "{name} should be on page {page}: {t}");
    }
}

/// A heading above the TOC keeps its index when the entries go in below it, so its page is read
/// where it is, not `entries` blocks further down.
#[test]
fn toc_page_number_for_heading_above_toc() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Preface"}));
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.pageBreak", json!({}));
    run(&mut s, "references.toc", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    for name in ["Alpha", "Bravo"] {
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.insert", json!({"text": name}));
        run(&mut s, "para.heading1", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "references.updateToc", json!({}));
    let t = text(&s);
    for (name, page) in [("Preface", 1), ("Alpha", 3), ("Bravo", 4)] {
        assert!(t.contains(&format!("{name}\t{page}")), "{name} should be on page {page}: {t:?}");
    }
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
        // These reach outside the session (files). Read Aloud (`review.readAloud`, `readAloud.*`)
        // is fuzzed too: under `cfg(test)` its backend is the silent `Hold`, asserted below.
        if spec.id.starts_with("file.") || spec.id == "insert.picture" || spec.id == "insert.textFromFile" {
            continue;
        }
        for j in &junk {
            let mut s = Session::new(crate::sample::sample_document());
            assert_ne!(s.read_aloud.backend, crate::speech::Backend::System, "tests must never start real speech");
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
fn text_box_in_a_table_cell_shows_its_text() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 1, "cols": 1}));
    assert!(s.sel.focus.path.0.len() > 1);
    let id = run(&mut s, "insert.textBox", json!({}))["story"].as_u64().unwrap() as u32;
    // The caret goes into the box, and typing shows there (it has a caret position).
    assert_eq!(s.sel.focus.story, StoryRef::Part(id));
    run(&mut s, "text.insert", json!({"text": "In a cell"}));
    let caret = s.layout().caret(&s.sel.focus).expect("laid out");
    let cell = s.layout().find_object(0, |o| o.text_box == Some(id)).unwrap();
    assert!(cell.rect.contains(wordcraft_geom::Point::new(caret.x, caret.top)), "{caret:?} in {:?}", cell.rect);
    // No text box inside a text box.
    assert!(s.run("insert.textBox", &json!({})).is_err());
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
    // Moving it again anchors it to the paragraph under its top edge, relative to that
    // paragraph (it moves with the text), landing exactly where dropped.
    let before = box_area(&mut s).rect;
    select_box(&mut s);
    run(&mut s, "arrange.bounds", json!({"x": before.x + 30.0, "y": before.y - 5.0}));
    let o = box_area(&mut s);
    assert!((o.rect.x - before.x - 30.0).abs() < 0.01 && (o.rect.y - before.y + 5.0).abs() < 0.01, "{:?} → {:?}", before, o.rect);
    let p = s.doc.para(StoryRef::Body, &o.path).unwrap();
    let Some(wordcraft_doc::InlineObject::Shape { float, .. }) = p.object_at(o.off) else { panic!() };
    assert_eq!((float.h_rel, float.v_rel), (Anchor::Column, Anchor::Paragraph));
    assert!((float.x - (o.rect.x - o.origin.x)).abs() < 0.01 && (float.y - (o.rect.y - o.origin.y)).abs() < 0.01);
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
fn nudging_an_aligned_object_moves_it_from_where_it_is() {
    use wordcraft_doc::para::{Anchor, FloatAlign};
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Some text "}));
    run(&mut s, "insert.textBox", json!({"text": "Box"}));
    select_box(&mut s);
    run(&mut s, "arrange.position", json!({"preset": "topLeft"}));
    // As a .docx can place it: centred on the page, at the bottom margin.
    let o = box_area(&mut s);
    let p = s.doc.para_mut(StoryRef::Body, &o.path).unwrap();
    if let Some(wordcraft_doc::InlineObject::Shape { float, .. }) = p.object_at_mut(o.off) {
        (float.h_rel, float.h_align, float.v_rel, float.v_align) =
            (Anchor::Page, Some(FloatAlign::Center), Anchor::BottomMargin, Some(FloatAlign::Start));
    }
    p.touch();
    s.touch();
    let before = box_area(&mut s).rect;
    select_box(&mut s);
    run(&mut s, "arrange.nudge", json!({"dx": 6, "dy": -2}));
    let o = box_area(&mut s);
    assert!((o.rect.x - before.x - 6.0).abs() < 0.01 && (o.rect.y - before.y + 2.0).abs() < 0.01, "{before:?} → {:?}", o.rect);
    let p = s.doc.para(StoryRef::Body, &o.path).unwrap();
    let Some(wordcraft_doc::InlineObject::Shape { float, .. }) = p.object_at(o.off) else { panic!() };
    assert_eq!((float.h_align, float.v_align), (None, None));
    // Dragging it clears alignment too, and it lands where dropped.
    select_box(&mut s);
    run(&mut s, "arrange.bounds", json!({"x": 100, "y": 150}));
    let o = box_area(&mut s);
    assert_eq!((o.rect.x, o.rect.y), (100.0, 150.0));
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

#[test]
fn moved_text_box_makes_the_text_it_lands_on_wrap() {
    let mut s = s();
    for i in 0..6 {
        run(
            &mut s,
            "text.insert",
            json!({"text": format!("Paragraph {i}: the quick brown fox jumps over the lazy dog, again and again, across the page.")}),
        );
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "caret.set", json!({"pos": Pos::body(4, 0)}));
    run(&mut s, "insert.textBox", json!({"text": "Box"}));
    select_box(&mut s);
    // Drop it over paragraph 1, right half of the page: well above its own paragraph (4).
    let l = s.layout();
    let p1_top = wordcraft_layout::hit::page_lines(&l.pages[0], StoryRef::Body).iter().find(|ln| ln.path.0 == [1] && ln.li == 0).unwrap().top;
    run(&mut s, "arrange.bounds", json!({"x": 300, "y": p1_top + 2.0}));
    let o = box_area(&mut s);
    assert_eq!(o.path.0, vec![1], "anchored to the paragraph it was dropped on");
    assert_eq!((o.rect.x, o.rect.y), (300.0, p1_top + 2.0));
    // Lines beside the box stop short of it; lines below it use the full width again.
    let l = s.layout();
    let lines = wordcraft_layout::hit::page_lines(&l.pages[0], StoryRef::Body);
    let beside: Vec<_> = lines.iter().filter(|ln| ln.bottom > o.rect.y && ln.top < o.rect.bottom()).collect();
    assert!(!beside.is_empty());
    assert!(beside.iter().all(|ln| ln.right <= o.rect.x + 0.5 || ln.left >= o.rect.right() - 0.5), "text overlaps the box");
    assert!(lines.iter().any(|ln| ln.top > o.rect.bottom() + 10.0 && ln.right > o.rect.right()));
    // Its old paragraph no longer holds it.
    assert!(s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(4)).unwrap().objects.is_empty());
}

#[test]
fn deleted_text_box_takes_its_text_with_it_until_undo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "A"}));
    let id = run(&mut s, "insert.textBox", json!({"text": "Inside"}))["story"].as_u64().unwrap() as u32;
    let box_sel = json!({"anchor": Pos::body(0, 1), "focus": Pos::body(0, 1 + 3)});
    run(&mut s, "select.range", box_sel.clone());
    run(&mut s, "text.delete", json!({}));
    assert!(!s.doc.parts.contains_key(&id), "the deleted box's text is gone");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Part(id)), "Inside", "undo brings it back");
    // Cut and paste: one box, one story, same text.
    run(&mut s, "select.range", box_sel);
    run(&mut s, "edit.cut", json!({}));
    assert!(s.doc.parts.is_empty());
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.doc.parts.len(), 1);
    let (_, part) = s.doc.parts.iter().next().unwrap();
    assert_eq!(part.kind, wordcraft_doc::PartKind::TextBox);
    assert_eq!(s.doc.word_count_including_notes(), 2);
}

#[test]
fn word_count_with_an_object_selected_counts_the_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "one two "}));
    run(&mut s, "insert.textBox", json!({"text": "three"}));
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 8), "focus": Pos::body(0, 8 + 3)}));
    assert_eq!(run(&mut s, "review.wordCount", json!({}))["words"].as_u64(), Some(3));
    // A real text selection still counts just itself.
    run(&mut s, "select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, 3)}));
    assert_eq!(run(&mut s, "review.wordCount", json!({}))["words"].as_u64(), Some(1));
}

#[test]
fn custom_properties_set_read_remove_and_undo() {
    let mut s = s();
    let r = run(&mut s, "file.properties", json!({"custom": {"ZOTERO_PREF_1": "<data/>", "Status": "draft"}}));
    let custom = r["custom"].as_array().cloned().unwrap_or_default();
    assert!(custom.iter().any(|p| p["name"] == "Status" && p["value"] == "draft" && p["kind"] == "lpwstr"));
    assert_eq!(custom.len(), 2);
    assert_eq!(s.doc.custom_prop("zotero_pref_1"), Some("<data/>"));
    let r = run(&mut s, "file.properties", json!({"custom": {"status": null}}));
    assert_eq!(r["custom"].as_array().map(|a| a.len()), Some(1));
    assert!(s.run("file.properties", &json!({"custom": "x"})).is_err());
    assert!(s.run("file.properties", &json!({"custom": {"": "x"}})).is_err());
    assert!(s.run("file.properties", &json!({"custom": {"n": 3}})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.custom_prop("Status"), Some("draft"));
}

/// `document_id` tells an edit from a replacement (the UI drops a "Save changes?" prompt
/// about a document that has been replaced).
#[test]
fn document_id_changes_only_when_the_document_is_replaced() {
    let mut s = s();
    let first = s.document_id();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "format.bold", json!({}));
    assert_eq!(s.document_id(), first);
    run(&mut s, "file.new", json!({"template": "letter"}));
    let second = s.document_id();
    assert_ne!(second, first);
    run(&mut s, "file.new", json!({}));
    assert_ne!(s.document_id(), second);
}

/// Envelopes, Labels and Finish & Merge make a new, untitled document, as Word does, and like New
/// its undo history starts afresh. Undo/Redo across the swap put the wrong document under a file
/// (Undo, Save As, Redo, Save wrote the envelope over the saved file), so it isn't offered; the UI
/// asks to save the replaced document first.
#[test]
fn mailings_results_are_new_untitled_documents() {
    let dir = std::env::temp_dir().join(format!("wordcraft-engine-mailings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("letter.docx");
    let saved_as = dir.join("saved-as.docx");
    let on_file = |p: &std::path::Path| crate::io::open_path(p).unwrap().plain_text(StoryRef::Body);
    for id in ["mailings.envelopes", "mailings.labels", "mailings.finish"] {
        let mut s = s();
        run(&mut s, "mailings.recipients", json!({"csv": "First Name\nAda\nAlan"}));
        run(&mut s, "text.insert", json!({"text": "Dear "}));
        run(&mut s, "mailings.insertField", json!({"field": "First Name"}));
        run(&mut s, "file.save", json!({"path": original.to_string_lossy()}));
        let on_disk = std::fs::read(&original).unwrap();
        run(&mut s, "text.insert", json!({"text": ", unsaved"}));
        let before = text(&s);
        let document = s.document_id();

        run(&mut s, id, json!({}));
        let result = text(&s);
        assert_ne!(result, before, "{id}: the result replaced the document");
        assert_ne!(s.document_id(), document, "{id}: a different document");
        assert_eq!(s.path, None, "{id}: the result is untitled");
        assert_eq!(run(&mut s, "file.save", json!({}))["saved"], false, "{id}: Save asks where (Save As)");
        assert_eq!(std::fs::read(&original).unwrap(), on_disk, "{id}: the original file is untouched");

        // Undo doesn't bring the old document back under the new one's identity; Redo can't
        // put the result back under a file saved in between.
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(text(&s), result, "{id}: history starts afresh, like New");
        run(&mut s, "file.saveAs", json!({"path": saved_as.to_string_lossy()}));
        let written = on_file(&saved_as);
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(text(&s), result, "{id}: nothing to redo");
        run(&mut s, "file.save", json!({}));
        assert_eq!(on_file(&saved_as), written, "{id}: the saved file still holds what was saved");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_load_relative_to_the_file() {
    // Issue #98: `<img src="logo.png">` beside an HTML file is embedded when it is opened.
    let dir = std::env::temp_dir().join(format!("wordcraft-html-img-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("img")).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(dir.join("img/my logo.png"), &png).unwrap();
    std::fs::write(dir.join("secret.txt"), b"not a picture").unwrap();
    let html = r#"<p><img src="img/my%20logo.png" alt="Logo"></p><p><img src="secret.txt" alt="T"></p><p><img src="http://example.com/x.png" alt="Web"></p>"#;
    std::fs::write(dir.join("page.html"), html).unwrap();
    let doc = crate::io::open_path(&dir.join("page.html"));
    let _ = std::fs::remove_dir_all(&dir);
    let doc = doc.unwrap();
    assert_eq!(doc.media.len(), 1);
    let text = doc.plain_text(StoryRef::Body);
    assert!(!text.contains("Logo") && text.contains('T') && text.contains("Web"), "{text:?}");
}

/// A fresh, empty scratch folder for one test.
#[cfg(not(target_arch = "wasm32"))]
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(not(target_arch = "wasm32"))]
fn tiny_png() -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_outside_the_folder_are_not_loaded() {
    // #98 review: `..` must not reach files outside the HTML file's folder.
    let root = scratch_dir("html-img-escape");
    std::fs::create_dir_all(root.join("site/img")).unwrap();
    std::fs::write(root.join("outside.png"), tiny_png()).unwrap();
    std::fs::write(root.join("site/inside.png"), tiny_png()).unwrap();
    let html = r#"<p><img src="../outside.png" alt="Up"></p><p><img src="img/../../outside.png" alt="Sneak"></p><p><img src="img/../inside.png" alt="In"></p>"#;
    std::fs::write(root.join("site/page.html"), html).unwrap();
    let doc = crate::io::open_path(&root.join("site/page.html"));
    let _ = std::fs::remove_dir_all(&root);
    let doc = doc.unwrap();
    assert_eq!(doc.media.len(), 1, "only the picture inside the folder loads");
    let text = doc.plain_text(StoryRef::Body);
    assert!(text.contains("Up") && text.contains("Sneak") && !text.contains("In"), "{text:?}");
}

#[cfg(unix)]
#[test]
fn html_pictures_through_a_symlink_out_of_the_folder_are_not_loaded() {
    // #98 review: a symlink inside the folder must not lead outside it.
    let root = scratch_dir("html-img-symlink");
    std::fs::create_dir_all(root.join("site")).unwrap();
    std::fs::write(root.join("outside.png"), tiny_png()).unwrap();
    std::os::unix::fs::symlink(root.join("outside.png"), root.join("site/link.png")).unwrap();
    std::fs::write(root.join("site/page.html"), r#"<p><img src="link.png" alt="Link"></p>"#).unwrap();
    let doc = crate::io::open_path(&root.join("site/page.html"));
    let _ = std::fs::remove_dir_all(&root);
    let doc = doc.unwrap();
    assert!(doc.media.is_empty());
    assert!(doc.plain_text(StoryRef::Body).contains("Link"));
}

#[cfg(unix)]
#[test]
fn html_picture_that_is_a_named_pipe_is_skipped_without_blocking() {
    // #98 review: opening a FIFO blocks until a writer appears; it must never be opened.
    let dir = scratch_dir("html-img-fifo");
    let fifo = dir.join("pipe.png");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("mkfifo unavailable; skipping");
        return;
    }
    std::fs::write(dir.join("page.html"), r#"<p><img src="pipe.png" alt="Pipe"></p>"#).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let page = dir.join("page.html");
    std::thread::spawn(move || {
        let _ = tx.send(crate::io::open_path(&page));
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    if got.is_err() {
        // Unblock the stuck reader so the thread ends, then fail.
        let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let doc = got.expect("opening the HTML file blocked on a named pipe").unwrap();
    assert!(doc.media.is_empty());
    assert!(doc.plain_text(StoryRef::Body).contains("Pipe"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_picture_size_limit_holds_whatever_length_the_file_reports() {
    use crate::io::LocalImages;
    let dir = scratch_dir("html-img-limit");
    std::fs::write(dir.join("ten.bin"), [7u8; 10]).unwrap();
    std::fs::write(dir.join("eleven.bin"), [7u8; 11]).unwrap();
    let images = LocalImages::with_limits(&dir, 10, 1000);
    let ten = images.load("ten.bin").map(|d| d.len());
    let eleven = images.load("eleven.bin");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(ten, Some(10));
    assert!(eleven.is_none());
    #[cfg(unix)]
    {
        // A device reports a length of 0 and never ends: it is not a regular file, and the read is
        // bounded anyway.
        let dev = LocalImages::with_limits(std::path::Path::new("/dev"), 1 << 20, 1 << 20);
        assert!(dev.load("zero").is_none());
        assert!(dev.load("null").is_none());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_share_one_read_and_a_per_document_budget() {
    // #98 review: a picture referenced many times is read once, and every reference counts
    // against the document's budget so a large picture can't be multiplied without bound.
    use crate::io::LocalImages;
    let dir = scratch_dir("html-img-budget");
    let png = tiny_png();
    let n = png.len() as u64;
    std::fs::write(dir.join("a.png"), &png).unwrap();
    let images = LocalImages::with_limits(&dir, 1 << 20, 3 * n);
    let first = images.load("a.png").unwrap();
    std::fs::write(dir.join("a.png"), b"changed on disk").unwrap();
    let second = images.load("./a.png").unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second), "the second reference reuses the first read");
    assert_eq!(*second, png);
    assert!(images.load("a.png").is_some());
    assert!(images.load("a.png").is_none(), "budget spent");
    // Files over the per-picture limit still cost what was read.
    std::fs::write(dir.join("big.bin"), vec![1u8; 100]).unwrap();
    std::fs::write(dir.join("small.bin"), [1u8; 5]).unwrap();
    let images = LocalImages::with_limits(&dir, 10, 25);
    assert!(images.load("big.bin").is_none());
    assert!(images.load("big.bin").is_none());
    let small = images.load("small.bin");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(small.is_none(), "two oversized reads used up the budget");
}

fn para_style(s: &Session, i: usize) -> Option<String> {
    s.doc.para_at(&Pos::body(i, 0)).and_then(|p| p.props.style.clone())
}

#[test]
fn styles_apply_by_name() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Title text"}));
    run(&mut s, "styles.apply", json!({"style": "Heading 1"}));
    assert_eq!(para_style(&s, 0).as_deref(), Some("Heading1"));
    assert_eq!(s.undo_label(), Some("Apply Style"));
    assert!(s.dirty);
}

#[test]
fn styles_apply_pane_is_not_an_edit() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    s.dirty = false;
    let undo = s.undo_labels();
    s.view.styles_pane = false;
    assert_eq!(run(&mut s, "styles.apply", json!({})), json!({"pane": true}));
    assert!(s.view.styles_pane);
    // Shows, never toggles.
    run(&mut s, "styles.apply", json!({}));
    assert!(s.view.styles_pane);
    assert_eq!(s.undo_labels(), undo);
    assert!(!s.dirty);
    // Works on a read-only document.
    s.doc.settings.protection = Some("readOnly".into());
    s.view.styles_pane = false;
    run(&mut s, "styles.apply", json!({}));
    assert!(s.view.styles_pane);
}

#[test]
fn styles_apply_rejects_bad_style_params() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let undo = s.undo_labels();
    let before = text(&s);
    for bad in [json!(5), json!(null), json!([]), json!({}), json!(""), json!("   "), json!(true)] {
        let r = s.run("styles.apply", &json!({"style": bad}));
        assert!(matches!(r, Err(crate::CmdError::Params(_))), "{bad}: {r:?}");
    }
    assert_eq!(s.undo_labels(), undo);
    assert_eq!(text(&s), before);
    assert_eq!(para_style(&s, 0), None);
}

#[test]
fn styles_apply_unknown_style_changes_nothing() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    let undo = s.undo_labels();
    let doc = s.doc.clone();
    assert!(s.run("styles.apply", &json!({"style": "No Such Style"})).is_err());
    assert_eq!(s.undo_labels(), undo);
    assert_eq!(s.doc, doc);
    assert!(!s.can_redo());
}

#[test]
fn styles_apply_undo_redo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "styles.apply", json!({"style": "Heading 2"}));
    assert_eq!(para_style(&s, 0).as_deref(), Some("Heading2"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(para_style(&s, 0), None);
    assert_eq!(text(&s), "Hello");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(para_style(&s, 0).as_deref(), Some("Heading2"));
}

#[test]
fn styles_apply_character_style() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "make this strong"}));
    run(&mut s, "select.text", json!({"text": "this"}));
    run(&mut s, "styles.apply", json!({"style": "Strong"}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(6).style.as_deref(), Some("Strong"));
    assert_eq!(p.props_of_char(0).style, None);
    // A character style leaves the paragraph style alone.
    assert_eq!(p.props.style, None);
}

#[test]
fn styles_apply_multi_paragraph_selection() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "One"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Two"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Three"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "styles.apply", json!({"style": "Quote"}));
    for i in 0..3 {
        assert_eq!(para_style(&s, i).as_deref(), Some("Quote"), "paragraph {i}");
    }
    // One undo step for the whole selection.
    run(&mut s, "edit.undo", json!({}));
    for i in 0..3 {
        assert_eq!(para_style(&s, i), None, "paragraph {i}");
    }
}

#[test]
fn styles_apply_rejected_on_protected_document() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    s.doc.settings.protection = Some("readOnly".into());
    let undo = s.undo_labels();
    let r = s.run("styles.apply", &json!({"style": "Heading 1"}));
    assert!(matches!(r, Err(crate::CmdError::Disabled(_))), "{r:?}");
    assert_eq!(para_style(&s, 0), None);
    assert_eq!(s.undo_labels(), undo);
}
