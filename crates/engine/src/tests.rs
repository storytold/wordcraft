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
        // These reach outside the session: files, and Read Aloud starts the system speech
        // synthesiser (`say` on macOS), which would read the sample document aloud on every run.
        if spec.id.starts_with("file.") || spec.id == "insert.picture" || spec.id == "insert.textFromFile" || spec.id == "review.readAloud" {
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

#[test]
fn norwegian_proofing_and_mixed_language_documents() {
    let mut s = s();
    run(&mut s, "file.new", json!({"locale": "nb-NO"}));
    run(&mut s, "text.insert", json!({"text": "Ærlig norsk bokmål. Bøøkene er åpne."}));
    let issues = run(&mut s, "review.issues", json!({}));
    let issues = issues.as_array().unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0]["text"], "Bøøkene");
    assert_eq!(issues[0]["kind"], "spelling");
    assert!(issues[0]["suggestions"].as_array().unwrap().contains(&json!("Bøkene")));
    run(&mut s, "select.text", json!({"text": "Bøøkene"}));
    assert_eq!(run(&mut s, "review.suggestions", json!({}))["text"], "Bøøkene");
    assert_eq!(run(&mut s, "review.spelling", json!({}))["text"], "Bøøkene");
    run(&mut s, "text.insert", json!({"text": "Bøkene"}));
    assert!(run(&mut s, "review.issues", json!({})).as_array().unwrap().is_empty());
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "review.language", json!({"lang": "en-US"}));
    run(&mut s, "text.insert", json!({"text": "Hello wrold."}));
    let issues = run(&mut s, "review.issues", json!({}));
    assert_eq!(issues.as_array().unwrap().len(), 1, "{issues}");
    assert_eq!(issues[0]["text"], "wrold");
    assert!(issues[0]["suggestions"].as_array().unwrap().contains(&json!("world")));
    let bytes = crate::io::save_bytes("mixed.docx", &s.doc).unwrap();
    let mut reopened = Session::new(crate::io::open_bytes("mixed.docx", &bytes).unwrap());
    assert_eq!(run(&mut reopened, "review.issues", json!({})), issues);
}
